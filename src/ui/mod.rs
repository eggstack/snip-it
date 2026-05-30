mod highlight;
mod theme;
mod variables;

pub use theme::get_theme;
#[allow(unused_imports)]
pub use theme::Theme;
pub use variables::{prompt_variables, VariablePromptResult};

#[allow(unused_imports)]
pub use crate::utils::variables::Variable;

use std::io;
use std::sync::LazyLock;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::text::Line;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{
        Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};

use crate::clipboard;
use crate::utils::extract_variables_for_display;
use crate::utils::has_unmatched_angle_bracket;
use crate::utils::strip_escape_sequences;

use highlight::highlight_command;
use theme::{style_fg, style_fg_bg};

static TERMINATE: LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    LazyLock::new(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));

pub fn get_terminate() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    TERMINATE.clone()
}

static MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);

fn is_ctrl_key(key: &crossterm::event::KeyEvent, c: char) -> bool {
    key.code == crossterm::event::KeyCode::Char(c)
        && key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
}

#[derive(Clone, Debug, Default, PartialEq)]
enum SortMode {
    #[default]
    None,
    Newest,
    Oldest,
    AlphaAsc,
    AlphaDesc,
}

#[derive(Clone, Default)]
struct SelectState {
    selected: usize,
    list_state: ratatui::widgets::ListState,
    scroll_state: ScrollbarState,
}

impl SelectState {
    fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        SelectState {
            selected: 0,
            list_state,
            scroll_state: ScrollbarState::default(),
        }
    }

    fn update(&mut self, filtered_len: usize) {
        if filtered_len == 0 {
            self.selected = 0;
        } else if self.selected >= filtered_len {
            self.selected = filtered_len.saturating_sub(1);
        }
        self.list_state.select(Some(self.selected));
        self.scroll_state = self
            .scroll_state
            .content_length(filtered_len)
            .position(self.selected);
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self, max: usize) {
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    fn move_to_top(&mut self) {
        self.selected = 0;
    }

    fn move_to_bottom(&mut self, max: usize) {
        self.selected = max.saturating_sub(1);
    }
}

#[derive(Clone, Default)]
struct FilterState {
    sort_mode: SortMode,
    tag_filter_text: String,
}

impl FilterState {
    fn toggle_sort_new(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::Newest {
            SortMode::None
        } else {
            SortMode::Newest
        };
    }
    fn toggle_sort_old(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::Oldest {
            SortMode::None
        } else {
            SortMode::Oldest
        };
    }
    fn toggle_sort_alpha(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::AlphaAsc {
            SortMode::None
        } else {
            SortMode::AlphaAsc
        };
    }
    fn toggle_sort_alpha_rev(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::AlphaDesc {
            SortMode::None
        } else {
            SortMode::AlphaDesc
        };
    }
}

fn extract_variables(command: &str) -> Vec<String> {
    extract_variables_for_display(command)
}

#[allow(clippy::too_many_arguments)]
pub fn select_snippet(
    descriptions: &[String],
    commands: &[String],
    tags: &[Vec<String>],
    is_search: bool,
    initial_filter: Option<&str>,
    folders: &[Vec<String>],
    favorites: &[bool],
    snippets: &[crate::library::Snippet],
) -> io::Result<Option<(usize, Option<String>)>> {
    select_snippet_inner(
        descriptions,
        commands,
        tags,
        is_search,
        initial_filter,
        folders,
        favorites,
        snippets,
    )
}

#[allow(clippy::collapsible_match)]
#[allow(clippy::too_many_arguments)]
fn select_snippet_inner(
    descriptions: &[String],
    commands: &[String],
    tags: &[Vec<String>],
    is_search: bool,
    initial_filter: Option<&str>,
    _folders: &[Vec<String>],
    favorites: &[bool],
    snippets: &[crate::library::Snippet],
) -> io::Result<Option<(usize, Option<String>)>> {
    // Enable mouse capture before initializing terminal
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::init();

    // Pre-compute syntax-highlighted commands once at startup (not inside draw loop)
    // This avoids the closure-capture issues that cause TUI to hang
    let highlighted_commands: Vec<Line<'static>> =
        commands.iter().map(|cmd| highlight_command(cmd)).collect();

    let mut sel = SelectState::new();
    let mut filter = initial_filter.map(String::from).unwrap_or_default();
    let mut incremental_search = String::new();
    // input_text tracks what user types in insert mode - displayed in filter input box, NOT in title bar
    let mut input_text = String::new();
    let mut filter_state = FilterState::default();
    let mut filtered: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut insert_mode = true;
    let mut tag_filter_mode = false;
    let mut list_display_mode = 0;
    let mut should_copy: Option<String> = None;
    let mut copied_message: Option<(String, std::time::Instant)> = None;
    let mut visual_mode = false;
    let mut visual_start = 0usize;
    let mut visual_end = 0usize;
    let mut pending_gg = false;
    let mut pending_gg_time: Option<std::time::Instant> = None;
    const GG_TIMEOUT_MS: u64 = 500;

    // Debounce filter updates to avoid fuzzy matching on every keystroke
    let mut filter_dirty = false;
    let mut last_filter_update: Option<std::time::Instant> = None;
    const FILTER_DEBOUNCE_MS: u64 = 150;

    // Mouse double-click tracking
    let mut last_click_row: Option<u16> = None;
    let mut last_click_time: Option<std::time::Instant> = None;
    const DOUBLE_CLICK_DURATION_MS: u64 = 500;

    let all_display: Vec<String> = descriptions
        .iter()
        .enumerate()
        .map(|(i, _)| format!("[{}]: {}", descriptions[i], commands[i]))
        .collect();

    let all_tags = tags.to_vec();

    loop {
        // Debounce: only recompute filtered list if enough time has passed since last filter change
        let debounce_elapsed = last_filter_update.map_or(true, |t| {
            t.elapsed().as_millis() >= FILTER_DEBOUNCE_MS as u128
        });
        let should_recompute = if filter_dirty {
            // Always recompute immediately when filter becomes empty (backspace cleared filter)
            // to avoid 150ms delay showing stale filtered results
            let filter_is_empty = filter.is_empty() && filter_state.tag_filter_text.is_empty();
            if filter_is_empty || debounce_elapsed {
                filter_dirty = false;
                true
            } else {
                // Filter changed but debounce window not elapsed, skip this frame
                false
            }
        } else {
            // No filter change, use previous results
            false
        };

        let has_incremental_search = !incremental_search.is_empty();
        let has_main_filter = !filter.is_empty() || !filter_state.tag_filter_text.is_empty();
        let current_filter_text = if tag_filter_mode {
            filter_state.tag_filter_text.clone()
        } else {
            filter.clone()
        };

        // should_recompute = false means either: no change, or debounce window not elapsed yet
        // In both cases, reuse previous filtered results. Only build fresh from all_display
        // when we actually need to recompute (should_recompute = true).
        let mut candidates: Vec<(usize, String, Vec<String>, Option<i64>)> = if !should_recompute {
            if filtered.is_empty() {
                // First frame or no previous results
                all_display
                    .iter()
                    .enumerate()
                    .zip(all_tags.iter())
                    .map(|((i, d), t)| (i, d.clone(), t.clone(), None))
                    .collect()
            } else {
                filtered
                    .iter()
                    .map(|(i, d, t)| (*i, d.clone(), t.clone(), None))
                    .collect()
            }
        } else {
            all_display
                .iter()
                .enumerate()
                .zip(all_tags.iter())
                .map(|((i, d), t)| (i, d.clone(), t.clone(), None))
                .collect()
        };

        if has_incremental_search {
            candidates = candidates
                .into_iter()
                .filter_map(|(i, display, tags, _)| {
                    MATCHER
                        .fuzzy_match(&display, &incremental_search)
                        .map(|score| {
                            let is_exact =
                                display.to_lowercase() == incremental_search.to_lowercase();
                            (
                                i,
                                display,
                                tags,
                                Some(if is_exact { i64::MAX } else { score }),
                            )
                        })
                })
                .collect();
        }

        if !has_incremental_search && has_main_filter {
            let filter_text = &current_filter_text;
            let filter_lower = filter_text.to_lowercase();

            candidates = candidates
                .into_iter()
                .filter_map(|(i, display, snippet_tags, _)| {
                    let display_match = MATCHER.fuzzy_match(&display, filter_text);
                    let is_exact_display = display.to_lowercase() == filter_lower;
                    let tag_match = if tag_filter_mode || !filter_state.tag_filter_text.is_empty() {
                        snippet_tags
                            .iter()
                            .any(|t| t.to_lowercase().contains(&filter_lower))
                    } else {
                        false
                    };

                    if is_exact_display || tag_match {
                        Some((i, display, snippet_tags, Some(i64::MAX)))
                    } else {
                        display_match.map(|score| (i, display, snippet_tags, Some(score)))
                    }
                })
                .collect();
        }

        let has_filter = has_incremental_search || has_main_filter;
        candidates.sort_by(|a, b| {
            let score_cmp = match (a.3, b.3) {
                (Some(sa), Some(sb)) => sb.cmp(&sa),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };

            if score_cmp != std::cmp::Ordering::Equal || !has_filter {
                let explicit_sort = match filter_state.sort_mode {
                    SortMode::Newest => Some(b.0.cmp(&a.0)),
                    SortMode::Oldest => Some(a.0.cmp(&b.0)),
                    _ => None,
                };

                let secondary = match filter_state.sort_mode {
                    SortMode::AlphaAsc => Some(a.1.to_lowercase().cmp(&b.1.to_lowercase())),
                    SortMode::AlphaDesc => Some(b.1.to_lowercase().cmp(&a.1.to_lowercase())),
                    _ => None,
                };

                match explicit_sort {
                    Some(c) if c != std::cmp::Ordering::Equal => c,
                    _ => secondary.unwrap_or(score_cmp),
                }
            } else {
                score_cmp
            }
        });

        filtered = candidates
            .into_iter()
            .map(|(i, d, t, _)| (i, d, t))
            .collect();

        sel.update(filtered.len());

        // Filter indicator in title bar - ONLY shows incremental search (/), NOT the main filter.
        // Main filter text is displayed in the filter input box below, not in the title.
        // This ensures the input field position remains stable and text appears in the correct location.
        let filter_indicator = if tag_filter_mode {
            format!("[tag: {}]", filter_state.tag_filter_text)
        } else if !insert_mode && !incremental_search.is_empty() {
            format!("/{}", incremental_search)
        } else {
            String::new()
        };

        let sort_indicator = match filter_state.sort_mode {
            SortMode::None => String::new(),
            SortMode::Newest => "[new]".to_string(),
            SortMode::Oldest => "[old]".to_string(),
            SortMode::AlphaAsc => "[a-z]".to_string(),
            SortMode::AlphaDesc => "[z-a]".to_string(),
        };

        let _ = terminal.draw(|f| {
            let size = f.area();
            if size.width < 10 || size.height < 10 {
                let error_msg = "Terminal too small - resize to at least 10x10";
                let paragraph = Paragraph::new(error_msg)
                    .centered()
                    .block(Block::default().title("Error").borders(Borders::ALL));
                f.render_widget(paragraph, size);
                return;
            }
            let count = filtered.len();
            let title_part = format!("Snippets [{count}] {}{}", filter_indicator, sort_indicator);
            let separator = "─".repeat((size.width as usize).saturating_sub(title_part.len() + 8));
            let theme = get_theme();
            let block = Block::default()
                .title(format!("{title_part} {separator}"))
                .borders(Borders::ALL)
                .border_style(style_fg(theme.border))
                .style(TuiStyle::default().bg(theme.background));
            let input_block_title = if tag_filter_mode {
                "Tag Filter"
            } else {
                "Filter"
            };
            let input_block = Block::default()
                .title(input_block_title)
                .borders(Borders::ALL)
                .border_style(style_fg(theme.border))
                .style(TuiStyle::default().bg(theme.background));
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(3),
                    Constraint::Length(6),
                    Constraint::Length(1),
                ])
                .split(size);
            // Use pre-computed highlights (computed once at startup, outside draw loop)
            let items: Vec<ListItem> = filtered
                .iter()
                .map(|(idx, _desc, _tags)| {
                    let is_fav = *favorites.get(*idx).unwrap_or(&false);
                    let fav_indicator = if is_fav { "★ " } else { "  " };

                    let line = if list_display_mode == 1 {
                        let desc = &descriptions[*idx];
                        let cmd_spans = highlighted_commands
                            .get(*idx)
                            .cloned()
                            .map(|line| line.spans)
                            .unwrap_or_default();

                        let mut combined: Vec<ratatui::text::Span<'static>> =
                            vec![ratatui::text::Span::styled(
                                format!("[{}] ", desc),
                                style_fg(theme.text),
                            )];
                        combined.extend(cmd_spans);
                        Line::from(combined)
                    } else {
                        highlighted_commands
                            .get(*idx)
                            .cloned()
                            .unwrap_or_else(|| Line::from(""))
                    };

                    let mut spans = vec![ratatui::text::Span::raw(fav_indicator)];
                    spans.extend(line.spans);
                    let final_line = Line::from(spans);

                    ListItem::new(final_line)
                })
                .collect();
            let list = List::new(items)
                .block(block)
                .style(TuiStyle::default().bg(theme.background))
                .highlight_style(
                    ratatui::style::Style::default()
                        .fg(theme.text)
                        .bg(theme.selected_bg),
                )
                .highlight_symbol(if is_search { "" } else { "▶ " });

            f.render_widget(input_block, chunks[0]);
            // Filter text displayed in the filter input box (NOT in title bar).
            // Insert mode: show input_text (what user is typing)
            // Normal mode: show filter (applied filter)
            // Tag filter mode: show tag_filter_text
            // DO NOT move this text to the title bar - it belongs in the filter input box.
            let filter_text = if tag_filter_mode {
                if filter_state.tag_filter_text.is_empty() {
                    ""
                } else {
                    &filter_state.tag_filter_text
                }
            } else if insert_mode {
                &input_text
            } else {
                &filter
            };
            let filter_widget = Paragraph::new(filter_text)
                .style(style_fg_bg(theme.text, theme.background));
            f.render_widget(
                filter_widget,
                ratatui::layout::Rect::new(
                    chunks[0].x + 1,
                    chunks[0].y + 1,
                    chunks[0].width - 2,
                    1,
                ),
            );

            if insert_mode {
                let cursor_x = chunks[0].x
                    + 1
                    + input_text.len() as u16;
                let cursor_y = chunks[0].y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            f.render_stateful_widget(list, chunks[1], &mut sel.list_state);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(ratatui::style::Style::default().bg(ratatui::style::Color::Cyan));
            f.render_stateful_widget(scrollbar, chunks[1], &mut sel.scroll_state);

            if sel.selected < filtered.len() {
                let idx = filtered[sel.selected].0;
                let snippet_cmd = &commands[idx];
                let snippet_desc = &descriptions[idx];
                let _snippet_tags = &tags[idx];

                let vars = extract_variables(snippet_cmd);
                let has_vars = !vars.is_empty();

                let preview_title = format!("Preview: {}", snippet_desc);
                let preview_block = Block::default()
                    .title(preview_title)
                    .borders(Borders::ALL)
                    .border_style(style_fg(theme.border))
                    .style(TuiStyle::default().bg(theme.background));

                let has_unmatched = has_unmatched_angle_bracket(snippet_cmd);
                let preview_content = if has_vars || has_unmatched {
                    let mut content = format!("{}\n\nVars: {}", strip_escape_sequences(snippet_cmd), vars.join(", "));
                    if has_unmatched {
                        content.push_str("\n\nWarning: unmatched '<' found - will be treated as literal");
                    }
                    content
                } else {
                    strip_escape_sequences(snippet_cmd)
                };

                let preview_widget = Paragraph::new(preview_content)
                    .block(preview_block)
                    .style(style_fg_bg(theme.text, theme.background));
                f.render_widget(preview_widget, chunks[2]);
            }

            let copied_desc = copied_message.as_ref().map(|(desc, _)| desc.clone());
            let mode_str = if insert_mode { "INS" } else { "NOR" };
            let tag_mode_str = if tag_filter_mode { " TAG" } else { "" };

            let status_text: String = if let Some(desc) = copied_desc {
                format!("[{}]{} | {}", mode_str, tag_mode_str, desc)
            } else if is_search {
                if insert_mode {
                    format!(
                        "[{}]{} | esc: normal | /: search | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear | tab: mode",
                        mode_str, tag_mode_str
                    )
                } else {
                    format!(
                        "[{}]{} | i: insert | y/ctrl+c: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear | tab: mode",
                        mode_str, tag_mode_str
                    )
                }
            } else if insert_mode {
                format!(
                    "[{}]{} | esc: normal | enter: run | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear | tab: mode",
                    mode_str, tag_mode_str
                )
            } else {
                format!(
                    "[{}]{} | i: insert | y: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear | tab: mode | double-click: run",
                    mode_str, tag_mode_str
                )
            };
            let status_widget = Paragraph::new(status_text)
                .style(style_fg(theme.muted));
            let status_area = ratatui::layout::Rect::new(
                chunks[3].x + 1,
                chunks[3].y,
                chunks[3].width - 2,
                1,
            );
            f.render_widget(status_widget, status_area);
        });

        // Get terminal size for mouse click detection
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let terminal_size = ratatui::layout::Rect::new(0, 0, cols, rows);

        // Calculate list area for mouse click detection (same layout as in draw closure)
        let list_area = {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(3),
                    Constraint::Length(6),
                    Constraint::Length(1),
                ])
                .split(terminal_size);
            chunks[1]
        };

        if event::poll(Duration::from_millis(200))? {
            match event::read() {
                Ok(CEvent::Mouse(mouse_event)) => {
                    // Check for scroll events
                    if mouse_event.kind == crossterm::event::MouseEventKind::ScrollDown {
                        sel.move_down(filtered.len());
                    } else if mouse_event.kind == crossterm::event::MouseEventKind::ScrollUp {
                        sel.move_up();
                    }
                    // Handle click to select in list area
                    else if let crossterm::event::MouseEventKind::Up(
                        crossterm::event::MouseButton::Left,
                    ) = mouse_event.kind
                    {
                        if mouse_event.row >= list_area.y
                            && mouse_event.row < list_area.y + list_area.height
                            && mouse_event.row
                                < list_area.y + filtered.len().min(list_area.height as usize) as u16
                        {
                            let clicked_row = (mouse_event.row - list_area.y) as usize;

                            // Check for double-click (same row within time window)
                            let now = std::time::Instant::now();
                            let is_double_click = last_click_row == Some(mouse_event.row)
                                && last_click_time
                                    .map(|t| {
                                        now.duration_since(t).as_millis()
                                            < DOUBLE_CLICK_DURATION_MS as u128
                                    })
                                    .unwrap_or(false);

                            if is_double_click {
                                // Double-click: run selected snippet
                                break;
                            } else {
                                // Single click: select item
                                sel.selected = clicked_row;
                                last_click_row = Some(mouse_event.row);
                                last_click_time = Some(now);
                            }
                        } else {
                            // Clicked outside list, reset double-click state
                            last_click_row = None;
                            last_click_time = None;
                        }
                    }
                    sel.update(filtered.len());
                }
                Ok(CEvent::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if let Some((_, instant)) = copied_message {
                            if instant.elapsed().as_secs() >= 3 {
                                copied_message = None;
                            }
                        }

                        if is_ctrl_key(&key, 'c') && sel.selected < filtered.len() {
                            let idx = filtered[sel.selected].0;
                            let cmd = strip_escape_sequences(&commands[idx]);
                            if let Err(e) = clipboard::copy_to_clipboard_auto(&cmd) {
                                tracing::warn!("Clipboard copy failed: {}", e);
                            }
                            should_copy = Some(descriptions[idx].clone());
                            if !is_search {
                                break;
                            }
                        }

                        if key.code == KeyCode::Char('v') && !visual_mode {
                            visual_mode = true;
                            visual_start = sel.selected;
                            visual_end = sel.selected;
                            continue;
                        }

                        if key.code == KeyCode::Char('V') && !visual_mode {
                            visual_mode = true;
                            visual_start = sel.selected;
                            visual_end = filtered.len().saturating_sub(1);
                            sel.selected = visual_end;
                            continue;
                        }

                        if key.code == KeyCode::Esc && visual_mode {
                            visual_mode = false;
                            continue;
                        }

                        if visual_mode {
                            match key.code {
                                KeyCode::Char('j') | KeyCode::Down
                                    if sel.selected + 1 < filtered.len() =>
                                {
                                    if sel.selected < visual_end {
                                        sel.selected += 1;
                                    } else {
                                        visual_end = (sel.selected + 1)
                                            .min(filtered.len().saturating_sub(1));
                                        sel.selected = visual_end;
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up if sel.selected > 0 => {
                                    if sel.selected > visual_start {
                                        sel.selected -= 1;
                                    } else if visual_start > 0 {
                                        visual_start -= 1;
                                        sel.selected = visual_start;
                                    }
                                }
                                KeyCode::Char('y') => {
                                    let start = std::cmp::min(visual_start, visual_end)
                                        .min(filtered.len().saturating_sub(1));
                                    let end = std::cmp::max(visual_start, visual_end)
                                        .min(filtered.len().saturating_sub(1));
                                    let selected_items: Vec<String> = filtered
                                        .iter()
                                        .skip(start)
                                        .take(end - start + 1)
                                        .map(|(idx, _, _)| strip_escape_sequences(&commands[*idx]))
                                        .collect();
                                    let copy_text = selected_items.join("\n");
                                    if let Err(e) = clipboard::copy_to_clipboard_auto(&copy_text) {
                                        tracing::warn!("Clipboard copy failed: {}", e);
                                    }
                                    if let Some((idx, _, _)) = filtered.get(start) {
                                        if let Err(e) =
                                            crate::logging::audit_log("copy", &snippets[*idx], None)
                                        {
                                            tracing::debug!("Audit log write failed: {}", e);
                                        }
                                    }
                                    should_copy =
                                        Some(format!("{} snippets copied", end - start + 1));
                                    visual_mode = false;
                                    if !is_search {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            sel.update(filtered.len());
                            continue;
                        }

                        if is_ctrl_key(&key, 'f') {
                            sel.selected =
                                (sel.selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if is_ctrl_key(&key, 'd') {
                            sel.selected =
                                (sel.selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if is_ctrl_key(&key, 'b') {
                            sel.selected = sel.selected.saturating_sub(10);
                        }

                        if is_ctrl_key(&key, 'u') {
                            sel.selected = sel.selected.saturating_sub(10);
                        }

                        if key.code == KeyCode::PageDown {
                            sel.selected =
                                (sel.selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if key.code == KeyCode::PageUp {
                            sel.selected = sel.selected.saturating_sub(10);
                        }

                        if insert_mode {
                            match key.code {
                                KeyCode::Char('/') => {
                                    insert_mode = true;
                                    incremental_search.clear();
                                    input_text.clear();
                                    filter_dirty = true;
                                    last_filter_update = Some(std::time::Instant::now());
                                }
                                KeyCode::Esc => {
                                    if tag_filter_mode {
                                        tag_filter_mode = false;
                                    } else if !input_text.is_empty() {
                                        filter = input_text.clone();
                                        input_text.clear();
                                        insert_mode = false;
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else if !incremental_search.is_empty() {
                                        incremental_search.clear();
                                        insert_mode = false;
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else {
                                        insert_mode = false;
                                    }
                                }
                                KeyCode::Down if sel.selected + 1 < filtered.len() => {
                                    sel.move_down(filtered.len())
                                }
                                KeyCode::Up if sel.selected > 0 => sel.move_up(),
                                KeyCode::Enter => {
                                    break;
                                }
                                KeyCode::Backspace => {
                                    if tag_filter_mode {
                                        filter_state.tag_filter_text.pop();
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else if insert_mode {
                                        input_text.pop();
                                        filter.pop();
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else if !incremental_search.is_empty() {
                                        incremental_search.pop();
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else {
                                        filter.pop();
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    }
                                    if !filtered.is_empty() {
                                        sel.move_to_top();
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if tag_filter_mode {
                                        filter_state.tag_filter_text.push(c);
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else if insert_mode {
                                        input_text.push(c);
                                        filter.push(c);
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else if !incremental_search.is_empty() {
                                        incremental_search.push(c);
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    } else {
                                        filter.push(c);
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    }
                                    if !filtered.is_empty() {
                                        sel.move_to_top();
                                    }
                                }
                                KeyCode::Tab => {
                                    list_display_mode = (list_display_mode + 1) % 2;
                                }
                                _ => {}
                            }
                        } else {
                            if key.code != KeyCode::Char('g') && !is_ctrl_key(&key, 'g') {
                                pending_gg = false;
                            }
                            match key.code {
                                KeyCode::Char('q') => {
                                    sel.selected = filtered.len();
                                    break;
                                }
                                KeyCode::Enter => {
                                    // Run selected snippet (exit loop with selection)
                                    break;
                                }
                                KeyCode::Char('i') => {
                                    insert_mode = true;
                                    input_text = filter.clone();
                                }
                                KeyCode::Char('p') => {}
                                KeyCode::Char('y') => {
                                    if sel.selected < filtered.len() {
                                        let idx = filtered[sel.selected].0;
                                        let cmd = strip_escape_sequences(&commands[idx]);
                                        if let Err(e) = clipboard::copy_to_clipboard_auto(&cmd) {
                                            tracing::warn!("Clipboard copy failed: {}", e);
                                        }
                                        should_copy = Some(descriptions[idx].clone());
                                        if !is_search {
                                            break;
                                        }
                                    }
                                }
                                KeyCode::Tab => {
                                    list_display_mode = (list_display_mode + 1) % 2;
                                }
                                KeyCode::Char('g') if is_ctrl_key(&key, 'g') => {
                                    sel.move_to_top();
                                    pending_gg = false;
                                }
                                KeyCode::Char('g') if pending_gg => {
                                    let now = std::time::Instant::now();
                                    let timed_out = pending_gg_time
                                        .map(|t| {
                                            now.duration_since(t).as_millis()
                                                > GG_TIMEOUT_MS as u128
                                        })
                                        .unwrap_or(true);
                                    if timed_out {
                                        pending_gg = true;
                                        pending_gg_time = Some(now);
                                    } else {
                                        sel.move_to_top();
                                        pending_gg = false;
                                    }
                                }
                                KeyCode::Char('g') => {
                                    pending_gg = true;
                                    pending_gg_time = Some(std::time::Instant::now());
                                }
                                KeyCode::Char('G') => sel.move_to_bottom(filtered.len()),
                                KeyCode::Char('h') | KeyCode::Left if sel.selected > 0 => {
                                    sel.move_up()
                                }
                                KeyCode::Char('j') | KeyCode::Down
                                    if sel.selected + 1 < filtered.len() =>
                                {
                                    sel.move_down(filtered.len())
                                }
                                KeyCode::Char('k') | KeyCode::Up if sel.selected > 0 => {
                                    sel.move_up()
                                }
                                KeyCode::Char('l') | KeyCode::Right
                                    if sel.selected + 1 < filtered.len() =>
                                {
                                    sel.move_down(filtered.len())
                                }
                                KeyCode::Char('n') => filter_state.toggle_sort_new(),
                                KeyCode::Char('o') => filter_state.toggle_sort_old(),
                                KeyCode::Char('a') => filter_state.toggle_sort_alpha(),
                                KeyCode::Char('z') => filter_state.toggle_sort_alpha_rev(),
                                KeyCode::Char('x') | KeyCode::Char('c') => {
                                    filter.clear();
                                    incremental_search.clear();
                                    filter_dirty = true;
                                    last_filter_update = Some(std::time::Instant::now());
                                }
                                KeyCode::Char('t') => {
                                    tag_filter_mode = !tag_filter_mode;
                                    if tag_filter_mode {
                                        filter_state.tag_filter_text.clear();
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
                                    }
                                }
                                KeyCode::Char('/') => {
                                    insert_mode = true;
                                    incremental_search.clear();
                                    filter_dirty = true;
                                    last_filter_update = Some(std::time::Instant::now());
                                }
                                KeyCode::Esc => {
                                    // Esc no longer quits - use q or Ctrl+C instead
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(CEvent::Resize(_, _)) => {
                    // Terminal resize - redraw will happen on next iteration
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Event read error: {}", e);
                }
            }
        }
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    Ok(if sel.selected < filtered.len() {
        Some((filtered[sel.selected].0, should_copy))
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_state_toggle_sort_new() {
        let mut state = FilterState::default();
        assert_eq!(state.sort_mode, SortMode::None);

        state.toggle_sort_new();
        assert_eq!(state.sort_mode, SortMode::Newest);

        state.toggle_sort_new();
        assert_eq!(state.sort_mode, SortMode::None);
    }

    #[test]
    fn test_filter_state_toggle_sort_alpha() {
        let mut state = FilterState::default();
        assert_eq!(state.sort_mode, SortMode::None);

        state.toggle_sort_alpha();
        assert_eq!(state.sort_mode, SortMode::AlphaAsc);

        state.toggle_sort_alpha_rev();
        assert_eq!(state.sort_mode, SortMode::AlphaDesc);
    }

    #[test]
    fn test_is_ctrl_key_true() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('c'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(is_ctrl_key(&key, 'c'));
    }

    #[test]
    fn test_is_ctrl_key_false_no_modifier() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('c'),
            modifiers: crossterm::event::KeyModifiers::empty(),
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(!is_ctrl_key(&key, 'c'));
    }

    #[test]
    fn test_is_ctrl_key_false_different_char() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(!is_ctrl_key(&key, 'c'));
    }

    #[test]
    fn test_terminate_functionality() {
        let terminate = get_terminate();
        terminate.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!terminate.load(std::sync::atomic::Ordering::SeqCst));

        terminate.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(terminate.load(std::sync::atomic::Ordering::SeqCst));
    }
}

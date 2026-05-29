//! TUI module for the snp snippet manager.
//!
//! ## Critical Implementation Notes
//!
//! ### List Item Rendering (2024-03-04)
//! **IMPORTANT**: When rendering list items in the TUI, be careful with closures that capture
//! external variables. The `.map()` closure used to build `ListItem` widgets must NOT call
//! expensive functions like `highlight_command()` that cause terminal.draw() to hang.
//!
//! **Solution: Pre-compute highlights outside the draw loop**
//!
//! ```ignore
//! // At the start of select_snippet_inner(), pre-compute all highlights:
//! let highlighted_commands: Vec<Line<'static>> = commands
//!     .iter()
//!     .map(|cmd| highlight_command(cmd))
//!     .collect();
//!
//! // Then inside the draw loop, use the pre-computed highlights:
//! let items: Vec<ListItem> = filtered.iter().map(|(idx, _desc, _tags)| {
//!     let highlighted = highlighted_commands.get(*idx).cloned().unwrap_or_else(|| Line::from(""));
//!     // ... use highlighted content
//! }).collect();
//! ```
//!
//! This approach works because:
//! 1. Highlights are computed once at startup (not on every frame)
//! 2. The draw closure just looks up pre-computed values (no expensive computation)
//! 3. No complex closure captures that could cause deadlocks
//!
//! ### Theme Colors
//! All widgets must have explicit background colors set via `.style(Style::default().bg(theme.background))`
//! to ensure visibility. Using `Color::Reset` is not reliable across all terminals.

use std::io;
use std::sync::LazyLock;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::text::{Line, Span};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};

use crate::clipboard;
use crate::utils::extract_variables_for_display;
use crate::utils::shell_keywords::SHELL_KEYWORDS_SET;
use crate::utils::strip_escape_sequences;

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

pub use crate::utils::variables::Variable;

/// Result of prompting the user for variable values.
pub enum VariablePromptResult {
    /// User cancelled the operation (pressed q).
    Cancel,
    /// User chose to skip (pressed enter with defaults).
    Skip,
    /// User provided variable values.
    Values(Vec<(String, String)>),
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub primary: ratatui::style::Color,
    pub secondary: ratatui::style::Color,
    pub accent: ratatui::style::Color,
    pub background: ratatui::style::Color,
    pub text: ratatui::style::Color,
    pub border: ratatui::style::Color,
    pub selected_bg: ratatui::style::Color,
    pub muted: ratatui::style::Color,
}

const DARK_THEME: Theme = Theme {
    primary: ratatui::style::Color::Blue,
    secondary: ratatui::style::Color::Cyan,
    accent: ratatui::style::Color::Yellow,
    background: ratatui::style::Color::Black,
    text: ratatui::style::Color::White,
    border: ratatui::style::Color::Cyan,
    selected_bg: ratatui::style::Color::Blue,
    muted: ratatui::style::Color::Gray,
};

const BRIGHT_THEME: Theme = Theme {
    primary: ratatui::style::Color::Blue,
    secondary: ratatui::style::Color::Blue,
    accent: ratatui::style::Color::Magenta,
    background: ratatui::style::Color::White,
    text: ratatui::style::Color::Black,
    border: ratatui::style::Color::Blue,
    selected_bg: ratatui::style::Color::LightBlue,
    muted: ratatui::style::Color::Gray,
};

fn resolve_theme(theme_name: &str) -> Theme {
    match theme_name {
        "bright" | "light" => BRIGHT_THEME,
        "dark" => DARK_THEME,
        "auto" => {
            if std::env::var("COLORFGBG")
                .map(|v| v.starts_with("15;") || v.starts_with("7;"))
                .unwrap_or(false)
            {
                BRIGHT_THEME
            } else {
                DARK_THEME
            }
        }
        _ => DARK_THEME,
    }
}

static ACTIVE_THEME: LazyLock<std::sync::Mutex<Theme>> = LazyLock::new(|| {
    std::sync::Mutex::new({
        let theme_name = std::env::var("SNP_THEME").unwrap_or_else(|_| "auto".to_string());
        resolve_theme(&theme_name)
    })
});

fn style_fg(fg: ratatui::style::Color) -> TuiStyle {
    TuiStyle::default().fg(fg)
}

fn style_fg_bg(fg: ratatui::style::Color, bg: ratatui::style::Color) -> TuiStyle {
    TuiStyle::default().fg(fg).bg(bg)
}

pub fn get_theme() -> std::sync::MutexGuard<'static, Theme> {
    ACTIVE_THEME.lock().unwrap()
}

fn extract_variables(command: &str) -> Vec<String> {
    extract_variables_for_display(command)
}

fn highlight_command(command: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = command.chars().peekable();
    let mut prev_was_backslash = false;

    let theme = get_theme();
    let color_default = style_fg(theme.text);
    let color_variable = style_fg(theme.accent);
    let color_keyword = style_fg(theme.primary);
    let color_string = style_fg(ratatui::style::Color::Green);
    let color_flag = style_fg(theme.secondary);
    let color_comment = style_fg(theme.muted);
    let color_escape = style_fg(ratatui::style::Color::Magenta);

    let shell_keywords = &*SHELL_KEYWORDS_SET;

    while let Some(c) = chars.next() {
        if prev_was_backslash {
            prev_was_backslash = false;
            if c == '<' || c == '>' {
                spans.push(Span::styled(c.to_string(), color_default));
            } else {
                spans.push(Span::styled(format!("\\{}", c), color_escape));
            }
            continue;
        }

        if c == '\\' {
            prev_was_backslash = true;
            continue;
        }

        if c == '<' {
            let mut var_content = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                var_content.push(chars.next().unwrap());
            }
            spans.push(Span::styled(format!("<{}>", var_content), color_variable));
            continue;
        }

        if c == '#' && spans.last().map(|s| s.content.is_empty()).unwrap_or(true) {
            let mut comment = String::from(c);
            for next in chars.by_ref() {
                comment.push(next);
            }
            spans.push(Span::styled(comment, color_comment));
        } else if c == '"' || c == '\'' {
            let quote = c;
            let mut string_content = String::new();
            string_content.push(c);
            for next in chars.by_ref() {
                string_content.push(next);
                if next == quote {
                    break;
                }
            }
            spans.push(Span::styled(string_content, color_string));
        } else if c == '-' && chars.peek().map(|&c| c == '-').unwrap_or(false) {
            let mut flag = String::from(c);
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || next == '-' || next == '=' {
                    flag.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            spans.push(Span::styled(flag, color_flag));
        } else if c == '-' {
            let mut flag = String::from(c);
            if let Some(&next) = chars.peek() {
                if next.is_alphabetic() {
                    flag.push(chars.next().unwrap());
                }
            }
            spans.push(Span::styled(flag, color_flag));
        } else if c.is_whitespace() {
            let mut ws = String::from(c);
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    ws.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            spans.push(Span::styled(ws, color_default));
        } else {
            let mut word = String::new();
            word.push(c);
            while let Some(&next) = chars.peek() {
                if next.is_alphanumeric() || next == '_' || next == '-' || next == '.' {
                    word.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            let is_kw = shell_keywords.contains(word.as_str());
            let style = if is_kw { color_keyword } else { color_default };
            spans.push(Span::styled(word, style));
        }
    }

    Line::from(spans)
}

pub fn select_snippet(
    descriptions: &[String],
    commands: &[String],
    tags: &[Vec<String>],
    is_search: bool,
    initial_filter: Option<&str>,
    folders: &[Vec<String>],
    favorites: &[bool],
) -> io::Result<Option<(usize, Option<String>)>> {
    select_snippet_inner(
        descriptions,
        commands,
        tags,
        is_search,
        initial_filter,
        folders,
        favorites,
    )
}

fn select_snippet_inner(
    descriptions: &[String],
    commands: &[String],
    tags: &[Vec<String>],
    is_search: bool,
    initial_filter: Option<&str>,
    _folders: &[Vec<String>],
    favorites: &[bool],
) -> io::Result<Option<(usize, Option<String>)>> {
    // Enable mouse capture before initializing terminal
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::init();

    // Pre-compute syntax-highlighted commands once at startup (not inside draw loop)
    // This avoids the closure-capture issues that cause TUI to hang
    let highlighted_commands: Vec<Line<'static>> =
        commands.iter().map(|cmd| highlight_command(cmd)).collect();

    let mut state = ratatui::widgets::ListState::default();
    let mut filter = initial_filter.map(String::from).unwrap_or_default();
    let mut incremental_search = String::new();
    // input_text tracks what user types in insert mode - displayed in filter input box, NOT in title bar
    let mut input_text = String::new();
    let mut filter_state = FilterState::default();
    let mut selected = 0usize;
    let mut filtered: Vec<(usize, String, Vec<String>)> = Vec::new();
    let mut insert_mode = true;
    let mut tag_filter_mode = false;
    let mut list_display_mode = 0;
    let mut scrollbar_state = ScrollbarState::default();
    let mut should_copy: Option<String> = None;
    let mut copied_message: Option<(String, std::time::Instant)> = None;
    let mut visual_mode = false;
    let mut visual_start = 0usize;
    let mut visual_end = 0usize;

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

    state.select(Some(0));

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

        if filtered.is_empty() {
            selected = 0;
        } else if selected >= filtered.len() {
            selected = filtered.len().saturating_sub(1);
        }
        state.select(Some(selected));
        scrollbar_state = scrollbar_state
            .content_length(filtered.len())
            .position(selected);

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

                        let mut combined: Vec<Span<'static>> =
                            vec![Span::styled(format!("[{}] ", desc), style_fg(theme.text))];
                        combined.extend(cmd_spans);
                        Line::from(combined)
                    } else {
                        highlighted_commands
                            .get(*idx)
                            .cloned()
                            .unwrap_or_else(|| Line::from(""))
                    };

                    let mut spans = vec![Span::raw(fav_indicator)];
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

            f.render_stateful_widget(list, chunks[1], &mut state);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_style(ratatui::style::Style::default().bg(ratatui::style::Color::Cyan));
            f.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);

            if selected < filtered.len() {
                let idx = filtered[selected].0;
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

                let preview_content = if has_vars {
                    format!("{}\n\nVars: {}", strip_escape_sequences(snippet_cmd), vars.join(", "))
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
                    "[{}]{} | i: insert | y: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear | tab: mode",
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
                        if selected + 1 < filtered.len() {
                            selected += 1;
                        }
                    } else if mouse_event.kind == crossterm::event::MouseEventKind::ScrollUp {
                        selected = selected.saturating_sub(1);
                    }
                    // Handle click to select in list area
                    else if let crossterm::event::MouseEventKind::Up(
                        crossterm::event::MouseButton::Left,
                    ) = mouse_event.kind
                    {
                        if mouse_event.row >= list_area.y
                            && mouse_event.row < list_area.y + list_area.height
                            && mouse_event.row < list_area.y + filtered.len() as u16
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
                                selected = clicked_row;
                                last_click_row = Some(mouse_event.row);
                                last_click_time = Some(now);
                            }
                        } else {
                            // Clicked outside list, reset double-click state
                            last_click_row = None;
                            last_click_time = None;
                        }
                    }
                    state.select(Some(selected));
                }
                Ok(CEvent::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if let Some((_, instant)) = copied_message {
                            if instant.elapsed().as_secs() >= 3 {
                                copied_message = None;
                            }
                        }

                        if is_ctrl_key(&key, 'c') && selected < filtered.len() {
                            let idx = filtered[selected].0;
                            let cmd = strip_escape_sequences(&commands[idx]);
                            let _ = clipboard::copy_to_clipboard_auto(&cmd);
                            should_copy = Some(descriptions[idx].clone());
                            if !is_search {
                                break;
                            }
                        }

                        if key.code == KeyCode::Char('v') && !visual_mode {
                            visual_mode = true;
                            visual_start = selected;
                            visual_end = selected;
                            continue;
                        }

                        if key.code == KeyCode::Char('V') && !visual_mode {
                            visual_mode = true;
                            visual_start = selected;
                            visual_end = filtered.len().saturating_sub(1);
                            continue;
                        }

                        if key.code == KeyCode::Esc && visual_mode {
                            visual_mode = false;
                            continue;
                        }

                        if visual_mode {
                            match key.code {
                                KeyCode::Char('j') | KeyCode::Down
                                    if selected + 1 < filtered.len() =>
                                {
                                    if selected < visual_end {
                                        selected += 1;
                                    } else {
                                        visual_end += 1;
                                        selected = visual_end;
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up if selected > 0 => {
                                    if selected > visual_start {
                                        selected -= 1;
                                    } else if visual_start > 0 {
                                        visual_start -= 1;
                                        selected = visual_start;
                                    }
                                }
                                KeyCode::Char('y') => {
                                    let start = std::cmp::min(visual_start, visual_end);
                                    let end = std::cmp::max(visual_start, visual_end);
                                    let selected_items: Vec<&str> = filtered
                                        .iter()
                                        .skip(start)
                                        .take(end - start + 1)
                                        .map(|(_, desc, _)| desc.as_str())
                                        .collect();
                                    let copy_text = selected_items.join("\n");
                                    let _ = clipboard::copy_to_clipboard_auto(&copy_text);
                                    should_copy =
                                        Some(format!("{} snippets copied", end - start + 1));
                                    visual_mode = false;
                                    if !is_search {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                            state.select(Some(selected));
                            continue;
                        }

                        if is_ctrl_key(&key, 'f') {
                            selected = (selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if is_ctrl_key(&key, 'd') {
                            selected = (selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if is_ctrl_key(&key, 'b') {
                            selected = selected.saturating_sub(10);
                        }

                        if is_ctrl_key(&key, 'u') {
                            selected = selected.saturating_sub(10);
                        }

                        if key.code == KeyCode::PageDown {
                            selected = (selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if key.code == KeyCode::PageUp {
                            selected = selected.saturating_sub(10);
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
                                KeyCode::Down if selected + 1 < filtered.len() => selected += 1,
                                KeyCode::Up if selected > 0 => selected -= 1,
                                KeyCode::Enter => {
                                    if !is_search {
                                        break;
                                    }
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
                                        selected = 0;
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
                                        selected = 0;
                                    }
                                }
                                KeyCode::Tab => {
                                    list_display_mode = (list_display_mode + 1) % 2;
                                }
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Char('q') => {
                                    selected = filtered.len();
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
                                    if selected < filtered.len() {
                                        let idx = filtered[selected].0;
                                        let cmd = strip_escape_sequences(&commands[idx]);
                                        let _ = clipboard::copy_to_clipboard_auto(&cmd);
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
                                    selected = 0;
                                }
                                KeyCode::Char('G') => selected = filtered.len().saturating_sub(1),
                                KeyCode::Char('h') | KeyCode::Left if selected > 0 => selected -= 1,
                                KeyCode::Char('j') | KeyCode::Down
                                    if selected + 1 < filtered.len() =>
                                {
                                    selected += 1
                                }
                                KeyCode::Char('k') | KeyCode::Up if selected > 0 => selected -= 1,
                                KeyCode::Char('l') | KeyCode::Right
                                    if selected + 1 < filtered.len() =>
                                {
                                    selected += 1
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
                _ => {}
            }
        }
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();
    Ok(if selected < filtered.len() {
        Some((filtered[selected].0, should_copy))
    } else {
        None
    })
}

pub fn prompt_variables(vars: Vec<Variable>) -> io::Result<VariablePromptResult> {
    if vars.is_empty() {
        return Ok(VariablePromptResult::Skip);
    }

    prompt_variables_inner(vars)
}

fn prompt_variables_inner(vars: Vec<Variable>) -> io::Result<VariablePromptResult> {
    let mut terminal = ratatui::init();

    let defaults: Vec<String> = vars
        .iter()
        .map(|v| v.default.clone().unwrap_or_default())
        .collect();

    let mut values: Vec<String> = defaults.clone();
    let mut selected = 0usize;
    let mut mode = 1usize;
    let mut show_defaults = true;

    loop {
        terminal.draw(|f| {
            let size = f.area();
            f.render_widget(Clear, size);
            if size.width < 10 || size.height < 10 {
                let error_msg = "Terminal too small - resize to at least 10x10";
                let paragraph = Paragraph::new(error_msg)
                    .centered()
                    .block(Block::default().title("Error").borders(Borders::ALL));
                f.render_widget(paragraph, size);
                return;
            }
            let theme = get_theme();
            let block = Block::default()
                .title("Enter variables")
                .borders(Borders::ALL)
                .style(style_fg(theme.border));

            let num_vars = values.len().min(10);
            let var_height = num_vars * 3;
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([Constraint::Length(var_height as u16), Constraint::Length(1)])
                .split(size);

            f.render_widget(block, size);

            for (i, var) in vars.iter().enumerate() {
                if i >= num_vars {
                    break;
                }

                let var_block = Block::default()
                    .title(var.name.as_str())
                    .borders(Borders::ALL)
                    .style(if i == selected {
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow)
                    } else {
                        ratatui::style::Style::default()
                    });

                let prefix = if i == selected { "▶ " } else { "  " };
                let display_value =
                    if show_defaults && values[i] == defaults[i] && !defaults[i].is_empty() {
                        format!("{} (default: {})", values[i], defaults[i])
                    } else if values[i].is_empty() {
                        "_".to_string()
                    } else {
                        values[i].clone()
                    };
                let text = format!("{}{}", prefix, display_value);

                let p = Paragraph::new(text)
                    .block(var_block)
                    .style(style_fg(theme.text));

                let var_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Length(3); num_vars])
                    .split(chunks[0]);

                f.render_widget(p, var_chunks[i]);
            }

            if mode == 1 && selected < values.len() {
                let var_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Length(3); num_vars])
                    .split(chunks[0]);
                let prefix_len = 2;
                let cursor_x =
                    var_chunks[selected].x + 1 + prefix_len + values[selected].len() as u16;
                let cursor_y = var_chunks[selected].y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            let status_text =
                "↑/↓/ j/k: move | tab: next | enter: save | esc: back | q: cancel | d: defaults";
            let warning_text = "Values are interpolated directly into shell commands. Do not enter untrusted input.";
            let status_widget = Paragraph::new(Line::from(vec![
                Span::styled(status_text, style_fg(theme.muted)),
                Span::raw("  "),
                Span::styled(warning_text, style_fg(ratatui::style::Color::Yellow)),
            ]));
            f.render_widget(status_widget, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(200))? {
            if let CEvent::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => {
                            let _ = crossterm::execute!(
                                std::io::stdout(),
                                crossterm::event::DisableMouseCapture
                            );
                            ratatui::restore();
                            return Ok(VariablePromptResult::Cancel);
                        }
                        KeyCode::Esc => {
                            // Esc no longer quits - use q instead
                        }
                        KeyCode::Up | KeyCode::Char('k') if selected > 0 => {
                            selected -= 1;
                            mode = 1;
                        }
                        KeyCode::Down | KeyCode::Char('j') if selected + 1 < values.len() => {
                            selected += 1;
                            mode = 1;
                        }
                        KeyCode::Tab => {
                            if selected + 1 < values.len() {
                                selected += 1;
                            } else {
                                selected = 0;
                            }
                            mode = 1;
                        }
                        KeyCode::Enter => {
                            if values[selected].is_empty() && !defaults[selected].is_empty() {
                                values[selected] = defaults[selected].clone();
                            }
                            break;
                        }
                        KeyCode::Backspace if mode == 1 => {
                            values[selected].pop();
                        }
                        KeyCode::Char('d') if mode == 1 => {
                            show_defaults = !show_defaults;
                        }
                        KeyCode::Char(c) if mode == 1 => {
                            if values[selected] == defaults[selected]
                                && !defaults[selected].is_empty()
                            {
                                values[selected] = String::new();
                            }
                            values[selected].push(c);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    ratatui::restore();

    let result: Vec<(String, String)> = vars
        .iter()
        .zip(values.iter())
        .map(|(v, val)| (v.name.clone(), val.trim().to_string()))
        .collect();
    Ok(VariablePromptResult::Values(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_command_empty() {
        let result = highlight_command("");
        assert_eq!(result.spans.len(), 0);
    }

    #[test]
    fn test_highlight_command_simple() {
        let result = highlight_command("echo hello");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_highlight_command_with_variable() {
        let result = highlight_command("ssh <user@host>");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_highlight_command_with_quotes() {
        let result = highlight_command("echo 'hello world'");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_highlight_command_with_flags() {
        let result = highlight_command("ls -la /home");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_highlight_command_with_comment() {
        let result = highlight_command("# this is a comment");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_highlight_command_with_escaped_char() {
        let result = highlight_command("\\<escaped");
        assert!(!result.spans.is_empty());
    }

    #[test]
    fn test_resolve_theme_dark() {
        let theme = resolve_theme("dark");
        assert_eq!(theme.background, ratatui::style::Color::Black);
    }

    #[test]
    fn test_resolve_theme_bright() {
        let theme = resolve_theme("bright");
        assert_eq!(theme.background, ratatui::style::Color::White);
    }

    #[test]
    fn test_resolve_theme_light() {
        let theme = resolve_theme("light");
        assert_eq!(theme.background, ratatui::style::Color::White);
    }

    #[test]
    fn test_resolve_theme_unknown() {
        let theme = resolve_theme("unknown");
        assert_eq!(theme.background, ratatui::style::Color::Black);
    }

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

//! Terminal user interface for snippet selection.
//!
//! Provides the main TUI event loop with fuzzy search, syntax highlighting,
//! visual multi-select mode, and keyboard navigation. Re-exports the theme
//! system and variable prompting dialog.

mod highlight;
mod state;
mod theme;
mod variables;

mod _generated_bundled_themes;

pub use theme::get_theme;
pub use variables::{VariablePromptResult, prompt_variables};

use std::io;
use std::sync::LazyLock;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::text::Line;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation},
};

use crate::clipboard;
use crate::utils::extract_variables_for_display;
use crate::utils::has_unmatched_angle_bracket;
use crate::utils::strip_escape_sequences;

use highlight::highlight_command;
use state::{FilterState, SelectState, SortMode, is_ctrl_key};
use theme::{style_fg, style_fg_bg};

static TERMINATE: LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    LazyLock::new(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));

pub fn get_terminate() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    TERMINATE.clone()
}

static MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);

fn extract_variables(command: &str) -> Vec<String> {
    extract_variables_for_display(command)
}

fn ensure_theme_picker_ready(
    manager: &mut Option<theme::ThemeManager>,
    list: &mut Vec<theme::ThemeInfo>,
    filtered: &mut Vec<usize>,
    active_idx: &mut usize,
) -> io::Result<()> {
    if manager.is_some() {
        return Ok(());
    }
    let mut mgr =
        theme::ThemeManager::new().map_err(|e| io::Error::other(format!("theme manager: {e}")))?;
    mgr.init_themes_dir()
        .map_err(|e| io::Error::other(format!("init themes: {e}")))?;
    *list = mgr
        .list_themes()
        .map_err(|e| io::Error::other(format!("list themes: {e}")))?;
    *filtered = (0..list.len()).collect();
    *active_idx = mgr
        .get_active_theme_name()
        .as_ref()
        .and_then(|n| list.iter().position(|t| &t.name == n))
        .unwrap_or(0);
    *manager = Some(mgr);
    Ok(())
}

fn apply_theme_from_picker(
    manager: &Option<theme::ThemeManager>,
    list: &[theme::ThemeInfo],
    filtered: &[usize],
    active_idx: usize,
) -> bool {
    let Some(mgr) = manager else { return false };
    let Some(&idx) = filtered.get(active_idx) else {
        return false;
    };
    let Some(info) = list.get(idx) else {
        return false;
    };
    match mgr.load_theme(&info.name) {
        Ok(t) => {
            theme::set_active_theme(t);
            true
        }
        Err(e) => {
            tracing::warn!("failed to load theme {}: {e}", info.name);
            false
        }
    }
}

fn commit_theme_picker(
    manager: &mut Option<theme::ThemeManager>,
    list: &[theme::ThemeInfo],
    filtered: &[usize],
    active_idx: usize,
) -> io::Result<()> {
    if let Some(mgr) = manager.as_mut()
        && let Some(&idx) = filtered.get(active_idx)
        && let Some(info) = list.get(idx)
    {
        mgr.set_active_theme(&info.name)
            .map_err(|e| io::Error::other(format!("save theme: {e}")))?;
    }
    Ok(())
}

fn cancel_theme_picker(manager: &Option<theme::ThemeManager>, original: Option<theme::Theme>) {
    // Restore the in-memory theme that was active before the picker opened.
    // We snapshot the actual `Theme` (not the persisted name) so cancellation
    // works even when no theme has ever been saved to `themes.toml`.
    if let Some(t) = original {
        theme::set_active_theme(t);
        return;
    }
    // Fallback: if the original was never captured (shouldn't happen in the
    // real code path), try to restore from the persisted name.
    if let Some(mgr) = manager
        && let Some(name) = mgr.get_active_theme_name()
        && let Ok(t) = mgr.load_theme(&name)
    {
        theme::set_active_theme(t);
    }
}

pub struct SnippetListParams<'a> {
    pub descriptions: &'a [String],
    pub commands: &'a [String],
    pub tags: &'a [Vec<String>],
    pub is_search: bool,
    pub initial_filter: Option<&'a str>,
    pub folders: &'a [Vec<String>],
    pub favorites: &'a [bool],
    pub snippets: &'a [crate::library::Snippet],
    pub original_indices: &'a [usize],
}

pub fn select_snippet(params: SnippetListParams) -> io::Result<Option<(usize, Option<String>)>> {
    select_snippet_inner(params)
}

#[allow(clippy::collapsible_match)]
fn select_snippet_inner(params: SnippetListParams) -> io::Result<Option<(usize, Option<String>)>> {
    let SnippetListParams {
        descriptions,
        commands,
        tags,
        is_search,
        initial_filter,
        folders: _folders,
        favorites,
        snippets,
        original_indices: _original_indices,
    } = params;
    // Enable mouse capture before initializing terminal
    // Gracefully degrade if mouse capture is not supported (e.g., headless SSH)
    if let Err(e) = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture) {
        tracing::warn!(
            "Mouse capture not available, continuing without mouse support: {}",
            e
        );
    }
    let mut terminal = ratatui::init();

    // Pre-compute syntax-highlighted commands once at startup (not inside draw loop)
    // This avoids the closure-capture issues that cause TUI to hang
    let mut highlighted_commands: Vec<Line<'static>> =
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

    // Theme picker state
    let mut theme_picker_mode = false;
    let mut theme_picker_insert_mode = false;
    let mut theme_filter = String::new();
    let mut theme_input_text = String::new();
    let mut theme_filtered: Vec<usize> = Vec::new();
    let mut theme_dirty = false;
    let mut theme_active_idx: usize = 0;
    let mut theme_picker_manager: Option<theme::ThemeManager> = None;
    let mut theme_picker_list: Vec<theme::ThemeInfo> = Vec::new();
    // Snapshot of the in-memory theme captured when the picker opens, so
    // `cancel_theme_picker` can restore the user's previewed-from state
    // unconditionally (even if no theme was ever persisted to `themes.toml`).
    let mut theme_picker_original: Option<theme::Theme> = None;
    let mut pending_rehighlight = false;
    let mut last_theme_update: Option<std::time::Instant> = None;
    const THEME_DEBOUNCE_MS: u64 = 150;

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
        // Check for signal-induced termination (SIGINT/SIGTERM)
        if TERMINATE.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        // Lazy init theme picker
        if theme_picker_mode && theme_picker_manager.is_none() {
            if let Err(e) = ensure_theme_picker_ready(
                &mut theme_picker_manager,
                &mut theme_picker_list,
                &mut theme_filtered,
                &mut theme_active_idx,
            ) {
                tracing::warn!("theme picker init failed: {e}");
                theme_picker_mode = false;
                theme_picker_insert_mode = false;
                theme_filter.clear();
                theme_input_text.clear();
                theme_dirty = false;
            } else {
                // Snapshot the current in-memory theme so cancel can restore it
                // even when no theme is persisted to `themes.toml`.
                theme_picker_original = Some(theme::get_theme());
                if apply_theme_from_picker(
                    &theme_picker_manager,
                    &theme_picker_list,
                    &theme_filtered,
                    theme_active_idx,
                ) {
                    pending_rehighlight = true;
                }
            }
        }

        // Re-highlight on theme change
        if pending_rehighlight {
            highlighted_commands = commands.iter().map(|c| highlight_command(c)).collect();
            pending_rehighlight = false;
        }

        // Debounced theme filter rebuild
        let theme_debounce_elapsed =
            last_theme_update.is_none_or(|t| t.elapsed().as_millis() >= THEME_DEBOUNCE_MS as u128);
        if theme_dirty && theme_debounce_elapsed {
            theme_filtered = if theme_filter.is_empty() {
                (0..theme_picker_list.len()).collect()
            } else {
                theme_picker_list
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| {
                        if MATCHER.fuzzy_match(&t.name, &theme_filter).is_some() {
                            Some(i)
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            if theme_active_idx >= theme_filtered.len() {
                theme_active_idx = theme_filtered.len().saturating_sub(1);
            }
            theme_dirty = false;
            last_theme_update = None;
        }

        // Debounce: only recompute filtered list if enough time has passed since last filter change
        let debounce_elapsed = last_filter_update
            .is_none_or(|t| t.elapsed().as_millis() >= FILTER_DEBOUNCE_MS as u128);
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
        let has_any_filter = has_incremental_search || has_main_filter;
        let current_filter_text = if tag_filter_mode {
            filter_state.tag_filter_text.clone()
        } else {
            filter.clone()
        };

        // Optimization: when no recompute is needed AND no filter is active,
        // reuse previous filtered results directly without rebuilding candidates
        if !should_recompute && !has_any_filter && !filtered.is_empty() {
            sel.update(filtered.len());
        } else {
            // should_recompute = false means either: no change, or debounce window not elapsed yet
            // In both cases, reuse previous filtered results. Only build fresh from all_display
            // when we actually need to recompute (should_recompute = true).
            let mut candidates: Vec<(usize, String, Vec<String>, Option<i64>)> =
                if !should_recompute {
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
                        let tag_match =
                            if tag_filter_mode || !filter_state.tag_filter_text.is_empty() {
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
                        SortMode::Newest => Some(
                            snippets[b.0]
                                .created_at
                                .cmp(&snippets[a.0].created_at)
                                .then_with(|| b.0.cmp(&a.0)),
                        ),
                        SortMode::Oldest => Some(
                            snippets[a.0]
                                .created_at
                                .cmp(&snippets[b.0].created_at)
                                .then_with(|| a.0.cmp(&b.0)),
                        ),
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
        } // end else (has_any_filter || should_recompute || filtered.is_empty())

        // Filter indicator in title bar - ONLY shows incremental search (/), NOT the main filter.
        // Main filter text is displayed in the filter input box below, not in the title.
        // This ensures the input field position remains stable and text appears in the correct location.
        let filter_indicator = if tag_filter_mode {
            format!("[tag: {}]", filter_state.tag_filter_text)
        } else if !insert_mode && !incremental_search.is_empty() {
            format!("/{incremental_search}")
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

        if let Err(e) = terminal.draw(|f| {
            let size = f.area();
            if size.width < 10 || size.height < 10 {
                let error_msg = "Terminal too small - resize to at least 10x10";
                let paragraph = Paragraph::new(error_msg)
                    .centered()
                    .block(Block::default().title("Error").borders(Borders::ALL));
                f.render_widget(paragraph, size);
                return;
            }
            let picker_mode = theme_picker_mode;
            let picker_ins = theme_picker_insert_mode;
            let picker_filter_text: &str = if picker_ins {
                &theme_input_text
            } else {
                &theme_filter
            };
            let count = if picker_mode {
                theme_filtered.len()
            } else {
                filtered.len()
            };
            let title_part = if picker_mode {
                let active_name = theme_picker_manager
                    .as_ref()
                    .and_then(|m| m.get_active_theme_name())
                    .unwrap_or_default();
                let active_str = if active_name.is_empty() {
                    String::new()
                } else {
                    format!(" [{active_name}]")
                };
                format!("Themes [{count}]{active_str} {filter_indicator}")
            } else {
                format!("Snippets [{count}] {filter_indicator}{sort_indicator}")
            };
            let separator = "─".repeat((size.width as usize).saturating_sub(title_part.len() + 8));
            let theme = get_theme();
            let block = Block::default()
                .title(format!("{title_part} {separator}"))
                .borders(Borders::ALL)
                .border_style(style_fg(theme.border))
                .title_style(style_fg(theme.secondary))
                .style(TuiStyle::default().bg(theme.background));
            let input_block_title = if picker_mode {
                "Theme Filter"
            } else if tag_filter_mode {
                "Tag Filter"
            } else {
                "Filter"
            };
            let input_block = Block::default()
                .title(input_block_title)
                .borders(Borders::ALL)
                .border_style(style_fg(theme.border))
                .title_style(style_fg(theme.secondary))
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

            f.render_widget(input_block, chunks[0]);
            // Filter text displayed in the filter input box (NOT in title bar).
            // Insert mode: show input_text (what user is typing)
            // Normal mode: show filter (applied filter)
            // Tag filter mode: show tag_filter_text
            // Picker mode: show theme_input_text in INS or theme_filter in NOR
            let filter_text: &str = if picker_mode {
                picker_filter_text
            } else if tag_filter_mode {
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

            if (picker_mode && picker_ins) || (!picker_mode && insert_mode) {
                use unicode_width::UnicodeWidthStr;
                let cursor_text: &str = if picker_mode {
                    &theme_input_text
                } else {
                    &input_text
                };
                let cursor_x = chunks[0].x
                    + 1
                    + cursor_text.width().min(u16::MAX as usize) as u16;
                let cursor_y = chunks[0].y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            if picker_mode {
                // Theme picker list
                let saved_name = theme_picker_manager
                    .as_ref()
                    .and_then(|m| m.get_active_theme_name());
                let picker_items: Vec<ListItem> = theme_filtered
                    .iter()
                    .filter_map(|&idx| theme_picker_list.get(idx))
                    .map(|info| {
                        let is_active = saved_name.as_deref() == Some(info.name.as_str());
                        let prefix = if is_active { "★ " } else { "  " };
                        let style = if is_active {
                            style_fg(theme.accent)
                        } else {
                            style_fg(theme.text)
                        };
                        let line = Line::from(vec![
                            ratatui::text::Span::raw(prefix),
                            ratatui::text::Span::styled(info.name.clone(), style),
                        ]);
                        ListItem::new(line)
                    })
                    .collect();
                let picker_list = List::new(picker_items)
                    .block(block)
                    .style(TuiStyle::default().bg(theme.background))
                    .highlight_style(
                        ratatui::style::Style::default()
                            .fg(theme.text)
                            .bg(theme.selected_bg),
                    )
                    .highlight_symbol("▶ ");
                let mut picker_list_state = ratatui::widgets::ListState::default();
                picker_list_state.select(Some(theme_active_idx));
                f.render_stateful_widget(picker_list, chunks[1], &mut picker_list_state);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(ratatui::style::Style::default().bg(theme.secondary));
                let mut picker_scroll_state = ratatui::widgets::ScrollbarState::default()
                    .content_length(theme_filtered.len())
                    .position(theme_active_idx);
                f.render_stateful_widget(scrollbar, chunks[1], &mut picker_scroll_state);

                // Theme swatch preview
                let preview_name = theme_filtered
                    .get(theme_active_idx)
                    .and_then(|&idx| theme_picker_list.get(idx))
                    .map(|info| info.name.clone())
                    .unwrap_or_else(|| "(no selection)".to_string());
                let is_saved_active = saved_name.as_deref() == Some(preview_name.as_str());
                let preview_title = format!("Preview: {preview_name}");
                let preview_block = Block::default()
                    .title(preview_title)
                    .borders(Borders::ALL)
                    .border_style(style_fg(theme.border))
                    .title_style(style_fg(theme.secondary))
                    .style(TuiStyle::default().bg(theme.background));

                let swatch = |c: ratatui::style::Color| {
                    ratatui::text::Span::styled("███", style_fg(c))
                };
                let label = |s: &str| -> ratatui::text::Span<'static> {
                    ratatui::text::Span::styled(s.to_string(), style_fg(theme.text))
                };
                let dim = |s: &str| -> ratatui::text::Span<'static> {
                    ratatui::text::Span::styled(s.to_string(), style_fg(theme.muted))
                };

                let mut preview_spans: Vec<ratatui::text::Span<'static>> = vec![
                    swatch(theme.primary),
                    label(" primary  "),
                    swatch(theme.secondary),
                    label(" tertiary  "),
                    swatch(theme.accent),
                    label(" accent"),
                    ratatui::text::Span::raw("\n"),
                    label("Background: "),
                    swatch(theme.background),
                    ratatui::text::Span::raw("\n"),
                    label("> Selected: "),
                    swatch(theme.selected_bg),
                    ratatui::text::Span::raw("\n"),
                    label("Text: "),
                    swatch(theme.text),
                    label("  Border: "),
                    swatch(theme.border),
                    label("  Muted: "),
                    swatch(theme.muted),
                    ratatui::text::Span::raw("\n"),
                    label("String literal: "),
                    swatch(theme.string_color),
                    label("  Escape: "),
                    swatch(theme.escape_color),
                ];
                if is_saved_active {
                    preview_spans.push(ratatui::text::Span::raw("\n"));
                    preview_spans.push(dim("(this is your saved active theme)"));
                }

                let preview_widget = Paragraph::new(Line::from(preview_spans))
                    .block(preview_block)
                    .style(style_fg_bg(theme.text, theme.background));
                f.render_widget(preview_widget, chunks[2]);
            } else {
                // Snippet list (existing rendering)
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
                                    format!("[{desc}] "),
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

                f.render_stateful_widget(list, chunks[1], &mut sel.list_state);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(ratatui::style::Style::default().bg(theme.secondary));
                f.render_stateful_widget(scrollbar, chunks[1], &mut sel.scroll_state);

                if sel.selected < filtered.len() {
                    let idx = filtered[sel.selected].0;
                    let snippet_cmd = &commands[idx];
                    let snippet_desc = &descriptions[idx];

                    let vars = extract_variables(snippet_cmd);
                    let has_vars = !vars.is_empty();

                    let preview_title = format!("Preview: {snippet_desc}");
                    let preview_block = Block::default()
                        .title(preview_title)
                        .borders(Borders::ALL)
                        .border_style(style_fg(theme.border))
                        .title_style(style_fg(theme.secondary))
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
            }

            let copied_desc = copied_message.as_ref().map(|(desc, _)| desc.clone());
            let mode_str = if insert_mode { "INS" } else { "NOR" };
            let tag_mode_str = if tag_filter_mode { " TAG" } else { "" };

            let status_text: String = if picker_mode {
                if picker_ins {
                    "[THEME-INS] | esc: leave ins | enter: apply | up/down: navigate".to_string()
                } else {
                    "[THEME-NOR] | i: filter | enter: apply | up/down: preview | e/q: back"
                        .to_string()
                }
            } else if let Some(desc) = copied_desc {
                format!("[{mode_str}]{tag_mode_str} | {desc}")
            } else if is_search {
                if insert_mode {
                    format!(
                        "[{mode_str}]{tag_mode_str} | esc: normal | /: search | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear | tab: mode"
                    )
                } else {
                    format!(
                        "[{mode_str}]{tag_mode_str} | i: insert | y/ctrl+c: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear | tab: mode"
                    )
                }
            } else if insert_mode {
                format!(
                    "[{mode_str}]{tag_mode_str} | esc: normal | enter: run | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear | tab: mode"
                )
            } else {
                format!(
                    "[{mode_str}]{tag_mode_str} | i: insert | y: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear | tab: mode | double-click: run"
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
        }) {
            tracing::warn!("Terminal draw error: {}", e);
        }

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

        let polled = event::poll(Duration::from_millis(200)).unwrap_or(false);
        if polled {
            match event::read() {
                Ok(CEvent::Mouse(mouse_event)) => {
                    if !theme_picker_mode {
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
                                    < list_area.y
                                        + filtered.len().min(list_area.height as usize) as u16
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
                }
                Ok(CEvent::Key(key)) => {
                    if key.kind == KeyEventKind::Press {
                        if let Some((_, instant)) = copied_message
                            && instant.elapsed().as_secs() >= 3
                        {
                            copied_message = None;
                        }

                        if theme_picker_mode {
                            if theme_picker_insert_mode {
                                match key.code {
                                    KeyCode::Esc => {
                                        if !theme_input_text.is_empty() {
                                            theme_filter = theme_input_text.clone();
                                            theme_input_text.clear();
                                        }
                                        theme_picker_insert_mode = false;
                                    }
                                    KeyCode::Down | KeyCode::Char('j')
                                        if theme_active_idx + 1 < theme_filtered.len() =>
                                    {
                                        theme_active_idx += 1;
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Char('k') if theme_active_idx > 0 => {
                                        theme_active_idx -= 1;
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        if !theme_input_text.is_empty() {
                                            theme_input_text.pop();
                                            theme_filter = theme_input_text.clone();
                                            theme_dirty = true;
                                            last_theme_update = Some(std::time::Instant::now());
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        theme_input_text.push(c);
                                        theme_filter = theme_input_text.clone();
                                        theme_dirty = true;
                                        last_theme_update = Some(std::time::Instant::now());
                                    }
                                    KeyCode::Enter => {
                                        if let Err(e) = commit_theme_picker(
                                            &mut theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            tracing::warn!("commit theme failed: {e}");
                                        }
                                        theme_picker_mode = false;
                                        theme_picker_insert_mode = false;
                                        theme_filter.clear();
                                        theme_input_text.clear();
                                        theme_dirty = false;
                                        pending_rehighlight = true;
                                    }
                                    _ => {}
                                }
                            } else {
                                match key.code {
                                    KeyCode::Char('i') => {
                                        theme_picker_insert_mode = true;
                                        theme_input_text = theme_filter.clone();
                                    }
                                    KeyCode::Char('q') | KeyCode::Esc => {
                                        cancel_theme_picker(
                                            &theme_picker_manager,
                                            theme_picker_original,
                                        );
                                        theme_picker_mode = false;
                                        theme_picker_insert_mode = false;
                                        theme_filter.clear();
                                        theme_input_text.clear();
                                        theme_dirty = false;
                                        pending_rehighlight = true;
                                    }
                                    KeyCode::Char('e') => {
                                        cancel_theme_picker(
                                            &theme_picker_manager,
                                            theme_picker_original,
                                        );
                                        theme_picker_mode = false;
                                        theme_picker_insert_mode = false;
                                        theme_filter.clear();
                                        theme_input_text.clear();
                                        theme_dirty = false;
                                        pending_rehighlight = true;
                                    }
                                    KeyCode::Enter => {
                                        if let Err(e) = commit_theme_picker(
                                            &mut theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            tracing::warn!("commit theme failed: {e}");
                                        }
                                        theme_picker_mode = false;
                                        theme_picker_insert_mode = false;
                                        theme_filter.clear();
                                        theme_input_text.clear();
                                        theme_dirty = false;
                                        pending_rehighlight = true;
                                    }
                                    KeyCode::Char('j') | KeyCode::Down
                                        if theme_active_idx + 1 < theme_filtered.len() =>
                                    {
                                        theme_active_idx += 1;
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Up if theme_active_idx > 0 => {
                                        theme_active_idx -= 1;
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::Char('g') if is_ctrl_key(&key, 'g') => {
                                        theme_active_idx = 0;
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::Char('G') => {
                                        theme_active_idx = theme_filtered.len().saturating_sub(1);
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::PageDown | KeyCode::Char('d')
                                        if is_ctrl_key(&key, 'd') =>
                                    {
                                        theme_active_idx = (theme_active_idx + 10)
                                            .min(theme_filtered.len().saturating_sub(1));
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    KeyCode::PageUp | KeyCode::Char('u')
                                        if is_ctrl_key(&key, 'u') =>
                                    {
                                        theme_active_idx = theme_active_idx.saturating_sub(10);
                                        if apply_theme_from_picker(
                                            &theme_picker_manager,
                                            &theme_picker_list,
                                            &theme_filtered,
                                            theme_active_idx,
                                        ) {
                                            pending_rehighlight = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            continue;
                        }

                        if is_ctrl_key(&key, 'c') && sel.selected < filtered.len() {
                            let idx = filtered[sel.selected].0;
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
                                    match clipboard::copy_to_clipboard_auto(&copy_text) {
                                        Ok(()) => {
                                            should_copy = Some(format!(
                                                "{} snippets copied",
                                                end - start + 1
                                            ));
                                        }
                                        Err(e) => {
                                            tracing::warn!("Clipboard copy failed: {}", e);
                                            copied_message = Some((
                                                format!("Copy failed: {e}"),
                                                std::time::Instant::now(),
                                            ));
                                        }
                                    }
                                    if let Some((idx, _, _)) = filtered.get(start) {
                                        let original_idx = *idx;
                                        if original_idx < snippets.len()
                                            && let Err(e) = crate::logging::audit_log(
                                                "copy",
                                                &snippets[original_idx],
                                                None,
                                            )
                                        {
                                            tracing::debug!("Audit log write failed: {}", e);
                                        }
                                    }
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

                        if is_ctrl_key(&key, 'f')
                            || is_ctrl_key(&key, 'd')
                            || key.code == KeyCode::PageDown
                        {
                            sel.selected =
                                (sel.selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if is_ctrl_key(&key, 'b')
                            || is_ctrl_key(&key, 'u')
                            || key.code == KeyCode::PageUp
                        {
                            sel.selected = sel.selected.saturating_sub(10);
                        }

                        if insert_mode {
                            match key.code {
                                KeyCode::Char('/') => {
                                    insert_mode = true;
                                    incremental_search.clear();
                                    input_text.clear();
                                    filter.clear();
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
                                        // Clear filter when exiting insert mode with empty input
                                        // to avoid stale filter state confusing the user
                                        filter.clear();
                                        insert_mode = false;
                                        filter_dirty = true;
                                        last_filter_update = Some(std::time::Instant::now());
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
                                KeyCode::Char('y') => {
                                    if sel.selected < filtered.len() {
                                        let idx = filtered[sel.selected].0;
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
                                    let is_expired = pending_gg_time
                                        .map(|t| {
                                            now.duration_since(t).as_millis()
                                                > GG_TIMEOUT_MS as u128
                                        })
                                        .unwrap_or(true);
                                    if is_expired {
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
                                    filter_state.tag_filter_text.clear();
                                    tag_filter_mode = false;
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
                                    input_text.clear();
                                    filter.clear();
                                    filter_dirty = true;
                                    last_filter_update = Some(std::time::Instant::now());
                                }
                                KeyCode::Esc => {
                                    // Esc no longer quits - use q or Ctrl+C instead
                                }
                                KeyCode::Char('e') => {
                                    if !theme_picker_mode {
                                        theme_picker_mode = true;
                                    }
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
    if let Err(e) = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture) {
        tracing::warn!("Failed to disable mouse capture: {}", e);
    }
    ratatui::restore();
    Ok(if !filtered.is_empty() && sel.selected < filtered.len() {
        Some((filtered[sel.selected].0, should_copy))
    } else {
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminate_functionality() {
        let terminate = get_terminate();
        terminate.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!terminate.load(std::sync::atomic::Ordering::SeqCst));

        terminate.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(terminate.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn bundled_themes_decoder_roundtrip() {
        use super::_generated_bundled_themes::bundled_themes_decoded;
        let decoded = bundled_themes_decoded().unwrap();
        assert!(
            decoded.len() >= 40,
            "expected ~50 bundled themes, got {}",
            decoded.len()
        );
        for (name, content) in &decoded {
            let first_line = content.lines().next().unwrap_or("");
            assert!(
                first_line.trim() == "[general]",
                "theme {name:?} does not start with [general]: {first_line:?}"
            );
        }
        let default = super::_generated_bundled_themes::DEFAULT_BUNDLED;
        assert!(default.contains("[general]"));
        assert!(default.contains("[text]"));
        assert!(default.contains("[buffer]"));
    }
}

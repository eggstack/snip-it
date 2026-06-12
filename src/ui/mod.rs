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
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::text::{Line, Span};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation},
};

use crate::clipboard;
use crate::utils::extract_variables_for_display;
use crate::utils::has_unmatched_angle_bracket;
use crate::utils::strip_escape_sequences;

/// RAII guard that disables mouse capture and restores the terminal when dropped.
/// Ensures the terminal is always restored even on early return or panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        ratatui::restore();
    }
}

use highlight::highlight_command;
use state::{FilterState, SelectState, SortMode, is_ctrl_key};
use theme::{style_fg, style_fg_bg};

static TERMINATE: LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    LazyLock::new(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));

pub fn get_terminate() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    TERMINATE.clone()
}

static MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);

#[derive(Clone, Debug, PartialEq, Eq)]
struct FilterRequest {
    text: String,
    text_lower: String,
    include_tags: bool,
    incremental: bool,
}

impl FilterRequest {
    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn can_narrow_from(&self, previous: &Self) -> bool {
        !previous.text.is_empty()
            && self.incremental == previous.incremental
            && self.include_tags == previous.include_tags
            && self.text.starts_with(&previous.text)
    }
}

fn current_filter_request(
    filter: &str,
    incremental_search: &str,
    filter_state: &FilterState,
    tag_filter_mode: bool,
) -> FilterRequest {
    if !incremental_search.is_empty() {
        return FilterRequest {
            text: incremental_search.to_string(),
            text_lower: incremental_search.to_lowercase(),
            include_tags: false,
            incremental: true,
        };
    }

    let has_main_filter = !filter.is_empty() || !filter_state.tag_filter_text.is_empty();
    if !has_main_filter {
        return FilterRequest {
            text: String::new(),
            text_lower: String::new(),
            include_tags: false,
            incremental: false,
        };
    }

    let text = if tag_filter_mode {
        &filter_state.tag_filter_text
    } else {
        filter
    };
    FilterRequest {
        text: text.to_string(),
        text_lower: text.to_lowercase(),
        include_tags: tag_filter_mode || !filter_state.tag_filter_text.is_empty(),
        incremental: false,
    }
}

fn filter_request_would_narrow(
    filter: &str,
    incremental_search: &str,
    filter_state: &FilterState,
    tag_filter_mode: bool,
    last_filter_request: Option<&FilterRequest>,
) -> bool {
    let request = current_filter_request(filter, incremental_search, filter_state, tag_filter_mode);
    last_filter_request.is_some_and(|previous| request.can_narrow_from(previous))
}

fn filter_update_deadline_for_insert(
    filter: &str,
    incremental_search: &str,
    filter_state: &FilterState,
    tag_filter_mode: bool,
    last_filter_request: Option<&FilterRequest>,
) -> Option<Instant> {
    if last_filter_request.is_none()
        || filter_request_would_narrow(
            filter,
            incremental_search,
            filter_state,
            tag_filter_mode,
            last_filter_request,
        )
    {
        None
    } else {
        Some(Instant::now())
    }
}

fn rebuild_filter_candidates(
    candidates: &mut Vec<(usize, Option<i64>)>,
    source_indices: impl Iterator<Item = usize>,
    request: &FilterRequest,
    all_display: &[String],
    all_display_lower: &[String],
    all_tags_search: &[String],
) {
    candidates.clear();

    if request.is_empty() {
        candidates.extend((0..all_display.len()).map(|i| (i, None)));
        return;
    }

    for i in source_indices {
        let Some(display) = all_display.get(i) else {
            continue;
        };
        let is_exact_display = all_display_lower
            .get(i)
            .is_some_and(|display| display == &request.text_lower);
        let tag_match = request.include_tags
            && all_tags_search
                .get(i)
                .is_some_and(|snippet_tags| snippet_tags.contains(&request.text_lower));

        if is_exact_display || tag_match {
            candidates.push((i, Some(i64::MAX)));
        } else if let Some(score) = MATCHER.fuzzy_match(display, &request.text) {
            candidates.push((i, Some(score)));
        }
    }
}

fn sort_filtered_indices(
    filtered: &mut [(usize, Option<i64>)],
    filter_state: &FilterState,
    snippets: &[crate::library::Snippet],
    display_lower: &[String],
    has_filter: bool,
) {
    if filter_state.sort_mode == SortMode::None && !has_filter {
        return;
    }

    filtered.sort_unstable_by(|a, b| {
        let score_cmp = match (a.1, b.1) {
            (Some(sa), Some(sb)) => sb.cmp(&sa),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };

        if score_cmp != std::cmp::Ordering::Equal || !has_filter {
            let explicit_sort = match filter_state.sort_mode {
                SortMode::Newest => {
                    snippets
                        .get(b.0)
                        .zip(snippets.get(a.0))
                        .map(|(b_snip, a_snip)| {
                            b_snip
                                .created_at
                                .cmp(&a_snip.created_at)
                                .then_with(|| b.0.cmp(&a.0))
                        })
                }
                SortMode::Oldest => {
                    snippets
                        .get(a.0)
                        .zip(snippets.get(b.0))
                        .map(|(a_snip, b_snip)| {
                            a_snip
                                .created_at
                                .cmp(&b_snip.created_at)
                                .then_with(|| a.0.cmp(&b.0))
                        })
                }
                _ => None,
            };

            let secondary = match filter_state.sort_mode {
                SortMode::AlphaAsc => display_lower
                    .get(a.0)
                    .zip(display_lower.get(b.0))
                    .map(|(a_display, b_display)| a_display.cmp(b_display)),
                SortMode::AlphaDesc => display_lower
                    .get(a.0)
                    .zip(display_lower.get(b.0))
                    .map(|(a_display, b_display)| b_display.cmp(a_display)),
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
}

fn debounce_elapsed(last_update: Option<Instant>, debounce_ms: u64) -> bool {
    last_update.is_none_or(|t| t.elapsed() >= Duration::from_millis(debounce_ms))
}

fn pending_debounce_timeout(
    dirty: bool,
    last_update: Option<Instant>,
    debounce_ms: u64,
) -> Option<Duration> {
    if !dirty {
        return None;
    }

    let last_update = last_update?;
    let debounce = Duration::from_millis(debounce_ms);
    Some(debounce.saturating_sub(last_update.elapsed()))
}

fn next_event_poll_timeout(
    filter_dirty: bool,
    last_filter_update: Option<Instant>,
    filter_debounce_ms: u64,
    theme_dirty: bool,
    last_theme_update: Option<Instant>,
    theme_debounce_ms: u64,
) -> Duration {
    [
        pending_debounce_timeout(filter_dirty, last_filter_update, filter_debounce_ms),
        pending_debounce_timeout(theme_dirty, last_theme_update, theme_debounce_ms),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or_else(|| Duration::from_millis(200))
}

fn command_line_for_display(
    idx: usize,
    commands: &[String],
    highlighted_commands: &[Option<Line<'static>>],
    style: TuiStyle,
) -> Line<'static> {
    if let Some(Some(line)) = highlighted_commands.get(idx) {
        return line.clone();
    }

    Line::from(Span::styled(
        commands.get(idx).cloned().unwrap_or_default(),
        style,
    ))
}

fn warm_visible_highlights(
    commands: &[String],
    highlighted_commands: &mut [Option<Line<'static>>],
    visible_indices: &[usize],
    budget: Duration,
) -> bool {
    let started = Instant::now();
    let mut warmed_any = false;
    for &idx in visible_indices {
        let Some(command) = commands.get(idx) else {
            continue;
        };
        let Some(slot) = highlighted_commands.get_mut(idx) else {
            continue;
        };
        if slot.is_some() {
            continue;
        }

        *slot = Some(highlight_command(command));
        warmed_any = true;
        if started.elapsed() >= budget {
            break;
        }
    }
    warmed_any
}

fn extract_variables(command: &str) -> Vec<String> {
    extract_variables_for_display(command)
}

#[derive(Clone, Debug)]
struct SnippetPreview {
    content: String,
}

impl SnippetPreview {
    fn from_command(command: &str) -> Self {
        let vars = extract_variables(command);
        let has_unmatched = has_unmatched_angle_bracket(command);
        let content = if !vars.is_empty() || has_unmatched {
            let mut content = format!(
                "{}\n\nVars: {}",
                strip_escape_sequences(command),
                vars.join(", ")
            );
            if has_unmatched {
                content.push_str("\n\nWarning: unmatched '<' found - will be treated as literal");
            }
            content
        } else {
            strip_escape_sequences(command)
        };

        Self { content }
    }
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
    let _guard = TerminalGuard; // Ensures terminal is restored on any exit path

    // Highlight commands lazily so large libraries do not pay the full syntax
    // highlighting cost before the first frame is shown.
    let mut highlighted_commands: Vec<Option<Line<'static>>> = vec![None; commands.len()];
    let mut snippet_previews: Vec<Option<SnippetPreview>> = vec![None; commands.len()];

    let mut sel = SelectState::new();
    let mut filter = initial_filter.map(String::from).unwrap_or_default();
    let mut incremental_search = String::new();
    // input_text tracks what user types in insert mode - displayed in filter input box, NOT in title bar
    let mut input_text = String::new();
    let mut filter_state = FilterState::default();
    let mut filtered: Vec<usize> = (0..descriptions.len()).collect();
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
    const THEME_DEBOUNCE_MS: u64 = 100;

    // Debounce filter updates to avoid fuzzy matching on every keystroke
    let mut filter_dirty = !filter.is_empty();
    let mut last_filter_update: Option<std::time::Instant> = None;
    const FILTER_DEBOUNCE_MS: u64 = 35;
    const HIGHLIGHT_WARM_BUDGET: Duration = Duration::from_millis(2);

    // Mouse double-click tracking
    let mut last_click_row: Option<u16> = None;
    let mut last_click_time: Option<std::time::Instant> = None;
    const DOUBLE_CLICK_DURATION_MS: u64 = 500;

    let all_display: Vec<String> = descriptions
        .iter()
        .enumerate()
        .map(|(i, _)| format!("[{}]: {}", descriptions[i], commands[i]))
        .collect();
    let all_display_lower: Vec<String> = all_display.iter().map(|d| d.to_lowercase()).collect();
    let all_tags_search: Vec<String> = tags
        .iter()
        .map(|snippet_tags| snippet_tags.join("\n").to_lowercase())
        .collect();
    let mut filter_candidates: Vec<(usize, Option<i64>)> =
        (0..all_display.len()).map(|i| (i, None)).collect();
    let mut last_filter_request: Option<FilterRequest> = None;
    let mut needs_redraw = true;
    sel.update(filtered.len());

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
                needs_redraw = true;
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
                needs_redraw = true;
            }
        }

        // Re-highlight on theme change
        if pending_rehighlight {
            highlighted_commands.fill_with(|| None);
            pending_rehighlight = false;
            needs_redraw = true;
        }

        // Debounced theme filter rebuild
        let theme_debounce_elapsed = debounce_elapsed(last_theme_update, THEME_DEBOUNCE_MS);
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
            needs_redraw = true;
        }

        // Debounce: only recompute filtered list if enough time has passed since last filter change
        let debounce_elapsed = debounce_elapsed(last_filter_update, FILTER_DEBOUNCE_MS);
        let should_recompute = if filter_dirty {
            // Always recompute immediately when filter becomes empty (backspace cleared filter)
            // to avoid showing stale filtered results after the user clears input.
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

        if should_recompute {
            let filter_request = current_filter_request(
                &filter,
                &incremental_search,
                &filter_state,
                tag_filter_mode,
            );
            if last_filter_request.as_ref() != Some(&filter_request) {
                if last_filter_request
                    .as_ref()
                    .is_some_and(|previous| filter_request.can_narrow_from(previous))
                {
                    rebuild_filter_candidates(
                        &mut filter_candidates,
                        filtered.iter().copied(),
                        &filter_request,
                        &all_display,
                        &all_display_lower,
                        &all_tags_search,
                    );
                } else {
                    rebuild_filter_candidates(
                        &mut filter_candidates,
                        0..all_display.len(),
                        &filter_request,
                        &all_display,
                        &all_display_lower,
                        &all_tags_search,
                    );
                }
            }

            let has_filter = !filter_request.is_empty();
            sort_filtered_indices(
                &mut filter_candidates,
                &filter_state,
                snippets,
                &all_display_lower,
                has_filter,
            );

            filtered.clear();
            filtered.extend(filter_candidates.iter().map(|(i, _)| *i));
            last_filter_request = Some(filter_request);

            sel.update(filtered.len());
            needs_redraw = true;
        } else {
            sel.update(filtered.len());
        } // end filter recompute

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let terminal_size = ratatui::layout::Rect::new(0, 0, cols, rows);
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
        let list_visible_rows = list_area.height.saturating_sub(2) as usize;
        let list_offset = if list_visible_rows == 0 {
            0
        } else {
            sel.selected
                .saturating_sub(list_visible_rows.saturating_sub(1))
        };
        let list_end = (list_offset + list_visible_rows).min(filtered.len());
        if needs_redraw {
            needs_redraw = false;

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

            let draw_result = terminal.draw(|f| {
                let size = f.area();
                if size.width < 10 || size.height < 10 {
                    let error_msg = "Terminal too small - resize to at least 10x10";
                    let paragraph = Paragraph::new(error_msg)
                        .centered()
                        .block(Block::default().title("Error").borders(Borders::ALL));
                    f.render_widget(paragraph, size);
                    return;
                }
                {
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
            let title = Line::from(vec![
                Span::styled(title_part.clone(), style_fg(theme.secondary)),
                Span::raw(" "),
                Span::styled(separator, style_fg(theme.border)),
            ]);
            let block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(style_fg(theme.border))
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
                let visible_filtered = &filtered[list_offset..list_end];
                let items: Vec<ListItem> = visible_filtered
                    .iter()
                    .map(|idx| {
                        let is_fav = *favorites.get(*idx).unwrap_or(&false);
                        let fav_indicator = if is_fav { "★ " } else { "  " };

                        let line = if list_display_mode == 1 {
                            let desc = &descriptions[*idx];
                            let cmd_spans = command_line_for_display(
                                *idx,
                                commands,
                                &highlighted_commands,
                                style_fg(theme.text),
                            )
                            .spans;

                            let mut combined: Vec<ratatui::text::Span<'static>> =
                                vec![ratatui::text::Span::styled(
                                    format!("[{desc}] "),
                                    style_fg(theme.text),
                                )];
                            combined.extend(cmd_spans);
                            Line::from(combined)
                        } else {
                            command_line_for_display(
                                *idx,
                                commands,
                                &highlighted_commands,
                                style_fg(theme.text),
                            )
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

                let mut visible_list_state = ratatui::widgets::ListState::default();
                visible_list_state.select(sel.selected.checked_sub(list_offset));
                f.render_stateful_widget(list, chunks[1], &mut visible_list_state);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(ratatui::style::Style::default().bg(theme.secondary));
                f.render_stateful_widget(scrollbar, chunks[1], &mut sel.scroll_state);

                if sel.selected < filtered.len() {
                    let idx = filtered[sel.selected];
                    let snippet_cmd = &commands[idx];
                    let snippet_desc = &descriptions[idx];

                    let preview_title = format!("Preview: {snippet_desc}");
                    let preview_block = Block::default()
                        .title(preview_title)
                        .borders(Borders::ALL)
                        .border_style(style_fg(theme.border))
                        .title_style(style_fg(theme.secondary))
                        .style(TuiStyle::default().bg(theme.background));

                    let preview = snippet_previews
                        .get_mut(idx)
                        .and_then(|slot| {
                            if slot.is_none() {
                                *slot = Some(SnippetPreview::from_command(snippet_cmd));
                            }
                            slot.as_ref()
                        });
                    let preview_content = preview
                        .map(|preview| preview.content.as_str())
                        .unwrap_or(snippet_cmd.as_str());

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
                }
            });
            if let Err(e) = draw_result {
                tracing::warn!("Terminal draw error: {}", e);
            }

            if !theme_picker_mode
                && !event::poll(Duration::from_millis(0)).unwrap_or(false)
                && list_offset < list_end
                && warm_visible_highlights(
                    commands,
                    &mut highlighted_commands,
                    &filtered[list_offset..list_end],
                    HIGHLIGHT_WARM_BUDGET,
                )
            {
                needs_redraw = true;
            }

            if needs_redraw {
                continue;
            }
        }

        let poll_timeout = next_event_poll_timeout(
            filter_dirty,
            last_filter_update,
            FILTER_DEBOUNCE_MS,
            theme_dirty,
            last_theme_update,
            THEME_DEBOUNCE_MS,
        );
        let polled = event::poll(poll_timeout).unwrap_or(false);
        if polled {
            match event::read() {
                Ok(CEvent::Mouse(mouse_event)) => {
                    needs_redraw = true;
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
                            let list_inner_y = list_area.y.saturating_add(1);
                            let list_inner_height = list_area.height.saturating_sub(2);
                            if mouse_event.row >= list_inner_y
                                && mouse_event.row < list_inner_y + list_inner_height
                                && mouse_event.row
                                    < list_inner_y
                                        + (list_end - list_offset).min(list_inner_height as usize)
                                            as u16
                            {
                                let clicked_row =
                                    list_offset + (mouse_event.row - list_inner_y) as usize;

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
                        needs_redraw = true;
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
                            let idx = filtered[sel.selected];
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
                                    if filtered.is_empty() {
                                        visual_mode = false;
                                        if !is_search {
                                            break;
                                        }
                                        continue;
                                    }
                                    let start = std::cmp::min(visual_start, visual_end)
                                        .min(filtered.len().saturating_sub(1));
                                    let end = std::cmp::max(visual_start, visual_end)
                                        .min(filtered.len().saturating_sub(1));
                                    let selected_items: Vec<String> = filtered
                                        .iter()
                                        .skip(start)
                                        .take(end - start + 1)
                                        .map(|idx| strip_escape_sequences(&commands[*idx]))
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
                                    if let Some(idx) = filtered.get(start) {
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
                                KeyCode::Char(c)
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    if tag_filter_mode {
                                        filter_state.tag_filter_text.push(c);
                                        filter_dirty = true;
                                    } else if insert_mode {
                                        input_text.push(c);
                                        filter.push(c);
                                        filter_dirty = true;
                                    } else if !incremental_search.is_empty() {
                                        incremental_search.push(c);
                                        filter_dirty = true;
                                    } else {
                                        filter.push(c);
                                        filter_dirty = true;
                                    }
                                    last_filter_update = filter_update_deadline_for_insert(
                                        &filter,
                                        &incremental_search,
                                        &filter_state,
                                        tag_filter_mode,
                                        last_filter_request.as_ref(),
                                    );
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
                                        let idx = filtered[sel.selected];
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
                                KeyCode::Char('n') => {
                                    filter_state.toggle_sort_new();
                                    filter_dirty = true;
                                    last_filter_update = None;
                                }
                                KeyCode::Char('o') => {
                                    filter_state.toggle_sort_old();
                                    filter_dirty = true;
                                    last_filter_update = None;
                                }
                                KeyCode::Char('a') => {
                                    filter_state.toggle_sort_alpha();
                                    filter_dirty = true;
                                    last_filter_update = None;
                                }
                                KeyCode::Char('z') => {
                                    filter_state.toggle_sort_alpha_rev();
                                    filter_dirty = true;
                                    last_filter_update = None;
                                }
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
                    needs_redraw = true;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Event read error: {}", e);
                }
            }
        }
    }
    Ok(if !filtered.is_empty() && sel.selected < filtered.len() {
        Some((filtered[sel.selected], should_copy))
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
    fn filter_request_detects_incremental_narrowing() {
        let previous = FilterRequest {
            text: "git".to_string(),
            text_lower: "git".to_string(),
            include_tags: false,
            incremental: false,
        };
        let current = FilterRequest {
            text: "git st".to_string(),
            text_lower: "git st".to_string(),
            include_tags: false,
            incremental: false,
        };
        assert!(current.can_narrow_from(&previous));

        let changed_mode = FilterRequest {
            text: "git st".to_string(),
            text_lower: "git st".to_string(),
            include_tags: true,
            incremental: false,
        };
        assert!(!changed_mode.can_narrow_from(&previous));
    }

    #[test]
    fn rebuild_filter_candidates_matches_display_and_tags() {
        let all_display = vec![
            "[status]: git status".to_string(),
            "[logs]: journalctl -u snip".to_string(),
            "[deploy]: ./release".to_string(),
        ];
        let all_display_lower: Vec<String> = all_display.iter().map(|d| d.to_lowercase()).collect();
        let all_tags_search = vec![
            "git".to_string(),
            "systemd".to_string(),
            "release".to_string(),
        ];
        let all_indices = [0, 1, 2];
        let mut candidates = Vec::new();

        rebuild_filter_candidates(
            &mut candidates,
            all_indices.iter().copied(),
            &FilterRequest {
                text: "systemd".to_string(),
                text_lower: "systemd".to_string(),
                include_tags: true,
                incremental: false,
            },
            &all_display,
            &all_display_lower,
            &all_tags_search,
        );

        assert_eq!(candidates, vec![(1, Some(i64::MAX))]);
    }

    #[test]
    fn current_filter_request_prefers_incremental_search() {
        let mut filter_state = FilterState {
            sort_mode: SortMode::None,
            tag_filter_text: "tag".to_string(),
        };

        let request = current_filter_request("main", "inc", &filter_state, true);
        assert_eq!(
            request,
            FilterRequest {
                text: "inc".to_string(),
                text_lower: "inc".to_string(),
                include_tags: false,
                incremental: true,
            }
        );

        filter_state.tag_filter_text.clear();
        let request = current_filter_request("main", "", &filter_state, false);
        assert_eq!(
            request,
            FilterRequest {
                text: "main".to_string(),
                text_lower: "main".to_string(),
                include_tags: false,
                incremental: false,
            }
        );
    }

    #[test]
    fn filter_request_would_narrow_detects_typing_extension() {
        let filter_state = FilterState::default();
        let previous = FilterRequest {
            text: "git".to_string(),
            text_lower: "git".to_string(),
            include_tags: false,
            incremental: false,
        };

        assert!(filter_request_would_narrow(
            "git s",
            "",
            &filter_state,
            false,
            Some(&previous)
        ));
        assert!(!filter_request_would_narrow(
            "gi",
            "",
            &filter_state,
            false,
            Some(&previous)
        ));
    }

    #[test]
    fn next_event_poll_timeout_uses_pending_debounce_deadline() {
        let last_update = Instant::now();
        let timeout = next_event_poll_timeout(true, Some(last_update), 35, false, None, 100);

        assert!(timeout <= Duration::from_millis(35));
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

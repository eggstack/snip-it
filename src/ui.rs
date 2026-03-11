//! TUI module for the snip-it snippet manager.
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
use std::sync::Mutex;
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

type Candidate = (usize, String, Vec<String>);
type PluginHandler = fn(&mut Vec<Candidate>, &[String], &[Vec<String>]) -> Option<Vec<usize>>;
type BoxedPluginHandler =
    Box<dyn Fn(&mut Vec<Candidate>, &[String], &[Vec<String>]) -> Option<Vec<usize>> + Send + Sync>;

static TERMINATE: LazyLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    LazyLock::new(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)));

#[allow(dead_code)]
pub fn set_terminate() {
    TERMINATE.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn get_terminate() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    TERMINATE.clone()
}

static MATCHER: LazyLock<SkimMatcherV2> = LazyLock::new(SkimMatcherV2::default);

static PLUGINS: LazyLock<Mutex<Vec<Plugin>>> = LazyLock::new(|| Mutex::new(Vec::new()));

#[derive(Clone, Default)]
struct FilterState {
    sort_new: bool,
    sort_old: bool,
    sort_alpha: bool,
    sort_alpha_rev: bool,
    tag_filter_text: String,
}

#[derive(Clone)]
pub struct Variable {
    pub name: String,
    pub default: Option<String>,
}

#[allow(dead_code)]
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

static ACTIVE_THEME: LazyLock<std::sync::Mutex<Theme>> = LazyLock::new(|| {
    std::sync::Mutex::new({
        let theme_name = std::env::var("SNP_THEME").unwrap_or_else(|_| "auto".to_string());
        match theme_name.as_str() {
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
    })
});

pub fn get_theme() -> std::sync::MutexGuard<'static, Theme> {
    ACTIVE_THEME.lock().unwrap()
}

#[allow(dead_code)]
pub fn set_theme(theme_name: &str) {
    let new_theme = match theme_name {
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
    };
    if let Ok(mut theme) = ACTIVE_THEME.lock() {
        *theme = new_theme;
    }
}

pub struct Plugin {
    #[allow(dead_code)]
    name: String,
    handler: BoxedPluginHandler,
}

#[allow(dead_code)]
pub fn register_plugin(name: &str, handler: PluginHandler) {
    PLUGINS.lock().unwrap().push(Plugin {
        name: name.to_string(),
        handler: Box::new(
            move |candidates: &mut Vec<Candidate>,
                  descriptions: &[String],
                  tags: &[Vec<String>]| { handler(candidates, descriptions, tags) },
        ),
    });
}

pub fn apply_plugins(
    candidates: &mut Vec<(usize, String, Vec<String>, Option<i64>)>,
    descriptions: &[String],
    tags: &[Vec<String>],
) {
    let scores: Vec<Option<i64>> = candidates.iter().map(|c| c.3).collect();
    for plugin in PLUGINS.lock().unwrap().iter() {
        let mut legacy_candidates: Vec<(usize, String, Vec<String>)> = candidates
            .iter()
            .map(|(i, d, t, _)| (*i, d.clone(), t.clone()))
            .collect();
        if let Some(filtered) = (plugin.handler)(&mut legacy_candidates, descriptions, tags) {
            *candidates = filtered
                .into_iter()
                .map(|old_idx| {
                    let (i, d, t) = legacy_candidates[old_idx].clone();
                    (i, d, t, scores.get(old_idx).copied().flatten())
                })
                .collect();
        }
    }
}

pub fn expand_command(command: &str, values: &[(String, String)]) -> String {
    let mut result = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();

    let mut usage_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut var_content = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                var_content.push(chars.next().unwrap());
            }

            let eq_pos = var_content.find('=');
            if let Some(eq_pos) = eq_pos {
                let name = var_content[..eq_pos].trim().to_string();
                let default = var_content[eq_pos + 1..].trim();

                let count = usage_count.entry(name.clone()).or_insert(0);
                let replacement = values
                    .iter()
                    .filter(|(n, _)| n.trim() == name.trim())
                    .nth(*count)
                    .map(|(_, v)| v.trim());

                *count += 1;

                let replacement = replacement.unwrap_or(default);
                result.push_str(replacement);
            } else {
                let name = var_content.trim().to_string();
                let count = usage_count.entry(name.clone()).or_insert(0);
                let replacement = values
                    .iter()
                    .filter(|(n, _)| n.trim() == name.trim())
                    .nth(*count)
                    .map(|(_, v)| v.trim());

                *count += 1;

                let replacement = replacement.unwrap_or(&name);
                result.push_str(replacement);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn extract_variables(command: &str) -> Vec<String> {
    extract_variables_for_display(command)
}

fn highlight_command(command: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = command.chars().peekable();
    let mut prev_was_backslash = false;

    let theme = get_theme();
    let color_default = TuiStyle::default().fg(theme.text);
    let color_variable = TuiStyle::default().fg(theme.accent);
    let color_keyword = TuiStyle::default().fg(theme.primary);
    let color_string = TuiStyle::default().fg(ratatui::style::Color::Green);
    let color_flag = TuiStyle::default().fg(theme.secondary);
    let color_comment = TuiStyle::default().fg(theme.muted);
    let color_escape = TuiStyle::default().fg(ratatui::style::Color::Magenta);

    let shell_keywords = &*SHELL_KEYWORDS_SET;

    while let Some(c) = chars.next() {
        if prev_was_backslash {
            prev_was_backslash = false;
            spans.push(Span::styled(format!("\\{}", c), color_escape));
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

            let is_kw = shell_keywords.iter().any(|kw| word == *kw);
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
    folders: &[Vec<String>],
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
    let _all_folders = folders.to_vec();
    let _all_favorites = favorites.to_vec();

    state.select(Some(0));

    loop {
        // Debounce: only recompute filtered list if enough time has passed since last filter change
        let should_recompute = !filter_dirty
            || last_filter_update.map_or(true, |t| {
                t.elapsed().as_millis() >= FILTER_DEBOUNCE_MS as u128
            });

        if filter_dirty && should_recompute {
            filter_dirty = false;
        }

        let has_incremental_search = !incremental_search.is_empty();
        let has_main_filter = !filter.is_empty() || !filter_state.tag_filter_text.is_empty();
        let current_filter_text = if tag_filter_mode {
            filter_state.tag_filter_text.clone()
        } else {
            filter.clone()
        };

        let mut candidates: Vec<(usize, String, Vec<String>, Option<i64>)> = if should_recompute {
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

        apply_plugins(&mut candidates, descriptions, tags);

        let has_filter = has_incremental_search || has_main_filter;
        candidates.sort_by(|a, b| {
            let score_cmp = match (a.3, b.3) {
                (Some(sa), Some(sb)) => sb.cmp(&sa),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };

            if score_cmp != std::cmp::Ordering::Equal || !has_filter {
                let explicit_sort = if filter_state.sort_new {
                    Some(b.0.cmp(&a.0))
                } else if filter_state.sort_old {
                    Some(a.0.cmp(&b.0))
                } else {
                    None
                };

                let secondary = if filter_state.sort_alpha {
                    Some(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
                } else if filter_state.sort_alpha_rev {
                    Some(b.1.to_lowercase().cmp(&a.1.to_lowercase()))
                } else {
                    None
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

        let sort_indicators: Vec<&str> = {
            let mut ind = Vec::new();
            if filter_state.sort_new {
                ind.push("new");
            }
            if filter_state.sort_old {
                ind.push("old");
            }
            if filter_state.sort_alpha {
                ind.push("a-z");
            }
            if filter_state.sort_alpha_rev {
                ind.push("z-a");
            }
            ind
        };
        let sort_indicator = if sort_indicators.is_empty() {
            String::new()
        } else {
            format!("[{}]", sort_indicators.join(","))
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
                .border_style(TuiStyle::default().fg(theme.border))
                .style(TuiStyle::default().bg(theme.background));
            let input_block_title = if tag_filter_mode {
                "Tag Filter"
            } else {
                "Filter"
            };
            let input_block = Block::default()
                .title(input_block_title)
                .borders(Borders::ALL)
                .border_style(TuiStyle::default().fg(theme.border))
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

                    // Get the pre-computed highlighted command
                    let highlighted = highlighted_commands.get(*idx).cloned().unwrap_or_else(|| Line::from(""));

                    // Combine favorite indicator with highlighted content
                    let mut spans = vec![Span::raw(fav_indicator)];
                    spans.extend(highlighted.spans);
                    let line = Line::from(spans);

                    ListItem::new(line)
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
                .style(TuiStyle::default().fg(theme.text).bg(theme.background));
            f.render_widget(
                filter_widget,
                ratatui::layout::Rect::new(
                    chunks[0].x + 1,
                    chunks[0].y + 1,
                    chunks[0].width - 2,
                    chunks[0].y + 1,
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
                    .border_style(TuiStyle::default().fg(theme.border))
                    .style(TuiStyle::default().bg(theme.background));

                let preview_content = if has_vars {
                    format!("{}\n\nVars: {}", snippet_cmd, vars.join(", "))
                } else {
                    snippet_cmd.clone()
                };

                let preview_widget = Paragraph::new(preview_content)
                    .block(preview_block)
                    .style(TuiStyle::default().fg(theme.text).bg(theme.background));
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
                        "[{}]{} | esc: normal | /: search | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear",
                        mode_str, tag_mode_str
                    )
                } else {
                    format!(
                        "[{}]{} | i: insert | y/ctrl+c: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear",
                        mode_str, tag_mode_str
                    )
                }
            } else if insert_mode {
                format!(
                    "[{}]{} | esc: normal | enter: run | ctrl+u/d : page | n/o/a/z: sort | t: tags | x/c: clear",
                    mode_str, tag_mode_str
                )
            } else {
                format!(
                    "[{}]{} | i: insert | y: copy | ctrl+u/d : page | n/o/a/z: sort | t: tags | q: quit | x/c: clear",
                    mode_str, tag_mode_str
                )
            };
            let status_widget = Paragraph::new(status_text)
                .style(TuiStyle::default().fg(theme.muted));
            let status_area = ratatui::layout::Rect::new(
                chunks[3].x + 1,
                chunks[3].y,
                chunks[3].x + chunks[3].width - 1,
                chunks[3].y + 1,
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

                        if key.code == KeyCode::Char('c')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                            && selected < filtered.len()
                        {
                            let idx = filtered[selected].0;
                            let cmd = &commands[idx];
                            let _ = clipboard::copy_to_clipboard(cmd);
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
                                    let _ = clipboard::copy_to_clipboard(&copy_text);
                                    visual_mode = false;
                                    continue;
                                }
                                _ => {}
                            }
                            state.select(Some(selected));
                            continue;
                        }

                        if key.code == KeyCode::Char('f')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            selected = (selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if key.code == KeyCode::Char('d')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            selected = (selected + 10).min(filtered.len().saturating_sub(1));
                        }

                        if key.code == KeyCode::Char('b')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            selected = selected.saturating_sub(10);
                        }

                        if key.code == KeyCode::Char('u')
                            && key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
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
                                        let cmd = &commands[idx];
                                        let _ = clipboard::copy_to_clipboard(cmd);
                                        should_copy = Some(descriptions[idx].clone());
                                        if !is_search {
                                            break;
                                        }
                                    }
                                }
                                KeyCode::Char('g')
                                    if key
                                        .modifiers
                                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                                {
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
                                KeyCode::Char('n') => {
                                    filter_state.sort_new = !filter_state.sort_new;
                                    if filter_state.sort_new {
                                        filter_state.sort_old = false;
                                    }
                                }
                                KeyCode::Char('o') => {
                                    filter_state.sort_old = !filter_state.sort_old;
                                    if filter_state.sort_old {
                                        filter_state.sort_new = false;
                                    }
                                }
                                KeyCode::Char('a') => {
                                    filter_state.sort_alpha = !filter_state.sort_alpha;
                                    if filter_state.sort_alpha {
                                        filter_state.sort_alpha_rev = false;
                                    }
                                }
                                KeyCode::Char('z') => {
                                    filter_state.sort_alpha_rev = !filter_state.sort_alpha_rev;
                                    if filter_state.sort_alpha_rev {
                                        filter_state.sort_alpha = false;
                                    }
                                }
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

#[allow(clippy::type_complexity)]
pub fn prompt_variables(vars: Vec<Variable>) -> io::Result<Option<Option<Vec<(String, String)>>>> {
    if vars.is_empty() {
        return Ok(None);
    }

    prompt_variables_inner(vars)
}

#[allow(clippy::type_complexity)]
fn prompt_variables_inner(
    vars: Vec<Variable>,
) -> io::Result<Option<Option<Vec<(String, String)>>>> {
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
                .style(TuiStyle::default().fg(theme.border));

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
                    .style(TuiStyle::default().fg(theme.text));

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
            let status_widget =
                Paragraph::new(status_text).style(TuiStyle::default().fg(theme.muted));
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
                            return Ok(None);
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
    Ok(Some(Some(result)))
}

#[allow(clippy::ptr_arg)]
#[allow(dead_code)]
pub fn folder_filter_plugin(
    candidates: &mut Vec<Candidate>,
    _descriptions: &[String],
    _tags: &[Vec<String>],
) -> Option<Vec<usize>> {
    let mut filtered_indices = Vec::new();
    for (i, _) in candidates.iter().enumerate() {
        filtered_indices.push(i);
    }
    Some(filtered_indices)
}

#[allow(dead_code)]
pub fn init_plugins() {
    register_plugin("folder_filter", folder_filter_plugin);
}

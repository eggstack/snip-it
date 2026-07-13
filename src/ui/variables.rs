//! Variable prompting dialog for snippet parameters.
//!
//! Displays a modal TUI dialog that prompts the user to fill in
//! variable values for snippets using `<name=default>` syntax.
//!
//! Modal editing mirrors the main TUI:
//!
//! - **Insert (INS)** (default): typed characters are inserted at the cursor.
//!   Pressing `q` simply inserts the letter `q` (or whatever char happens to
//!   share its keybinding in NOR mode) so common words like "queue" type
//!   naturally.
//! - **Normal (NOR)**: navigation/editing commands (h/l for cursor,
//!   j/k for variables, d for defaults, q to go back to the snippet
//!   selector, i to return to INS).
//!
//! Only `Ctrl+C` (or SIGINT/SIGTERM) exits the whole program.
//!
//! The cursor is tracked per-field as a byte index on a Unicode scalar
//! value boundary.

use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::{Line, Span};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::utils::variables::{Variable, VariableKind};

use super::get_terminate;
use super::state::is_ctrl_key;
use super::theme::{get_theme, style_fg};

/// RAII guard that restores the terminal when dropped.
/// Ensures the terminal is always restored even on early return or panic.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// Result of prompting the user for variable values.
#[derive(Debug)]
pub enum VariablePromptResult {
    /// User explicitly cancelled the program (`Ctrl+C` or SIGINT/SIGTERM).
    /// The caller should exit the program entirely.
    Cancel,
    /// User backed out of the prompt (`q`). The caller should return to
    /// the snippet selector without running the snippet.
    Back,
    /// No variables to fill (skipped the prompt entirely).
    Skip,
    /// User provided variable values.
    Values(Vec<(String, String)>),
}

/// What a single keypress in the variable prompt should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    /// Keep editing; the prompt stays open.
    Continue,
    /// Close the prompt and return to the snippet selector.
    Back,
}

pub fn prompt_variables(vars: Vec<Variable>) -> io::Result<VariablePromptResult> {
    if vars.is_empty() {
        return Ok(VariablePromptResult::Skip);
    }

    prompt_variables_inner(vars)
}

/// Editable field for a single variable value.
#[derive(Clone)]
struct Field {
    value: String,
    /// Cursor position as a byte index on a char boundary in `value`.
    cursor: usize,
}

impl Field {
    fn new(default: &str) -> Self {
        let value = default.to_string();
        let cursor = value.len();
        Self { value, cursor }
    }

    fn insert_char(&mut self, c: char) {
        let pos = self.cursor.min(self.value.len());
        self.value.insert(pos, c);
        self.cursor = pos + c.len_utf8();
    }

    /// Backspace: remove the character before the cursor.
    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.value, self.cursor);
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
    }

    /// Delete: remove the character at the cursor.
    fn delete(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let next = next_char_boundary(&self.value, self.cursor);
        self.value.drain(self.cursor..next);
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = prev_char_boundary(&self.value, self.cursor);
    }

    fn move_right(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        self.cursor = next_char_boundary(&self.value, self.cursor);
    }

    fn move_start(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }
}

/// A prompt field — either a text input or a choice selector.
#[derive(Clone)]
enum PromptField {
    /// Free-text input field.
    Text(Field),
    /// Multiple-choice selector (Pet-style variables).
    Choice {
        choices: Vec<String>,
        selected: usize,
    },
}

impl PromptField {
    fn as_text(&self) -> Option<&Field> {
        match self {
            PromptField::Text(f) => Some(f),
            _ => None,
        }
    }

    fn as_text_mut(&mut self) -> Option<&mut Field> {
        match self {
            PromptField::Text(f) => Some(f),
            _ => None,
        }
    }

    fn value_string(&self) -> String {
        match self {
            PromptField::Text(f) => f.value.clone(),
            PromptField::Choice { choices, selected } => choices[*selected].clone(),
        }
    }
}

/// Returns the byte index of the char boundary strictly before `idx` in `s`.
/// `idx` is assumed to be a char boundary (`> 0` and `<= s.len()`).
fn prev_char_boundary(s: &str, idx: usize) -> usize {
    debug_assert!(idx <= s.len());
    let mut i = idx - 1;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Returns the byte index of the char boundary strictly after `idx` in `s`.
/// `idx` is assumed to be a char boundary (`< s.len()`).
fn next_char_boundary(s: &str, idx: usize) -> usize {
    debug_assert!(idx < s.len());
    let mut i = idx + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

fn prompt_variables_inner(vars: Vec<Variable>) -> io::Result<VariablePromptResult> {
    if !std::io::stdin().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Cannot prompt for variables: no interactive terminal available",
        ));
    }

    // Deduplicate variables by name: prompt once per unique name, reuse
    // the selected value for all occurrences with the same name.
    let mut seen_names = std::collections::HashMap::new();
    let mut deduped_vars: Vec<Variable> = Vec::new();
    let mut field_index_for_var: Vec<usize> = Vec::with_capacity(vars.len());
    for var in &vars {
        if let Some(&idx) = seen_names.get(&var.name) {
            field_index_for_var.push(idx);
        } else {
            let idx = deduped_vars.len();
            seen_names.insert(var.name.clone(), idx);
            deduped_vars.push(var.clone());
            field_index_for_var.push(idx);
        }
    }

    let mut terminal = ratatui::init();
    let _guard = TerminalGuard; // Ensures terminal is restored on any exit path

    let defaults: Vec<String> = deduped_vars
        .iter()
        .map(|v| v.default.clone().unwrap_or_default())
        .collect();

    let mut fields: Vec<PromptField> = deduped_vars
        .iter()
        .map(|v| match &v.kind {
            VariableKind::Choices {
                values,
                default_index,
            } => PromptField::Choice {
                choices: values.clone(),
                selected: default_index.unwrap_or(0),
            },
            VariableKind::Required | VariableKind::DefaultValue(_) => {
                let default = v.default.as_deref().unwrap_or("");
                PromptField::Text(Field::new(default))
            }
        })
        .collect();
    let mut selected = 0usize;
    let mut show_defaults = true;
    let mut insert_mode = true;
    let terminate = get_terminate();

    loop {
        if terminate.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(VariablePromptResult::Cancel);
        }

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

            let var_chunks: Vec<_> = deduped_vars
                .iter()
                .enumerate()
                .map(|(i, _var)| {
                    let height: u16 = match &fields[i] {
                        PromptField::Choice { choices, .. } => {
                            (choices.len() as u16 + 1).min(8) // cap at 8 lines visible
                        }
                        PromptField::Text(_) => 3,
                    };
                    height
                })
                .collect();
            let total_var_height: u16 = var_chunks.iter().sum();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([
                    Constraint::Length(total_var_height),
                    Constraint::Length(1),
                ])
                .split(size);

            f.render_widget(block, size);

            let var_constraints: Vec<_> = var_chunks
                .iter()
                .map(|&h| Constraint::Length(h))
                .collect();
            let var_areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints(var_constraints)
                .split(chunks[0]);

            for (i, var) in deduped_vars.iter().enumerate() {
                let var_block = Block::default()
                    .title(var.name.as_str())
                    .borders(Borders::ALL)
                    .style(if i == selected {
                        TuiStyle::default().fg(theme.accent)
                    } else {
                        TuiStyle::default()
                    });

                match &fields[i] {
                    PromptField::Text(field) => {
                        let prefix = if i == selected { "▶ " } else { "  " };
                        let display_value = if show_defaults
                            && field.value == defaults[i]
                            && !defaults[i].is_empty()
                        {
                            format!("{} (default: {})", field.value, defaults[i])
                        } else if field.value.is_empty() {
                            "_".to_string()
                        } else {
                            field.value.clone()
                        };
                        let text = format!("{prefix}{display_value}");

                        let p = Paragraph::new(text)
                            .block(var_block)
                            .style(style_fg(theme.text));

                        f.render_widget(p, var_areas[i]);
                    }
                    PromptField::Choice { choices, selected: sel } => {
                        let area = var_areas[i];
                        // Draw the block border
                        f.render_widget(var_block, area);
                        // Render choices inside the block's inner area
                        let inner = ratatui::layout::Rect {
                            x: area.x + 1,
                            y: area.y + 1,
                            width: area.width.saturating_sub(2),
                            height: area.height.saturating_sub(2),
                        };
                        // Show at most `inner.height` choices, centered on selected
                        let visible_count = inner.height as usize;
                        let total = choices.len();
                        let scroll = if total <= visible_count {
                            0
                        } else {
                            sel.saturating_sub(visible_count / 2).min(total - visible_count)
                        };
                        for row in 0..visible_count.min(total) {
                            let idx = scroll + row;
                            if idx >= total {
                                break;
                            }
                            let is_selected = idx == *sel;
                            let marker = if is_selected { "▶ " } else { "  " };
                            let text = format!("{}{}", marker, choices[idx]);
                            let style = if is_selected {
                                style_fg(theme.accent)
                            } else {
                                style_fg(theme.text)
                            };
                            let line = Line::from(Span::styled(text, style));
                            let p = Paragraph::new(line);
                            let row_area = ratatui::layout::Rect {
                                x: inner.x,
                                y: inner.y + row as u16,
                                width: inner.width,
                                height: 1,
                            };
                            f.render_widget(p, row_area);
                        }
                    }
                }
            }

            // Position the cursor inside the selected field at the tracked
            // byte offset.
            if selected < fields.len()
                && let Some(field) = fields[selected].as_text()
            {
                use unicode_width::UnicodeWidthStr;
                let prefix_len = 2;
                let cursor_byte = field.cursor.min(field.value.len());
                let prefix_str = field.value.get(..cursor_byte).unwrap_or("");
                let cursor_x = var_areas[selected].x
                    + 1
                    + prefix_len
                    + prefix_str.width().min(u16::MAX as usize) as u16;
                let cursor_y = var_areas[selected].y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            let status_text = if insert_mode {
                "[INS] type to enter | \u{2190}/\u{2192}: move | \u{2191}/\u{2193}/Tab/ctrl+d/ctrl+u: var | Enter: save | Esc: normal"
            } else {
                "[NOR] i: insert | h/l: move | 0/$: start/end | j/k/Tab: var | x: delete | d: defaults | Enter: save | q: back | ctrl+c: quit"
            };
            let status_widget = Paragraph::new(Line::from(vec![Span::styled(
                status_text,
                style_fg(theme.muted),
            )]));
            f.render_widget(status_widget, chunks[1]);
        })?;

        let polled = event::poll(Duration::from_millis(200)).unwrap_or(false);
        if polled {
            let key_event = event::read().ok();
            if let Some(CEvent::Key(key)) = key_event
                && key.kind == KeyEventKind::Press
            {
                if is_ctrl_key(&key, 'c') {
                    terminate.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Ok(VariablePromptResult::Cancel);
                }
                let action = if insert_mode {
                    handle_insert_key(key, &mut fields, &mut selected, &defaults, &mut insert_mode)?
                } else {
                    handle_normal_key(
                        key,
                        &mut fields,
                        &mut selected,
                        &mut show_defaults,
                        &mut insert_mode,
                    )?
                };
                match action {
                    KeyAction::Back => return Ok(VariablePromptResult::Back),
                    KeyAction::Continue => {
                        if matches!(key.code, KeyCode::Enter) {
                            break;
                        }
                    }
                }
            }
        }
    }

    for (field, default) in fields.iter_mut().zip(defaults.iter()) {
        if let Some(text_field) = field.as_text_mut()
            && text_field.value.is_empty()
            && !default.is_empty()
        {
            text_field.value = default.clone();
        }
    }

    let result: Vec<(String, String)> = vars
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let field_idx = field_index_for_var[i];
            let val = if let Some(text_field) = fields[field_idx].as_text() {
                if text_field.value.is_empty() {
                    defaults[field_idx].clone()
                } else {
                    text_field.value.clone()
                }
            } else {
                fields[field_idx].value_string()
            };
            (v.name.clone(), val.trim().to_string())
        })
        .collect();
    Ok(VariablePromptResult::Values(result))
}

fn handle_insert_key(
    key: crossterm::event::KeyEvent,
    fields: &mut [PromptField],
    selected: &mut usize,
    defaults: &[String],
    insert_mode: &mut bool,
) -> io::Result<KeyAction> {
    let num_vars = fields.len();
    match key.code {
        KeyCode::Esc => {
            *insert_mode = false;
        }
        KeyCode::Up => match &mut fields[*selected] {
            PromptField::Choice { selected: sel, .. } => {
                if *sel > 0 {
                    *sel -= 1;
                }
            }
            _ => {
                if *selected > 0 {
                    *selected -= 1;
                }
            }
        },
        KeyCode::Down => match &mut fields[*selected] {
            PromptField::Choice {
                choices,
                selected: sel,
            } => {
                if *sel + 1 < choices.len() {
                    *sel += 1;
                }
            }
            _ => {
                if *selected + 1 < num_vars {
                    *selected += 1;
                }
            }
        },
        KeyCode::Tab => {
            if *selected + 1 < num_vars {
                *selected += 1;
            } else {
                *selected = 0;
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if *selected + 1 < num_vars {
                *selected += 1;
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if *selected > 0 {
                *selected -= 1;
            }
        }
        KeyCode::Left => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_left();
            }
        }
        KeyCode::Right => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_right();
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.backspace();
            }
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(field) = fields[*selected].as_text_mut() {
                if field.cursor >= field.value.len()
                    && !defaults[*selected].is_empty()
                    && field.value == defaults[*selected]
                {
                    field.clear();
                }
                field.insert_char(c);
            }
        }
        KeyCode::Enter => {}
        _ => {}
    }
    Ok(KeyAction::Continue)
}

fn handle_normal_key(
    key: crossterm::event::KeyEvent,
    fields: &mut [PromptField],
    selected: &mut usize,
    show_defaults: &mut bool,
    insert_mode: &mut bool,
) -> io::Result<KeyAction> {
    let num_vars = fields.len();
    let mut action = KeyAction::Continue;
    match key.code {
        KeyCode::Char('q') => action = KeyAction::Back,
        KeyCode::Char('i') | KeyCode::Esc => {
            *insert_mode = true;
        }
        KeyCode::Char('a') => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_right();
            }
            *insert_mode = true;
        }
        KeyCode::Char('A') => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_end();
            }
            *insert_mode = true;
        }
        KeyCode::Char('I') => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_start();
            }
            *insert_mode = true;
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_left();
            }
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_right();
            }
        }
        KeyCode::Char('0') => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_start();
            }
        }
        KeyCode::Char('$') => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.move_end();
            }
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.delete();
            }
        }
        KeyCode::Backspace => {
            if let Some(field) = fields[*selected].as_text_mut() {
                field.backspace();
            }
        }
        KeyCode::Char('j') | KeyCode::Down => match &mut fields[*selected] {
            PromptField::Choice {
                choices,
                selected: sel,
            } => {
                if *sel + 1 < choices.len() {
                    *sel += 1;
                }
            }
            _ => {
                if *selected + 1 < num_vars {
                    *selected += 1;
                }
            }
        },
        KeyCode::Char('k') | KeyCode::Up => match &mut fields[*selected] {
            PromptField::Choice { selected: sel, .. } => {
                if *sel > 0 {
                    *sel -= 1;
                }
            }
            _ => {
                if *selected > 0 {
                    *selected -= 1;
                }
            }
        },
        KeyCode::Tab => {
            if *selected + 1 < num_vars {
                *selected += 1;
            } else {
                *selected = 0;
            }
        }
        KeyCode::Char('d') => {
            *show_defaults = !*show_defaults;
        }
        KeyCode::Enter => {}
        _ => {}
    }
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_new_places_cursor_at_end() {
        let f = Field::new("hello");
        assert_eq!(f.value, "hello");
        assert_eq!(f.cursor, "hello".len());
    }

    #[test]
    fn field_insert_at_cursor() {
        let mut f = Field::new("hello");
        f.cursor = 0;
        f.insert_char('X');
        assert_eq!(f.value, "Xhello");
        assert_eq!(f.cursor, 1);
    }

    #[test]
    fn field_insert_in_middle() {
        let mut f = Field::new("hello");
        f.cursor = 2;
        f.insert_char('X');
        assert_eq!(f.value, "heXllo");
        assert_eq!(f.cursor, 3);
    }

    #[test]
    fn field_backspace_removes_previous_char() {
        let mut f = Field::new("hello");
        f.backspace();
        assert_eq!(f.value, "hell");
        assert_eq!(f.cursor, 4);
    }

    #[test]
    fn field_backspace_at_start_is_noop() {
        let mut f = Field::new("hello");
        f.cursor = 0;
        f.backspace();
        assert_eq!(f.value, "hello");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn field_delete_removes_char_at_cursor() {
        let mut f = Field::new("hello");
        f.cursor = 1;
        f.delete();
        assert_eq!(f.value, "hllo");
        assert_eq!(f.cursor, 1);
    }

    #[test]
    fn field_move_left_right() {
        let mut f = Field::new("hello");
        f.move_left();
        f.move_left();
        assert_eq!(f.cursor, 3);
        f.move_right();
        assert_eq!(f.cursor, 4);
    }

    #[test]
    fn field_handles_multibyte_chars() {
        let mut f = Field::new("héllo");
        assert_eq!(f.value.len(), 6);
        f.cursor = 1;
        f.insert_char('X');
        assert_eq!(f.value, "hXéllo");
        assert_eq!(f.cursor, 2);
        f.backspace();
        assert_eq!(f.value, "héllo");
        assert_eq!(f.cursor, 1);
    }

    #[test]
    fn field_start_end() {
        let mut f = Field::new("hello");
        f.move_start();
        assert_eq!(f.cursor, 0);
        f.move_end();
        assert_eq!(f.cursor, 5);
    }

    #[test]
    fn field_clear() {
        let mut f = Field::new("hello");
        f.clear();
        assert_eq!(f.value, "");
        assert_eq!(f.cursor, 0);
    }

    #[test]
    fn prompt_variables_with_no_vars_skips() {
        let result = prompt_variables(vec![]).unwrap();
        assert!(matches!(result, VariablePromptResult::Skip));
    }

    fn press(c: char) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char(c),
            modifiers: crossterm::event::KeyModifiers::empty(),
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn press_ctrl(c: char) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char(c),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn press_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent {
            code,
            modifiers: crossterm::event::KeyModifiers::empty(),
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn text_field(value: &str) -> PromptField {
        PromptField::Text(Field::new(value))
    }

    fn choice_field(choices: Vec<&str>, selected: usize) -> PromptField {
        PromptField::Choice {
            choices: choices.into_iter().map(String::from).collect(),
            selected,
        }
    }

    #[test]
    fn ins_mode_inserts_alphanumerics_including_q() {
        let mut fields = vec![text_field("")];
        let mut selected = 0;
        let defaults = vec![String::new()];
        let mut insert_mode = true;

        for c in "djckDJCKqQ".chars() {
            let action = handle_insert_key(
                press(c),
                &mut fields,
                &mut selected,
                &defaults,
                &mut insert_mode,
            )
            .unwrap();
            assert_eq!(action, KeyAction::Continue, "{c:?} should continue");
            assert!(insert_mode, "{c:?} should keep INS mode");
        }
        assert_eq!(fields[0].value_string(), "djckDJCKqQ");
    }

    #[test]
    fn ins_mode_q_inserts_as_char() {
        let mut fields = vec![text_field("")];
        let mut selected = 0;
        let defaults = vec![String::new()];
        let mut insert_mode = true;

        for c in "queue".chars() {
            let action = handle_insert_key(
                press(c),
                &mut fields,
                &mut selected,
                &defaults,
                &mut insert_mode,
            )
            .unwrap();
            assert_eq!(action, KeyAction::Continue);
        }
        assert_eq!(fields[0].value_string(), "queue");
    }

    #[test]
    fn ins_mode_ctrl_d_moves_to_next_var() {
        let mut fields = vec![text_field("a"), text_field("b"), text_field("c")];
        let mut selected = 0;
        let defaults = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press_ctrl('d'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(selected, 1);
        assert!(insert_mode);
    }

    #[test]
    fn ins_mode_ctrl_d_at_last_var_clamps() {
        let mut fields = vec![text_field("a"), text_field("b")];
        let mut selected = 1;
        let defaults = vec!["a".to_string(), "b".to_string()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press_ctrl('d'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(selected, 1, "should stay at last var");
    }

    #[test]
    fn ins_mode_ctrl_u_moves_to_prev_var() {
        let mut fields = vec![text_field("a"), text_field("b"), text_field("c")];
        let mut selected = 2;
        let defaults = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press_ctrl('u'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(selected, 1);
        assert!(insert_mode);
    }

    #[test]
    fn ins_mode_ctrl_u_at_first_var_clamps() {
        let mut fields = vec![text_field("a"), text_field("b")];
        let mut selected = 0;
        let defaults = vec!["a".to_string(), "b".to_string()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press_ctrl('u'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(selected, 0, "should stay at first var");
    }

    #[test]
    fn ins_mode_plain_d_still_inserts() {
        let mut fields = vec![text_field("")];
        let mut selected = 0;
        let defaults = vec![String::new()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press('d'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(fields[0].value_string(), "d");
    }

    #[test]
    fn ins_mode_plain_u_still_inserts() {
        let mut fields = vec![text_field("")];
        let mut selected = 0;
        let defaults = vec![String::new()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press('u'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(fields[0].value_string(), "u");
    }

    #[test]
    fn ins_mode_typing_into_default_clears_then_inserts() {
        let mut fields = vec![text_field("any.com")];
        let mut selected = 0;
        let defaults = vec!["any.com".to_string()];
        let mut insert_mode = true;

        for c in "dandy.com".chars() {
            let action = handle_insert_key(
                press(c),
                &mut fields,
                &mut selected,
                &defaults,
                &mut insert_mode,
            )
            .unwrap();
            assert_eq!(action, KeyAction::Continue);
        }
        assert_eq!(fields[0].value_string(), "dandy.com");
    }

    #[test]
    fn nor_mode_q_returns_back_action() {
        let mut fields = vec![text_field("hello")];
        let mut selected = 0;
        let mut show_defaults = true;
        let mut insert_mode = false;

        let action = handle_normal_key(
            press('q'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Back);
        assert_eq!(fields[0].value_string(), "hello");
    }

    #[test]
    fn ins_mode_ctrl_c_is_not_treated_as_char() {
        let mut fields = vec![text_field("")];
        let mut selected = 0;
        let defaults = vec![String::new()];
        let mut insert_mode = true;

        let action = handle_insert_key(
            press_ctrl('c'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Continue);
        assert_eq!(fields[0].value_string(), "", "Ctrl+C must not insert 'c'");
    }

    #[test]
    fn test_variable_prompt_result_cancel_is_distinct_from_skip() {
        let cancel = VariablePromptResult::Cancel;
        let skip = VariablePromptResult::Skip;
        assert_ne!(
            std::mem::discriminant(&cancel),
            std::mem::discriminant(&skip),
            "Cancel and Skip must be distinct enum variants"
        );
    }

    #[test]
    fn test_variable_prompt_cancel_returns_empty_values() {
        let cancel = VariablePromptResult::Cancel;
        let values = match cancel {
            VariablePromptResult::Values(v) => v,
            _ => vec![],
        };
        assert!(
            values.is_empty(),
            "Cancel variant must not carry any values"
        );
    }

    #[test]
    fn test_variable_prompt_skip_returns_empty_values() {
        let result = prompt_variables(vec![]).unwrap();
        assert!(
            matches!(result, VariablePromptResult::Skip),
            "Empty vars should produce Skip, got {:?}",
            result
        );
        let values = match result {
            VariablePromptResult::Values(v) => v,
            _ => vec![],
        };
        assert!(values.is_empty(), "Skip variant must not carry any values");
    }

    // ========================================================================
    // Choice field tests
    // ========================================================================

    #[test]
    fn choice_field_value_string_returns_selected() {
        let f = choice_field(vec!["red", "green", "blue"], 1);
        assert_eq!(f.value_string(), "green");
    }

    #[test]
    fn choice_field_value_string_first() {
        let f = choice_field(vec!["a", "b", "c"], 0);
        assert_eq!(f.value_string(), "a");
    }

    #[test]
    fn choice_field_value_string_last() {
        let f = choice_field(vec!["x", "y", "z"], 2);
        assert_eq!(f.value_string(), "z");
    }

    #[test]
    fn prompt_field_is_text() {
        assert!(text_field("hello").as_text().is_some());
        assert!(choice_field(vec!["a", "b"], 0).as_text().is_none());
    }

    #[test]
    fn prompt_field_as_text() {
        assert!(text_field("hello").as_text().is_some());
        assert!(choice_field(vec!["a", "b"], 0).as_text().is_none());
    }

    #[test]
    fn ins_mode_up_down_on_text_moves_between_fields() {
        let mut fields = vec![text_field("a"), text_field("b"), text_field("c")];
        let mut selected = 1;
        let defaults = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Up),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 0);

        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 1);

        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 2);

        // Down at end clamps
        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 2);
    }

    #[test]
    fn ins_mode_up_down_on_choice_cycles_choices() {
        let mut fields = vec![choice_field(vec!["a", "b", "c"], 0)];
        let mut selected = 0;
        let defaults = vec!["a".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 0); // stays on same field
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }

        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 2),
            _ => panic!("expected choice"),
        }

        // Down at end clamps
        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 2),
            _ => panic!("expected choice"),
        }

        handle_insert_key(
            press_key(KeyCode::Up),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }

        // Up at start clamps
        handle_insert_key(
            press_key(KeyCode::Up),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }

        handle_insert_key(
            press_key(KeyCode::Up),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn nor_mode_jk_on_choice_cycles_choices() {
        let mut fields = vec![choice_field(vec!["red", "green", "blue"], 0)];
        let mut selected = 0;
        let mut show_defaults = true;
        let mut insert_mode = false;

        handle_normal_key(
            press('j'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }

        handle_normal_key(
            press('j'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 2),
            _ => panic!("expected choice"),
        }

        handle_normal_key(
            press('k'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn mixed_text_and_choice_fields_tab_navigates() {
        let mut fields = vec![
            text_field("default"),
            choice_field(vec!["opt1", "opt2"], 0),
            text_field("other"),
        ];
        let mut selected = 0;
        let defaults = vec![
            "default".to_string(),
            "opt1".to_string(),
            "other".to_string(),
        ];
        let mut insert_mode = true;

        // Tab from text to choice
        handle_insert_key(
            press_key(KeyCode::Tab),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 1);

        // Tab from choice to text
        handle_insert_key(
            press_key(KeyCode::Tab),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 2);

        // Tab wraps around
        handle_insert_key(
            press_key(KeyCode::Tab),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(selected, 0);
    }

    #[test]
    fn choice_field_left_right_are_noop() {
        let mut fields = vec![choice_field(vec!["a", "b"], 0)];
        let mut selected = 0;
        let defaults = vec!["a".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Left),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        handle_insert_key(
            press_key(KeyCode::Right),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn choice_field_backspace_is_noop() {
        let mut fields = vec![choice_field(vec!["a", "b"], 1)];
        let mut selected = 0;
        let defaults = vec!["a".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Backspace),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn choice_field_char_input_is_noop() {
        let mut fields = vec![choice_field(vec!["a", "b"], 0)];
        let mut selected = 0;
        let defaults = vec!["a".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press('z'),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn nor_mode_arrow_keys_on_choice() {
        let mut fields = vec![choice_field(vec!["x", "y", "z"], 0)];
        let mut selected = 0;
        let mut show_defaults = true;
        let mut insert_mode = false;

        handle_normal_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 1),
            _ => panic!("expected choice"),
        }

        handle_normal_key(
            press_key(KeyCode::Up),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn choice_field_single_choice() {
        let mut fields = vec![choice_field(vec!["only"], 0)];
        let mut selected = 0;
        let defaults = vec!["only".to_string()];
        let mut insert_mode = true;

        // Down at end of single choice clamps
        handle_insert_key(
            press_key(KeyCode::Down),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Choice { selected: s, .. } => assert_eq!(*s, 0),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn text_field_left_right_still_work() {
        let mut fields = vec![text_field("hello")];
        let mut selected = 0;
        let defaults = vec!["hello".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Left),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Text(f) => assert_eq!(f.cursor, 4),
            _ => panic!("expected text"),
        }

        handle_insert_key(
            press_key(KeyCode::Right),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        match &fields[0] {
            PromptField::Text(f) => assert_eq!(f.cursor, 5),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn text_field_backspace_still_works() {
        let mut fields = vec![text_field("hello")];
        let mut selected = 0;
        let defaults = vec!["hello".to_string()];
        let mut insert_mode = true;

        handle_insert_key(
            press_key(KeyCode::Backspace),
            &mut fields,
            &mut selected,
            &defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(fields[0].value_string(), "hell");
    }

    #[test]
    fn choice_field_nor_mode_q_still_returns_back() {
        let mut fields = vec![choice_field(vec!["a", "b"], 0)];
        let mut selected = 0;
        let mut show_defaults = true;
        let mut insert_mode = false;

        let action = handle_normal_key(
            press('q'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert_eq!(action, KeyAction::Back);
    }

    #[test]
    fn choice_field_nor_mode_i_enters_insert() {
        let mut fields = vec![choice_field(vec!["a", "b"], 0)];
        let mut selected = 0;
        let mut show_defaults = true;
        let mut insert_mode = false;

        handle_normal_key(
            press('i'),
            &mut fields,
            &mut selected,
            &mut show_defaults,
            &mut insert_mode,
        )
        .unwrap();
        assert!(insert_mode);
    }
}

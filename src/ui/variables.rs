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
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::text::{Line, Span};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::utils::variables::Variable;

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
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard; // Ensures terminal is restored on any exit path

    let defaults: Vec<String> = vars
        .iter()
        .map(|v| v.default.clone().unwrap_or_default())
        .collect();

    let mut fields: Vec<Field> = defaults.iter().map(|d| Field::new(d)).collect();
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

            let num_vars = fields.len();
            let var_height = num_vars * 3;
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([
                    Constraint::Length(var_height.min(u16::MAX as usize) as u16),
                    Constraint::Length(1),
                ])
                .split(size);

            f.render_widget(block, size);

            let var_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Length(3); num_vars])
                .split(chunks[0]);

            for (i, var) in vars.iter().enumerate() {
                let var_block = Block::default()
                    .title(var.name.as_str())
                    .borders(Borders::ALL)
                    .style(if i == selected {
                        TuiStyle::default().fg(theme.accent)
                    } else {
                        TuiStyle::default()
                    });

                let prefix = if i == selected { "▶ " } else { "  " };
                let display_value = if show_defaults
                    && fields[i].value == defaults[i]
                    && !defaults[i].is_empty()
                {
                    format!("{} (default: {})", fields[i].value, defaults[i])
                } else if fields[i].value.is_empty() {
                    "_".to_string()
                } else {
                    fields[i].value.clone()
                };
                let text = format!("{prefix}{display_value}");

                let p = Paragraph::new(text)
                    .block(var_block)
                    .style(style_fg(theme.text));

                f.render_widget(p, var_chunks[i]);
            }

            // Position the cursor inside the selected field at the tracked
            // byte offset.
            if selected < fields.len() {
                use unicode_width::UnicodeWidthStr;
                let prefix_len = 2;
                let field = &fields[selected];
                let cursor_byte = field.cursor.min(field.value.len());
                let prefix_str = field.value.get(..cursor_byte).unwrap_or("");
                let cursor_x = var_chunks[selected].x
                    + 1
                    + prefix_len
                    + prefix_str.width().min(u16::MAX as usize) as u16;
                let cursor_y = var_chunks[selected].y + 1;
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
        if field.value.is_empty() && !default.is_empty() {
            field.value = default.clone();
        }
    }

    let result: Vec<(String, String)> = vars
        .iter()
        .zip(fields.iter())
        .map(|(v, f)| (v.name.clone(), f.value.trim().to_string()))
        .collect();
    Ok(VariablePromptResult::Values(result))
}

fn handle_insert_key(
    key: crossterm::event::KeyEvent,
    fields: &mut [Field],
    selected: &mut usize,
    defaults: &[String],
    insert_mode: &mut bool,
) -> io::Result<KeyAction> {
    let num_vars = fields.len();
    match key.code {
        KeyCode::Esc => {
            *insert_mode = false;
        }
        KeyCode::Up => {
            if *selected > 0 {
                *selected -= 1;
            }
        }
        KeyCode::Down => {
            if *selected + 1 < num_vars {
                *selected += 1;
            }
        }
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
            fields[*selected].move_left();
        }
        KeyCode::Right => {
            fields[*selected].move_right();
        }
        KeyCode::Backspace => {
            fields[*selected].backspace();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let field = &mut fields[*selected];
            if field.cursor >= field.value.len()
                && !defaults[*selected].is_empty()
                && field.value == defaults[*selected]
            {
                field.clear();
            }
            field.insert_char(c);
        }
        KeyCode::Enter => {}
        _ => {}
    }
    Ok(KeyAction::Continue)
}

fn handle_normal_key(
    key: crossterm::event::KeyEvent,
    fields: &mut [Field],
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
            fields[*selected].move_right();
            *insert_mode = true;
        }
        KeyCode::Char('A') => {
            fields[*selected].move_end();
            *insert_mode = true;
        }
        KeyCode::Char('I') => {
            fields[*selected].move_start();
            *insert_mode = true;
        }
        KeyCode::Char('h') | KeyCode::Left => {
            fields[*selected].move_left();
        }
        KeyCode::Char('l') | KeyCode::Right => {
            fields[*selected].move_right();
        }
        KeyCode::Char('0') => {
            fields[*selected].move_start();
        }
        KeyCode::Char('$') => {
            fields[*selected].move_end();
        }
        KeyCode::Char('x') | KeyCode::Delete => {
            fields[*selected].delete();
        }
        KeyCode::Backspace => {
            fields[*selected].backspace();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if *selected + 1 < num_vars {
                *selected += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if *selected > 0 {
                *selected -= 1;
            }
        }
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

    #[test]
    fn ins_mode_inserts_alphanumerics_including_q() {
        let mut fields = vec![Field::new("")];
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
        assert_eq!(fields[0].value, "djckDJCKqQ");
    }

    #[test]
    fn ins_mode_q_inserts_as_char() {
        // 'q' is reserved as "back to selector" in NOR mode, but in INS mode
        // it must type as a regular character so words like "queue" or "q1"
        // can be entered.
        let mut fields = vec![Field::new("")];
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
        assert_eq!(fields[0].value, "queue");
    }

    #[test]
    fn ins_mode_ctrl_d_moves_to_next_var() {
        let mut fields = vec![Field::new("a"), Field::new("b"), Field::new("c")];
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
        let mut fields = vec![Field::new("a"), Field::new("b")];
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
        let mut fields = vec![Field::new("a"), Field::new("b"), Field::new("c")];
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
        let mut fields = vec![Field::new("a"), Field::new("b")];
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
        let mut fields = vec![Field::new("")];
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
        assert_eq!(fields[0].value, "d");
    }

    #[test]
    fn ins_mode_plain_u_still_inserts() {
        let mut fields = vec![Field::new("")];
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
        assert_eq!(fields[0].value, "u");
    }

    #[test]
    fn ins_mode_typing_into_default_clears_then_inserts() {
        let mut fields = vec![Field::new("any.com")];
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
        assert_eq!(fields[0].value, "dandy.com");
    }

    #[test]
    fn nor_mode_q_returns_back_action() {
        let mut fields = vec![Field::new("hello")];
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
        assert_eq!(fields[0].value, "hello");
    }

    #[test]
    fn ins_mode_ctrl_c_is_not_treated_as_char() {
        // The dispatcher checks Ctrl+C before the handler, but verify the
        // INS handler itself would not blindly insert a control-modified
        // character: with the CONTROL guard, Char(c) is only matched when
        // the modifier is absent.
        let mut fields = vec![Field::new("")];
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
        assert_eq!(fields[0].value, "", "Ctrl+C must not insert 'c'");
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
}

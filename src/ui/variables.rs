//! Variable prompting dialog for snippet parameters.
//!
//! Displays a modal TUI dialog that prompts the user to fill in
//! variable values for snippets using `<name=default>` syntax.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use ratatui::text::{Line, Span};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style as TuiStyle,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::utils::variables::Variable;

use super::theme::{get_theme, style_fg};

/// Result of prompting the user for variable values.
pub enum VariablePromptResult {
    /// User cancelled the operation (pressed q).
    Cancel,
    /// User chose to skip (pressed enter with defaults).
    Skip,
    /// User provided variable values.
    Values(Vec<(String, String)>),
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

            let num_vars = values.len();
            let var_height = num_vars * 3;
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([Constraint::Length(var_height as u16), Constraint::Length(1)])
                .split(size);

            f.render_widget(block, size);

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

            if selected < values.len() {
                let var_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(vec![Constraint::Length(3); num_vars])
                    .split(chunks[0]);
                let prefix_len = 2;
                let cursor_x =
                    var_chunks[selected].x + 1 + prefix_len + values[selected].len().min(u16::MAX as usize) as u16;
                let cursor_y = var_chunks[selected].y + 1;
                f.set_cursor_position((cursor_x, cursor_y));
            }

            let status_text =
                "↑/↓/ j/k: move | tab: next | enter: save | esc: back | q: cancel | d: defaults";
            let warning_text = "Values are interpolated directly into shell commands. Do not enter untrusted input.";
            let status_widget = Paragraph::new(Line::from(vec![
                Span::styled(status_text, style_fg(theme.muted)),
                Span::raw("  "),
                Span::styled(warning_text, style_fg(theme.accent)),
            ]));
            f.render_widget(status_widget, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(200))?
            && let CEvent::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') => {
                    ratatui::restore();
                    return Ok(VariablePromptResult::Cancel);
                }
                KeyCode::Esc => {
                    // Esc no longer quits - use q instead
                }
                KeyCode::Up | KeyCode::Char('k') if selected > 0 => {
                    selected -= 1;
                }
                KeyCode::Down | KeyCode::Char('j') if selected + 1 < values.len() => {
                    selected += 1;
                }
                KeyCode::Tab => {
                    if selected + 1 < values.len() {
                        selected += 1;
                    } else {
                        selected = 0;
                    }
                }
                KeyCode::Enter => {
                    for (i, val) in values.iter_mut().enumerate() {
                        if val.is_empty() && !defaults[i].is_empty() {
                            *val = defaults[i].clone();
                        }
                    }
                    break;
                }
                KeyCode::Backspace => {
                    values[selected].pop();
                }
                KeyCode::Char('d') => {
                    show_defaults = !show_defaults;
                }
                KeyCode::Char(c) => {
                    if values[selected] == defaults[selected] && !defaults[selected].is_empty() {
                        values[selected] = String::new();
                    }
                    values[selected].push(c);
                }
                _ => {}
            }
        }
    }

    ratatui::restore();

    let result: Vec<(String, String)> = vars
        .iter()
        .zip(values.iter())
        .map(|(v, val)| (v.name.clone(), val.trim().to_string()))
        .collect();
    Ok(VariablePromptResult::Values(result))
}

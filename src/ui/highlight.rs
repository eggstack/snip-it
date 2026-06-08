//! Syntax highlighting for shell commands.
//!
//! Tokenizes shell commands into keywords, flags, strings, variables,
//! escape sequences, and comments, applying theme-aware colors to each.

use ratatui::text::{Line, Span};

use crate::utils::shell_keywords::SHELL_KEYWORDS_SET;

use super::theme::{get_theme, style_fg};

pub(crate) fn highlight_command(command: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = command.chars().peekable();
    let mut prev_was_backslash = false;

    let theme = get_theme();
    let color_default = style_fg(theme.text);
    let color_variable = style_fg(theme.accent);
    let color_keyword = style_fg(theme.primary);
    let color_string = style_fg(theme.string_color);
    let color_flag = style_fg(theme.secondary);
    let color_comment = style_fg(theme.muted);
    let color_escape = style_fg(theme.escape_color);

    let shell_keywords = &*SHELL_KEYWORDS_SET;

    while let Some(c) = chars.next() {
        if prev_was_backslash {
            prev_was_backslash = false;
            if c == '<' || c == '>' {
                spans.push(Span::styled(c.to_string(), color_default));
            } else {
                spans.push(Span::styled(format!("\\{c}"), color_escape));
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
                if let Some(c) = chars.next() {
                    var_content.push(c);
                }
            }
            spans.push(Span::styled(format!("<{var_content}>"), color_variable));
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
                    if let Some(c) = chars.next() {
                        flag.push(c);
                    }
                } else {
                    break;
                }
            }
            spans.push(Span::styled(flag, color_flag));
        } else if c == '-' {
            let mut flag = String::from(c);
            if let Some(&next) = chars.peek()
                && next.is_alphabetic()
            {
                if let Some(c) = chars.next() {
                    flag.push(c);
                }
            }
            spans.push(Span::styled(flag, color_flag));
        } else if c.is_whitespace() {
            let mut ws = String::from(c);
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    if let Some(c) = chars.next() {
                        ws.push(c);
                    }
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
                    if let Some(c) = chars.next() {
                        word.push(c);
                    }
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
}

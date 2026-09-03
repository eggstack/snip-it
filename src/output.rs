//! **Layer: Domain/Core**
//!
//! Shared presentation model for snippet output metadata.
//!
//! The `output` field on [`Snippet`](crate::library::Snippet) stores
//! descriptive metadata (notes, example output, reminders) that travels
//! with a snippet. This module provides safe rendering helpers for
//! terminal display without interpreting escape sequences or executing content.

/// Maximum characters of output to include when scoring for fuzzy search.
pub const OUTPUT_SEARCH_BUDGET: usize = 512;

/// Summary of an output field for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputPresentation<'a> {
    raw: &'a str,
}

impl<'a> OutputPresentation<'a> {
    /// Create a new presentation from the raw output string.
    pub fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    /// Whether the output field has content.
    #[allow(dead_code)]
    pub fn is_present(&self) -> bool {
        !self.raw.is_empty()
    }

    /// The raw, unmodified output value.
    #[allow(dead_code)]
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    /// Line count of the output content.
    #[allow(dead_code)]
    pub fn line_count(&self) -> usize {
        if self.raw.is_empty() {
            0
        } else {
            self.raw.lines().count()
        }
    }

    /// A short single-line summary suitable for inline display.
    /// Truncates to `max_chars` and appends "..." if truncated.
    pub fn summary(&self, max_chars: usize) -> String {
        if self.raw.is_empty() {
            return String::new();
        }
        let first_line = self.raw.lines().next().unwrap_or("");
        let sanitized = sanitize_for_terminal(first_line);
        if sanitized.len() <= max_chars {
            sanitized
        } else {
            let truncated: String = sanitized
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect();
            format!("{truncated}...")
        }
    }

    /// Full multiline content with terminal control sequences neutralized.
    pub fn display(&self) -> String {
        sanitize_for_terminal(self.raw)
    }

    /// Content truncated to a bounded number of lines, with a note if truncated.
    #[allow(dead_code)]
    pub fn display_bounded(&self, max_lines: usize) -> String {
        if self.raw.is_empty() {
            return String::new();
        }
        let lines: Vec<&str> = self.raw.lines().collect();
        let total = lines.len();
        if total <= max_lines {
            sanitize_for_terminal(self.raw)
        } else {
            let truncated: String = lines[..max_lines]
                .iter()
                .map(|l| sanitize_for_terminal(l))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{truncated}\n... ({total} lines total)")
        }
    }

    /// Content suitable for fuzzy-match scoring.
    /// Returns a bounded substring to avoid pathological inputs.
    pub fn for_scoring(&self) -> String {
        let budget = OUTPUT_SEARCH_BUDGET;
        if self.raw.len() <= budget {
            sanitize_for_terminal(self.raw)
        } else {
            let safe_budget = if self.raw.is_char_boundary(budget) {
                budget
            } else {
                self.raw
                    .char_indices()
                    .take_while(|&(i, _)| i < budget)
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            };
            sanitize_for_terminal(&self.raw[..safe_budget])
        }
    }
}

/// Neutralize terminal control sequences for safe human display.
///
/// Strips ANSI SGR codes (colors, bold, etc.), OSC sequences (hyperlinks,
/// terminal titles), and C0 control characters except newline and tab.
/// Does not mutate the stored value — only the presentation copy.
pub fn sanitize_for_terminal(input: &str) -> String {
    use std::iter::Peekable;
    use std::str::Chars;
    let mut result = String::with_capacity(input.len());
    let mut chars: Peekable<Chars> = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            // ESC — start of escape sequence
            '\x1b' => match chars.next() {
                None => break,
                Some('[') => {
                    // CSI sequence: ESC [ <params> <final byte>
                    // Skip parameter bytes (0x30-0x3F)
                    while matches!(chars.peek(), Some('\x30'..='\x3f')) {
                        chars.next();
                    }
                    // Skip intermediate bytes (0x20-0x2F)
                    while matches!(chars.peek(), Some('\x20'..='\x2f')) {
                        chars.next();
                    }
                    // Skip final byte (0x40-0x7E)
                    if matches!(chars.peek(), Some('\x40'..='\x7e')) {
                        chars.next();
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] <params> ST or BEL
                    // Skip until ST (ESC \) or BEL (\x07)
                    loop {
                        match chars.next() {
                            None => break,
                            Some('\x07') => break,
                            Some('\x1b') => {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                    break;
                                }
                            }
                            Some(_) => {}
                        }
                    }
                }
                // Other escape: ESC + char already consumed; drop both.
                Some(_) => {}
            },
            // Preserve newline and tab
            '\n' | '\t' => result.push(c),
            // Strip other C0 control characters (0x00-0x1F except \n \t)
            '\x00'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' => {}
            // Strip DEL
            '\x7f' => {}
            // Strip C1 control characters (0x80-0x9F)
            '\u{0080}'..='\u{009f}' => {}
            _ => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_output() {
        let p = OutputPresentation::new("");
        assert!(!p.is_present());
        assert_eq!(p.line_count(), 0);
        assert_eq!(p.summary(50), "");
        assert_eq!(p.display(), "");
        assert_eq!(p.display_bounded(10), "");
        assert_eq!(p.for_scoring(), "");
    }

    #[test]
    fn test_single_line() {
        let p = OutputPresentation::new("hello world");
        assert!(p.is_present());
        assert_eq!(p.line_count(), 1);
        assert_eq!(p.summary(50), "hello world");
        assert_eq!(p.display(), "hello world");
    }

    #[test]
    fn test_summary_truncation() {
        let p = OutputPresentation::new("this is a long output");
        assert_eq!(p.summary(10), "this is...");
    }

    #[test]
    fn test_multiline() {
        let p = OutputPresentation::new("line1\nline2\nline3");
        assert_eq!(p.line_count(), 3);
        assert_eq!(p.display(), "line1\nline2\nline3");
    }

    #[test]
    fn test_display_bounded_truncation() {
        let p = OutputPresentation::new("a\nb\nc\nd\ne");
        let result = p.display_bounded(3);
        assert!(result.contains("a\nb\nc"));
        assert!(result.contains("5 lines total"));
    }

    #[test]
    fn test_display_bounded_no_truncation() {
        let p = OutputPresentation::new("a\nb");
        assert_eq!(p.display_bounded(10), "a\nb");
    }

    #[test]
    fn test_sanitize_ansi_colors() {
        let input = "\x1b[31mred text\x1b[0m";
        assert_eq!(sanitize_for_terminal(input), "red text");
    }

    #[test]
    fn test_sanitize_osc_hyperlink() {
        let input = "\x1b]8;;https://example.com\x1b\\click here\x1b]8;;\x1b\\";
        assert_eq!(sanitize_for_terminal(input), "click here");
    }

    #[test]
    fn test_sanitize_osc_bel_terminated() {
        let input = "\x1b]0;window title\x07";
        assert_eq!(sanitize_for_terminal(input), "");
    }

    #[test]
    fn test_preserves_newline_and_tab() {
        let input = "line1\nline2\ttab";
        assert_eq!(sanitize_for_terminal(input), "line1\nline2\ttab");
    }

    #[test]
    fn test_strips_control_chars() {
        let input = "hello\x00world\x08test";
        assert_eq!(sanitize_for_terminal(input), "helloworldtest");
    }

    #[test]
    fn test_strips_del() {
        let input = "before\x7fafter";
        assert_eq!(sanitize_for_terminal(input), "beforeafter");
    }

    #[test]
    fn test_for_scoring_budget() {
        let long = "a".repeat(1000);
        let p = OutputPresentation::new(&long);
        assert!(p.for_scoring().len() <= OUTPUT_SEARCH_BUDGET);
    }

    #[test]
    fn test_for_scoring_within_budget() {
        let short = "hello";
        let p = OutputPresentation::new(short);
        assert_eq!(p.for_scoring(), "hello");
    }

    #[test]
    fn test_for_scoring_multibyte_utf8_no_panic() {
        // "中" is 3 bytes in UTF-8. 171 chars = 513 bytes, so byte 512
        // lands mid-character. This must not panic.
        let cjk = "中".repeat(171);
        assert!(cjk.len() > OUTPUT_SEARCH_BUDGET);
        let p = OutputPresentation::new(&cjk);
        let scored = p.for_scoring();
        assert!(scored.len() <= OUTPUT_SEARCH_BUDGET);
        // Result must be valid UTF-8 (sanitized_for_terminal preserves validity).
        assert!(!scored.is_empty());
    }
}

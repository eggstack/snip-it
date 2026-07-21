//! TOML escape sequence handling.
//!
//! Handles edge cases with backslash-containing strings in TOML configuration.
//!
//! # Problem
//!
//! TOML double-quoted strings interpret `\<` as an escape sequence, which fails
//! because `\<` is not a valid TOML escape. This module converts such strings
//! to single-quoted TOML strings which are raw literals.
//!
//! # Scope
//!
//! Both helpers operate on hand-written TOML files that pre-date this code
//! path. snip-it's own save pipeline writes `toml::to_string_pretty` output
//! verbatim and does not call these helpers on its own output. That is
//! deliberate: the helpers cannot reliably distinguish TOML escape sequences
//! from content inside triple-quoted multi-line strings, and converting
//! basic-string `\t`, `\r`, `\n` escapes into a single-quoted raw literal
//! would silently corrupt those bytes. The helpers therefore scan only
//! single-line double-quoted strings and skip triple-quoted regions.

/// Applies `needs_fix` to each single-line double-quoted TOML string,
/// rewriting it as a single-quoted raw literal (or with escaped backslashes
/// inside double quotes) when the condition is true.
///
/// The scanner is hand-written so it correctly recognizes TOML token
/// boundaries: line and block comments, table / array-of-tables headers,
/// keys, single-quoted literal strings, multi-line basic strings
/// (`"""..."""`), multi-line literal strings (`'''...'''`), and ordinary
/// single-line basic strings. Only single-line basic strings are passed to
/// `needs_fix`. Triple-quoted multi-line regions are left untouched because
/// they always contain valid TOML escapes and the helper cannot safely
/// rewrite them.
fn fix_toml_strings(toml_str: &str, needs_fix: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(toml_str.len());
    let bytes = toml_str.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        // Skip line comments until end of line.
        if c == b'#' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push_str(&toml_str[start..i]);
            continue;
        }

        // Whitespace and structural TOML bytes pass through unchanged.
        if c <= b' ' {
            out.push(c as char);
            i += 1;
            continue;
        }

        // Multi-line basic string: """ ...""" — copy verbatim, never rewrite.
        if c == b'"' && bytes.get(i + 1) == Some(&b'"') && bytes.get(i + 2) == Some(&b'"') {
            let start = i;
            i += 3;
            while i < bytes.len() {
                if bytes[i] == b'"'
                    && bytes.get(i + 1) == Some(&b'"')
                    && bytes.get(i + 2) == Some(&b'"')
                {
                    i += 3;
                    break;
                }
                i += 1;
            }
            out.push_str(&toml_str[start..i]);
            continue;
        }

        // Multi-line literal string: '''...''' — copy verbatim.
        if c == b'\'' && bytes.get(i + 1) == Some(&b'\'') && bytes.get(i + 2) == Some(&b'\'') {
            let start = i;
            i += 3;
            while i < bytes.len() {
                if bytes[i] == b'\''
                    && bytes.get(i + 1) == Some(&b'\'')
                    && bytes.get(i + 2) == Some(&b'\'')
                {
                    i += 3;
                    break;
                }
                i += 1;
            }
            out.push_str(&toml_str[start..i]);
            continue;
        }

        // Single-line literal string: 'foo' — copy verbatim (no escapes).
        if c == b'\'' {
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'\'' {
                i += 1;
            }
            out.push_str(&toml_str[start..i]);
            continue;
        }

        // Single-line basic string: "foo" — capture content, possibly rewrite.
        if c == b'"' {
            let start = i;
            i += 1;
            let mut content = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\n' {
                    // Unterminated basic string — pass through unchanged.
                    out.push_str(&toml_str[start..i]);
                    i += 1;
                    content.clear();
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    content.push(bytes[i] as char);
                    content.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    content.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i >= bytes.len() {
                out.push_str(&toml_str[start..i]);
                break;
            }
            if bytes[i] == b'\n' {
                continue;
            }
            i += 1;

            if needs_fix(&content) {
                if content.contains('\'') {
                    let escaped = content.replace('\\', "\\\\");
                    out.push('"');
                    out.push_str(&escaped);
                    out.push('"');
                } else {
                    let unescaped = content.replace("\\\\", "\\").replace("\\\"", "\"");
                    out.push('\'');
                    out.push_str(&unescaped);
                    out.push('\'');
                }
            } else {
                // Reuse the original bytes so non-ASCII UTF-8 survives intact.
                out.push_str(&toml_str[start..i]);
            }
            continue;
        }

        out.push(c as char);
        i += 1;
    }

    out
}

/// Converts double-quoted strings in TOML to single-quoted strings if they contain
/// backslashes. This is necessary because TOML double-quoted strings interpret
/// backslash-escape sequences, while single-quoted strings are raw literals.
///
/// For strings containing both backslashes and single quotes, the backslash is
/// escaped instead (using \\) to maintain valid TOML with double quotes.
///
/// **Do not call this on output produced by `toml::to_string_pretty`.** The
/// serializer already picks the correct quoting style for any string and uses
/// TOML native escapes (`\t`, `\r`, `\n`) inside basic strings. Running this
/// helper on that output silently corrupts tabs, carriage returns, and newlines
/// (the basic-string `\t` becomes the two literal characters `\` and `t` in the
/// raw single-quoted string) and it also mangles triple-quoted multi-line
/// strings. snip-it's own save pipeline writes `toml::to_string_pretty` output
/// verbatim. This helper is retained for callers that hand-write TOML and need
/// the same conversion.
#[allow(dead_code)]
pub(crate) fn quote_strings_containing_backslashes(toml_str: &str) -> String {
    fix_toml_strings(toml_str, |content| content.contains('\\'))
}

/// Fixes invalid TOML escape sequences in single-line double-quoted strings
/// before parsing. Handles `\<` and `\>` which are not valid TOML escape
/// sequences.
///
/// For double-quoted strings containing `\<` or `\>`:
/// - If no single quotes in string: converts the string to single-quoted (preserving content)
/// - If single quotes present: escapes backslash with \\ in double quotes
///
/// Multi-line basic strings (`"""..."""`) are passed through verbatim because
/// the serializer always emits them with valid TOML escape sequences inside.
pub fn fix_invalid_toml_escapes(toml_str: &str) -> String {
    fix_toml_strings(toml_str, |content| {
        content.contains("\\<") || content.contains("\\>")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_change_when_no_backslash() {
        let input = r#"key = "hello world""#;
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_backslash_converted_to_single_quotes() {
        let input = "command = \"ping \\<website\\>\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, "command = 'ping \\<website\\>'");
    }

    #[test]
    fn test_backslash_and_single_quote_escaped() {
        let input = "command = \"echo 'test\\\\value'\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, "command = \"echo 'test\\\\\\\\value'\"");
    }

    #[test]
    fn test_multiple_strings() {
        let input = "command = \"ping \\<website\\>\"\nname = \"test\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, "command = 'ping \\<website\\>'\nname = \"test\"");
    }

    #[test]
    fn test_nested_quotes() {
        let input = r#"command = "echo hello""#;
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_empty_string() {
        let input = r#"key = """#;
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_escaped_backslash_in_double_quotes() {
        let input = r#"path = "C:\\Users\\test""#;
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, r#"path = 'C:\Users\test'"#);
    }

    #[test]
    fn test_fix_invalid_escape_simple() {
        let input = r#"command = "sudo iptables-restore \< /path""#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, r#"command = 'sudo iptables-restore \< /path'"#);
    }

    #[test]
    fn test_fix_invalid_escape_with_single_quotes() {
        let input = r#"command = "echo 'test \<value'""#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, r#"command = "echo 'test \\<value'""#);
    }

    #[test]
    fn test_fix_invalid_escape_multiple() {
        let input = r#"command = "echo \<foo\> and \<bar\>""#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, r#"command = 'echo \<foo\> and \<bar\>'"#);
    }

    #[test]
    fn test_fix_invalid_escape_no_change_needed() {
        let input = r#"command = "echo hello""#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_fix_invalid_escape_with_single_quote_and_invalid_escape() {
        let input = r#"command = "it's a \<test\>""#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, r#"command = "it's a \\<test\\>""#);
    }

    #[test]
    fn test_fix_invalid_escape_with_escaped_quote() {
        let input = "command = \"it's a \\\"test\\\" \\<value\\>\"";
        let result = fix_invalid_toml_escapes(input);
        assert!(result.contains("\\\\\"test\\\\\""));
        assert!(result.contains("\\\\<value\\\\>"));
    }

    #[test]
    fn test_fix_invalid_escape_in_single_quoted() {
        let input = r#"command = 'echo \<test\>'"#;
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_fix_invalid_escape_in_triple_quoted_basic_string() {
        let input = "command = \"\"\"echo \\<start\\>\nend\"\"\"\n";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_fix_invalid_escape_in_triple_quoted_basic_string_with_crlf() {
        let input = "command = \"\"\"echo \\<start\\>\r\necho \\<end\\>\"\"\"\n";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_quote_strings_in_triple_quoted_basic_string_with_backslash() {
        let input = "command = \"\"\"path \\\\foo\\<bar\\>\"\"\"\n";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_fix_invalid_escape_in_triple_quoted_literal_string() {
        let input = "command = '''echo \\<literal\\>\n'''\n";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_quote_strings_with_triple_quote_inside_double_quoted() {
        let input = "key = \"contains \"\" pair\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_line_comment_with_backslash_passes_through() {
        let input = "# legacy: command = \"bad \\<thing\\>\"\nkey = \"ok\"";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_multiple_strings_mixed() {
        let input =
            "a = \"hello\\<x\\>\"\nb = 'plain'\nc = \"\"\"multi\nline\\<y\\>\"\"\"\nd = \"plain\"";
        let result = fix_invalid_toml_escapes(input);
        assert!(result.contains("a = 'hello\\<x\\>'"));
        assert!(result.contains("b = 'plain'"));
        assert!(result.contains("c = \"\"\"multi\nline\\<y\\>\"\"\""));
        assert!(result.contains("d = \"plain\""));
    }

    #[test]
    fn test_utf8_strings_preserved_in_double_quoted() {
        let input = "description = \"Ünïcödé test 🎉\"";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_utf8_strings_preserved_in_single_quoted() {
        let input = "description = 'Ünïcödé test 🎉'";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_utf8_strings_preserved_in_triple_quoted() {
        let input = "description = \"\"\"Ünïcödé test 🎉\"\"\"";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_utf8_strings_preserved_when_quote_strings_helper_runs() {
        let input = "description = \"Ünïcödé test 🎉\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_save_then_load_round_trip_with_escapes_and_crlf() {
        // The input TOML below is the literal byte-for-byte output produced
        // by `toml::to_string_pretty` for a snippet whose `command` Rust
        // String is `echo \<start\>\r\necho \<end\>`. The scanner must leave
        // the triple-quoted region alone.
        let toml = "\
[[snippets]]
description = \"desc\"
command = \"\"\"echo \\\\<start\\\\>\r\necho \\\\<end\\\\>\"\"\"
tag = []
output = \"\"
";
        let fixed = fix_invalid_toml_escapes(toml);
        assert_eq!(fixed, toml);
        #[derive(serde::Deserialize)]
        struct S {
            snippets: Vec<Snippet>,
        }
        #[derive(serde::Deserialize)]
        struct Snippet {
            command: String,
        }
        let parsed: S = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.snippets.len(), 1);
        assert_eq!(
            parsed.snippets[0].command,
            "echo \\<start\\>\r\necho \\<end\\>"
        );
    }
}

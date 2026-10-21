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
//! path. The helpers scan only single-line double-quoted strings and skip
//! triple-quoted regions: they cannot reliably distinguish TOML escape
//! sequences from content inside triple-quoted multi-line strings.
//!
//! A rewritten string is decoded to the value it denotes and re-emitted with
//! the quoting style that can represent that value — a raw literal string
//! when the value allows one, a basic string otherwise. The parsed value is
//! therefore unchanged, including for valid escapes (`\t`, `\r`, `\n`,
//! `\uXXXX`, ...) that a raw literal could not represent.

/// Applies `needs_fix` to each single-line double-quoted TOML string,
/// rewriting it when the condition is true.
///
/// A rewritten string is decoded to the value it denotes and re-emitted: as a
/// single-quoted raw literal when that value can be represented by one, and
/// as a double-quoted basic string with proper escapes otherwise. The parsed
/// value is preserved either way.
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
        if c == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
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
            let body_start = i;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    closed = true;
                    break;
                }
                if bytes[i] == b'\n' {
                    break;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    // Skip the escape pair so the captured body keeps the
                    // original escape text (`\r`, `\"`, ...) intact.
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if !closed {
                // Unterminated basic string (newline or EOF) — pass through
                // verbatim so the parse error stays attributable to the real
                // input instead of scanner-mangled output.
                out.push_str(&toml_str[start..i]);
                if i < bytes.len() {
                    out.push('\n');
                    i += 1;
                }
                continue;
            }
            let body_end = i;
            i += 1;

            // Slice the ORIGINAL source range. Both ends sit on ASCII
            // boundaries, so the slice is valid UTF-8 and every non-ASCII
            // byte survives byte-for-byte. (Accumulating bytes and mapping
            // each one through Latin-1 mojibaked UTF-8 content.)
            let body = &toml_str[body_start..body_end];

            if needs_fix(body) {
                // Decode the basic string to the value it denotes, then
                // re-emit it so the parsed value is unchanged. Re-emitting a
                // raw literal would freeze `\t`/`\r`/`\n` into two literal
                // characters, so control characters force the basic form.
                let value = decode_basic_string(body);
                if can_be_literal_string(&value) {
                    out.push('\'');
                    out.push_str(&value);
                    out.push('\'');
                } else {
                    out.push('"');
                    out.push_str(&encode_basic_string(&value));
                    out.push('"');
                }
            } else {
                // Reuse the original bytes so non-ASCII UTF-8 survives intact.
                out.push_str(&toml_str[start..i]);
            }
            continue;
        }

        // Structural byte — copy the whole UTF-8 character so non-ASCII bare
        // keys and table headers survive.
        match toml_str[i..].chars().next() {
            Some(ch) => {
                out.push(ch);
                i += ch.len_utf8();
            }
            None => {
                out.push(c as char);
                i += 1;
            }
        }
    }

    out
}

/// Decodes the body of a TOML basic string into the value it denotes.
///
/// Valid TOML escapes (`\b`, `\e`, `\f`, `\n`, `\r`, `\t`, `\"`, `\\`,
/// `\uXXXX`, `\UXXXXXXXX`) are decoded. Any other backslash sequence — `\<`
/// and `\>` in particular — is kept as the two literal characters `\` and the
/// escaped character, which is what the hand-written legacy files mean.
fn decode_basic_string(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('\\') => {
                chars.next();
                out.push('\\');
            }
            Some('"') => {
                chars.next();
                out.push('"');
            }
            Some('b') => {
                chars.next();
                out.push('\u{8}');
            }
            Some('e') => {
                chars.next();
                out.push('\u{1b}');
            }
            Some('f') => {
                chars.next();
                out.push('\u{c}');
            }
            Some('n') => {
                chars.next();
                out.push('\n');
            }
            Some('r') => {
                chars.next();
                out.push('\r');
            }
            Some('t') => {
                chars.next();
                out.push('\t');
            }
            Some(c @ ('u' | 'U')) => {
                let width = if c == 'u' { 4 } else { 8 };
                chars.next();
                let mut hex = String::with_capacity(width);
                let mut probe = chars.clone();
                for _ in 0..width {
                    match probe.peek().copied() {
                        Some(h) if h.is_ascii_hexdigit() => {
                            hex.push(h);
                            probe.next();
                        }
                        _ => break,
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(cp) if hex.len() == width => {
                        for _ in 0..width {
                            chars.next();
                        }
                        out.push(cp);
                    }
                    _ => {
                        // Not a valid `\u`/`\U` escape — keep the backslash
                        // and the escape character as literal text.
                        out.push('\\');
                        out.push(c);
                    }
                }
            }
            // Invalid escape (e.g. `\<`): keep the backslash. The escaped
            // character is pushed by the next iteration.
            Some(_) | None => out.push('\\'),
        }
    }
    out
}

/// Encodes a string value as TOML basic-string content (no surrounding quotes).
fn encode_basic_string(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{8}' => out.push_str("\\b"),
            '\u{1b}' => out.push_str("\\e"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) <= 0x1f || c as u32 == 0x7f => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// `true` when the value can be written as a TOML literal string
/// (single-quoted): no `'` and no control characters other than tab.
fn can_be_literal_string(value: &str) -> bool {
    !value.contains('\'')
        && !value.chars().any(|c| {
            let n = c as u32;
            (n < 0x20 && n != 0x09) || n == 0x7f
        })
}

/// Converts double-quoted strings in TOML to single-quoted strings if they contain
/// backslashes. This is necessary because TOML double-quoted strings interpret
/// backslash-escape sequences, while single-quoted strings are raw literals.
///
/// For strings whose value contains both backslashes and single quotes, the
/// value is re-emitted as a double-quoted basic string with the backslashes
/// escaped (using \\), because a literal string cannot hold a single quote.
/// The parsed value is the same either way.
///
/// **Do not call this on output produced by `toml::to_string_pretty`.** The
/// serializer already picks the correct quoting style for any string and uses
/// TOML native escapes (`\t`, `\r`, `\n`) inside basic strings; rewriting that
/// output changes the quoting style for no reason. snip-it's own save pipeline
/// writes `toml::to_string_pretty` output verbatim. This helper is retained
/// for callers that hand-write TOML and need the same conversion.
#[allow(dead_code)]
pub(crate) fn quote_strings_containing_backslashes(toml_str: &str) -> String {
    fix_toml_strings(toml_str, |content| content.contains('\\'))
}

/// Fixes invalid TOML escape sequences in single-line double-quoted strings
/// before parsing. Handles `\<` and `\>` which are not valid TOML escape
/// sequences.
///
/// For double-quoted strings containing `\<` or `\>`:
/// - The string is decoded to its value and re-emitted as a single-quoted raw
///   literal when that value allows one
/// - Otherwise it is re-emitted as a double-quoted basic string with proper
///   escapes
///
/// In both cases the parsed value is unchanged.
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
        // The source text holds one escaped backslash (`\\`), so the parsed
        // value is `echo 'test\value'`. The rewrite must preserve that value
        // rather than doubling the backslash.
        let input = "command = \"echo 'test\\\\value'\"";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, "command = \"echo 'test\\\\value'\"");
        #[derive(serde::Deserialize)]
        struct S {
            command: String,
        }
        let parsed: S = toml::from_str(&result).unwrap();
        assert_eq!(parsed.command, "echo 'test\\value'");
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
        assert!(result.contains("\\\"test\\\""));
        assert!(result.contains("\\\\<value\\\\>"));
        // The parsed value must be the original text: `it's a "test" \<value\>`.
        #[derive(serde::Deserialize)]
        struct S {
            command: String,
        }
        let parsed: S = toml::from_str(&result).unwrap();
        assert_eq!(parsed.command, "it's a \"test\" \\<value\\>");
    }

    #[test]
    fn test_unterminated_basic_string_at_newline_not_duplicated() {
        // Input ending right after the newline must pass through unchanged —
        // not duplicated by the close-quote epilogue.
        let input = "key = \"hello\n";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_unterminated_basic_string_with_trailing_content_not_mangled() {
        // Content after the unterminated string's newline must not be swallowed.
        let input = "key = \"abc\ndef";
        let result = fix_invalid_toml_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_unterminated_basic_string_at_eof_passthrough() {
        let input = "key = \"hello";
        let result = quote_strings_containing_backslashes(input);
        assert_eq!(result, input);
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
    fn test_hash_inside_bare_key_is_not_a_comment() {
        let input = "a#b = \"value\" # comment\n";
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

    #[derive(serde::Deserialize)]
    struct CmdOnly {
        command: String,
    }

    #[derive(serde::Deserialize)]
    struct SnippetTable {
        snippets: Vec<CmdOnly>,
    }

    #[test]
    fn test_fix_invalid_escape_preserves_carriage_return_escape() {
        // Source text emitted by `toml::to_string_pretty` for the Rust value
        // `a<CR>b\<c`. The `\r` is a valid TOML escape and must survive the
        // rewrite; only the invalid `\<` needs attention.
        let input = "command = \"a\\rb\\\\<c\"";
        let fixed = fix_invalid_toml_escapes(input);
        let parsed: CmdOnly = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.command, "a\rb\\<c");
    }

    #[test]
    fn test_fix_invalid_escape_preserves_tab_escape() {
        let input = "command = \"a\\tb\\\\<c\"";
        let fixed = fix_invalid_toml_escapes(input);
        let parsed: CmdOnly = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.command, "a\tb\\<c");
    }

    #[test]
    fn test_fix_invalid_escape_preserves_non_ascii_when_rewriting() {
        // Rewriting a string that also contains `\<` must not mojibake its
        // non-ASCII content.
        let input = "command = \"caf\u{e9} \\\\<x\\\\>\"";
        let fixed = fix_invalid_toml_escapes(input);
        assert!(fixed.contains("caf\u{e9}"), "mojibake: {fixed}");
        let parsed: CmdOnly = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.command, "caf\u{e9} \\<x\\>");
    }

    #[test]
    fn test_fix_invalid_escape_preserves_non_ascii_in_bare_keys() {
        // Non-ASCII outside a string value must survive too.
        let input = "caf\u{e9} = \"v\"";
        assert_eq!(fix_invalid_toml_escapes(input), input);
    }

    #[test]
    fn test_fix_invalid_escape_preserves_unicode_escape() {
        let input = "command = \"\\u00e9\\<x\"";
        let fixed = fix_invalid_toml_escapes(input);
        let parsed: CmdOnly = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.command, "\u{e9}\\<x");
    }

    #[test]
    fn test_library_file_round_trip_with_lone_cr_and_escaped_brackets() {
        // End-to-end shape of the corruption: a snippet whose command holds a
        // lone CR (so `toml::to_string_pretty` emits a single-line basic
        // string) plus the legacy `\<` convention.
        let toml = "[[snippets]]\ncommand = \"a\\rb\\\\<c\"\n";
        let fixed = fix_invalid_toml_escapes(toml);
        let parsed: SnippetTable = toml::from_str(&fixed).unwrap();
        assert_eq!(parsed.snippets.len(), 1);
        assert_eq!(parsed.snippets[0].command, "a\rb\\<c");
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

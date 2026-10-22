//! TOML escape sequence handling for premade libraries.
//!
//! The server loads premade library files that are written by hand, so they
//! may contain escape sequences that are invalid in TOML (`\<` and `\>`).
//! Those strings are rewritten before parsing.
//!
//! A rewritten string is decoded to the value it denotes and re-emitted with
//! the quoting style that can represent that value — a raw literal string when
//! the value allows one, a basic string otherwise. The parsed value is
//! therefore unchanged, including for valid escapes (`\t`, `\r`, `\n`,
//! `\uXXXX`, ...) that a raw literal could not represent.
//!
//! This is the same algorithm as the client's `src/utils/toml_helpers.rs`.
//! The two crates are separate, so the implementation is duplicated rather
//! than shared; the client copy is the reference.

/// Fixes invalid TOML escape sequences (`\<` and `\>`) in single-line
/// double-quoted strings.
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

/// Applies `needs_fix` to each single-line double-quoted TOML string,
/// rewriting it when the condition is true.
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
            // byte survives byte-for-byte.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Snippet {
        command: String,
    }

    #[test]
    fn test_invalid_escape_is_repaired_without_changing_the_value() {
        let input = r#"command = "echo \<hello\>""#;
        let output = fix_invalid_toml_escapes(input);
        let parsed: Snippet = toml::from_str(&output).unwrap();
        assert_eq!(parsed.command, r"echo \<hello\>");
    }

    #[test]
    fn test_valid_escapes_survive_the_repair() {
        // The value contains a real newline. A raw literal string cannot hold
        // it, so the rewrite must fall back to a basic string with `\n`.
        let input = "command = \"echo \\\\<x\\>\\nnext\"";
        let output = fix_invalid_toml_escapes(input);
        let parsed: Snippet = toml::from_str(&output).unwrap();
        assert_eq!(parsed.command, "echo \\<x\\>\nnext");
    }

    #[test]
    fn test_no_fix_needed_leaves_the_document_untouched() {
        let input = r#"command = "echo normal""#;
        assert_eq!(fix_invalid_toml_escapes(input), input);
    }

    #[test]
    fn test_single_quoted_strings_are_untouched() {
        let input = r#"command = 'echo \<hello\>'"#;
        assert_eq!(fix_invalid_toml_escapes(input), input);
    }

    #[test]
    fn test_multiline_basic_strings_are_untouched() {
        let input = "command = \"\"\"echo \\\\<x\\>\"\"\"";
        assert_eq!(fix_invalid_toml_escapes(input), input);
    }
}

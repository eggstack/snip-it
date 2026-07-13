//! Variable expansion for dynamic snippets.
//!
//! Handles parsing and expansion of `<variable>` and `<variable=default>` syntax
//! in snippet commands. Supports escaped angle brackets (`\<` and `\>`).
//! Also supports Pet-compatible multiple-choice syntax:
//! `<name=|_opt1_||_opt2_||_opt3_||>`.

/// Severity level for a variable diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

/// A diagnostic message produced during variable parsing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VariableDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub span: Option<std::ops::Range<usize>>,
}

/// The kind of a parsed variable, determining how it should be prompted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariableKind {
    /// `<name>` — required, no default value.
    Required,
    /// `<name=default>` — has a default value.
    DefaultValue(String),
    /// `<name=|_opt1_||_opt2_||_opt3_||>` — multiple choice (Pet syntax).
    Choices {
        values: Vec<String>,
        default_index: Option<usize>,
    },
}

/// A parsed variable from a snippet command.
#[derive(Clone)]
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
    /// Backward-compatible convenience field: `None` for Required, `Some(val)` for DefaultValue,
    /// `Some(first_choice)` for Choices.
    pub default: Option<String>,
}

/// Strips escape sequences from a command string.
/// Converts `\<` to `<`, `\>` to `>`, and `\\` to `\` for shell execution.
///
/// This should be called whenever a command is copied or executed,
/// regardless of whether it contains variables.
#[allow(clippy::while_let_on_iterator)]
pub fn strip_escape_sequences(command: &str) -> String {
    let mut result = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    let mut prev_char_was_backslash = false;

    while let Some(c) = chars.next() {
        let is_prev_unescaped_backslash = prev_char_was_backslash;
        prev_char_was_backslash = false;

        if c == '\\' {
            if is_prev_unescaped_backslash {
                result.push('\\');
            } else {
                prev_char_was_backslash = true;
            }
            continue;
        }

        if is_prev_unescaped_backslash {
            match c {
                '<' | '>' => result.push(c),
                _ => {
                    result.push('\\');
                    result.push(c);
                }
            }
            continue;
        }

        result.push(c);
    }

    if prev_char_was_backslash {
        result.push('\\');
    }

    result
}

/// Detects if the content after `=` matches Pet multiple-choice syntax.
/// The syntax is `|_opt1_||_opt2_||_opt3_||` — starts with `|_`, ends with `||`.
fn is_choice_syntax(default_content: &str) -> bool {
    default_content.starts_with("|_") && default_content.ends_with("||")
}

/// Extracts individual choice values from Pet choice syntax `|_opt1_||_opt2_||_opt3_||`.
/// Returns `None` if the syntax is malformed.
///
/// The syntax is built from individual choices `|_value_|` concatenated with `|` separators.
/// E.g., three choices: `|_red_||_green_||_blue_||`
/// where the trailing `||` is the final `_|` + separator `|`.
fn extract_choices(choice_content: &str) -> Option<Vec<String>> {
    // choice_content is the part after `=`, e.g. `|_red_||_green_||_blue_||`
    //
    // Strategy: scan for `|_` openers and matching `_|` closers.
    let mut choices = Vec::new();
    let bytes = choice_content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len - 1 {
        // Look for `|_` opener
        if bytes[i] == b'|' && bytes[i + 1] == b'_' {
            i += 2; // skip `|_`
            // Find the matching `_|`
            let start = i;
            let mut found = false;
            while i < len - 1 {
                if bytes[i] == b'_' && bytes[i + 1] == b'|' {
                    let value = &choice_content[start..i];
                    choices.push(value.to_string());
                    i += 2; // skip `_|`
                    found = true;
                    break;
                }
                i += 1;
            }
            if !found {
                // Malformed — unclosed choice
                return None;
            }
        } else {
            i += 1;
        }
    }

    if choices.is_empty() {
        return None;
    }
    Some(choices)
}

/// Internal representation returned by `extract_variable_tokens`.
#[derive(Clone)]
enum TokenKind {
    /// `<name>` — required
    Required,
    /// `<name=default>` — has a default value
    DefaultValue(String),
    /// `<name=|_opt1_||_opt2_||>` — multiple choice
    Choices(Vec<String>),
}

#[derive(Clone)]
struct VariableToken {
    name: String,
    kind: TokenKind,
}

fn extract_variable_tokens(command: &str) -> Vec<VariableToken> {
    let mut tokens = Vec::new();
    let mut chars = command.chars().peekable();
    let mut prev_char_was_backslash = false;

    while let Some(c) = chars.next() {
        let is_prev_unescaped_backslash = prev_char_was_backslash;
        prev_char_was_backslash = false;

        if c == '\\' {
            if is_prev_unescaped_backslash {
            } else {
                prev_char_was_backslash = true;
            }
            continue;
        }

        if is_prev_unescaped_backslash && c == '<' {
            continue;
        }

        if c == '<' {
            let mut var_content = String::new();
            let mut depth = 1;
            while let Some(&next) = chars.peek() {
                if next == '\\' {
                    chars.next();
                    if let Some(&escaped) = chars.peek() {
                        match escaped {
                            '\\' => {
                                var_content.push('\\');
                                chars.next();
                            }
                            '<' => {
                                var_content.push('<');
                                chars.next();
                            }
                            '>' => {
                                var_content.push('>');
                                chars.next();
                            }
                            _ => {
                                var_content.push('\\');
                            }
                        }
                    } else {
                        var_content.push('\\');
                    }
                } else if next == '<' {
                    depth += 1;
                    if let Some(c) = chars.next() {
                        var_content.push(c);
                    }
                } else if next == '>' {
                    depth -= 1;
                    if depth == 0 {
                        chars.next();
                        break;
                    }
                    if let Some(c) = chars.next() {
                        var_content.push(c);
                    }
                } else if let Some(c) = chars.next() {
                    var_content.push(c);
                }
            }

            if !var_content.is_empty() && depth == 0 {
                let token = if let Some(eq_pos) = var_content.find('=') {
                    let name = var_content[..eq_pos].trim().to_string();
                    let default_val = var_content[eq_pos + 1..].trim().to_string();

                    if !default_val.is_empty() && is_choice_syntax(&default_val) {
                        if let Some(choices) = extract_choices(&default_val) {
                            VariableToken {
                                name,
                                kind: TokenKind::Choices(choices),
                            }
                        } else {
                            // Malformed choice syntax — fall back to default value
                            VariableToken {
                                name,
                                kind: TokenKind::DefaultValue(default_val),
                            }
                        }
                    } else if default_val.is_empty() {
                        VariableToken {
                            name,
                            kind: TokenKind::Required,
                        }
                    } else {
                        VariableToken {
                            name,
                            kind: TokenKind::DefaultValue(default_val),
                        }
                    }
                } else {
                    VariableToken {
                        name: var_content.trim().to_string(),
                        kind: TokenKind::Required,
                    }
                };
                // Skip empty variable names (e.g., bare `<>`)
                if !token.name.is_empty() {
                    tokens.push(token);
                }
            }
        }
    }
    tokens
}

pub fn parse_variables(command: &str) -> Vec<Variable> {
    extract_variable_tokens(command)
        .into_iter()
        .map(|token| {
            let (kind, default) = match token.kind {
                TokenKind::Required => (VariableKind::Required, None),
                TokenKind::DefaultValue(ref val) => {
                    (VariableKind::DefaultValue(val.clone()), Some(val.clone()))
                }
                TokenKind::Choices(ref choices) => {
                    let default_val = choices.first().cloned();
                    (
                        VariableKind::Choices {
                            values: choices.clone(),
                            default_index: Some(0),
                        },
                        default_val,
                    )
                }
            };
            Variable {
                name: token.name,
                kind,
                default,
            }
        })
        .collect()
}

/// Parses variables from a command string, returning both the variables and
/// any diagnostic messages (warnings or errors) encountered during parsing.
///
/// Diagnostics include:
/// - Malformed choice syntax (e.g., `<name=|_unclosed`)
/// - Empty choices list
/// - Duplicate variable names
#[allow(dead_code)]
pub fn parse_variables_diagnostics(command: &str) -> (Vec<Variable>, Vec<VariableDiagnostic>) {
    let tokens = extract_variable_tokens(command);
    let mut diagnostics = Vec::new();
    let mut seen_names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // Check for malformed choice syntax by scanning the raw command for
    // choice-like patterns that the token extractor silently falls back on.
    let mut scan_chars = command.chars().peekable();
    let mut scan_prev_was_backslash = false;
    let mut scan_pos = 0;
    while let Some(c) = scan_chars.next() {
        let is_prev = scan_prev_was_backslash;
        scan_prev_was_backslash = false;
        if c == '\\' {
            if !is_prev {
                scan_prev_was_backslash = true;
            }
            scan_pos += c.len_utf8();
            continue;
        }
        if is_prev && c == '<' {
            scan_pos += c.len_utf8();
            continue;
        }
        if c == '<' {
            // Found an unescaped '<' — scan to matching '>'
            let start_pos = scan_pos;
            let mut depth = 1;
            let mut inner = String::new();
            while let Some(&next) = scan_chars.peek() {
                if next == '\\' {
                    scan_chars.next();
                    scan_pos += 1;
                    if let Some(&escaped) = scan_chars.peek() {
                        match escaped {
                            '\\' => {
                                inner.push('\\');
                                scan_chars.next();
                                scan_pos += 1;
                            }
                            '<' => {
                                inner.push('<');
                                scan_chars.next();
                                scan_pos += 1;
                            }
                            '>' => {
                                inner.push('>');
                                scan_chars.next();
                                scan_pos += 1;
                            }
                            _ => {
                                inner.push('\\');
                            }
                        }
                    } else {
                        inner.push('\\');
                    }
                } else if next == '<' {
                    depth += 1;
                    if let Some(c) = scan_chars.next() {
                        inner.push(c);
                        scan_pos += c.len_utf8();
                    }
                } else if next == '>' {
                    depth -= 1;
                    scan_chars.next();
                    scan_pos += 1;
                    if depth == 0 {
                        break;
                    }
                    inner.push(next);
                } else if let Some(c) = scan_chars.next() {
                    inner.push(c);
                    scan_pos += c.len_utf8();
                }
            }

            if depth > 0 {
                // Unclosed bracket — not a variable, skip diagnostic for this
                // (the existing parse_variables already handles this by returning
                // no variable). Only warn if it looks like it was intended to be
                // a choice (contains `=|_`).
                scan_pos += c.len_utf8();
                continue;
            }

            let full = inner.trim();
            if let Some(eq_pos) = full.find('=') {
                let name = full[..eq_pos].trim().to_string();
                let default_val = full[eq_pos + 1..].trim().to_string();
                if !default_val.is_empty() && default_val.starts_with("|_") {
                    // Looks like choice syntax — validate it
                    if default_val.ends_with("||") {
                        // Check that extract_choices can parse it
                        if extract_choices(&default_val).is_none() {
                            diagnostics.push(VariableDiagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!("Malformed choice syntax for variable '{name}'"),
                                span: Some(start_pos..start_pos + c.len_utf8() + full.len() + 1),
                            });
                        } else if let Some(choices) = extract_choices(&default_val)
                            && choices.is_empty()
                        {
                            diagnostics.push(VariableDiagnostic {
                                severity: DiagnosticSeverity::Warning,
                                message: format!("Empty choices list for variable '{name}'"),
                                span: Some(start_pos..start_pos + c.len_utf8() + full.len() + 1),
                            });
                        }
                    } else if !default_val.ends_with("_|") || default_val == "|_" {
                        // Starts with `|_` but doesn't close properly
                        diagnostics.push(VariableDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            message: format!("Unclosed choice syntax for variable '{name}'"),
                            span: Some(start_pos..start_pos + c.len_utf8() + full.len() + 1),
                        });
                    }
                }
            }
        }
        scan_pos += c.len_utf8();
    }

    // Check for duplicate variable names
    for token in &tokens {
        let count = seen_names.entry(token.name.clone()).or_insert(0);
        *count += 1;
        if *count == 2 {
            diagnostics.push(VariableDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!("Duplicate variable name '{name}'", name = token.name),
                span: None,
            });
        }
    }

    let variables: Vec<Variable> = tokens
        .into_iter()
        .map(|token| {
            let (kind, default) = match token.kind {
                TokenKind::Required => (VariableKind::Required, None),
                TokenKind::DefaultValue(ref val) => {
                    (VariableKind::DefaultValue(val.clone()), Some(val.clone()))
                }
                TokenKind::Choices(ref choices) => {
                    let default_val = choices.first().cloned();
                    (
                        VariableKind::Choices {
                            values: choices.clone(),
                            default_index: Some(0),
                        },
                        default_val,
                    )
                }
            };
            Variable {
                name: token.name,
                kind,
                default,
            }
        })
        .collect();

    (variables, diagnostics)
}

pub fn extract_variables_for_display(command: &str) -> Vec<String> {
    extract_variable_tokens(command)
        .into_iter()
        .map(|token| match token.kind {
            TokenKind::Required => format!("{} (prompt)", token.name),
            TokenKind::DefaultValue(ref val) => format!("{} = {}", token.name, val),
            TokenKind::Choices(ref choices) => {
                let display = choices.join(" | ");
                format!("{} = [{}]", token.name, display)
            }
        })
        .collect()
}

#[allow(clippy::while_let_on_iterator)]
pub fn has_unmatched_angle_bracket(command: &str) -> bool {
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
            continue;
        }
        if c == '<' {
            let mut depth = 1;
            while let Some(&next) = chars.peek() {
                if next == '\\' {
                    chars.next();
                    if matches!(chars.peek(), Some('\\' | '<' | '>')) {
                        chars.next();
                    }
                } else if next == '<' {
                    depth += 1;
                    chars.next();
                } else if next == '>' {
                    depth -= 1;
                    chars.next();
                    if depth == 0 {
                        break;
                    }
                } else {
                    chars.next();
                }
            }
            if depth > 0 {
                return true;
            }
        }
    }
    false
}

pub fn expand_command(command: &str, values: &[(String, String)]) -> String {
    let tokens: Vec<String> = extract_variable_tokens(command)
        .into_iter()
        .map(|token| token.name)
        .collect();
    let mut result = String::with_capacity(command.len());
    let mut chars = command.chars().peekable();
    let mut token_idx = 0;
    let mut usage_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut prev_char_was_backslash = false;

    while let Some(c) = chars.next() {
        let is_prev_unescaped_backslash = prev_char_was_backslash;
        prev_char_was_backslash = false;

        if c == '\\' {
            if is_prev_unescaped_backslash {
                result.push('\\');
            } else {
                prev_char_was_backslash = true;
            }
            continue;
        }

        if is_prev_unescaped_backslash && c == '<' {
            result.push('<');
            prev_char_was_backslash = false;
            continue;
        }

        if c == '<' {
            let mut var_content = String::new();
            let mut depth = 1;
            while let Some(&next) = chars.peek() {
                if next == '\\' {
                    chars.next();
                    if let Some(&escaped) = chars.peek() {
                        match escaped {
                            '\\' => {
                                var_content.push('\\');
                                chars.next();
                            }
                            '<' => {
                                var_content.push('<');
                                chars.next();
                            }
                            '>' => {
                                var_content.push('>');
                                chars.next();
                            }
                            _ => {
                                var_content.push('\\');
                            }
                        }
                    } else {
                        var_content.push('\\');
                    }
                } else if next == '<' {
                    depth += 1;
                    if let Some(c) = chars.next() {
                        var_content.push(c);
                    }
                } else if next == '>' {
                    depth -= 1;
                    if depth == 0 {
                        chars.next();
                        break;
                    }
                    if let Some(c) = chars.next() {
                        var_content.push(c);
                    }
                } else if let Some(c) = chars.next() {
                    var_content.push(c);
                }
            }

            if let Some(name) = tokens.get(token_idx).filter(|n| **n == var_content.trim()) {
                token_idx += 1;
                let count = usage_count.entry(name.clone()).or_insert(0);
                let replacement = values
                    .iter()
                    .filter(|(n, _)| n.trim() == name.trim())
                    .nth(*count)
                    .map(|(_, v)| v.trim());
                *count += 1;

                match replacement {
                    Some(val) => result.push_str(val),
                    None => {
                        tracing::debug!(variable = %name, "No value provided for variable, using raw name");
                        result.push_str(name);
                    }
                }
            } else {
                result.push('<');
                result.push_str(&var_content);
                if depth == 0 {
                    result.push('>');
                }
            }
        } else {
            result.push(c);
        }
    }

    if prev_char_was_backslash {
        result.push('\\');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_variables_simple() {
        let vars = parse_variables("<name>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "name");
        assert_eq!(vars[0].default, None);
    }

    #[test]
    fn test_parse_variables_with_default() {
        let vars = parse_variables("<name=default>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "name");
        assert_eq!(vars[0].default, Some("default".to_string()));
    }

    #[test]
    fn test_parse_variables_multiple() {
        let vars = parse_variables("<a> and <b=val> and <c>");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].name, "a");
        assert_eq!(vars[1].name, "b");
        assert_eq!(vars[1].default, Some("val".to_string()));
        assert_eq!(vars[2].name, "c");
    }

    #[test]
    fn test_parse_variables_no_vars() {
        let vars = parse_variables("echo hello");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_extract_variables_for_display() {
        let vars = extract_variables_for_display("<name>");
        assert_eq!(vars.len(), 1);
        assert!(vars[0].contains("name"));
        assert!(vars[0].contains("prompt"));
    }

    #[test]
    fn test_extract_variables_for_display_with_default() {
        let vars = extract_variables_for_display("<name=default>");
        assert_eq!(vars.len(), 1);
        assert!(vars[0].contains("name"));
        assert!(vars[0].contains("default"));
    }

    #[test]
    fn test_expand_command_simple() {
        let result = expand_command("<name>", &[("name".to_string(), "value".to_string())]);
        assert_eq!(result, "value");
    }

    #[test]
    fn test_expand_command_with_default() {
        let result = expand_command("<name>", &[]);
        assert_eq!(result, "name");
    }

    #[test]
    fn test_expand_command_multiple() {
        let result = expand_command(
            "<a> and <b>",
            &[
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
            ],
        );
        assert_eq!(result, "1 and 2");
    }

    #[test]
    fn test_expand_command_no_vars() {
        let result = expand_command("echo hello", &[]);
        assert_eq!(result, "echo hello");
    }

    #[test]
    fn test_expand_command_escaped_angle_brackets() {
        let result = expand_command(r"ping \<website\>", &[]);
        assert_eq!(result, "ping <website>");
    }

    #[test]
    fn test_expand_command_escaped_backslash() {
        let result = expand_command(r"echo \\", &[]);
        assert_eq!(result, r"echo \");
    }

    #[test]
    fn test_expand_command_mixed_escapes() {
        let result = expand_command(r"echo \<foo\>", &[]);
        assert_eq!(result, "echo <foo>");
    }

    #[test]
    fn test_expand_command_escape_before_variable() {
        let result = expand_command(
            r"ping \<website\> and <host>",
            &[("host".to_string(), "example.com".to_string())],
        );
        assert_eq!(result, "ping <website> and example.com");
    }

    #[test]
    fn test_expand_command_trailing_backslash() {
        let result = expand_command(r"echo hello\", &[]);
        assert_eq!(result, r"echo hello\");
    }

    #[test]
    fn test_expand_command_escaped_backslash_before_bracket() {
        let result = expand_command(r"echo \\<foo>", &[("foo".to_string(), "bar".to_string())]);
        assert_eq!(result, r"echo \bar");
    }

    // UTILS-1: strip_escape_sequences consistency tests

    #[test]
    fn test_strip_escape_angled_brackets() {
        assert_eq!(strip_escape_sequences(r"\<"), "<");
        assert_eq!(strip_escape_sequences(r"\>"), ">");
    }

    #[test]
    fn test_strip_escape_double_backslash() {
        assert_eq!(strip_escape_sequences(r"\\"), "\\");
    }

    #[test]
    fn test_strip_escape_trailing_backslash() {
        assert_eq!(strip_escape_sequences(r"hello\"), r"hello\");
    }

    #[test]
    fn test_strip_escape_unknown_escape_preserved() {
        assert_eq!(strip_escape_sequences(r"\n"), r"\n");
    }

    #[test]
    fn test_strip_escape_consistent_with_expand_no_vars() {
        let cmd = r"echo \<hello\>";
        let stripped = strip_escape_sequences(cmd);
        let expanded = expand_command(cmd, &[]);
        assert_eq!(stripped, expanded);
        assert_eq!(stripped, "echo <hello>");
    }

    // UTILS-2: double-backslash edge case tests

    #[test]
    fn test_double_backslash_before_opening_bracket() {
        let result = expand_command(r"\\<foo>", &[("foo".to_string(), "bar".to_string())]);
        assert_eq!(result, r"\bar");
    }

    #[test]
    fn test_parse_variables_double_backslash_before_bracket() {
        let vars = parse_variables(r"\\<foo>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "foo");
    }

    #[test]
    fn test_strip_double_backslash_before_bracket() {
        assert_eq!(strip_escape_sequences(r"\\<foo>"), r"\<foo>");
    }

    // UTILS-4: chained escape sequence tests

    #[test]
    fn test_triple_backslash_before_opening_bracket() {
        let result = expand_command(r"\\\<foo>", &[("foo".to_string(), "bar".to_string())]);
        assert_eq!(result, r"\<foo>");
    }

    #[test]
    fn test_parse_triple_backslash_before_bracket() {
        let vars = parse_variables(r"\\\<foo>");
        assert_eq!(vars.len(), 0);
    }

    #[test]
    fn test_strip_triple_backslash_before_bracket() {
        assert_eq!(strip_escape_sequences(r"\\\<foo>"), r"\<foo>");
    }

    #[test]
    fn test_quad_backslash_before_bracket() {
        let result = expand_command(r"\\\\<foo>", &[("foo".to_string(), "bar".to_string())]);
        assert_eq!(result, r"\\bar");
    }

    // UTILS-5: nested angle bracket tests

    #[test]
    fn test_nested_angle_brackets() {
        let vars = parse_variables("<outer<inner>>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "outer<inner>");
    }

    #[test]
    fn test_expand_nested_angle_brackets() {
        let result = expand_command(
            "<outer<inner>>",
            &[("outer<inner>".to_string(), "val".to_string())],
        );
        assert_eq!(result, "val");
    }

    #[test]
    fn test_deeply_nested_angle_brackets() {
        let vars = parse_variables("a<b<c>>d");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "b<c>");
    }

    #[test]
    fn test_empty_nested_brackets() {
        let vars = parse_variables("<<>>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "<>");
    }

    #[test]
    fn test_unmatched_nested_opening() {
        assert!(has_unmatched_angle_bracket("<a<b>"));
    }

    #[test]
    fn test_matched_nested_brackets() {
        assert!(!has_unmatched_angle_bracket("<a<b>>"));
    }

    // UTILS-6: backslash at end of variable content tests

    #[test]
    fn test_escaped_closing_bracket_inside_variable() {
        let vars = parse_variables(r"<var\>>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "var>");
    }

    #[test]
    fn test_expand_escaped_closing_bracket() {
        let result = expand_command(r"<var\>>", &[("var>".to_string(), "val".to_string())]);
        assert_eq!(result, "val");
    }

    #[test]
    fn test_unmatched_backslash_closing_bracket() {
        let vars = parse_variables(r"<var\>");
        assert_eq!(vars.len(), 0);
    }

    #[test]
    fn test_escaped_opening_inside_variable() {
        let vars = parse_variables(r"<var\<inner>>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "var<inner");
    }

    #[test]
    fn test_double_backslash_inside_variable() {
        let vars = parse_variables(r"<var\\end>>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, r"var\end");
    }

    #[test]
    fn test_has_unmatched_escaped_closing_bracket() {
        assert!(!has_unmatched_angle_bracket(r"<var\>>"));
    }

    #[test]
    fn test_has_unmatched_truly_unmatched() {
        assert!(has_unmatched_angle_bracket(r"<var\>"));
    }

    #[test]
    fn test_bare_unmatched_angle_bracket() {
        let vars = parse_variables("echo <unclosed");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_expand_bare_unmatched_angle_bracket() {
        let result = expand_command("echo <unclosed", &[]);
        assert_eq!(result, "echo <unclosed");
    }

    #[test]
    fn test_expand_command_with_empty_value_produces_empty() {
        let result = expand_command("<host>", &[("host".to_string(), "".to_string())]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_expand_escaped_closing_bracket_inside_var_silently_drops_backslash() {
        // Quirk: `<x\>foo` — the `\>` is treated as an escape sequence inside
        // the variable body, so the `>` ends up terminating the variable and
        // the backslash is silently dropped. The result is `<x>foo`. The
        // `foo` becomes a separate literal (no closing `>`), so it stays as
        // plain text. Documenting the current behavior here.
        let result = expand_command(r"<x\>foo", &[]);
        assert_eq!(result, "<x>foo");
    }

    // ========================================================================
    // Pet-compatibility regression tests — behavioral contract for core APIs
    // These must not change in future releases.
    // ========================================================================

    #[test]
    fn test_extract_basic_variable_no_default() {
        let vars = parse_variables("<name>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "name");
        assert_eq!(vars[0].default, None);
    }

    #[test]
    fn test_extract_variable_with_default() {
        let vars = parse_variables("<host=localhost>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "host");
        assert_eq!(vars[0].default, Some("localhost".to_string()));
    }

    #[test]
    fn test_extract_variable_with_default_containing_slash() {
        let vars = parse_variables("<path=/usr/local/bin>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "path");
        assert_eq!(vars[0].default, Some("/usr/local/bin".to_string()));
    }

    #[test]
    fn test_extract_variable_with_default_containing_spaces() {
        let vars = parse_variables("<greeting=hello world>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "greeting");
        assert_eq!(vars[0].default, Some("hello world".to_string()));
    }

    #[test]
    fn test_extract_multiple_variables() {
        let vars = parse_variables("ssh <user>@<host> -p <port=22>");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].name, "user");
        assert_eq!(vars[0].default, None);
        assert_eq!(vars[1].name, "host");
        assert_eq!(vars[1].default, None);
        assert_eq!(vars[2].name, "port");
        assert_eq!(vars[2].default, Some("22".to_string()));
    }

    #[test]
    fn test_extract_no_variables() {
        let vars = parse_variables("echo hello world");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_extract_empty_string() {
        let vars = parse_variables("");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_expand_single_variable() {
        let result = expand_command("<name>", &[("name".to_string(), "david".to_string())]);
        assert_eq!(result, "david");
    }

    #[test]
    fn test_expand_variable_with_default_not_provided() {
        // expand_command does NOT use the default value from the syntax;
        // without a matching entry in values, the literal token is preserved.
        let result = expand_command("<host=localhost>", &[]);
        assert_eq!(result, "<host=localhost>");
    }

    #[test]
    fn test_expand_variable_with_default_provided() {
        // expand_command does NOT expand <var=default> syntax at all;
        // the entire token is preserved as a literal.
        let result = expand_command(
            "<host=localhost>",
            &[("host".to_string(), "server1".to_string())],
        );
        assert_eq!(result, "<host=localhost>");
    }

    #[test]
    fn test_expand_multiple_variables() {
        // expand_command does NOT expand <var=default> syntax;
        // only plain <var> tokens are expanded. The <port=22> literal is preserved.
        let result = expand_command(
            "ssh <user>@<host> -p <port=22>",
            &[
                ("user".to_string(), "root".to_string()),
                ("host".to_string(), "10.0.0.1".to_string()),
            ],
        );
        assert_eq!(result, "ssh root@10.0.0.1 -p <port=22>");
    }

    #[test]
    fn test_expand_preserves_literal_text() {
        let result = expand_command(
            "echo <var> done",
            &[("var".to_string(), "hello".to_string())],
        );
        assert_eq!(result, "echo hello done");
    }

    #[test]
    fn test_expand_with_empty_value() {
        let result = expand_command("<name>", &[("name".to_string(), "".to_string())]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_strip_escapes_literal_angle_brackets() {
        let result = strip_escape_sequences(r"\<hello\>");
        assert_eq!(result, "<hello>");
    }

    #[test]
    fn test_strip_escapes_backslash() {
        let result = strip_escape_sequences(r"\\path");
        assert_eq!(result, r"\path");
    }

    #[test]
    fn test_strip_escapes_no_escapes() {
        let result = strip_escape_sequences("hello world");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_has_unmatched_angle_bracket_simple() {
        assert!(has_unmatched_angle_bracket("<name"));
    }

    #[test]
    fn test_has_unmatched_angle_bracket_matched() {
        assert!(!has_unmatched_angle_bracket("<name>"));
    }

    #[test]
    fn test_has_unmatched_angle_bracket_escaped() {
        assert!(!has_unmatched_angle_bracket(r"\<literal\>"));
    }

    #[test]
    fn test_parse_variables_returns_unique_names() {
        let vars = parse_variables("<a> <a> <b>");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].name, "a");
        assert_eq!(vars[1].name, "a");
        assert_eq!(vars[2].name, "b");
    }

    // ========================================================================
    // Pet multiple-choice variable tests (Release 3A)
    // ========================================================================

    #[test]
    fn test_parse_choice_variable_basic() {
        let vars = parse_variables("<color=|_red_||_green_||_blue_||>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "color");
        assert_eq!(
            vars[0].kind,
            VariableKind::Choices {
                values: vec!["red".to_string(), "green".to_string(), "blue".to_string()],
                default_index: Some(0),
            }
        );
        assert_eq!(vars[0].default, Some("red".to_string()));
    }

    #[test]
    fn test_parse_choice_variable_two_choices() {
        let vars = parse_variables("<yes_no=|_yes_||_no_||>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "yes_no");
        match &vars[0].kind {
            VariableKind::Choices {
                values,
                default_index,
            } => {
                assert_eq!(values, &vec!["yes".to_string(), "no".to_string()]);
                assert_eq!(*default_index, Some(0));
            }
            _ => panic!("Expected Choices kind"),
        }
        assert_eq!(vars[0].default, Some("yes".to_string()));
    }

    #[test]
    fn test_parse_choice_variable_single_choice() {
        let vars = parse_variables("<only=|_one_||>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "only");
        match &vars[0].kind {
            VariableKind::Choices {
                values,
                default_index,
            } => {
                assert_eq!(values, &vec!["one".to_string()]);
                assert_eq!(*default_index, Some(0));
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_parse_choice_variable_mixed_with_others() {
        let vars = parse_variables("ssh <user>@<host> -p <port=|_22_||_80_||_443_||>");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].kind, VariableKind::Required);
        assert_eq!(vars[1].kind, VariableKind::Required);
        assert!(matches!(vars[2].kind, VariableKind::Choices { .. }));
        assert_eq!(vars[2].default, Some("22".to_string()));
    }

    #[test]
    fn test_parse_choice_variable_with_spaces() {
        let vars = parse_variables("<greeting=|_hello_||_hello world_||_hi_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(
                    values,
                    &vec![
                        "hello".to_string(),
                        "hello world".to_string(),
                        "hi".to_string()
                    ]
                );
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_expand_choice_variable_like_required() {
        // Choice variables expand just like required variables when a value is provided.
        let result = expand_command("<color>", &[("color".to_string(), "green".to_string())]);
        assert_eq!(result, "green");
    }

    #[test]
    fn test_expand_choice_variable_no_value() {
        // Without a value, the raw token is preserved.
        let result = expand_command("<color>", &[]);
        assert_eq!(result, "color");
    }

    #[test]
    fn test_expand_choice_in_full_command() {
        let result = expand_command(
            "echo <color> and <size>",
            &[
                ("color".to_string(), "red".to_string()),
                ("size".to_string(), "large".to_string()),
            ],
        );
        assert_eq!(result, "echo red and large");
    }

    #[test]
    fn test_extract_variables_for_display_choice() {
        let vars = extract_variables_for_display("<color=|_red_||_green_||_blue_||>");
        assert_eq!(vars.len(), 1);
        assert!(vars[0].contains("color"));
        assert!(vars[0].contains("red"));
        assert!(vars[0].contains("green"));
        assert!(vars[0].contains("blue"));
    }

    #[test]
    fn test_extract_variables_for_display_mixed() {
        let vars = extract_variables_for_display("<name> <host=localhost> <port=|_22_||_80_||>");
        assert_eq!(vars.len(), 3);
        assert!(vars[0].contains("prompt"));
        assert!(vars[1].contains("localhost"));
        assert!(vars[2].contains("22"));
        assert!(vars[2].contains("80"));
    }

    #[test]
    fn test_choice_variable_backward_compat_default() {
        let vars = parse_variables("<color=|_red_||_green_||>");
        assert_eq!(vars[0].default, Some("red".to_string()));
    }

    #[test]
    fn test_choice_variable_default_index_always_zero() {
        let vars = parse_variables("<x=|_a_||_b_||_c_||>");
        match &vars[0].kind {
            VariableKind::Choices { default_index, .. } => {
                assert_eq!(*default_index, Some(0));
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_regular_default_not_confused_with_choices() {
        // A value containing `|_` that doesn't match the full pattern is a regular default.
        let vars = parse_variables("<path=/data|_backup>");
        assert_eq!(vars.len(), 1);
        assert_eq!(
            vars[0].kind,
            VariableKind::DefaultValue("/data|_backup".to_string())
        );
        assert_eq!(vars[0].default, Some("/data|_backup".to_string()));
    }

    #[test]
    fn test_empty_equals_sign_not_choices() {
        // `<name=>` has empty content after `=`, which is a Required variable.
        let vars = parse_variables("<name=>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].kind, VariableKind::Required);
        assert_eq!(vars[0].default, None);
    }

    // ========================================================================
    // Workstream G: Comprehensive choice variable unit tests
    // ========================================================================

    #[test]
    fn test_choice_two_choices() {
        let vars = parse_variables("<x=|_a_||_b_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(values, &vec!["a".to_string(), "b".to_string()]);
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_many_choices() {
        let vars = parse_variables("<x=|_a_||_b_||_c_||_d_||_e_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(values.len(), 5);
                assert_eq!(values[0], "a");
                assert_eq!(values[4], "e");
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_single_choice() {
        let vars = parse_variables("<only=|_one_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices {
                values,
                default_index,
            } => {
                assert_eq!(values, &vec!["one".to_string()]);
                assert_eq!(*default_index, Some(0));
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_default_is_first() {
        let vars = parse_variables("<x=|_first_||_second_||_third_||>");
        assert_eq!(vars[0].default, Some("first".to_string()));
        match &vars[0].kind {
            VariableKind::Choices { default_index, .. } => {
                assert_eq!(*default_index, Some(0));
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_unicode_values() {
        let vars = parse_variables("<lang=|_日本語_||_中文_||_한국어_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(
                    values,
                    &vec![
                        "日本語".to_string(),
                        "中文".to_string(),
                        "한국어".to_string()
                    ]
                );
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_spaces_in_values() {
        let vars = parse_variables("<greeting=|_hello world_||_goodbye world_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(
                    values,
                    &vec!["hello world".to_string(), "goodbye world".to_string()]
                );
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_punctuation_in_values() {
        let vars = parse_variables("<cmd=|_echo \"hi\"_||_echo 'hi'_||>");
        assert_eq!(vars.len(), 1);
        match &vars[0].kind {
            VariableKind::Choices { values, .. } => {
                assert_eq!(
                    values,
                    &vec!["echo \"hi\"".to_string(), "echo 'hi'".to_string()]
                );
            }
            _ => panic!("Expected Choices kind"),
        }
    }

    #[test]
    fn test_choice_empty_after_eq_not_choices() {
        // `<name=|_>` — starts with `|_` but has no `_|` closure
        let vars = parse_variables("<name=|_>");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].kind, VariableKind::DefaultValue("|_".to_string()));
    }

    #[test]
    fn test_choice_malformed_missing_closer() {
        // `<name=|_opt1_||_opt2_` — missing trailing `|`
        let vars = parse_variables("<name=|_opt1_||_opt2_>");
        assert_eq!(vars.len(), 1);
        // Falls back to default value since extract_choices fails
        assert_eq!(
            vars[0].kind,
            VariableKind::DefaultValue("|_opt1_||_opt2_".to_string())
        );
    }

    #[test]
    fn test_choice_malformed_missing_opener() {
        // `<name=opt1_||>` — no `|_` opener
        let vars = parse_variables("<name=opt1_||>");
        assert_eq!(vars.len(), 1);
        assert_eq!(
            vars[0].kind,
            VariableKind::DefaultValue("opt1_||".to_string())
        );
    }

    #[test]
    fn test_choice_mixed_with_regular_variables() {
        let vars = parse_variables("<name> <color=|_red_||_blue_||> <host=localhost>");
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0].kind, VariableKind::Required);
        assert!(matches!(vars[1].kind, VariableKind::Choices { .. }));
        assert_eq!(
            vars[2].kind,
            VariableKind::DefaultValue("localhost".to_string())
        );
    }

    #[test]
    fn test_choice_mixed_with_escaped_brackets() {
        let vars = parse_variables(r"echo \<literal\> <color=|_red_||_blue_||>");
        assert_eq!(vars.len(), 1);
        assert!(matches!(vars[0].kind, VariableKind::Choices { .. }));
        assert_eq!(vars[0].name, "color");
    }

    #[test]
    fn test_choice_repeated_variables() {
        let vars = parse_variables("<x=|_a_||_b_||> and <x=|_c_||_d_||>");
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "x");
        assert_eq!(vars[1].name, "x");
    }

    #[test]
    fn test_expand_choice_variable_provided() {
        // expand_command does NOT expand <var=|_choices_||> syntax;
        // the entire token is preserved as a literal (same as <var=default>).
        let result = expand_command(
            "<color=|_red_||_green_||_blue_||>",
            &[("color".to_string(), "green".to_string())],
        );
        assert_eq!(result, "<color=|_red_||_green_||_blue_||>");
    }

    #[test]
    fn test_expand_choice_variable_not_provided() {
        let result = expand_command("<color=|_red_||_green_||_blue_||>", &[]);
        assert_eq!(result, "<color=|_red_||_green_||_blue_||>");
    }

    #[test]
    fn test_expand_choice_mixed_positional() {
        // Only plain <var> tokens are expanded; <var=|_choices_||> is preserved.
        let result = expand_command(
            "ssh <user>@<host> -p <port=|_22_||_8022_||>",
            &[
                ("user".to_string(), "root".to_string()),
                ("host".to_string(), "10.0.0.1".to_string()),
            ],
        );
        assert_eq!(result, "ssh root@10.0.0.1 -p <port=|_22_||_8022_||>");
    }

    #[test]
    fn test_has_unmatched_choice_syntax() {
        // Choice syntax is properly closed, so no unmatched bracket
        assert!(!has_unmatched_angle_bracket("<color=|_red_||_green_||>"));
    }

    #[test]
    fn test_has_unmatched_choice_syntax_single() {
        assert!(!has_unmatched_angle_bracket("<x=|_a_||>"));
    }

    #[test]
    fn test_extract_display_choice() {
        let display = extract_variables_for_display("<color=|_red_||_green_||_blue_||>");
        assert_eq!(display.len(), 1);
        assert!(display[0].contains("color"));
        assert!(display[0].contains("["));
        assert!(display[0].contains("red"));
        assert!(display[0].contains("green"));
        assert!(display[0].contains("blue"));
    }

    // ========================================================================
    // Workstream F: Parser diagnostics tests
    // ========================================================================

    #[test]
    fn test_diagnostics_clean_input() {
        let (vars, diags) = parse_variables_diagnostics("<color=|_red_||_green_||_blue_||>");
        assert_eq!(vars.len(), 1);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_diagnostics_unclosed_choice() {
        let (_vars, diags) = parse_variables_diagnostics("<name=|_unclosed>");
        // Starts with `|_` but doesn't end with `||` and doesn't close with `_|`
        assert!(!diags.is_empty());
        assert_eq!(diags[0].severity, DiagnosticSeverity::Warning);
        assert!(diags[0].message.contains("Unclosed"));
    }

    #[test]
    fn test_diagnostics_duplicate_names() {
        let (_vars, diags) = parse_variables_diagnostics("<x> <y> <x>");
        let dup_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("Duplicate"))
            .collect();
        assert_eq!(dup_diags.len(), 1);
        assert!(dup_diags[0].message.contains("x"));
    }

    #[test]
    fn test_diagnostics_no_duplicates_unique() {
        let (_vars, diags) = parse_variables_diagnostics("<a> <b> <c>");
        let dup_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("Duplicate"))
            .collect();
        assert!(dup_diags.is_empty());
    }

    #[test]
    fn test_diagnostics_clean_required_var() {
        let (vars, diags) = parse_variables_diagnostics("<name>");
        assert_eq!(vars.len(), 1);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_diagnostics_clean_default_var() {
        let (vars, diags) = parse_variables_diagnostics("<host=localhost>");
        assert_eq!(vars.len(), 1);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_diagnostics_regular_default_not_flagged() {
        let (vars, diags) = parse_variables_diagnostics("<path=/data|_backup>");
        assert_eq!(vars.len(), 1);
        assert!(diags.is_empty());
        assert_eq!(
            vars[0].kind,
            VariableKind::DefaultValue("/data|_backup".to_string())
        );
    }
}

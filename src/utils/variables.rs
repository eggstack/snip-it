//! Variable expansion for dynamic snippets.
//!
//! Handles parsing and expansion of `<variable>` and `<variable=default>` syntax
//! in snippet commands. Supports escaped angle brackets (`\<` and `\>`).

/// A parsed variable from a snippet command.
#[derive(Clone)]
pub struct Variable {
    pub name: String,
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

fn extract_variable_tokens(command: &str) -> Vec<(String, Option<String>)> {
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
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
                } else if next == '>' {
                    depth -= 1;
                    if depth == 0 {
                        chars.next();
                        break;
                    }
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
                } else {
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
                }
            }

            if !var_content.is_empty() && depth == 0 {
                let (name, default) = if let Some(eq_pos) = var_content.find('=') {
                    let name = var_content[..eq_pos].trim().to_string();
                    let default_val = var_content[eq_pos + 1..].trim().to_string();
                    let default = if default_val.is_empty() {
                        None
                    } else {
                        Some(default_val)
                    };
                    (name, default)
                } else {
                    (var_content.trim().to_string(), None)
                };
                tokens.push((name, default));
            }
        }
    }
    tokens
}

pub fn parse_variables(command: &str) -> Vec<Variable> {
    extract_variable_tokens(command)
        .into_iter()
        .map(|(name, default)| Variable { name, default })
        .collect()
}

pub fn extract_variables_for_display(command: &str) -> Vec<String> {
    extract_variable_tokens(command)
        .into_iter()
        .map(|(name, default)| {
            if let Some(default_val) = default {
                format!("{} = {}", name, default_val)
            } else {
                format!("{} (prompt)", name)
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
        .map(|(name, _)| name)
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
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
                } else if next == '>' {
                    depth -= 1;
                    if depth == 0 {
                        chars.next();
                        break;
                    }
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
                } else {
                    // peek() confirmed Some above
                    var_content.push(chars.next().unwrap());
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
}

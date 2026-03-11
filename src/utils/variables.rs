use crate::ui::Variable;

pub fn parse_variables(command: &str) -> Vec<Variable> {
    let mut vars = Vec::new();
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut var_content = String::new();
            while let Some(&next) = chars.peek() {
                if next == '>' {
                    chars.next();
                    break;
                }
                var_content.push(chars.next().unwrap());
            }

            if !var_content.is_empty() {
                if let Some(eq_pos) = var_content.find('=') {
                    let name = var_content[..eq_pos].trim().to_string();
                    let default = var_content[eq_pos + 1..].trim().to_string();
                    if default.is_empty() {
                        vars.push(Variable {
                            name,
                            default: None,
                        });
                    } else {
                        vars.push(Variable {
                            name,
                            default: Some(default),
                        });
                    }
                } else {
                    vars.push(Variable {
                        name: var_content,
                        default: None,
                    });
                }
            }
        }
    }
    vars
}

pub fn extract_variables_for_display(command: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut chars = command.chars().peekable();
    let mut prev_was_backslash = false;

    while let Some(c) = chars.next() {
        if prev_was_backslash {
            prev_was_backslash = false;
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
                var_content.push(chars.next().unwrap());
            }

            if !var_content.is_empty() {
                if let Some(eq_pos) = var_content.find('=') {
                    let name = var_content[..eq_pos].trim().to_string();
                    let default = var_content[eq_pos + 1..].trim().to_string();
                    if default.is_empty() {
                        vars.push(format!("{} (prompt)", name));
                    } else {
                        vars.push(format!("{} = {}", name, default));
                    }
                } else {
                    vars.push(format!("{} (prompt)", var_content.trim()));
                }
            }
        }
    }
    vars
}

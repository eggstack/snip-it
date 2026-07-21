//! **Layer: Application**
//!
//! Deterministic non-TUI snippet retrieval.
//!
//! `snp get` retrieves a snippet by ID, exact description, exact command,
//! or fuzzy query. It never executes, opens a TUI, or accesses clipboard.

use crate::error::{SnipError, SnipResult};
use crate::outcome::CliOutcome;
use crate::selector::{
    LibraryScope, ResolutionPolicy, SelectionResult, SnippetSelector, resolve_selector,
};
use crate::utils::variables::{VariableAssignments, parse_variables, strip_escape_sequences};
use serde::Serialize;

/// Output field selector for `snp get`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum GetField {
    /// Output the full command text.
    Command,
    /// Output the description.
    Description,
    /// Output the snippet ID.
    Id,
    /// Output the tags as a comma-separated list.
    Tags,
}

/// JSON output for `snp get`.
#[derive(Debug, Serialize)]
pub struct GetJsonOutput {
    pub schema: u32,
    pub id: String,
    pub description: String,
    pub command: String,
    pub expanded: Option<String>,
    pub tags: Vec<String>,
    pub library: String,
    pub library_id: String,
}

/// Run the `snp get` command.
///
/// # Arguments
/// * `id` - Match by exact snippet UUID
/// * `description_exact` - Match by exact description (case-insensitive)
/// * `command_exact` - Match by exact command text (case-insensitive)
/// * `query` - Fuzzy query match
/// * `library` - Library scope (name or "all")
/// * `field` - Output only a specific field
/// * `raw` - Output raw stored bytes (no variable expansion, no trailing newline)
/// * `expanded` - Output with variables expanded
/// * `json` - Output as JSON
/// * `resolution` - Resolution policy (unique, first, all)
#[allow(clippy::too_many_arguments)]
pub fn run(
    id: Option<String>,
    description_exact: Option<String>,
    command_exact: Option<String>,
    query: Option<String>,
    library: Option<String>,
    field: Option<GetField>,
    raw: bool,
    expanded: bool,
    json: bool,
    resolution: ResolutionPolicy,
    vars: Option<Vec<String>>,
) -> SnipResult<CliOutcome> {
    let assignments = if let Some(raw_vars) = vars {
        let pairs = raw_vars
            .iter()
            .map(|s| VariableAssignments::parse_arg(s))
            .collect::<SnipResult<Vec<_>>>()?;
        Some(VariableAssignments::from_pairs(pairs.into_iter())?)
    } else {
        None
    };
    // Validate that at least one targeting selector is provided
    if id.is_none() && description_exact.is_none() && command_exact.is_none() && query.is_none() {
        return Err(SnipError::runtime_error(
            "No selector specified",
            Some("Provide --id, --description-exact, --command-exact, or --query"),
        ));
    }

    // Validate conflicting output modes
    if raw && expanded {
        return Err(SnipError::runtime_error(
            "Conflicting output modes",
            Some("--raw and --expanded cannot be used together"),
        ));
    }
    if json && (raw || expanded) {
        return Err(SnipError::runtime_error(
            "Conflicting output modes",
            Some("--json cannot be used with --raw or --expanded"),
        ));
    }
    if field.is_some() && (json || raw || expanded) {
        return Err(SnipError::runtime_error(
            "Conflicting output modes",
            Some("--field cannot be used with --json, --raw, or --expanded"),
        ));
    }

    // Build selector
    let lib_scope = match library {
        Some(ref name) if name == "all" => LibraryScope::AllLibraries,
        Some(name) => LibraryScope::Named(name),
        None => LibraryScope::Primary,
    };

    let mut selector = SnippetSelector::new(resolution).with_library(lib_scope);

    if let Some(id) = id {
        selector = selector.with_id(id);
    }
    if let Some(desc) = description_exact {
        selector = selector.with_description_exact(desc);
    }
    if let Some(cmd) = command_exact {
        selector = selector.with_command_exact(cmd);
    }
    if let Some(q) = query {
        selector = selector.with_query(q);
    }

    // Resolve
    let result = resolve_selector(&selector)?;

    match result {
        SelectionResult::NotFound => Ok(CliOutcome::NotFound),
        SelectionResult::Ambiguous(identities) => {
            if !json {
                eprintln!("Ambiguous match. Multiple snippets match:");
                for identity in &identities {
                    eprintln!(
                        "  {} - {} ({})",
                        identity.id, identity.description, identity.library_name
                    );
                }
                eprintln!("Use --id for exact matching, or refine your query.");
            }
            Ok(CliOutcome::Ambiguous)
        }
        SelectionResult::One(m) => {
            output_match(&m, field, raw, expanded, json, &assignments)?;
            Ok(CliOutcome::Success)
        }
        SelectionResult::Many(matches) => {
            for m in &matches {
                output_match(m, field, raw, expanded, json, &assignments)?;
            }
            Ok(CliOutcome::Success)
        }
    }
}

fn output_match(
    m: &crate::selector::SnippetMatch,
    field: Option<GetField>,
    raw: bool,
    expanded: bool,
    json: bool,
    assignments: &Option<VariableAssignments>,
) -> SnipResult<()> {
    if json {
        let expanded_cmd = if has_variables(&m.snippet.command) {
            Some(expand_without_prompt(&m.snippet.command, assignments))
        } else {
            None
        };
        let output = GetJsonOutput {
            schema: 1,
            id: m.snippet.id.clone(),
            description: m.snippet.description.clone(),
            command: m.snippet.command.clone(),
            expanded: expanded_cmd,
            tags: m.snippet.tags.clone(),
            library: m.library_name.clone(),
            library_id: m.library_id.clone(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&output).map_err(|e| SnipError::runtime_error(
                "JSON serialization failed",
                Some(&e.to_string())
            ))?
        );
        return Ok(());
    }

    let text = if let Some(f) = field {
        match f {
            GetField::Command => {
                if raw {
                    m.snippet.command.clone()
                } else {
                    strip_escape_sequences(&m.snippet.command)
                }
            }
            GetField::Description => m.snippet.description.clone(),
            GetField::Id => m.snippet.id.clone(),
            GetField::Tags => m.snippet.tags.join(","),
        }
    } else if raw {
        m.snippet.command.clone()
    } else if expanded {
        expand_without_prompt(&m.snippet.command, assignments)
    } else {
        // Default: output command with escape sequences stripped
        strip_escape_sequences(&m.snippet.command)
    };

    // Exact-byte mode: no trailing newline for --raw and --field
    if raw || field.is_some() {
        // Write bytes directly to stdout without adding newline
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle
            .write_all(text.as_bytes())
            .map_err(|e| SnipError::io_error("write stdout", std::path::PathBuf::new(), e))?;
    } else {
        println!("{text}");
    }

    Ok(())
}

/// Check if a command string contains variable tokens.
fn has_variables(command: &str) -> bool {
    !parse_variables(command).is_empty()
}

/// Expand a command without prompting (noninteractive).
/// Uses explicit assignments first, then defaults for variables that have them,
/// leaves required vars as-is.
fn expand_without_prompt(command: &str, assignments: &Option<VariableAssignments>) -> String {
    let vars = parse_variables(command);
    if vars.is_empty() {
        return strip_escape_sequences(command);
    }

    let mut result = command.to_string();
    for var in &vars {
        match &var.kind {
            crate::utils::variables::VariableKind::Required => {
                let token = format!("<{}>", var.name);
                let replacement = assignments
                    .as_ref()
                    .and_then(|a| a.get(&var.name))
                    .unwrap_or(&var.name);
                result = result.replace(&token, replacement);
            }
            crate::utils::variables::VariableKind::DefaultValue(default) => {
                let token = format!("<{}={}>", var.name, default);
                let replacement = assignments
                    .as_ref()
                    .and_then(|a| a.get(&var.name))
                    .unwrap_or(default);
                result = result.replace(&token, replacement);
            }
            crate::utils::variables::VariableKind::Choices {
                values,
                default_index,
            } => {
                let fallback = default_index
                    .and_then(|i| values.get(i))
                    .or_else(|| values.first())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let replacement = assignments
                    .as_ref()
                    .and_then(|a| a.get(&var.name))
                    .unwrap_or(fallback);
                if let Some(start) = result.find(&format!("<{}=", var.name))
                    && let Some(end) = result[start..].find('>')
                {
                    let full_token = &result[start..=start + end];
                    result = result.replacen(full_token, replacement, 1);
                }
            }
        }
    }
    strip_escape_sequences(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_variables_true() {
        assert!(has_variables("echo <name>"));
        assert!(has_variables("ssh <user>@<host>"));
        assert!(has_variables("<cmd> --verbose"));
    }

    #[test]
    fn test_has_variables_false() {
        assert!(!has_variables("echo hello"));
        assert!(!has_variables("ls -la"));
        assert!(!has_variables(""));
    }

    #[test]
    fn test_expand_without_prompt_no_vars() {
        assert_eq!(expand_without_prompt("echo hello", &None), "echo hello");
    }

    #[test]
    fn test_expand_without_prompt_with_default() {
        assert_eq!(
            expand_without_prompt("ssh <host=localhost> 'uptime'", &None),
            "ssh localhost 'uptime'"
        );
    }

    #[test]
    fn test_expand_without_prompt_required_var_stays() {
        // Required vars without defaults stay as-is in the output
        assert_eq!(expand_without_prompt("echo <name>", &None), "echo name");
    }

    #[test]
    fn test_expand_without_prompt_escaped_brackets() {
        assert_eq!(
            expand_without_prompt(r"ping \<website\>", &None),
            "ping <website>"
        );
    }

    #[test]
    fn test_get_field_values() {
        assert_eq!(GetField::Command, GetField::Command);
        assert_eq!(GetField::Description, GetField::Description);
        assert_eq!(GetField::Id, GetField::Id);
        assert_eq!(GetField::Tags, GetField::Tags);
    }

    #[test]
    fn test_expand_without_prompt_with_assignment_required() {
        let assignments = Some(
            VariableAssignments::from_pairs(vec![("name".into(), "david".into())].into_iter())
                .unwrap(),
        );
        assert_eq!(
            expand_without_prompt("echo <name>", &assignments),
            "echo david"
        );
    }

    #[test]
    fn test_expand_without_prompt_with_assignment_default_override() {
        let assignments = Some(
            VariableAssignments::from_pairs(vec![("host".into(), "prod.com".into())].into_iter())
                .unwrap(),
        );
        assert_eq!(
            expand_without_prompt("ssh <host=localhost> 'uptime'", &assignments),
            "ssh prod.com 'uptime'"
        );
    }

    #[test]
    fn test_expand_without_prompt_with_assignment_choices() {
        let assignments = Some(
            VariableAssignments::from_pairs(vec![("color".into(), "green".into())].into_iter())
                .unwrap(),
        );
        assert_eq!(
            expand_without_prompt("echo <color=|_red_||_green_||_blue_||>", &assignments),
            "echo green"
        );
    }

    #[test]
    fn test_expand_without_prompt_assignment_overrides_default() {
        let assignments = Some(
            VariableAssignments::from_pairs(vec![("port".into(), "8080".into())].into_iter())
                .unwrap(),
        );
        assert_eq!(
            expand_without_prompt("ssh <host> -p <port=22>", &assignments),
            "ssh host -p 8080"
        );
    }
}

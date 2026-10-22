//! **Layer: Application**
//!
//! Deterministic non-TUI snippet retrieval.
//!
//! `snp get` retrieves a snippet by ID, exact description, exact command,
//! or fuzzy query. It never executes, opens a TUI, or accesses clipboard.

use crate::error::{SnipError, SnipResult};
use crate::outcome::CliOutcome;
use crate::selector::{ResolutionPolicy, SelectionResult, resolve_selector_readonly};
use crate::utils::variables::{VariableAssignments, parse_variables, strip_escape_sequences};
use serde::Serialize;

/// Canonical Clap arguments for `snp get`.
#[derive(Debug, Clone, clap::Args)]
pub struct GetArgs {
    /// Match by exact snippet ID
    #[arg(long, conflicts_with_all = ["description_exact", "command_exact", "query"])]
    pub id: Option<String>,
    /// Match by exact description (case-insensitive)
    #[arg(long = "description-exact", conflicts_with_all = ["id", "command_exact", "query"])]
    pub description_exact: Option<String>,
    /// Match by exact command text (case-insensitive)
    #[arg(long = "command-exact", conflicts_with_all = ["id", "description_exact", "query"])]
    pub command_exact: Option<String>,
    /// Fuzzy query match
    #[arg(short, long, conflicts_with_all = ["id", "description_exact", "command_exact"])]
    pub query: Option<String>,
    /// Library scope (name, or "all" for all libraries)
    #[arg(short, long)]
    pub library: Option<String>,
    /// Output only a specific field
    #[arg(long, value_enum)]
    pub field: Option<GetField>,
    /// Output raw stored bytes (no variable expansion, no trailing newline)
    #[arg(long, conflicts_with = "expanded")]
    pub raw: bool,
    /// Output with variables expanded using defaults
    #[arg(long, conflicts_with = "raw")]
    pub expanded: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Resolution policy for multiple matches
    #[arg(long, value_enum, default_value_t = crate::selector::ResolutionPolicy::Unique)]
    pub resolution: crate::selector::ResolutionPolicy,
    /// Explicit variable assignment (repeatable: --var host=example.com --var env=prod)
    #[arg(long = "var", value_name = "KEY=VALUE", action = clap::ArgAction::Append)]
    pub vars: Option<Vec<String>>,
}

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
/// Read-only: resolves via [`resolve_selector_readonly`] so a legacy
/// single-file checkout is read in place without migration or file creation.
///
/// # Arguments
/// * `id` - Match by exact snippet ID
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
        return Err(SnipError::usage_error(
            "No selector specified",
            Some("Provide --id, --description-exact, --command-exact, or --query"),
        ));
    }

    // Validate conflicting output modes
    if raw && expanded {
        return Err(SnipError::usage_error(
            "Conflicting output modes",
            Some("--raw and --expanded cannot be used together"),
        ));
    }
    if json && (raw || expanded) {
        return Err(SnipError::usage_error(
            "Conflicting output modes",
            Some("--json cannot be used with --raw or --expanded"),
        ));
    }
    if field.is_some() && (json || raw || expanded) {
        return Err(SnipError::usage_error(
            "Conflicting output modes",
            Some("--field cannot be used with --json, --raw, or --expanded"),
        ));
    }

    // Build selector via the canonical exact-target constructor so library
    // `"all"` handling, ID case-sensitivity, and description/command
    // case-insensitivity cannot drift from run/clip/edit/MCP.
    let mut selector = crate::selector::exact_selector(library, None, None, None);
    // `exact_selector` starts Unique; apply the requested resolution policy.
    selector.resolution = resolution;
    // Re-apply targeting fields (exact_selector took Nones above to keep the
    // library-scope mapping in one place).
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

    // Resolve (read-only: no legacy migration, no file creation)
    let result = resolve_selector_readonly(&selector)?;

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
            if json {
                // One JSON document: an array of the per-snippet envelopes.
                // Emitting the objects back-to-back produced a stream of
                // concatenated objects that no JSON consumer can parse.
                let outputs = matches
                    .iter()
                    .map(|m| match_json_output(m, &assignments))
                    .collect::<SnipResult<Vec<_>>>()?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&outputs).map_err(|e| {
                        SnipError::runtime_error("JSON serialization failed", Some(&e.to_string()))
                    })?
                );
            } else {
                for m in &matches {
                    output_match(m, field, raw, expanded, json, &assignments)?;
                }
            }
            Ok(CliOutcome::Success)
        }
    }
}

/// Builds the documented JSON envelope for one matched snippet.
fn match_json_output(
    m: &crate::selector::SnippetMatch,
    assignments: &Option<VariableAssignments>,
) -> SnipResult<GetJsonOutput> {
    let expanded_cmd = if has_variables(&m.snippet.command) {
        Some(expand_without_prompt(&m.snippet.command, assignments))
    } else {
        None
    };
    Ok(GetJsonOutput {
        schema: 1,
        id: m.snippet.id.clone(),
        description: m.snippet.description.clone(),
        command: m.snippet.command.clone(),
        expanded: expanded_cmd,
        tags: m.snippet.tags.clone(),
        library: m.library_name.clone(),
        library_id: m.library_id.clone(),
    })
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
        let output = match_json_output(m, assignments)?;
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

/// Return the byte offset of the `>` that closes the variable token starting
/// at `start`, or `None` when the token is unterminated.
///
/// The escape and nesting rules mirror the token extractor in
/// `crate::utils::variables`: an escaped character never changes the nesting
/// depth, and a `<` inside the body increases it.
fn find_variable_token_end(command: &str, start: usize) -> Option<usize> {
    let bytes = command.as_bytes();
    let mut index = start + 1;
    let mut depth = 1usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                // The escaped character belongs to the body and can never
                // close or open a token.
                index += 1;
                if index < bytes.len() {
                    let escaped = command[index..].chars().next().unwrap();
                    index += escaped.len_utf8();
                }
            }
            b'<' => {
                depth += 1;
                index += 1;
            }
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => {
                let c = command[index..].chars().next().unwrap();
                index += c.len_utf8();
            }
        }
    }
    None
}

/// Undo the escape sequences inside a variable token body.
///
/// Mirrors the token extractor: `\\`, `\<` and `\>` collapse to a single
/// character, any other escaped character keeps its backslash.
fn unescape_variable_body(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                Some('<') => {
                    out.push('<');
                    chars.next();
                }
                Some('>') => {
                    out.push('>');
                    chars.next();
                }
                _ => out.push('\\'),
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Resolve the replacement text for one variable token.
///
/// `vars` holds one entry per token *occurrence*, in command order. The
/// occurrence is matched positionally, with a resynchronization on the name so
/// that a single disagreement between the scanner and the parser stays local
/// instead of misaligning every later variable.
fn resolve_token_replacement(
    vars: &[crate::utils::variables::Variable],
    var_index: &mut usize,
    name: &str,
    assignments: &Option<VariableAssignments>,
) -> Option<String> {
    let matched = vars
        .get(*var_index)
        .filter(|var| var.name == name)
        .map(|_| *var_index)
        .or_else(|| {
            vars.iter()
                .skip(*var_index + 1)
                .position(|var| var.name == name)
                .map(|offset| *var_index + 1 + offset)
        })?;
    let var = &vars[matched];
    *var_index = matched + 1;

    match &var.kind {
        crate::utils::variables::VariableKind::Required => {
            // An unassigned required variable stays verbatim: the output must
            // never silently substitute a semantically broken bare name.
            assignments
                .as_ref()
                .and_then(|a| a.get(name))
                .map(|value| value.to_string())
        }
        crate::utils::variables::VariableKind::DefaultValue(default) => Some(
            assignments
                .as_ref()
                .and_then(|a| a.get(name))
                .unwrap_or(default)
                .to_string(),
        ),
        crate::utils::variables::VariableKind::Choices {
            values,
            default_index,
        } => {
            let fallback = default_index
                .and_then(|i| values.get(i))
                .or_else(|| values.first())
                .map(|s| s.as_str())
                .unwrap_or("");
            Some(
                assignments
                    .as_ref()
                    .and_then(|a| a.get(name))
                    .unwrap_or(fallback)
                    .to_string(),
            )
        }
    }
}

/// Expand a command without prompting (noninteractive).
/// Uses explicit assignments first, then defaults for variables that have them.
/// Required variables without an assignment are left verbatim so the output
/// never silently substitutes a semantically broken bare name.
///
/// The expansion is a single left-to-right pass over the *original* command.
/// Every token is resolved against the original text, so text produced by one
/// substitution is never re-scanned for another token: a value that happens to
/// look like `<other>` stays verbatim. This matches the interactive expansion
/// path in `crate::utils::variables::expand_command`.
fn expand_without_prompt(command: &str, assignments: &Option<VariableAssignments>) -> String {
    let vars = parse_variables(command);
    if vars.is_empty() {
        return strip_escape_sequences(command);
    }

    let mut result = String::with_capacity(command.len());
    let mut var_index = 0usize;
    let mut cursor = 0usize;

    while cursor < command.len() {
        let c = command[cursor..].chars().next().unwrap();
        if c == '\\' {
            // Copy the escape sequence verbatim: an escaped character can
            // never start a variable token.
            result.push('\\');
            cursor += 1;
            if cursor < command.len() {
                let escaped = command[cursor..].chars().next().unwrap();
                result.push(escaped);
                cursor += escaped.len_utf8();
            }
            continue;
        }
        if c != '<' {
            result.push(c);
            cursor += c.len_utf8();
            continue;
        }

        let Some(end) = find_variable_token_end(command, cursor) else {
            // An unterminated `<` is not a token.
            result.push('<');
            cursor += 1;
            continue;
        };

        let body = unescape_variable_body(&command[cursor + 1..end]);
        let name = body
            .split_once('=')
            .map(|(name, _)| name.trim().to_string())
            .unwrap_or_else(|| body.trim().to_string());

        match resolve_token_replacement(&vars, &mut var_index, &name, assignments) {
            Some(replacement) => result.push_str(&replacement),
            None => {
                // Not a known variable: keep the original token text.
                result.push_str(&command[cursor..=end]);
            }
        }
        cursor = end + 1;
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
        // Required vars without defaults or assignments stay as-is in the output
        assert_eq!(expand_without_prompt("echo <name>", &None), "echo <name>");
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
        // `<host>` is required and unassigned, so it stays verbatim.
        assert_eq!(
            expand_without_prompt("ssh <host> -p <port=22>", &assignments),
            "ssh <host> -p 8080"
        );
    }

    /// A substituted value must never be re-scanned for another token: the
    /// expansion is a single pass over the original command.
    #[test]
    fn test_expand_without_prompt_does_not_expand_substituted_text() {
        let assignments = Some(
            VariableAssignments::from_pairs(
                vec![("a".into(), "<b>".into()), ("b".into(), "X".into())].into_iter(),
            )
            .unwrap(),
        );
        assert_eq!(
            expand_without_prompt("echo <a> <b>", &assignments),
            "echo <b> X"
        );
    }

    /// The same rule applies to a default value that looks like a token.
    #[test]
    fn test_expand_without_prompt_does_not_expand_a_default_that_looks_like_a_token() {
        let assignments = Some(
            VariableAssignments::from_pairs(vec![("b".into(), "X".into())].into_iter()).unwrap(),
        );
        assert_eq!(
            expand_without_prompt("echo <a=<b>> <b>", &assignments),
            "echo <b> X"
        );
    }
}

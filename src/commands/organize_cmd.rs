//! **Layer: Application**
//!
//! Deterministic non-interactive folder and favorite updates.
//!
//! `snp organize` mutates a snippet's local-only `folders` and `favorite`
//! fields without a TUI. It resolves its target through the mutating selector
//! ([`crate::selector::resolve_selector`]) and applies the shared field-update
//! primitive ([`crate::commands::update_snippet_fields`]).

use crate::commands::{
    FieldUpdateSyncTrigger, SnippetFieldMutation, SnippetFieldUpdateOutcome, update_snippet_fields,
};
use crate::error::{SnipError, SnipResult};
use crate::outcome::CliOutcome;
use crate::selector::{ResolutionPolicy, SelectionResult, SnippetIdentity, SnippetMatch};
use serde::Serialize;
/// Canonical Clap arguments for `snp organize`.
#[derive(Debug, Clone, clap::Args)]
pub struct OrganizeArgs {
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
    /// Library scope (name); `all` is not supported
    #[arg(short, long)]
    pub library: Option<String>,
    /// Mark the snippet as a favorite
    #[arg(
        long,
        group = "organize_action",
        conflicts_with_all = ["unfavorite", "add_folder", "remove_folder"]
    )]
    pub favorite: bool,
    /// Clear the favorite flag
    #[arg(
        long,
        group = "organize_action",
        conflicts_with_all = ["favorite", "add_folder", "remove_folder"]
    )]
    pub unfavorite: bool,
    /// Add a folder membership
    #[arg(
        long = "add-folder",
        group = "organize_action",
        conflicts_with_all = ["favorite", "unfavorite", "remove_folder"]
    )]
    pub add_folder: Option<String>,
    /// Remove a folder membership
    #[arg(
        long = "remove-folder",
        group = "organize_action",
        conflicts_with_all = ["favorite", "unfavorite", "add_folder"]
    )]
    pub remove_folder: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// JSON output for `snp organize`.
#[derive(Debug, Serialize)]
pub struct OrganizeJsonOutput {
    pub schema: u32,
    pub status: String,
    pub id: String,
    pub description: String,
    pub library: String,
    pub favorite: bool,
    pub folders: Vec<String>,
}

/// The verdict an organize reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrganizeStatus {
    Updated,
    Unchanged,
}

impl OrganizeStatus {
    fn as_str(self) -> &'static str {
        match self {
            OrganizeStatus::Updated => "updated",
            OrganizeStatus::Unchanged => "unchanged",
        }
    }
}

/// Run the `snp organize` command.
pub fn run(args: OrganizeArgs) -> SnipResult<CliOutcome> {
    if args.id.is_none()
        && args.description_exact.is_none()
        && args.command_exact.is_none()
        && args.query.is_none()
    {
        return Err(SnipError::usage_error(
            "No selector specified",
            Some("Provide --id, --description-exact, --command-exact, or --query"),
        ));
    }

    let mutation = parse_mutation(&args)?;

    if args.library.as_deref() == Some("all") {
        return Err(SnipError::runtime_error(
            "Library 'all' is not supported by 'snp organize'",
            Some(
                "An organize mutation targets one snippet in one library. Use 'snp list --library all' to list every library, or 'snp get --library all' to fetch a snippet by id.",
            ),
        ));
    }

    let mut selector = crate::selector::exact_selector(args.library.clone(), None, None, None);
    selector.resolution = ResolutionPolicy::Unique;
    if let Some(id) = args.id.clone() {
        selector = selector.with_id(id);
    }
    if let Some(description) = args.description_exact.clone() {
        selector = selector.with_description_exact(description);
    }
    if let Some(command) = args.command_exact.clone() {
        selector = selector.with_command_exact(command);
    }
    if let Some(query) = args.query.clone() {
        selector = selector.with_query(query);
    }

    match crate::selector::resolve_selector(&selector)? {
        SelectionResult::One(m) => {
            let outcome = update_snippet_fields(
                &m.library_path,
                &m.snippet.id,
                mutation,
                FieldUpdateSyncTrigger::AutoNotify,
            )?;
            report_primitive_outcome(outcome, &m.library_name, args.json)
        }
        SelectionResult::Many(matches) => {
            report_ambiguous(&identities_of(&matches));
            Ok(CliOutcome::Ambiguous)
        }
        SelectionResult::Ambiguous(identities) => {
            report_ambiguous(&identities);
            Ok(CliOutcome::Ambiguous)
        }
        SelectionResult::NotFound => {
            eprintln!("Snippet not found: no live snippet matches the given selector.");
            Ok(CliOutcome::NotFound)
        }
    }
}

/// Resolve exactly one action flag into a typed mutation.
fn parse_mutation(args: &OrganizeArgs) -> SnipResult<SnippetFieldMutation> {
    let mut actions = 0u8;
    let mut mutation = None;

    if args.favorite {
        actions += 1;
        mutation = Some(SnippetFieldMutation::SetFavorite);
    }
    if args.unfavorite {
        actions += 1;
        mutation = Some(SnippetFieldMutation::ClearFavorite);
    }
    if let Some(name) = args.add_folder.as_deref() {
        actions += 1;
        mutation = Some(SnippetFieldMutation::AddFolder(validate_folder_name(name)?));
    }
    if let Some(name) = args.remove_folder.as_deref() {
        actions += 1;
        mutation = Some(SnippetFieldMutation::RemoveFolder(validate_folder_name(
            name,
        )?));
    }

    if actions == 0 {
        return Err(SnipError::usage_error(
            "No action specified",
            Some(
                "Provide exactly one of --favorite, --unfavorite, --add-folder, or --remove-folder",
            ),
        ));
    }
    if actions > 1 {
        return Err(SnipError::usage_error(
            "Multiple actions specified",
            Some(
                "Provide exactly one of --favorite, --unfavorite, --add-folder, or --remove-folder",
            ),
        ));
    }

    mutation.ok_or_else(|| {
        SnipError::usage_error(
            "No action specified",
            Some(
                "Provide exactly one of --favorite, --unfavorite, --add-folder, or --remove-folder",
            ),
        )
    })
}

/// Validate a folder name for the write path.
fn validate_folder_name(name: &str) -> SnipResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SnipError::usage_error(
            "Folder name must not be empty",
            None,
        ));
    }
    if trimmed.contains(';') {
        return Err(SnipError::usage_error(
            "Folder name must not contain ';'",
            Some("Semicolons are the CSV column separator for folder lists"),
        ));
    }
    Ok(trimmed.to_string())
}

fn report_primitive_outcome(
    outcome: SnippetFieldUpdateOutcome,
    library_name: &str,
    json: bool,
) -> SnipResult<CliOutcome> {
    match outcome {
        SnippetFieldUpdateOutcome::Updated(snippet) => {
            emit_result(OrganizeStatus::Updated, &snippet, library_name, json)?;
            Ok(CliOutcome::Success)
        }
        SnippetFieldUpdateOutcome::Unchanged(snippet) => {
            emit_result(OrganizeStatus::Unchanged, &snippet, library_name, json)?;
            Ok(CliOutcome::Success)
        }
        SnippetFieldUpdateOutcome::NotFound => {
            eprintln!("Snippet not found: the target was deleted before the mutation ran.");
            Ok(CliOutcome::NotFound)
        }
    }
}

fn emit_result(
    status: OrganizeStatus,
    snippet: &crate::library::Snippet,
    library_name: &str,
    json: bool,
) -> SnipResult<()> {
    if json {
        let payload = OrganizeJsonOutput {
            schema: 1,
            status: status.as_str().to_string(),
            id: snippet.id.clone(),
            description: snippet.description.clone(),
            library: library_name.to_string(),
            favorite: snippet.favorite,
            folders: snippet.folders.clone(),
        };
        let json_str = serde_json::to_string_pretty(&payload).map_err(|e| {
            SnipError::runtime_error("Failed to serialize organize result", Some(&e.to_string()))
        })?;
        println!("{json_str}");
    } else {
        match status {
            OrganizeStatus::Updated => {
                println!("Updated snippet '{}' ({})", snippet.description, snippet.id);
            }
            OrganizeStatus::Unchanged => {
                println!(
                    "Snippet '{}' ({}) already matches the requested state",
                    snippet.description, snippet.id
                );
            }
        }
    }
    Ok(())
}

fn report_ambiguous(identities: &[SnippetIdentity]) {
    eprintln!("Ambiguous match. Multiple snippets match:");
    for identity in identities {
        eprintln!(
            "  {} - {} ({})",
            identity.id, identity.description, identity.library_name
        );
    }
    eprintln!("Use --id for exact matching, or refine your query.");
}

fn identities_of(matches: &[SnippetMatch]) -> Vec<SnippetIdentity> {
    matches
        .iter()
        .map(|m| SnippetIdentity {
            id: m.snippet.id.clone(),
            description: m.snippet.description.clone(),
            command: m.snippet.command.clone(),
            library_name: m.library_name.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{LibraryManager, Snippet, Snippets};
    use std::path::PathBuf;
    use tempfile::TempDir;

    const FUTURE_UPDATED_AT: i64 = 4_000_000_000;

    struct Fixture {
        _tmp: TempDir,
        _config_dir: crate::utils::config::ScopedConfigDir,
        library_path: PathBuf,
    }

    impl Fixture {
        fn new(snippets: Vec<Snippet>) -> Self {
            let tmp = TempDir::new().expect("test helper: temp dir");
            let config_dir = tmp.path().join(".config").join("snp");
            let guard = crate::utils::config::ScopedConfigDir::new(config_dir);
            let mut manager = LibraryManager::new().expect("test helper: manager");
            let library_path = manager
                .create_library("mylib")
                .expect("test helper: create library");
            let collection = Snippets {
                snippets,
                folders: vec![],
            };
            std::fs::write(
                &library_path,
                toml::to_string_pretty(&collection).expect("test helper: serialize library"),
            )
            .expect("test helper: write library");
            Self {
                _tmp: tmp,
                _config_dir: guard,
                library_path,
            }
        }

        fn snippet(id: &str, description: &str, command: &str) -> Snippet {
            Snippet {
                id: id.to_string(),
                description: description.to_string(),
                command: command.to_string(),
                updated_at: FUTURE_UPDATED_AT,
                ..Default::default()
            }
        }

        fn library_bytes(&self) -> String {
            std::fs::read_to_string(&self.library_path).expect("test helper: read library")
        }
    }

    fn organize_by_id(id: &str, favorite: bool) -> OrganizeArgs {
        OrganizeArgs {
            id: Some(id.to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            favorite,
            unfavorite: false,
            add_folder: None,
            remove_folder: None,
            json: false,
        }
    }

    #[test]
    fn favorite_mutation_preserves_identifier_and_advances_timestamp() {
        let fixture = Fixture::new(vec![Fixture::snippet(
            "id-1",
            "git push",
            "git push origin main",
        )]);
        let original_id = "id-1".to_string();
        let live = crate::library::load_library(&fixture.library_path)
            .expect("load fixture")
            .snippets[0]
            .updated_at;

        let outcome = run(organize_by_id("id-1", true)).expect("organize succeeds");
        assert_eq!(outcome, CliOutcome::Success);

        let persisted = crate::library::load_library(&fixture.library_path).expect("reload");
        let target = &persisted.snippets[0];
        assert_eq!(target.id, original_id);
        assert!(target.favorite);
        assert_eq!(target.updated_at, live + 1);
    }

    #[test]
    fn noop_favorite_reports_unchanged_and_leaves_file_bytes_identical() {
        let mut snippet = Fixture::snippet("id-1", "git push", "git push");
        snippet.favorite = true;
        let fixture = Fixture::new(vec![snippet]);
        let before = fixture.library_bytes();

        let outcome = run(organize_by_id("id-1", true)).expect("organize reports unchanged");
        assert_eq!(outcome, CliOutcome::Success);
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn removing_last_folder_drops_toml_key_and_unfavorite_keeps_false_field() {
        let mut snippet = Fixture::snippet("id-1", "alpha", "echo");
        snippet.folders = vec!["work".to_string()];
        snippet.favorite = true;
        let fixture = Fixture::new(vec![snippet]);

        let remove = OrganizeArgs {
            id: Some("id-1".to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            favorite: false,
            unfavorite: false,
            add_folder: None,
            remove_folder: Some("work".to_string()),
            json: false,
        };
        run(remove).expect("remove folder succeeds");
        let toml_text = fixture.library_bytes();
        assert!(
            !toml_text.contains("folders"),
            "empty folders list must omit the key: {toml_text}"
        );

        run(OrganizeArgs {
            id: Some("id-1".to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            favorite: false,
            unfavorite: true,
            add_folder: None,
            remove_folder: None,
            json: false,
        })
        .expect("unfavorite succeeds");
        let toml_text = fixture.library_bytes();
        assert!(
            toml_text.contains("favorite = false"),
            "favorite=false must stay present: {toml_text}"
        );
    }

    #[test]
    fn invalid_folder_name_is_a_usage_error_and_writes_nothing() {
        let fixture = Fixture::new(vec![Fixture::snippet("id-1", "alpha", "echo")]);
        let before = fixture.library_bytes();
        let error = run(OrganizeArgs {
            id: Some("id-1".to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            favorite: false,
            unfavorite: false,
            add_folder: Some("bad;name".to_string()),
            remove_folder: None,
            json: false,
        })
        .expect_err("semicolon folder names are rejected");
        assert!(error.to_string().contains(';'));
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn ambiguous_selector_changes_nothing() {
        let fixture = Fixture::new(vec![
            Fixture::snippet("id-1", "duplicate", "echo one"),
            Fixture::snippet("id-2", "duplicate", "echo two"),
        ]);
        let before = fixture.library_bytes();
        let outcome = run(OrganizeArgs {
            id: None,
            description_exact: Some("duplicate".to_string()),
            command_exact: None,
            query: None,
            library: None,
            favorite: true,
            unfavorite: false,
            add_folder: None,
            remove_folder: None,
            json: false,
        })
        .expect("ambiguity is an outcome");
        assert_eq!(outcome, CliOutcome::Ambiguous);
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn golden_corpus_command_field_survives_favorite_mutation_round_trip() {
        let command = "echo\ttrailing \r\n";
        let fixture = Fixture::new(vec![Fixture::snippet("id-1", "corpus", command)]);
        run(organize_by_id("id-1", true)).expect("organize mutates golden snippet");
        let reloaded = crate::library::load_library(&fixture.library_path).expect("reload");
        assert_eq!(reloaded.snippets[0].command, command);
    }
}

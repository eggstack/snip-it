//! **Layer: Application**
//!
//! Deterministic non-interactive snippet delete.
//!
//! `snp delete` tombstones one snippet without a TUI, an execution, or a
//! confirmation prompt. It resolves its target through the mutating selector
//! ([`crate::selector::resolve_selector`]) and applies the shared delete
//! primitive ([`crate::commands::delete_snippet`]), so the persisted state is
//! identical to the one the interactive delete produces.

use crate::commands::{DeleteSyncTrigger, SnippetDeleteOutcome, delete_snippet};
use crate::error::{SnipError, SnipResult};
use crate::outcome::CliOutcome;
use crate::selector::{
    ResolutionPolicy, SelectionResult, SnippetIdentity, SnippetMatch, SnippetSelector,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Canonical Clap arguments for `snp delete`.
#[derive(Debug, Clone, clap::Args)]
pub struct DeleteArgs {
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
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// JSON output for `snp delete`.
///
/// `status` is `deleted` when the snippet was tombstoned by this run and
/// `already_deleted` when the target already carried a tombstone.
#[derive(Debug, Serialize)]
pub struct DeleteJsonOutput {
    pub schema: u32,
    pub status: String,
    pub id: String,
    pub description: String,
    pub library: String,
}

/// The verdict a delete reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteStatus {
    /// The snippet was tombstoned by this run.
    Deleted,
    /// The target already carried a tombstone; nothing was changed.
    AlreadyDeleted,
}

impl DeleteStatus {
    /// The stable machine-readable status token.
    fn as_str(self) -> &'static str {
        match self {
            DeleteStatus::Deleted => "deleted",
            DeleteStatus::AlreadyDeleted => "already_deleted",
        }
    }
}

/// Run the `snp delete` command.
///
/// Resolves one target through the mutating selector and tombstones it with
/// the shared delete primitive. There is no confirmation prompt: the
/// selector's ambiguity rules are what protect against deleting the wrong
/// target.
pub fn run(args: DeleteArgs) -> SnipResult<CliOutcome> {
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

    // A delete tombstones one snippet in one library file. The cross-library
    // scope keyword is refused explicitly instead of failing later with the
    // misleading "Library 'all' does not exist" — `all` is the scope keyword
    // owned by `LibraryScope::from_filter_arg`, and it does work for
    // `snp get`, `list`, and MCP.
    if args.library.as_deref() == Some("all") {
        return Err(SnipError::runtime_error(
            "Library 'all' is not supported by 'snp delete'",
            Some(
                "A delete targets one snippet in one library. Use 'snp list --library all' to list every library, or 'snp get --library all' to fetch a snippet by id.",
            ),
        ));
    }

    // Build the selector through the canonical constructor so the library
    // scope mapping, the ID case-sensitivity, and the description/command
    // case-insensitivity cannot drift from run/clip/edit/get/MCP.
    let mut selector = crate::selector::exact_selector(args.library.clone(), None, None, None);
    // A delete never fans out: one target, one tombstone.
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
            let outcome = delete_snippet(
                &m.library_path,
                &m.snippet.id,
                DeleteSyncTrigger::AutoNotify,
            )?;
            Ok(report_outcome(outcome, &m.library_name, args.json)?)
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
            // The selector excludes tombstoned snippets from every mode, so
            // "not found" can also mean "already deleted". Probe for the
            // tombstone so the two outcomes stay distinct.
            if let Some(path) = scope_library_path(args.library.as_deref())?
                && let Some(snippet) = probe_deleted_target(&selector, &path)?
            {
                emit_result(
                    DeleteStatus::AlreadyDeleted,
                    &snippet,
                    &library_name_of(&path),
                    args.json,
                )?;
                return Ok(CliOutcome::ConflictOrRefused);
            }
            eprintln!("Snippet not found: no live snippet matches the given selector.");
            Ok(CliOutcome::NotFound)
        }
    }
}

/// Turn a primitive outcome into the command's report.
///
/// `Deleted` succeeds, `AlreadyDeleted` is reported as a refusal (the
/// tombstone is already there, so nothing was changed), and `NotFound` means
/// the target vanished between the resolution and the mutation.
fn report_outcome(
    outcome: SnippetDeleteOutcome,
    library_name: &str,
    json: bool,
) -> SnipResult<CliOutcome> {
    match outcome {
        SnippetDeleteOutcome::Deleted(snippet) => {
            emit_result(DeleteStatus::Deleted, &snippet, library_name, json)?;
            Ok(CliOutcome::Success)
        }
        SnippetDeleteOutcome::AlreadyDeleted(snippet) => {
            emit_result(DeleteStatus::AlreadyDeleted, &snippet, library_name, json)?;
            Ok(CliOutcome::ConflictOrRefused)
        }
        SnippetDeleteOutcome::NotFound => {
            eprintln!("Snippet not found: the target was deleted before the mutation ran.");
            Ok(CliOutcome::NotFound)
        }
    }
}

/// Write the result document to stdout.
///
/// The human-readable result goes to stdout and the machine-readable JSON
/// replaces it; diagnostics never go to stdout.
fn emit_result(
    status: DeleteStatus,
    snippet: &crate::library::Snippet,
    library_name: &str,
    json: bool,
) -> SnipResult<()> {
    if json {
        let payload = DeleteJsonOutput {
            schema: 1,
            status: status.as_str().to_string(),
            id: snippet.id.clone(),
            description: snippet.description.clone(),
            library: library_name.to_string(),
        };
        let json_str = serde_json::to_string_pretty(&payload).map_err(|e| {
            SnipError::runtime_error("Failed to serialize delete result", Some(&e.to_string()))
        })?;
        println!("{json_str}");
    } else {
        match status {
            DeleteStatus::Deleted => {
                println!("Deleted snippet '{}' ({})", snippet.description, snippet.id);
            }
            DeleteStatus::AlreadyDeleted => {
                println!(
                    "Snippet '{}' ({}) is already deleted",
                    snippet.description, snippet.id
                );
            }
        }
    }
    Ok(())
}

/// Report an ambiguous target on stderr and change nothing.
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

/// Convert resolved matches into the identity list the ambiguity report uses.
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

/// The library file the selector scope resolves to.
///
/// `None` means the scope has no visible library. The path comes from the
/// same resolver the mutating selector uses, so the probe looks in exactly
/// the library the resolution searched.
fn scope_library_path(library: Option<&str>) -> SnipResult<Option<PathBuf>> {
    crate::commands::get_library_path(library.map(|name| name.to_string()))
}

/// The library name of a library file (its filename without `.toml`).
fn library_name_of(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Find an already-tombstoned snippet the selector would have matched.
///
/// The selector excludes deleted snippets from every mode, so a target that
/// was tombstoned earlier resolves as "not found". This probe re-runs the
/// same selector against a copy of the library whose `deleted` flags are
/// cleared, so the selector's own matching rules decide — the fuzzy ranking
/// and the case-sensitivity rules are not re-implemented here. The result is
/// mapped back onto the original library so the reported snippet keeps its
/// tombstone.
fn probe_deleted_target(
    selector: &SnippetSelector,
    library_path: &Path,
) -> SnipResult<Option<crate::library::Snippet>> {
    let snippets = crate::library::load_library(library_path)?;
    let mut visible = snippets.clone();
    for snippet in &mut visible.snippets {
        snippet.deleted = false;
    }
    let probe = SnippetSelector {
        resolution: ResolutionPolicy::All,
        ..selector.clone()
    };
    let matches = match probe.resolve(&visible, library_path, &library_name_of(library_path), "")? {
        SelectionResult::One(m) => vec![*m],
        SelectionResult::Many(matches) => matches,
        _ => Vec::new(),
    };
    Ok(matches.into_iter().find_map(|m| {
        snippets
            .snippets
            .iter()
            .find(|s| s.id == m.snippet.id && s.deleted)
            .cloned()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{LibraryManager, Snippet, Snippets};
    use tempfile::TempDir;

    /// A far-future `updated_at` so a tombstone's timestamp is
    /// deterministic: `updated_at.max(now).saturating_add(1)` keeps the
    /// fixture's value instead of the wall clock, which is what lets the
    /// interactive and the non-interactive delete be compared byte for byte.
    const FUTURE_UPDATED_AT: i64 = 4_000_000_000;

    /// An isolated config dir with one primary library holding `snippets`.
    ///
    /// The gate, the local-data lock, and the audit log are all derived from
    /// the config directory, so the override keeps the test hermetic.
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

    /// `snp delete --id <id>` with every other selector left unset.
    fn delete_by_id(id: &str) -> DeleteArgs {
        DeleteArgs {
            id: Some(id.to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            json: false,
        }
    }

    /// The interactive delete and the command must persist the same state.
    ///
    /// Both paths run against the same fixture; the persisted library files
    /// are compared byte for byte. The phases are sequential because the
    /// config-directory override is thread-local.
    #[test]
    fn interactive_and_command_deletes_persist_identical_state() {
        let interactive_bytes = {
            let fixture = Fixture::new(vec![
                Fixture::snippet("id-1", "git push", "git push origin main"),
                Fixture::snippet("id-2", "list files", "ls -la"),
            ]);
            // The interactive path: the picker hands back the library index
            // and the selection loop applies the delete.
            let mut picked = crate::library::load_library(&fixture.library_path)
                .expect("test helper: load fixture");
            let original_idx = picked
                .snippets
                .iter()
                .position(|s| s.id == "id-1")
                .expect("test helper: target present");
            crate::commands::apply_snippet_delete(
                &mut picked,
                &fixture.library_path,
                original_idx,
                crate::commands::DeleteSyncTrigger::AutoNotify,
            )
            .expect("interactive delete succeeds");
            // The picker's in-memory copy mirrors the tombstone.
            assert!(picked.snippets[original_idx].deleted);
            fixture.library_bytes()
        };

        let command_bytes = {
            let fixture = Fixture::new(vec![
                Fixture::snippet("id-1", "git push", "git push origin main"),
                Fixture::snippet("id-2", "list files", "ls -la"),
            ]);
            let outcome = run(delete_by_id("id-1")).expect("command delete succeeds");
            assert_eq!(outcome, CliOutcome::Success);
            fixture.library_bytes()
        };

        assert_eq!(
            interactive_bytes, command_bytes,
            "the interactive and the non-interactive delete must persist identical state"
        );
    }

    #[test]
    fn ambiguous_selector_deletes_nothing_and_changes_no_file() {
        let fixture = Fixture::new(vec![
            Fixture::snippet("id-1", "duplicate", "echo one"),
            Fixture::snippet("id-2", "duplicate", "echo two"),
        ]);
        let before = fixture.library_bytes();

        let outcome = run(DeleteArgs {
            id: None,
            description_exact: Some("duplicate".to_string()),
            command_exact: None,
            query: None,
            library: None,
            json: false,
        })
        .expect("ambiguity is reported as an outcome, not an error");

        assert_eq!(outcome, CliOutcome::Ambiguous);
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn already_deleted_target_is_reported_distinctly_and_changes_nothing() {
        let fixture = Fixture::new(vec![Fixture::snippet("id-1", "git push", "git push")]);

        let first = run(delete_by_id("id-1")).expect("first delete succeeds");
        assert_eq!(first, CliOutcome::Success);
        let after_first = fixture.library_bytes();

        let second = run(delete_by_id("id-1")).expect("second delete reports a verdict");
        assert_eq!(
            second,
            CliOutcome::ConflictOrRefused,
            "an already-deleted target is reported distinctly from a not-found target"
        );
        assert_eq!(fixture.library_bytes(), after_first);
    }

    #[test]
    fn missing_target_is_reported_as_not_found_and_changes_nothing() {
        let fixture = Fixture::new(vec![Fixture::snippet("id-1", "git push", "git push")]);
        let before = fixture.library_bytes();

        let outcome = run(delete_by_id("does-not-exist")).expect("missing target is reported");
        assert_eq!(outcome, CliOutcome::NotFound);
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn no_selector_is_a_usage_error() {
        let error = run(DeleteArgs {
            id: None,
            description_exact: None,
            command_exact: None,
            query: None,
            library: None,
            json: false,
        })
        .expect_err("a delete without a selector must fail");
        assert!(error.to_string().contains("No selector specified"));
    }

    #[test]
    fn cross_library_scope_is_refused() {
        let error = run(DeleteArgs {
            id: Some("id-1".to_string()),
            description_exact: None,
            command_exact: None,
            query: None,
            library: Some("all".to_string()),
            json: false,
        })
        .expect_err("the cross-library scope must be refused");
        assert!(error.to_string().contains("not supported"));
    }
}

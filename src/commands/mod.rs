//! CLI command implementations.
//!
//! Each subcommand in the CLI has its own module with a `run()` function.
//! This module also provides shared utilities for loading and saving snippets.

pub mod clip_cmd;
pub mod cron_cmd;
pub mod doctor_cmd;
pub mod edit_cmd;
pub mod import_cmd;
pub mod keybindings_cmd;
pub mod library_cmd;
pub mod list_cmd;
pub mod new_cmd;
pub mod pet_analysis;
pub mod premade_cmd;
pub mod register_cmd;
pub mod run_cmd;
pub mod search_cmd;
pub mod select_cmd;
pub mod shell_cmd;
pub mod sync_cmd;

use crate::config::invalidate_toml_cache;
use crate::error::{SnipError, SnipResult};
use crate::sort::SortOptions;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::path::PathBuf;

/// Result of expanding a snippet command with variables.
pub enum ExpandedCommand {
    /// User cancelled the operation.
    Cancel,
    /// User chose to skip (continue to next snippet).
    Skip,
    /// Command was expanded successfully.
    Expanded(String),
}

/// Resolves the config path from CLI argument or default location.
pub fn get_config_path(config: &Option<PathBuf>) -> SnipResult<PathBuf> {
    use std::fs::{self, OpenOptions};
    use std::io::ErrorKind;

    match config {
        Some(path) => {
            if path.is_file() {
                Ok(path.clone())
            } else if path.exists() {
                Err(SnipError::runtime_error(
                    "Config path is not a file",
                    Some(&format!(
                        "'{}' exists but is not a regular file",
                        path.display()
                    )),
                ))
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| SnipError::io_error("create directory", parent, e))?;
                }
                match OpenOptions::new().write(true).create_new(true).open(path) {
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::AlreadyExists && path.is_file() => {}
                    Err(e) => {
                        return Err(SnipError::io_error("create config file", path.clone(), e));
                    }
                }
                Ok(path.clone())
            }
        }
        None => Ok(crate::utils::config::get_snippets_path()),
    }
}

/// Resolves the path to a named library or returns the primary library path.
pub fn get_library_path(library_name: Option<String>) -> SnipResult<Option<PathBuf>> {
    use crate::library::LibraryManager;

    let mut mgr = LibraryManager::new()?;

    mgr.ensure_library_mode()?;

    let path = match library_name {
        Some(name) => {
            let lib = mgr.get_library_by_filename(&name)
                .ok_or_else(|| SnipError::runtime_error(
                    "Library not found",
                    Some(&format!("Library '{name}' does not exist. Use 'snp library list' to see available libraries.")),
                ))?;
            Some(
                mgr.get_libraries_dir()
                    .join(format!("{}.toml", lib.filename)),
            )
        }
        None => mgr.get_primary_library().map(|primary| {
            mgr.get_libraries_dir()
                .join(format!("{}.toml", primary.filename))
        }),
    };

    Ok(path)
}

/// Initializes a LibraryManager with library mode enabled, handling errors gracefully.
pub fn init_library_manager() -> SnipResult<crate::library::LibraryManager> {
    let mut mgr = crate::library::LibraryManager::new()?;
    mgr.ensure_library_mode()?;
    Ok(mgr)
}

/// Loads snippets from a TOML file, returning an empty collection if the file doesn't exist.
pub fn load_snippets(config: &Option<PathBuf>) -> SnipResult<crate::library::Snippets> {
    use std::fs;

    let path = get_config_path(config)?;

    if !path.exists() {
        crate::logging::log_config_operation("load", &path, &Err("file not found"));
        return Ok(crate::library::Snippets::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| {
        crate::logging::log_config_operation("load", &path, &Err(&e.to_string()));
        SnipError::io_error("read snippets file", path.clone(), e)
    })?;

    if content.is_empty() || content.trim().is_empty() {
        crate::logging::log_config_operation("load", &path, &Ok(()));
        return Ok(crate::library::Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: crate::library::Snippets = toml::from_str(&fixed_content).map_err(|e| {
        crate::logging::log_config_operation("parse", &path, &Err(&e.to_string()));
        let backup_path = path.with_extension("toml.bak");
        if let Err(backup_err) = std::fs::copy(&path, &backup_path) {
            tracing::warn!(
                error = %e,
                backup_error = %backup_err,
                "Failed to parse config and could not create backup"
            );
        } else {
            tracing::warn!(
                backup = %backup_path.display(),
                "Failed to parse config file. Backup saved."
            );
        }
        SnipError::toml_error("parse snippets file", e)
    })?;

    crate::logging::log_config_operation("load", &path, &Ok(()));

    Ok(snippets)
}

/// Saves snippets to a TOML file, creating directories as needed.
///
/// Uses atomic write (temp file + rename) and creates a backup before saving,
/// matching the safety guarantees of `save_library`.
pub fn save_snippets(s: &crate::library::Snippets, config: &Option<PathBuf>) -> SnipResult<()> {
    let path = get_config_path(config)?;

    if let Err(e) = crate::library::backup_library(&path) {
        tracing::warn!(error = %e, "Failed to create backup before save");
    }

    let toml_str =
        toml::to_string_pretty(s).map_err(|e| SnipError::toml_error("serialize config", e))?;

    let temp_prefix = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    crate::utils::atomic::write_private_atomic(&path, &toml_str, temp_prefix)?;

    invalidate_toml_cache(&path);

    crate::logging::log_config_operation("save", &path, &Ok(()));
    Ok(())
}

/// Extracts snippet data arrays for TUI display.
///
/// Returns parallel arrays of descriptions, commands, tags, folders, and favorites,
/// along with a mapping from filtered indices to original snippet indices.
pub fn get_snippet_data(snippets: &crate::library::Snippets) -> (crate::SnippetData, Vec<usize>) {
    let filtered: Vec<_> = snippets
        .snippets
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.deleted)
        .collect();
    let original_indices: Vec<usize> = filtered.iter().map(|(i, _)| *i).collect();
    let descriptions: Vec<String> = filtered
        .iter()
        .map(|(_, s)| s.description.clone())
        .collect();
    let commands: Vec<String> = filtered.iter().map(|(_, s)| s.command.clone()).collect();
    let outputs: Vec<String> = filtered.iter().map(|(_, s)| s.output.clone()).collect();
    let tags: Vec<Vec<String>> = filtered.iter().map(|(_, s)| s.tags.clone()).collect();
    let folders: Vec<Vec<String>> = filtered.iter().map(|(_, s)| s.folders.clone()).collect();
    let favorites: Vec<bool> = filtered.iter().map(|(_, s)| s.favorite).collect();
    (
        crate::SnippetData {
            descriptions,
            commands,
            outputs,
            tags,
            folders,
            favorites,
        },
        original_indices,
    )
}

/// Expands a snippet command, prompting for variables if present.
pub fn expand_snippet_command(snippet: &crate::library::Snippet) -> SnipResult<ExpandedCommand> {
    use crate::ui;
    use crate::utils::{parse_variables, strip_escape_sequences};

    let vars = parse_variables(&snippet.command);
    if vars.is_empty() {
        return Ok(ExpandedCommand::Expanded(strip_escape_sequences(
            &snippet.command,
        )));
    }

    match ui::prompt_variables(vars)? {
        ui::VariablePromptResult::Cancel => Ok(ExpandedCommand::Cancel),
        ui::VariablePromptResult::Back => Ok(ExpandedCommand::Skip),
        ui::VariablePromptResult::Skip => Ok(ExpandedCommand::Skip),
        ui::VariablePromptResult::Values(values) => Ok(ExpandedCommand::Expanded(
            crate::utils::expand_command(&snippet.command, &values),
        )),
    }
}

/// Opens the TUI snippet selector and runs the given processing function on selection.
///
/// Handles loading the library, extracting snippet data, and optionally running
/// a background sync after processing. The `process_fn` callback is invoked with
/// the selected snippet and any copy flag from the TUI.
///
/// Returns `SelectionOutcome::Selected` when a snippet was selected and processed,
/// or `SelectionOutcome::Cancelled` when the user cancelled the primary selector.
pub fn run_snippet_selection<F>(
    filter: Option<String>,
    library: Option<String>,
    do_sync: bool,
    sort_opts: Option<SortOptions>,
    runtime: &tokio::runtime::Runtime,
    mut process_fn: F,
) -> crate::error::SnipResult<crate::SelectionOutcome>
where
    F: FnMut(
        &crate::library::Snippet,
        Option<String>,
    ) -> crate::error::SnipResult<crate::ProcessResult>,
{
    let lib_path = match get_library_path(library)? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Ok(crate::SelectionOutcome::Cancelled);
        }
    };
    let mut snippets = crate::library::load_library(&lib_path)?;

    let usage_index = crate::usage::UsageIndex::load();
    let usage_data: Vec<crate::usage::UsageData> = snippets
        .snippets
        .iter()
        .map(|s| usage_index.get_usage(&s.id))
        .collect();

    let mut selected_and_processed = false;
    let mut cancelled = false;
    loop {
        let (snippet_data, original_indices) = get_snippet_data(&snippets);
        let result = crate::ui::select_snippet(crate::ui::SnippetListParams {
            descriptions: &snippet_data.descriptions,
            commands: &snippet_data.commands,
            tags: &snippet_data.tags,
            is_search: false,
            initial_filter: filter.as_deref(),
            folders: &snippet_data.folders,
            favorites: &snippet_data.favorites,
            snippets: &snippets.snippets,
            original_indices: &original_indices,
            sort_opts: sort_opts.as_ref(),
            usage: Some(&usage_data),
        })?;
        if let Some(result) = result {
            match result {
                crate::ui::SnippetSelection::Cancelled => {
                    cancelled = true;
                    break;
                }
                crate::ui::SnippetSelection::Delete(idx) => {
                    let original_idx = *original_indices.get(idx).ok_or_else(|| {
                        SnipError::runtime_error(
                            "Snippet not found",
                            Some("The selected snippet is no longer available"),
                        )
                    })?;
                    let deleted_snippet = mark_snippet_deleted(&mut snippets, original_idx)?;
                    crate::library::save_library(&lib_path, &snippets)?;
                    if let Err(e) = crate::logging::audit_log("delete", &deleted_snippet, None) {
                        tracing::debug!("Audit log write failed: {}", e);
                    }
                    if do_sync {
                        // Explicit sync: run immediately and clear pending auto-sync
                        // to prevent duplicate delayed sync (Workstream D).
                        let state_dir = crate::auto_sync::notification::derive_state_dir();
                        match crate::auto_sync::execution_lock::wait_acquire(
                            &state_dir,
                            std::time::Duration::from_secs(30),
                        ) {
                            Ok(_exec_lock) => {
                                let observed = crate::auto_sync::observe_pending_generation();
                                let sync_result = crate::sync_commands::run_default_sync(runtime);
                                let sync_succeeded = sync_result.is_ok();
                                if sync_result.is_err() {
                                    tracing::warn!("post-delete sync failed");
                                }
                                crate::auto_sync::clear_pending_after_explicit_sync(
                                    observed,
                                    sync_succeeded,
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "could not acquire sync lock for post-delete sync"
                                );
                            }
                        }
                    } else {
                        // Auto-sync trigger: notify after successful delete commit (Workstream B3).
                        crate::auto_sync::notify_mutation(
                            crate::auto_sync::MutationKind::SnippetDelete,
                            crate::auto_sync::MutationOrigin::User,
                        );
                    }
                    continue;
                }
                crate::ui::SnippetSelection::Selected(idx, copy_flag) => {
                    let snippet = &snippets.snippets[original_indices[idx]];
                    match process_fn(snippet, copy_flag)? {
                        crate::ProcessResult::Cancel => {
                            cancelled = true;
                            break;
                        }
                        crate::ProcessResult::Continue => continue,
                        crate::ProcessResult::Done(_msg) => {
                            selected_and_processed = true;
                            break;
                        }
                    }
                }
            }
        } else {
            cancelled = true;
            break;
        }
    }
    let explicit_observed = if do_sync && selected_and_processed {
        crate::auto_sync::observe_pending_generation()
    } else {
        None
    };
    let explicit_sync_succeeded = if do_sync && selected_and_processed {
        let state_dir = crate::auto_sync::notification::derive_state_dir();
        match crate::auto_sync::execution_lock::wait_acquire(
            &state_dir,
            std::time::Duration::from_secs(30),
        ) {
            Ok(_exec_lock) => match crate::sync_commands::run_default_sync(runtime) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(error = %e, "Background sync failed");
                    false
                }
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "could not acquire sync lock for post-selection sync"
                );
                false
            }
        }
    } else {
        false
    };
    if do_sync && selected_and_processed {
        // Explicit sync completed: clear pending auto-sync to prevent
        // duplicate delayed sync (Workstream D).
        crate::auto_sync::clear_pending_after_explicit_sync(
            explicit_observed,
            explicit_sync_succeeded,
        );
    }
    if cancelled {
        Ok(crate::SelectionOutcome::Cancelled)
    } else {
        Ok(crate::SelectionOutcome::Selected)
    }
}

/// Marks a selected snippet as deleted while preserving its tombstone for sync.
fn mark_snippet_deleted(
    snippets: &mut crate::library::Snippets,
    original_idx: usize,
) -> SnipResult<crate::library::Snippet> {
    let snippet = snippets.snippets.get_mut(original_idx).ok_or_else(|| {
        SnipError::runtime_error(
            "Snippet not found",
            Some("The selected snippet is no longer available"),
        )
    })?;

    snippet.deleted = true;
    let now = chrono::Utc::now().timestamp();
    snippet.updated_at = snippet.updated_at.max(now).saturating_add(1);
    Ok(snippet.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_snippets_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = Some(tmp.path().join("nonexistent.toml"));
        let snippets = load_snippets(&path).unwrap();
        assert!(snippets.snippets.is_empty());
    }

    #[test]
    fn test_load_snippets_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.toml");
        std::fs::write(&path, "").unwrap();
        let snippets = load_snippets(&Some(path)).unwrap();
        assert!(snippets.snippets.is_empty());
    }

    #[test]
    fn test_load_snippets_valid_toml() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("valid.toml");
        std::fs::write(
            &path,
            r#"[[Snippets]]
description = "test"
command = "echo hello"
"#,
        )
        .unwrap();
        let snippets = load_snippets(&Some(path)).unwrap();
        assert_eq!(snippets.snippets.len(), 1);
        assert_eq!(snippets.snippets[0].description, "test");
        assert_eq!(snippets.snippets[0].command, "echo hello");
    }

    #[test]
    fn test_load_snippets_invalid_toml_creates_backup() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("invalid.toml");
        std::fs::write(&path, "invalid = [toml").unwrap();
        let backup_path = path.with_extension("toml.bak");
        let result = load_snippets(&Some(path));
        assert!(result.is_err());
        assert!(backup_path.exists());
    }

    #[test]
    fn test_get_snippet_data_filters_deleted() {
        let snippets = crate::library::Snippets {
            snippets: vec![
                crate::library::Snippet {
                    id: "1".to_string(),
                    description: "active".to_string(),
                    command: "echo 1".to_string(),
                    ..Default::default()
                },
                crate::library::Snippet {
                    id: "2".to_string(),
                    description: "deleted".to_string(),
                    command: "echo 2".to_string(),
                    deleted: true,
                    ..Default::default()
                },
                crate::library::Snippet {
                    id: "3".to_string(),
                    description: "also active".to_string(),
                    command: "echo 3".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let (data, indices) = get_snippet_data(&snippets);
        assert_eq!(data.descriptions.len(), 2);
        assert_eq!(data.descriptions[0], "active");
        assert_eq!(data.descriptions[1], "also active");
        assert_eq!(indices, vec![0, 2]);
    }

    #[test]
    fn test_mark_snippet_deleted_preserves_tombstone_for_sync() {
        let mut snippets = crate::library::Snippets {
            snippets: vec![crate::library::Snippet {
                id: "1".to_string(),
                description: "remove me".to_string(),
                command: "echo remove me".to_string(),
                updated_at: 10,
                ..Default::default()
            }],
            ..Default::default()
        };

        let deleted = mark_snippet_deleted(&mut snippets, 0).unwrap();

        assert!(deleted.deleted);
        assert!(deleted.updated_at > 10);
        assert!(snippets.snippets[0].deleted);
        assert_eq!(snippets.snippets[0].command, "echo remove me");
    }
}

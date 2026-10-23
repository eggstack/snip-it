//! **Layer: Application**
//!
//! CLI command implementations.
//!
//! Each subcommand in the CLI has its own module with a `run()` function.
//! This module also provides shared utilities for loading and saving snippets.

pub mod backup_archive;
pub mod backup_cmd;
pub mod clip_cmd;
pub mod cron_cmd;
pub mod delete_cmd;
pub mod doctor_cmd;
pub mod doctor_report;
pub mod edit_cmd;
pub mod get_cmd;
pub mod import_cmd;
pub mod keybindings_cmd;
pub mod library_cmd;
pub mod list_cmd;
pub mod new_cmd;
pub mod pet_analysis;
pub mod premade_cmd;
pub mod register_cmd;
pub mod repair_cmd;
pub mod restore_cmd;
pub mod run_cmd;
pub mod search_cmd;
pub mod select_cmd;
pub mod shell_cmd;
pub mod status_cmd;
pub mod sync_cmd;
pub mod validate_cmd;

use crate::config::invalidate_toml_cache;
use crate::error::{SnipError, SnipResult};
use crate::sort::SortOptions;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::path::PathBuf;

/// Result of expanding a snippet command with variables.
#[non_exhaustive]
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
            // Single `metadata` call (follows symlinks, like `is_file`/`exists`)
            // so a concurrent delete/replace cannot misclassify the error.
            match fs::metadata(path) {
                Ok(meta) if meta.is_file() => Ok(path.clone()),
                Ok(_) => Err(SnipError::runtime_error(
                    "Config path is not a file",
                    Some(&format!(
                        "'{}' exists but is not a regular file",
                        path.display()
                    )),
                )),
                Err(e) if e.kind() == ErrorKind::NotFound => {
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
                Err(e) => Err(SnipError::io_error("stat config path", path.clone(), e)),
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
            let lib = mgr
                .get_library_by_filename(&name)
                .ok_or_else(|| crate::library::library_not_found(&name))?;
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

/// How a selection command resolves the library it browses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LibraryResolution {
    /// Resolve through [`get_library_path`], which calls
    /// `ensure_library_mode()` and can therefore migrate a legacy
    /// single-file checkout into `libraries/`.
    ///
    /// Only for commands that are allowed to rewrite library state.
    Mutable,
    /// Resolve through the canonical read-only resolver: no migration, no
    /// file or directory creation, no generation bump.
    ReadOnly,
}

/// Resolves the path of the single library a selection command browses,
/// without mutating any state.
///
/// Read-only counterpart of [`get_library_path`]. It reuses the canonical
/// [`crate::library::readonly_library_sources`] resolver — the same one
/// `snp list`, `snp get`, and `snp validate` use — so a legacy single-file
/// checkout is read *in place*: no `libraries/` directory, no
/// `libraries.toml`, no generation bump.
///
/// Returns `None` when no library is visible (the read-only counterpart of
/// the "No library found" case). The canonical `all` scope selects several
/// libraries at once and cannot be represented by a single path, so it is
/// rejected the same way the mutating resolver rejects an unknown library.
pub(crate) fn readonly_library_path(library: Option<&str>) -> SnipResult<Option<PathBuf>> {
    if library == Some("all") {
        return Err(crate::library::library_not_found("all"));
    }
    let sources = crate::library::readonly_library_sources(library)?;
    match sources.as_slice() {
        [] => Ok(None),
        [source] => Ok(Some(source.path.clone())),
        _ => Err(SnipError::runtime_error(
            "Multiple libraries selected",
            Some("This command browses exactly one library"),
        )),
    }
}

/// Loads snippets for a read-only command across the resolved library scope.
///
/// Uses [`crate::library::readonly_library_sources`], which performs no
/// migration and creates no files or directories. It also routes `"all"`
/// through [`crate::selector::LibraryScope::from_filter_arg`], the single
/// definition of the keyword, so `--library all` means the same thing here as
/// on `snp get`, exact-mode `run`/`clip`/`edit`, and MCP.
///
/// Returns the resolved sources (empty when no library is configured) together
/// with the concatenation of their snippets, in resolution order.
pub fn load_readonly_snippets(
    library: Option<&str>,
) -> SnipResult<(
    Vec<crate::library::ResolvedLibrarySource>,
    crate::library::Snippets,
)> {
    let sources = crate::library::readonly_library_sources(library)?;

    let mut merged = crate::library::Snippets::default();
    for source in &sources {
        let loaded = crate::library::load_library(&source.path)?;
        merged.snippets.extend(loaded.snippets);
        for folder in loaded.folders {
            if !merged.folders.contains(&folder) {
                merged.folders.push(folder);
            }
        }
    }

    Ok((sources, merged))
}

/// Initializes a LibraryManager with library mode enabled, handling errors gracefully.
pub fn init_library_manager() -> SnipResult<crate::library::LibraryManager> {
    let mut mgr = crate::library::LibraryManager::new()?;
    mgr.ensure_library_mode()?;
    Ok(mgr)
}

/// Loads snippets from a TOML file, returning an empty collection if the file doesn't exist.
pub fn load_snippets(config: &Option<PathBuf>) -> SnipResult<crate::library::Snippets> {
    let path = get_config_path(config)?;
    load_snippets_at(&path)
}

/// Loads snippets from an explicitly given TOML path without mutating
/// anything.
///
/// The read-only counterpart of [`load_snippets`]: a missing file yields an
/// empty collection instead of being created together with its parent
/// directories, and a malformed file is reported without writing a `.bak`
/// sibling next to it. Read-only commands must use this variant.
pub fn load_snippets_readonly_at(path: &std::path::Path) -> SnipResult<crate::library::Snippets> {
    use std::fs;

    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(SnipError::runtime_error(
                "Config path is not a file",
                Some(&format!(
                    "'{}' exists but is not a regular file",
                    path.display()
                )),
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A missing file is an empty collection — never create it.
            return Ok(crate::library::Snippets::default());
        }
        Err(e) => {
            return Err(SnipError::io_error(
                "stat config path",
                path.to_path_buf(),
                e,
            ));
        }
    }

    let content = fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read snippets file", path.to_path_buf(), e))?;

    if content.trim().is_empty() {
        return Ok(crate::library::Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);
    toml::from_str(&fixed_content).map_err(|e| SnipError::toml_error("parse snippets file", e))
}

/// Loads snippets from an already-resolved TOML path.
fn load_snippets_at(path: &std::path::Path) -> SnipResult<crate::library::Snippets> {
    use std::fs;

    if !path.exists() {
        crate::logging::log_config_operation("load", path, &Err("file not found"));
        return Ok(crate::library::Snippets::default());
    }

    let content = fs::read_to_string(path).map_err(|e| {
        crate::logging::log_config_operation("load", path, &Err(&e.to_string()));
        SnipError::io_error("read snippets file", path.to_path_buf(), e)
    })?;

    if content.is_empty() || content.trim().is_empty() {
        crate::logging::log_config_operation("load", path, &Ok(()));
        return Ok(crate::library::Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: crate::library::Snippets = toml::from_str(&fixed_content).map_err(|e| {
        crate::logging::log_config_operation("parse", path, &Err(&e.to_string()));
        let backup_path = path.with_extension("toml.bak");
        if let Err(backup_err) = std::fs::copy(path, &backup_path) {
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

    crate::logging::log_config_operation("load", path, &Ok(()));

    Ok(snippets)
}

/// Saves snippets to a TOML file, creating directories as needed.
///
/// Uses atomic write (temp file + rename) and creates a backup before saving,
/// matching the safety guarantees of `save_library`.
pub fn save_snippets(s: &crate::library::Snippets, config: &Option<PathBuf>) -> SnipResult<()> {
    let path = get_config_path(config)?;

    // Check for interrupted transactions before any mutation.
    let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
    let transaction_dir = crate::local_data::transaction_dir();
    crate::transaction::gate_mutation_on_interrupted_transactions(
        &sync_state_dir,
        &transaction_dir,
    )?;

    // Acquire the local-data lock to serialize against backup snapshot capture.
    let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;

    save_snippets_at(&path, s)
}

/// Writes the already-resolved snippet file.
///
/// The caller must hold the local-data lock.
fn save_snippets_at(path: &std::path::Path, s: &crate::library::Snippets) -> SnipResult<()> {
    if let Err(e) = crate::library::backup_library(path) {
        tracing::warn!(error = %e, "Failed to create backup before save");
    }

    let toml_str =
        toml::to_string_pretty(s).map_err(|e| SnipError::toml_error("serialize config", e))?;

    let temp_prefix = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    crate::utils::atomic::write_private_atomic(path, &toml_str, temp_prefix)?;

    invalidate_toml_cache(path);

    crate::logging::log_config_operation("save", path, &Ok(()));
    Ok(())
}

/// Atomically applies a mutation to the legacy single-file snippet store.
///
/// The file is loaded, mutated, and saved while the local-data lock is held
/// for the whole cycle, so a concurrent writer cannot lose either side's
/// changes. This is the legacy-mode counterpart of
/// [`crate::library::update_library`].
pub fn update_snippets<F>(config: &Option<PathBuf>, mutate: F) -> SnipResult<()>
where
    F: FnOnce(&mut crate::library::Snippets) -> SnipResult<()>,
{
    let path = get_config_path(config)?;

    // Check for interrupted transactions before any mutation.
    let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
    let transaction_dir = crate::local_data::transaction_dir();
    crate::transaction::gate_mutation_on_interrupted_transactions(
        &sync_state_dir,
        &transaction_dir,
    )?;

    // The local-data lock is not reentrant, so the save below must use the
    // internal variant instead of `save_snippets`.
    let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;

    let mut snippets = load_snippets_at(&path)?;
    mutate(&mut snippets)?;
    save_snippets_at(&path, &snippets)
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

/// Runs a canonical explicit sync: acquire execution lock, observe pending
/// generation, run default sync, and clear pending if successful.
///
/// This is the single implementation shared by TUI `--sync` paths and
/// exact-selector `--sync` paths. It must be called after any local
/// mutation has committed (or after the local operation completes).
pub fn run_explicit_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let state_dir = crate::auto_sync::notification::derive_state_dir();
    let observed = crate::auto_sync::observe_pending_generation();
    match crate::auto_sync::execution_lock::wait_acquire(
        &state_dir,
        std::time::Duration::from_secs(30),
    ) {
        Ok(_exec_lock) => {
            let sync_result = crate::sync_commands::run_default_sync(runtime);
            let sync_succeeded = sync_result.is_ok();
            if sync_result.is_err() {
                tracing::warn!("explicit sync failed");
            }
            crate::auto_sync::clear_pending_after_explicit_sync(observed, sync_succeeded);
            sync_result
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not acquire sync lock for explicit sync"
            );
            Err(SnipError::runtime_error(
                "could not acquire sync lock",
                Some(&e.to_string()),
            ))
        }
    }
}

/// Resolves the library a selection command browses.
///
/// Read-only resolution reuses [`readonly_library_path`], so a legacy
/// single-file checkout is read in place instead of being migrated.
fn resolve_selection_library_path(
    library: Option<&str>,
    resolution: LibraryResolution,
) -> SnipResult<Option<PathBuf>> {
    match resolution {
        LibraryResolution::Mutable => get_library_path(library.map(str::to_string)),
        LibraryResolution::ReadOnly => readonly_library_path(library),
    }
}

/// Opens the TUI snippet selector and runs the given processing function on selection.
///
/// Handles loading the library, extracting snippet data, and optionally running
/// a background sync after processing. The `process_fn` callback is invoked with
/// the selected snippet and any copy flag from the TUI.
///
/// The browsed library is resolved through the mutating resolver, which may
/// migrate a legacy single-file checkout. Read-only callers must use
/// [`run_snippet_selection_with_resolution`] with
/// [`LibraryResolution::ReadOnly`] instead.
///
/// Returns `SelectionOutcome::Selected` when a snippet was selected and processed,
/// or `SelectionOutcome::Cancelled` when the user cancelled the primary selector.
pub fn run_snippet_selection<F>(
    filter: Option<String>,
    library: Option<String>,
    do_sync: bool,
    allow_delete: bool,
    sort_opts: Option<SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
    process_fn: F,
) -> crate::error::SnipResult<crate::SelectionOutcome>
where
    F: FnMut(
        &crate::library::Snippet,
        Option<String>,
    ) -> crate::error::SnipResult<crate::ProcessResult>,
{
    run_snippet_selection_with_resolution(
        filter,
        library,
        LibraryResolution::Mutable,
        do_sync,
        allow_delete,
        sort_opts,
        runtime,
        process_fn,
    )
}

/// Same as [`run_snippet_selection`], with an explicit library-resolution mode.
///
/// `resolution` selects how the browsed library is resolved:
/// [`LibraryResolution::ReadOnly`] never creates or rewrites library state
/// (used by `snp select`, which is classified read-only), while
/// [`LibraryResolution::Mutable`] keeps the historical behaviour of
/// `ensure_library_mode()`.
pub(crate) fn run_snippet_selection_with_resolution<F>(
    filter: Option<String>,
    library: Option<String>,
    resolution: LibraryResolution,
    do_sync: bool,
    allow_delete: bool,
    sort_opts: Option<SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
    mut process_fn: F,
) -> crate::error::SnipResult<crate::SelectionOutcome>
where
    F: FnMut(
        &crate::library::Snippet,
        Option<String>,
    ) -> crate::error::SnipResult<crate::ProcessResult>,
{
    if resolution == LibraryResolution::ReadOnly && allow_delete {
        return Err(SnipError::runtime_error(
            "Read-only selection cannot delete snippets",
            Some("allow_delete must be false for a read-only library resolution"),
        ));
    }

    let runtime = if do_sync {
        Some(runtime.ok_or_else(|| {
            SnipError::runtime_error(
                "sync requested but no runtime",
                Some("run_snippet_selection called with do_sync=true and runtime=None"),
            )
        })?)
    } else {
        None
    };

    // The sync trigger the shared delete primitive performs once its
    // mutation is durable: an explicit sync when the caller asked for one,
    // otherwise the auto-sync notification. `runtime.is_some()` holds
    // exactly when `do_sync` is true.
    let delete_trigger = match runtime {
        Some(rt) => DeleteSyncTrigger::ExplicitSync(rt),
        None => DeleteSyncTrigger::AutoNotify,
    };

    let lib_path = match resolve_selection_library_path(library.as_deref(), resolution)? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Ok(crate::SelectionOutcome::Cancelled);
        }
    };
    let mut snippets = crate::library::load_library(&lib_path)?;

    let usage_index = crate::usage::UsageIndex::load();

    let mut selected_and_processed = false;
    let mut cancelled = false;
    let mut failed_exit_code: Option<Option<i32>> = None;
    loop {
        let (snippet_data, original_indices) = get_snippet_data(&snippets);
        let live_snippets: Vec<crate::library::Snippet> = original_indices
            .iter()
            .map(|&i| snippets.snippets[i].clone())
            .collect();
        // One-shot index so per-snippet usage lookup is O(1) instead of a
        // linear scan per snippet (O(k*m) over the selection).
        let usage_map: std::collections::HashMap<&str, &crate::usage::UsageEntry> = usage_index
            .entries()
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect();
        let live_usage: Vec<crate::usage::UsageData> = live_snippets
            .iter()
            .map(|s| {
                usage_map
                    .get(s.id.as_str())
                    .map(|e| crate::usage::UsageData {
                        use_count: e.use_count,
                        last_used_at: e.last_used_at,
                    })
                    .unwrap_or_default()
            })
            .collect();
        let result = crate::ui::select_snippet(crate::ui::SnippetListParams {
            descriptions: &snippet_data.descriptions,
            commands: &snippet_data.commands,
            tags: &snippet_data.tags,
            is_search: false,
            initial_filter: filter.as_deref(),
            folders: &snippet_data.folders,
            favorites: &snippet_data.favorites,
            snippets: &live_snippets,
            original_indices: &original_indices,
            sort_opts: sort_opts.as_ref(),
            usage: Some(&live_usage),
            allow_delete,
        })?;
        if let Some(result) = result {
            match result {
                crate::ui::SnippetSelection::Cancelled => {
                    cancelled = true;
                    break;
                }
                crate::ui::SnippetSelection::Delete(idx) if allow_delete => {
                    let original_idx = *original_indices.get(idx).ok_or_else(|| {
                        SnipError::runtime_error(
                            "Snippet not found",
                            Some("The selected snippet is no longer available"),
                        )
                    })?;
                    // The delete itself is the shared primitive; only the
                    // picker's in-memory mirror stays in this loop.
                    apply_snippet_delete(&mut snippets, &lib_path, original_idx, delete_trigger)?;
                    continue;
                }
                crate::ui::SnippetSelection::Delete(_) => continue,
                crate::ui::SnippetSelection::Copied => {
                    selected_and_processed = true;
                    break;
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
                        crate::ProcessResult::Failed { exit_code, .. } => {
                            selected_and_processed = true;
                            failed_exit_code = Some(exit_code);
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
    if do_sync
        && selected_and_processed
        && let Some(rt) = runtime
        && let Err(e) = run_explicit_sync(rt)
    {
        tracing::warn!(error = %e, "post-selection explicit sync failed");
    }
    if cancelled {
        Ok(crate::SelectionOutcome::Cancelled)
    } else if let Some(exit_code) = failed_exit_code {
        Ok(crate::SelectionOutcome::ExecutionFailed { exit_code })
    } else {
        Ok(crate::SelectionOutcome::Selected)
    }
}

/// The sync trigger a snippet delete performs once its mutation is durable.
///
/// The interactive path asks for an explicit sync when the caller passed
/// `--sync`; every other caller gets the auto-sync notification.
#[derive(Debug, Clone, Copy)]
pub enum DeleteSyncTrigger<'a> {
    /// Notify the auto-sync subsystem after the mutation commits.
    AutoNotify,
    /// Run an explicit sync with the caller's runtime after the mutation commits.
    ExplicitSync(&'a tokio::runtime::Runtime),
}

/// The outcome of [`delete_snippet`].
///
/// `AlreadyDeleted` and `NotFound` are distinct: the first means the
/// identifier exists but already carries a tombstone, the second means no
/// snippet with that identifier exists at all.
#[derive(Debug, Clone)]
pub enum SnippetDeleteOutcome {
    /// The snippet was tombstoned and persisted.
    Deleted(crate::library::Snippet),
    /// The target exists but was already tombstoned; nothing was changed.
    AlreadyDeleted(crate::library::Snippet),
    /// No snippet with that identifier exists; nothing was changed.
    NotFound,
}

/// Why the shared delete primitive could not tombstone the target.
///
/// The lookup happens inside `library::update_library`, so the distinction
/// has to travel out of the mutation closure.
enum DeleteProbe {
    /// The identifier matches a snippet that already carries a tombstone.
    Tombstoned(crate::library::Snippet),
    /// The identifier matches nothing in the library.
    Missing,
}

/// Tombstone a snippet by identifier — the one shared snippet-delete primitive.
///
/// This is the single implementation of the delete sequence. It performs, in
/// one place:
///
/// 1. the mutation gate and the local-data lock, through
///    [`crate::library::update_library`];
/// 2. the re-read of the library inside the critical section, so a snippet
///    created elsewhere while the caller was resolving its target is not
///    overwritten by a stale in-memory copy;
/// 3. the lookup of the target **by identifier** with `!deleted`, so a
///    concurrent insertion cannot shift the target;
/// 4. the tombstone itself: `deleted = true` and
///    `updated_at = updated_at.max(now).saturating_add(1)`, which makes the
///    tombstone timestamp strictly greater than the live version's — the
///    property that lets a deletion beat live content in the client merge;
/// 5. the durable save through the atomic write path;
/// 6. the `delete` audit entry — best effort, a failed audit write is logged
///    at debug level and does not fail the delete;
/// 7. the sync trigger, after the commit succeeded.
///
/// The primitive contains no picker-specific logic: no display index and no
/// in-memory mirror. The interactive path mirrors the tombstone into the
/// picker's in-memory copy itself (see [`apply_snippet_delete`]).
///
/// When the target cannot be tombstoned the library is left untouched: the
/// mutation closure fails, so `update_library` never reaches its save.
pub fn delete_snippet(
    library_path: &std::path::Path,
    target_id: &str,
    trigger: DeleteSyncTrigger<'_>,
) -> SnipResult<SnippetDeleteOutcome> {
    let mut probe: Option<DeleteProbe> = None;
    let mut deleted: Option<crate::library::Snippet> = None;

    let mutation = crate::library::update_library(library_path, |current| {
        let index = current
            .snippets
            .iter()
            .position(|s| s.id == target_id && !s.deleted);
        let Some(index) = index else {
            // The target is gone (deleted by somebody else, or never
            // existed). Record which of the two it was, then fail the
            // mutation so the library is not rewritten.
            let tombstoned = current
                .snippets
                .iter()
                .find(|s| s.id == target_id && s.deleted)
                .cloned();
            probe = Some(match tombstoned {
                Some(snippet) => DeleteProbe::Tombstoned(snippet),
                None => DeleteProbe::Missing,
            });
            return Err(SnipError::runtime_error(
                "Snippet not found",
                Some("The selected snippet is no longer available"),
            ));
        };
        let target = &mut current.snippets[index];
        target.deleted = true;
        let now = chrono::Utc::now().timestamp();
        target.updated_at = target.updated_at.max(now).saturating_add(1);
        deleted = Some(target.clone());
        Ok(())
    });

    match mutation {
        Ok(_) => {}
        Err(e) => {
            return match probe {
                Some(DeleteProbe::Tombstoned(snippet)) => {
                    Ok(SnippetDeleteOutcome::AlreadyDeleted(snippet))
                }
                Some(DeleteProbe::Missing) => Ok(SnippetDeleteOutcome::NotFound),
                // The gate, the lock, the load, or the save failed: report
                // the real error instead of a lookup verdict.
                None => Err(e),
            };
        }
    }

    let deleted = deleted.ok_or_else(|| {
        SnipError::runtime_error(
            "Snippet not found",
            Some("The selected snippet is no longer available"),
        )
    })?;

    // Audit is best effort: the tombstone is already durable.
    if let Err(e) = crate::logging::audit_log("delete", &deleted, None) {
        tracing::debug!("Audit log write failed: {}", e);
    }

    // Sync trigger: only after the mutation is durable.
    match trigger {
        DeleteSyncTrigger::ExplicitSync(rt) => {
            if let Err(e) = run_explicit_sync(rt) {
                tracing::warn!(error = %e, "post-delete explicit sync failed");
            }
        }
        DeleteSyncTrigger::AutoNotify => {
            crate::auto_sync::report_notification_result(crate::auto_sync::notify_mutation(
                crate::auto_sync::MutationKind::SnippetDelete,
                crate::auto_sync::MutationOrigin::User,
            ));
        }
    }

    Ok(SnippetDeleteOutcome::Deleted(deleted))
}

/// Apply a TUI delete selection: the shared primitive plus the picker's
/// in-memory mirror.
///
/// The mirror is picker-specific — it updates the copy the picker renders
/// from and never touches the file — so it stays in the selection loop's
/// helper rather than in [`delete_snippet`]. The persisted state comes from
/// the primitive alone, which is what makes the interactive and the
/// non-interactive delete produce identical files.
pub(crate) fn apply_snippet_delete(
    snippets: &mut crate::library::Snippets,
    library_path: &std::path::Path,
    original_idx: usize,
    trigger: DeleteSyncTrigger<'_>,
) -> SnipResult<crate::library::Snippet> {
    let target_id = snippets
        .snippets
        .get(original_idx)
        .map(|s| s.id.clone())
        .ok_or_else(|| {
            SnipError::runtime_error(
                "Snippet not found",
                Some("The selected snippet is no longer available"),
            )
        })?;
    let deleted = match delete_snippet(library_path, &target_id, trigger)? {
        SnippetDeleteOutcome::Deleted(snippet) => snippet,
        SnippetDeleteOutcome::AlreadyDeleted(_) | SnippetDeleteOutcome::NotFound => {
            return Err(SnipError::runtime_error(
                "Snippet not found",
                Some("The selected snippet is no longer available"),
            ));
        }
    };
    // Mirror the tombstone into the in-memory copy the picker renders from.
    mark_snippet_deleted(snippets, original_idx)?;
    Ok(deleted)
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

    /// A far-future `updated_at` so a tombstone's timestamp is
    /// deterministic: `updated_at.max(now).saturating_add(1)` keeps the
    /// fixture's value instead of the wall clock, so two runs against the
    /// same fixture produce byte-identical library files.
    const FUTURE_UPDATED_AT: i64 = 4_000_000_000;

    /// An isolated config dir with one primary library holding `snippets`.
    ///
    /// The gate, the local-data lock, and the audit log are all derived from
    /// the config directory, so the override keeps the test hermetic.
    struct DeleteFixture {
        _tmp: TempDir,
        _config_dir: crate::utils::config::ScopedConfigDir,
        config_dir: std::path::PathBuf,
        library_path: std::path::PathBuf,
    }

    impl DeleteFixture {
        fn new(snippets: Vec<crate::library::Snippet>) -> Self {
            let tmp = TempDir::new().expect("test helper: temp dir");
            let config_dir = tmp.path().join(".config").join("snp");
            let guard = crate::utils::config::ScopedConfigDir::new(config_dir.clone());
            let mut manager = crate::library::LibraryManager::new().expect("test helper: manager");
            let library_path = manager
                .create_library("mylib")
                .expect("test helper: create library");
            let collection = crate::library::Snippets {
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
                config_dir,
                library_path,
            }
        }

        fn snippet(id: &str, description: &str, command: &str) -> crate::library::Snippet {
            crate::library::Snippet {
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

    #[test]
    fn delete_snippet_tombstones_with_a_strictly_advancing_timestamp() {
        let fixture = DeleteFixture::new(vec![DeleteFixture::snippet(
            "id-1",
            "git push",
            "git push origin main",
        )]);
        let live = crate::library::load_library(&fixture.library_path)
            .expect("load fixture")
            .snippets[0]
            .updated_at;

        let outcome = delete_snippet(&fixture.library_path, "id-1", DeleteSyncTrigger::AutoNotify)
            .expect("delete succeeds");

        match outcome {
            SnippetDeleteOutcome::Deleted(snippet) => {
                // The identifier is deterministic and is not reassigned.
                assert_eq!(snippet.id, "id-1");
                assert!(snippet.deleted);
                // The tombstone timestamp is strictly greater than the live
                // version's, which is what lets a deletion beat live content
                // in the client merge.
                assert!(
                    snippet.updated_at > live,
                    "tombstone timestamp {} must exceed the live timestamp {live}",
                    snippet.updated_at
                );
                assert_eq!(snippet.updated_at, live + 1);
            }
            other => panic!("expected a tombstone, got {other:?}"),
        }

        // The persisted file carries the same tombstone.
        let persisted = crate::library::load_library(&fixture.library_path).expect("reload");
        let target = &persisted.snippets[0];
        assert_eq!(target.id, "id-1");
        assert!(target.deleted);
        assert_eq!(target.updated_at, live + 1);
    }

    #[test]
    fn delete_snippet_reports_an_already_deleted_target_distinctly() {
        let mut snippet = DeleteFixture::snippet("id-1", "git push", "git push");
        snippet.deleted = true;
        let fixture = DeleteFixture::new(vec![snippet]);
        let before = fixture.library_bytes();

        let outcome = delete_snippet(&fixture.library_path, "id-1", DeleteSyncTrigger::AutoNotify)
            .expect("delete reports a verdict");

        match outcome {
            SnippetDeleteOutcome::AlreadyDeleted(snippet) => {
                assert_eq!(snippet.id, "id-1");
            }
            other => panic!("expected an already-deleted verdict, got {other:?}"),
        }
        // The already-deleted target is not modified.
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn delete_snippet_reports_a_missing_target_distinctly() {
        let fixture =
            DeleteFixture::new(vec![DeleteFixture::snippet("id-1", "git push", "git push")]);
        let before = fixture.library_bytes();

        let outcome = delete_snippet(
            &fixture.library_path,
            "does-not-exist",
            DeleteSyncTrigger::AutoNotify,
        )
        .expect("delete reports a verdict");

        assert!(
            matches!(outcome, SnippetDeleteOutcome::NotFound),
            "expected a not-found verdict, got {outcome:?}"
        );
        assert_eq!(fixture.library_bytes(), before);
    }

    #[test]
    fn delete_snippet_writes_the_delete_audit_entry() {
        let fixture =
            DeleteFixture::new(vec![DeleteFixture::snippet("id-1", "git push", "git push")]);

        delete_snippet(&fixture.library_path, "id-1", DeleteSyncTrigger::AutoNotify)
            .expect("delete succeeds");

        let audit = std::fs::read_to_string(fixture.config_dir.join("audit.log"))
            .expect("audit log written");
        let entry = audit
            .lines()
            .find(|line| line.contains("|delete|"))
            .expect("audit log names the delete action");
        assert!(
            entry.contains("id-1"),
            "audit entry must name the target: {entry}"
        );
    }

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

    #[test]
    fn test_run_explicit_sync_compiles_with_correct_signature() {
        // Verify run_explicit_sync exists and has the expected signature
        // by passing a function pointer. The function itself is tested
        // through integration tests and run_exact unit tests.
        fn _assert_signature(_f: fn(&tokio::runtime::Runtime) -> crate::error::SnipResult<()>) {}
        _assert_signature(super::run_explicit_sync);
    }

    #[test]
    fn test_run_snippet_selection_requires_runtime_for_sync() {
        let result = run_snippet_selection(None, None, true, false, None, None, |_snippet, _| {
            Ok(crate::ProcessResult::Continue)
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("sync selection without a runtime should fail"),
        };
        assert!(error.to_string().contains("no runtime"));
    }

    #[test]
    fn test_read_only_selection_refuses_delete() {
        let result = run_snippet_selection_with_resolution(
            None,
            None,
            LibraryResolution::ReadOnly,
            false,
            true,
            None,
            None,
            |_snippet, _| Ok(crate::ProcessResult::Continue),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("a read-only selection must not allow deletion"),
        };
        assert!(error.to_string().contains("cannot delete"));
    }
}

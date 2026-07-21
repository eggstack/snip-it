use crate::commands::init_library_manager;
use crate::config::{AutoSyncFailureMode, SyncDirection, load_sync_settings, save_sync_settings};
use crate::error::{SnipError, SnipResult};
use crate::library::LibraryManager;
use crate::proto::Library;
use std::io::{self, Write};

fn server_library_filename(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

fn has_local_snippets(lib_path: &std::path::Path) -> bool {
    lib_path.exists()
        && crate::library::load_library(lib_path)
            .is_ok_and(|snippets| snippets.snippets.iter().any(|snippet| !snippet.deleted))
}

fn link_library_to_server(filename: &str, server_id: &str, mgr: &mut LibraryManager) -> bool {
    if let Err(e) = mgr.link_server_library(filename, server_id) {
        eprintln!("  Failed to link '{filename}': {e}");
        return false;
    }
    true
}

fn clear_library_for_server_pull(
    filename: &str,
    lib_path: &std::path::Path,
    server_id: &str,
    mgr: &mut LibraryManager,
) -> bool {
    if !link_library_to_server(filename, server_id, mgr) {
        return false;
    }

    let empty = crate::library::Snippets::default();
    if let Err(e) = crate::library::save_library(lib_path, &empty) {
        eprintln!("    Failed to clear original library: {e}");
        if let Err(unlink_err) = mgr.unlink_server_library(filename) {
            eprintln!("    Failed to roll back server link: {unlink_err}");
        }
        return false;
    }

    true
}

fn link_server_library(lib: &Library, mgr: &mut LibraryManager, print_linked: bool) -> bool {
    let filename = server_library_filename(&lib.name);
    let existing_lib_id = mgr
        .get_library_by_filename(&filename)
        .map(|l| l.library_id.clone());

    if let Some(existing_id) = &existing_lib_id {
        if !existing_id.is_empty() && existing_id != &lib.id {
            println!("  Library '{}' has different server ID, skipping", lib.name);
            return false;
        }

        let lib_path = mgr.get_libraries_dir().join(format!("{filename}.toml"));
        let local_has_content = has_local_snippets(&lib_path);

        if existing_id.is_empty() && local_has_content && lib.snippet_count > 0 {
            println!("\n  Local library '{filename}' has snippets. Server also has snippets.");
            match prompt_conflict(&filename).as_deref() {
                Some("overwrite") => {
                    println!("  Replacing local library with server version");
                    return clear_library_for_server_pull(&filename, &lib_path, &lib.id, mgr);
                }
                Some("rename") => {
                    let new_name = format!("{filename}_local");
                    println!("  Renaming to '{new_name}' and pulling from server");
                    if let Err(e) = mgr.create_library(&new_name) {
                        eprintln!("    Failed to create backup: {e}");
                        return false;
                    }
                    // Move local snippets to the backup library
                    let local_snippets =
                        crate::library::load_library(&lib_path).unwrap_or_default();
                    let backup_path = mgr.get_libraries_dir().join(format!("{new_name}.toml"));
                    if let Err(e) = crate::library::save_library(&backup_path, &local_snippets) {
                        eprintln!("    Failed to save backup: {e}");
                        return false;
                    }
                    // Link the original library to the server ID so server
                    // content syncs into it. The backup stays unlinked (created
                    // by create_library with empty library_id).
                    // Clear original library for server content
                    if !clear_library_for_server_pull(&filename, &lib_path, &lib.id, mgr) {
                        return false;
                    }
                    println!(
                        "    Created '{new_name}' with local content, original cleared for server content"
                    );
                    return true;
                }
                _ => {
                    println!("  Skipping, keeping local version");
                    return false;
                }
            }
        }

        if existing_id.is_empty() {
            if !link_library_to_server(&filename, &lib.id, mgr) {
                return false;
            }
            println!("  Linked '{}' to server library '{}'", filename, lib.id);
            return true;
        } else if print_linked {
            println!("  Library '{}' already linked, skipping", lib.name);
        }
        return false;
    }

    match mgr.add_server_library(&lib.name, &lib.id) {
        Ok(path) => {
            println!("  Created '{}' at {}", lib.name, path.display());
            true
        }
        Err(e) => {
            eprintln!("  Failed to create library '{}': {}", lib.name, e);
            false
        }
    }
}

/// Prompts the user to resolve a local/server library conflict.
///
/// Returns `"overwrite"`, `"rename"`, or `None` (skip) based on user input.
pub fn prompt_conflict(lib_name: &str) -> Option<String> {
    println!("\nConflict: Local library '{lib_name}' has different content than server");
    println!("  (s)kip - keep local version");
    println!("  (o)verwrite - replace with server version");
    println!("  (r)ename - rename local and pull from server");
    print!("Choice [s/o/r]: ");

    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        match input.trim().to_lowercase().as_str() {
            "o" => Some("overwrite".to_string()),
            "r" => Some("rename".to_string()),
            _ => None,
        }
    } else {
        None
    }
}

/// Options for the `sync` command.
pub struct SyncOptions {
    pub library: Option<String>,
    pub servers: bool,
    pub push_only: bool,
    pub pull_only: bool,
    pub dry_run: bool,
}

/// Runs the sync command with the given options.
///
/// Supports listing servers, push-only, pull-only, bidirectional, and dry-run modes.
pub fn run(options: SyncOptions, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let sync_settings = load_sync_settings().map_err(|e| {
        eprintln!("Failed to load sync settings: {e}");
        e
    })?;

    if !sync_settings.enabled {
        eprintln!("Sync is not enabled. Configure sync settings first.");
        return Ok(());
    }
    if sync_settings.api_key.is_empty() {
        eprintln!("Sync is enabled but no API key is configured. Run 'snp register --force'.");
        return Ok(());
    }

    // Acquire execution lock with bounded wait for foreground sync
    let state_dir = crate::auto_sync::notification::derive_state_dir();
    let _exec_lock = crate::auto_sync::execution_lock::wait_acquire(
        &state_dir,
        std::time::Duration::from_secs(30),
    )
    .map_err(|e| match e {
        crate::auto_sync::execution_lock::ExecutionLockError::Timeout { owner_pid, .. } => {
            SnipError::runtime_error(
                "sync already in progress",
                Some(&format!(
                    "owner pid={owner_pid}; wait for it to complete or kill the process"
                )),
            )
        }
        crate::auto_sync::execution_lock::ExecutionLockError::AlreadyHeld { pid, .. } => {
            SnipError::runtime_error(
                "sync already in progress",
                Some(&format!("held by pid={pid}")),
            )
        }
        other => SnipError::runtime_error("failed to acquire sync lock", Some(&other.to_string())),
    })?;

    if options.servers {
        let mut client = runtime
            .block_on(crate::sync::SyncClient::create(sync_settings.clone()))
            .map_err(|e| {
                SnipError::sync_failure(
                    crate::error::SyncFailureKind::ConnectFailed,
                    Some(&e.to_string()),
                )
            })?;

        match runtime.block_on(client.list_libraries()) {
            Ok(libs) => {
                println!("Server libraries:");
                for lib in libs {
                    println!("  {} ({})", lib.name, lib.id);
                }
            }
            Err(e) => eprintln!("Failed to list server libraries: {e}"),
        }
        return Ok(());
    }

    let mut client = runtime
        .block_on(crate::sync::SyncClient::create(sync_settings.clone()))
        .map_err(|e| {
            SnipError::sync_failure(
                crate::error::SyncFailureKind::ConnectFailed,
                Some(&e.to_string()),
            )
        })?;

    match runtime.block_on(client.list_libraries()) {
        Ok(libs) => {
            let mut mgr = init_library_manager().map_err(|e| {
                SnipError::sync_failure(
                    crate::error::SyncFailureKind::LibraryManagerInitFailed,
                    Some(&e.to_string()),
                )
            })?;

            if options.dry_run {
                println!("\n[DRY RUN] Would sync snippets:");
                let lib_path = match crate::commands::get_library_path(options.library)? {
                    Some(p) => p,
                    None => {
                        println!("  No library selected");
                        return Ok(());
                    }
                };
                let snippets = crate::library::load_library(&lib_path)?;
                let direction = if options.push_only {
                    "push to server"
                } else if options.pull_only {
                    "pull from server"
                } else {
                    "bidirectional"
                };
                println!("  Direction: {direction}");
                println!("  Snippets in library: {}", snippets.snippets.len());
                for s in &snippets.snippets {
                    if !s.deleted {
                        println!("  - {} ({})", s.description, &s.id[..8.min(s.id.len())]);
                    }
                }
                return Ok(());
            }

            for lib in libs {
                link_server_library(&lib, &mut mgr, true);
            }

            println!("\nSyncing snippets...");
            // Respect config direction when no CLI flags are provided
            // When both push and pull are effective (bidirectional), pass neither
            // so run_sync defaults to Bidirectional instead of Push-only.
            let effective_push = options.push_only
                || (!options.pull_only && sync_settings.sync_direction == SyncDirection::Push);
            let effective_pull = options.pull_only
                || (!options.push_only && sync_settings.sync_direction == SyncDirection::Pull);
            // Capture the observed pending generation BEFORE running sync so
            // a mutation arriving during sync is preserved (Workstream D5).
            let observed_generation = crate::auto_sync::observe_pending_generation();
            let sync_result = crate::sync_commands::run_sync(
                &sync_settings,
                options.library.as_deref(),
                effective_push,
                effective_pull,
                runtime,
            );

            // Record durable status for foreground sync (Workstream H).
            // Status write is best-effort; failure must not prevent sync result
            // from propagating to the caller.
            let observed_gen = observed_generation.unwrap_or(0);
            match &sync_result {
                Ok(()) => {
                    let _ = crate::auto_sync::status::record_success(
                        &state_dir,
                        observed_gen,
                        "foreground sync completed",
                    );
                }
                Err(e) => {
                    let failure_class = crate::auto_sync::policy::FailureClass::from_error(e);
                    let _ = crate::auto_sync::status::record_failure(
                        &state_dir,
                        observed_gen,
                        failure_class,
                        crate::auto_sync::executor::ExecutorExitCode::from_failure_class(
                            failure_class,
                        )
                        .to_exit_status(),
                        0,
                        0,
                        &e.to_string(),
                        crate::auto_sync::status::compute_config_fingerprint(&sync_settings),
                    );
                }
            }

            let sync_succeeded = sync_result.is_ok();
            sync_result?;

            // Explicit sync succeeded: clear pending auto-sync to prevent
            // duplicate delayed sync (Workstream D).
            crate::auto_sync::clear_pending_after_explicit_sync(
                observed_generation,
                sync_succeeded,
            );

            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to pull libraries: {e}");
            Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::ConnectFailed,
                Some(&e.to_string()),
            ))
        }
    }
}

/// Runs the `snp sync config` command to inspect or update auto-sync policy.
pub fn run_config(
    show: bool,
    auto_sync: Option<String>,
    debounce: Option<u64>,
    max_delay: Option<u64>,
    failure: Option<String>,
    timeout: Option<u64>,
) -> SnipResult<()> {
    let mut settings = load_sync_settings().map_err(|e| {
        eprintln!("Failed to load sync settings: {e}");
        e
    })?;

    let has_changes = auto_sync.is_some()
        || debounce.is_some()
        || max_delay.is_some()
        || failure.is_some()
        || timeout.is_some();

    if !show && !has_changes {
        eprintln!(
            "Usage: snp sync config --show | --auto-sync on|off | --debounce <secs> | --max-delay <secs> | --timeout <secs> | --failure ignore|warn|error"
        );
        return Ok(());
    }

    if show {
        println!("Auto-sync configuration:");
        println!(
            "  auto_sync:                {}",
            if settings.auto_sync { "on" } else { "off" }
        );
        println!(
            "  auto_sync_debounce_seconds: {}",
            settings.auto_sync_debounce_seconds
        );
        println!(
            "  auto_sync_max_delay_seconds: {:?}",
            settings.auto_sync_max_delay_seconds
        );
        println!(
            "  auto_sync_timeout_seconds:  {:?} (resolved: {}s)",
            settings.auto_sync_timeout_seconds,
            settings.auto_sync_timeout().as_secs()
        );
        println!("  auto_sync_failure:        {}", settings.auto_sync_failure);
        println!(
            "  sync_enabled:             {}",
            if settings.enabled { "on" } else { "off" }
        );
        if settings.auto_sync && !settings.enabled {
            println!("  warning: auto_sync is on but sync is not enabled");
        }
        if settings.auto_sync && settings.api_key.is_empty() {
            println!("  warning: auto_sync is on but no API key is configured");
        }
    }

    if let Some(ref value) = auto_sync {
        match value.to_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => {
                settings.auto_sync = true;
                eprintln!("Auto-sync enabled");
            }
            "off" | "false" | "0" | "no" => {
                settings.auto_sync = false;
                eprintln!("Auto-sync disabled");
            }
            other => {
                eprintln!("Invalid value '{other}': expected on/off");
                return Err(SnipError::runtime_error(
                    "invalid auto_sync value",
                    Some("expected on, off, true, false, 1, 0, yes, or no"),
                ));
            }
        }
    }

    if let Some(secs) = debounce {
        if secs > crate::config::AUTO_SYNC_DEBOUNCE_MAX {
            eprintln!(
                "Debounce value {} exceeds maximum {}; clamping to {}",
                secs,
                crate::config::AUTO_SYNC_DEBOUNCE_MAX,
                crate::config::AUTO_SYNC_DEBOUNCE_MAX
            );
        }
        settings.auto_sync_debounce_seconds = secs.clamp(
            crate::config::AUTO_SYNC_DEBOUNCE_MIN,
            crate::config::AUTO_SYNC_DEBOUNCE_MAX,
        );
        eprintln!(
            "Debounce set to {} seconds",
            settings.auto_sync_debounce_seconds
        );
    }

    if let Some(secs) = max_delay {
        if secs > crate::config::AUTO_SYNC_MAX_DELAY_MAX {
            eprintln!(
                "Max delay value {} exceeds maximum {}; clamping to {}",
                secs,
                crate::config::AUTO_SYNC_MAX_DELAY_MAX,
                crate::config::AUTO_SYNC_MAX_DELAY_MAX
            );
        }
        settings.auto_sync_max_delay_seconds = Some(secs.clamp(
            crate::config::AUTO_SYNC_MAX_DELAY_MIN,
            crate::config::AUTO_SYNC_MAX_DELAY_MAX,
        ));
        eprintln!(
            "Max delay set to {} seconds",
            settings.auto_sync_max_delay().as_secs()
        );
    }

    if let Some(ref mode_str) = failure {
        let mode: AutoSyncFailureMode = mode_str.parse().map_err(|e: String| {
            SnipError::runtime_error("invalid auto_sync_failure value", Some(&e))
        })?;
        settings.auto_sync_failure = mode.clone();
        eprintln!("Failure mode set to {mode}");
    }

    if let Some(secs) = timeout {
        if !(crate::config::MIN_SYNC_TIMEOUT_SECS..=crate::config::MAX_SYNC_TIMEOUT_SECS)
            .contains(&secs)
        {
            eprintln!(
                "Timeout value {} outside valid range {}-{}; clamping",
                secs,
                crate::config::MIN_SYNC_TIMEOUT_SECS,
                crate::config::MAX_SYNC_TIMEOUT_SECS
            );
        }
        settings.auto_sync_timeout_seconds = Some(secs.clamp(
            crate::config::MIN_SYNC_TIMEOUT_SECS,
            crate::config::MAX_SYNC_TIMEOUT_SECS,
        ));
        eprintln!(
            "Timeout set to {} seconds",
            settings.auto_sync_timeout().as_secs()
        );
    }

    if has_changes {
        save_sync_settings(&settings).map_err(|e| {
            eprintln!("Failed to save sync settings: {e}");
            e
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_filename_slug() {
        // link_server_library derives filenames by lowercasing and replacing spaces.
        // The function takes a &mut LibraryManager, but the slug transformation
        // is the testable contract — verify the expected mapping directly.
        let cases = vec![
            ("My Library", "my-library"),
            ("UPPERCASE", "uppercase"),
            ("multi word name", "multi-word-name"),
        ];
        for (input, expected) in cases {
            assert_eq!(server_library_filename(input), expected);
        }
    }

    #[test]
    fn test_has_local_snippets_ignores_empty_and_deleted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("library.toml");

        assert!(!has_local_snippets(&path));

        crate::library::save_library(&path, &crate::library::Snippets::default()).unwrap();
        assert!(!has_local_snippets(&path));

        let deleted = crate::library::Snippets {
            snippets: vec![crate::library::Snippet {
                id: "deleted".to_string(),
                description: "deleted".to_string(),
                command: "echo deleted".to_string(),
                deleted: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        crate::library::save_library(&path, &deleted).unwrap();
        assert!(!has_local_snippets(&path));
    }

    #[test]
    fn test_has_local_snippets_detects_active_snippet() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("library.toml");
        let snippets = crate::library::Snippets {
            snippets: vec![crate::library::Snippet {
                id: "active".to_string(),
                description: "active".to_string(),
                command: "echo active".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        crate::library::save_library(&path, &snippets).unwrap();

        assert!(has_local_snippets(&path));
    }

    #[test]
    fn test_sync_options_defaults() {
        let opts = SyncOptions {
            library: None,
            servers: false,
            push_only: false,
            pull_only: false,
            dry_run: false,
        };
        assert!(!opts.servers);
        assert!(!opts.push_only);
        assert!(!opts.pull_only);
    }
}

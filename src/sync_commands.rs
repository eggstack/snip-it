//! Sync orchestration and merge logic.
//!
//! Coordinates the bidirectional sync flow between local snippet libraries
//! and the remote server. Handles merge conflict resolution using
//! last-write-wins based on `updated_at` timestamps.

use crate::config::{SyncDirection, SyncSettings};
use crate::error::{SnipError, SnipResult};
use crate::library::{self, Snippet, Snippets};
use crate::proto::Snippet as ProtoSnippet;
use crate::sync;
use std::fs;

impl From<&Snippet> for ProtoSnippet {
    fn from(s: &Snippet) -> Self {
        ProtoSnippet {
            id: s.id.clone(),
            description: s.description.clone(),
            command: s.command.clone(),
            tags: s.tags.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            device_id: s.device_id.clone(),
            deleted: s.deleted,
            encrypted: false,
        }
    }
}

fn get_library_sync_info(mgr: &library::LibraryManager, lib_name: &str) -> (String, i64) {
    match mgr.get_library_by_filename(lib_name) {
        Some(l) => {
            let id = l.library_id.clone();
            if !id.is_empty() && l.server_id.as_deref() != Some(id.as_str()) {
                tracing::warn!(
                    "Library '{}' has library_id '{}' but server_id '{:?}' — possible stale config",
                    lib_name,
                    id,
                    l.server_id
                );
            }
            (id, l.last_sync.unwrap_or(0))
        }
        None => (String::new(), 0),
    }
}

fn ensure_sync_configured(settings: &SyncSettings) -> bool {
    if !settings.enabled {
        tracing::warn!("Sync is not enabled. Configure sync settings first.");
        return false;
    }
    if settings.api_key.is_empty() {
        tracing::warn!("Sync is enabled but no API key configured");
        return false;
    }
    true
}

fn create_sync_client(
    runtime: &tokio::runtime::Runtime,
    settings: &SyncSettings,
) -> Option<sync::SyncClient> {
    runtime
        .block_on(sync::SyncClient::create(settings.clone()))
        .ok()
}

fn check_server_health(
    runtime: &tokio::runtime::Runtime,
    client: &mut sync::SyncClient,
    server_url: &str,
) -> bool {
    match runtime.block_on(client.health_check()) {
        Ok(true) => true,
        _ => {
            tracing::error!("Server is not reachable at {}", server_url);
            false
        }
    }
}

/// Synchronizes premade libraries from the server to the local filesystem.
///
/// Downloads any premade libraries that don't already exist locally.
/// Returns an error if the sync client cannot be created or if any downloads fail.
pub fn run_premade_sync(
    sync_settings: &SyncSettings,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    if !sync_settings.enabled || sync_settings.api_key.is_empty() {
        return Ok(());
    }

    let mut client = match runtime.block_on(sync::SyncClient::create(sync_settings.clone())) {
        Ok(c) => c,
        Err(e) => {
            return Err(SnipError::runtime_error(
                "Failed to create sync client",
                Some(&e.to_string()),
            ));
        }
    };

    if let Ok(libs) = runtime.block_on(client.list_premade_libraries()) {
        if libs.is_empty() {
            return Ok(());
        }

        let mgr = match library::LibraryManager::new() {
            Ok(m) => m,
            Err(e) => {
                return Err(SnipError::runtime_error(
                    "Failed to initialize library manager",
                    Some(&e.to_string()),
                ));
            }
        };

        let mut premade_results: Vec<(String, bool, String)> = Vec::new();

        for lib in libs {
            if mgr.premade_exists(&lib.filename) {
                continue;
            }

            match runtime.block_on(client.get_premade_library(&lib.filename)) {
                Ok(content) => match mgr.save_premade_library(&lib.filename, &content) {
                    Ok(path) => {
                        premade_results.push((lib.filename, true, path.display().to_string()));
                    }
                    Err(e) => {
                        premade_results.push((lib.filename, false, e.to_string()));
                    }
                },
                Err(e) => {
                    premade_results.push((lib.filename, false, e.to_string()));
                }
            }
        }

        if !premade_results.is_empty() {
            println!("\nPremade libraries:");
            for (name, success, msg) in &premade_results {
                if *success {
                    println!("  + {name} → {msg}");
                } else {
                    println!("  ✗ {name}: {msg}");
                }
            }

            if premade_results.iter().any(|(_, success, _)| !success) {
                return Err(SnipError::runtime_error(
                    "Some premade libraries failed to sync",
                    None,
                ));
            }
        }
    }

    Ok(())
}

struct SyncStatus {
    pushed: u32,
    pulled: u32,
    conflicts: u32,
    failed: u32,
}

impl SyncStatus {
    fn new() -> Self {
        Self {
            pushed: 0,
            pulled: 0,
            conflicts: 0,
            failed: 0,
        }
    }
}

fn merge_and_save(
    lib_path: &std::path::Path,
    lib_name: &str,
    snippets: &Snippets,
    server_snippets: &[ProtoSnippet],
    device_id: &str,
) -> SnipResult<(Snippets, Option<String>, Vec<String>)> {
    let conflicting_ids = sync::detect_device_conflict(server_snippets, device_id);
    if !conflicting_ids.is_empty() {
        tracing::warn!(
            library = %lib_name,
            count = conflicting_ids.len(),
            "Device conflicts detected during merge"
        );
    }

    let merged = merge_snippets(snippets, server_snippets);

    let backup_path = match library::backup_library(lib_path) {
        Ok(Some(p)) => Some(p.display().to_string()),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("Backup failed before merge save: {}", e);
            None
        }
    };

    if let Err(e) = library::save_library(lib_path, &merged) {
        if let Some(ref backup) = backup_path {
            if let Err(restore_err) = fs::copy(backup, lib_path) {
                tracing::error!(
                    "Failed to restore backup after merge save failure: {}",
                    restore_err
                );
            } else {
                tracing::info!("Restored library from backup after merge save failure");
            }
        }
        return Err(SnipError::runtime_error(
            "Failed to save merged library",
            Some(&e.to_string()),
        ));
    }

    Ok((merged, backup_path, conflicting_ids))
}

/// Performs a full sync operation across one or more libraries.
///
/// Supports push-only, pull-only, and bidirectional modes. Creates server-side
/// libraries for any unlinked local libraries, then merges snippets using
/// last-write-wins conflict resolution.
pub fn run_sync(
    sync_settings: &SyncSettings,
    library_name: Option<&str>,
    _non_interactive: bool,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let direction = if push_only {
        SyncDirection::Push
    } else if pull_only {
        SyncDirection::Pull
    } else {
        SyncDirection::Bidirectional
    };

    if direction == SyncDirection::Push {
        tracing::warn!(
            "Push-only mode: local changes will be uploaded but remote changes from other devices \
             will NOT be downloaded. Use bidirectional sync for multi-device support."
        );
    }

    if !ensure_sync_configured(sync_settings) {
        return Err(SnipError::runtime_error("Sync not configured", None));
    }

    let mut client = match create_sync_client(runtime, sync_settings) {
        Some(c) => c,
        None => {
            return Err(SnipError::runtime_error(
                "Failed to create sync client",
                None,
            ));
        }
    };

    if !check_server_health(runtime, &mut client, &sync_settings.server_url) {
        return Err(SnipError::runtime_error("Server health check failed", None));
    }

    let mut mgr = match library::LibraryManager::new() {
        Ok(m) => m,
        Err(e) => {
            return Err(SnipError::runtime_error(
                "Failed to initialize library manager",
                Some(&e.to_string()),
            ));
        }
    };

    if let Err(e) = mgr.ensure_library_mode() {
        return Err(SnipError::runtime_error(
            "Failed to initialize library mode",
            Some(&e.to_string()),
        ));
    }

    let libraries_to_sync: Vec<_> = if let Some(name) = library_name {
        vec![name.to_string()]
    } else {
        match std::fs::read_dir(mgr.get_libraries_dir()) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect(),
            Err(e) => {
                tracing::error!(
                    directory = %mgr.get_libraries_dir().display(),
                    error = %e,
                    "Failed to read libraries directory"
                );
                return Err(SnipError::runtime_error(
                    "Failed to read libraries directory",
                    Some(&e.to_string()),
                ));
            }
        }
    };

    if libraries_to_sync.is_empty() {
        tracing::warn!("No libraries to sync");
        return Err(SnipError::runtime_error("No libraries to sync", None));
    }

    for lib_name in &libraries_to_sync {
        let lib_path = mgr.get_libraries_dir().join(format!("{lib_name}.toml"));

        if !lib_path.exists() {
            tracing::warn!(library = %lib_name, "Library file not found, skipping");
            continue;
        }

        let (library_id, _last_sync) = get_library_sync_info(&mgr, lib_name);

        if library_id.is_empty() {
            tracing::info!(library = %lib_name, "Creating library on server");
            let normalized_name = lib_name.to_lowercase().replace(' ', "-");

            match runtime.block_on(client.create_library(&normalized_name)) {
                Ok(server_lib) => {
                    let new_id = server_lib.id.clone();

                    if mgr.get_library_by_filename(lib_name).is_none()
                        && let Err(e) = mgr.add_existing_library(lib_name)
                    {
                        tracing::warn!(library = %lib_name, error = %e, "Failed to add library to config");
                    }

                    if let Err(e) = mgr.update_library_id(lib_name, &new_id) {
                        tracing::warn!(library = %lib_name, error = %e, "Failed to link library in config");
                    }

                    tracing::info!(
                        library = %lib_name,
                        server_id = %new_id,
                        "Created and linked library to server"
                    );
                }
                Err(e) => {
                    tracing::error!(library = %lib_name, error = %e, "Failed to create library on server");
                    continue;
                }
            }
        }
    }

    let total = libraries_to_sync.len();
    let mut completed = 0;
    let mut status = SyncStatus::new();
    let mut results: Vec<(String, bool, String)> = Vec::new();

    for lib_name in &libraries_to_sync {
        print!("\r[{}/{}] Syncing {}...", completed + 1, total, lib_name);
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let lib_path = mgr.get_libraries_dir().join(format!("{lib_name}.toml"));

        if !lib_path.exists() {
            tracing::warn!(library = %lib_name, "Library file not found, skipping sync");
            continue;
        }

        let (library_id, _last_sync) = get_library_sync_info(&mgr, lib_name);

        if library_id.is_empty() {
            tracing::warn!(library = %lib_name, "Library not linked to server, skipping");
            continue;
        }

        let mut snippets = match library::load_library(&lib_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(library = %lib_name, error = %e, "Failed to load library");
                continue;
            }
        };

        let mut snippets_needing_ids: Vec<usize> = Vec::new();
        let now = chrono::Utc::now().timestamp();

        for (idx, s) in snippets.snippets.iter_mut().enumerate() {
            if s.id.is_empty() {
                s.id = uuid::Uuid::new_v4().to_string();
                s.created_at = now;
                s.updated_at = now;
                snippets_needing_ids.push(idx);
            }
        }

        if !snippets_needing_ids.is_empty()
            && let Err(e) = library::save_library(&lib_path, &snippets)
        {
            tracing::warn!(library = %lib_name, error = %e, "Failed to save generated IDs");
        }

        if direction == SyncDirection::Push || direction == SyncDirection::Bidirectional {
            let local_snippets: Vec<ProtoSnippet> = snippets
                .snippets
                .iter()
                .filter(|s| s.updated_at >= _last_sync || s.created_at >= _last_sync)
                .map(ProtoSnippet::from)
                .collect();

            if local_snippets.is_empty() && direction == SyncDirection::Push {
                tracing::info!(library = %lib_name, "No local changes to push, skipping");
                continue;
            }

            let result =
                runtime.block_on(client.sync_encrypted(local_snippets, _last_sync, &library_id));

            match result {
                Ok(response) => {
                    if response.success {
                        let new_timestamp = response.server_timestamp;

                        // Don't advance last_sync when encryption failures occurred,
                        // so failed snippets are retried on next sync.
                        let has_failures = response.skipped_count > 0;

                        if direction == SyncDirection::Push {
                            if !has_failures {
                                if let Err(e) = mgr.update_last_sync(lib_name, new_timestamp) {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update sync timestamp");
                                }
                                completed += 1;
                                status.pushed += 1;
                            } else {
                                status.conflicts += 1;
                            }
                            continue;
                        }

                        let server_snippets = response.snippets;

                        match merge_and_save(
                            &lib_path,
                            lib_name,
                            &snippets,
                            &server_snippets,
                            &sync_settings.device_id,
                        ) {
                            Ok((_merged, _backup, conflicts)) => {
                                if !has_failures
                                    && let Err(e) = mgr.update_last_sync(lib_name, new_timestamp)
                                {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update sync timestamp");
                                }

                                status.pulled += server_snippets.len() as u32;
                                if has_failures {
                                    status.conflicts += 1;
                                }

                                if has_failures {
                                    results.push((
                                        lib_name.clone(),
                                        true,
                                        format!(
                                            "{} snippets skipped (will retry)",
                                            response.skipped_count
                                        ),
                                    ));
                                } else if !conflicts.is_empty() {
                                    results.push((
                                        lib_name.clone(),
                                        true,
                                        format!(
                                            "{} snippets overwritten by another device",
                                            conflicts.len()
                                        ),
                                    ));
                                } else {
                                    results.push((lib_name.clone(), true, String::new()));
                                }
                            }
                            Err(e) => {
                                status.failed += 1;
                                results.push((lib_name.clone(), false, e.to_string()));
                                continue;
                            }
                        }
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, response.message));
                    }
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("Library not found") {
                        // Server library was deleted — re-create it
                        tracing::info!(library = %lib_name, "Server library deleted, re-creating on server");
                        let normalized_name = lib_name.to_lowercase().replace(' ', "-");
                        match runtime.block_on(client.create_library(&normalized_name)) {
                            Ok(server_lib) => {
                                if let Err(e) = mgr.update_library_id(lib_name, &server_lib.id) {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update library ID");
                                }
                                if let Err(e) = mgr.update_last_sync(lib_name, 0) {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to reset sync timestamp");
                                }
                                tracing::info!(library = %lib_name, server_id = %server_lib.id, "Re-created and relinked library");
                                // Retry sync with new library ID — push local snippets to fresh server library
                                let local_snippets_for_retry: Vec<ProtoSnippet> = snippets
                                    .snippets
                                    .iter()
                                    .map(ProtoSnippet::from)
                                    .collect();
                                let retry_result = runtime.block_on(
                                    client.sync_encrypted(local_snippets_for_retry, 0, &server_lib.id)
                                );
                                match retry_result {
                                    Ok(retry_response) if retry_response.success => {
                                        let server_snippets = retry_response.snippets;
                                        match merge_and_save(
                                            &lib_path,
                                            lib_name,
                                            &snippets,
                                            &server_snippets,
                                            &sync_settings.device_id,
                                        ) {
                                            Ok((_merged, _backup, _conflicts)) => {
                                                if let Err(e) = mgr.update_last_sync(lib_name, retry_response.server_timestamp) {
                                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update sync timestamp after re-creation");
                                                }
                                                status.pulled += server_snippets.len() as u32;
                                                results.push((lib_name.clone(), true, "Re-linked and synced".to_string()));
                                            }
                                            Err(e) => {
                                                status.failed += 1;
                                                results.push((lib_name.clone(), false, e.to_string()));
                                            }
                                        }
                                    }
                                    Ok(retry_response) => {
                                        status.failed += 1;
                                        results.push((lib_name.clone(), false, retry_response.message));
                                    }
                                    Err(e) => {
                                        status.failed += 1;
                                        results.push((lib_name.clone(), false, e.to_string()));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(library = %lib_name, error = %e, "Failed to re-create library on server");
                                status.failed += 1;
                                results.push((lib_name.clone(), false, format!("Library deleted and re-creation failed: {e}")));
                            }
                        }
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, err_msg));
                    }
                }
            }
        }

        if direction == SyncDirection::Pull && !library_id.is_empty() {
            let result = runtime.block_on(client.sync_encrypted(vec![], _last_sync, &library_id));

            match result {
                Ok(response) => {
                    if response.success {
                        let new_timestamp = response.server_timestamp;
                        let server_snippets = response.snippets;

                        match merge_and_save(
                            &lib_path,
                            lib_name,
                            &snippets,
                            &server_snippets,
                            &sync_settings.device_id,
                        ) {
                            Ok((_merged, _backup, conflicts)) => {
                                let has_failures = response.skipped_count > 0;
                                if !has_failures
                                    && let Err(e) = mgr.update_last_sync(lib_name, new_timestamp)
                                {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update sync timestamp");
                                }
                                status.pulled += server_snippets.len() as u32;
                                if !conflicts.is_empty() {
                                    results.push((
                                        lib_name.clone(),
                                        true,
                                        format!("{} snippets overwritten by another device", conflicts.len()),
                                    ));
                                } else {
                                    results.push((lib_name.clone(), true, String::new()));
                                }
                            }
                            Err(e) => {
                                status.failed += 1;
                                results.push((lib_name.clone(), false, e.to_string()));
                            }
                        }
                    }
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if err_msg.contains("Library not found") {
                        tracing::info!(library = %lib_name, "Server library deleted, re-creating on server");
                        let normalized_name = lib_name.to_lowercase().replace(' ', "-");
                        match runtime.block_on(client.create_library(&normalized_name)) {
                            Ok(server_lib) => {
                                if let Err(e) = mgr.update_library_id(lib_name, &server_lib.id) {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update library ID");
                                }
                                if let Err(e) = mgr.update_last_sync(lib_name, 0) {
                                    tracing::warn!(library = %lib_name, error = %e, "Failed to reset sync timestamp");
                                }
                                tracing::info!(library = %lib_name, server_id = %server_lib.id, "Re-created and relinked library");
                                let local_snippets_for_retry: Vec<ProtoSnippet> = snippets
                                    .snippets
                                    .iter()
                                    .map(ProtoSnippet::from)
                                    .collect();
                                let retry_result = runtime.block_on(
                                    client.sync_encrypted(local_snippets_for_retry, 0, &server_lib.id)
                                );
                                match retry_result {
                                    Ok(retry_response) if retry_response.success => {
                                        let server_snippets = retry_response.snippets;
                                        match merge_and_save(
                                            &lib_path,
                                            lib_name,
                                            &snippets,
                                            &server_snippets,
                                            &sync_settings.device_id,
                                        ) {
                                            Ok((_merged, _backup, _conflicts)) => {
                                                if let Err(e) = mgr.update_last_sync(lib_name, retry_response.server_timestamp) {
                                                    tracing::warn!(library = %lib_name, error = %e, "Failed to update sync timestamp after re-creation");
                                                }
                                                status.pulled += server_snippets.len() as u32;
                                                results.push((lib_name.clone(), true, "Re-linked and synced".to_string()));
                                            }
                                            Err(e) => {
                                                status.failed += 1;
                                                results.push((lib_name.clone(), false, e.to_string()));
                                            }
                                        }
                                    }
                                    Ok(retry_response) => {
                                        status.failed += 1;
                                        results.push((lib_name.clone(), false, retry_response.message));
                                    }
                                    Err(e) => {
                                        status.failed += 1;
                                        results.push((lib_name.clone(), false, e.to_string()));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(library = %lib_name, error = %e, "Failed to re-create library on server");
                                status.failed += 1;
                                results.push((lib_name.clone(), false, format!("Library deleted and re-creation failed: {e}")));
                            }
                        }
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, err_msg));
                    }
                }
            }
        }

        completed += 1;
    }

    for (name, _success, msg) in &results {
        if !msg.is_empty() {
            tracing::info!(library = %name, details = %msg, "Sync result");
        }
    }

    tracing::info!(
        pushed = status.pushed,
        pulled = status.pulled,
        conflicts = status.conflicts,
        failed = status.failed,
        "Sync complete"
    );

    if status.failed > 0 {
        Err(SnipError::runtime_error(
            "Some libraries failed to sync",
            None,
        ))
    } else {
        Ok(())
    }
}

fn merge_snippets(local: &Snippets, server_snippets: &[ProtoSnippet]) -> Snippets {
    let local_by_id: std::collections::HashMap<_, _> =
        local.snippets.iter().map(|s| (s.id.clone(), s)).collect();

    let mut merged_snippets: Vec<Snippet> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for server_snip in server_snippets {
        seen_ids.insert(server_snip.id.clone());

        if server_snip.deleted {
            // Server deleted this snippet. If a local copy exists, mark it as
            // deleted (preserving the data) rather than silently removing it.
            if let Some(local_snip) = local_by_id.get(&server_snip.id)
                && !local_snip.deleted
            {
                merged_snippets.push(Snippet {
                    id: local_snip.id.clone(),
                    description: local_snip.description.clone(),
                    command: local_snip.command.clone(),
                    output: local_snip.output.clone(),
                    tags: local_snip.tags.clone(),
                    folders: local_snip.folders.clone(),
                    favorite: local_snip.favorite,
                    created_at: local_snip.created_at,
                    updated_at: server_snip.updated_at,
                    device_id: local_snip.device_id.clone(),
                    deleted: true,
                });
            }
            // If both server and local agree deleted, skip entirely
            continue;
        }

        if let Some(local_snip) = local_by_id.get(&server_snip.id) {
            if server_snip.updated_at >= local_snip.updated_at {
                if server_snip.updated_at != local_snip.updated_at {
                    tracing::warn!(
                        snippet_id = %server_snip.id,
                        old_updated_at = local_snip.updated_at,
                        new_updated_at = server_snip.updated_at,
                        "Snippet ID collision: server version overwrites local"
                    );
                }
                merged_snippets.push(Snippet {
                    id: server_snip.id.clone(),
                    description: server_snip.description.clone(),
                    command: server_snip.command.clone(),
                    output: local_snip.output.clone(),
                    tags: server_snip.tags.clone(),
                    folders: local_snip.folders.clone(),
                    favorite: local_snip.favorite,
                    created_at: local_snip.created_at.min(server_snip.created_at),
                    updated_at: server_snip.updated_at,
                    device_id: local_snip.device_id.clone(),
                    deleted: false,
                });
            } else {
                merged_snippets.push((*local_snip).clone());
            }
        } else {
            merged_snippets.push(Snippet {
                id: server_snip.id.clone(),
                description: server_snip.description.clone(),
                command: server_snip.command.clone(),
                output: String::new(),
                tags: server_snip.tags.clone(),
                folders: Vec::new(),
                favorite: false,
                created_at: server_snip.created_at,
                updated_at: server_snip.updated_at,
                device_id: server_snip.device_id.clone(),
                deleted: false,
            });
        }
    }

    for local_snip in &local.snippets {
        if !seen_ids.contains(&local_snip.id) && !local_snip.deleted {
            merged_snippets.push(local_snip.clone());
        }
    }

    merged_snippets.sort_by_key(|b| std::cmp::Reverse(b.updated_at));

    Snippets {
        snippets: merged_snippets,
        folders: local.folders.clone(),
    }
}

/// Runs a sync with the default settings (bidirectional, all libraries).
pub fn run_default_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let settings = crate::config::load_sync_settings().unwrap_or_default();
    run_sync(&settings, None, false, false, false, runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Snippet, Snippets};
    use crate::proto::Snippet as ProtoSnippet;

    fn make_local_snippet(id: &str, desc: &str, cmd: &str, updated_at: i64) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            tags: vec!["local".to_string()],
            folders: vec!["work".to_string()],
            output: "cached".to_string(),
            favorite: true,
            created_at: 100,
            updated_at,
            device_id: "device-1".to_string(),
            deleted: false,
        }
    }

    fn make_server_snippet(id: &str, desc: &str, cmd: &str, updated_at: i64) -> ProtoSnippet {
        ProtoSnippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            tags: vec!["server".to_string()],
            created_at: 100,
            updated_at,
            device_id: "device-2".to_string(),
            deleted: false,
            encrypted: false,
        }
    }

    #[test]
    fn test_server_wins_with_newer_timestamp() {
        let local = Snippets {
            snippets: vec![make_local_snippet("1", "local desc", "local cmd", 100)],
            folders: vec![],
        };
        let server = vec![make_server_snippet("1", "server desc", "server cmd", 200)];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 1);
        assert_eq!(merged.snippets[0].description, "server desc");
        assert_eq!(merged.snippets[0].command, "server cmd");
        assert_eq!(merged.snippets[0].updated_at, 200);
        // Local-only fields preserved
        assert_eq!(merged.snippets[0].output, "cached");
        assert_eq!(merged.snippets[0].folders, vec!["work"]);
        assert!(merged.snippets[0].favorite);
    }

    #[test]
    fn test_local_wins_with_newer_timestamp() {
        let local = Snippets {
            snippets: vec![make_local_snippet("1", "local desc", "local cmd", 300)],
            folders: vec![],
        };
        let server = vec![make_server_snippet("1", "server desc", "server cmd", 200)];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 1);
        assert_eq!(merged.snippets[0].description, "local desc");
        assert_eq!(merged.snippets[0].command, "local cmd");
    }

    #[test]
    fn test_new_server_snippet_added() {
        let local = Snippets {
            snippets: vec![make_local_snippet("1", "local", "echo 1", 100)],
            folders: vec![],
        };
        let server = vec![
            make_server_snippet("1", "local", "echo 1", 100),
            make_server_snippet("2", "new server", "echo 2", 150),
        ];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 2);
        let ids: Vec<&str> = merged.snippets.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"2"));
    }

    #[test]
    fn test_deleted_server_snippet_excluded() {
        let local = Snippets {
            snippets: vec![make_local_snippet("1", "local", "echo 1", 100)],
            folders: vec![],
        };
        let server = vec![ProtoSnippet {
            id: "1".to_string(),
            description: "deleted".to_string(),
            command: "echo deleted".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200,
            device_id: "d".to_string(),
            deleted: true,
            encrypted: false,
        }];

        let merged = merge_snippets(&local, &server);
        // Server-deleted snippet with existing local copy: local marked deleted, data preserved
        assert_eq!(merged.snippets.len(), 1);
        assert!(merged.snippets[0].deleted);
        assert_eq!(merged.snippets[0].description, "local");
        assert_eq!(merged.snippets[0].command, "echo 1");
    }

    #[test]
    fn test_server_delete_local_already_deleted_excluded() {
        let local = Snippets {
            snippets: vec![Snippet {
                id: "1".to_string(),
                description: "deleted locally".to_string(),
                command: "echo 1".to_string(),
                tags: vec![],
                folders: vec![],
                output: String::new(),
                favorite: false,
                created_at: 100,
                updated_at: 100,
                device_id: "d".to_string(),
                deleted: true,
            }],
            folders: vec![],
        };
        let server = vec![ProtoSnippet {
            id: "1".to_string(),
            description: "deleted".to_string(),
            command: "echo deleted".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200,
            device_id: "d".to_string(),
            deleted: true,
            encrypted: false,
        }];

        let merged = merge_snippets(&local, &server);
        // Both agree deleted: excluded entirely
        assert_eq!(merged.snippets.len(), 0);
    }

    #[test]
    fn test_local_only_snippet_preserved() {
        let local = Snippets {
            snippets: vec![
                make_local_snippet("1", "local 1", "echo 1", 100),
                make_local_snippet("2", "local 2", "echo 2", 100),
            ],
            folders: vec![],
        };
        let server = vec![make_server_snippet("1", "server 1", "echo 1", 100)];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 2);
        assert!(merged.snippets.iter().any(|s| s.id == "2"));
    }

    #[test]
    fn test_local_deleted_snippet_not_preserved() {
        let local = Snippets {
            snippets: vec![Snippet {
                id: "1".to_string(),
                description: "deleted locally".to_string(),
                command: "echo 1".to_string(),
                tags: vec![],
                folders: vec![],
                output: String::new(),
                favorite: false,
                created_at: 100,
                updated_at: 100,
                device_id: "d".to_string(),
                deleted: true,
            }],
            folders: vec![],
        };
        let server = vec![];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 0);
    }

    #[test]
    fn test_merge_preserves_folders() {
        let local = Snippets {
            snippets: vec![make_local_snippet("1", "local", "echo 1", 100)],
            folders: vec!["work".to_string(), "personal".to_string()],
        };
        let server = vec![];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.folders, vec!["work", "personal"]);
    }

    #[test]
    fn test_merge_sorted_by_updated_at_descending() {
        let local = Snippets {
            snippets: vec![
                make_local_snippet("1", "old", "echo 1", 100),
                make_local_snippet("2", "mid", "echo 2", 200),
            ],
            folders: vec![],
        };
        let server = vec![make_server_snippet("3", "new", "echo 3", 300)];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 3);
        assert_eq!(merged.snippets[0].updated_at, 300);
        assert_eq!(merged.snippets[1].updated_at, 200);
        assert_eq!(merged.snippets[2].updated_at, 100);
    }
}

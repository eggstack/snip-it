//! **Layer: Sync-Client**
//!
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
use crate::utils::atomic::{AtomicWriteOptions, Durability, atomic_replace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, IsTerminal};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SyncRecoveryMarker {
    schema: u32,
    local_library_name: String,
    #[serde(default)]
    local_library_id: String,
    #[serde(default)]
    server_library_id: Option<String>,
    created_at_unix_ms: i64,
    phase: RecoveryPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum RecoveryPhase {
    Creating,
    RemoteCreated,
    Linked,
}

struct KeyCacheGuard;

impl Drop for KeyCacheGuard {
    fn drop(&mut self) {
        crate::encryption::clear_key_cache();
    }
}

fn recovery_marker_path(libraries_dir: &Path, library_name: &str) -> std::path::PathBuf {
    libraries_dir.join(format!("{library_name}.sync_recovery"))
}

fn write_recovery_marker(path: &Path, marker: &SyncRecoveryMarker) -> SnipResult<()> {
    let bytes = toml::to_string_pretty(marker).map_err(|e| {
        SnipError::runtime_error("serialize sync recovery marker", Some(&e.to_string()))
    })?;
    atomic_replace(
        path,
        bytes.as_bytes(),
        &AtomicWriteOptions::for_durability(Durability::DurableUserData),
    )?;
    Ok(())
}

fn read_recovery_marker(path: &Path) -> SnipResult<SyncRecoveryMarker> {
    let content = fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read sync recovery marker", path, e))?;
    toml::from_str(&content)
        .map_err(|e| SnipError::runtime_error("invalid sync recovery marker", Some(&e.to_string())))
}

fn normalized_library_name(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

/// Handles "Library not found" recovery by re-creating the server library
/// and retrying the sync operation.
fn handle_library_not_found(
    lib_name: &str,
    lib_path: &std::path::Path,
    snippets: &Snippets,
    sync_settings: &SyncSettings,
    client: &mut sync::SyncClient,
    mgr: &mut library::LibraryManager,
    runtime: &tokio::runtime::Runtime,
    status: &mut SyncStatus,
    results: &mut Vec<(String, bool, String)>,
) {
    tracing::info!(library = %lib_name, "Server library deleted, re-creating on server");
    let normalized_name = normalized_library_name(lib_name);

    let Some(recovery_dir) = lib_path.parent() else {
        tracing::error!(library = %lib_name, "Cannot create recovery marker: path has no parent");
        status.failed += 1;
        results.push((
            lib_name.to_string(),
            false,
            "Recovery path has no parent".to_string(),
        ));
        return;
    };
    let recovery_marker = recovery_marker_path(recovery_dir, lib_name);
    let mut marker = match fs::symlink_metadata(&recovery_marker) {
        Ok(_) => match read_recovery_marker(&recovery_marker) {
            Ok(marker) => marker,
            Err(e) => {
                tracing::error!(library = %lib_name, error = %e, "Recovery marker is corrupt; refusing blind recreation");
                status.failed += 1;
                results.push((lib_name.to_string(), false, e.to_string()));
                return;
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => SyncRecoveryMarker {
            schema: 1,
            local_library_name: lib_name.to_string(),
            local_library_id: mgr
                .get_library_by_filename(lib_name)
                .map(|lib| lib.library_id.clone())
                .unwrap_or_default(),
            server_library_id: None,
            created_at_unix_ms: chrono::Utc::now().timestamp_millis(),
            phase: RecoveryPhase::Creating,
        },
        Err(error) => {
            let e = SnipError::io_error("inspect sync recovery marker", &recovery_marker, error);
            tracing::error!(library = %lib_name, error = %e, "Recovery marker lookup failed closed");
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        }
    };

    let Some(local_meta) = mgr.get_library_by_filename(lib_name) else {
        let e = SnipError::runtime_error(
            "Sync recovery identity mismatch",
            Some("local library is not registered"),
        );
        status.failed += 1;
        results.push((lib_name.to_string(), false, e.to_string()));
        return;
    };
    let marker_stem = recovery_marker
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".sync_recovery"));
    let identity_ok = marker.schema == 1
        && normalized_library_name(&marker.local_library_name) == normalized_name
        && marker_stem == Some(lib_name)
        && (marker.local_library_id.is_empty() || marker.local_library_id == local_meta.library_id)
        && lib_path.exists();
    if !identity_ok {
        let e = SnipError::runtime_error(
            "Sync recovery identity mismatch",
            Some("marker identity does not match the current local library"),
        );
        tracing::error!(library = %lib_name, error = %e, "Refusing mismatched recovery marker");
        status.failed += 1;
        results.push((lib_name.to_string(), false, e.to_string()));
        return;
    }

    let already_linked = marker.phase == RecoveryPhase::Linked;
    if already_linked {
        let Some(server_id) = marker.server_library_id.as_deref() else {
            let e = SnipError::runtime_error(
                "Invalid linked recovery marker",
                Some("missing server library ID"),
            );
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        };
        if local_meta.server_id.as_deref() != Some(server_id) || local_meta.library_id != server_id
        {
            let e = SnipError::runtime_error(
                "Sync recovery identity mismatch",
                Some("linked marker does not match local linkage"),
            );
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        }
    }

    if let Err(e) = write_recovery_marker(&recovery_marker, &marker) {
        tracing::error!(library = %lib_name, error = %e, "Failed to persist recovery marker; refusing remote recreation");
        status.failed += 1;
        results.push((lib_name.to_string(), false, e.to_string()));
        return;
    }

    let server_lib = if already_linked {
        let Some(server_id) = marker.server_library_id.clone() else {
            let e = SnipError::runtime_error(
                "Invalid linked recovery marker",
                Some("missing server library ID"),
            );
            tracing::error!(library = %lib_name, error = %e, "Refusing recovery with missing server library ID");
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        };
        crate::proto::Library {
            id: server_id,
            name: normalized_name.clone(),
            created_at: 0,
            snippet_count: 0,
        }
    } else if let Some(server_id) = marker.server_library_id.clone() {
        crate::proto::Library {
            id: server_id,
            name: normalized_name.clone(),
            created_at: 0,
            snippet_count: 0,
        }
    } else {
        let existing = match runtime.block_on(client.list_libraries()) {
            Ok(libraries) => libraries
                .into_iter()
                .filter(|lib| normalized_library_name(&lib.name) == normalized_name)
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::error!(library = %lib_name, error = %e, "Could not inspect remote libraries during recovery");
                status.failed += 1;
                results.push((lib_name.to_string(), false, e.to_string()));
                return;
            }
        };

        match existing.as_slice() {
            [server_lib] => server_lib.clone(),
            [] => match runtime.block_on(client.create_library(&normalized_name)) {
                Ok(server_lib) => server_lib,
                Err(e) => {
                    tracing::error!(library = %lib_name, error = %e, "Failed to re-create library on server");
                    status.failed += 1;
                    results.push((
                        lib_name.to_string(),
                        false,
                        format!("Library deleted and re-creation failed: {e}"),
                    ));
                    return;
                }
            },
            _ => {
                let e = SnipError::runtime_error(
                    "Ambiguous remote library recovery",
                    Some(&format!(
                        "multiple remote libraries normalize to '{normalized_name}'"
                    )),
                );
                tracing::error!(library = %lib_name, error = %e, "Refusing ambiguous recovery");
                status.failed += 1;
                results.push((lib_name.to_string(), false, e.to_string()));
                return;
            }
        }
    };

    marker.server_library_id = Some(server_lib.id.clone());
    marker.phase = RecoveryPhase::RemoteCreated;
    if let Err(e) = write_recovery_marker(&recovery_marker, &marker) {
        tracing::error!(library = %lib_name, error = %e, "Failed to persist recovered server ID");
        status.failed += 1;
        results.push((lib_name.to_string(), false, e.to_string()));
        return;
    }

    if !already_linked {
        if let Err(e) = mgr.relink_server_library(lib_name, &server_lib.id, Some(0)) {
            tracing::error!(library = %lib_name, error = %e, "Failed to persist recovered library linkage");
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        }
        marker.phase = RecoveryPhase::Linked;
        if let Err(e) = write_recovery_marker(&recovery_marker, &marker) {
            tracing::error!(library = %lib_name, error = %e, "Failed to persist linked recovery state");
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
            return;
        }
    }

    tracing::info!(library = %lib_name, server_id = %server_lib.id, "Re-created and relinked library");
    let local_snippets_for_retry: Vec<ProtoSnippet> =
        snippets.snippets.iter().map(ProtoSnippet::from).collect();
    let retry_result =
        runtime.block_on(client.sync_encrypted(local_snippets_for_retry, 0, &server_lib.id));
    match retry_result {
        Ok(retry_response) if retry_response.success => {
            let server_snippets = retry_response.snippets;
            match merge_and_save(
                lib_path,
                lib_name,
                snippets,
                &server_snippets,
                &sync_settings.device_id,
            ) {
                Ok((_merged, _backup, _conflicts)) => {
                    if let Err(e) = mgr.update_last_sync(lib_name, retry_response.server_timestamp)
                    {
                        status.failed += 1;
                        results.push((lib_name.to_string(), false, e.to_string()));
                        return;
                    }
                    if recovery_marker.exists()
                        && let Err(e) = fs::remove_file(&recovery_marker)
                    {
                        tracing::warn!(library = %lib_name, error = %e, "Failed to remove recovery marker");
                    }
                    status.add_pulled(server_snippets.len());
                    results.push((
                        lib_name.to_string(),
                        true,
                        "Re-linked and synced".to_string(),
                    ));
                }
                Err(e) => {
                    status.failed += 1;
                    results.push((lib_name.to_string(), false, e.to_string()));
                }
            }
        }
        Ok(retry_response) => {
            status.failed += 1;
            results.push((lib_name.to_string(), false, retry_response.message));
        }
        Err(e) => {
            status.failed += 1;
            results.push((lib_name.to_string(), false, e.to_string()));
        }
    }
}

fn check_and_complete_recovery_markers(
    libraries_dir: &Path,
    sync_settings: &SyncSettings,
    client: &mut sync::SyncClient,
    mgr: &mut library::LibraryManager,
    runtime: &tokio::runtime::Runtime,
    status: &mut SyncStatus,
    results: &mut Vec<(String, bool, String)>,
) -> SnipResult<HashSet<String>> {
    let mut completed = HashSet::new();
    let entries = match fs::read_dir(libraries_dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(error = %e, path = %libraries_dir.display(), "Failed to read libraries directory for recovery marker check");
            return Ok(completed);
        }
    };

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sync_recovery") {
            let Some(stem) = path.file_stem() else {
                continue;
            };
            let lib_name = stem.to_string_lossy();
            match read_recovery_marker(&path) {
                Ok(_marker) => {
                    let lib_name = lib_name.to_string();
                    let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
                    if !lib_path.exists() {
                        tracing::error!(library = %lib_name, "Recovery marker has no local library; preserving marker");
                        continue;
                    }
                    let snippets = match library::load_library(&lib_path) {
                        Ok(snippets) => snippets,
                        Err(error) => {
                            tracing::error!(library = %lib_name, %error, "Could not load library for recovery");
                            status.failed += 1;
                            results.push((lib_name, false, error.to_string()));
                            continue;
                        }
                    };
                    let before = results.len();
                    handle_library_not_found(
                        &lib_name,
                        &lib_path,
                        &snippets,
                        sync_settings,
                        client,
                        mgr,
                        runtime,
                        status,
                        results,
                    );
                    if results.len() > before
                        && results.last().is_some_and(|(_, success, _)| *success)
                    {
                        completed.insert(lib_name);
                    }
                }
                Err(e) => {
                    tracing::error!(library = %lib_name, error = %e, "Recovery marker is corrupt and was preserved")
                }
            }
        }
    }
    Ok(completed)
}

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

fn get_library_sync_info(mgr: &mut library::LibraryManager, lib_name: &str) -> (String, i64) {
    match mgr.get_library_by_filename(lib_name) {
        Some(l) => {
            let id = l.library_id.clone();
            let server_id = l.server_id.clone();
            let last_sync = l.last_sync.unwrap_or(0);
            if server_id.as_deref() != Some(id.as_str()) {
                tracing::warn!(
                    "Library '{}' has library_id '{}' but server_id '{:?}' — possible stale config",
                    lib_name,
                    id,
                    server_id
                );
                if let Some(server_id) = server_id {
                    if let Err(e) = mgr.relink_server_library(lib_name, &server_id, Some(last_sync))
                    {
                        tracing::error!(library = %lib_name, error = %e, "Failed to repair stale library linkage");
                    }
                    return (server_id, last_sync);
                }
                // Without an authoritative server ID, return an empty ID so
                // the caller follows the normal create/re-link path.
                return (String::new(), last_sync);
            }
            (id, last_sync)
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

fn check_server_health(
    runtime: &tokio::runtime::Runtime,
    client: &mut sync::SyncClient,
    server_url: &str,
) -> SnipResult<()> {
    match runtime.block_on(client.health_check()) {
        Ok(true) => Ok(()),
        Ok(false) => {
            tracing::error!("Server is not reachable at {}", server_url);
            Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::HealthCheckFailed,
                None,
            ))
        }
        Err(error) => Err(error),
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
            return Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::ConnectFailed,
                Some(&e.to_string()),
            ));
        }
    };

    let libs = match runtime.block_on(client.list_premade_libraries()) {
        Ok(libs) => libs,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to list premade libraries");
            return Ok(());
        }
    };

    if libs.is_empty() {
        return Ok(());
    }

    let mgr = match library::LibraryManager::new() {
        Ok(m) => m,
        Err(e) => {
            return Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::LibraryManagerInitFailed,
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
            return Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::PremadePartialFailure,
                None,
            ));
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

    fn add_pulled(&mut self, count: usize) {
        self.pulled = self
            .pulled
            .saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
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

    // save_library uses atomic rename, so the original file is always safe
    // on failure. No explicit backup/restore is needed here.
    if let Err(e) = library::save_library(lib_path, &merged) {
        return Err(SnipError::sync_failure(
            crate::error::SyncFailureKind::SaveMergedLibraryFailed,
            Some(&e.to_string()),
        ));
    }

    Ok((merged, None, conflicting_ids))
}

/// Performs a full sync operation across one or more libraries.
///
/// Supports push-only, pull-only, and bidirectional modes. Creates server-side
/// libraries for any unlinked local libraries, then merges snippets using
/// last-write-wins conflict resolution.
pub fn run_sync(
    sync_settings: &SyncSettings,
    library_name: Option<&str>,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    run_sync_with_limits(
        sync_settings,
        library_name,
        push_only,
        pull_only,
        runtime,
        None,
    )
}

pub(crate) fn run_sync_with_limits(
    sync_settings: &SyncSettings,
    library_name: Option<&str>,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
    limits: Option<sync::SyncRunLimits>,
) -> SnipResult<()> {
    let _key_cache_guard = KeyCacheGuard;
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
        return Err(SnipError::sync_failure(
            crate::error::SyncFailureKind::NotConfigured,
            None,
        ));
    }

    let mut client = runtime
        .block_on(sync::SyncClient::create_with_limits(
            sync_settings.clone(),
            limits,
        ))
        .map_err(|e| {
            SnipError::sync_failure(
                crate::error::SyncFailureKind::ConnectFailed,
                Some(&e.to_string()),
            )
        })?;

    check_server_health(runtime, &mut client, &sync_settings.server_url)?;

    let mut mgr = match library::LibraryManager::new() {
        Ok(m) => m,
        Err(e) => {
            return Err(SnipError::sync_failure(
                crate::error::SyncFailureKind::LibraryManagerInitFailed,
                Some(&e.to_string()),
            ));
        }
    };

    if let Err(e) = mgr.ensure_library_mode() {
        return Err(SnipError::sync_failure(
            crate::error::SyncFailureKind::LibraryModeInitFailed,
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
                return Err(SnipError::sync_failure(
                    crate::error::SyncFailureKind::LibrariesDirReadFailed,
                    Some(&e.to_string()),
                ));
            }
        }
    };

    if libraries_to_sync.is_empty() {
        tracing::warn!("No libraries to sync");
        return Err(SnipError::sync_failure(
            crate::error::SyncFailureKind::NoLibrariesToSync,
            None,
        ));
    }

    for lib_name in &libraries_to_sync {
        let lib_path = mgr.get_libraries_dir().join(format!("{lib_name}.toml"));

        if !lib_path.exists() {
            tracing::warn!(library = %lib_name, "Library file not found, skipping");
            continue;
        }

        let (library_id, _last_sync) = get_library_sync_info(&mut mgr, lib_name);

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

                    if let Err(e) = mgr.link_server_library(lib_name, &new_id) {
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

    let libraries_dir = mgr.get_libraries_dir().clone();
    let recovered = match check_and_complete_recovery_markers(
        &libraries_dir,
        sync_settings,
        &mut client,
        &mut mgr,
        runtime,
        &mut status,
        &mut results,
    ) {
        Ok(recovered) => recovered,
        Err(error) => {
            tracing::warn!(%error, "Recovery marker scan failed");
            HashSet::new()
        }
    };

    for lib_name in &libraries_to_sync {
        if recovered.contains(lib_name) {
            continue;
        }
        completed += 1;
        if std::io::stdout().is_terminal() {
            print!("\r[{completed}/{total}] Syncing {lib_name}...");
            std::io::Write::flush(&mut std::io::stdout()).ok();
        }

        let lib_path = mgr.get_libraries_dir().join(format!("{lib_name}.toml"));

        if !lib_path.exists() {
            tracing::warn!(library = %lib_name, "Library file not found, skipping sync");
            continue;
        }

        let (library_id, last_sync) = get_library_sync_info(&mut mgr, lib_name);

        if library_id.is_empty() {
            tracing::warn!(library = %lib_name, "Library not linked to server, skipping");
            continue;
        }

        let snippets = match library::load_library(&lib_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(library = %lib_name, error = %e, "Failed to load library");
                continue;
            }
        };

        if direction == SyncDirection::Push || direction == SyncDirection::Bidirectional {
            let local_snippets: Vec<ProtoSnippet> =
                snippets.snippets.iter().map(ProtoSnippet::from).collect();

            if local_snippets.is_empty() && direction == SyncDirection::Push {
                tracing::info!(library = %lib_name, "No local changes to push, skipping");
                continue;
            }

            let result =
                runtime.block_on(client.sync_encrypted(local_snippets, last_sync, &library_id));

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
                                status.pushed += 1;
                            } else {
                                status.conflicts += 1;
                                results.push((
                                    lib_name.clone(),
                                    true,
                                    format!(
                                        "{} snippets skipped (will retry)",
                                        response.skipped_count
                                    ),
                                ));
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

                                status.add_pulled(server_snippets.len());
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
                    if matches!(
                        e,
                        SnipError::SyncFailure {
                            kind: crate::error::SyncFailureKind::LibraryNotFound,
                            ..
                        }
                    ) {
                        handle_library_not_found(
                            lib_name,
                            &lib_path,
                            &snippets,
                            sync_settings,
                            &mut client,
                            &mut mgr,
                            runtime,
                            &mut status,
                            &mut results,
                        );
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, e.to_string()));
                    }
                }
            }
        }

        if direction == SyncDirection::Pull && !library_id.is_empty() {
            let result = runtime.block_on(client.sync_encrypted(vec![], last_sync, &library_id));

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
                                status.add_pulled(server_snippets.len());
                                if !conflicts.is_empty() {
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
                            }
                        }
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, response.message));
                    }
                }
                Err(e) => {
                    if matches!(
                        e,
                        SnipError::SyncFailure {
                            kind: crate::error::SyncFailureKind::LibraryNotFound,
                            ..
                        }
                    ) {
                        handle_library_not_found(
                            lib_name,
                            &lib_path,
                            &snippets,
                            sync_settings,
                            &mut client,
                            &mut mgr,
                            runtime,
                            &mut status,
                            &mut results,
                        );
                    } else {
                        status.failed += 1;
                        results.push((lib_name.clone(), false, e.to_string()));
                    }
                }
            }
        }
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
        Err(SnipError::sync_failure(
            crate::error::SyncFailureKind::PartialSyncFailure,
            None,
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VersionKey {
    updated_at: i64,
    device_id: String,
    fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionWinner {
    Local,
    Remote,
    Equivalent,
}

fn hash_field(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn fingerprint(
    id: &str,
    description: &str,
    command: &str,
    tags: &[String],
    created_at: i64,
    updated_at: i64,
    device_id: &str,
    deleted: bool,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, id);
    hash_field(&mut hasher, description);
    hash_field(&mut hasher, command);
    hasher.update((tags.len() as u64).to_le_bytes());
    for tag in tags {
        hash_field(&mut hasher, tag);
    }
    hasher.update(created_at.to_le_bytes());
    hasher.update(updated_at.to_le_bytes());
    hash_field(&mut hasher, device_id);
    hasher.update([deleted as u8]);
    hasher.finalize().into()
}

fn local_version_key(snippet: &Snippet) -> VersionKey {
    VersionKey {
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        fingerprint: fingerprint(
            &snippet.id,
            &snippet.description,
            &snippet.command,
            &snippet.tags,
            snippet.created_at,
            snippet.updated_at,
            &snippet.device_id,
            snippet.deleted,
        ),
    }
}

fn remote_version_key(snippet: &ProtoSnippet) -> VersionKey {
    VersionKey {
        updated_at: snippet.updated_at,
        device_id: snippet.device_id.clone(),
        fingerprint: fingerprint(
            &snippet.id,
            &snippet.description,
            &snippet.command,
            &snippet.tags,
            snippet.created_at,
            snippet.updated_at,
            &snippet.device_id,
            snippet.deleted,
        ),
    }
}

fn choose_version(local: &Snippet, remote: &ProtoSnippet) -> VersionWinner {
    // This product intentionally has no resurrection: an explicit deletion
    // wins even when the other copy has a later wall-clock timestamp.
    match (local.deleted, remote.deleted) {
        (true, false) => VersionWinner::Local,
        (false, true) => VersionWinner::Remote,
        (true, true) => VersionWinner::Equivalent,
        (false, false) => match local_version_key(local).cmp(&remote_version_key(remote)) {
            Ordering::Less => VersionWinner::Remote,
            Ordering::Greater => VersionWinner::Local,
            Ordering::Equal => VersionWinner::Equivalent,
        },
    }
}

fn merge_snippets(local: &Snippets, server_snippets: &[ProtoSnippet]) -> Snippets {
    let local_by_id: std::collections::HashMap<_, _> =
        local.snippets.iter().map(|s| (s.id.clone(), s)).collect();

    let mut merged_snippets: Vec<Snippet> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for server_snip in server_snippets {
        seen_ids.insert(server_snip.id.clone());

        if let Some(local_snip) = local_by_id.get(&server_snip.id) {
            match choose_version(local_snip, server_snip) {
                VersionWinner::Remote if server_snip.deleted => merged_snippets.push(Snippet {
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
                }),
                VersionWinner::Remote => merged_snippets.push(Snippet {
                    id: server_snip.id.clone(),
                    description: server_snip.description.clone(),
                    command: server_snip.command.clone(),
                    output: local_snip.output.clone(),
                    tags: server_snip.tags.clone(),
                    folders: local_snip.folders.clone(),
                    favorite: local_snip.favorite,
                    created_at: local_snip.created_at.min(server_snip.created_at),
                    updated_at: server_snip.updated_at,
                    device_id: server_snip.device_id.clone(),
                    deleted: false,
                }),
                VersionWinner::Local if local_snip.deleted => merged_snippets.push(Snippet {
                    id: local_snip.id.clone(),
                    description: local_snip.description.clone(),
                    command: local_snip.command.clone(),
                    output: local_snip.output.clone(),
                    tags: local_snip.tags.clone(),
                    folders: local_snip.folders.clone(),
                    favorite: local_snip.favorite,
                    created_at: local_snip.created_at,
                    updated_at: local_snip.updated_at.max(server_snip.updated_at),
                    device_id: local_snip.device_id.clone(),
                    deleted: true,
                }),
                VersionWinner::Local => merged_snippets.push((*local_snip).clone()),
                VersionWinner::Equivalent if local_snip.deleted => continue,
                VersionWinner::Equivalent => merged_snippets.push((*local_snip).clone()),
            }
        } else {
            // A server-only tombstone is intentionally dropped: the local
            // device never observed this snippet and therefore cannot have a
            // local deletion to preserve. This is the no-resurrection policy
            // for deletions that were never synced to this device.
            if server_snip.deleted {
                continue;
            }
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

    merged_snippets.sort_by_key(|b| {
        (
            std::cmp::Reverse(b.updated_at),
            std::cmp::Reverse(local_version_key(b)),
        )
    });

    Snippets {
        snippets: merged_snippets,
        folders: local.folders.clone(),
    }
}

/// Runs a sync with the default settings (bidirectional, all libraries).
pub fn run_default_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let settings = crate::config::load_sync_settings().unwrap_or_default();
    run_sync(&settings, None, false, false, runtime)
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

    fn as_proto(snippet: &Snippet) -> ProtoSnippet {
        ProtoSnippet::from(snippet)
    }

    fn as_snippet(snippet: &ProtoSnippet) -> Snippet {
        Snippet {
            id: snippet.id.clone(),
            description: snippet.description.clone(),
            command: snippet.command.clone(),
            output: String::new(),
            tags: snippet.tags.clone(),
            folders: Vec::new(),
            favorite: false,
            created_at: snippet.created_at,
            updated_at: snippet.updated_at,
            device_id: snippet.device_id.clone(),
            deleted: snippet.deleted,
        }
    }

    fn merged_description(local: &Snippet, remote: &ProtoSnippet) -> String {
        merge_snippets(
            &Snippets {
                snippets: vec![local.clone()],
                folders: Vec::new(),
            },
            std::slice::from_ref(remote),
        )
        .snippets
        .first()
        .map(|snippet| snippet.description.clone())
        .unwrap_or_default()
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
    fn test_equal_timestamp_different_devices_is_role_independent() {
        let mut a = make_local_snippet("1", "A", "echo A", 100);
        a.device_id = "device-a".to_string();
        let mut b = make_server_snippet("1", "B", "echo B", 100);
        b.device_id = "device-b".to_string();

        let forward = merged_description(&a, &b);
        let reverse = merged_description(&as_snippet(&b), &as_proto(&a));
        assert_eq!(forward, reverse);
        assert!(matches!(
            choose_version(&a, &b),
            VersionWinner::Local | VersionWinner::Remote
        ));
    }

    #[test]
    fn test_equal_timestamp_same_device_uses_content_fingerprint() {
        let mut a = make_local_snippet("1", "A", "echo A", 100);
        a.device_id = "same-device".to_string();
        let mut b = make_server_snippet("1", "B", "echo B", 100);
        b.device_id = "same-device".to_string();

        let forward = merged_description(&a, &b);
        let reverse = merged_description(&as_snippet(&b), &as_proto(&a));
        assert_eq!(forward, reverse);
        assert_ne!(forward, "");
    }

    #[test]
    fn test_equal_timestamp_delete_live_is_role_independent() {
        let mut deleted = make_local_snippet("1", "deleted", "echo deleted", 100);
        deleted.deleted = true;
        let live = make_server_snippet("1", "live", "echo live", 100);

        let forward = merge_snippets(
            &Snippets {
                snippets: vec![deleted.clone()],
                folders: Vec::new(),
            },
            std::slice::from_ref(&live),
        );
        let reverse = merge_snippets(
            &Snippets {
                snippets: vec![as_snippet(&live)],
                folders: Vec::new(),
            },
            &[as_proto(&deleted)],
        );
        assert_eq!(forward.snippets[0].deleted, reverse.snippets[0].deleted);
        assert!(forward.snippets[0].deleted);
        assert!(reverse.snippets[0].deleted);
    }

    #[test]
    fn test_recovery_marker_is_atomic_and_corrupt_marker_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = recovery_marker_path(dir.path(), "work");
        let marker = SyncRecoveryMarker {
            schema: 1,
            local_library_name: "work".to_string(),
            local_library_id: "local-id".to_string(),
            server_library_id: Some("server-id".to_string()),
            created_at_unix_ms: 1,
            phase: RecoveryPhase::RemoteCreated,
        };
        write_recovery_marker(&path, &marker).unwrap();
        assert_eq!(read_recovery_marker(&path).unwrap(), marker);

        fs::write(&path, "not valid toml = [").unwrap();
        assert!(read_recovery_marker(&path).is_err());
        assert!(path.exists());
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

    #[test]
    fn test_local_deleted_not_resurrected_by_newer_server() {
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
            description: "server version".to_string(),
            command: "echo server".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        }];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 1);
        assert!(
            merged.snippets[0].deleted,
            "locally deleted snippet should stay deleted even when server has a newer non-deleted copy"
        );
        assert_eq!(merged.snippets[0].updated_at, 200);
    }

    #[test]
    fn test_proto_snippet_excludes_usage_metadata() {
        // Verify that converting library::Snippet to ProtoSnippet does not
        // carry over local-only fields (output, folders, favorite).  Usage
        // data (use_count, last_used_at) lives in a separate file
        // (usage.toml) and is never loaded during sync, so there is no
        // field on library::Snippet to carry.  This test is a regression
        // guard: if someone adds usage fields to the proto schema, this
        // test ensures they are not silently included in sync payloads.
        let local = Snippet {
            id: "test-id".to_string(),
            description: "desc".to_string(),
            command: "echo hello".to_string(),
            tags: vec!["tag".to_string()],
            folders: vec!["folder".to_string()],
            output: "cached output".to_string(),
            favorite: true,
            created_at: 1000,
            updated_at: 2000,
            device_id: "device-1".to_string(),
            deleted: false,
        };

        let proto: ProtoSnippet = (&local).into();

        // ProtoSnippet should carry sync-relevant fields
        assert_eq!(proto.id, "test-id");
        assert_eq!(proto.description, "desc");
        assert_eq!(proto.command, "echo hello");
        assert_eq!(proto.tags, vec!["tag".to_string()]);
        assert_eq!(proto.created_at, 1000);
        assert_eq!(proto.updated_at, 2000);
        assert_eq!(proto.device_id, "device-1");

        // ProtoSnippet (prost-generated) intentionally does NOT have these
        // fields: output, folders, favorite, use_count, last_used_at.
        // The compiler enforces their absence — any attempt to access a
        // nonexistent field is a compile error.  This test documents that
        // contract and serves as a regression guard for future changes.
        //
        // If you need to add a field to ProtoSnippet, ensure it is not
        // local-only usage metadata before adding it here.
    }

    #[test]
    fn test_merge_preserves_local_output_when_server_wins() {
        let local = Snippets {
            snippets: vec![Snippet {
                id: "1".to_string(),
                description: "local desc".to_string(),
                command: "echo local".to_string(),
                tags: vec![],
                folders: vec![],
                output: "local output metadata".to_string(),
                favorite: false,
                created_at: 100,
                updated_at: 100,
                device_id: "d".to_string(),
                deleted: false,
            }],
            folders: vec![],
        };
        let server = vec![ProtoSnippet {
            id: "1".to_string(),
            description: "server desc".to_string(),
            command: "echo server".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200, // server is newer
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        }];

        let merged = merge_snippets(&local, &server);
        assert_eq!(merged.snippets.len(), 1);
        // Server wins on description/command (newer timestamp)
        assert_eq!(merged.snippets[0].description, "server desc");
        assert_eq!(merged.snippets[0].command, "echo server");
        // But local output is preserved (it's a local-only field)
        assert_eq!(merged.snippets[0].output, "local output metadata");
    }
}

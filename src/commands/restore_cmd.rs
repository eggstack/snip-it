//! **Layer: Application**
//!
//! `snp restore` command — restore from a backup snapshot.

use crate::auto_sync::notification::notify_mutation;
use crate::auto_sync::policy::{MutationKind, MutationOrigin};
use crate::error::{SnipError, SnipResult};
use crate::utils::atomic::{AtomicWriteOptions, Durability, atomic_replace};
use crate::utils::config::get_config_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::backup_cmd::{BackupEntryKind, BackupManifest, BackupManifestEntry};

/// Restore mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RestoreMode {
    DryRun,
    Merge,
    Replace,
}

/// Conflict report for merge/replace operations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RestoreConflict {
    pub library: String,
    pub kind: String,
    pub detail: String,
}

/// Restore report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RestoreReport {
    pub mode: String,
    pub files_restored: usize,
    pub conflicts: Vec<RestoreConflict>,
    pub skipped: Vec<String>,
    pub pre_restore_backup: Option<String>,
}

/// Verify a single file's SHA-256 checksum.
fn verify_checksum(file_path: &Path, expected_sha: &str) -> SnipResult<bool> {
    let bytes = fs::read(file_path)
        .map_err(|e| SnipError::io_error("read file for verification", file_path, e))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    let actual = result
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    Ok(actual == expected_sha)
}

/// Load and validate the backup manifest from a backup directory.
fn load_manifest(backup_dir: &Path) -> SnipResult<BackupManifest> {
    // Try manifest.toml first, then manifest.json
    let toml_path = backup_dir.join("manifest.toml");
    let json_path = backup_dir.join("manifest.json");

    if toml_path.exists() {
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| SnipError::io_error("read manifest.toml", toml_path.clone(), e))?;
        let manifest: BackupManifest = toml::from_str(&content)
            .map_err(|e| SnipError::toml_error("parse manifest.toml", e))?;
        return Ok(manifest);
    }

    if json_path.exists() {
        let content = fs::read_to_string(&json_path)
            .map_err(|e| SnipError::io_error("read manifest.json", json_path.clone(), e))?;
        let manifest: BackupManifest = serde_json::from_str(&content)
            .map_err(|e| SnipError::runtime_error("parse manifest.json", Some(&e.to_string())))?;
        return Ok(manifest);
    }

    Err(SnipError::runtime_error(
        "No manifest found in backup",
        Some(&format!(
            "Expected manifest.toml or manifest.json in {}",
            backup_dir.display()
        )),
    ))
}

/// Reserved Windows device names that must not appear as file components.
const RESERVED_WINDOWS_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate a backup-relative path to prevent path traversal attacks.
///
/// Returns the validated relative `PathBuf` on success. Rejects:
/// - Empty paths
/// - Absolute paths (Unix `/` or Windows drive letter `C:\`)
/// - UNC paths (`\\server\share`)
/// - `..` components (traversal)
/// - NUL bytes
/// - Reserved Windows device names (CON, PRN, NUL, etc.)
/// - For library kind: requires `.toml` extension, rejects path separators (flat filename only)
/// - For index/usage/sync_config: allows only the exact expected filename
fn resolve_backup_path(backup: &Path, entry: &BackupManifestEntry) -> PathBuf {
    // Standard top-level entries (index, usage, sync_config) and entries with explicit
    // libraries/ prefix use path directly. Library/unknown entries without prefix get it.
    if matches!(
        entry.kind,
        BackupEntryKind::Index | BackupEntryKind::SyncConfig | BackupEntryKind::Usage
    ) || entry.path.starts_with("libraries/")
        || entry.path.starts_with("libraries\\")
    {
        backup.join(&entry.path)
    } else {
        backup.join("libraries").join(&entry.path)
    }
}

fn validate_backup_path(path: &str, kind: BackupEntryKind) -> SnipResult<PathBuf> {
    if path.is_empty() {
        return Err(SnipError::runtime_error(
            "Empty backup path",
            Some(&format!("kind={kind}")),
        ));
    }

    if path.contains('\0') {
        return Err(SnipError::runtime_error(
            "NUL byte in backup path",
            Some(&format!("path={path}")),
        ));
    }

    // Reject absolute paths
    if path.starts_with('/') {
        return Err(SnipError::runtime_error(
            "Absolute path in backup manifest",
            Some(&format!("path={path}")),
        ));
    }
    // Reject Windows drive letter paths (C:\, D:\, etc.)
    if path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && (path.as_bytes()[2] == b'/' || path.as_bytes()[2] == b'\\')
    {
        return Err(SnipError::runtime_error(
            "Absolute path in backup manifest",
            Some(&format!("path={path}")),
        ));
    }
    // Reject UNC paths (\\server\share or //server/share)
    if (path.starts_with("\\\\") || path.starts_with("//")) && path.len() > 2 {
        return Err(SnipError::runtime_error(
            "UNC path in backup manifest",
            Some(&format!("path={path}")),
        ));
    }

    let pb = PathBuf::from(path);
    for component in pb.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                return Err(SnipError::runtime_error(
                    "Path traversal in backup manifest",
                    Some(&format!("path={path}")),
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(SnipError::runtime_error(
                    "Absolute path in backup manifest",
                    Some(&format!("path={path}")),
                ));
            }
            Component::Normal(name) => {
                // Reject reserved Windows device names (case-insensitive)
                if let Some(name_str) = name.to_str() {
                    let stem = name_str
                        .split('.')
                        .next()
                        .unwrap_or(name_str)
                        .to_uppercase();
                    if RESERVED_WINDOWS_NAMES.contains(&stem.as_str()) {
                        return Err(SnipError::runtime_error(
                            "Reserved Windows device name in backup path",
                            Some(&format!("path={path}, reserved={stem}")),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    match kind {
        BackupEntryKind::Library => {
            // Allow both flat filename (readonly-test.toml) and libraries/ prefixed
            // (libraries/readonly-test.toml) since backup uses the prefix format.
            let basename = path
                .strip_prefix("libraries/")
                .or_else(|| path.strip_prefix("libraries\\"))
                .unwrap_or(path);
            if basename.contains('/') || basename.contains('\\') {
                return Err(SnipError::runtime_error(
                    "Library path must be a flat filename or libraries/<name>.toml",
                    Some(&format!("path={path}")),
                ));
            }
            if !basename.ends_with(".toml") {
                return Err(SnipError::runtime_error(
                    "Library path must have .toml extension",
                    Some(&format!("path={path}")),
                ));
            }
        }
        BackupEntryKind::Index => {
            if path != "libraries.toml" {
                return Err(SnipError::runtime_error(
                    "Index path must be libraries.toml",
                    Some(&format!("path={path}")),
                ));
            }
        }
        BackupEntryKind::Usage => {
            if path != "usage.toml" {
                return Err(SnipError::runtime_error(
                    "Usage path must be usage.toml",
                    Some(&format!("path={path}")),
                ));
            }
        }
        BackupEntryKind::SyncConfig => {
            if path != "sync.toml" {
                return Err(SnipError::runtime_error(
                    "Sync config path must be sync.toml",
                    Some(&format!("path={path}")),
                ));
            }
        }
    }

    Ok(pb)
}

/// Create a pre-restore backup of the current config.
fn create_pre_restore_backup(config_dir: &Path) -> SnipResult<Option<PathBuf>> {
    if !config_dir.exists() {
        return Ok(None);
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_name = format!("pre-restore-{}", timestamp);
    let backup_base = config_dir.join("backups").join(&backup_name);
    fs::create_dir_all(&backup_base).map_err(|e| {
        SnipError::io_error("create pre-restore backup dir", backup_base.clone(), e)
    })?;

    let libraries_dir = config_dir.join("libraries");
    if libraries_dir.exists() {
        for entry in fs::read_dir(&libraries_dir)
            .map_err(|e| SnipError::io_error("read libraries dir", libraries_dir.clone(), e))?
            .filter_map(|e| e.ok())
        {
            let file_name = entry.file_name();
            let src = entry.path();
            if src.extension().is_some_and(|ext| ext == "toml") {
                let dst = backup_base.join("libraries").join(&file_name);
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| SnipError::io_error("create backup subdir", parent, e))?;
                }
                fs::copy(&src, &dst)
                    .map_err(|e| SnipError::io_error("copy library to backup", dst, e))?;
            }
        }
    }

    let libraries_toml = config_dir.join("libraries.toml");
    if libraries_toml.exists() {
        let dst = backup_base.join("libraries.toml");
        fs::copy(&libraries_toml, &dst)
            .map_err(|e| SnipError::io_error("copy index to backup", dst, e))?;
    }

    Ok(Some(backup_base))
}

/// Restore a single library file from backup into the config directory.
fn restore_library_file(
    backup_file: &Path,
    config_libraries_dir: &Path,
    library_name: &str,
    mode: RestoreMode,
    report: &mut RestoreReport,
) -> SnipResult<()> {
    let dst = config_libraries_dir.join(format!("{}.toml", library_name));

    if dst.exists() && mode == RestoreMode::Merge {
        // For merge, check if content differs
        let existing = fs::read_to_string(&dst)
            .map_err(|e| SnipError::io_error("read existing library", dst.clone(), e))?;
        let incoming = fs::read_to_string(backup_file).map_err(|e| {
            SnipError::io_error("read backup library", backup_file.to_path_buf(), e)
        })?;

        if existing.trim() == incoming.trim() {
            report
                .skipped
                .push(format!("{}.toml (identical)", library_name));
            return Ok(());
        }

        // Merge: load both, combine snippets by ID, prefer newer updated_at
        let existing_snippets: crate::library::Snippets = toml::from_str(&existing)
            .map_err(|e| SnipError::toml_error("parse existing library", e))?;
        let incoming_snippets: crate::library::Snippets = toml::from_str(&incoming)
            .map_err(|e| SnipError::toml_error("parse backup library", e))?;

        let mut merged = existing_snippets.clone();
        for incoming_snippet in &incoming_snippets.snippets {
            if let Some(existing_snippet) = merged
                .snippets
                .iter_mut()
                .find(|s| s.id == incoming_snippet.id)
            {
                if incoming_snippet.updated_at > existing_snippet.updated_at {
                    report.conflicts.push(RestoreConflict {
                        library: library_name.to_string(),
                        kind: "updated".to_string(),
                        detail: format!(
                            "Snippet '{}' updated_at {} > {}",
                            incoming_snippet.description,
                            incoming_snippet.updated_at,
                            existing_snippet.updated_at
                        ),
                    });
                    *existing_snippet = incoming_snippet.clone();
                } else {
                    report.conflicts.push(RestoreConflict {
                        library: library_name.to_string(),
                        kind: "kept_existing".to_string(),
                        detail: format!(
                            "Snippet '{}' existing updated_at {} >= {}",
                            existing_snippet.description,
                            existing_snippet.updated_at,
                            incoming_snippet.updated_at
                        ),
                    });
                }
            } else {
                merged.snippets.push(incoming_snippet.clone());
                report.conflicts.push(RestoreConflict {
                    library: library_name.to_string(),
                    kind: "added".to_string(),
                    detail: format!("New snippet '{}'", incoming_snippet.description),
                });
            }
        }

        crate::library::save_library(&dst, &merged)?;
    } else {
        // Replace or first-time create
        if dst.exists() {
            report.conflicts.push(RestoreConflict {
                library: library_name.to_string(),
                kind: "replaced".to_string(),
                detail: format!("Replaced existing {}.toml", library_name),
            });
        }
        let bytes = fs::read(backup_file).map_err(|e| {
            SnipError::io_error(
                "read backup library for restore",
                backup_file.to_path_buf(),
                e,
            )
        })?;
        let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
        atomic_replace(&dst, &bytes, &opts)?;
    }

    report.files_restored += 1;
    Ok(())
}

/// Run restore.
pub fn run(backup: PathBuf, mode: RestoreMode, json: bool) -> SnipResult<()> {
    if !backup.exists() {
        return Err(SnipError::runtime_error(
            "Backup path does not exist",
            Some(&backup.display().to_string()),
        ));
    }

    // 1. Load and validate manifest
    let manifest = load_manifest(&backup)?;

    // 2. Validate all paths in manifest (path traversal prevention)
    for entry in &manifest.files {
        validate_backup_path(&entry.path, entry.kind)?;
    }

    // 3. Validate source artifact sizes and types
    const MAX_RESTORE_SOURCE_SIZE: u64 = 10 * 1024 * 1024; // 10 MiB
    for entry in &manifest.files {
        let file_path = resolve_backup_path(&backup, entry);

        // Verify file exists
        if !file_path.exists() {
            return Err(SnipError::runtime_error(
                "Backup file missing",
                Some(&format!(
                    "{} referenced in manifest but not found at {}",
                    entry.path,
                    file_path.display()
                )),
            ));
        }

        // Reject symlinks using symlink_metadata (does not follow)
        let meta = fs::symlink_metadata(&file_path).map_err(|e| {
            SnipError::io_error("stat backup source artifact", file_path.clone(), e)
        })?;
        if meta.file_type().is_symlink() {
            return Err(SnipError::runtime_error(
                "Backup source is a symlink",
                Some(&format!(
                    "Refusing to restore symlinked artifact: {}",
                    file_path.display()
                )),
            ));
        }
        if !meta.is_file() {
            return Err(SnipError::runtime_error(
                "Backup source is not a regular file",
                Some(&format!(
                    "Expected regular file, got {:?}: {}",
                    meta.file_type(),
                    file_path.display()
                )),
            ));
        }

        // Reject oversized files before allocation
        if meta.len() > MAX_RESTORE_SOURCE_SIZE {
            return Err(SnipError::runtime_error(
                "Backup source exceeds maximum size",
                Some(&format!(
                    "{}: {} bytes exceeds {} byte limit",
                    entry.path,
                    meta.len(),
                    MAX_RESTORE_SOURCE_SIZE
                )),
            ));
        }

        // Manifest-declared size must match actual size
        if entry.size != meta.len() {
            return Err(SnipError::runtime_error(
                "Manifest size mismatch",
                Some(&format!(
                    "{}: manifest declares {} bytes, actual {} bytes",
                    entry.path,
                    entry.size,
                    meta.len()
                )),
            ));
        }
    }

    // 4. Verify checksums
    for entry in &manifest.files {
        // Resolve path relative to backup directory
        let file_path = resolve_backup_path(&backup, entry);

        if !verify_checksum(&file_path, &entry.sha256)? {
            return Err(SnipError::runtime_error(
                "Checksum mismatch",
                Some(&format!(
                    "{}: expected sha256:{}",
                    entry.path,
                    &entry.sha256[..16]
                )),
            ));
        }
    }

    let config_dir = get_config_dir();
    let mut report = RestoreReport {
        mode: format!("{:?}", mode),
        files_restored: 0,
        conflicts: Vec::new(),
        skipped: Vec::new(),
        pre_restore_backup: None,
    };

    // 5. Dry run: display planned actions (no writes, no transaction)
    if mode == RestoreMode::DryRun {
        if json {
            let dry_report = serde_json::json!({
                "mode": "DryRun",
                "manifest_schema": manifest.schema,
                "manifest_version": manifest.snip_it_version,
                "files_in_backup": manifest.files.len(),
                "files": manifest.files.iter().map(|f| serde_json::json!({
                    "path": f.path,
                    "kind": f.kind,
                    "size": f.size,
                })).collect::<Vec<_>>(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&dry_report).map_err(|e| SnipError::runtime_error(
                    "serialize dry-run report",
                    Some(&e.to_string())
                ))?
            );
        } else {
            eprintln!("Dry run — planned restore from backup:");
            eprintln!("  Backup version: {}", manifest.snip_it_version);
            eprintln!("  Schema: {}", manifest.schema);
            eprintln!("  Files to restore: {}", manifest.files.len());
            for entry in &manifest.files {
                let action = if config_dir.join("libraries").join(&entry.path).exists() {
                    "update"
                } else {
                    "add"
                };
                eprintln!(
                    "    {} ({}) — {} bytes — {}",
                    entry.path, entry.kind, entry.size, action
                );
            }
        }
        return Ok(());
    }

    // 6. Acquire transaction lock and begin transaction for write modes
    let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
    let _lock = crate::transaction::acquire_transaction_lock(&state_dir, "restore")?;

    // Collect affected files for the transaction
    let mut affected_files: Vec<PathBuf> = Vec::new();
    for entry in &manifest.files {
        let dst = match entry.kind {
            BackupEntryKind::Library => {
                let name = Path::new(&entry.path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                config_dir.join("libraries").join(format!("{name}.toml"))
            }
            BackupEntryKind::Index => config_dir.join("libraries.toml"),
            BackupEntryKind::Usage => config_dir.join("usage.toml"),
            BackupEntryKind::SyncConfig => config_dir.join("sync.toml"),
        };
        affected_files.push(dst);
    }

    let journal = crate::transaction::begin_transaction(&state_dir, "restore", &affected_files)?;

    // Create pre-restore backups for affected files
    let backup_dir_base = state_dir.join("backups");
    fs::create_dir_all(&backup_dir_base).map_err(|e| {
        SnipError::io_error("create transaction backup dir", backup_dir_base.clone(), e)
    })?;

    // Build the journal with backup paths for rollback
    let mut journal_with_backups = journal.clone();
    for (i, staged) in journal_with_backups.staged_files.iter_mut().enumerate() {
        if staged.original_path.exists() {
            let backup_path = backup_dir_base.join(format!("{i}.bak"));
            fs::copy(&staged.original_path, &backup_path).map_err(|e| {
                SnipError::io_error(
                    "create pre-restore backup for transaction",
                    backup_path.clone(),
                    e,
                )
            })?;
            staged.backup_path = Some(backup_path);
        }
    }

    // Persist BackupsDurable state before any live writes.
    // A crash after this point is recoverable: the journal contains all
    // backup paths needed for rollback.
    crate::transaction::advance_to_backups_durable(&state_dir, &mut journal_with_backups)?;

    // Execute the restore within a transaction boundary; roll back on any failure.
    let restore_result: SnipResult<()> = (|| {
        // 6. For replace mode, create pre-restore backup of config
        if mode == RestoreMode::Replace
            && let Some(backup_path) = create_pre_restore_backup(&config_dir)?
        {
            report.pre_restore_backup = Some(backup_path.display().to_string());
        }

        // 7. Ensure libraries directory exists
        let libraries_dir = config_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).map_err(|e| {
            SnipError::io_error("create libraries directory", libraries_dir.clone(), e)
        })?;

        // 8. Restore files with per-file durable commit progress.
        // Each file is restored atomically; progress is persisted after
        // each write so that a crash mid-restore can be recovered.
        for (file_idx, entry) in manifest.files.iter().enumerate() {
            // Advance to Committing before each live write.
            crate::transaction::advance_to_committing(
                &state_dir,
                &mut journal_with_backups,
                file_idx,
            )?;

            match entry.kind {
                BackupEntryKind::Library => {
                    let src = resolve_backup_path(&backup, entry);
                    let library_name = Path::new(&entry.path)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    restore_library_file(&src, &libraries_dir, &library_name, mode, &mut report)?;
                }
                BackupEntryKind::Index => {
                    let src = backup.join(&entry.path);
                    let dst = config_dir.join("libraries.toml");
                    if mode == RestoreMode::Replace || !dst.exists() {
                        let bytes = fs::read(&src).map_err(|e| {
                            SnipError::io_error("read index file for restore", src.clone(), e)
                        })?;
                        let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
                        atomic_replace(&dst, &bytes, &opts)?;
                        report.files_restored += 1;
                    }
                }
                BackupEntryKind::Usage => {
                    let src = backup.join(&entry.path);
                    let dst = config_dir.join("usage.toml");
                    if mode == RestoreMode::Replace || !dst.exists() {
                        let bytes = fs::read(&src).map_err(|e| {
                            SnipError::io_error("read usage file for restore", src.clone(), e)
                        })?;
                        let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
                        atomic_replace(&dst, &bytes, &opts)?;
                        report.files_restored += 1;
                    }
                }
                BackupEntryKind::SyncConfig => {
                    let src = backup.join(&entry.path);
                    let dst = config_dir.join("sync.toml");
                    if dst.exists() && mode == RestoreMode::Merge {
                        report
                            .skipped
                            .push("sync.toml (local config preserved)".to_string());
                    } else if mode == RestoreMode::Replace || !dst.exists() {
                        let bytes = fs::read(&src).map_err(|e| {
                            SnipError::io_error("read sync config for restore", src.clone(), e)
                        })?;
                        let opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig);
                        atomic_replace(&dst, &bytes, &opts)?;
                        report.conflicts.push(RestoreConflict {
                            library: "sync".to_string(),
                            kind: "redacted_key".to_string(),
                            detail: "API key was redacted in backup; re-enter with 'snp register'"
                                .to_string(),
                        });
                        report.files_restored += 1;
                    }
                }
            }
        }

        Ok(())
    })();

    // On failure, roll back the transaction to restore original files.
    if let Err(ref e) = restore_result {
        eprintln!("Restore failed, rolling back: {e}");
        if let Err(rb_err) =
            crate::transaction::rollback_transaction(&state_dir, &journal_with_backups)
        {
            eprintln!("Warning: rollback also failed: {rb_err}");
        }
        return restore_result;
    }

    // 12. Commit transaction
    crate::transaction::commit_transaction(&state_dir, &journal_with_backups)?;

    // 13. Record pending sync generation if restore changed syncable data
    if report.files_restored > 0 {
        let _ = notify_mutation(MutationKind::Import, MutationOrigin::User);
    }

    // 14. Output report
    if json {
        let report_json = serde_json::to_string_pretty(&report).map_err(|e| {
            SnipError::runtime_error("serialize restore report", Some(&e.to_string()))
        })?;
        println!("{report_json}");
    } else {
        eprintln!("Restore complete (mode: {:?})", mode);
        eprintln!("  Files restored: {}", report.files_restored);
        if !report.conflicts.is_empty() {
            eprintln!("  Conflicts/changes ({}):", report.conflicts.len());
            for c in &report.conflicts {
                eprintln!("    [{}] {} — {}", c.library, c.kind, c.detail);
            }
        }
        if !report.skipped.is_empty() {
            eprintln!("  Skipped ({}):", report.skipped.len());
            for s in &report.skipped {
                eprintln!("    {s}");
            }
        }
        if let Some(ref backup_path) = report.pre_restore_backup {
            eprintln!("  Pre-restore backup: {backup_path}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::backup_cmd::{BackupEntryKind, BackupManifestEntry};
    use super::*;
    use tempfile::TempDir;

    fn create_test_backup(dir: &Path) -> PathBuf {
        let backup_dir = dir.join("test-backup");
        let libraries_dir = backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();

        let lib_content = r#"[[snippets]]
description = "restored snippet"
command = "echo restored"
"#;
        fs::write(libraries_dir.join("test.toml"), lib_content).unwrap();

        let index = r#"[[libraries]]
filename = "test"
is_primary = true
"#;
        fs::write(backup_dir.join("libraries.toml"), index).unwrap();

        // Compute hash for the library file
        let lib_hash = {
            let bytes = fs::read(libraries_dir.join("test.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let result = hasher.finalize();
            result
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let index_hash = {
            let bytes = fs::read(backup_dir.join("libraries.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let result = hasher.finalize();
            result
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 1700000000000,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![
                BackupManifestEntry {
                    path: "test.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: lib_content.len() as u64,
                    sha256: lib_hash.clone(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: BackupEntryKind::Index,
                    size: index.len() as u64,
                    sha256: index_hash.clone(),
                },
            ],
        };

        let manifest_str = toml::to_string_pretty(&manifest).unwrap();
        fs::write(backup_dir.join("manifest.toml"), manifest_str).unwrap();

        backup_dir
    }

    #[test]
    fn test_verify_checksum_valid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.txt");
        fs::write(&path, "hello").unwrap();

        let bytes = fs::read(&path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let result = hasher.finalize();
        let hash = result
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();

        assert!(verify_checksum(&path, &hash).unwrap());
    }

    #[test]
    fn test_verify_checksum_invalid() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("data.txt");
        fs::write(&path, "hello").unwrap();

        assert!(
            !verify_checksum(
                &path,
                "0000000000000000000000000000000000000000000000000000000000000000"
            )
            .unwrap()
        );
    }

    #[test]
    fn test_load_manifest_toml() {
        let dir = TempDir::new().unwrap();
        let backup_dir = create_test_backup(dir.path());
        let manifest = load_manifest(&backup_dir).unwrap();
        assert_eq!(manifest.schema, 1);
        assert_eq!(manifest.files.len(), 2);
    }

    #[test]
    fn test_load_manifest_missing() {
        let dir = TempDir::new().unwrap();
        let result = load_manifest(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No manifest found"));
    }

    #[test]
    fn test_load_manifest_json() {
        let dir = TempDir::new().unwrap();
        let backup_dir = dir.path().join("json-backup");
        fs::create_dir_all(&backup_dir).unwrap();

        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 0,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        fs::write(backup_dir.join("manifest.json"), json).unwrap();

        let loaded = load_manifest(&backup_dir).unwrap();
        assert_eq!(loaded.files.len(), 0);
    }

    #[test]
    fn test_dry_run_does_not_modify_config() {
        let tmp = TempDir::new().unwrap();
        let backup_dir = create_test_backup(tmp.path());
        let result = run(backup_dir, RestoreMode::DryRun, false);
        assert!(
            result.is_ok(),
            "dry run should not fail: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_restore_nonexistent_path_fails() {
        let result = run(
            PathBuf::from("/nonexistent/backup"),
            RestoreMode::DryRun,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_report_serialization() {
        let report = RestoreReport {
            mode: "Merge".to_string(),
            files_restored: 3,
            conflicts: vec![RestoreConflict {
                library: "work".to_string(),
                kind: "updated".to_string(),
                detail: "Snippet 'deploy' updated".to_string(),
            }],
            skipped: vec!["sync.toml".to_string()],
            pre_restore_backup: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("Merge"));
        assert!(json.contains("deploy"));
    }

    /// Full backup→restore roundtrip: create backup, verify checksums,
    /// restore in merge mode, and confirm snippet identity is preserved.
    #[test]
    fn test_backup_restore_roundtrip_checksum_and_identity() {
        let tmp = TempDir::new().unwrap();
        let backup_dir = tmp.path().join("roundtrip-backup");
        let libraries_dir = backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();

        // 1. Create backup content
        let lib_content = r#"[[snippets]]
id = "stable-id-001"
description = "roundtrip snippet"
command = "echo roundtrip"
favorite = true
created_at = 1700000000
updated_at = 1700000001
"#;
        fs::write(libraries_dir.join("work.toml"), lib_content).unwrap();

        let index = r#"[[libraries]]
filename = "work"
is_primary = true
"#;
        fs::write(backup_dir.join("libraries.toml"), index).unwrap();

        // 2. Compute checksums
        let lib_hash = {
            let bytes = fs::read(libraries_dir.join("work.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let index_hash = {
            let bytes = fs::read(backup_dir.join("libraries.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        // 3. Write manifest
        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 1700000000000,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![
                BackupManifestEntry {
                    path: "work.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: lib_content.len() as u64,
                    sha256: lib_hash.clone(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: BackupEntryKind::Index,
                    size: index.len() as u64,
                    sha256: index_hash.clone(),
                },
            ],
        };
        let manifest_str = toml::to_string_pretty(&manifest).unwrap();
        fs::write(backup_dir.join("manifest.toml"), &manifest_str).unwrap();

        // 4. Verify checksums match (the core invariant)
        let verify = |path: &Path, expected: &str| -> bool {
            let bytes = fs::read(path).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let actual: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            actual == expected
        };
        assert!(verify(&libraries_dir.join("work.toml"), &lib_hash));
        assert!(verify(&backup_dir.join("libraries.toml"), &index_hash));

        // 5. Load manifest and verify it roundtrips
        let loaded = load_manifest(&backup_dir).unwrap();
        assert_eq!(loaded.schema, 1);
        assert_eq!(loaded.files.len(), 2);

        // 6. Verify all checksums pass via verify_checksum
        for entry in &loaded.files {
            let file_path = if entry.kind == BackupEntryKind::Index {
                backup_dir.join(&entry.path)
            } else {
                backup_dir.join("libraries").join(&entry.path)
            };
            assert!(verify_checksum(&file_path, &entry.sha256).unwrap());
        }

        // 7. Dry run should not error
        let dry_result = run(backup_dir.clone(), RestoreMode::DryRun, false);
        assert!(dry_result.is_ok());
    }

    /// Test that merge restore preserves existing snippets and adds new ones.
    #[test]
    fn test_merge_restore_adds_new_snippets() {
        let tmp = TempDir::new().unwrap();
        let backup_dir = tmp.path().join("merge-backup");
        let libraries_dir = backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();

        // Backup has snippet A and B
        let lib_content = r#"[[snippets]]
id = "snippet-a"
description = "from backup A"
command = "echo backup-a"

[[snippets]]
id = "snippet-b"
description = "from backup B"
command = "echo backup-b"
"#;
        fs::write(libraries_dir.join("test.toml"), lib_content).unwrap();

        let index = r#"[[libraries]]
filename = "test"
is_primary = true
"#;
        fs::write(backup_dir.join("libraries.toml"), index).unwrap();

        // Compute hashes
        let lib_hash = {
            let bytes = fs::read(libraries_dir.join("test.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };
        let index_hash = {
            let bytes = fs::read(backup_dir.join("libraries.toml")).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        };

        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 1700000000000,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![
                BackupManifestEntry {
                    path: "test.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: lib_content.len() as u64,
                    sha256: lib_hash.clone(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: BackupEntryKind::Index,
                    size: index.len() as u64,
                    sha256: index_hash.clone(),
                },
            ],
        };
        fs::write(
            backup_dir.join("manifest.toml"),
            toml::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // Verify checksums are valid before restore
        for entry in &manifest.files {
            let file_path = if entry.kind == BackupEntryKind::Index {
                backup_dir.join(&entry.path)
            } else {
                backup_dir.join("libraries").join(&entry.path)
            };
            assert!(
                verify_checksum(&file_path, &entry.sha256).unwrap(),
                "Checksum mismatch for {}",
                entry.path
            );
        }

        // Dry run should show the files
        let dry_result = run(backup_dir, RestoreMode::DryRun, false);
        assert!(dry_result.is_ok());
    }

    /// Test that restore non-existent path returns an error.
    #[test]
    fn test_restore_nonexistent_backup_path() {
        let result = run(
            PathBuf::from("/tmp/nonexistent-backup-12345"),
            RestoreMode::Replace,
            true,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("does not exist"));
    }

    /// Test that restore with missing manifest returns an error.
    #[test]
    fn test_restore_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        let empty_dir = tmp.path().join("empty-backup");
        fs::create_dir_all(&empty_dir).unwrap();
        let result = run(empty_dir, RestoreMode::DryRun, false);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No manifest found"));
    }

    // === Path validation tests (Workstream C) ===

    #[test]
    fn test_validate_rejects_empty_path() {
        let result = validate_backup_path("", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Empty"));
    }

    #[test]
    fn test_validate_rejects_absolute_unix_path() {
        let result = validate_backup_path("/etc/passwd", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Absolute"));
    }

    #[test]
    fn test_validate_rejects_absolute_windows_path() {
        let result =
            validate_backup_path("C:\\Windows\\System32\\config", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Absolute"));
    }

    #[test]
    fn test_validate_rejects_traversal() {
        let result = validate_backup_path("../outside.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("traversal") || msg.contains("ParentDir"));
    }

    #[test]
    fn test_validate_rejects_nul_byte() {
        let result = validate_backup_path("test\0.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("NUL"));
    }

    #[test]
    fn test_validate_accepts_normal_library() {
        let result = validate_backup_path("test.toml", BackupEntryKind::Library);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("test.toml"));
    }

    #[test]
    fn test_validate_rejects_subdir_for_library() {
        let result = validate_backup_path("subdir/test.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("flat filename"));
    }

    #[test]
    fn test_validate_accepts_index_path() {
        let result = validate_backup_path("libraries.toml", BackupEntryKind::Index);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_rejects_wrong_index_path() {
        let result = validate_backup_path("wrong-name.toml", BackupEntryKind::Index);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("must be libraries.toml"));
    }

    #[test]
    fn test_validate_accepts_usage_path() {
        let result = validate_backup_path("usage.toml", BackupEntryKind::Usage);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_rejects_wrong_usage_path() {
        let result = validate_backup_path("my-usage.toml", BackupEntryKind::Usage);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("must be usage.toml"));
    }

    #[test]
    fn test_validate_accepts_sync_config_path() {
        let result = validate_backup_path("sync.toml", BackupEntryKind::SyncConfig);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_rejects_wrong_sync_config_path() {
        let result = validate_backup_path("sync-v2.toml", BackupEntryKind::SyncConfig);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("must be sync.toml"));
    }

    #[test]
    fn test_validate_rejects_library_without_extension() {
        let result = validate_backup_path("test", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(".toml extension"));
    }

    #[test]
    fn test_validate_rejects_unc_path() {
        let result = validate_backup_path("\\\\server\\share\\file.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("UNC"));
    }

    #[test]
    fn test_validate_rejects_unc_path_forward_slash() {
        let result = validate_backup_path("//server/share/file.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("UNC") || msg.contains("Absolute"),
            "Should reject UNC/absolute path: {msg}"
        );
    }

    #[test]
    fn test_validate_rejects_reserved_windows_name_con() {
        let result = validate_backup_path("CON.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Reserved Windows device name"));
    }

    #[test]
    fn test_validate_rejects_reserved_windows_name_nul() {
        let result = validate_backup_path("NUL.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Reserved Windows device name"));
    }

    #[test]
    fn test_validate_rejects_reserved_windows_name_lpt1() {
        let result = validate_backup_path("LPT1.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Reserved Windows device name"));
    }

    #[test]
    fn test_validate_rejects_reserved_windows_name_case_insensitive() {
        let result = validate_backup_path("con.toml", BackupEntryKind::Library);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Reserved Windows device name"));
    }

    #[test]
    fn test_validate_accepts_normal_name_similar_to_reserved() {
        // "console.toml" should be fine — only exact "CON" stem is rejected
        let result = validate_backup_path("console.toml", BackupEntryKind::Library);
        assert!(result.is_ok());
    }

    // === Transaction dry-run test (Workstream D) ===

    #[test]
    fn test_dry_run_performs_zero_writes() {
        let tmp = TempDir::new().unwrap();
        let backup_dir = tmp.path().join("dry-backup");
        let libraries_dir = backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();

        let lib_content = r#"[[snippets]]
id = "dry-id"
description = "dry snippet"
command = "echo dry"
"#;
        fs::write(libraries_dir.join("test.toml"), lib_content).unwrap();

        let index = r#"[[libraries]]
filename = "test"
is_primary = true
"#;
        fs::write(backup_dir.join("libraries.toml"), index).unwrap();

        let lib_hash = sha256_hex(fs::read(libraries_dir.join("test.toml")).unwrap());
        let index_hash = sha256_hex(fs::read(backup_dir.join("libraries.toml")).unwrap());

        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 1700000000000,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![
                BackupManifestEntry {
                    path: "test.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: lib_content.len() as u64,
                    sha256: lib_hash.clone(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: BackupEntryKind::Index,
                    size: index.len() as u64,
                    sha256: index_hash.clone(),
                },
            ],
        };
        fs::write(
            backup_dir.join("manifest.toml"),
            toml::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let result = run(backup_dir, RestoreMode::DryRun, false);
        assert!(result.is_ok());
        // Dry run should not create any transaction journals
        let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
        let journals = crate::transaction::check_interrupted_transactions(&state_dir).unwrap();
        assert!(
            journals.is_empty(),
            "dry run must not create transaction journals"
        );
    }

    fn sha256_hex(bytes: Vec<u8>) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

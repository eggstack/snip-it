//! **Layer: Application**
//!
//! `snp backup` command — create a secret-free backup snapshot.

pub use super::backup_archive::{
    BackupEntryKind, BackupFormat, BackupManifest, BackupManifestEntry, BackupRelativePath,
};
use super::backup_archive::{
    atomic_write_backup, read_for_snapshot, read_library_generation, redact_sync_config,
    sha256_hex, validate_canonical_containment, validate_toml_content,
};
use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_config_dir;
use std::fs;
use std::path::PathBuf;

/// Canonical Clap arguments for `snp backup` and `snp data backup`.
///
/// Single source of truth for both spellings (`data backup` uses
/// `alias = "b"`). Field lists must not be duplicated in `main.rs`.
#[derive(Debug, Clone, clap::Args)]
pub struct BackupArgs {
    /// Output directory (default: ~/.config/snp/backups/\{timestamp\}/)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Include usage metadata in backup
    #[arg(long)]
    pub include_usage: bool,

    /// Include sync.toml in backup (API key redacted)
    #[arg(long)]
    pub include_sync_state: bool,
    /// Backup format
    #[arg(long, value_enum, default_value = "directory")]
    pub format: BackupFormat,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

/// Run backup.
pub fn run(
    output: Option<PathBuf>,
    include_usage: bool,
    include_sync_state: bool,
    format: BackupFormat,
    json: bool,
) -> SnipResult<()> {
    let _ = format;
    let config_dir = get_config_dir();
    if !config_dir.exists() {
        return Err(SnipError::runtime_error(
            "No snp config directory found",
            Some(&format!("Expected config at {}", config_dir.display())),
        ));
    }
    let canonical_config = config_dir
        .canonicalize()
        .map_err(|e| SnipError::io_error("canonicalize config directory", config_dir.clone(), e))?;
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let final_backup_dir = match output {
        Some(ref path) => path.clone(),
        None => config_dir.join("backups").join(&timestamp),
    };
    let staging_dir = final_backup_dir.with_extension(format!(
        "{}.staging.{}",
        final_backup_dir
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or(""),
        uuid::Uuid::new_v4()
    ));

    // Acquire the local-data lock for the duration of snapshot capture.
    // This ensures the backup captures either the complete before-state or
    // complete after-state of all local data, never a mixed state.
    let state_dir = crate::local_data::transaction_dir();
    let _local_lock = crate::local_data::acquire_local_data_lock(&state_dir)?;
    let generation_before = read_library_generation(&config_dir)?;
    let mut manifest = BackupManifest {
        schema: 1,
        created_at_unix_ms: chrono::Utc::now().timestamp_millis(),
        snip_it_version: crate::diagnostics::version().to_string(),
        layout: "directory".to_string(),
        files: Vec::new(),
    };
    let mut snapshot: Vec<(PathBuf, String, Vec<u8>)> = Vec::new();
    let libraries_dir = config_dir.join("libraries");
    if libraries_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&libraries_dir)
            .map_err(|e| SnipError::io_error("read libraries directory", libraries_dir.clone(), e))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.ends_with(".toml") && !name.starts_with('.')
            })
            .collect();
        for entry in &entries {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let src = entry.path();
            validate_canonical_containment(&src, &canonical_config, "library file")?;
            let bytes = read_for_snapshot(&src, "library")?;
            snapshot.push((src, file_name, bytes));
        }
    }
    let libraries_toml = config_dir.join("libraries.toml");
    if libraries_toml.exists() {
        validate_canonical_containment(&libraries_toml, &canonical_config, "libraries index")?;
        let bytes = read_for_snapshot(&libraries_toml, "libraries index")?;
        snapshot.push((libraries_toml, "libraries.toml".to_string(), bytes));
    }
    if include_usage {
        let usage_path = config_dir.join("usage.toml");
        if usage_path.exists() {
            validate_canonical_containment(&usage_path, &canonical_config, "usage")?;
            let bytes = read_for_snapshot(&usage_path, "usage")?;
            snapshot.push((usage_path, "usage.toml".to_string(), bytes));
        }
    }
    let sync_snapshot = if include_sync_state {
        let sync_path = config_dir.join("sync.toml");
        if sync_path.exists() {
            validate_canonical_containment(&sync_path, &canonical_config, "sync config")?;
            let content = fs::read_to_string(&sync_path).map_err(|e| {
                SnipError::io_error("read sync config for snapshot", sync_path.clone(), e)
            })?;
            Some(content)
        } else {
            None
        }
    } else {
        None
    };
    let generation_after = read_library_generation(&config_dir)?;
    if generation_before != generation_after {
        return Err(SnipError::runtime_error(
            "Library generation changed during snapshot",
            Some(&format!(
                "generation before: {generation_before}, after: {generation_after}. \
                 A concurrent mutation occurred; retry the backup."
            )),
        ));
    }
    for (_src, name, bytes) in &snapshot {
        if _src.extension().is_some_and(|e| e == "toml")
            && _src.parent().is_some_and(|p| p == libraries_dir)
        {
            validate_toml_content(bytes, &format!("library file '{name}'"))?;
        }
    }
    for (_src, name, bytes) in &snapshot {
        if name == "libraries.toml" {
            validate_toml_content(bytes, "libraries index 'libraries.toml'")?;
        }
    }
    let mut backup_files: Vec<(String, String, Vec<u8>)> = Vec::new();
    for (_src, file_name, bytes) in &snapshot {
        if _src.extension().is_some_and(|e| e == "toml")
            && _src.parent().is_some_and(|p| p == libraries_dir)
        {
            let sha = sha256_hex(bytes);
            manifest.files.push(BackupManifestEntry {
                path: format!("libraries/{file_name}"),
                kind: BackupEntryKind::Library,
                size: bytes.len() as u64,
                sha256: sha,
            });
            backup_files.push((
                format!("libraries/{file_name}"),
                BackupEntryKind::Library.to_string(),
                bytes.clone(),
            ));
        }
    }
    for (_src, name, bytes) in &snapshot {
        if name == "libraries.toml" {
            let sha = sha256_hex(bytes);
            manifest.files.push(BackupManifestEntry {
                path: "libraries.toml".to_string(),
                kind: BackupEntryKind::Index,
                size: bytes.len() as u64,
                sha256: sha,
            });
            backup_files.push((
                "libraries.toml".to_string(),
                BackupEntryKind::Index.to_string(),
                bytes.clone(),
            ));
        }
    }
    for (_src, name, bytes) in &snapshot {
        if name == "usage.toml" {
            let sha = sha256_hex(bytes);
            manifest.files.push(BackupManifestEntry {
                path: "usage.toml".to_string(),
                kind: BackupEntryKind::Usage,
                size: bytes.len() as u64,
                sha256: sha,
            });
            backup_files.push((
                "usage.toml".to_string(),
                BackupEntryKind::Usage.to_string(),
                bytes.clone(),
            ));
        }
    }
    if let Some(content) = sync_snapshot {
        let redacted = redact_sync_config(&content)?;
        let redacted_bytes = redacted.into_bytes();
        let sha = sha256_hex(&redacted_bytes);
        manifest.files.push(BackupManifestEntry {
            path: "sync.toml".to_string(),
            kind: BackupEntryKind::SyncConfig,
            size: redacted_bytes.len() as u64,
            sha256: sha,
        });
        backup_files.push((
            "sync.toml".to_string(),
            BackupEntryKind::SyncConfig.to_string(),
            redacted_bytes,
        ));
    }
    manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
    let file_refs: Vec<(&str, &str, &[u8])> = backup_files
        .iter()
        .map(|(path, kind, bytes)| (path.as_str(), kind.as_str(), bytes.as_slice()))
        .collect();
    atomic_write_backup(&staging_dir, &final_backup_dir, &file_refs, &manifest)?;
    if json {
        let report = serde_json::json!({
            "backup_dir": final_backup_dir.display().to_string(),
            "schema": manifest.schema,
            "version": manifest.snip_it_version,
            "file_count": manifest.files.len(),
            "total_bytes": manifest.files.iter().map(|f| f.size).sum::<u64>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| SnipError::runtime_error("serialize report", Some(&e.to_string())))?
        );
    } else {
        eprintln!("Backup created: {}", final_backup_dir.display());
        eprintln!("  Schema: {}", manifest.schema);
        eprintln!("  Version: {}", manifest.snip_it_version);
        eprintln!("  Files: {}", manifest.files.len());
        let total_bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
        eprintln!("  Total size: {} bytes", total_bytes);
        for entry in &manifest.files {
            eprintln!(
                "    {} ({}): {} bytes, sha256:{}",
                entry.kind,
                entry.path,
                entry.size,
                &entry.sha256[..16]
            );
        }
    }
    Ok(())
}

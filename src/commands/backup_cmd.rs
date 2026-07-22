//! **Layer: Application**
//!
//! `snp backup` command — create a secret-free backup snapshot.

use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_config_dir;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Backup format (currently only directory layout is supported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BackupFormat {
    Directory,
}

/// Known kinds of entries in a backup manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupEntryKind {
    Library,
    Index,
    Usage,
    SyncConfig,
}

impl BackupEntryKind {
    /// Parse a kind string into a typed variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "library" => Some(Self::Library),
            "index" => Some(Self::Index),
            "usage" => Some(Self::Usage),
            "sync_config" => Some(Self::SyncConfig),
            _ => None,
        }
    }
}

/// A validated path relative to the backup root.
///
/// Rejects absolute paths, parent traversal, NUL bytes, and reserved
/// Windows device names. Used to prevent path-traversal attacks via
/// malicious backup manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRelativePath(PathBuf);

impl BackupRelativePath {
    /// Reserved Windows device names that must not appear as file components.
    const RESERVED_WINDOWS_NAMES: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    /// Parse and validate a backup-relative path.
    pub fn parse(input: &str, expected_kind: BackupEntryKind) -> SnipResult<Self> {
        if input.is_empty() {
            return Err(SnipError::runtime_error(
                "Empty backup path",
                Some("Backup manifest entry path must not be empty"),
            ));
        }

        // Reject NUL bytes
        if input.contains('\0') {
            return Err(SnipError::runtime_error(
                "Path contains NUL byte",
                Some(&format!("Backup path contains NUL: {input}")),
            ));
        }

        let path = Path::new(input);

        // Reject absolute paths
        if path.is_absolute() {
            return Err(SnipError::runtime_error(
                "Absolute path in backup manifest",
                Some(&format!("Path must be relative: {input}")),
            ));
        }

        // Reject Windows drive letters and UNC paths
        #[cfg(windows)]
        {
            use std::path::Component;
            for component in path.components() {
                if let Component::Prefix(p) = component {
                    return Err(SnipError::runtime_error(
                        "Windows prefix in backup path",
                        Some(&format!("Path contains Windows prefix: {input}")),
                    ));
                }
            }
        }

        // Reject traversal components
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    return Err(SnipError::runtime_error(
                        "Path traversal in backup manifest",
                        Some(&format!("Parent traversal rejected: {input}")),
                    ));
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(SnipError::runtime_error(
                        "Absolute path component in backup manifest",
                        Some(&format!("Absolute component rejected: {input}")),
                    ));
                }
                _ => {}
            }
        }

        // Kind-specific constraints
        match expected_kind {
            BackupEntryKind::Library => {
                // Must be a flat filename (no directory components), ending in .toml
                if path.components().count() > 1 {
                    return Err(SnipError::runtime_error(
                        "Library entry must be a flat filename",
                        Some(&format!("Nested path rejected for library: {input}")),
                    ));
                }
                if !input.ends_with(".toml") {
                    return Err(SnipError::runtime_error(
                        "Library entry must end with .toml",
                        Some(&format!("Missing .toml extension: {input}")),
                    ));
                }
                // Check reserved Windows names
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                for reserved in Self::RESERVED_WINDOWS_NAMES {
                    if stem.eq_ignore_ascii_case(reserved) {
                        return Err(SnipError::runtime_error(
                            "Reserved Windows device name",
                            Some(&format!("Filename '{stem}' is reserved on Windows")),
                        ));
                    }
                }
            }
            BackupEntryKind::Index => {
                if input != "libraries.toml" {
                    return Err(SnipError::runtime_error(
                        "Index entry must be libraries.toml",
                        Some(&format!("Unexpected index path: {input}")),
                    ));
                }
            }
            BackupEntryKind::Usage => {
                if input != "usage.toml" {
                    return Err(SnipError::runtime_error(
                        "Usage entry must be usage.toml",
                        Some(&format!("Unexpected usage path: {input}")),
                    ));
                }
            }
            BackupEntryKind::SyncConfig => {
                if input != "sync.toml" {
                    return Err(SnipError::runtime_error(
                        "Sync config entry must be sync.toml",
                        Some(&format!("Unexpected sync config path: {input}")),
                    ));
                }
            }
        }

        Ok(Self(path.to_path_buf()))
    }

    /// Resolve the path relative to a backup root directory.
    pub fn resolve_source(&self, backup_root: &Path) -> PathBuf {
        backup_root.join(&self.0)
    }
}

/// Read a file for backup snapshot, rejecting symlinks and non-regular files.
fn read_for_snapshot(path: &Path, label: &str) -> SnipResult<Vec<u8>> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| SnipError::io_error(&format!("stat {label} for snapshot"), path, e))?;
    if meta.file_type().is_symlink() {
        return Err(SnipError::runtime_error(
            "Backup entry is a symlink",
            Some(&format!(
                "Refusing to back up symlink {} (could point outside config)",
                path.display()
            )),
        ));
    }
    if !meta.is_file() {
        return Err(SnipError::runtime_error(
            "Backup entry is not a regular file",
            Some(&format!(
                "Expected regular file, got {:?}: {}",
                meta.file_type(),
                path.display()
            )),
        ));
    }
    fs::read(path).map_err(|e| SnipError::io_error(&format!("read {label} for snapshot"), path, e))
}

/// Backup manifest entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifestEntry {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub sha256: String,
}

/// Complete backup manifest.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    pub schema: u32,
    pub created_at_unix_ms: i64,
    pub snip_it_version: String,
    pub layout: String,
    pub files: Vec<BackupManifestEntry>,
}

/// Run backup.
pub fn run(
    output: Option<PathBuf>,
    include_usage: bool,
    include_config: bool,
    include_sync_state: bool,
    format: BackupFormat,
    json: bool,
) -> SnipResult<()> {
    let _ = format; // Only Directory variant exists; flag kept for CLI compatibility
    let config_dir = get_config_dir();
    if !config_dir.exists() {
        return Err(SnipError::runtime_error(
            "No snp config directory found",
            Some(&format!("Expected config at {}", config_dir.display())),
        ));
    }

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_dir = match output {
        Some(ref path) => {
            fs::create_dir_all(path).map_err(|e| {
                SnipError::io_error("create backup output directory", path.clone(), e)
            })?;
            path.clone()
        }
        None => {
            let default_base = config_dir.join("backups").join(&timestamp);
            fs::create_dir_all(&default_base).map_err(|e| {
                SnipError::io_error("create backup directory", default_base.clone(), e)
            })?;
            default_base
        }
    };

    let mut manifest = BackupManifest {
        schema: 1,
        created_at_unix_ms: chrono::Utc::now().timestamp_millis(),
        snip_it_version: crate::diagnostics::version().to_string(),
        layout: "directory".to_string(),
        files: Vec::new(),
    };

    // Take a consistent in-memory snapshot of all source files before copying.
    // This prevents a concurrent mutation from producing an inconsistent backup
    // (e.g., index referencing a snippet that was deleted mid-copy).
    let mut snapshot: Vec<(PathBuf, String, Vec<u8>)> = Vec::new();

    // 1. Snapshot all library files
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
            let bytes = read_for_snapshot(&src, "library")?;
            snapshot.push((src, file_name, bytes));
        }
    }

    // 2. Snapshot libraries.toml index
    let libraries_toml = config_dir.join("libraries.toml");
    if libraries_toml.exists() {
        let bytes = read_for_snapshot(&libraries_toml, "libraries index")?;
        snapshot.push((libraries_toml, "libraries.toml".to_string(), bytes));
    }

    // 3. Snapshot usage.toml if requested
    if include_usage {
        let usage_path = config_dir.join("usage.toml");
        if usage_path.exists() {
            let bytes = read_for_snapshot(&usage_path, "usage")?;
            snapshot.push((usage_path, "usage.toml".to_string(), bytes));
        }
    }

    // 4. Snapshot sync.toml if requested (for redaction)
    let sync_snapshot = if include_sync_state {
        let sync_path = config_dir.join("sync.toml");
        if sync_path.exists() {
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

    // 5. Snapshot general config files if requested
    let config_snapshot: Vec<(PathBuf, String, Vec<u8>)> = if include_config {
        let handled_files: HashSet<&str> = [
            "libraries.toml",
            "usage.toml",
            "sync.toml",
            "themes.toml",
            "auto-sync-status.toml",
        ]
        .into_iter()
        .collect();

        let excluded_dirs: HashSet<&str> = [
            "libraries",
            "premade",
            "themes",
            "backups",
            "transaction-journals",
        ]
        .into_iter()
        .collect();

        let mut result = Vec::new();
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if excluded_dirs.contains(name.as_str())
                    || name.starts_with('.')
                    || name.starts_with("manifest.")
                    || handled_files.contains(name.as_str())
                {
                    continue;
                }
                if !name.ends_with(".toml") {
                    continue;
                }
                let src = entry.path();
                if src.is_file() {
                    let bytes = read_for_snapshot(&src, &name)?;
                    result.push((src, name, bytes));
                }
            }
        }
        result
    } else {
        Vec::new()
    };

    // === Write from snapshot to backup directory ===

    // 1. Write library files from snapshot
    for (_src, file_name, bytes) in &snapshot {
        if _src.extension().is_some_and(|e| e == "toml")
            && _src.parent().is_some_and(|p| p == libraries_dir)
        {
            let dst = backup_dir.join("libraries").join(file_name);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| SnipError::io_error("create backup subdirectory", parent, e))?;
            }
            fs::write(&dst, bytes)
                .map_err(|e| SnipError::io_error("write library to backup", dst.clone(), e))?;
            let sha = {
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect()
            };
            manifest.files.push(BackupManifestEntry {
                path: file_name.clone(),
                kind: "library".to_string(),
                size: bytes.len() as u64,
                sha256: sha,
            });
        }
    }

    // 2. Write index from snapshot
    for (_src, name, bytes) in &snapshot {
        if name == "libraries.toml" {
            let dst = backup_dir.join("libraries.toml");
            fs::write(&dst, bytes)
                .map_err(|e| SnipError::io_error("write index to backup", dst.clone(), e))?;
            let sha = {
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect()
            };
            manifest.files.push(BackupManifestEntry {
                path: "libraries.toml".to_string(),
                kind: "index".to_string(),
                size: bytes.len() as u64,
                sha256: sha,
            });
        }
    }

    // 3. Write usage from snapshot
    for (_src, name, bytes) in &snapshot {
        if name == "usage.toml" {
            let dst = backup_dir.join("usage.toml");
            fs::write(&dst, bytes)
                .map_err(|e| SnipError::io_error("write usage to backup", dst.clone(), e))?;
            let sha = {
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                hasher
                    .finalize()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect()
            };
            manifest.files.push(BackupManifestEntry {
                path: "usage.toml".to_string(),
                kind: "usage".to_string(),
                size: bytes.len() as u64,
                sha256: sha,
            });
        }
    }

    // 4. Write redacted sync.toml if snapshotted
    if let Some(content) = sync_snapshot {
        let redacted = redact_sync_config(&content)?;
        let dst = backup_dir.join("sync.toml");
        crate::utils::atomic::write_private_atomic(&dst, &redacted, "sync")?;
        let sha = {
            let mut hasher = Sha256::new();
            hasher.update(redacted.as_bytes());
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect()
        };
        manifest.files.push(BackupManifestEntry {
            path: "sync.toml".to_string(),
            kind: "sync_config".to_string(),
            size: redacted.len() as u64,
            sha256: sha,
        });
    }

    // 5. Write config files from snapshot
    for (_src, name, bytes) in &config_snapshot {
        let dst = backup_dir.join(name);
        fs::write(&dst, bytes)
            .map_err(|e| SnipError::io_error("write config to backup", dst.clone(), e))?;
        let sha = {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect()
        };
        manifest.files.push(BackupManifestEntry {
            path: name.clone(),
            kind: "config".to_string(),
            size: bytes.len() as u64,
            sha256: sha,
        });
    }

    // 6. Write manifest
    let manifest_path = backup_dir.join("manifest.toml");
    let manifest_str = toml::to_string_pretty(&manifest)
        .map_err(|e| SnipError::toml_error("serialize backup manifest", e))?;
    crate::utils::atomic::write_private_atomic(&manifest_path, &manifest_str, "manifest")?;

    // 7. Output report
    if json {
        let report = serde_json::json!({
            "backup_dir": backup_dir.display().to_string(),
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
        eprintln!("Backup created: {}", backup_dir.display());
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

/// Redact API key from sync.toml content for safe backup.
fn redact_sync_config(content: &str) -> SnipResult<String> {
    let mut result = String::with_capacity(content.len());
    for line in content.lines() {
        let trimmed = line.trim();
        if (trimmed.starts_with("api_key")
            || trimmed.starts_with("ApiKey")
            || trimmed.starts_with("api-key"))
            && let Some(eq_pos) = trimmed.find('=')
        {
            let key_part = &trimmed[..eq_pos];
            result.push_str(&format!("{} = \"<redacted>\"\n", key_part.trim()));
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_redact_sync_config() {
        let input = r#"enabled = true
server_url = "https://sync.example.com"
api_key = "sk-secret-key-12345"
"#;
        let redacted = redact_sync_config(input).unwrap();
        assert!(!redacted.contains("sk-secret-key-12345"));
        assert!(redacted.contains("<redacted>"));
        assert!(redacted.contains("server_url"));
    }

    #[test]
    fn test_redact_sync_config_preserves_other_keys() {
        let input = r#"enabled = true
server_url = "https://sync.example.com"
api_key = "sk-secret"
timeout = 30
"#;
        let redacted = redact_sync_config(input).unwrap();
        assert!(redacted.contains("timeout = 30"));
        assert!(redacted.contains("server_url"));
    }

    #[test]
    fn test_backup_format_is_directory() {
        assert_eq!(BackupFormat::Directory, BackupFormat::Directory);
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 1700000000000,
            snip_it_version: "1.0.0".to_string(),
            layout: "directory".to_string(),
            files: vec![BackupManifestEntry {
                path: "lib.toml".to_string(),
                kind: "library".to_string(),
                size: 100,
                sha256: "abc123".to_string(),
            }],
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        let restored: BackupManifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.schema, 1);
        assert_eq!(restored.files.len(), 1);
        assert_eq!(restored.files[0].kind, "library");

        let json_str = serde_json::to_string(&manifest).unwrap();
        let restored_json: BackupManifest = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored_json.schema, 1);
    }

    #[cfg(unix)]
    #[test]
    fn test_read_for_snapshot_rejects_symlink() {
        let dir = TempDir::new().unwrap();
        let real_file = dir.path().join("real.toml");
        fs::write(&real_file, "[[snippets]]\nid = \"test\"\n").unwrap();
        let symlink_file = dir.path().join("linked.toml");
        std::os::unix::fs::symlink(&real_file, &symlink_file).unwrap();

        let result = read_for_snapshot(&symlink_file, "test");
        assert!(result.is_err(), "should reject symlink");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("symlink"), "Expected symlink error: {msg}");
    }

    #[cfg(unix)]
    #[test]
    fn test_read_for_snapshot_rejects_directory() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        let result = read_for_snapshot(&subdir, "test");
        assert!(result.is_err(), "should reject directory");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("regular file"),
            "Expected regular file error: {msg}"
        );
    }

    #[test]
    fn test_read_for_snapshot_accepts_regular_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.toml");
        fs::write(&file, "content").unwrap();

        let result = read_for_snapshot(&file, "test");
        assert!(result.is_ok(), "should accept regular file");
        assert_eq!(result.unwrap(), b"content");
    }
}

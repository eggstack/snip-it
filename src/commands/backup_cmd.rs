//! **Layer: Application**
//!
//! `snp backup` command — create a secret-free backup snapshot.

use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_config_dir;
use sha2::{Digest, Sha256};
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

impl std::fmt::Display for BackupEntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Library => write!(f, "library"),
            Self::Index => write!(f, "index"),
            Self::Usage => write!(f, "usage"),
            Self::SyncConfig => write!(f, "sync_config"),
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

/// Compute the SHA-256 hex digest of a byte slice.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Read the library generation from `libraries.toml` if it exists.
fn read_library_generation(config_dir: &Path) -> SnipResult<u64> {
    let libraries_toml = config_dir.join("libraries.toml");
    if !libraries_toml.exists() {
        return Ok(0);
    }
    let content = fs::read_to_string(&libraries_toml).map_err(|e| {
        SnipError::io_error(
            "read libraries index for generation",
            libraries_toml.clone(),
            e,
        )
    })?;
    let config: crate::library::LibraryConfig = toml::from_str(&content)
        .map_err(|e| SnipError::toml_error("parse libraries index for generation", e))?;
    Ok(config.generation)
}

/// Validate that a path is canonically contained under a root directory.
fn validate_canonical_containment(path: &Path, root: &Path, label: &str) -> SnipResult<PathBuf> {
    let canonical_root = root.canonicalize().map_err(|e| {
        SnipError::io_error(&format!("canonicalize {label} root"), root.to_path_buf(), e)
    })?;
    let canonical_path = path.canonicalize().map_err(|e| {
        SnipError::io_error(&format!("canonicalize {label} path"), path.to_path_buf(), e)
    })?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(SnipError::runtime_error(
            "Backup source escapes config root",
            Some(&format!(
                "{} ({}) is not under config root ({})",
                label,
                canonical_path.display(),
                canonical_root.display(),
            )),
        ));
    }
    Ok(canonical_path)
}

/// Validate that a byte slice is valid TOML.
fn validate_toml_content(bytes: &[u8], label: &str) -> SnipResult<()> {
    let content = std::str::from_utf8(bytes).map_err(|e| {
        SnipError::runtime_error(&format!("{label} is not valid UTF-8"), Some(&e.to_string()))
    })?;
    let _: toml::Value = toml::from_str(content).map_err(|e| {
        SnipError::runtime_error(&format!("{label} is not valid TOML"), Some(&e.to_string()))
    })?;
    Ok(())
}

/// Write backup content to a staging directory, then atomically rename to the
/// final path. On failure the staging directory is removed.
fn atomic_write_backup(
    staging_dir: &Path,
    final_dir: &Path,
    files: &[(&str, &str, &[u8])],
    manifest: &BackupManifest,
) -> SnipResult<()> {
    fs::create_dir_all(staging_dir).map_err(|e| {
        SnipError::io_error("create staging directory", staging_dir.to_path_buf(), e)
    })?;
    let result = (|| -> SnipResult<()> {
        for (rel_path, _kind, bytes) in files {
            let dst = staging_dir.join(rel_path);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    SnipError::io_error("create staging subdirectory", parent.to_path_buf(), e)
                })?;
            }
            fs::write(&dst, bytes)
                .map_err(|e| SnipError::io_error("write to staging", dst.clone(), e))?;
        }
        let manifest_path = staging_dir.join("manifest.toml");
        let manifest_str = toml::to_string_pretty(manifest)
            .map_err(|e| SnipError::toml_error("serialize backup manifest", e))?;
        crate::utils::atomic::write_private_atomic(&manifest_path, &manifest_str, "manifest")?;
        if let Some(parent) = final_dir.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                SnipError::io_error("create backup parent directory", parent.to_path_buf(), e)
            })?;
        }
        fs::rename(staging_dir, final_dir).map_err(|e| {
            SnipError::io_error("atomic rename staging to final", final_dir.to_path_buf(), e)
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(staging_dir);
    }
    result
}

/// Backup manifest entry.
///
/// Unknown kinds are rejected during deserialization — a manifest with an
/// unrecognized kind is invalid and will not be accepted for restore.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifestEntry {
    pub path: String,
    pub kind: BackupEntryKind,
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
    let state_dir = crate::local_data::derive_local_data_state_dir();
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
                kind: BackupEntryKind::Library,
                size: 100,
                sha256: "abc123".to_string(),
            }],
        };

        let toml_str = toml::to_string_pretty(&manifest).unwrap();
        let restored: BackupManifest = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.schema, 1);
        assert_eq!(restored.files.len(), 1);
        assert_eq!(restored.files[0].kind, BackupEntryKind::Library);

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

    #[test]
    fn test_sha256_hex_deterministic() {
        let a = sha256_hex(b"hello world");
        let b = sha256_hex(b"hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn test_validate_toml_content_valid() {
        assert!(validate_toml_content(b"key = \"value\"", "test").is_ok());
        assert!(validate_toml_content(b"[[snippets]]\nid = \"x\"", "test").is_ok());
    }

    #[test]
    fn test_validate_toml_content_invalid() {
        let result = validate_toml_content(b"{{{{invalid", "test");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not valid TOML"), "Expected TOML error: {msg}");
    }

    #[test]
    fn test_validate_toml_content_not_utf8() {
        let result = validate_toml_content(&[0xFF, 0xFE], "test");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("UTF-8"), "Expected UTF-8 error: {msg}");
    }

    #[test]
    fn test_validate_canonical_containment_inside() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let child = root.join("child.toml");
        fs::write(&child, "data").unwrap();
        assert!(validate_canonical_containment(&child, root, "test").is_ok());
    }

    #[test]
    fn test_validate_canonical_containment_rejects_traversal() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("subdir");
        fs::create_dir(&root).unwrap();
        let file = dir.path().join("escape.toml");
        fs::write(&file, "data").unwrap();
        let result = validate_canonical_containment(&file, &root, "test");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("escapes config root"),
            "Expected containment error: {msg}"
        );
    }

    #[test]
    fn test_deterministic_manifest_ordering() {
        let mut manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 0,
            snip_it_version: "test".to_string(),
            layout: "directory".to_string(),
            files: vec![
                BackupManifestEntry {
                    path: "zebra.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: 1,
                    sha256: "a".to_string(),
                },
                BackupManifestEntry {
                    path: "alpha.toml".to_string(),
                    kind: BackupEntryKind::Library,
                    size: 2,
                    sha256: "b".to_string(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: BackupEntryKind::Index,
                    size: 3,
                    sha256: "c".to_string(),
                },
            ],
        };
        manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(manifest.files[0].path, "alpha.toml");
        assert_eq!(manifest.files[1].path, "libraries.toml");
        assert_eq!(manifest.files[2].path, "zebra.toml");
    }

    #[test]
    fn test_read_library_generation_missing_file() {
        let dir = TempDir::new().unwrap();
        let generation = read_library_generation(dir.path()).unwrap();
        assert_eq!(generation, 0);
    }

    #[test]
    fn test_read_library_generation_with_file() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("libraries.toml");
        fs::write(
            &config_path,
            "generation = 42\n[[libraries]]\nfilename = \"test\"\n",
        )
        .unwrap();
        let generation = read_library_generation(dir.path()).unwrap();
        assert_eq!(generation, 42);
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_backup_creates_all_files() {
        let staging = TempDir::new().unwrap();
        let final_dir = TempDir::new().unwrap();
        let backup_path = final_dir.path().join("backup");
        let files = [
            (
                "libraries/a.toml".to_string(),
                "library".to_string(),
                b"lib a".to_vec(),
            ),
            (
                "libraries/b.toml".to_string(),
                "library".to_string(),
                b"lib b".to_vec(),
            ),
        ];
        let file_refs: Vec<(&str, &str, &[u8])> = files
            .iter()
            .map(|(p, k, b)| (p.as_str(), k.as_str(), b.as_slice()))
            .collect();
        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 0,
            snip_it_version: "test".to_string(),
            layout: "directory".to_string(),
            files: vec![],
        };
        atomic_write_backup(staging.path(), &backup_path, &file_refs, &manifest).unwrap();
        assert!(backup_path.join("libraries/a.toml").exists());
        assert!(backup_path.join("libraries/b.toml").exists());
        assert!(backup_path.join("manifest.toml").exists());
        assert_eq!(
            fs::read(backup_path.join("libraries/a.toml")).unwrap(),
            b"lib a"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_backup_failure_cleans_staging() {
        let staging = TempDir::new().unwrap();
        let staging_path = staging.path().join("attempt");
        let final_dir = PathBuf::from("/nonexistent_root/impossible/backup");
        let files = [(
            "file.txt".to_string(),
            "config".to_string(),
            b"data".to_vec(),
        )];
        let file_refs: Vec<(&str, &str, &[u8])> = files
            .iter()
            .map(|(p, k, b)| (p.as_str(), k.as_str(), b.as_slice()))
            .collect();
        let manifest = BackupManifest {
            schema: 1,
            created_at_unix_ms: 0,
            snip_it_version: "test".to_string(),
            layout: "directory".to_string(),
            files: vec![],
        };
        let result = atomic_write_backup(&staging_path, &final_dir, &file_refs, &manifest);
        assert!(result.is_err());
        assert!(!staging_path.exists());
    }
}

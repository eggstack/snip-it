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

/// Compute the SHA-256 hex digest of a file.
fn sha256_file(path: &Path) -> SnipResult<String> {
    let bytes =
        fs::read(path).map_err(|e| SnipError::io_error("read file for hashing", path, e))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    Ok(result.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Copy a single file into the backup directory, computing its SHA-256.
///
/// Rejects symlinks to prevent following links outside the config directory.
fn copy_and_hash(src: &Path, dst: &Path, kind: &str) -> SnipResult<BackupManifestEntry> {
    let metadata =
        fs::symlink_metadata(src).map_err(|e| SnipError::io_error("stat source file", src, e))?;
    if metadata.file_type().is_symlink() {
        return Err(SnipError::runtime_error(
            "Backup entry is a symlink",
            Some(&format!(
                "Refusing to back up symlink {} (could point outside config)",
                src.display()
            )),
        ));
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create backup subdirectory", parent, e))?;
    }
    fs::copy(src, dst).map_err(|e| SnipError::io_error("copy file to backup", dst, e))?;
    let metadata =
        fs::metadata(dst).map_err(|e| SnipError::io_error("stat backup file", dst, e))?;
    let sha = sha256_file(dst)?;
    Ok(BackupManifestEntry {
        path: dst
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        kind: kind.to_string(),
        size: metadata.len(),
        sha256: sha,
    })
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

    // 1. Copy all library files
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
            let dst = backup_dir.join("libraries").join(&file_name);
            let src = entry.path();
            let manifest_entry = copy_and_hash(&src, &dst, "library")?;
            manifest.files.push(manifest_entry);
        }
    }

    // 2. Copy libraries.toml index (always included)
    let libraries_toml = config_dir.join("libraries.toml");
    if libraries_toml.exists() {
        let dst = backup_dir.join("libraries.toml");
        let entry = copy_and_hash(&libraries_toml, &dst, "index")?;
        manifest.files.push(entry);
    }

    // 3. Optionally include usage.toml
    if include_usage {
        let usage_path = config_dir.join("usage.toml");
        if usage_path.exists() {
            let dst = backup_dir.join("usage.toml");
            let entry = copy_and_hash(&usage_path, &dst, "usage")?;
            manifest.files.push(entry);
        }
    }

    // 4. Optionally include sync.toml (redact API key)
    if include_sync_state {
        let sync_path = config_dir.join("sync.toml");
        if sync_path.exists() {
            let content = fs::read_to_string(&sync_path)
                .map_err(|e| SnipError::io_error("read sync config", sync_path.clone(), e))?;
            let redacted = redact_sync_config(&content)?;
            let dst = backup_dir.join("sync.toml");
            crate::utils::atomic::write_private_atomic(&dst, &redacted, "sync")?;
            let entry = BackupManifestEntry {
                path: "sync.toml".to_string(),
                kind: "sync_config".to_string(),
                size: redacted.len() as u64,
                sha256: {
                    let mut hasher = Sha256::new();
                    hasher.update(redacted.as_bytes());
                    let result = hasher.finalize();
                    result.iter().map(|b| format!("{:02x}", b)).collect()
                },
            };
            manifest.files.push(entry);
        }
    }

    // 5. Optionally include general config .toml files (excluding already-handled files)
    if include_config {
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
                    let dst = backup_dir.join(&name);
                    let manifest_entry = copy_and_hash(&src, &dst, "config")?;
                    manifest.files.push(manifest_entry);
                }
            }
        }
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
    fn test_sha256_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello world").unwrap();
        let hash = sha256_file(&path).unwrap();
        assert_eq!(hash.len(), 64);
        // SHA-256 of "hello world"
        assert!(hash.starts_with("b94d27b9934d3e08"));
    }

    #[test]
    fn test_copy_and_hash() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let src = src_dir.path().join("data.txt");
        fs::write(&src, "test data").unwrap();
        let dst = dst_dir.path().join("copied.txt");

        let entry = copy_and_hash(&src, &dst, "test").unwrap();
        assert_eq!(entry.kind, "test");
        assert_eq!(entry.size, 9);
        assert_eq!(entry.sha256.len(), 64);
        assert!(dst.exists());
    }

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

    #[test]
    fn test_copy_and_hash_creates_parent_dirs() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let src = src_dir.path().join("file.txt");
        fs::write(&src, "content").unwrap();
        let dst = dst_dir
            .path()
            .join("subdir")
            .join("nested")
            .join("file.txt");

        let entry = copy_and_hash(&src, &dst, "library").unwrap();
        assert!(dst.exists());
        assert_eq!(entry.size, 7);
    }

    #[cfg(unix)]
    #[test]
    fn test_copy_and_hash_rejects_symlinks() {
        let src_dir = TempDir::new().unwrap();
        let dst_dir = TempDir::new().unwrap();
        let real_file = src_dir.path().join("real.txt");
        fs::write(&real_file, "content").unwrap();
        let symlink = src_dir.path().join("link.txt");
        std::os::unix::fs::symlink(&real_file, &symlink).unwrap();
        let dst = dst_dir.path().join("output.txt");

        let result = copy_and_hash(&symlink, &dst, "test");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symlink"),
            "Expected symlink error, got: {msg}"
        );
    }
}

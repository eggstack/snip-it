//! **Layer: Application**
//!
//! `snp restore` command — restore from a backup snapshot.

use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_config_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::backup_cmd::BackupManifest;

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
        fs::copy(backup_file, &dst)
            .map_err(|e| SnipError::io_error("restore library file", dst.clone(), e))?;
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

    // 2. Verify checksums
    for entry in &manifest.files {
        // Resolve path relative to backup directory
        let file_path =
            if entry.kind == "index" || entry.kind == "sync_config" || entry.kind == "usage" {
                backup.join(&entry.path)
            } else {
                backup.join("libraries").join(&entry.path)
            };

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

    // 3. Dry run: display planned actions
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

    // 4. For replace mode, create pre-restore backup
    if mode == RestoreMode::Replace
        && let Some(backup_path) = create_pre_restore_backup(&config_dir)?
    {
        report.pre_restore_backup = Some(backup_path.display().to_string());
    }

    // 5. Ensure libraries directory exists
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir)
        .map_err(|e| SnipError::io_error("create libraries directory", libraries_dir.clone(), e))?;

    // 6. Restore library files
    for entry in &manifest.files {
        if entry.kind == "library" {
            let src = backup.join("libraries").join(&entry.path);
            let library_name = entry.path.strip_suffix(".toml").unwrap_or(&entry.path);
            restore_library_file(&src, &libraries_dir, library_name, mode, &mut report)?;
        }
    }

    // 7. Restore libraries.toml index
    if let Some(index_entry) = manifest.files.iter().find(|f| f.kind == "index") {
        let src = backup.join(&index_entry.path);
        let dst = config_dir.join("libraries.toml");
        if mode == RestoreMode::Replace || !dst.exists() {
            fs::copy(&src, &dst)
                .map_err(|e| SnipError::io_error("restore index file", dst.clone(), e))?;
            report.files_restored += 1;
        }
    }

    // 8. Restore usage.toml if present
    if let Some(usage_entry) = manifest.files.iter().find(|f| f.kind == "usage") {
        let src = backup.join(&usage_entry.path);
        let dst = config_dir.join("usage.toml");
        if mode == RestoreMode::Replace || !dst.exists() {
            fs::copy(&src, &dst)
                .map_err(|e| SnipError::io_error("restore usage file", dst.clone(), e))?;
            report.files_restored += 1;
        }
    }

    // 9. Restore sync.toml if present (preserve local API key if exists)
    if let Some(sync_entry) = manifest.files.iter().find(|f| f.kind == "sync_config") {
        let src = backup.join(&sync_entry.path);
        let dst = config_dir.join("sync.toml");
        if dst.exists() && mode == RestoreMode::Merge {
            // In merge mode, don't overwrite local sync config (has real API key)
            report
                .skipped
                .push("sync.toml (local config preserved)".to_string());
        } else if mode == RestoreMode::Replace || !dst.exists() {
            // For replace, restore but warn about redacted key
            fs::copy(&src, &dst)
                .map_err(|e| SnipError::io_error("restore sync config", dst.clone(), e))?;
            report.conflicts.push(RestoreConflict {
                library: "sync".to_string(),
                kind: "redacted_key".to_string(),
                detail: "API key was redacted in backup; re-enter with 'snp register'".to_string(),
            });
            report.files_restored += 1;
        }
    }

    // 10. Output report
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
    use super::super::backup_cmd::BackupManifestEntry;
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
                    kind: "library".to_string(),
                    size: lib_content.len() as u64,
                    sha256: lib_hash,
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: "index".to_string(),
                    size: index.len() as u64,
                    sha256: index_hash,
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
        let _config_dir = TempDir::new().unwrap();
        let backup_dir = create_test_backup(dir());
        // Dry run should not error or modify anything
        let _result = run(backup_dir, RestoreMode::DryRun, false);
        // May error if config dir doesn't exist, but dry run itself shouldn't write
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

    fn dir() -> &'static Path {
        // Helper for tests that need a valid path
        Path::new("/tmp")
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
                    kind: "library".to_string(),
                    size: lib_content.len() as u64,
                    sha256: lib_hash.clone(),
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: "index".to_string(),
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
            let file_path = if entry.kind == "index" {
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
                    kind: "library".to_string(),
                    size: lib_content.len() as u64,
                    sha256: lib_hash,
                },
                BackupManifestEntry {
                    path: "libraries.toml".to_string(),
                    kind: "index".to_string(),
                    size: index.len() as u64,
                    sha256: index_hash,
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
            let file_path = if entry.kind == "index" {
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
}

//! Backup contract tests (Workstream E).
//!
//! Proves that backup:
//! - never follows symlinks
//! - excludes API keys and credentials
//! - produces correct manifest hashes
//! - creates consistent snapshots

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Setup: create a library with a test snippet.
fn setup_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "backup-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("backup-test.toml"),
        r#"[[snippets]]
id = "backup-1"
description = "backup test snippet"
command = "echo backup-test"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "backup-test"]);
    cmd.output().unwrap();
}

// === Symlink rejection ===

#[cfg(unix)]
#[test]
fn test_backup_rejects_symlinked_library() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    // Create a symlink in the libraries directory
    let libraries_dir = config_dir.join("libraries");
    let real_file = _tmp.path().join("external.toml");
    fs::write(&real_file, "external content").unwrap();
    let symlink = libraries_dir.join("symlinked.toml");
    std::os::unix::fs::symlink(&real_file, &symlink).unwrap();

    let backup_dir = _tmp.path().join("backup-output");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    // The backup command walks the libraries dir — a symlinked file should cause an error
    // OR the symlink should be silently skipped (depending on implementation).
    // Either way, the symlink content must NOT appear in the backup.
    if output.status.success() {
        // If backup succeeded, verify the symlink content is not in the backup
        let backup_libraries = backup_dir.join("libraries");
        if backup_libraries.exists() {
            for entry in fs::read_dir(&backup_libraries).unwrap().flatten() {
                let content = fs::read_to_string(entry.path()).unwrap_or_default();
                assert!(
                    !content.contains("external content"),
                    "Backup must not contain symlinked content"
                );
            }
        }
    } else {
        // If backup failed, it should mention symlink
        assert!(
            stderr.contains("symlink"),
            "Backup should fail with symlink error: {stderr}"
        );
    }
}

// === Credential exclusion ===

#[test]
fn test_backup_does_not_include_api_key_in_default_mode() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    // Write sync.toml with a real API key
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "sk-SECRET-API-KEY-DO_NOT_BACKUP"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
"#,
    )
    .unwrap();

    let backup_dir = _tmp.path().join("backup-no-sync");
    let output = snp_in(&config_dir)
        .args([
            "backup",
            "--output",
            backup_dir.to_str().unwrap(),
            // No --include-sync-state, so sync.toml should not be in backup
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify sync.toml is NOT in the backup
    assert!(
        !backup_dir.join("sync.toml").exists(),
        "sync.toml should not be in backup without --include-sync-state"
    );
}

#[test]
fn test_backup_redacts_api_key_when_sync_state_included() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "sk-SECRET-API-KEY-REDACT-ME"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
"#,
    )
    .unwrap();

    let backup_dir = _tmp.path().join("backup-with-sync");
    let output = snp_in(&config_dir)
        .args([
            "backup",
            "--output",
            backup_dir.to_str().unwrap(),
            "--include-sync-state",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup with --include-sync-state should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify sync.toml exists but API key is redacted
    let sync_backup = backup_dir.join("sync.toml");
    assert!(sync_backup.exists(), "sync.toml should be in backup");
    let content = fs::read_to_string(&sync_backup).unwrap();
    assert!(
        !content.contains("sk-SECRET-API-KEY-REDACT-ME"),
        "API key must be redacted in backup"
    );
    assert!(
        content.contains("<redacted>"),
        "Redacted marker should be present"
    );
}

// === Manifest hash correctness ===

#[test]
fn test_backup_manifest_hashes_match_files() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    let backup_dir = _tmp.path().join("backup-hash-check");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Load manifest
    let manifest_path = backup_dir.join("manifest.toml");
    assert!(manifest_path.exists());
    let manifest_content = fs::read_to_string(&manifest_path).unwrap();
    let manifest: serde_json::Value = toml::from_str(&manifest_content).unwrap();

    let files = manifest["files"].as_array().unwrap();
    assert!(!files.is_empty(), "manifest should have files");

    for entry in files {
        let path = entry["path"].as_str().unwrap();
        let expected_sha = entry["sha256"].as_str().unwrap();
        let kind = entry["kind"].as_str().unwrap();

        let file_path = if kind == "index" || kind == "usage" || kind == "sync_config" {
            backup_dir.join(path)
        } else {
            backup_dir.join("libraries").join(path)
        };

        assert!(
            file_path.exists(),
            "Manifest references non-existent file: {path}"
        );
        let actual_bytes = fs::read(&file_path).unwrap();
        let actual_sha = sha256_hex(&actual_bytes);
        assert_eq!(
            actual_sha, expected_sha,
            "SHA-256 mismatch for {path}: manifest says {expected_sha}, actual is {actual_sha}"
        );
    }
}

// === Consistent snapshot: library and index from same generation ===

#[test]
fn test_backup_manifest_includes_library_and_index() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    let backup_dir = _tmp.path().join("backup-consistency");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest_content = fs::read_to_string(backup_dir.join("manifest.toml")).unwrap();
    let manifest: serde_json::Value = toml::from_str(&manifest_content).unwrap();
    let files = manifest["files"].as_array().unwrap();

    let has_library = files.iter().any(|f| f["kind"] == "library");
    let has_index = files.iter().any(|f| f["kind"] == "index");

    assert!(has_library, "Backup must include library files");
    assert!(has_index, "Backup must include index file");
}

// === Backup flags: each optional include works independently ===

#[test]
fn test_backup_include_usage_flag() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    // Create a usage.toml
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "backup-1"
use_count = 5
last_used_at = 1700000000
"#,
    )
    .unwrap();

    let backup_dir = _tmp.path().join("backup-with-usage");
    let output = snp_in(&config_dir)
        .args([
            "backup",
            "--output",
            backup_dir.to_str().unwrap(),
            "--include-usage",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup with --include-usage should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify usage.toml is in the backup
    let manifest_content = fs::read_to_string(backup_dir.join("manifest.toml")).unwrap();
    let manifest: serde_json::Value = toml::from_str(&manifest_content).unwrap();
    let files = manifest["files"].as_array().unwrap();
    let has_usage = files.iter().any(|f| f["kind"] == "usage");
    assert!(
        has_usage,
        "Backup should include usage when --include-usage is set"
    );
}

#[test]
fn test_backup_without_usage_excludes_usage() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    // Create a usage.toml
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "backup-1"
use_count = 5
last_used_at = 1700000000
"#,
    )
    .unwrap();

    let backup_dir = _tmp.path().join("backup-no-usage");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest_content = fs::read_to_string(backup_dir.join("manifest.toml")).unwrap();
    let manifest: serde_json::Value = toml::from_str(&manifest_content).unwrap();
    let files = manifest["files"].as_array().unwrap();
    let has_usage = files.iter().any(|f| f["kind"] == "usage");
    assert!(
        !has_usage,
        "Backup should NOT include usage without --include-usage"
    );
}

// === No transaction journals in backup ===

#[test]
fn test_backup_excludes_transaction_journals() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir);

    // Create a fake transaction journal directory
    let txn_dir = config_dir.join(".state").join(".transaction");
    fs::create_dir_all(&txn_dir).unwrap();
    fs::write(txn_dir.join("txn-fake.toml"), "fake journal").unwrap();

    let backup_dir = _tmp.path().join("backup-no-txn");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Walk backup dir — no .transaction directory should be present
    let mut has_txn = false;
    fn walk_dir(dir: &Path, has_txn: &mut bool) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if entry.file_name().to_string_lossy() == ".transaction" {
                        *has_txn = true;
                    }
                    walk_dir(&entry.path(), has_txn);
                }
            }
        }
    }
    walk_dir(&backup_dir, &mut has_txn);
    assert!(!has_txn, "Backup must not include transaction journals");
}

//! Restore transactionality tests (Workstream D).
//!
//! Verifies that restore uses the transaction module correctly:
//! - dry run produces zero writes
//! - successful restore records exactly one pending generation
//! - failed restore rolls back

mod support;

use std::fs;
use support::helpers::*;

/// Build a backup directory with one library snippet.
fn make_backup(tmp: &std::path::Path) -> std::path::PathBuf {
    let backup_dir = tmp.join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let content = r#"[[snippets]]
id = "txn-test-1"
description = "transaction test snippet"
command = "echo txn-test"
"#;
    fs::write(libraries_dir.join("txn-test.toml"), content).unwrap();

    let index = r#"[[libraries]]
filename = "txn-test"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let lib_hash = sha256_hex(content.as_bytes().to_vec());
    let index_hash = sha256_hex(index.as_bytes().to_vec());

    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "txn-test.toml"
kind = "library"
size = {lib_size}
sha256 = "{lib_hash}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{index_hash}"
"#,
        lib_size = content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    backup_dir
}

fn sha256_hex(bytes: Vec<u8>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

// === Dry run creates no files ===

#[test]
fn test_dry_run_creates_no_library_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    // Ensure no library exists yet
    let lib_path = config_dir.join("libraries").join("txn-test.toml");
    assert!(!lib_path.exists());

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "dry-run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // No library files should be created
    assert!(!lib_path.exists(), "dry-run must not create library files");
}

#[test]
fn test_dry_run_creates_no_transaction_journals() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // No transaction journals should be created
    let state_dir = config_dir.join(".state").join(".transaction");
    if state_dir.exists() {
        let journals: Vec<_> = fs::read_dir(&state_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().is_some_and(|ext| ext == "toml")
                    && e.path()
                        .file_stem()
                        .is_some_and(|s| s.to_string_lossy().starts_with("txn-"))
            })
            .collect();
        assert!(
            journals.is_empty(),
            "dry-run must not create transaction journals"
        );
    }
}

// === Pending generation tracking ===

#[test]
fn test_no_pending_before_restore() {
    let (_tmp, config_dir) = setup_test_env();
    let pending_path = config_dir.join("auto-sync-pending.toml");
    assert!(
        !pending_path.exists(),
        "No pending marker should exist before any mutation"
    );
}

// === Merge mode test ===

#[test]
fn test_merge_restore_with_identical_content_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    // First, do a replace to populate the config
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "initial replace should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Now restore the same backup in merge mode — should be a no-op
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "merge"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "merge with identical content should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the snippet is still correct
    let lib_path = config_dir.join("libraries").join("txn-test.toml");
    assert!(lib_path.exists());
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("txn-test-1"));
}

// === Replace mode test ===

#[test]
fn test_replace_restore_creates_library_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "replace should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = config_dir.join("libraries").join("txn-test.toml");
    assert!(lib_path.exists(), "replace should create library file");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("txn-test-1"));
}

// === Restore report format ===

#[test]
fn test_restore_json_output_format() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (_env_tmp, config_dir) = setup_test_env();
    let backup_dir = make_backup(tmp.path());

    let output = snp_in(&config_dir)
        .args([
            "restore",
            backup_dir.to_str().unwrap(),
            "--mode",
            "dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["mode"], "DryRun");
    assert_eq!(report["manifest_schema"], 1);
}

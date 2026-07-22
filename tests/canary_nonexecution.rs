//! Canary non-execution tests (Workstream K).
//!
//! Verifies that read-only commands never execute snippet commands,
//! even when the snippet contains shell metacharacters or malicious content.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

/// Setup a library with a canary snippet.
fn setup_canary_library(config_dir: &Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let content = r#"[[snippets]]
id = "canary-1"
description = "canary non-execution test"
command = "touch /tmp/snp-canary-nonexecution-pwned"
"#;
    fs::write(libraries_dir.join("canary.toml"), content).unwrap();

    let index = r#"[[libraries]]
filename = "canary"
is_primary = true
"#;
    fs::write(config_dir.join("libraries.toml"), index).unwrap();
}

fn snp_in(config_dir: &Path) -> std::process::Command {
    support::helpers::snp_in(config_dir)
}

// === Non-execution canary tests ===

#[test]
fn test_get_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-1"])
        .output()
        .unwrap();

    assert!(
        !canary_path.exists(),
        "snp get must not execute the snippet command"
    );
    assert!(
        output.status.success(),
        "snp get should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_list_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let _output = snp_in(&config_dir).args(["list"]).output().unwrap();

    assert!(
        !canary_path.exists(),
        "snp list must not execute the snippet command"
    );
}

#[test]
fn test_status_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let _output = snp_in(&config_dir).args(["status"]).output().unwrap();

    assert!(
        !canary_path.exists(),
        "snp status must not execute the snippet command"
    );
}

#[test]
fn test_validate_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let _output = snp_in(&config_dir).args(["validate"]).output().unwrap();

    assert!(
        !canary_path.exists(),
        "snp validate must not execute the snippet command"
    );
}

#[test]
fn test_backup_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let backup_out = _tmp.path().join("backup-out");
    let _output = snp_in(&config_dir)
        .args(["backup", "--output", backup_out.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        !canary_path.exists(),
        "snp backup must not execute the snippet command"
    );
}

#[test]
fn test_search_help_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let _output = snp_in(&config_dir)
        .args(["search", "--help"])
        .output()
        .unwrap();

    assert!(
        !canary_path.exists(),
        "snp search --help must not execute the snippet command"
    );
}

#[test]
fn test_library_list_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    let _output = snp_in(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();

    assert!(
        !canary_path.exists(),
        "snp library list must not execute the snippet command"
    );
}

#[test]
fn test_restore_dry_run_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    setup_canary_library(&config_dir);

    let canary_path = std::path::PathBuf::from("/tmp/snp-canary-nonexecution-pwned");
    let _ = fs::remove_file(&canary_path);

    // Create a minimal backup
    let backup_dir = _tmp.path().join("backup");
    let lib_dir = backup_dir.join("libraries");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(
        lib_dir.join("canary.toml"),
        fs::read_to_string(config_dir.join("libraries").join("canary.toml")).unwrap(),
    )
    .unwrap();
    fs::write(
        backup_dir.join("libraries.toml"),
        fs::read_to_string(config_dir.join("libraries.toml")).unwrap(),
    )
    .unwrap();

    let content = fs::read_to_string(config_dir.join("libraries").join("canary.toml")).unwrap();
    let index = fs::read_to_string(config_dir.join("libraries.toml")).unwrap();
    let lib_sha = sha256_hex(content.as_bytes().to_vec());
    let idx_sha = sha256_hex(index.as_bytes().to_vec());
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "canary.toml"
kind = "library"
size = {lib_size}
sha256 = "{lib_sha}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{idx_sha}"
"#,
        lib_size = content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let _output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();

    assert!(
        !canary_path.exists(),
        "snp restore --mode dry-run must not execute the snippet command"
    );
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

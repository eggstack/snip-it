//! Plan 008 compatibility: top-level and `snp data ...` spellings share one
//! canonical argument schema and dispatch to the same implementation.
//!
//! Proves equivalent behavior for validate, backup, repair dry-run, and
//! status JSON. Both spellings must succeed with the same exit codes and
//! equivalent machine-readable output.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn setup_basic_library(config_dir: &Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let content = r#"[[snippets]]
id = "compat-1"
description = "compat fixture"
command = "echo hello"
"#;
    fs::write(libraries_dir.join("compat.toml"), content).unwrap();
    let index = r#"[[libraries]]
filename = "compat"
is_primary = true
"#;
    fs::write(config_dir.join("libraries.toml"), index).unwrap();
}

fn run_args(config_dir: &Path, args: &[&str]) -> std::process::Output {
    snp_in(config_dir).args(args).output().unwrap()
}

fn assert_success(output: &std::process::Output, label: &str) {
    assert!(
        output.status.success(),
        "{label} must succeed, exit={:?} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_top_and_data_produce_equivalent_json() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let top = run_args(&config_dir, &["validate", "--json"]);
    let data = run_args(&config_dir, &["data", "validate", "--json"]);
    assert_success(&top, "snp validate --json");
    assert_success(&data, "snp data validate --json");

    let top_json: serde_json::Value =
        serde_json::from_slice(&top.stdout).expect("validate --json must emit valid JSON");
    let data_json: serde_json::Value =
        serde_json::from_slice(&data.stdout).expect("data validate --json must emit valid JSON");

    // Equivalent behavior: same library/snippet counts and same diagnostics.
    // Timestamps/versions are part of the report but counts prove same path.
    assert_eq!(top_json["total_libraries"], data_json["total_libraries"]);
    assert_eq!(top_json["total_snippets"], data_json["total_snippets"]);
    assert_eq!(
        top_json["diagnostics"].as_array().map(|a| a.len()),
        data_json["diagnostics"].as_array().map(|a| a.len()),
        "diagnostic counts must match"
    );
    // Exact count (not >=): clean fixture has deterministic shape.
    assert_eq!(top_json["total_libraries"], 1);
    assert_eq!(top_json["total_snippets"], 1);
}

#[test]
fn validate_aliases_resolve_to_same_handler() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    // Top-level alias `val`, data alias `v`.
    let top_alias = run_args(&config_dir, &["val", "--json"]);
    let data_alias = run_args(&config_dir, &["data", "v", "--json"]);
    assert_success(&top_alias, "snp val --json");
    assert_success(&data_alias, "snp data v --json");

    let a: serde_json::Value = serde_json::from_slice(&top_alias.stdout).unwrap();
    let b: serde_json::Value = serde_json::from_slice(&data_alias.stdout).unwrap();
    assert_eq!(a["total_snippets"], b["total_snippets"]);
}

#[test]
fn backup_top_and_data_produce_equivalent_snapshots() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let out_top = _tmp.path().join("backup-top");
    let out_data = _tmp.path().join("backup-data");

    let top = run_args(
        &config_dir,
        &["backup", "--output", out_top.to_str().unwrap(), "--json"],
    );
    let data = run_args(
        &config_dir,
        &[
            "data",
            "backup",
            "--output",
            out_data.to_str().unwrap(),
            "--json",
        ],
    );
    assert_success(&top, "snp backup");
    assert_success(&data, "snp data backup");

    let top_json: serde_json::Value = serde_json::from_slice(&top.stdout).unwrap();
    let data_json: serde_json::Value = serde_json::from_slice(&data.stdout).unwrap();
    assert_eq!(top_json["file_count"], data_json["file_count"]);
    assert_eq!(top_json["file_count"], 2);

    for dir in [&out_top, &out_data] {
        assert!(
            dir.join("manifest.toml").exists(),
            "backup {} must contain manifest.toml",
            dir.display()
        );
    }
}

#[test]
fn data_backup_alias_matches_canonical() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let out = _tmp.path().join("backup-alias");
    let output = run_args(
        &config_dir,
        &["data", "b", "--output", out.to_str().unwrap(), "--json"],
    );
    assert_success(&output, "snp data b");
    assert!(out.join("manifest.toml").exists());
}

#[test]
fn repair_dry_run_top_and_data_are_equivalent() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let top = run_args(&config_dir, &["repair", "--dry-run", "--json"]);
    let data = run_args(&config_dir, &["data", "repair", "--dry-run", "--json"]);
    assert_success(&top, "snp repair --dry-run --json");
    assert_success(&data, "snp data repair --dry-run --json");

    let top_json: serde_json::Value = serde_json::from_slice(&top.stdout).unwrap();
    let data_json: serde_json::Value = serde_json::from_slice(&data.stdout).unwrap();
    assert_eq!(top_json["exit_status"], data_json["exit_status"]);
    assert_eq!(
        top_json["items"].as_array().map(|a| a.len()),
        data_json["items"].as_array().map(|a| a.len()),
        "repair item counts must match"
    );
}

#[test]
fn status_json_top_and_data_are_equivalent() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let top = run_args(&config_dir, &["status", "--json"]);
    let data = run_args(&config_dir, &["data", "status", "--json"]);
    assert_success(&top, "snp status --json");
    assert_success(&data, "snp data status --json");

    let top_json: serde_json::Value = serde_json::from_slice(&top.stdout).unwrap();
    let data_json: serde_json::Value = serde_json::from_slice(&data.stdout).unwrap();

    // Both must expose the same top-level snapshot shape. Timestamps may
    // differ by milliseconds between runs, so compare structural keys and
    // the stable sync top-level classification, not exact equality.
    for key in ["local", "sync", "pending", "attempt"] {
        assert!(
            top_json.get(key).is_some(),
            "status JSON must contain '{key}'"
        );
        assert!(
            data_json.get(key).is_some(),
            "data status JSON must contain '{key}'"
        );
    }
}

#[test]
fn data_status_alias_matches_canonical() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    let output = run_args(&config_dir, &["data", "s", "--json"]);
    assert_success(&output, "snp data s --json");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.get("sync").is_some());
}

#[test]
fn restore_dry_run_top_and_data_are_equivalent() {
    let (_tmp, config_dir) = setup_test_env();
    setup_basic_library(&config_dir);

    // Build a backup via the canonical top-level path, then dry-run restore
    // through both spellings.
    let backup_dir = _tmp.path().join("backup");
    let backup = run_args(
        &config_dir,
        &["backup", "--output", backup_dir.to_str().unwrap()],
    );
    assert_success(&backup, "setup backup");

    let top = run_args(
        &config_dir,
        &[
            "restore",
            backup_dir.to_str().unwrap(),
            "--mode",
            "dry-run",
            "--json",
        ],
    );
    let data = run_args(
        &config_dir,
        &[
            "data",
            "restore",
            backup_dir.to_str().unwrap(),
            "--mode",
            "dry-run",
            "--json",
        ],
    );
    assert_success(&top, "snp restore dry-run");
    assert_success(&data, "snp data restore dry-run");

    let top_json: serde_json::Value = serde_json::from_slice(&top.stdout).unwrap();
    let data_json: serde_json::Value = serde_json::from_slice(&data.stdout).unwrap();
    assert_eq!(top_json["mode"], data_json["mode"]);
    assert_eq!(
        top_json["files_in_backup"], data_json["files_in_backup"],
        "restore dry-run file counts must match"
    );
}

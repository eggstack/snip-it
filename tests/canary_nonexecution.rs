mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn setup_canary_library(config_dir: &Path, snippet_id: &str, sentinel: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "canary-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let toml = format!(
        r#"[[snippets]]
id = "{snippet_id}"
description = "canary snippet"
command = "touch {sentinel}"
tags = ["canary"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#
    );
    fs::write(libraries_dir.join("canary-test.toml"), toml).unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "canary-test"]);
    cmd.output().unwrap();
}

fn sentinel_path(name: &str) -> String {
    format!("/tmp/08a-canary-{name}")
}

fn clean_sentinel(name: &str) {
    let path = sentinel_path(name);
    let _ = fs::remove_file(&path);
}

fn assert_sentinel_was_not_created(name: &str) {
    let path = sentinel_path(name);
    assert!(
        !std::path::Path::new(&path).exists(),
        "sentinel file {path:?} was created — snippet was executed"
    );
}

#[test]
fn test_get_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "get";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-1", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_get_field_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "get-field";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-2", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-2", "--field", "command"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp get --field failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_get_raw_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "get-raw";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-3", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-3", "--raw"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp get --raw failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_get_json_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "get-json";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-4", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-4", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp get --json failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_get_expanded_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "get-expanded";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-9", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir)
        .args(["get", "--id", "canary-9", "--expanded"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp get --expanded failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_list_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "list";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-5", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir).args(["list"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_status_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "status";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-6", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir).args(["status"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_validate_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "validate";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-7", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let output = snp_in(&config_dir).args(["validate"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp validate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_backup_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "backup";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-8", &sentinel);
    assert!(!std::path::Path::new(&sentinel).exists());

    let backup_dir = _tmp.path().join("backup-output");
    fs::create_dir_all(&backup_dir).unwrap();

    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp backup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

// ── Additional canary tests for remaining non-executing surfaces ──

#[test]
fn test_search_help_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "search-help";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-10", &sentinel);

    let output = snp_in(&config_dir)
        .args(["search", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp search --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_library_list_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "library-list";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-11", &sentinel);

    let output = snp_in(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp library list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_library_show_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "library-show";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-12", &sentinel);

    let output = snp_in(&config_dir)
        .args(["library", "show", "canary-test"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp library show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_restore_dry_run_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "restore-dry-run";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-13", &sentinel);

    // Create a backup first
    let backup_dir = _tmp.path().join("backup-for-restore");
    fs::create_dir_all(&backup_dir).unwrap();
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp backup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Restore in dry-run mode
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp restore --dry-run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_sync_run_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "sync-run";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-14", &sentinel);

    // Configure sync with a non-existent server
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
"#,
    )
    .unwrap();

    // sync run will fail (no server), but should not execute snippets
    let output = snp_in(&config_dir).args(["sync", "run"]).output().unwrap();
    // Command may fail due to no server — that's fine, we just need to ensure
    // no sentinel was created (snippet was not executed)
    let _ = output;

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

#[test]
fn test_list_filter_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();
    let name = "list-filter";
    let sentinel = sentinel_path(name);
    clean_sentinel(name);
    setup_canary_library(&config_dir, "canary-15", &sentinel);

    let output = snp_in(&config_dir)
        .args(["list", "--filter", "canary"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp list --filter failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_sentinel_was_not_created(name);
    clean_sentinel(name);
}

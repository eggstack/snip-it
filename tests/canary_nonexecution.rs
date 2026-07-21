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

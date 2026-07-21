mod support;

use std::path::Path;
use support::helpers::*;
use tempfile::TempDir;

fn create_selector_test_library(config_dir: &Path) {
    snp_in(config_dir)
        .args(["library", "create", "sel-test"])
        .output()
        .unwrap();
    snp_in(config_dir)
        .args(["library", "set-primary", "sel-test"])
        .output()
        .unwrap();
    std::fs::write(
        config_dir.join("libraries/sel-test.toml"),
        r#"[[snippets]]
id = "sel-aaa-111"
description = "git commit"
command = "git commit -m \"msg\""
tags = ["git"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "sel-bbb-222"
description = "git push"
command = "git push origin main"
tags = ["git"]
output = ""
folders = []
favorite = true
created_at = 200
updated_at = 200

[[snippets]]
id = "sel-ccc-333"
description = "list files"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "sel-ddd-444"
description = "Git Commit"
command = "git commit --amend"
tags = ["git"]
output = ""
folders = []
favorite = false
created_at = 400
updated_at = 400
"#,
    )
    .unwrap();
}

#[test]
fn test_get_by_id() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-bbb-222"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "git push origin main");
}

#[test]
fn test_get_by_id_not_found() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "nonexistent"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_get_by_description_exact() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--description-exact", "git commit"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn test_get_by_description_exact_unique() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--description-exact", "list files"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "ls -la");
}

#[test]
fn test_get_by_command_exact() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--command-exact", "ls -la"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "ls -la");
}

#[test]
fn test_get_ambiguous_returns_exit_5() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--description-exact", "git commit"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn test_get_not_found_returns_exit_3() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "nonexistent"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
}

#[test]
fn test_get_json_includes_library() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-aaa-111", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("sel-test"),
        "JSON output should contain library name 'sel-test': {stdout}"
    );
}

#[test]
fn test_get_raw_no_trailing_newline() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-aaa-111", "--raw"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "git commit -m \"msg\"");
}

#[test]
fn test_get_field_id() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-aaa-111", "--field", "id"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "sel-aaa-111");
}

#[test]
fn test_get_field_description() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-aaa-111", "--field", "description"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout, "git commit");
}

#[test]
fn test_get_first_policy() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--query", "git", "--resolution", "first"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_get_conflicting_selectors() {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    std::fs::create_dir_all(&config_dir).unwrap();
    create_selector_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "sel-aaa-111", "--description-exact", "foo"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

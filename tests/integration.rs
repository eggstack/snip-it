use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn snp_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_snp"))
}

fn setup_test_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();
    (tmp, config_dir)
}

fn snp_in(config_dir: &Path) -> Command {
    let mut cmd = snp_cmd();
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd
}

// --- Version ---

#[test]
fn test_version_flag() {
    let output = snp_cmd().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected),
        "Expected version '{expected}' in output: {stdout}"
    );
}

#[test]
fn test_version_subcommand() {
    let output = snp_cmd().arg("version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected),
        "Expected version '{expected}' in output: {stdout}"
    );
}

// --- Keybindings ---

#[test]
fn test_keybindings_show() {
    let output = snp_cmd().arg("keybindings").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Normal Mode"));
    assert!(stdout.contains("Insert Mode"));
    assert!(stdout.contains("insert mode"));
}

// --- Library Management ---

#[test]
fn test_library_create_list_delete() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-integration"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // List
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("test-integration"),
        "Expected 'test-integration' in list output: {stdout}"
    );

    // Show
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "show", "test-integration"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test-integration"));

    // Delete
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "delete", "test-integration", "--force"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Verify deleted
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "list"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("test-integration"),
        "Library should be deleted: {stdout}"
    );
}

#[test]
fn test_library_create_invalid_name() {
    let (_tmp, config_dir) = setup_test_env();

    // Empty name
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", ""]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    // Name with slash
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "foo/bar"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_library_delete_nonexistent() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "delete", "does-not-exist", "--force"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}

// --- List Command ---

#[test]
fn test_list_empty_library() {
    let (_tmp, config_dir) = setup_test_env();

    // Create library
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-list"]);
    cmd.output().unwrap();

    // Set it as primary
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-list"]);
    cmd.output().unwrap();

    // List should succeed but show nothing
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty() || stdout.contains("-----"));
}

#[test]
fn test_list_no_library() {
    let (_tmp, config_dir) = setup_test_env();

    // List without creating a library should print a message
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "Expected 'No library found' in stderr: {stderr}"
    );
}

// --- Config path ---

#[test]
fn test_config_dir_created_on_init() {
    let (_tmp, config_dir) = setup_test_env();

    // Run any command that initializes config
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "init-test"]);
    cmd.output().unwrap();

    // Config dir should exist
    assert!(config_dir.exists());
    assert!(config_dir.join("libraries").exists());
    assert!(config_dir.join("libraries.toml").exists());
}

// --- Error handling ---

#[test]
fn test_invalid_subcommand() {
    let output = snp_cmd().arg("nonexistent").output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_missing_subcommand() {
    let output = snp_cmd().output().unwrap();
    assert!(!output.status.success());
}

// --- Snippet list with data ---

#[test]
fn test_list_with_snippets() {
    let (_tmp, config_dir) = setup_test_env();

    // Create library
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-snippets"]);
    cmd.output().unwrap();

    // Write snippets directly to the library file
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("test-snippets.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
description = "list files"
command = "ls -la"
tags = ["files", "basic"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "show disk usage"
command = "df -h"
tags = ["system"]
output = ""
folders = []
favorite = true
"#,
    )
    .unwrap();

    // Set this library as primary so `list` finds it
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-snippets"]);
    cmd.output().unwrap();

    // List should show both snippets
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ls -la"), "Expected 'ls -la' in: {stdout}");
    assert!(stdout.contains("df -h"), "Expected 'df -h' in: {stdout}");
}

#[test]
fn test_list_with_filter() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-filter"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("test-filter.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
description = "list files"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "show disk usage"
command = "df -h"
tags = ["system"]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    // Set this library as primary
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-filter"]);
    cmd.output().unwrap();

    // Filter should show only matching snippets
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--filter", "disk"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("df -h"), "Expected 'df -h' in: {stdout}");
    assert!(
        !stdout.contains("ls -la"),
        "Should not contain 'ls -la': {stdout}"
    );
}

// --- Cron command ---

#[test]
fn test_cron_output() {
    let output = snp_cmd().args(["cron"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Crontab entry"),
        "Expected crontab entry header: {stdout}"
    );
    assert!(
        stdout.contains("sync --non-interactive"),
        "Expected sync command: {stdout}"
    );
}

#[test]
fn test_cron_custom_interval() {
    let output = snp_cmd().args(["cron", "-i", "30"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("*/30"), "Expected interval of 30: {stdout}");
}

// --- Library set-primary ---

#[test]
fn test_library_set_primary() {
    let (_tmp, config_dir) = setup_test_env();

    // Create two libraries
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "lib-a"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "lib-b"]);
    cmd.output().unwrap();

    // Set lib-b as primary
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lib-b"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Show should confirm lib-b is primary
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "show"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("lib-b") && stdout.contains("primary"),
        "Expected lib-b to be primary: {stdout}"
    );
}

use std::fs;
use std::path::PathBuf;
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

fn snp_in(config_dir: &PathBuf) -> Command {
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

    // List should succeed but show nothing
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "test-list"]);
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

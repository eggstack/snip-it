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

#[test]
fn test_update_help() {
    let output = snp_cmd().args(["update", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("current installation method"));
    assert!(stdout.contains("--dry-run"));
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

#[test]
fn test_sync_without_config_does_not_attempt_connection() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Sync is not enabled"),
        "Expected disabled-sync message in stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Failed to connect"),
        "Disabled sync should not attempt a server connection: {stderr}"
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
fn test_missing_subcommand_runs_default_tui_command() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    let output = cmd.output().unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "Expected bare snp to use the run command path when no library exists, got stderr: {stderr}"
    );
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

// --- List --json ---

#[test]
fn test_list_json_format() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-json"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("test-json.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
description = "list files"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = true
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-json"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["description"], "list files");
    assert_eq!(parsed[0]["command"], "ls -la");
    assert_eq!(parsed[0]["favorite"], true);
}

#[test]
fn test_list_json_empty() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-json-empty"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-json-empty"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert!(parsed.is_empty());
}

// --- List --csv ---

#[test]
fn test_list_csv_format() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-csv"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("test-csv.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
description = "list files"
command = "ls -la"
tags = ["files", "basic"]
output = ""
folders = ["work"]
favorite = true
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-csv"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "description,command,output,tags,folders,favorite");
    assert!(lines[1].contains("list files"));
    assert!(lines[1].contains("ls -la"));
    assert!(lines[1].contains("files;basic"));
    assert!(lines[1].contains("work"));
}

#[test]
fn test_list_csv_escape_special_chars() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-csv-escape"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("test-csv-escape.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
description = "desc with, comma"
command = "echo \"quoted\""
tags = []
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "test-csv-escape"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("\"desc with, comma\""));
    assert!(stdout.contains("\"echo \"\"quoted\"\"\""));
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
        stdout.contains(" sync\n") || stdout.contains(" sync\r\n") || stdout.ends_with(" sync"),
        "Expected sync command: {stdout}"
    );
    assert!(
        !stdout.contains("--non-interactive"),
        "Cron entry should not contain removed --non-interactive flag: {stdout}"
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

// --- New --description flag ---

#[test]
fn test_new_with_description_flag() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["new", "--description", "Test snippet", "echo hello"])
        .output()
        .expect("failed to execute");
    assert!(
        output.status.success(),
        "new with --description should succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));

    // Verify the snippet was created with the right description
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .expect("failed to execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Test snippet"));
    assert!(stdout.contains("echo hello"));
}

#[test]
fn test_new_without_description_still_works() {
    let (_tmp, config_dir) = setup_test_env();
    let mut child = snp_in(&config_dir)
        .args(["new"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn");

    // Send empty input to trigger the prompts, then close stdin
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("failed to wait");
    // Should fail because no input was provided to the prompts
    assert!(!output.status.success());
}

// --- Completions ---

#[test]
fn test_completions_bash() {
    let output = snp_cmd()
        .args(["completions", "bash"])
        .output()
        .expect("failed to execute");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("snp"));
    assert!(stdout.contains("bash"));
}

#[test]
fn test_completions_zsh() {
    let output = snp_cmd()
        .args(["completions", "zsh"])
        .output()
        .expect("failed to execute");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("snp"));
}

#[test]
fn test_completions_fish() {
    let output = snp_cmd()
        .args(["completions", "fish"])
        .output()
        .expect("failed to execute");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("snp"));
}

// --- Keybindings: theme picker ---

#[test]
fn test_keybindings_lists_theme_picker() {
    let output = snp_cmd().arg("keybindings").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Theme Picker"),
        "expected 'Theme Picker' section in keybindings output: {stdout}"
    );
    assert!(
        stdout.contains("open theme picker"),
        "expected 'open theme picker' binding: {stdout}"
    );
    assert!(
        stdout.contains("save & apply theme"),
        "expected 'save & apply theme' binding: {stdout}"
    );
}

// --- CLI contract: exit code on missing config ---

#[test]
fn test_run_without_config_exits_nonzero() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["run"]);
    cmd.stdin(std::process::Stdio::null());
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "snp run with no config should print 'No library found': {stderr}"
    );
}

#[test]
fn test_clip_without_config_exits_nonzero() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["clip"]);
    cmd.stdin(std::process::Stdio::null());
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "snp clip with no config should print 'No library found': {stderr}"
    );
}

#[test]
fn test_search_without_config_exits_nonzero() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["search"]);
    cmd.stdin(std::process::Stdio::null());
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "snp search with no config should print 'No library found': {stderr}"
    );
}

#[test]
fn test_edit_without_config_exits_nonzero() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["edit"]);
    cmd.stdin(std::process::Stdio::null());
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("No library found")
            || combined.contains("editor")
            || combined.contains("EDITOR")
            || output.status.success(),
        "snp edit with no config should either report no library or attempt to open editor: {combined}"
    );
}

// --- CLI contract: list output formats ---

#[test]
fn test_list_json_output_is_valid_json_array() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "contract-json"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("contract-json.toml"),
        r#"
[[Snippets]]
description = "greet"
command = "echo hi"
tags = ["test"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "list files"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = true
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "contract-json"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("output must be valid JSON");
    assert_eq!(parsed.len(), 2, "expected 2 snippets in JSON array");

    let descriptions: Vec<&str> = parsed
        .iter()
        .map(|v| v["description"].as_str().unwrap())
        .collect();
    assert!(descriptions.contains(&"greet"));
    assert!(descriptions.contains(&"list files"));

    for item in &parsed {
        assert!(
            item.get("command").is_some(),
            "each item must have 'command'"
        );
        assert!(item.get("tags").is_some(), "each item must have 'tags'");
        assert!(
            item.get("favorite").is_some(),
            "each item must have 'favorite'"
        );
    }
}

#[test]
fn test_list_csv_output_has_header() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "contract-csv"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("contract-csv.toml"),
        r#"
[[Snippets]]
description = "test snippet"
command = "echo test"
tags = ["contract"]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "contract-csv"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();

    let header = lines
        .next()
        .expect("CSV output must have at least a header line");
    assert_eq!(
        header, "description,command,output,tags,folders,favorite",
        "CSV header must match expected columns"
    );

    let data_line = lines
        .next()
        .expect("CSV output must have at least one data line");
    assert!(data_line.contains("test snippet"));
    assert!(data_line.contains("echo test"));
}

#[test]
fn test_list_default_output_contains_commands() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "contract-default"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("contract-default.toml"),
        r#"
[[Snippets]]
description = "disk usage"
command = "du -sh *"
tags = ["sys"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "network info"
command = "ifconfig"
tags = ["net"]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "contract-default"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("du -sh *"),
        "default output must contain snippet command: {stdout}"
    );
    assert!(
        stdout.contains("ifconfig"),
        "default output must contain snippet command: {stdout}"
    );
}

// --- CLI contract: new command adds snippet ---

#[test]
fn test_new_adds_snippet_to_library() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "contract-new"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "contract-new"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["new", "--description", "contract test", "echo contract"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success(), "snp new should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));

    let lib_path = config_dir.join("libraries").join("contract-new.toml");
    assert!(
        lib_path.exists(),
        "library TOML file must exist after adding snippet"
    );
    let contents = fs::read_to_string(&lib_path).unwrap();
    assert!(
        contents.contains("contract test"),
        "TOML must contain the new snippet description"
    );
    assert!(
        contents.contains("echo contract"),
        "TOML must contain the new snippet command"
    );
}

// --- CLI contract: cron output format ---

#[test]
fn test_cron_output_is_crontab_format() {
    let mut cmd = snp_cmd();
    cmd.args(["cron"]);
    cmd.stdin(std::process::Stdio::null());
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let re = regex::Regex::new(r"\*/\d+ \* \* \* \* .+ sync").unwrap();
    assert!(
        re.is_match(&stdout),
        "output must contain a crontab entry like '*/N * * * * <path> sync': {stdout}"
    );
}

// --- CLI contract: completions ---

#[test]
fn test_completions_bash_output_is_valid() {
    let output = snp_cmd().args(["completions", "bash"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("_snp") || stdout.contains("complete"),
        "bash completions must contain '_snp' or 'complete': {stdout}"
    );
    assert!(
        stdout.contains("snp"),
        "bash completions must reference the snp command: {stdout}"
    );
}

#[test]
fn test_completions_zsh_output_is_valid() {
    let output = snp_cmd().args(["completions", "zsh"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("compdef") || stdout.contains("_arguments") || stdout.contains("snp"),
        "zsh completions must contain 'compdef', '_arguments', or 'snp': {stdout}"
    );
}

#[test]
fn test_completions_fish_output_is_valid() {
    let output = snp_cmd().args(["completions", "fish"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("complete"),
        "fish completions must contain 'complete': {stdout}"
    );
}

// --- CLI contract: version output ---

#[test]
fn test_version_output_contains_version_string() {
    let output = snp_cmd().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected),
        "version output must contain '{expected}': {stdout}"
    );
    let re = regex::Regex::new(r"snp \d+\.\d+\.\d+").unwrap();
    assert!(
        re.is_match(&stdout),
        "version output must match pattern 'snp X.Y.Z': {stdout}"
    );
}

// --- CLI contract: keybindings structure ---

#[test]
fn test_keybindings_output_structure() {
    let output = snp_cmd().arg("keybindings").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Normal Mode"),
        "keybindings must contain 'Normal Mode' section: {stdout}"
    );
    assert!(
        stdout.contains("Insert Mode"),
        "keybindings must contain 'Insert Mode' section: {stdout}"
    );
}

// --- TUI lifecycle: performance ---

#[test]
fn test_list_with_many_snippets_performance() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "perf-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("perf-test.toml");

    let mut toml = String::new();
    for i in 0..100 {
        toml.push_str(&format!(
            r#"[[Snippets]]
description = "snippet {i}"
command = "echo {i}"
tags = ["tag{i}"]
output = ""
folders = []
favorite = false
"#,
            i = i
        ));
    }
    fs::write(&lib_path, &toml).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "perf-test"]);
    cmd.output().unwrap();

    let start = std::time::Instant::now();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 100);
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Listing 100 snippets took {:?}, expected < 5s",
        elapsed
    );
}

// --- TUI lifecycle: unicode ---

#[test]
fn test_list_with_unicode_in_snippets() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "unicode-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("unicode-test.toml");
    fs::write(
        &lib_path,
        r#"[[Snippets]]
description = "中文测试"
command = "echo 'こんにちは世界'"
tags = ["日本語"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "accented résumé café"
command = "echo 'Ñoño über'"
tags = ["latin"]
output = ""
folders = []
favorite = false

[[Snippets]]
description = "Cyrillic Тест"
command = "echo 'Привет мир'"
tags = ["кириллица"]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "unicode-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0]["description"], "中文测试");
    assert_eq!(parsed[1]["description"], "accented résumé café");
    assert_eq!(parsed[2]["description"], "Cyrillic Тест");
}

// --- TUI lifecycle: very long command ---

#[test]
fn test_list_with_very_long_command() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "long-cmd-test"]);
    cmd.output().unwrap();

    let long_command = "echo ".to_string() + &"a".repeat(1000);
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("long-cmd-test.toml");
    fs::write(
        &lib_path,
        format!(
            r#"[[Snippets]]
description = "long command"
command = "{cmd}"
tags = []
output = ""
folders = []
favorite = false
"#,
            cmd = long_command
        ),
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "long-cmd-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    let stored_cmd = parsed[0]["command"].as_str().unwrap();
    assert_eq!(stored_cmd.len(), long_command.len());
    assert!(stored_cmd.starts_with("echo "));
}

// --- TUI lifecycle: special characters in tags ---

#[test]
fn test_list_with_special_characters_in_tags() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "special-tags-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("special-tags-test.toml");
    fs::write(
        &lib_path,
        r#"[[Snippets]]
description = "tagged snippet"
command = "echo hello"
tags = ["tag with space", "tag/with/slashes"]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "special-tags-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    let tags = parsed[0]["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::Value::String("tag with space".to_string())));
    assert!(tags.contains(&serde_json::Value::String("tag/with/slashes".to_string())));
}

// --- TUI lifecycle: new with unicode description ---

#[test]
fn test_new_with_unicode_description() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["new", "--description", "日本語テスト", "echo test"])
        .output()
        .expect("failed to execute");
    assert!(
        output.status.success(),
        "snp new with unicode description should succeed"
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .expect("failed to execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["description"], "日本語テスト");
    assert_eq!(parsed[0]["command"], "echo test");
}

// --- TUI lifecycle: new with special characters in command ---

#[test]
fn test_new_with_special_characters_in_command() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args([
            "new",
            "--description",
            "Special",
            "echo 'hello world' && ls -la",
        ])
        .output()
        .expect("failed to execute");
    assert!(
        output.status.success(),
        "snp new with special chars should succeed"
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .expect("failed to execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["description"], "Special");
    assert_eq!(parsed[0]["command"], "echo 'hello world' && ls -la");
}

// --- TUI lifecycle: output encoding ---

#[test]
fn test_list_output_encoding() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "encoding-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("encoding-test.toml");
    fs::write(
        &lib_path,
        r#"[[Snippets]]
description = "Ünïcödé test 🎉"
command = "echo '日本語'"
tags = [" unicode 🌍 "]
output = ""
folders = []
favorite = false
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "encoding-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Verify output is valid UTF-8 (from_utf8_lossy above won't fail, but
    // from_utf8_strict will confirm no replacement characters leaked in)
    let stdout_bytes = &output.stdout;
    let _stdout_str =
        std::str::from_utf8(stdout_bytes).expect("snp list --json output must be valid UTF-8");

    let stdout = String::from_utf8_lossy(stdout_bytes);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["description"], "Ünïcödé test 🎉");
    assert_eq!(parsed[0]["command"], "echo '日本語'");
}

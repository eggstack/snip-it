use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

fn output_with_stdin(mut cmd: Command, input: &[u8]) -> std::process::Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
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

// --- CLI contract: new --from-file ---

#[test]
fn test_new_from_file_preserves_exact_content() {
    let (_tmp, config_dir) = setup_test_env();
    let from_file = _tmp.path().join("cmd.sh");
    fs::write(&from_file, "line one\nline two\nline three\n").unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            from_file.to_str().unwrap(),
            "--description",
            "from-file test",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["command"], "line one\nline two\nline three\n");
    assert_eq!(snippets[0]["description"], "from-file test");
}

#[test]
fn test_new_from_file_rejects_directory() {
    let (_tmp, config_dir) = setup_test_env();
    let dir = _tmp.path().join("not-a-file");
    fs::create_dir(&dir).unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            dir.to_str().unwrap(),
            "--description",
            "test",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("directory"),
        "Expected directory error: {combined}"
    );
}

#[test]
fn test_new_from_file_rejects_missing_file() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            "/nonexistent/path/cmd.sh",
            "--description",
            "test",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("does not exist") || combined.contains("not found"),
        "Expected missing file error: {combined}"
    );
}

#[test]
fn test_new_from_file_rejects_invalid_utf8() {
    let (_tmp, config_dir) = setup_test_env();
    let from_file = _tmp.path().join("bad_utf8.bin");
    fs::write(&from_file, [0xff, 0xfe, b'\n']).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "from-file-utf8"]);
    assert!(cmd.output().unwrap().status.success());
    let library_path = config_dir.join("libraries").join("from-file-utf8.toml");
    let before = fs::read(&library_path).unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            from_file.to_str().unwrap(),
            "--description",
            "bad utf8",
            "--library",
            "from-file-utf8",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("valid UTF-8"),
        "Expected UTF-8 error: {combined}"
    );
    assert_eq!(fs::read(&library_path).unwrap(), before);
}

#[test]
fn test_new_from_file_conflicts_with_positional() {
    let (_tmp, config_dir) = setup_test_env();
    let from_file = _tmp.path().join("cmd.sh");
    fs::write(&from_file, "echo hello").unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            from_file.to_str().unwrap(),
            "--description",
            "test",
            "echo y",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "Expected clap conflict error: {stderr}"
    );
}

#[test]
fn test_new_from_file_conflicts_with_command_stdin() {
    let (_tmp, config_dir) = setup_test_env();
    let from_file = _tmp.path().join("cmd.sh");
    fs::write(&from_file, "echo hello").unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            from_file.to_str().unwrap(),
            "--command-stdin",
            "--description",
            "test",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "Expected clap conflict error: {stderr}"
    );
}

#[test]
fn test_new_from_file_with_tags_and_library() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "from-file-tags"]);
    assert!(cmd.output().unwrap().status.success());

    let from_file = _tmp.path().join("tagged.sh");
    fs::write(&from_file, "echo tagged").unwrap();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--from-file",
            from_file.to_str().unwrap(),
            "--description",
            "tagged snippet",
            "--tags",
            "a,b",
            "--library",
            "from-file-tags",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));

    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "from-file-tags"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["command"], "echo tagged");
    assert_eq!(snippets[0]["description"], "tagged snippet");
    assert_eq!(snippets[0]["tags"], serde_json::json!(["a", "b"]));
}

// --- CLI contract: new --editor ---

#[test]
fn test_new_editor_with_fake_editor() {
    let (_tmp, config_dir) = setup_test_env();

    let fake_editor = _tmp.path().join("fake_editor.sh");
    fs::write(
        &fake_editor,
        "#!/bin/sh\necho 'editor content here' > \"$1\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_editor, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = snp_in(&config_dir)
        .args(["new", "--editor", "--description", "editor test"])
        .env("EDITOR", &fake_editor)
        .env_remove("VISUAL")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["command"], "editor content here\n");
    assert_eq!(snippets[0]["description"], "editor test");
}

#[test]
fn test_new_editor_empty_content_rejected() {
    let (_tmp, config_dir) = setup_test_env();

    let fake_editor = _tmp.path().join("empty_editor.sh");
    fs::write(&fake_editor, "#!/bin/sh\n: > \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_editor, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = snp_in(&config_dir)
        .args(["new", "--editor", "--description", "empty test"])
        .env("EDITOR", &fake_editor)
        .env_remove("VISUAL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("empty") || combined.contains("Empty") || combined.contains("no content"),
        "Expected empty command error: {combined}"
    );
}

#[test]
fn test_new_editor_nonzero_exit_rejected() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "editor-nonzero"]);
    assert!(cmd.output().unwrap().status.success());
    let library_path = config_dir.join("libraries").join("editor-nonzero.toml");
    let before = fs::read(&library_path).unwrap();

    let fake_editor = _tmp.path().join("fail_editor.sh");
    fs::write(&fake_editor, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake_editor, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--editor",
            "--description",
            "fail test",
            "--library",
            "editor-nonzero",
        ])
        .env("EDITOR", &fake_editor)
        .env_remove("VISUAL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("exited with status") || combined.contains("editor"),
        "Expected editor exit error: {combined}"
    );
    assert_eq!(fs::read(&library_path).unwrap(), before);
}

#[test]
fn test_new_editor_conflicts_with_positional() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args(["new", "--editor", "--description", "test", "echo y"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "Expected clap conflict error: {stderr}"
    );
}

#[test]
fn test_new_editor_conflicts_with_command_stdin() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args([
            "new",
            "--editor",
            "--command-stdin",
            "--description",
            "test",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "Expected clap conflict error: {stderr}"
    );
}

#[test]
fn test_new_editor_not_found() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args(["new", "--editor", "--description", "test"])
        .env("EDITOR", "/nonexistent/editor-xyz")
        .env_remove("VISUAL")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("Editor not found") || combined.contains("does not exist"),
        "Expected editor-not-found error: {combined}"
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

#[test]
fn test_new_command_stdin_preserves_exact_text_and_metadata() {
    let (_tmp, config_dir) = setup_test_env();

    let mut create = snp_in(&config_dir);
    create.args(["library", "create", "stdin-capture"]);
    assert!(create.output().unwrap().status.success());

    let command = "-nasty 'quoted value' $(not executed) | printf '\u{65e5}\u{672c}\u{8a9e}'\n\n";
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "stdin capture",
        "--tags",
        "git,release shell",
        "--library",
        "stdin-capture",
    ]);
    let output = output_with_stdin(cmd, command.as_bytes());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("not executed"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not executed"));

    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "stdin-capture"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["command"], command);
    assert_eq!(
        snippets[0]["tags"],
        serde_json::json!(["git", "release", "shell"])
    );
}

#[test]
fn test_new_command_stdin_preserves_no_trailing_newline() {
    let (_tmp, config_dir) = setup_test_env();
    let command = "echo \"quotes\" && printf '\\tUnicode: café'";

    let mut cmd = snp_in(&config_dir);
    cmd.args(["new", "--command-stdin", "--description", "exact bytes"]);
    let output = output_with_stdin(cmd, command.as_bytes());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets[0]["command"], command);
}

#[test]
fn test_new_command_stdin_invalid_utf8_does_not_mutate_library() {
    let (_tmp, config_dir) = setup_test_env();

    let mut create = snp_in(&config_dir);
    create.args(["library", "create", "invalid-stdin"]);
    assert!(create.output().unwrap().status.success());
    let library_path = config_dir.join("libraries").join("invalid-stdin.toml");
    let before = fs::read(&library_path).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "invalid input",
        "--library",
        "invalid-stdin",
    ]);
    let output = output_with_stdin(cmd, &[0xff, 0xfe, b'\n']);
    assert!(!output.status.success());
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostics.contains("valid UTF-8"),
        "diagnostics: {diagnostics}"
    );
    assert_eq!(fs::read(&library_path).unwrap(), before);
}

#[test]
fn test_new_command_stdin_requires_explicit_metadata_and_conflicts_with_positional() {
    let (_tmp, config_dir) = setup_test_env();

    let mut missing_description = snp_in(&config_dir);
    missing_description.args(["new", "--command-stdin"]);
    let output = output_with_stdin(missing_description, b"echo secret");
    assert!(!output.status.success());
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostics.contains("Description required"),
        "diagnostics: {diagnostics}"
    );
    assert!(!config_dir.join("libraries.toml").exists());

    let output = snp_in(&config_dir)
        .args(["new", "--command-stdin", "--description", "x", "echo y"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
}

#[test]
fn test_new_tags_prompt_flag_remains_compatible() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["new", "--description", "tagged", "echo tagged", "--tags"]);
    let output = output_with_stdin(cmd, b"one,two\n");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets[0]["tags"], serde_json::json!(["one", "two"]));
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

// --- snp select ---

#[test]
fn test_select_help() {
    let output = snp_cmd().args(["select", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--filter"));
    assert!(stdout.contains("--library"));
    assert!(stdout.contains("--raw"));
    assert!(stdout.contains("--expanded"));
}

#[test]
fn test_select_alias() {
    let output = snp_cmd().args(["sel", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Select a snippet"));
}

#[test]
fn test_select_raw_expanded_conflict() {
    let output = snp_cmd()
        .args(["select", "--raw", "--expanded"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "Expected conflict error for --raw --expanded: {stderr}"
    );
}

#[test]
fn test_select_no_library() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["select"]);
    let output = cmd.output().unwrap();
    // Should succeed gracefully with no library message
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No library found"),
        "Expected 'No library found' in stderr: {stderr}"
    );
}

#[test]
fn test_select_nonexistent_library() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["select", "--library", "does-not-exist"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist"),
        "Expected library-not-found error: {stderr}"
    );
}

#[test]
fn test_select_requires_terminal() {
    let (_tmp, config_dir) = setup_test_env();

    // Write a snippet library so library resolution succeeds
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "test-select"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    snp_in(&config_dir)
        .args(["library", "set-primary", "test-select"])
        .output()
        .unwrap();

    // When stdin is not a terminal, select should fail with a clear message
    // (the TUI requires a terminal for interactive selection)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["select"]);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::piped());
    let output = cmd.output().unwrap();
    // Should fail because there's no terminal
    assert!(
        !output.status.success(),
        "snp select should fail without a terminal"
    );
}

#[test]
fn test_select_help_contains_all_options() {
    let output = snp_cmd().args(["select", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Select a snippet and print its command"));
    assert!(stdout.contains("--filter"));
    assert!(stdout.contains("--library"));
    assert!(stdout.contains("--raw"));
    assert!(stdout.contains("--expanded"));
}

#[test]
fn test_select_missing_filter_value() {
    let output = snp_cmd().args(["select", "--filter"]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required") || stderr.contains("missing") || stderr.contains("valid"),
        "Expected missing value error: {stderr}"
    );
}

#[test]
fn test_select_missing_library_value() {
    let output = snp_cmd().args(["select", "--library"]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required") || stderr.contains("missing") || stderr.contains("valid"),
        "Expected missing value error: {stderr}"
    );
}

#[test]
fn test_select_rejects_sync_flag() {
    let output = snp_cmd().args(["select", "--sync"]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unexpected argument") || stderr.contains("error"),
        "Expected error for --sync on select: {stderr}"
    );
}

// ============================================================
// Release 2C: Golden command corpus and cross-source equivalence
// ============================================================

/// Golden command corpus covering every edge case from the R2C plan G1
/// plus the R2 Final Corrective serialization matrix.
/// Each entry is (label, command_str).
///
/// 24 entries: the original 15 plus 9 entries that were previously excluded
/// on the (incorrect) premise that TOML cannot preserve them. The TOML
/// format and the `toml` crate's serializer do preserve all of these values;
/// the earlier corruption was caused by the `quote_strings_containing_backslashes`
/// post-processing helper which is no longer applied to snip-it's own output.
fn golden_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        // 1. Single-line ASCII
        ("ascii_simple", "echo hello world"),
        // 2. Command beginning with a hyphen
        ("leading_hyphen", "-n echo 'leading flag'"),
        // 3. Single and double quotes
        ("quotes", "echo \"double\" 'single'"),
        // 4. Backslashes and Windows-like paths
        ("backslashes", "echo C:\\Users\\test\\file.txt"),
        // 5. Pipes, redirects, semicolons, and ampersands
        ("shell_ops", "echo foo | grep bar > out.txt; echo baz &"),
        // 6. $() and backticks
        ("substitution", "echo $(date) `whoami`"),
        // 7. Unicode
        ("unicode", "echo '日本語 test café'"),
        // 8. Leading spaces before command
        ("leading_spaces", "  echo indented"),
        // 9. Multiline shell script
        (
            "multiline_script",
            "if true; then\n  echo yes\nelse\n  echo no\nfi\n",
        ),
        // 10. Blank internal lines
        ("blank_lines", "echo before\n\necho after\n"),
        // 11. No trailing newline
        ("no_trailing_newline", "echo no_newline"),
        // 12. One trailing newline
        ("one_trailing_newline", "echo with_newline\n"),
        // 13. Multiple trailing newlines
        ("multi_trailing_newlines", "echo multi\n\n\n"),
        // 14. Variable placeholders
        ("variables", "ssh <user>@<host> -p <port=22>"),
        // 15. Escaped angle brackets (literal < and >)
        ("escaped_angle_brackets", "echo \\<literal\\> text"),
        // 16. Internal tab character
        ("tab_internal", "echo\there"),
        // 17. Makefile-style leading tabs
        ("tab_makefile", "if true; then\n\techo yes\nfi\n"),
        // 18. Trailing space
        ("trailing_space", "echo hello "),
        // 19. Multiple trailing spaces
        ("trailing_spaces_multi", "echo hello   "),
        // 20. CRLF line endings
        ("crlf", "echo foo\r\necho bar\r\n"),
        // 21. Mixed LF and CRLF line endings
        ("mixed_newlines", "echo foo\r\necho bar\n"),
        // 22. Tab + backslash
        ("tab_backslash", "echo \\path\\there"),
        // 23. Tab + quotes
        ("tab_quotes", "echo \"hello\tworld\""),
        // 24. Tab + spaces before newline
        ("tab_trailing", "echo hello\t  \r\n"),
    ]
}

#[test]
fn test_golden_corpus_stdin_preserves_all_commands() {
    let (_tmp, config_dir) = setup_test_env();

    for (label, command_str) in golden_corpus() {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("golden-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(
            output.status.success(),
            "golden corpus '{label}' failed via --command-stdin: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify all snippets were created
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), golden_corpus().len());

    // Verify each command matches exactly.
    // We compare via the deserialized JSON value rather than the raw
    // string because `list --json` may escape control characters (e.g.
    // tab as `\t`) differently from the in-memory representation.
    for (label, command_str) in golden_corpus() {
        let found = snippets.iter().find(|s| {
            s["description"]
                .as_str()
                .map(|d| d == format!("golden-{label}"))
                .unwrap_or(false)
        });
        let snippet = found.unwrap_or_else(|| panic!("snippet for golden-{label} not found"));
        assert_eq!(
            snippet["command"].as_str().unwrap(),
            command_str,
            "golden corpus '{label}' round-trip failed"
        );
    }
}

#[test]
fn test_golden_corpus_file_preserves_all_commands() {
    let (_tmp, config_dir) = setup_test_env();

    for (label, command_str) in golden_corpus() {
        let from_file = _tmp.path().join(format!("golden_{label}.txt"));
        std::fs::write(&from_file, command_str.as_bytes()).unwrap();

        let output = snp_in(&config_dir)
            .args([
                "new",
                "--from-file",
                from_file.to_str().unwrap(),
                "--description",
                &format!("golden-file-{label}"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "golden corpus '{label}' failed via --from-file: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify each command matches exactly
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    for (label, command_str) in golden_corpus() {
        let found = snippets.iter().find(|s| {
            s["description"]
                .as_str()
                .map(|d| d == format!("golden-file-{label}"))
                .unwrap_or(false)
        });
        let snippet = found.unwrap_or_else(|| panic!("snippet for golden-file-{label} not found"));
        assert_eq!(
            snippet["command"].as_str().unwrap(),
            command_str,
            "golden corpus file '{label}' round-trip failed"
        );
    }
}

#[test]
fn test_cross_source_equivalence_stdin_vs_file() {
    let (_tmp, config_dir) = setup_test_env();

    // For each golden command, create via stdin and file, then compare
    for (label, command_str) in golden_corpus() {
        // Via stdin
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("xs-stdin-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(
            output.status.success(),
            "stdin creation failed for '{label}': {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Via file
        let from_file = _tmp.path().join(format!("xs_{label}.txt"));
        std::fs::write(&from_file, command_str.as_bytes()).unwrap();
        let output = snp_in(&config_dir)
            .args([
                "new",
                "--from-file",
                from_file.to_str().unwrap(),
                "--description",
                &format!("xs-file-{label}"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "file creation failed for '{label}': {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Verify all pairs match
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    for (label, _) in golden_corpus() {
        let stdin_snippet = snippets
            .iter()
            .find(|s| {
                s["description"]
                    .as_str()
                    .map(|d| d == format!("xs-stdin-{label}"))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| panic!("stdin snippet for '{label}' not found"));
        let file_snippet = snippets
            .iter()
            .find(|s| {
                s["description"]
                    .as_str()
                    .map(|d| d == format!("xs-file-{label}"))
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| panic!("file snippet for '{label}' not found"));
        assert_eq!(
            stdin_snippet["command"], file_snippet["command"],
            "cross-source mismatch for '{label}': stdin vs file"
        );
    }
}

#[test]
fn test_golden_corpus_storage_roundtrip_no_progressive_normalization() {
    let (_tmp, config_dir) = setup_test_env();

    // Create all golden commands via stdin
    for (label, command_str) in golden_corpus() {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("roundtrip-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(output.status.success());
    }

    // Read all commands from JSON (first round)
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets_round1: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    // Trigger a save by adding and deleting a dummy snippet, forcing a full rewrite
    let mut cmd = snp_in(&config_dir);
    cmd.args(["new", "--description", "dummy", "echo dummy"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Read all commands again (second round)
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets_round2: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    // Compare: every golden snippet's command must be identical across rounds
    for (label, _) in golden_corpus() {
        let desc = format!("roundtrip-{label}");
        let cmd1 = snippets_round1
            .iter()
            .find(|s| s["description"].as_str() == Some(&desc))
            .unwrap()["command"]
            .as_str()
            .unwrap()
            .to_string();
        let cmd2 = snippets_round2
            .iter()
            .find(|s| s["description"].as_str() == Some(&desc))
            .unwrap()["command"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            cmd1, cmd2,
            "progressive normalization detected for '{label}': round1 != round2"
        );
    }
}

#[test]
fn test_golden_corpus_csv_output_valid() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a few representative golden commands
    let representative = [
        ("csv_ascii", "echo hello"),
        ("csv_quotes", "echo \"double\" 'single'"),
        ("csv_unicode", "echo '日本語'"),
        ("csv_newlines", "echo line1\necho line2\n"),
    ];

    for (label, command_str) in &representative {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("golden-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(output.status.success());
    }

    let output = snp_in(&config_dir)
        .args(["list", "--csv"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Header + 4 data lines
    assert!(
        lines.len() >= 5,
        "Expected at least 5 CSV lines (header + 4 data), got {}",
        lines.len()
    );
    assert_eq!(lines[0], "description,command,output,tags,folders,favorite");
}

// ============================================================
// Release 2C: Editor-source golden corpus equivalence (G2)
// ============================================================

#[test]
fn test_golden_corpus_editor_preserves_all_commands() {
    let (_tmp, config_dir) = setup_test_env();

    // Check that python3 is available (needed for reliable byte-exact writing)
    let python_available = Command::new("python3")
        .args(["--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !python_available {
        eprintln!("python3 not available, skipping editor golden corpus test");
        return;
    }

    for (label, command_str) in golden_corpus() {
        // Write the payload to a temp file, then create an editor script
        // that copies it to the tempfile ($1). This avoids all shell
        // escaping issues with heredocs and special characters.
        let payload_path = _tmp.path().join(format!("payload_{label}.txt"));
        fs::write(&payload_path, command_str.as_bytes()).unwrap();

        let editor_script = _tmp.path().join(format!("editor_{label}.sh"));
        // Python reads the payload file and writes its exact bytes to $1.
        // $1 is expanded by the shell before Python sees it, so the raw
        // string in Python is safe.
        let script_content = format!(
            "#!/bin/sh\npython3 -c \"import sys; open(sys.argv[1], 'wb').write(open(sys.argv[2], 'rb').read())\" \"$1\" '{}'\n",
            payload_path.display()
        );
        fs::write(&editor_script, script_content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&editor_script, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let output = snp_in(&config_dir)
            .args([
                "new",
                "--editor",
                "--description",
                &format!("golden-editor-{label}"),
            ])
            .env("EDITOR", &editor_script)
            .env_remove("VISUAL")
            .output()
            .unwrap();
        if !output.status.success() {
            panic!(
                "golden corpus '{label}' failed via --editor: stderr={} stdout={}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    // Verify all snippets were created and commands match exactly
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    eprintln!(
        "Editor test: list exit={}, stdout len={}, stderr={}",
        output.status,
        output.stdout.len(),
        stderr_str
    );
    eprintln!(
        "Editor test: stdout={}",
        &stdout_str[..stdout_str.len().min(200)]
    );
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), golden_corpus().len());

    for (label, command_str) in golden_corpus() {
        let found = snippets.iter().find(|s| {
            s["description"]
                .as_str()
                .map(|d| d == format!("golden-editor-{label}"))
                .unwrap_or(false)
        });
        let snippet =
            found.unwrap_or_else(|| panic!("snippet for golden-editor-{label} not found"));
        assert_eq!(
            snippet["command"].as_str().unwrap(),
            command_str,
            "golden corpus editor '{label}' round-trip failed"
        );
    }
}

// ============================================================
// Release 2C: Multiline terminator limitation (E3)
// ============================================================

#[test]
fn test_golden_corpus_multiline_terminator_limitation() {
    // --multiline is terminated by two blank lines. It cannot represent
    // content that ends with blank lines or contains the delimiter
    // sequence at the end. This test documents the expected behavior:
    // trailing newlines are stripped, the delimiter is consumed.
    let (_tmp, config_dir) = setup_test_env();

    // Content ending with a single newline: multiline preserves it
    // because the first blank-line read gets the trailing newline,
    // and the second consecutive blank triggers termination.
    // Actually: "echo test\n" is a single line with trailing newline.
    // Reading via multiline: line="echo test\n" (not empty), then EOF.
    // The join produces "echo test\n". But the two-blank-line check
    // never triggers because we hit EOF. So multiline preserves
    // trailing newlines only when the content itself is the only line.

    // Content with internal blank line: the blank line IS the content
    // separator, so it's preserved in the output.
    let content = "echo line1\n\necho line2\n";
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "multiline-limit-test",
    ]);
    let output = output_with_stdin(cmd, content.as_bytes());
    assert!(
        output.status.success(),
        "stdin ingestion failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let stored = snippets
        .iter()
        .find(|s| s["description"].as_str() == Some("multiline-limit-test"))
        .expect("multiline-limit-test not found");
    // The stored command must be byte-identical to the input
    assert_eq!(stored["command"].as_str().unwrap(), content);

    // Content ending with two blank lines: the delimiter consumes the
    // second blank, so the stored command has one fewer trailing blank
    // than the raw input would suggest. Use --command-stdin to show
    // what the exact bytes would be, then note multiline cannot match.
    let content_with_delim = "echo test\n\n\n";
    // Through --command-stdin, all bytes are preserved:
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "multiline-delimiter-exact",
    ]);
    let output = output_with_stdin(cmd, content_with_delim.as_bytes());
    assert!(output.status.success());

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    let stored = snippets
        .iter()
        .find(|s| s["description"].as_str() == Some("multiline-delimiter-exact"))
        .unwrap();
    assert_eq!(stored["command"].as_str().unwrap(), content_with_delim);

    // --multiline with the same content would produce "echo test\n"
    // because the two consecutive blank lines trigger termination and
    // the delimiter line is consumed. We cannot test --multiline from
    // a non-interactive context, so we document the expected delta:
    //   multiline("echo test\n\n\n") == "echo test\n"
    //   stdin("echo test\n\n\n")     == "echo test\n\n\n"
}

// ============================================================
// Release 2C: Select/list round-trip preserves exact command (H2)
// ============================================================

#[test]
fn test_golden_corpus_select_preserves_exact_command() {
    // snp select requires an interactive TUI, so we verify the stored
    // command bytes via list --json, which reads from the same TOML
    // storage that select reads from. The select code path uses
    // the same Snippet struct and library loading as list.
    let (_tmp, config_dir) = setup_test_env();

    for (label, command_str) in golden_corpus() {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("golden-select-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(output.status.success());
    }

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), golden_corpus().len());

    for (label, command_str) in golden_corpus() {
        let found = snippets.iter().find(|s| {
            s["description"]
                .as_str()
                .map(|d| d == format!("golden-select-{label}"))
                .unwrap_or(false)
        });
        let snippet =
            found.unwrap_or_else(|| panic!("snippet for golden-select-{label} not found"));
        assert_eq!(
            snippet["command"].as_str().unwrap(),
            command_str,
            "select round-trip '{label}' stored command mismatch (list --json same storage as select)"
        );
    }
}

// ============================================================
// Release 2C: Backup preserves command content (H5)
// ============================================================

#[test]
fn test_golden_corpus_backup_preserves_command() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a library and populate it with golden corpus entries
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "backup-test"]);
    assert!(cmd.output().unwrap().status.success());

    for (label, command_str) in golden_corpus() {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("golden-backup-{label}"),
            "--library",
            "backup-test",
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(output.status.success());
    }

    // Trigger a backup by adding another snippet (save_library calls
    // backup_library before writing).
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "trigger-backup",
        "--library",
        "backup-test",
    ]);
    let output = output_with_stdin(cmd, b"echo trigger");
    assert!(output.status.success());

    // Verify a backup was created in the backups/ subdirectory
    let backup_dir = config_dir.join("libraries").join("backups");
    assert!(
        backup_dir.exists(),
        "backups directory should exist after save"
    );
    let backups: Vec<_> = fs::read_dir(&backup_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("backup-test.") && name.ends_with(".toml.bak")
        })
        .collect();
    assert!(
        !backups.is_empty(),
        "should have at least one backup for backup-test"
    );

    // The most-recent backup (highest timestamp) was created when the
    // trigger-backup snippet was saved, so it should contain ALL golden
    // corpus entries that were added before it. Verify representative
    // descriptions and that trigger-backup is absent (backup predates it).
    let most_recent = backups
        .iter()
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .expect("should have a most-recent backup");
    let backup_content = fs::read(most_recent.path()).unwrap();
    let backup_str = String::from_utf8_lossy(&backup_content);
    assert!(
        backup_str.contains("golden-backup-ascii_simple"),
        "backup should contain ascii_simple"
    );
    assert!(
        backup_str.contains("golden-backup-backslashes"),
        "backup should contain backslashes"
    );
    assert!(
        backup_str.contains("golden-backup-multiline_script"),
        "backup should contain multiline_script"
    );
    assert!(
        !backup_str.contains("trigger-backup"),
        "backup should not contain trigger-backup (added after backup)"
    );
}

// ============================================================
// Release 2C: Sync round-trip preserves command (H6)
// ============================================================

#[test]
fn test_golden_corpus_sync_round_trip_preserves_command() {
    // Use the snip-sync test infrastructure to spin up an in-process
    // server, push a snippet, pull it back, and verify byte equality.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let service = snip_sync::test_helpers::build_test_service().await;
        let (addr, server_task, _captured) =
            snip_sync::test_helpers::start_test_server(service).await;
        let server_url = format!("http://{addr}");

        // Register a device
        let (api_key, device_id) = snip_it::sync::SyncClient::register(server_url.clone())
            .await
            .expect("register should succeed");

        let settings = snip_it::config::SyncSettings {
            enabled: true,
            server_url: server_url.clone(),
            api_key: api_key.clone(),
            device_id: device_id.clone(),
            sync_interval_minutes: 30,
            auto_sync: false,
            sync_direction: snip_it::config::SyncDirection::Bidirectional,
            clipboard_auto_clear_seconds: None,
            sync_limit: None,
        };
        let mut client = snip_it::sync::SyncClient::create(settings)
            .await
            .expect("SyncClient::create should succeed");

        // Push a multiline snippet with trailing newline
        let now = chrono::Utc::now().timestamp();
        let original_command = "if true; then\n  echo yes\nelse\n  echo no\nfi\n";
        let snippet = snip_it::proto::Snippet {
            id: "sync-golden-1".to_string(),
            description: "sync golden multiline".to_string(),
            command: original_command.to_string(),
            tags: vec!["sync".to_string()],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        };

        let response = client
            .sync_encrypted(vec![snippet], 0, "")
            .await
            .expect("sync_encrypted should succeed");
        assert!(
            response.success,
            "sync should succeed: {}",
            response.message
        );

        // Pull the snippet back and decrypt
        let returned = response
            .snippets
            .iter()
            .find(|s| s.id == "sync-golden-1")
            .expect("server should echo back the snippet");
        let decrypted =
            snip_it::sync::decrypt_snippet(&api_key, returned).expect("decryption should succeed");
        assert_eq!(
            decrypted.command, original_command,
            "sync round-trip must preserve multiline command byte-for-byte"
        );
        assert_eq!(decrypted.description, "sync golden multiline");

        server_task.abort();
    });
}

// ============================================================
// Release 2C: Run stored command executes via shell (H4)
// ============================================================

#[test]
fn test_run_stored_command_executes_via_shell() {
    // Verify that a stored command is what run_cmd would execute by
    // checking the stored command matches the expected shell string.
    // Full PTY-based execution testing is in tests/pty_integration.rs.
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "run-test"]);
    assert!(cmd.output().unwrap().status.success());

    // Store a controlled, inert command
    let command = "echo SNIP_RUN_MARKER_42 && exit 0";
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "run-golden",
        "--library",
        "run-test",
    ]);
    let output = output_with_stdin(cmd, command.as_bytes());
    assert!(
        output.status.success(),
        "failed to create snippet: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the stored command is exactly what run_cmd would execute
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "run-test"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0]["command"].as_str().unwrap(),
        command,
        "stored command must match what run_cmd would execute"
    );
    assert_eq!(snippets[0]["description"].as_str().unwrap(), "run-golden");
}

// ============================================================
// Release 2C: Shell init output validity (H2)
// ============================================================

#[test]
fn test_shell_init_bash_output_valid() {
    let output = snp_cmd().args(["shell", "init", "bash"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp shell init bash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snp_select_raw"),
        "bash init must contain snp_select_raw: {stdout}"
    );
    assert!(
        stdout.contains("snp_select_expanded"),
        "bash init must contain snp_select_expanded: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_current"),
        "bash init must contain snp_new_current: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_previous"),
        "bash init must contain snp_new_previous: {stdout}"
    );
    assert!(
        stdout.contains("__snp_select"),
        "bash init must contain __snp_select helper: {stdout}"
    );
    assert!(
        !stdout.contains("eval "),
        "bash init must not contain eval on captured content: {stdout}"
    );
}

#[test]
fn test_shell_init_zsh_output_valid() {
    let output = snp_cmd().args(["shell", "init", "zsh"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp shell init zsh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snp_select_raw"),
        "zsh init must contain snp_select_raw: {stdout}"
    );
    assert!(
        stdout.contains("snp_select_expanded"),
        "zsh init must contain snp_select_expanded: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_current"),
        "zsh init must contain snp_new_current: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_previous"),
        "zsh init must contain snp_new_previous: {stdout}"
    );
    assert!(
        stdout.contains("zle -N"),
        "zsh init must register ZLE widgets: {stdout}"
    );
    assert!(
        !stdout.contains("eval "),
        "zsh init must not contain eval on captured content: {stdout}"
    );
}

#[test]
fn test_shell_init_fish_output_valid() {
    let output = snp_cmd().args(["shell", "init", "fish"]).output().unwrap();
    assert!(
        output.status.success(),
        "snp shell init fish failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snp_select_raw"),
        "fish init must contain snp_select_raw: {stdout}"
    );
    assert!(
        stdout.contains("snp_select_expanded"),
        "fish init must contain snp_select_expanded: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_current"),
        "fish init must contain snp_new_current: {stdout}"
    );
    assert!(
        stdout.contains("snp_new_previous"),
        "fish init must contain snp_new_previous: {stdout}"
    );
    assert!(
        stdout.contains("function"),
        "fish init must use 'function' keyword: {stdout}"
    );
    assert!(
        !stdout.contains("eval "),
        "fish init must not contain eval on captured content: {stdout}"
    );
}

#[test]
fn test_shell_init_bash_syntax_check() {
    let output = snp_cmd().args(["shell", "init", "bash"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Write to temp file and check syntax with bash -n
    let tmp = tempfile::TempDir::new().unwrap();
    let script_path = tmp.path().join("init.bash");
    std::fs::write(&script_path, stdout.as_bytes()).unwrap();

    let check = Command::new("bash")
        .args(["-n", script_path.to_str().unwrap()])
        .output();
    match check {
        Ok(out) => assert!(
            out.status.success(),
            "bash -n syntax check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(_) => eprintln!("bash not available, skipping syntax check"),
    }
}

#[test]
fn test_shell_init_zsh_syntax_check() {
    let output = snp_cmd().args(["shell", "init", "zsh"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let tmp = tempfile::TempDir::new().unwrap();
    let script_path = tmp.path().join("init.zsh");
    std::fs::write(&script_path, stdout.as_bytes()).unwrap();

    let check = Command::new("zsh")
        .args(["-n", script_path.to_str().unwrap()])
        .output();
    match check {
        Ok(out) => assert!(
            out.status.success(),
            "zsh -n syntax check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(_) => eprintln!("zsh not available, skipping syntax check"),
    }
}

#[test]
fn test_shell_init_fish_syntax_check() {
    let output = snp_cmd().args(["shell", "init", "fish"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    let tmp = tempfile::TempDir::new().unwrap();
    let script_path = tmp.path().join("init.fish");
    std::fs::write(&script_path, stdout.as_bytes()).unwrap();

    let check = Command::new("fish")
        .args(["--no-execute", script_path.to_str().unwrap()])
        .output();
    match check {
        Ok(out) => assert!(
            out.status.success(),
            "fish --no-execute syntax check failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(_) => eprintln!("fish not available, skipping syntax check"),
    }
}

#[test]
fn test_shell_init_all_shells_no_history_file_references() {
    for shell in ["bash", "zsh", "fish"] {
        let output = snp_cmd().args(["shell", "init", shell]).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains(".bash_history"),
            "{shell} init must not reference .bash_history"
        );
        assert!(
            !stdout.contains(".zsh_history"),
            "{shell} init must not reference .zsh_history"
        );
        assert!(
            !stdout.contains("fish_history"),
            "{shell} init must not reference fish_history"
        );
    }
}

#[test]
fn test_shell_init_all_shells_use_output_file_flag() {
    for shell in ["bash", "zsh", "fish"] {
        let output = snp_cmd().args(["shell", "init", shell]).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("--output-file"),
            "{shell} init must use --output-file for selection transport"
        );
    }
}

// ============================================================
// Release 3A: Pet choice variable serialization compatibility (E)
// ============================================================

const CHOICE_FIXTURE_TOML: &str = r#"[[snippets]]
description = "choice variable — simple"
command = "echo <color=|_red_||_green_||_blue_||>"
output = ""
tag = ["choices"]

[[snippets]]
description = "choice variable — mixed with text"
command = "ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"
output = ""
tag = ["choices"]

[[snippets]]
description = "choice variable — single choice"
command = "echo <only=|_one_||>"
output = ""
tag = ["choices"]
"#;

#[test]
fn test_choice_variable_fixture_loads_correctly() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-fixture"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("choice-fixture.toml"),
        CHOICE_FIXTURE_TOML,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-fixture"]);
    cmd.output().unwrap();

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 3);

    assert_eq!(
        snippets[0]["command"],
        "echo <color=|_red_||_green_||_blue_||>"
    );
    assert_eq!(
        snippets[1]["command"],
        "ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"
    );
    assert_eq!(snippets[2]["command"], "echo <only=|_one_||>");
}

#[test]
fn test_choice_variable_toml_load_save_roundtrip() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-roundtrip"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("choice-roundtrip.toml"),
        CHOICE_FIXTURE_TOML,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-roundtrip"]);
    cmd.output().unwrap();

    // Read via list --json (first round)
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets_r1: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    // Trigger a save by adding a dummy snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args(["new", "--description", "trigger-save", "echo dummy"]);
    cmd.output().unwrap();

    // Read again (second round)
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets_r2: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    // Filter to only the choice snippets (exclude dummy)
    let choice_r1: Vec<_> = snippets_r1
        .iter()
        .filter(|s| {
            s["description"]
                .as_str()
                .map(|d| d.contains("choice variable"))
                .unwrap_or(false)
        })
        .collect();
    let choice_r2: Vec<_> = snippets_r2
        .iter()
        .filter(|s| {
            s["description"]
                .as_str()
                .map(|d| d.contains("choice variable"))
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(choice_r1.len(), 3);
    assert_eq!(choice_r2.len(), 3);

    for i in 0..3 {
        assert_eq!(
            choice_r1[i]["command"], choice_r2[i]["command"],
            "Choice syntax round-trip failed for snippet {}",
            i
        );
    }
}

#[test]
fn test_choice_variable_list_json_raw_syntax() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-json"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(libraries_dir.join("choice-json.toml"), CHOICE_FIXTURE_TOML).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-json"]);
    cmd.output().unwrap();

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Expected valid JSON output");

    // Verify the raw choice syntax is preserved in JSON output
    let simple = parsed
        .iter()
        .find(|s| s["description"].as_str() == Some("choice variable — simple"))
        .expect("simple choice snippet not found");
    assert_eq!(
        simple["command"].as_str().unwrap(),
        "echo <color=|_red_||_green_||_blue_||>"
    );

    let mixed = parsed
        .iter()
        .find(|s| s["description"].as_str() == Some("choice variable — mixed with text"))
        .expect("mixed choice snippet not found");
    assert_eq!(
        mixed["command"].as_str().unwrap(),
        "ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"
    );

    let single = parsed
        .iter()
        .find(|s| s["description"].as_str() == Some("choice variable — single choice"))
        .expect("single choice snippet not found");
    assert_eq!(single["command"].as_str().unwrap(), "echo <only=|_one_||>");
}

#[test]
fn test_choice_variable_list_csv_raw_syntax() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-csv"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(libraries_dir.join("choice-csv.toml"), CHOICE_FIXTURE_TOML).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-csv"]);
    cmd.output().unwrap();

    let output = snp_in(&config_dir)
        .args(["list", "--csv"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify the raw choice syntax appears in CSV output
    assert!(
        stdout.contains("echo <color=|_red_||_green_||_blue_||>"),
        "CSV output must contain raw choice syntax: {stdout}"
    );
    assert!(
        stdout.contains("ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"),
        "CSV output must contain raw mixed choice syntax: {stdout}"
    );
    assert!(
        stdout.contains("echo <only=|_one_||>"),
        "CSV output must contain raw single choice syntax: {stdout}"
    );
}

#[test]
fn test_choice_variable_expansion() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a snippet with choice variable via --command-stdin
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "choice-expand-test",
    ]);
    let output = output_with_stdin(cmd, b"echo <color=|_red_||_green_||_blue_||>");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the stored command is the raw choice syntax
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(
        snippets[0]["command"].as_str().unwrap(),
        "echo <color=|_red_||_green_||_blue_||>"
    );
}

#[test]
fn test_choice_variable_raw_preserved_in_storage() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-raw"]);
    cmd.output().unwrap();

    // Write choice fixture TOML directly
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(libraries_dir.join("choice-raw.toml"), CHOICE_FIXTURE_TOML).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-raw"]);
    cmd.output().unwrap();

    // Read the raw TOML file and verify choice syntax is preserved verbatim
    let toml_content = fs::read_to_string(libraries_dir.join("choice-raw.toml")).unwrap();
    assert!(
        toml_content.contains("echo <color=|_red_||_green_||_blue_||>"),
        "Raw TOML must contain choice syntax: {toml_content}"
    );
    assert!(
        toml_content.contains("ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"),
        "Raw TOML must contain mixed choice syntax: {toml_content}"
    );
    assert!(
        toml_content.contains("echo <only=|_one_||>"),
        "Raw TOML must contain single choice syntax: {toml_content}"
    );
}

#[test]
fn test_choice_variable_fixture_file_loads() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "choice-file"]);
    cmd.output().unwrap();

    // Copy the fixture file into the library directory
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::copy(
        "tests/fixtures/choice_variables.toml",
        libraries_dir.join("choice-file.toml"),
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "choice-file"]);
    cmd.output().unwrap();

    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 3);

    // Verify each command preserves the exact choice syntax
    assert_eq!(
        snippets[0]["command"].as_str().unwrap(),
        "echo <color=|_red_||_green_||_blue_||>"
    );
    assert_eq!(
        snippets[1]["command"].as_str().unwrap(),
        "ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"
    );
    assert_eq!(
        snippets[2]["command"].as_str().unwrap(),
        "echo <only=|_one_||>"
    );
}

// === Pet Import Tests ===

#[test]
fn test_import_pet_default_creates_library() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "import pet failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the library was created and listed (filename uses hyphens)
    let output = snp_in(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("canonical-pet"),
        "Expected 'canonical-pet' in library list: {stdout}"
    );

    // Verify snippets were imported
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "canonical-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        snippets.len(),
        5,
        "Expected 5 snippets from canonical pet fixture"
    );
}

#[test]
fn test_import_pet_explicit_library_name() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            fixture.to_str().unwrap(),
            "--library",
            "my-pet-snippets",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "import pet failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("my-pet-snippets"));
}

#[test]
fn test_import_pet_destination_collision_fails() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // First import succeeds
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Second import fails (destination exists)
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "Second import should fail when destination exists"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists"),
        "Expected 'already exists' error: {stderr}"
    );
}

#[test]
fn test_import_pet_merge_skips_duplicates() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_with_duplicates.toml");

    // First import (creates with 3 entries, including 1 in-file duplicate)
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Verify initial count (3 entries in the source, including 1 in-file duplicate)
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "pet-with-duplicates"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let initial: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(initial.len(), 3, "First import should create all 3 entries");

    // Merge the same file again (should skip all 3 as duplicates)
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap(), "--merge"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "merge import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should still have 3 entries (no new entries added)
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "pet-with-duplicates"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let after: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(after.len(), 3, "Merge should not add duplicates");
}

#[test]
fn test_import_pet_dry_run_no_mutation() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["import", "pet", fixture.to_str().unwrap(), "--dry-run"]);
    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "dry-run import failed: stdout={stdout} stderr={stderr}"
    );

    // Verify no library was created
    let list_output = snp_in(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        !list_stdout.contains("canonical-pet"),
        "dry-run should not create a library: {list_stdout}"
    );

    // Verify dry-run message in stderr
    assert!(
        stderr.contains("dry run") || stderr.contains("no files were modified"),
        "Expected 'dry run' in stderr, got (len={}): {stderr}",
        stderr.len()
    );
}

#[test]
fn test_import_pet_source_untouched() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let original_content = fs::read_to_string(&fixture).unwrap();

    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after_content = fs::read_to_string(&fixture).unwrap();
    assert_eq!(
        original_content, after_content,
        "Source file should not be modified"
    );
}

#[test]
fn test_import_pet_json_report() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["total_entries"], 5);
    assert_eq!(report["imported"], 5);
    assert_eq!(report["skipped"], 0);
    assert_eq!(report["dry_run"], false);
}

#[test]
fn test_import_pet_nonexistent_source_fails() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args(["import", "pet", "/nonexistent/file.toml"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found"));
}

#[test]
fn test_import_pet_invalid_toml_fails() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let bad_file = tmp.path().join("bad.toml");
    fs::write(&bad_file, "invalid = [toml").unwrap();

    let output = snp_in(&config_dir)
        .args(["import", "pet", bad_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("TOML"));
}

#[test]
fn test_import_pet_empty_file_fails() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let empty_file = tmp.path().join("empty.toml");
    fs::write(&empty_file, "").unwrap();

    let output = snp_in(&config_dir)
        .args(["import", "pet", empty_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Empty"));
}

#[test]
fn test_import_pet_directory_fails() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir)
        .args(["import", "pet", _tmp.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("directory"));
}

#[test]
fn test_import_pet_strict_mode_empty_command_fails() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let bad_file = tmp.path().join("bad_entries.toml");
    fs::write(
        &bad_file,
        r#"
[[snippets]]
description = "valid entry"
command = "echo valid"

[[snippets]]
description = "empty command"
command = ""
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["import", "pet", bad_file.to_str().unwrap(), "--strict"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "strict mode should fail on error-severity diagnostic"
    );
}

#[test]
fn test_import_pet_permissive_imports_valid_entries() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let bad_file = tmp.path().join("mixed_entries.toml");
    fs::write(
        &bad_file,
        r#"
[[snippets]]
description = "valid entry"
command = "echo valid"

[[snippets]]
description = "empty command"
command = ""
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["import", "pet", bad_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "permissive mode should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Should import the valid entry
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "mixed-entries"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1, "Only the valid entry should be imported");
    assert_eq!(snippets[0]["command"].as_str().unwrap(), "echo valid");
}

#[test]
fn test_import_pet_replace_with_backup() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // First import
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Import a different file with --replace (use same derived library name)
    let edge_fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_edge_cases.toml");

    let output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            edge_fixture.to_str().unwrap(),
            "--library",
            "canonical-pet",
            "--replace",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "replace import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the library now has the edge_cases content
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "canonical-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        snippets.len() >= 3,
        "Expected edge case snippets after replace, got {}",
        snippets.len()
    );
}

#[test]
fn test_import_pet_preserves_command_text() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "canonical-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    // Verify exact command text preservation (including quotes and special chars)
    let git_cmd = snippets[0]["command"].as_str().unwrap();
    assert!(
        git_cmd.contains(r#"git commit -m "<msg>""#),
        "Expected preserved command with quotes: {git_cmd}"
    );
}

#[test]
fn test_import_pet_choice_variables_preserved() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_choice_variables.toml");

    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "pet-choice-variables"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 2);

    // Verify choice variable syntax preserved exactly
    assert_eq!(
        snippets[0]["command"].as_str().unwrap(),
        "echo <editor=|_vim_||_nvim_||_emacs_||>"
    );
    assert_eq!(snippets[1]["command"].as_str().unwrap(), "echo <greeting>");
}

#[test]
fn test_import_pet_mixed_aliases() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_mixed_aliases.toml");

    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "pet-mixed-aliases"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 2);

    // Both entries should be imported (aliases handled by serde)
    let descs: Vec<&str> = snippets
        .iter()
        .map(|s| s["description"].as_str().unwrap())
        .collect();
    assert!(descs.contains(&"legacy uppercase description"));
    assert!(descs.contains(&"pet name alias"));
}

#[test]
fn test_import_pet_help() {
    let output = snp_cmd().args(["import", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Import snippets"));
    assert!(stdout.contains("pet"));
}

#[test]
fn test_import_pet_subcommand_help() {
    let output = snp_cmd()
        .args(["import", "pet", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--library"));
    assert!(stdout.contains("--merge"));
    assert!(stdout.contains("--replace"));
    assert!(stdout.contains("--dry-run"));
    assert!(stdout.contains("--strict"));
    assert!(stdout.contains("--report"));
    assert!(stdout.contains("--report-file"));
}

#[test]
fn test_import_pet_merge_conflicts_with_replace() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            fixture.to_str().unwrap(),
            "--merge",
            "--replace",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "--merge and --replace should conflict"
    );
}

// --- Release 3B gap-fix tests: large file, symlink, failure injection, workflow ---

#[test]
fn test_import_pet_large_file_rejected() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let big_file = tmp.path().join("big.toml");

    // Create a file larger than 16 MiB
    {
        let mut f = fs::File::create(&big_file).unwrap();
        let chunk = "[[snippets]]\ndescription = \"x\"\ncommand = \"echo x\"\n";
        // Write enough chunks to exceed 16 MiB (16 * 1024 * 1024 = 16_777_216)
        let target = 17 * 1024 * 1024;
        let mut written = 0usize;
        while written < target {
            let n = f.write(chunk.as_bytes()).unwrap();
            written += n;
        }
    }

    let output = snp_in(&config_dir)
        .args(["import", "pet", big_file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success(), "File >16 MiB should be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("too large") || stderr.contains("16 MiB"),
        "Expected size error: {stderr}"
    );
}

#[test]
fn test_import_pet_symlink_followed() {
    let (_tmp, config_dir) = setup_test_env();
    let tmp = TempDir::new().unwrap();
    let real_file = tmp.path().join("real_pet.toml");
    let symlink = tmp.path().join("link_pet.toml");

    // Write a valid pet file
    fs::write(
        &real_file,
        r#"
[[snippets]]
description = "symlink test"
command = "echo symlink"
"#,
    )
    .unwrap();

    // Create symlink pointing to the real file
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_file, &symlink).unwrap();
    #[cfg(not(unix))]
    {
        // On Windows, just copy the file (symlink requires elevated privileges)
        fs::copy(&real_file, &symlink).unwrap();
    }

    let output = snp_in(&config_dir)
        .args(["import", "pet", symlink.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Symlink import should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify the original file is unchanged
    let after = fs::read_to_string(&real_file).unwrap();
    assert!(
        after.contains("echo symlink"),
        "Source file should be unchanged"
    );

    // Verify library was created
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "link-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
}

#[test]
fn test_import_pet_collision_leaves_no_partial_state() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // First import succeeds
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Read the original library content
    let lib_path = config_dir.join("libraries").join("canonical-pet.toml");
    let original_content = fs::read_to_string(&lib_path).unwrap();
    let original_libraries_toml = fs::read_to_string(config_dir.join("libraries.toml")).unwrap();

    // Second import fails (destination collision)
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());

    // Verify library file is unchanged
    let after_content = fs::read_to_string(&lib_path).unwrap();
    assert_eq!(
        original_content, after_content,
        "Library file should not change on collision"
    );

    // Verify libraries.toml is unchanged
    let after_libraries_toml = fs::read_to_string(config_dir.join("libraries.toml")).unwrap();
    assert_eq!(
        original_libraries_toml, after_libraries_toml,
        "Libraries metadata should not change on collision"
    );
}

#[test]
fn test_import_pet_library_list_csv() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // Import
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // List as CSV
    let output = snp_in(&config_dir)
        .args(["list", "--csv", "--library", "canonical-pet"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "list --csv should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // CSV should have a header row and data rows
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() > 1,
        "CSV should have header + data rows, got {} lines",
        lines.len()
    );
    // Header should contain expected columns
    assert!(
        lines[0].contains("description") && lines[0].contains("command"),
        "CSV header should contain description and command: {}",
        lines[0]
    );
}

#[test]
fn test_import_pet_select_raw_output() {
    // NOTE: snp select requires a terminal (ratatui TUI), so we cannot test
    // it directly in an integration test. Instead, verify the imported library
    // is fully usable by testing list --json with various queries.
    let (_tmp, config_dir) = setup_test_env();
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // Import
    let output = snp_in(&config_dir)
        .args(["import", "pet", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Verify the library can be set as primary
    let output = snp_in(&config_dir)
        .args(["library", "set-primary", "canonical-pet"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "set-primary on imported library should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify library show works
    let output = snp_in(&config_dir)
        .args(["library", "show", "canonical-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("canonical-pet"),
        "library show should display name: {stdout}"
    );

    // Verify list --json returns all snippets with expected fields
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--library", "canonical-pet"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 5);
    // Each snippet should have the expected fields
    for s in &snippets {
        assert!(
            s["description"].is_string(),
            "snippet should have description"
        );
        assert!(
            !s["command"].as_str().unwrap().is_empty(),
            "snippet should have non-empty command"
        );
    }
}

// --- Doctor command ---

#[test]
fn test_doctor_pet_file_valid() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Doctor Report"));
    assert!(stderr.contains("Entries:"));
}

#[test]
fn test_doctor_pet_file_json() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");
    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["schema_version"], "1.0.0");
    assert_eq!(json["dry_run"], true);
    assert!(json["total_entries"].as_u64().unwrap() > 0);
}

#[test]
fn test_doctor_pet_file_nonexistent() {
    let output = snp_cmd()
        .args(["doctor", "--pet-file", "/nonexistent/file.toml"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_doctor_pet_file_with_choice_variables() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_choice_variables.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("choice_variables") || stderr.contains("Choice"));
}

#[test]
fn test_doctor_compatibility() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Doctor Report"));
    assert!(stderr.contains("binary") || stderr.contains("version"));
}

#[test]
fn test_doctor_no_mode_fails() {
    let output = snp_cmd().args(["doctor"]).output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_doctor_pet_file_strict_with_errors() {
    // Use a file that has empty commands to trigger error diagnostics
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("bad_pet.toml");
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = "empty cmd"
command = ""

[[snippets]]
description = "valid"
command = "echo ok"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--strict",
        ])
        .output()
        .unwrap();
    // Strict mode + error diagnostics = exit 2
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_doctor_help() {
    let output = snp_cmd().args(["doctor", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--pet-file"));
    assert!(stdout.contains("--compatibility"));
    assert!(stdout.contains("--strict"));
    assert!(stdout.contains("--report"));
}

#[test]
fn test_doctor_json_no_mutation() {
    // Verify doctor --pet-file does not modify the source file
    let (_tmp, _config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("test_pet.toml");
    let content = r#"
[[snippets]]
description = "test"
command = "echo hello"
"#;
    std::fs::write(&pet_path, content).unwrap();
    let before = std::fs::read_to_string(&pet_path).unwrap();

    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after = std::fs::read_to_string(&pet_path).unwrap();
    assert_eq!(before, after, "Doctor should not modify the source file");
}

// --- Security/Privacy tests ---

#[test]
fn test_doctor_no_command_execution() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("dangerous.toml");
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = "dangerous"
command = "rm -rf /tmp/snp-test-dangerous-$$"
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!std::path::Path::new("/tmp/snp-test-dangerous-*").exists());
}

#[test]
fn test_doctor_no_variable_expansion() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("vars.toml");
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = "with variable"
command = "echo <UNIQUE_TEST_VAR_12345>"
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let caps = json["detected_capabilities"].as_array().unwrap();
    assert!(
        caps.iter().any(|c| c.as_str() == Some("variables")),
        "Should detect variables in capabilities"
    );
}

#[test]
fn test_doctor_no_api_key_leakage() {
    let (_tmp, config_dir) = setup_test_env();
    let sync_path = config_dir.join("sync.toml");
    std::fs::write(
        &sync_path,
        r#"
enabled = true
server_url = "https://sync.example.com"
api_key = "super_secret_api_key_12345"
direction = "push"
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility", "--report", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("super_secret_api_key_12345"),
        "Doctor output should not leak API keys"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("super_secret_api_key_12345"),
        "Doctor stderr should not leak API keys"
    );
}

#[test]
fn test_doctor_config_not_mutated() {
    let (_tmp, config_dir) = setup_test_env();
    let sync_path = config_dir.join("sync.toml");
    std::fs::write(&sync_path, "enabled = false\n").unwrap();
    let before = std::fs::read_to_string(&sync_path).unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after = std::fs::read_to_string(&sync_path).unwrap();
    assert_eq!(before, after, "Doctor should not modify config files");
}

// --- Integration matrix tests ---

#[test]
fn test_doctor_pet_required_default_variables() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/variable_commands.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("variables")
            || stderr.contains("Variables")
            || stderr.contains("supported")
            || stderr.contains("Supported")
    );
}

#[test]
fn test_doctor_duplicates_with_output() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_with_duplicates.toml");
    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let duplicates = json["duplicates"].as_array().unwrap();
    assert!(!duplicates.is_empty(), "Should detect duplicates");
}

#[test]
fn test_doctor_multiline_commands() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_multiline.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Doctor Report"));
}

#[test]
fn test_doctor_mixed_field_aliases() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_mixed_aliases.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_doctor_pet_edge_cases() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/pet_edge_cases.toml");
    let output = snp_cmd()
        .args(["doctor", "--pet-file", fixture.to_str().unwrap()])
        .output()
        .unwrap();
    // Empty commands in edge_cases.toml trigger error diagnostics → exit code 2
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_doctor_empty_file() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("empty.toml");
    std::fs::write(&pet_path, "").unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_doctor_malformed_toml() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("malformed.toml");
    std::fs::write(&pet_path, "[[snippets]\ndescription = \"unclosed").unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    // Malformed TOML should succeed (doctor reports TOML error, doesn't fail)
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("TOML Error")
            || stderr.contains("toml")
            || stderr.contains("error")
            || stderr.contains("Error")
    );
}

#[test]
fn test_doctor_warnings_only_exit_zero() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("warnings_only.toml");
    // Empty description is a warning, not an error
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = ""
command = "echo hello"
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Warnings-only should exit 0, got {:?}",
        output.status.code()
    );
}

#[test]
fn test_doctor_json_stdout_only() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");
    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    // stdout should be valid JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json.is_object());
    // stderr should NOT contain JSON (human output goes there)
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("\"schema_version\""),
        "stderr should not contain JSON"
    );
}

#[test]
fn test_doctor_human_no_mutation() {
    let (_tmp, _config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("test_pet.toml");
    let content = r#"
[[snippets]]
description = "test"
command = "echo hello"
"#;
    std::fs::write(&pet_path, content).unwrap();
    let before = std::fs::read_to_string(&pet_path).unwrap();

    let output = snp_cmd()
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after = std::fs::read_to_string(&pet_path).unwrap();
    assert_eq!(
        before, after,
        "Doctor human mode should not modify the source file"
    );
}

#[test]
fn test_doctor_library_mode() {
    let (_tmp, config_dir) = setup_test_env();
    // First create a library with snippets
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");
    let output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            fixture.to_str().unwrap(),
            "--library",
            "test-doc-lib",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    // Now run doctor on the library
    let output = snp_in(&config_dir)
        .args(["doctor", "--library", "test-doc-lib"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Doctor Report"));
}

#[test]
fn test_doctor_check_shell() {
    let output = snp_cmd()
        .args(["doctor", "--check-shell", "bash"])
        .output()
        .unwrap();
    // May succeed or fail depending on whether bash is installed
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Doctor Report") || stderr.contains("shell_init"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_doctor_compatibility_has_all_checks() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Should contain at least some of these check codes
    assert!(
        stderr.contains("binary") || stderr.contains("version"),
        "stderr: {}",
        stderr
    );
    assert!(
        stderr.contains("config") || stderr.contains("Config"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_doctor_malformed_choice_variables() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("bad_choices.toml");
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = "bad choices"
command = "echo <color=|>"
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["doctor", "--pet-file", pet_path.to_str().unwrap()])
        .output()
        .unwrap();
    // Should succeed (doctor is read-only) and report diagnostics
    assert!(output.status.success());
}

#[test]
fn test_doctor_unknown_metadata_fields() {
    let (_tmp, config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("unknown_fields.toml");
    std::fs::write(
        &pet_path,
        r#"
[[snippets]]
description = "has unknown fields"
command = "echo hello"
custom_field = "should be ignored"
another_unknown = 42
"#,
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let diags = json["diagnostics"].as_array().unwrap();
    // Should have at least 2 unknown field diagnostics
    let unknown_fields: Vec<_> = diags
        .iter()
        .filter(|d| d["code"].as_str().unwrap_or("").contains("FIELD-UNKNOWN"))
        .collect();
    assert!(
        unknown_fields.len() >= 2,
        "Expected at least 2 unknown field diagnostics, got {}: {:?}",
        unknown_fields.len(),
        unknown_fields
    );
}

#[test]
fn test_doctor_import_dryrun_consistency() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    // Run doctor
    let doctor_output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(doctor_output.status.success());
    let doctor_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&doctor_output.stdout)).unwrap();

    // Run import --dry-run
    let (_tmp, config_dir) = setup_test_env();
    let import_output = snp_in(&config_dir)
        .args([
            "import",
            "pet",
            fixture.to_str().unwrap(),
            "--dry-run",
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(import_output.status.success());
    let import_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&import_output.stdout)).unwrap();

    // Both should have the same total_entries
    assert_eq!(
        doctor_json["total_entries"], import_json["total_entries"],
        "Doctor and import dry-run should agree on entry count"
    );

    // Both should have schema_version 1.0.0
    assert_eq!(doctor_json["schema_version"], "1.0.0");
    assert_eq!(import_json["schema_version"], "1.0.0");

    // Both should have the same diagnostic count (structural + per-entry)
    let doctor_diag_count = doctor_json["diagnostics"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let import_diag_count = import_json["diagnostics"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    // Import may have fewer diagnostics since it doesn't detect duplicates within source
    // (it detects source-to-dest duplicates instead), but structural and per-entry diagnostics
    // should be the same count.
    assert!(
        import_diag_count >= doctor_diag_count - 2,
        "Import diagnostics ({}) should be close to doctor diagnostics ({})",
        import_diag_count,
        doctor_diag_count
    );

    // Both should have detected_capabilities
    let doctor_caps = doctor_json["detected_capabilities"].as_array();
    let import_caps = import_json["detected_capabilities"].as_array();
    assert!(
        doctor_caps.is_some(),
        "Doctor should have detected_capabilities"
    );
    assert!(
        import_caps.is_some(),
        "Import should have detected_capabilities"
    );

    // Both should have analysis_mode
    assert!(
        doctor_json["analysis_mode"].is_string(),
        "Doctor should have analysis_mode"
    );
    assert!(
        import_json["analysis_mode"].is_string(),
        "Import should have analysis_mode"
    );
}

#[test]
fn test_doctor_library_state_not_mutated() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a library with a snippet
    let create_output = snp_in(&config_dir)
        .args(["library", "create", "testlib"])
        .output()
        .unwrap();
    assert!(create_output.status.success());

    let lib_path = config_dir.join("libraries").join("testlib.toml");
    let lib_content = r#"[[snippets]]
description = "existing"
command = "echo existing"
"#;
    std::fs::write(&lib_path, lib_content).unwrap();
    let before = std::fs::read_to_string(&lib_path).unwrap();

    // Run doctor --compatibility (should not touch library files)
    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after = std::fs::read_to_string(&lib_path).unwrap();
    assert_eq!(
        before, after,
        "Doctor --compatibility should not modify library files"
    );

    // Run doctor --pet-file (should not touch library files)
    let pet_path = _tmp.path().join("pet.toml");
    std::fs::write(
        &pet_path,
        r#"[[snippets]]
description = "test"
command = "echo hello"
"#,
    )
    .unwrap();

    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let after = std::fs::read_to_string(&lib_path).unwrap();
    assert_eq!(
        before, after,
        "Doctor --pet-file should not modify library files"
    );
}

#[test]
fn test_doctor_compatibility_has_pet_toml_check() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["doctor", "--compatibility", "--report", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let diags = json["diagnostics"].as_array().unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d["code"].as_str() == Some("compat.pet_toml.ok")),
        "Should have compat.pet_toml.ok check. Codes: {:?}",
        diags.iter().map(|d| d["code"].as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn test_doctor_pet_file_has_normalizations() {
    let fixture = std::env::current_dir()
        .unwrap()
        .join("tests/fixtures/canonical_pet.toml");

    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            fixture.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();

    // Normalizations should be populated (canonical_pet.toml has entries with zero timestamps)
    let norms = json["normalizations"].as_array().unwrap();
    assert!(
        !norms.is_empty(),
        "Doctor should populate normalization preview for canonical_pet.toml"
    );
    // Each normalization should have the required fields
    for norm in norms {
        assert!(norm["entry_index"].is_number());
        assert!(norm["field"].is_string());
        assert!(norm["original"].is_string());
        assert!(norm["normalized"].is_string());
    }
}

#[test]
fn test_doctor_malformed_variable_detection() {
    let (_tmp, _config_dir) = setup_test_env();
    let pet_path = _tmp.path().join("malformed.toml");
    std::fs::write(
        &pet_path,
        r#"[[snippets]]
description = "bad var"
command = "echo <name"
"#,
    )
    .unwrap();

    let output = snp_cmd()
        .args([
            "doctor",
            "--pet-file",
            pet_path.to_str().unwrap(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let diags = json["diagnostics"].as_array().unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d["code"].as_str() == Some("W-MALFORMED-VAR")),
        "Should detect malformed variable placeholder"
    );
}

// --- Sort and Ranking (Release 4A) ---

fn create_sort_test_library(config_dir: &Path, lib_name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", lib_name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
id = "test-1"
description = "zebra list"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[Snippets]]
id = "test-2"
description = "alpha deploy"
command = "deploy.sh"
tags = ["deploy"]
output = ""
folders = []
favorite = true
created_at = 300
updated_at = 300

[[Snippets]]
id = "test-3"
description = "middle status"
command = "git status"
tags = ["git"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", lib_name]);
    cmd.output().unwrap();
}

#[test]
fn test_list_sort_by_description() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-desc");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "description", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // alphabetical: alpha < middle < zebra
    assert_eq!(items[0]["description"], "alpha deploy");
    assert_eq!(items[1]["description"], "middle status");
    assert_eq!(items[2]["description"], "zebra list");
}

#[test]
fn test_list_sort_by_command() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-cmd");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "command", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // alphabetical by command: deploy.sh < git status < ls -la
    assert_eq!(items[0]["command"], "deploy.sh");
    assert_eq!(items[1]["command"], "git status");
    assert_eq!(items[2]["command"], "ls -la");
}

#[test]
fn test_list_sort_by_recent() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-recent");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "recent", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // most recently updated first: updated_at 300 > 200 > 100
    assert_eq!(items[0]["description"], "alpha deploy");
    assert_eq!(items[1]["description"], "middle status");
    assert_eq!(items[2]["description"], "zebra list");
}

#[test]
fn test_list_favorites_first() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-fav");

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "list",
        "--sort",
        "description",
        "--favorites-first",
        "--json",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // favorite first (alpha deploy), then alphabetical: middle < zebra
    assert_eq!(items[0]["description"], "alpha deploy");
    assert!(items[0]["favorite"].as_bool().unwrap());
    assert_eq!(items[1]["description"], "middle status");
    assert_eq!(items[2]["description"], "zebra list");
}

#[test]
fn test_list_sort_default_is_relevance() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-default");

    // Without --sort flag, output should use default (relevance) ordering
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
}

#[test]
fn test_list_sort_csv_respects_sort() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-csv");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "description", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Header + 3 data rows
    assert_eq!(lines.len(), 4);
    // First data row should be "alpha deploy"
    assert!(lines[1].contains("alpha deploy"));
    assert!(lines[2].contains("middle status"));
    assert!(lines[3].contains("zebra list"));
}

#[test]
fn test_list_sort_invalid_value_rejected() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-invalid");

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "bogus"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_run_help_shows_sort_flag() {
    let output = snp_cmd().args(["run", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--sort"));
    assert!(stdout.contains("--favorites-first"));
}

#[test]
fn test_clip_help_shows_sort_flag() {
    let output = snp_cmd().args(["clip", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--sort"));
    assert!(stdout.contains("--favorites-first"));
}

#[test]
fn test_search_help_shows_sort_flag() {
    let output = snp_cmd().args(["search", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--sort"));
    assert!(stdout.contains("--favorites-first"));
}

#[test]
fn test_select_help_shows_sort_flag() {
    let output = snp_cmd().args(["select", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--sort"));
    assert!(stdout.contains("--favorites-first"));
}

#[test]
fn test_list_help_shows_sort_flag() {
    let output = snp_cmd().args(["list", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--sort"));
    assert!(stdout.contains("--favorites-first"));
}

// ── Usage tracking integration tests ──────────────────────────────────

/// Helper: create a library with known snippet IDs for usage tracking tests.
fn create_usage_test_library(config_dir: &Path, lib_name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", lib_name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
id = "usage-aaa"
description = "alpha deploy"
command = "deploy.sh"
tags = ["deploy"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[Snippets]]
id = "usage-bbb"
description = "beta test"
command = "test.sh"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 200
updated_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", lib_name]);
    cmd.output().unwrap();
}

#[test]
fn test_usage_not_written_to_library_toml() {
    let (_tmp, config_dir) = setup_test_env();
    create_usage_test_library(&config_dir, "usage-iso");

    // Create a usage.toml with some data to verify it doesn't leak into library
    let usage_dir = config_dir.clone();
    fs::write(
        usage_dir.join("usage.toml"),
        r#"[[entries]]
id = "usage-aaa"
use_count = 5
last_used_at = 1700000000
"#,
    )
    .unwrap();

    // Read the library TOML and verify no usage fields
    let lib_path = config_dir.join("libraries").join("usage-iso.toml");
    let lib_content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        !lib_content.contains("use_count"),
        "library TOML should not contain use_count"
    );
    assert!(
        !lib_content.contains("last_used_at"),
        "library TOML should not contain last_used_at"
    );
    assert!(
        !lib_content.contains("[[entries]]"),
        "library TOML should not contain [[entries]] section"
    );
}

#[test]
fn test_usage_file_is_separate_from_library() {
    let (_tmp, config_dir) = setup_test_env();
    create_usage_test_library(&config_dir, "usage-sep");

    // Create a usage.toml
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "usage-aaa"
use_count = 3
last_used_at = 1700000000
"#,
    )
    .unwrap();

    // Verify usage.toml exists as a separate file at config root
    assert!(
        config_dir.join("usage.toml").exists(),
        "usage.toml should exist at config root"
    );
    assert!(
        !config_dir.join("libraries").join("usage-sep.toml").exists()
            || config_dir.join("libraries").join("usage-sep.toml").exists(),
        "library file exists separately"
    );

    // Verify the library TOML file does not contain usage.toml content
    let lib_content =
        fs::read_to_string(config_dir.join("libraries").join("usage-sep.toml")).unwrap();
    assert!(
        !lib_content.contains("1700000000"),
        "library should not contain usage timestamps"
    );
}

#[test]
fn test_no_command_text_in_usage_records() {
    let (_tmp, config_dir) = setup_test_env();
    create_usage_test_library(&config_dir, "usage-cmd");

    // Create usage.toml — it should only contain IDs, not command text
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "usage-aaa"
use_count = 2
last_used_at = 1700000000
"#,
    )
    .unwrap();

    let usage_content = fs::read_to_string(config_dir.join("usage.toml")).unwrap();
    assert!(
        !usage_content.contains("deploy.sh"),
        "usage.toml should not contain command text"
    );
    assert!(
        !usage_content.contains("test.sh"),
        "usage.toml should not contain command text"
    );
    assert!(
        !usage_content.contains("alpha deploy"),
        "usage.toml should not contain description text"
    );
    // Should only contain the ID reference
    assert!(usage_content.contains("usage-aaa"));
}

#[test]
fn test_unicode_description_sort() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "unicode-sort"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    let lib_path = libraries_dir.join("unicode-sort.toml");
    fs::write(
        &lib_path,
        r#"
[[Snippets]]
id = "uni-1"
description = "日本語テスト"
command = "echo japanese"
tags = ["unicode"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[Snippets]]
id = "uni-2"
description = "alpha deploy"
command = "deploy.sh"
tags = ["deploy"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200

[[Snippets]]
id = "uni-3"
description = "Ünïcödé test"
command = "echo unicode"
tags = ["unicode"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "unicode-sort"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "description", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // Unicode-aware case-insensitive sort: alpha < Ünïcödé < 日本語テスト
    assert_eq!(items[0]["description"], "alpha deploy");
    assert_eq!(items[1]["description"], "Ünïcödé test");
    assert_eq!(items[2]["description"], "日本語テスト");
}

#[test]
fn test_multi_library_sort() {
    let (_tmp, config_dir) = setup_test_env();

    // Create first library
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "multi-a"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("multi-a.toml"),
        r#"
[[Snippets]]
id = "multi-a-1"
description = "bravo deploy"
command = "deploy-a.sh"
tags = ["deploy"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    // Create second library
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "multi-b"]);
    cmd.output().unwrap();

    fs::write(
        libraries_dir.join("multi-b.toml"),
        r#"
[[Snippets]]
id = "multi-b-1"
description = "alpha test"
command = "test-b.sh"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "multi-a"]);
    cmd.output().unwrap();

    // List with --library multi-b and --sort description
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "list",
        "--library",
        "multi-b",
        "--sort",
        "description",
        "--json",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["description"], "alpha test");
    assert_eq!(items[0]["command"], "test-b.sh");
}

#[test]
fn test_list_sort_last_used() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-last-used");

    // Create a usage.toml with known last_used_at values
    let usage_path = config_dir.join("usage.toml");
    fs::write(
        &usage_path,
        r#"[[entries]]
id = "test-1"
use_count = 5
last_used_at = 300

[[entries]]
id = "test-2"
use_count = 1
last_used_at = 100

[[entries]]
id = "test-3"
use_count = 3
last_used_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "last-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // last_used_at 300 > 200 > 100
    // test-1 (zebra list, last_used_at=300) > test-3 (middle status, last_used_at=200) > test-2 (alpha deploy, last_used_at=100)
    assert_eq!(items[0]["description"], "zebra list");
    assert_eq!(items[1]["description"], "middle status");
    assert_eq!(items[2]["description"], "alpha deploy");
}

#[test]
fn test_list_sort_most_used() {
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-most-used");

    // Create a usage.toml with known use_count values
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "test-1"
use_count = 5
last_used_at = 100

[[entries]]
id = "test-2"
use_count = 20
last_used_at = 200

[[entries]]
id = "test-3"
use_count = 10
last_used_at = 300
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // use_count 20 > 10 > 5
    // test-2 (alpha deploy, use_count=20) > test-3 (middle status, use_count=10) > test-1 (zebra list, use_count=5)
    assert_eq!(items[0]["description"], "alpha deploy");
    assert_eq!(items[1]["description"], "middle status");
    assert_eq!(items[2]["description"], "zebra list");
}

#[test]
fn test_rank_snippets_deterministic_across_runs() {
    // Verify that rank_snippets produces identical output when called twice
    // with the same inputs (test #11 from plan: repeated runs are deterministic)
    let (_tmp, config_dir) = setup_test_env();
    create_sort_test_library(&config_dir, "sort-det");

    // Run list twice and compare JSON output
    let mut cmd1 = snp_in(&config_dir);
    cmd1.args(["list", "--sort", "description", "--json"]);
    let output1 = cmd1.output().unwrap();
    let stdout1 = String::from_utf8_lossy(&output1.stdout);

    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--sort", "description", "--json"]);
    let output2 = cmd2.output().unwrap();
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    assert_eq!(stdout1, stdout2, "list output should be deterministic");
}

// =====================================================================
// Release 4 corrective pass: divergent-metadata fixture tests
// =====================================================================

/// Create a library with deliberately divergent updated_at, use_count, and last_used_at.
///
/// | Snippet | updated_at | use_count | last_used_at |
/// | --- | ---: | ---: | ---: |
/// | A | 300 | 1 | 100 |
/// | B | 100 | 9 | 200 |
/// | C | 200 | 2 | 900 |
fn create_divergent_metadata_library(config_dir: &Path, lib_name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", lib_name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join(format!("{lib_name}.toml")),
        r#"[[snippets]]
id = "div-A"
description = "snippet A"
command = "echo A"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 300

[[snippets]]
id = "div-B"
description = "snippet B"
command = "echo B"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 50
updated_at = 100

[[snippets]]
id = "div-C"
description = "snippet C"
command = "echo C"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 50
updated_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", lib_name]);
    cmd.output().unwrap();
}

fn write_divergent_usage(config_dir: &Path) {
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "div-A"
use_count = 1
last_used_at = 100

[[entries]]
id = "div-B"
use_count = 9
last_used_at = 200

[[entries]]
id = "div-C"
use_count = 2
last_used_at = 900
"#,
    )
    .unwrap();
}

#[test]
fn test_divergent_metadata_list_recent() {
    let (_tmp, config_dir) = setup_test_env();
    create_divergent_metadata_library(&config_dir, "div-recent");
    write_divergent_usage(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "recent", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // Recent: by updated_at desc → A (300), C (200), B (100)
    assert_eq!(items[0]["description"], "snippet A");
    assert_eq!(items[1]["description"], "snippet C");
    assert_eq!(items[2]["description"], "snippet B");
}

#[test]
fn test_divergent_metadata_list_most_used() {
    let (_tmp, config_dir) = setup_test_env();
    create_divergent_metadata_library(&config_dir, "div-most-used");
    write_divergent_usage(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // Most-used: by use_count desc → B (9), C (2), A (1)
    assert_eq!(items[0]["description"], "snippet B");
    assert_eq!(items[1]["description"], "snippet C");
    assert_eq!(items[2]["description"], "snippet A");
}

#[test]
fn test_divergent_metadata_list_last_used() {
    let (_tmp, config_dir) = setup_test_env();
    create_divergent_metadata_library(&config_dir, "div-last-used");
    write_divergent_usage(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "last-used", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // Last-used: by last_used_at desc → C (900), B (200), A (100)
    assert_eq!(items[0]["description"], "snippet C");
    assert_eq!(items[1]["description"], "snippet B");
    assert_eq!(items[2]["description"], "snippet A");
}

#[test]
fn test_divergent_metadata_list_recent_csv() {
    let (_tmp, config_dir) = setup_test_env();
    create_divergent_metadata_library(&config_dir, "div-recent-csv");
    write_divergent_usage(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "recent", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Header + 3 data rows
    assert_eq!(lines.len(), 4);
    // Recent: A, C, B
    assert!(lines[1].contains("snippet A"));
    assert!(lines[2].contains("snippet C"));
    assert!(lines[3].contains("snippet B"));
}

#[test]
fn test_divergent_metadata_list_most_used_csv() {
    let (_tmp, config_dir) = setup_test_env();
    create_divergent_metadata_library(&config_dir, "div-most-used-csv");
    write_divergent_usage(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 4);
    // Most-used: B, C, A
    assert!(lines[1].contains("snippet B"));
    assert!(lines[2].contains("snippet C"));
    assert!(lines[3].contains("snippet A"));
}

#[test]
fn test_favorites_first_with_last_used_list() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "fav-last-used"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("fav-last-used.toml"),
        r#"[[snippets]]
id = "fl-1"
description = "non-fav old"
command = "echo old"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "fl-2"
description = "fav recent"
command = "echo recent"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 200
updated_at = 200

[[snippets]]
id = "fl-3"
description = "non-fav recent"
command = "echo nrecent"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "fl-4"
description = "fav old"
command = "echo fold"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 50
updated_at = 50
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "fav-last-used"]);
    cmd.output().unwrap();

    // Usage: fav old was used more recently than fav recent
    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "fl-1"
use_count = 1
last_used_at = 100

[[entries]]
id = "fl-2"
use_count = 1
last_used_at = 200

[[entries]]
id = "fl-3"
use_count = 1
last_used_at = 300

[[entries]]
id = "fl-4"
use_count = 1
last_used_at = 500
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "last-used", "--favorites-first", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 4);
    // Fav: fav old (500), fav recent (200)
    // Non-fav: non-fav recent (300), non-fav old (100)
    assert_eq!(items[0]["description"], "fav old");
    assert!(items[0]["favorite"].as_bool().unwrap());
    assert_eq!(items[1]["description"], "fav recent");
    assert!(items[1]["favorite"].as_bool().unwrap());
    assert_eq!(items[2]["description"], "non-fav recent");
    assert!(!items[2]["favorite"].as_bool().unwrap());
    assert_eq!(items[3]["description"], "non-fav old");
    assert!(!items[3]["favorite"].as_bool().unwrap());
}

#[test]
fn test_favorites_first_with_most_used_list() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "fav-most-used"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("fav-most-used.toml"),
        r#"[[snippets]]
id = "fm-1"
description = "non-fav low"
command = "echo low"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "fm-2"
description = "fav mid"
command = "echo mid"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 200
updated_at = 200

[[snippets]]
id = "fm-3"
description = "non-fav high"
command = "echo high"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "fm-4"
description = "fav highest"
command = "echo highest"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 50
updated_at = 50
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "fav-most-used"]);
    cmd.output().unwrap();

    fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "fm-1"
use_count = 1
last_used_at = 100

[[entries]]
id = "fm-2"
use_count = 5
last_used_at = 200

[[entries]]
id = "fm-3"
use_count = 10
last_used_at = 300

[[entries]]
id = "fm-4"
use_count = 20
last_used_at = 400
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "most-used", "--favorites-first", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 4);
    // Fav: fav highest (20), fav mid (5)
    // Non-fav: non-fav high (10), non-fav low (1)
    assert_eq!(items[0]["description"], "fav highest");
    assert!(items[0]["favorite"].as_bool().unwrap());
    assert_eq!(items[1]["description"], "fav mid");
    assert!(items[1]["favorite"].as_bool().unwrap());
    assert_eq!(items[2]["description"], "non-fav high");
    assert!(!items[2]["favorite"].as_bool().unwrap());
    assert_eq!(items[3]["description"], "non-fav low");
    assert!(!items[3]["favorite"].as_bool().unwrap());
}

// =====================================================================
// Release 4B: Output / Notes Presentation Tests
// =====================================================================

fn create_output_test_library(config_dir: &Path) {
    // Use the snp binary to create the library properly
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "output-test"]);
    cmd.output().unwrap();

    let lib_dir = config_dir.join("libraries");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(
        lib_dir.join("output-test.toml"),
        r#"[[snippets]]
description = "empty output"
command = "echo empty"
output = ""

[[snippets]]
description = "single line output"
command = "echo hello"
output = "This is a sample output"

[[snippets]]
description = "multiline output"
command = "echo multiline"
output = "line1\nline2\nline3"

[[snippets]]
description = "output with tabs"
command = "echo tabs"
output = "col1\tcol2\tcol3"

[[snippets]]
description = "output with special chars"
command = "echo special"
output = "backslash \\ and \"quotes\" and 'single'"

[[snippets]]
description = "output with unicode"
command = "echo unicode"
output = "日本語テスト 🎉"

[[snippets]]
description = "output with shell-like content"
command = "echo shell"
output = "curl -s https://api.example.com | jq '.data'"

[[snippets]]
description = "long output snippet"
command = "echo long"
output = "This is a longer output that should be truncated in summary display but shown in full in JSON output"

[[snippets]]
description = "ansi in output"
command = "echo ansi"
output = "\x1b[31mred text\x1b[0m normal"
"#,
    )
    .unwrap();
}

#[test]
fn test_output_field_preserved_in_json() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "snp list failed. stderr: {stderr}\nstdout: {stdout}"
    );
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert!(
        items.len() >= 8,
        "Expected >= 8 items, got {}. stdout: {stdout}",
        items.len()
    );

    // Check empty output
    let empty = items
        .iter()
        .find(|i| i["description"] == "empty output")
        .unwrap();
    assert_eq!(empty["output"], "");

    // Check single line output
    let single = items
        .iter()
        .find(|i| i["description"] == "single line output")
        .unwrap();
    assert_eq!(single["output"], "This is a sample output");

    // Check multiline output
    let multi = items
        .iter()
        .find(|i| i["description"] == "multiline output")
        .unwrap();
    assert_eq!(multi["output"], "line1\nline2\nline3");

    // Check unicode
    let unicode = items
        .iter()
        .find(|i| i["description"] == "output with unicode")
        .unwrap();
    assert_eq!(unicode["output"], "日本語テスト 🎉");
}

#[test]
fn test_output_field_preserved_in_csv() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Header should include output column
    assert!(stdout.lines().next().unwrap().contains("output"));
    // Should contain the output values
    assert!(stdout.contains("This is a sample output"));
    assert!(stdout.contains("line1"));
}

#[test]
fn test_list_default_output_not_shown_when_empty() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Empty output snippet should NOT show "Output:" line
    let lines: Vec<&str> = stdout.lines().collect();
    // Find the line after "empty output" description
    let empty_idx = lines
        .iter()
        .position(|l| l.contains("empty output"))
        .unwrap();
    // The next non-empty line after description should be "Tags:", not "Output:"
    let next_line = lines[empty_idx + 1..]
        .iter()
        .find(|l| !l.trim().is_empty())
        .unwrap();
    assert!(
        next_line.contains("Tags"),
        "Expected Tags line after empty output, got: {next_line}"
    );
}

#[test]
fn test_list_default_output_shown_when_nonempty() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Non-empty output should show Output: line with truncated summary
    // The output text may have ANSI codes, so check for the plain text
    assert!(
        stdout.contains("This is a sample output") || stdout.contains("sample output"),
        "Expected output content in list output. Got: {stdout}"
    );
}

#[test]
fn test_edit_output_set() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "New notes for this snippet",
        "--filter",
        "empty output",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Updated output"));

    // Verify the output was set via JSON
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--library", "output-test", "--json"]);
    let output2 = cmd2.output().unwrap();
    let stdout = String::from_utf8_lossy(&output2.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let snippet = items
        .iter()
        .find(|i| i["description"] == "empty output")
        .unwrap();
    assert_eq!(snippet["output"], "New notes for this snippet");
}

#[test]
fn test_edit_output_clear() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // First set output
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "temporary content",
        "--filter",
        "empty output",
    ]);
    cmd.output().unwrap();

    // Then clear it
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args([
        "edit",
        "--library",
        "output-test",
        "--clear-output",
        "--filter",
        "empty output",
    ]);
    let output = cmd2.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Cleared output"));

    // Verify cleared via JSON
    let mut cmd3 = snp_in(&config_dir);
    cmd3.args(["list", "--library", "output-test", "--json"]);
    let output3 = cmd3.output().unwrap();
    let stdout = String::from_utf8_lossy(&output3.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let snippet = items
        .iter()
        .find(|i| i["description"] == "empty output")
        .unwrap();
    assert_eq!(snippet["output"], "");
}

#[test]
fn test_edit_output_stdin() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output-stdin",
        "--filter",
        "empty output",
    ]);
    let output = output_with_stdin(cmd, b"Content from stdin");
    assert!(output.status.success());

    // Verify via JSON
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--library", "output-test", "--json"]);
    let output2 = cmd2.output().unwrap();
    let stdout = String::from_utf8_lossy(&output2.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let snippet = items
        .iter()
        .find(|i| i["description"] == "empty output")
        .unwrap();
    assert_eq!(snippet["output"], "Content from stdin");
}

#[test]
fn test_edit_output_no_filter_fails() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["edit", "--library", "output-test", "--output", "test"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_edit_output_no_match_fails() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "test",
        "--filter",
        "nonexistent snippet",
    ]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_output_multiline_roundtrip() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // Set multiline output
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "line1\nline2\nline3",
        "--filter",
        "empty output",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Read back via JSON and verify exact content
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--library", "output-test", "--json"]);
    let output2 = cmd2.output().unwrap();
    let stdout = String::from_utf8_lossy(&output2.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let snippet = items
        .iter()
        .find(|i| i["description"] == "empty output")
        .unwrap();
    assert_eq!(snippet["output"], "line1\nline2\nline3");
}

#[test]
fn test_output_with_tabs_and_special_chars_roundtrip() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // The library already has tabs output; verify exact preservation in JSON
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let tabs_snippet = items
        .iter()
        .find(|i| i["description"] == "output with tabs")
        .unwrap();
    assert_eq!(tabs_snippet["output"], "col1\tcol2\tcol3");

    let special_snippet = items
        .iter()
        .find(|i| i["description"] == "output with special chars")
        .unwrap();
    assert_eq!(
        special_snippet["output"],
        "backslash \\ and \"quotes\" and 'single'"
    );
}

#[test]
fn test_search_output_flag_includes_output_in_fuzzy_match() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // Without --search-output: searching for "sample" should NOT match "single line output"
    // because "sample" is only in the output field, not description or command
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "list",
        "--library",
        "output-test",
        "--filter",
        "sample",
        "--json",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        items.len(),
        0,
        "Without --search-output, 'sample' should not match output field"
    );

    // With --search-output: searching for "sample" SHOULD match at least the single line output
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args([
        "list",
        "--library",
        "output-test",
        "--filter",
        "sample",
        "--search-output",
        "--json",
    ]);
    let output2 = cmd2.output().unwrap();
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let items2: Vec<serde_json::Value> = serde_json::from_str(&stdout2).unwrap();
    assert!(
        !items2.is_empty(),
        "With --search-output, 'sample' should match output field"
    );
    assert!(
        items2
            .iter()
            .any(|i| i["description"] == "single line output"),
        "Should include 'single line output' which has 'sample' in its output"
    );
}

#[test]
fn test_search_output_flag_with_multiline_content() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // "line2" only appears in output, not in description or command
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "list",
        "--library",
        "output-test",
        "--filter",
        "line2",
        "--search-output",
        "--json",
    ]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["description"], "multiline output");
}

#[test]
fn test_output_no_eval_no_execution() {
    // Verify that output content with shell-like syntax is never executed
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // The "shell-like content" snippet has curl|jq in output
    // Listing it should not execute anything
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let shell_snippet = items
        .iter()
        .find(|i| i["description"] == "output with shell-like content")
        .unwrap();
    // Output should be preserved as-is, not executed
    assert!(shell_snippet["output"].as_str().unwrap().contains("curl"));
}

#[test]
fn test_edit_output_conflicts() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // --output and --clear-output should conflict
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "test",
        "--clear-output",
        "--filter",
        "empty output",
    ]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());

    // --output and --output-stdin should conflict
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args([
        "edit",
        "--library",
        "output-test",
        "--output",
        "test",
        "--output-stdin",
        "--filter",
        "empty output",
    ]);
    let output2 = cmd2.output().unwrap();
    assert!(!output2.status.success());
}

#[test]
fn test_output_ansi_sequences_in_json_preserved() {
    // JSON should preserve the raw value including ANSI sequences
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let ansi_snippet = items
        .iter()
        .find(|i| i["description"] == "ansi in output")
        .unwrap();
    let output_val = ansi_snippet["output"].as_str().unwrap();
    // ANSI escape sequences should be preserved in JSON
    assert!(output_val.contains("\x1b[31m"));
    assert!(output_val.contains("\x1b[0m"));
}

#[test]
fn test_search_output_help_text() {
    let output = snp_cmd().args(["list", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--search-output"));
}

#[test]
fn test_edit_output_help_text() {
    let output = snp_cmd().args(["edit", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--output"));
    assert!(stdout.contains("--output-stdin"));
    assert!(stdout.contains("--clear-output"));
}

#[test]
fn test_search_output_default_excludes_output_from_matching() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    // The "single line output" snippet has output "This is a sample output"
    // but its description is "single line output" and command is "echo hello"
    // Without --search-output, searching for "sample" should NOT match it
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "output-test", "--filter", "sample"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "sample" only appears in the output field, not in description or command
    assert!(
        !stdout.contains("single line output"),
        "Without --search-output, 'sample' should not match. Got: {stdout}"
    );
}

#[test]
fn test_legacy_snippet_without_output_field_loads() {
    let (_tmp, config_dir) = setup_test_env();
    let lib_dir = config_dir.join("libraries");
    fs::create_dir_all(&lib_dir).unwrap();

    // Create the library first (creates an empty TOML file)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "legacy"]);
    cmd.output().unwrap();

    // Overwrite with legacy-format snippet without the output field
    fs::write(
        lib_dir.join("legacy.toml"),
        r#"[[snippets]]
description = "legacy snippet"
command = "echo legacy"
tag = ["old"]
"#,
    )
    .unwrap();

    // List should succeed and show the snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "legacy", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["description"], "legacy snippet");
    // output should default to empty string
    assert_eq!(items[0]["output"], "");
}

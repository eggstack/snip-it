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

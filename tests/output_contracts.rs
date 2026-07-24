use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn snp_cmd() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snp"));
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd
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

fn create_output_test_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "out-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "out-test"]);
    cmd.output().unwrap();

    let lib_dir = config_dir.join("libraries");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(
        lib_dir.join("out-test.toml"),
        r#"[[snippets]]
id = "out-1"
description = "simple command"
command = "echo hello"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "out-2"
description = "multiline command"
command = "if true; then\n  echo yes\nelse\n  echo no\nfi\n"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200

[[snippets]]
id = "out-3"
description = "command with variables"
command = "ssh <user>@<host> -p <port=22>"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "out-4"
description = "no trailing newline"
command = "echo nonewline"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 400
updated_at = 400

[[snippets]]
id = "out-5"
description = "unicode content"
command = "echo '日本語 café'"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 500
updated_at = 500

[[snippets]]
id = "out-6"
description = "special chars"
command = "echo \"hello\\tworld\""
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 600
updated_at = 600
"#,
    )
    .unwrap();
}

fn create_exact_test_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "exact-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "exact-test"]);
    cmd.output().unwrap();

    let lib_dir = config_dir.join("libraries");
    fs::create_dir_all(&lib_dir).unwrap();
    fs::write(
        lib_dir.join("exact-test.toml"),
        r#"[[snippets]]
id = "exc-1"
description = "test run exact"
command = "echo ran-exact"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "exc-2"
description = "test clip exact"
command = "echo clipped-exact"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200

[[snippets]]
id = "exc-3"
description = "ambiguous desc"
command = "echo first"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "exc-4"
description = "ambiguous desc"
command = "echo second"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 400
updated_at = 400
"#,
    )
    .unwrap();
}

// --- Output byte-level tests ---

#[test]
fn test_raw_output_exact_bytes() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-1", "--raw"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"echo hello");
}

#[test]
fn test_field_command_exact_bytes() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-1", "--field", "command"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"echo hello");
}

#[test]
fn test_multiline_raw_preserved() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-2", "--raw"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"if true; then\n  echo yes\nelse\n  echo no\nfi\n"
    );
}

#[test]
fn test_json_no_ansi() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-5", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !output.stdout.contains(&0x1b),
        "JSON output must not contain ANSI escape sequences"
    );
}

#[test]
fn test_get_default_strips_escapes() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-5"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !output.stdout.contains(&0x1b),
        "Default output must not contain ANSI escape sequences"
    );
}

#[test]
fn test_json_has_schema_version() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-1", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["schema"], 1);
}

#[test]
fn test_json_has_library_fields() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "out-1", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["library"], "out-test");
    assert!(
        json["library_id"].as_str().is_some(),
        "library_id must be a string"
    );
}

// --- Exact operation tests ---

#[test]
fn test_run_by_id_executes() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exc-1"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_run_ambiguous_returns_exit_5() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--description-exact", "ambiguous desc"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn test_run_not_found_returns_exit_3() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "nonexistent"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
}

#[test]
#[ignore] // Requires a display server (no clipboard on headless CI)
fn test_clip_by_id_succeeds() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["clip", "--id", "exc-2"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_clip_ambiguous_returns_exit_5() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["clip", "--description-exact", "ambiguous desc"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn test_edit_output_by_id() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args([
            "edit",
            "--id",
            "exc-1",
            "--output",
            "test-value",
            "--filter",
            "test run exact",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_edit_ambiguous_returns_exit_5() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args([
            "edit",
            "--description-exact",
            "ambiguous desc",
            "--output",
            "val",
            "--filter",
            "ambiguous desc",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn test_exact_selectors_bypass_tui() {
    let (_tmp, config_dir) = setup_test_env();
    create_exact_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exc-1"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

// --- Variable assignment tests ---

#[test]
fn test_get_expanded_with_var() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args([
            "get",
            "--id",
            "out-3",
            "--expanded",
            "--var",
            "user=admin",
            "--var",
            "host=example.com",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("admin"));
    assert!(stdout.contains("example.com"));
}

#[test]
fn test_get_expanded_default_port() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args([
            "get",
            "--id",
            "out-3",
            "--expanded",
            "--var",
            "user=root",
            "--var",
            "host=server.com",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("22"));
}

#[test]
fn test_get_expanded_override_port() {
    let (_tmp, config_dir) = setup_test_env();
    create_output_test_library(&config_dir);

    let output = snp_in(&config_dir)
        .args([
            "get",
            "--id",
            "out-3",
            "--expanded",
            "--var",
            "user=root",
            "--var",
            "host=server.com",
            "--var",
            "port=8080",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("8080"));
}

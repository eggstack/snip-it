mod support;

use std::fs;
use support::helpers::{golden_corpus, output_with_stdin, snp_cmd, snp_in};

fn setup_test_env() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();
    (tmp, config_dir)
}

fn cmd_with_xdg(config_dir: &std::path::Path) -> std::process::Command {
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd
}

#[test]
fn test_help_exits_zero() {
    let output = snp_cmd().arg("--help").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_version_exits_zero() {
    let output = snp_cmd().arg("--version").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_new_snippet_success() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = cmd_with_xdg(&config_dir);
    cmd.args(["new", "--command-stdin", "--description", "stdin test"]);
    let output = output_with_stdin(cmd, b"echo hello\n");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Snippet added"));
}

#[test]
fn test_new_snippet_with_description() {
    let (_tmp, config_dir) = setup_test_env();
    let output = cmd_with_xdg(&config_dir)
        .args(["new", "--description", "My test snippet", "echo test"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cmd_with_xdg(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 1);
    assert_eq!(snippets[0]["description"], "My test snippet");
    assert_eq!(snippets[0]["command"], "echo test");
}

#[test]
fn test_list_empty_library() {
    let (_tmp, config_dir) = setup_test_env();
    cmd_with_xdg(&config_dir)
        .args(["library", "create", "empty-list"])
        .output()
        .unwrap();
    cmd_with_xdg(&config_dir)
        .args(["library", "set-primary", "empty-list"])
        .output()
        .unwrap();

    let output = cmd_with_xdg(&config_dir).args(["list"]).output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_list_with_snippets() {
    let (_tmp, config_dir) = setup_test_env();
    cmd_with_xdg(&config_dir)
        .args(["library", "create", "list-many"])
        .output()
        .unwrap();
    cmd_with_xdg(&config_dir)
        .args(["library", "set-primary", "list-many"])
        .output()
        .unwrap();

    for i in 1..=3 {
        let output = cmd_with_xdg(&config_dir)
            .args([
                "new",
                "--description",
                &format!("snippet-{i}"),
                &format!("echo {i}"),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    let output = cmd_with_xdg(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), 3);
}

#[test]
fn test_library_create_and_delete() {
    let (_tmp, config_dir) = setup_test_env();
    let output = cmd_with_xdg(&config_dir)
        .args(["library", "create", "to-delete"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cmd_with_xdg(&config_dir)
        .args(["library", "delete", "to-delete", "--force"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cmd_with_xdg(&config_dir)
        .args(["library", "list"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("to-delete"));
}

#[test]
fn test_library_set_primary() {
    let (_tmp, config_dir) = setup_test_env();
    cmd_with_xdg(&config_dir)
        .args(["library", "create", "lib-alpha"])
        .output()
        .unwrap();
    cmd_with_xdg(&config_dir)
        .args(["library", "create", "lib-beta"])
        .output()
        .unwrap();

    let output = cmd_with_xdg(&config_dir)
        .args(["library", "set-primary", "lib-beta"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let output = cmd_with_xdg(&config_dir)
        .args(["library", "show"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("lib-beta") && stdout.contains("primary"),
        "Expected lib-beta to be primary: {stdout}"
    );
}

#[test]
fn test_empty_stdin_rejected() {
    let (_tmp, config_dir) = setup_test_env();
    let mut cmd = cmd_with_xdg(&config_dir);
    cmd.args(["new", "--command-stdin", "--description", "empty test"]);
    let output = output_with_stdin(cmd, b"");
    assert!(!output.status.success());
}

#[test]
fn test_help_subcommands_exist() {
    let subcommands = [
        "list",
        "new",
        "edit",
        "sync",
        "status",
        "library",
        "cron",
        "keybindings",
        "completions",
        "doctor",
    ];
    for subcmd in &subcommands {
        let output = snp_cmd()
            .args([subcmd, "--help"])
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn snp {subcmd} --help: {e}"));
        assert!(
            output.status.success(),
            "snp {subcmd} --help exited non-zero: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn test_sync_config_show() {
    let (_tmp, config_dir) = setup_test_env();
    let output = cmd_with_xdg(&config_dir)
        .args(["sync", "config", "--show"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn test_status_without_config() {
    let (_tmp, config_dir) = setup_test_env();
    let output = cmd_with_xdg(&config_dir).args(["status"]).output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_golden_corpus_preserves_exact_text() {
    let (_tmp, config_dir) = setup_test_env();

    for (label, command_str) in golden_corpus() {
        let mut cmd = cmd_with_xdg(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("golden-{label}"),
        ]);
        let output = output_with_stdin(cmd, command_str.as_bytes());
        assert!(
            output.status.success(),
            "golden corpus '{label}' failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = cmd_with_xdg(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snippets.len(), golden_corpus().len());

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

    // Trigger a full save/load cycle by adding then deleting a dummy snippet
    cmd_with_xdg(&config_dir)
        .args(["new", "--description", "dummy", "echo dummy"])
        .output()
        .unwrap();

    let output = cmd_with_xdg(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    let snippets_after: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();

    for (label, command_str) in golden_corpus() {
        let desc = format!("golden-{label}");
        let snippet = snippets_after
            .iter()
            .find(|s| s["description"].as_str() == Some(&desc))
            .unwrap_or_else(|| panic!("snippet for golden-{label} not found after save"));
        assert_eq!(
            snippet["command"].as_str().unwrap(),
            command_str,
            "golden corpus '{label}' failed after save/load cycle"
        );
    }
}

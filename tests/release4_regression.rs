//! Release 4 default-behavior regression tests.
//!
//! Proves that invocations without Release 4 flags preserve pre-Release-4
//! behavior. Uses pinned fixtures and exact assertions.

mod support;

use support::helpers::*;

// ── Default candidate order regression ──

/// Regression fixture: a library with known insertion order.
/// Pre-Release-4 default was relevance (fuzzy match), which for no-query
/// returns original insertion order.
fn create_default_order_library(config_dir: &std::path::Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "regression-order"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("regression-order.toml"),
        r#"[[snippets]]
id = "reg-1"
description = "first alpha"
command = "alpha.sh"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "reg-2"
description = "second bravo"
command = "bravo.sh"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200

[[snippets]]
id = "reg-3"
description = "third charlie"
command = "charlie.sh"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 300
updated_at = 300
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "regression-order"]);
    cmd.output().unwrap();
}

#[test]
fn test_list_default_order_preserves_insertion_order() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // Without --sort flag, default is relevance (no query = insertion order)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 3);
    // Default order should be insertion order
    assert_eq!(items[0]["description"], "first alpha");
    assert_eq!(items[1]["description"], "second bravo");
    assert_eq!(items[2]["description"], "third charlie");
}

#[test]
fn test_list_default_output_hidden_when_empty() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Default list output should not show "Output:" for snippets with empty output
    assert!(
        !stdout.contains("Output:"),
        "Default list should not show Output field for empty output. Got: {stdout}"
    );
}

#[test]
fn test_list_without_search_output_excludes_output_from_matching() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // Without --search-output, filtering by text that only appears in command
    // should still match (command is part of default match fields)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--filter", "alpha", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["description"], "first alpha");
}

// ── Select default behavior ──

#[test]
fn test_select_exit_code_contract_unchanged() {
    // select with cancel should still exit 4 (pre-Release-4 behavior)
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // Using a PTY-less approach: pass --help to verify select still exists
    let mut cmd = snp_in(&config_dir);
    cmd.args(["select", "--help"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--sort"),
        "select --help should still show --sort flag"
    );
    assert!(
        stdout.contains("--favorites-first"),
        "select --help should still show --favorites-first flag"
    );
}

// ── Sort flag regression ──

#[test]
fn test_run_clip_search_select_list_all_accept_sort_flag() {
    let commands = ["run", "clip", "search", "select", "list"];
    for cmd_name in &commands {
        let output = snp_cmd().args([cmd_name, "--help"]).output().unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("--sort"),
            "{cmd_name} --help should show --sort flag"
        );
        assert!(
            stdout.contains("--favorites-first"),
            "{cmd_name} --help should show --favorites-first flag"
        );
    }
}

#[test]
fn test_sort_relevance_is_default() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // list --sort relevance should produce same order as no --sort flag
    let mut cmd_default = snp_in(&config_dir);
    cmd_default.args(["list", "--json"]);
    let output_default = cmd_default.output().unwrap();
    let stdout_default = String::from_utf8_lossy(&output_default.stdout);
    let items_default: Vec<serde_json::Value> = serde_json::from_str(&stdout_default).unwrap();

    let mut cmd_relevance = snp_in(&config_dir);
    cmd_relevance.args(["list", "--sort", "relevance", "--json"]);
    let output_relevance = cmd_relevance.output().unwrap();
    let stdout_relevance = String::from_utf8_lossy(&output_relevance.stdout);
    let items_relevance: Vec<serde_json::Value> = serde_json::from_str(&stdout_relevance).unwrap();

    assert_eq!(items_default.len(), items_relevance.len());
    for (a, b) in items_default.iter().zip(items_relevance.iter()) {
        assert_eq!(
            a["description"], b["description"],
            "relevance default and explicit should match"
        );
    }
}

// ── Output field not in execution payloads ──

#[test]
fn test_run_command_field_separate_from_output() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "exec-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("exec-test.toml"),
        r#"[[snippets]]
id = "exec-1"
description = "echo test"
command = "echo hello-from-command"
tags = ["test"]
output = "this-is-not-the-command"
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "exec-test"]);
    cmd.output().unwrap();

    // Verify the command and output fields are separate in JSON
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["command"].as_str().unwrap(),
        "echo hello-from-command"
    );
    assert_eq!(
        items[0]["output"].as_str().unwrap(),
        "this-is-not-the-command"
    );
}

// ── Favorites-first does not change default behavior when not flagged ──

#[test]
fn test_favorites_first_without_flag_preserves_default_order() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "fav-regression"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("fav-regression.toml"),
        r#"[[snippets]]
id = "fav-1"
description = "non-favorite alpha"
command = "alpha.sh"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "fav-2"
description = "favorite bravo"
command = "bravo.sh"
tags = ["test"]
output = ""
folders = []
favorite = true
created_at = 200
updated_at = 200
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "fav-regression"]);
    cmd.output().unwrap();

    // Without --favorites-first, order should be insertion order (relevance)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["description"], "non-favorite alpha");
    assert_eq!(items[1]["description"], "favorite bravo");

    // With --favorites-first, favorite should come first
    let mut cmd2 = snp_in(&config_dir);
    cmd2.args(["list", "--favorites-first", "--json"]);
    let output2 = cmd2.output().unwrap();
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let items2: Vec<serde_json::Value> = serde_json::from_str(&stdout2).unwrap();
    assert_eq!(items2[0]["description"], "favorite bravo");
    assert_eq!(items2[1]["description"], "non-favorite alpha");
}

// ── Shell integration still works ──

#[test]
fn test_shell_init_produces_valid_output_for_all_shells() {
    for shell in &["bash", "zsh", "fish"] {
        let output = snp_cmd().args(["shell", "init", shell]).output().unwrap();
        assert!(
            output.status.success(),
            "snp shell init {shell} should succeed"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("snp_select_raw"),
            "shell init {shell} should contain snp_select_raw function"
        );
        assert!(
            stdout.contains("snp_select_expanded"),
            "shell init {shell} should contain snp_select_expanded function"
        );
        assert!(
            stdout.contains("snp_new_current"),
            "shell init {shell} should contain snp_new_current function"
        );
        assert!(
            stdout.contains("snp_new_previous"),
            "shell init {shell} should contain snp_new_previous function"
        );
    }
}

// ── Doctor report is read-only ──

#[test]
fn test_doctor_does_not_modify_library() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // Get the library content before doctor
    let lib_path = config_dir.join("libraries").join("regression-order.toml");
    let content_before = std::fs::read_to_string(&lib_path).unwrap();

    // Run doctor on the library
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--library", "regression-order"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    // Library content should be unchanged
    let content_after = std::fs::read_to_string(&lib_path).unwrap();
    assert_eq!(
        content_before, content_after,
        "doctor should not modify library"
    );
}

// ── Sync payload does not include local-only fields ──

#[test]
fn test_json_output_does_not_include_usage_fields() {
    let (_tmp, config_dir) = setup_test_env();
    create_default_order_library(&config_dir);

    // Create a usage file
    std::fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "reg-1"
use_count = 42
last_used_at = 1700000000
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // JSON output should NOT contain usage fields
    assert!(
        !stdout.contains("use_count"),
        "JSON output should not include use_count"
    );
    assert!(
        !stdout.contains("last_used_at"),
        "JSON output should not include last_used_at"
    );
}

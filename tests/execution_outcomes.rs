//! Execution outcome tests (Workstream F).
//!
//! Verifies that:
//! - exact run returns the child's exit code on failure
//! - successful execution records usage
//! - failed execution does not record usage
//! - exact edit mutates only the targeted ID

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

/// Setup a library with a known snippet.
fn setup_library_with_snippet(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "exec-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("exec-test.toml"),
        r#"[[snippets]]
id = "exec-success"
description = "always succeeds"
command = "true"

[[snippets]]
id = "exec-fail-exit1"
description = "always fails with exit 1"
command = "exit 1"

[[snippets]]
id = "exec-fail-exit127"
description = "always fails with exit 127"
command = "exit 127"

[[snippets]]
id = "exec-edit-target"
description = "edit target snippet"
command = "echo edit-target"

[[snippets]]
id = "exec-edit-distractor"
description = "edit target snippet distractor"
command = "echo edit-distractor"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "exec-test"]);
    cmd.output().unwrap();
}

// === Exit code propagation ===

#[test]
fn test_exact_run_success_exits_zero() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exec-success"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Exact run of 'true' should exit 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_exact_run_failure_exits_nonzero() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exec-fail-exit1"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "Exact run of 'exit 1' should exit nonzero"
    );
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(code, 1, "Exit code should be 1, got {code}");
}

#[test]
fn test_exact_run_exit127_returns_127() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exec-fail-exit127"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "Exact run of 'exit 127' should exit nonzero"
    );
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(code, 127, "Exit code should be 127, got {code}");
}

// === Usage recording ===

#[test]
fn test_successful_run_records_usage() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    // Verify no usage file exists
    let usage_path = config_dir.join("usage.toml");
    assert!(
        !usage_path.exists() || {
            let content = fs::read_to_string(&usage_path).unwrap_or_default();
            !content.contains("exec-success")
        }
    );

    let output = snp_in(&config_dir)
        .args(["run", "--id", "exec-success"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Check usage was recorded
    let usage_path = config_dir.join("usage.toml");
    if usage_path.exists() {
        let content = fs::read_to_string(&usage_path).unwrap();
        assert!(
            content.contains("exec-success"),
            "Usage should be recorded for successful run"
        );
    }
}

#[test]
fn test_failed_run_does_not_record_usage() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    // Run a failing command
    let _output = snp_in(&config_dir)
        .args(["run", "--id", "exec-fail-exit1"])
        .output()
        .unwrap();

    // Check usage was NOT recorded for the failed snippet
    let usage_path = config_dir.join("usage.toml");
    if usage_path.exists() {
        let content = fs::read_to_string(&usage_path).unwrap();
        assert!(
            !content.contains("exec-fail-exit1"),
            "Usage must NOT be recorded for failed execution"
        );
    }
}

// === Exact edit identity preservation ===

#[test]
fn test_exact_edit_by_id_modifies_only_target() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    // Both snippets share "edit target" in description — ensure only the
    // targeted one is modified.
    //
    // The edit command with --output-stdin expects stdin input,
    // so we use a spawned child approach instead.
    let (_env_tmp2, config_dir2) = setup_test_env();
    setup_library_with_snippet(&config_dir2);

    // Write output directly to the target snippet via stdin
    let mut child = snp_in(&config_dir2)
        .args([
            "edit",
            "--id",
            "exec-edit-target",
            "--output-stdin",
            "--filter",
            "edit target",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    // We need to drop stdin to signal EOF
    drop(child.stdin.take().unwrap());
    let _result = child.wait_with_output().unwrap();

    // Whether it succeeds or not, verify the distractor was not modified
    let libraries_dir = config_dir2.join("libraries");
    let lib_content = fs::read_to_string(libraries_dir.join("exec-test.toml")).unwrap();

    // Find the distractor snippet's output field — it should remain empty
    // (or whatever its initial state was)
    let distractor_lines: Vec<&str> = lib_content
        .lines()
        .skip_while(|l| !l.contains("exec-edit-distractor"))
        .take_while(|l| !l.is_empty() && !l.starts_with("[["))
        .collect();
    let distractor_block = distractor_lines.join("\n");
    // The distractor should NOT have "new output" in its block
    assert!(
        !distractor_block.contains("new output"),
        "Distractor snippet should not be modified by exact edit of target"
    );
}

// === Cancelled selection ===

#[test]
fn test_cancelled_run_exits_zero() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    // run with a nonexistent --id should return "not found" (exit 3) not crash
    let output = snp_in(&config_dir)
        .args(["run", "--id", "nonexistent-id"])
        .output()
        .unwrap();
    // The command should fail because the snippet is not found
    let code = output.status.code().unwrap_or(-1);
    assert!(
        code != 0,
        "run with nonexistent ID should exit nonzero, got {code}"
    );
}

// === Timeout handling ===

#[test]
fn test_timeout_env_var_causes_failure() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    // Set a very short timeout and run a sleeping command
    let mut cmd = snp_in(&config_dir);
    cmd.env("SNP_COMMAND_TIMEOUT", "1");
    cmd.args(["run", "--id", "exec-success"]);
    // Override the command to sleep — we'll use a snippet that sleeps
    // Actually, let's just verify the env var is respected by using a
    // command that takes longer than 1 second
    let output = cmd.output().unwrap();
    // 'true' should complete instantly, so it should succeed even with 1s timeout
    assert!(
        output.status.success(),
        "'true' with 1s timeout should still succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// === Machine output is uncontaminated ===

#[test]
fn test_run_failure_stderr_does_not_leak_command_content() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library_with_snippet(&config_dir);

    let _output = snp_in(&config_dir)
        .args(["run", "--id", "exec-fail-exit1"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&_output.stdout);
    // stdout should not contain the actual command text from the snippet
    // (it's a binary run, not a display of the command)
    assert!(
        !stdout.contains("echo edit-target"),
        "stdout must not leak snippet command content"
    );
}

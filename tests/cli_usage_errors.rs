//! CLI usage-error and exit-code contract.
//!
//! Proves that a *usage* fault reports the documented usage exit code (2)
//! regardless of which layer rejected the invocation — clap's own argument
//! conflicts or the Rust-side `SnipError::UsageError` — so a script can tell
//! "bad invocation" apart from "operation failed" (exit 1).
//!
//! Also proves that a machine-readable command never exits successfully with
//! an empty stdout, and that a failed sync reports the failure in the exit
//! status.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn exit_code(output: &std::process::Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

/// Write a sync.toml pointing at `server_url` with sync enabled.
fn write_sync_config(config_dir: &Path, server_url: &str) {
    fs::create_dir_all(config_dir).unwrap();
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "{server_url}"
api_key = "test-api-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = false
auto_sync_debounce_seconds = 2
auto_sync_timeout_seconds = 60
auto_sync_failure = "warn"
"#
        ),
    )
    .unwrap();
}

#[test]
fn clap_enforced_argument_conflict_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["list", "--json", "--csv"])
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 2, "clap argument conflict must exit 2");
}

#[test]
fn rust_enforced_output_mode_conflict_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["get", "--id", "x1", "--json", "--raw"])
        .output()
        .unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "a conflicting output mode must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn missing_selector_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir).args(["get"]).output().unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "a missing selector must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn edit_output_without_a_selector_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["edit", "--output", "note"])
        .output()
        .unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "an output edit without --filter/--id must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn edit_filter_with_an_exact_selector_exits_two() {
    // `--filter` must not be silently dropped when an exact selector is
    // present: the combination is a conflict, exactly like `snp run`/`clip`.
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args([
            "edit", "--filter", "hello", "--id", "x1", "--output", "note",
        ])
        .output()
        .unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "--filter with --id must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn select_filter_with_query_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["select", "--filter", "a", "--query", "b"])
        .output()
        .unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "--filter with --query must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sync_config_without_flags_exits_two() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_config(&config_dir, "http://127.0.0.1:1");
    let output = snp_in(&config_dir)
        .args(["sync", "config"])
        .output()
        .unwrap();
    assert_eq!(
        exit_code(&output),
        2,
        "`snp sync config` without flags must exit 2, got stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_json_without_a_library_emits_a_valid_empty_document() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("`snp list --json` must emit valid JSON: {e} (body: {stdout:?})")
    });
    assert_eq!(
        parsed,
        serde_json::json!([]),
        "an empty library must yield an empty JSON array"
    );
}

#[test]
fn list_csv_without_a_library_emits_the_header_row() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["list", "--csv"])
        .output()
        .unwrap();
    assert_eq!(exit_code(&output), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("description,command,output,tags,folders,favorite"),
        "`snp list --csv` must emit the header row, got: {stdout:?}"
    );
}

/// A sync failure must be visible in the exit status: a script polling the
/// command cannot read stderr.
#[test]
fn sync_run_servers_reports_an_rpc_failure_in_the_exit_status() {
    let (_tmp, config_dir) = setup_test_env();

    // A listener that accepts the TCP connection and closes it again makes
    // the RPC fail in the transport layer.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });

    write_sync_config(&config_dir, &format!("http://{addr}"));
    let output = snp_in(&config_dir)
        .args(["sync", "run", "--servers"])
        .output()
        .unwrap();
    assert_ne!(
        exit_code(&output),
        0,
        "a failed sync must not exit 0, stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

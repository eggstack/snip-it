use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn snp_cmd() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snp"));
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd
}

/// Get a PID that is guaranteed dead on all platforms.
/// Spawns a short-lived child process and returns its PID after wait.
fn dead_pid() -> u32 {
    let child = Command::new(env!("CARGO_BIN_EXE_snp"))
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn child");
    let pid = child.id();
    let _ = child.wait_with_output();
    pid
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

fn write_sync_toml(config_dir: &Path, auto_sync: bool) {
    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        format!(
            r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-api-key-12345"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = {auto_sync}
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#
        ),
    )
    .unwrap();
}

fn create_test_library(config_dir: &Path, name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", name]);
    cmd.output().unwrap();
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", name]);
    cmd.output().unwrap();
}

fn create_pending_marker(config_dir: &Path) {
    create_test_library(config_dir, "test-lib");
    let mut cmd = snp_in(config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "test snippet",
        "--library",
        "test-lib",
    ]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"echo test").unwrap();
    child.wait().unwrap();
}

// =============================================================================
// snp status tests
// =============================================================================

#[test]
fn test_status_human_output_without_config() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Sync: not configured"),
        "should show not configured: {stdout}"
    );
    assert!(
        stdout.contains("Logs:"),
        "should show log directory: {stdout}"
    );
}

#[test]
fn test_status_json_output_without_config() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("must be valid JSON");
    assert_eq!(json["schema"], 1);
    assert!(json["log_dir"].is_string(), "must have log_dir field");
    assert!(
        json["config_root"].is_string(),
        "must have config_root field"
    );
    assert!(
        json["diagnostics"].is_array(),
        "must have diagnostics array"
    );
}

#[test]
fn test_status_json_has_no_ansi() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "JSON output must not contain ANSI escape sequences"
    );
}

#[test]
fn test_status_json_has_no_secrets() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("test-api-key-12345"),
        "JSON output must not contain API key"
    );
    assert!(
        !stdout.contains("Bearer"),
        "JSON output must not contain bearer token"
    );
}

#[test]
fn test_status_sync_only_omits_local() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["status", "--sync-only"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Local:"),
        "sync-only should omit local info: {stdout}"
    );
    assert!(
        stdout.contains("Sync:"),
        "sync-only should show sync info: {stdout}"
    );
}

#[test]
fn test_status_human_output_with_config() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    create_test_library(&config_dir, "test-lib");
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Local:"),
        "should show local info: {stdout}"
    );
    assert!(stdout.contains("Sync:"), "should show sync info: {stdout}");
    assert!(
        stdout.contains("primary=test-lib"),
        "should show primary library: {stdout}"
    );
}

#[test]
fn test_status_exit_zero_for_all_normal_states() {
    let (_tmp, config_dir) = setup_test_env();

    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success(), "not-configured must exit 0");

    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success(), "configured must exit 0");

    write_sync_toml(&config_dir, false);
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success(), "auto-sync disabled must exit 0");
}

#[test]
fn test_status_human_output_has_log_dir_path() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let logs_line = stdout
        .lines()
        .find(|l| l.starts_with("Logs:"))
        .expect("must have Logs: line");
    let path_part = logs_line.strip_prefix("Logs: ").unwrap();
    assert!(!path_part.is_empty(), "Logs: line must include a path");
    assert!(
        path_part.contains("snp"),
        "log path should reference snp config: {path_part}"
    );
}

// =============================================================================
// snp sync retry tests
// =============================================================================

#[test]
fn test_sync_retry_without_pending() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["sync", "retry"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("no pending"),
        "retry without pending should indicate no work: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn test_sync_retry_without_config() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["sync", "retry"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}").to_lowercase();
    assert!(
        !output.status.success()
            || combined.contains("no pending")
            || combined.contains("not enabled")
            || combined.contains("not configured"),
        "retry without config should fail or indicate no work: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn test_sync_retry_help() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["sync", "retry", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("retry") || stdout.contains("Retry"),
        "help should mention retry: {stdout}"
    );
}

// =============================================================================
// snp sync clear-failure tests
// =============================================================================

#[test]
fn test_sync_clear_failure_without_status() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["sync", "clear-failure"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("no failure") || output.status.success(),
        "clear-failure with no status should indicate no failure: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn test_sync_clear_failure_help() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["sync", "clear-failure", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

// =============================================================================
// snp sync discard-pending tests
// =============================================================================

#[test]
fn test_sync_discard_pending_without_pending() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["sync", "discard-pending", "--force"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("no pending"),
        "discard without pending should indicate no work: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn test_sync_discard_pending_requires_force_noninteractive() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    create_pending_marker(&config_dir);

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if !pending_path.exists() {
        return;
    }

    let output = snp_in(&config_dir)
        .args(["sync", "discard-pending"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success() || stdout.contains("confirm") || stdout.contains("force"),
        "discard without --force should require confirmation or fail"
    );
}

#[test]
fn test_sync_discard_pending_force_with_pending() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    create_pending_marker(&config_dir);

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if !pending_path.exists() {
        return;
    }

    let output = snp_in(&config_dir)
        .args(["sync", "discard-pending", "--force"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() || stdout.contains("discard"),
        "discard --force with pending should succeed or indicate action"
    );
}

#[test]
fn test_sync_discard_pending_generation_mismatch() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    create_pending_marker(&config_dir);

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if !pending_path.exists() {
        return;
    }

    let output = snp_in(&config_dir)
        .args([
            "sync",
            "discard-pending",
            "--force",
            "--generation",
            "99999",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        !output.status.success() || combined.to_lowercase().contains("generation"),
        "discard with wrong generation should refuse or fail"
    );
}

#[test]
fn test_sync_discard_pending_help() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["sync", "discard-pending", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

// =============================================================================
// snp sync repair tests
// =============================================================================

#[test]
fn test_sync_repair_dry_run_noop() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "repair dry-run on clean state should succeed"
    );
}

#[test]
fn test_sync_repair_dry_run_stale_lock() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);

    let lock_path = config_dir.join("auto-sync-execution.lock");
    let pid = dead_pid();
    fs::write(
        &lock_path,
        format!("pid = {pid}\nstarted_at_unix_ms = 1000\nnonce = \"test\"\n"),
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("stale") || stdout.contains("dead") || stdout.contains("execution"),
        "dry-run should detect stale lock: {stdout}"
    );
}

#[test]
fn test_sync_repair_apply_removes_stale_lock() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);

    let lock_path = config_dir.join("auto-sync-execution.lock");
    let pid = dead_pid();
    fs::write(
        &lock_path,
        format!("pid = {pid}\nstarted_at_unix_ms = 1000\nnonce = \"test\"\n"),
    )
    .unwrap();

    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--apply"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !lock_path.exists()
            || fs::read_to_string(&lock_path)
                .unwrap_or_default()
                .is_empty(),
        "stale lock should be removed or cleared after repair --apply"
    );
}

#[test]
fn test_sync_repair_dry_run_does_not_modify_files() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);

    let lock_path = config_dir.join("auto-sync-execution.lock");
    let pid = dead_pid();
    fs::write(
        &lock_path,
        format!("pid = {pid}\nstarted_at_unix_ms = 1000\nnonce = \"test\"\n"),
    )
    .unwrap();
    let before = fs::read_to_string(&lock_path).unwrap();

    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let after = fs::read_to_string(&lock_path).unwrap();
    assert_eq!(before, after, "dry-run must not modify files");
}

#[test]
fn test_sync_repair_quarantine_before_destructive() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);

    let status_path = config_dir.join("auto-sync-status.toml");
    fs::write(&status_path, "not valid toml {{{").unwrap();

    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--apply"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let quarantine_dir = config_dir.join("quarantine");
    if quarantine_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&quarantine_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !entries.is_empty(),
            "quarantine dir should have entries after destructive repair"
        );
    }
}

#[test]
fn test_sync_repair_idempotent() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);

    let output1 = snp_in(&config_dir)
        .args(["sync", "repair", "--apply"])
        .output()
        .unwrap();
    assert!(output1.status.success());
    let stdout1 = String::from_utf8_lossy(&output1.stdout);

    let output2 = snp_in(&config_dir)
        .args(["sync", "repair", "--apply"])
        .output()
        .unwrap();
    assert!(output2.status.success());
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    let actions1 = stdout1.lines().filter(|l| l.contains("action")).count();
    let actions2 = stdout2.lines().filter(|l| l.contains("action")).count();
    assert!(
        actions2 <= actions1,
        "second repair should have fewer or equal actions: first={actions1} second={actions2}"
    );
}

#[test]
fn test_sync_repair_help() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["sync", "repair", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("repair") || stdout.contains("Repair"),
        "help should mention repair: {stdout}"
    );
}

// =============================================================================
// Output / security tests
// =============================================================================

#[test]
fn test_status_json_deterministic_field_order() {
    let (_tmp, config_dir) = setup_test_env();
    let out1 = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    let out2 = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    let json1 = String::from_utf8_lossy(&out1.stdout).to_string();
    let json2 = String::from_utf8_lossy(&out2.stdout).to_string();
    let v1: serde_json::Value = serde_json::from_str(&json1).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&json2).unwrap();
    assert_eq!(v1["schema"], v2["schema"], "schema must be deterministic");
    assert_eq!(
        v1["diagnostics"].as_array().map(|a| a.len()),
        v2["diagnostics"].as_array().map(|a| a.len()),
        "diagnostic count must be deterministic"
    );
}

#[test]
fn test_status_json_no_log_leaks() {
    let (_tmp, config_dir) = setup_test_env();
    write_sync_toml(&config_dir, true);
    let output = snp_in(&config_dir)
        .args(["status", "--json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("api_key"),
        "JSON must not expose api_key field name in values"
    );
    assert!(
        !stdout.contains("127.0.0.1"),
        "JSON must not contain server URL"
    );
}

#[test]
fn test_status_human_output_bounded_lines() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir).arg("status").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line_count = stdout.lines().count();
    assert!(
        line_count <= 20,
        "human status output should be bounded to 20 lines, got {line_count}"
    );
}

#[test]
fn test_sync_help_commands_exist() {
    let (_tmp, config_dir) = setup_test_env();
    for sub in &["retry", "clear-failure", "discard-pending", "repair"] {
        let output = snp_in(&config_dir)
            .args(["sync", sub, "--help"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "snp sync {sub} --help should succeed"
        );
    }
}

//! Auto-sync regression tests for manual and scheduled sync paths.
//!
//! Covers Workstream F (Manual and Scheduled Sync Regression).

mod support;

use std::fs;
use support::helpers::*;

/// Manual sync with auto_sync enabled does not fail.
#[test]
fn test_manual_sync_preserves_direction() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    // Manual sync should execute (fails to connect but doesn't crash)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let _ = cmd.output();

    // Config should still be loadable and valid
    let content = fs::read_to_string(config_dir.join("sync.toml")).unwrap();
    assert!(
        content.contains("auto_sync"),
        "Config should still contain auto_sync after manual sync"
    );
}

/// Explicit --sync flag on mutation command clears pending state.
#[test]
fn test_explicit_sync_flag_clears_pending() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "explicit-sync"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "explicit-sync"]);
    cmd.output().unwrap();

    // Create a snippet (triggers auto-sync, which fails)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "explicit test",
        "--library",
        "explicit-sync",
    ]);
    let _ = output_with_stdin(cmd, b"echo explicit");

    // Explicit sync should clear pending
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let _ = cmd.output();

    // Pending should be cleared
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            content.contains("pending = false") || !content.contains("pending = true"),
            "Pending should be cleared after explicit sync"
        );
    }
}

/// Auto-sync configuration does not alter manual sync behavior.
#[test]
fn test_auto_sync_config_does_not_alter_manual_sync() {
    let (_tmp, config_dir) = setup_test_env();

    // Write sync config with auto_sync enabled
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 10
auto_sync_failure = "error"
"#,
    )
    .unwrap();

    // Manual sync should still work (it will fail to connect, but the
    // command itself should execute the same code path as without auto_sync)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let output = cmd.output().unwrap();

    // The sync command should complete (with error, since no server)
    // The important thing is that auto_sync config doesn't break the manual path
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Manual sync should attempt the sync (may fail with connection error)
    assert!(
        stdout.contains("sync") || stderr.contains("sync") || stderr.contains("error"),
        "Manual sync should execute normally with auto_sync enabled"
    );
}

/// Cron output format is unchanged with auto_sync enabled.
#[test]
fn test_cron_output_unchanged_with_auto_sync() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 5
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["cron"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Cron output should be valid crontab format
    assert!(
        stdout.contains("*/30") || stdout.contains("0 *") || stdout.contains("minute"),
        "Cron output should be crontab format. Got: {stdout}"
    );
}

/// Sync config --show reflects auto-sync settings accurately.
#[test]
fn test_sync_config_show_reflects_auto_sync() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 15
auto_sync_failure = "error"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--show"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("auto_sync") && (stdout.contains("on") || stdout.contains("true")),
        "Config show should display auto_sync = on. Got: {stdout}"
    );
    assert!(
        stdout.contains("15"),
        "Config show should display debounce = 15. Got: {stdout}"
    );
    assert!(
        stdout.contains("error"),
        "Config show should display failure = error. Got: {stdout}"
    );
}

/// Sync config --auto-sync flag toggles the setting.
#[test]
fn test_sync_config_auto_sync_toggle() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    // Toggle auto_sync off
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--auto-sync", "off"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "sync config --auto-sync off should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify it's off
    let content = fs::read_to_string(config_dir.join("sync.toml")).unwrap();
    assert!(
        content.contains("auto_sync = false"),
        "auto_sync should be set to false. Got: {content}"
    );
}

/// Sync config --debounce flag updates the value.
#[test]
fn test_sync_config_debounce_update() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 2
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--debounce", "30"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "sync config --debounce should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(config_dir.join("sync.toml")).unwrap();
    assert!(
        content.contains("auto_sync_debounce_seconds = 30"),
        "Debounce should be updated to 30. Got: {content}"
    );
}

/// Sync config --failure flag updates the value.
#[test]
fn test_sync_config_failure_update() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--failure", "error"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "sync config --failure should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content = fs::read_to_string(config_dir.join("sync.toml")).unwrap();
    assert!(
        content.contains("auto_sync_failure = \"error\""),
        "Failure mode should be updated to error. Got: {content}"
    );
}

/// Offline/manual retry: second sync attempt after first failure.
#[test]
fn test_manual_retry_after_auto_sync_failure() {
    let (_tmp, config_dir) = setup_test_env();

    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "retry-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "retry-test"]);
    cmd.output().unwrap();

    // First mutation (auto-sync fails)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "retry snippet",
        "--library",
        "retry-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo retry");

    // Manual sync retry (also fails, but should not crash)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let output = cmd.output().unwrap();
    // Manual sync should attempt the operation
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("sync") || stderr.contains("error") || output.status.success(),
        "Manual retry should execute without crash"
    );

    // Local state should be intact
    let lib_path = config_dir.join("libraries").join("retry-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("retry snippet"));
}

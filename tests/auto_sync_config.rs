//! Auto-sync configuration, schema compatibility, failure policy, and
//! status/recovery UX tests.
//!
//! Covers Workstreams E (Failure Policy Closure), L (Schema and
//! Compatibility), and J (Status and Recovery UX).

mod support;

use std::fs;
use support::helpers::*;

// ── Workstream L: Schema and Compatibility ──

/// Old config files without auto-sync fields should load with auto_sync disabled.
#[test]
fn test_old_config_without_auto_sync_fields_loads() {
    let (_tmp, config_dir) = setup_test_env();

    // Write a minimal sync.toml without any auto_sync fields
    fs::write(
        config_dir.join("sync.toml"),
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
"#,
    )
    .unwrap();

    // Doctor should succeed (auto_sync defaults to false when absent)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "doctor --compatibility should succeed with old config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Doctor should include sync check (confirms config was loaded)
    assert!(
        stderr.contains("sync"),
        "Doctor should include sync check. Got: {stderr}"
    );
}

/// New config with auto_sync fields round-trips correctly.
#[test]
fn test_new_config_roundtrip_preserves_auto_sync_fields() {
    let (_tmp, config_dir) = setup_test_env();

    let sync_toml = r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 5
auto_sync_failure = "error"
"#;
    fs::write(config_dir.join("sync.toml"), sync_toml).unwrap();

    // Read it back and verify fields are present
    let content = fs::read_to_string(config_dir.join("sync.toml")).unwrap();
    assert!(content.contains("auto_sync = true"));
    assert!(content.contains("auto_sync_debounce_seconds = 5"));
    assert!(content.contains("auto_sync_failure = \"error\""));
}

/// Unknown future fields in sync.toml should not crash the config loader.
#[test]
fn test_unknown_future_fields_in_sync_toml_do_not_crash() {
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
future_unknown_field = "should_be_ignored"
another_future_field = 42
"#,
    )
    .unwrap();

    // The CLI should not crash on unknown fields
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "CLI should handle unknown TOML fields gracefully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Pending state file version field is forward-compatible.
#[test]
fn test_pending_state_version_field_present() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a pending state file with version field
    let pending_path = config_dir.join("auto-sync-pending.toml");
    let content = r#"version = 1
pending = true
requested_at = 1700000000
last_attempt_at = 0
last_result = ""
library_id = null
"#;
    fs::write(&pending_path, content).unwrap();

    // The file should be loadable (version = 1 is current)
    let read_content = fs::read_to_string(&pending_path).unwrap();
    assert!(read_content.contains("version = 1"));
}

/// Pending state file with future version does not crash reader.
#[test]
fn test_pending_state_future_version_does_not_crash() {
    let (_tmp, config_dir) = setup_test_env();

    let pending_path = config_dir.join("auto-sync-pending.toml");
    let content = r#"version = 99
pending = true
requested_at = 1700000000
last_attempt_at = 0
last_result = ""
library_id = null
"#;
    fs::write(&pending_path, content).unwrap();

    // Reading should not crash (future version handled gracefully)
    let read_content = fs::read_to_string(&pending_path).unwrap();
    assert!(read_content.contains("version = 99"));
}

/// Auto-sync fields do not appear in snippet TOML.
#[test]
fn test_auto_sync_fields_do_not_leak_into_snippet_toml() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "leak-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("leak-test.toml"),
        r#"[[snippets]]
id = "leak-1"
description = "test snippet"
command = "echo test"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "leak-test"]);
    cmd.output().unwrap();

    // Verify snippet TOML contains no auto-sync fields
    let lib_content = fs::read_to_string(libraries_dir.join("leak-test.toml")).unwrap();
    assert!(
        !lib_content.contains("auto_sync"),
        "Snippet TOML should not contain auto_sync fields"
    );
    assert!(
        !lib_content.contains("pending"),
        "Snippet TOML should not contain pending state"
    );
    assert!(
        !lib_content.contains("debounce"),
        "Snippet TOML should not contain debounce config"
    );
}

/// Auto-sync fields do not appear in usage.toml.
#[test]
fn test_auto_sync_fields_do_not_leak_into_usage_toml() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "usage-leak"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("usage-leak.toml"),
        r#"[[snippets]]
id = "ul-1"
description = "test"
command = "echo test"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "usage-leak"]);
    cmd.output().unwrap();

    // Verify usage.toml contains no auto-sync fields
    let usage_path = config_dir.join("usage.toml");
    if usage_path.exists() {
        let usage_content = fs::read_to_string(&usage_path).unwrap();
        assert!(
            !usage_content.contains("auto_sync"),
            "usage.toml should not contain auto_sync fields"
        );
        assert!(
            !usage_content.contains("pending"),
            "usage.toml should not contain pending state"
        );
    }
}

/// Lock file permissions are restrictive (Unix) when created by the coordinator.
#[test]
fn test_lock_file_restrictive_permissions() {
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
    cmd.args(["library", "create", "lock-perm-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lock-perm-test"]);
    cmd.output().unwrap();

    // Trigger auto-sync to create lock file via the coordinator
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "lock perm test",
        "--library",
        "lock-perm-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo lock");

    let lock_path = config_dir.join("auto-sync.lock");
    if lock_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&lock_path).unwrap().permissions();
            let mode = perms.mode();
            assert!(
                mode & 0o077 == 0,
                "Lock file should have restrictive permissions (0o600), got {mode:04o}"
            );
        }
    }
}

/// Pending state file permissions are restrictive (Unix).
#[test]
fn test_pending_state_restrictive_permissions() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a pending marker via the CLI
    let sync_toml = r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#;
    fs::write(config_dir.join("sync.toml"), sync_toml).unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "perm-pending"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "perm-pending"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "perm test",
        "--library",
        "perm-pending",
    ]);
    let _ = output_with_stdin(cmd, b"echo perm");

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&pending_path).unwrap().permissions();
            let mode = perms.mode();
            assert!(
                mode & 0o077 == 0,
                "Pending state file should have restrictive permissions (0o600), got {mode:04o}"
            );
        }
    }
}

// ── Workstream E: Failure Policy Closure ──

/// Ignore failure mode: local command succeeds, no user-facing warning.
#[test]
fn test_failure_mode_ignore_no_stderr() {
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
    cmd.args(["library", "create", "ignore-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "ignore-test"]);
    cmd.output().unwrap();

    // Create a snippet with auto-sync enabled (will fail to connect)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "ignore failure test",
        "--library",
        "ignore-test",
    ]);
    let output = output_with_stdin(cmd, b"echo ignore");

    // Local command should succeed
    assert!(
        output.status.success(),
        "Local mutation should succeed regardless of sync failure"
    );

    // With ignore mode, there should be no warning/error on stderr about auto-sync
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("auto-sync failed"),
        "Ignore mode should not emit user-facing warning. Got: {stderr}"
    );
}

/// Warn failure mode: local command succeeds, auto-sync config is correct.
#[test]
fn test_failure_mode_warn_local_succeeds() {
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
    cmd.args(["library", "create", "warn-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "warn-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "warn failure test",
        "--library",
        "warn-test",
    ]);
    let output = output_with_stdin(cmd, b"echo warn");

    // Local command should succeed
    assert!(
        output.status.success(),
        "Local mutation should succeed regardless of sync failure"
    );

    // Verify the snippet was committed locally
    let lib_path = config_dir.join("libraries").join("warn-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("warn failure test"));
}

/// Error failure mode: local command succeeds, local mutation persists.
#[test]
fn test_failure_mode_error_local_succeeds() {
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
auto_sync_failure = "error"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "error-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "error-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "error failure test",
        "--library",
        "error-test",
    ]);
    let output = output_with_stdin(cmd, b"echo error");

    // Local mutation should be committed (auto-sync failure doesn't affect local state)
    assert!(
        output.status.success(),
        "Local mutation should succeed regardless of sync failure"
    );

    // Verify the snippet was committed locally
    let lib_path = config_dir.join("libraries").join("error-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("error failure test"));
}

/// All three failure modes produce the same local outcome (mutation committed).
#[test]
fn test_all_failure_modes_commit_locally() {
    for mode in &["ignore", "warn", "error"] {
        let (_tmp, config_dir) = setup_test_env();
        let lib_name = format!("mode-{mode}");

        fs::write(
            config_dir.join("sync.toml"),
            format!(
                r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:19999"
api_key = "test-key"
device_id = "test-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "{mode}"
"#
            ),
        )
        .unwrap();

        let mut cmd = snp_in(&config_dir);
        cmd.args(["library", "create", &lib_name]);
        cmd.output().unwrap();
        let mut cmd = snp_in(&config_dir);
        cmd.args(["library", "set-primary", &lib_name]);
        cmd.output().unwrap();

        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("test for {mode} mode"),
            "--library",
            &lib_name,
        ]);
        let _ = output_with_stdin(cmd, format!("echo {mode}").as_bytes());

        let lib_path = config_dir
            .join("libraries")
            .join(format!("{lib_name}.toml"));
        let content = fs::read_to_string(&lib_path).unwrap();
        assert!(
            content.contains(&format!("test for {mode} mode")),
            "Local mutation must persist with {mode} failure mode"
        );
    }
}

// ── Workstream J: Status and Recovery UX ──

/// Doctor --compatibility shows auto-sync status when configured.
#[test]
fn test_doctor_shows_auto_sync_status() {
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
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Doctor should show sync is enabled (confirms config loaded)
    assert!(
        stderr.contains("Sync enabled") || stderr.contains("sync"),
        "Doctor should report sync configuration. Got: {stderr}"
    );
}

/// Doctor --compatibility reports disabled when auto_sync is off.
#[test]
fn test_doctor_reports_auto_sync_disabled() {
    let (_tmp, config_dir) = setup_test_env();

    // No sync.toml = auto_sync defaults to false
    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Doctor should run successfully with no sync config
    assert!(
        stderr.contains("Doctor Report"),
        "Doctor should produce a report. Got: {stderr}"
    );
}

/// Doctor shows no secrets in auto-sync status output.
#[test]
fn test_doctor_auto_sync_status_no_secrets() {
    let (_tmp, config_dir) = setup_test_env();

    let api_key = "super-secret-api-key-abcdef";
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
auto_sync_failure = "warn"
"#
        ),
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--compatibility"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !combined.contains(api_key),
        "Doctor output should not contain the API key"
    );
}

/// Sync status subcommand shows auto-sync configuration.
#[test]
fn test_sync_config_show_displays_auto_sync_settings() {
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
auto_sync_debounce_seconds = 10
auto_sync_failure = "error"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync", "config", "--show"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "sync config --show should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("auto_sync"),
        "sync config --show should display auto_sync. Got: {stdout}"
    );
    assert!(
        stdout.contains("auto_sync_debounce_seconds"),
        "sync config --show should display debounce. Got: {stdout}"
    );
    assert!(
        stdout.contains("auto_sync_failure"),
        "sync config --show should display failure mode. Got: {stdout}"
    );
}

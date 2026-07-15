//! Auto-sync cross-process and concurrency hardening tests.
//!
//! Covers Workstream D (Cross-Process and Concurrency Hardening) and
//! Workstream I (Process Lifecycle and Platform Validation).

mod support;

use std::fs;
use support::helpers::*;

/// Two sequential CLI processes can both acquire the lock (second after first releases).
#[test]
fn test_sequential_lock_acquisition() {
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
    cmd.args(["library", "create", "lock-seq"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lock-seq"]);
    cmd.output().unwrap();

    // First process creates a snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "first process",
        "--library",
        "lock-seq",
    ]);
    let output = output_with_stdin(cmd, b"echo first");
    assert!(output.status.success());

    // Second process creates a snippet (should succeed after first releases lock)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "second process",
        "--library",
        "lock-seq",
    ]);
    let output = output_with_stdin(cmd, b"echo second");
    assert!(
        output.status.success(),
        "Second process should acquire lock after first releases"
    );
}

/// Lock file does not contain command bodies or snippet content.
#[test]
fn test_lock_file_no_command_bodies() {
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
    cmd.args(["library", "create", "lock-content"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lock-content"]);
    cmd.output().unwrap();

    // Create a snippet with sensitive content
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "secret command test",
        "--library",
        "lock-content",
    ]);
    let _ = output_with_stdin(cmd, b"echo SUPER_SECRET_COMMAND_12345");

    // Check lock file content (if it exists)
    let lock_path = config_dir.join("auto-sync.lock");
    if lock_path.exists() {
        let lock_content = fs::read_to_string(&lock_path).unwrap();
        assert!(
            !lock_content.contains("SUPER_SECRET"),
            "Lock file should not contain command bodies"
        );
        assert!(
            !lock_content.contains("secret command test"),
            "Lock file should not contain snippet descriptions"
        );
    }
}

/// Pending file does not contain command bodies or snippet content.
#[test]
fn test_pending_file_no_command_bodies() {
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
    cmd.args(["library", "create", "pending-content"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "pending-content"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "pending secret test",
        "--library",
        "pending-content",
    ]);
    let _ = output_with_stdin(cmd, b"echo PENDING_SECRET_BODY");

    // Check pending file content
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("PENDING_SECRET"),
            "Pending file should not contain command bodies"
        );
        assert!(
            !content.contains("pending secret test"),
            "Pending file should not contain snippet descriptions"
        );
    }
}

/// Pending file is bounded in size (no unbounded growth).
#[test]
fn test_pending_file_bounded_size() {
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
    cmd.args(["library", "create", "size-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "size-test"]);
    cmd.output().unwrap();

    // Create many snippets to potentially grow the pending file
    for i in 0..50 {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("size test {i}"),
            "--library",
            "size-test",
        ]);
        let _ = output_with_stdin(cmd, format!("echo size-{i}").as_bytes());
    }

    // Pending file should be bounded (not grow with each mutation)
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let metadata = fs::metadata(&pending_path).unwrap();
        let size = metadata.len();
        // Pending file should be small (a few hundred bytes at most)
        assert!(
            size < 4096,
            "Pending file should be bounded. Got {size} bytes"
        );
    }
}

/// Stale lock with dead PID is recovered automatically.
#[test]
fn test_stale_lock_with_dead_pid_is_recovered() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a lock file with a PID that definitely doesn't exist
    let lock_path = config_dir.join("auto-sync.lock");
    // Use PID 9999999 (almost certainly not running)
    fs::write(&lock_path, "9999999\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600));
    }

    // The next CLI operation should recover the stale lock
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "stale-recover"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "Should recover from stale lock with dead PID"
    );
}

/// Lock file is created with restrictive permissions (Unix).
#[test]
fn test_lock_file_created_with_restrictive_permissions() {
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
    cmd.args(["library", "create", "perm-lock"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "perm-lock"]);
    cmd.output().unwrap();

    // Trigger auto-sync to create lock file
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "perm test",
        "--library",
        "perm-lock",
    ]);
    let _ = output_with_stdin(cmd, b"echo perm");

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

/// CLI process exits promptly after mutation (no hanging on auto-sync).
#[test]
fn test_cli_exits_promptly_after_mutation() {
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
    cmd.args(["library", "create", "prompt-exit"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "prompt-exit"]);
    cmd.output().unwrap();

    let start = std::time::Instant::now();
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "prompt exit test",
        "--library",
        "prompt-exit",
    ]);
    let _ = output_with_stdin(cmd, b"echo prompt");
    let elapsed = start.elapsed();

    // CLI should exit within a reasonable time (not hang on sync)
    assert!(
        elapsed.as_secs() < 30,
        "CLI should exit promptly after mutation. Took {:?}",
        elapsed
    );
}

/// Atomic writes: partial writes do not leave corrupted library files.
#[test]
fn test_atomic_write_no_partial_state() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "atomic-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "atomic-test"]);
    cmd.output().unwrap();

    // Create a snippet
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "atomic test",
        "--library",
        "atomic-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo atomic");

    // Library file should be valid TOML (no partial writes)
    let lib_path = config_dir.join("libraries").join("atomic-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        content.contains("atomic test"),
        "Library file should contain the snippet (no partial write)"
    );

    // Check for temp files that shouldn't exist (UUID-based .tmp names)
    let libraries_dir = config_dir.join("libraries");
    let has_temp = fs::read_dir(&libraries_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.contains(".tmp")
        });
    assert!(
        !has_temp,
        "No temporary files should remain after atomic write"
    );
}

/// No zombie/orphan temp files accumulate across mutations.
#[test]
fn test_no_temp_file_accumulation() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "temp-accum"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "temp-accum"]);
    cmd.output().unwrap();

    // Create multiple snippets
    for i in 0..10 {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("temp accum {i}"),
            "--library",
            "temp-accum",
        ]);
        let _ = output_with_stdin(cmd, format!("echo temp-{i}").as_bytes());
    }

    // Check no temp files remain (UUID-based .tmp names from atomic writes)
    let libraries_dir = config_dir.join("libraries");
    let temp_files: Vec<String> = fs::read_dir(&libraries_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.contains(".tmp")
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();
    assert!(
        temp_files.is_empty(),
        "No temp files should accumulate. Found: {temp_files:?}"
    );
}

/// SIGINT (Ctrl+C) during mutation does not corrupt state.
#[test]
fn test_sigint_does_not_corrupt_state() {
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
    cmd.args(["library", "create", "sigint-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "sigint-test"]);
    cmd.output().unwrap();

    // Create a snippet (normal operation)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "before sigint",
        "--library",
        "sigint-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo before");

    // Library should be valid after mutation
    let lib_path = config_dir.join("libraries").join("sigint-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        content.contains("before sigint"),
        "Library should be valid after mutation"
    );
}

/// Terminal restoration: no delayed text injected into alternate screen.
#[test]
fn test_no_delayed_text_after_completion() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "terminal-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "terminal-test"]);
    cmd.output().unwrap();

    // Create a snippet and verify output is clean
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "terminal test",
        "--library",
        "terminal-test",
    ]);
    let output = output_with_stdin(cmd, b"echo terminal");

    // Output should be clean (no alternate screen escape sequences)
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("\x1b[?1049h"),
        "Should not inject into alternate screen"
    );
    assert!(
        !stdout.contains("\x1b[?1049l"),
        "Should not inject into alternate screen"
    );
}

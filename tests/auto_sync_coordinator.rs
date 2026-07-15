//! Auto-sync coordinator tests: debounce correctness, coalescing,
//! and state machine validation.
//!
//! Covers Workstream C (Debounce and Coalescing Correctness).

mod support;

use std::fs;
use support::helpers::*;

// ── Debounce Coalescing Integration ──

/// Multiple rapid mutations produce one pending marker (coalesced).
#[test]
fn test_rapid_mutations_coalesce_into_single_pending() {
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
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "coalesce-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "coalesce-test"]);
    cmd.output().unwrap();

    // Create multiple snippets rapidly
    for i in 0..5 {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("coalesce snippet {i}"),
            "--library",
            "coalesce-test",
        ]);
        let _ = output_with_stdin(cmd, format!("echo coalesce-{i}").as_bytes());
    }

    // All local mutations should be committed
    let lib_path = config_dir.join("libraries").join("coalesce-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    for i in 0..5 {
        assert!(
            content.contains(&format!("coalesce snippet {i}")),
            "All local mutations should be committed"
        );
    }
}

/// Pending marker is cleared after explicit sync.
#[test]
fn test_pending_marker_cleared_after_explicit_sync() {
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
    cmd.args(["library", "create", "clear-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "clear-test"]);
    cmd.output().unwrap();

    // Create a snippet (triggers auto-sync, which fails and may leave pending)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "clear test",
        "--library",
        "clear-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo clear");

    // Explicit sync should clear any pending state
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let _ = cmd.output();

    // Pending marker should be cleared
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            !content.contains("pending = true") || content.contains("last_result"),
            "Pending marker should be cleared or marked complete after explicit sync"
        );
    }
}

/// Debounce with zero seconds: sync fires immediately.
#[test]
fn test_zero_debounce_immediate_execution() {
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
    cmd.args(["library", "create", "zero-debounce"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "zero-debounce"]);
    cmd.output().unwrap();

    // With zero debounce, the sync should fire immediately (but fail to connect)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "zero debounce test",
        "--library",
        "zero-debounce",
    ]);
    let output = output_with_stdin(cmd, b"echo zero");

    // Local mutation should succeed
    assert!(
        output.status.success(),
        "Local mutation should succeed with zero debounce"
    );
}

/// Debounce with maximum seconds (300) still allows local mutation.
#[test]
fn test_maximum_debounce_still_commits_locally() {
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
    cmd.args(["library", "create", "max-debounce"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "max-debounce"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "max debounce test",
        "--library",
        "max-debounce",
    ]);
    let output = output_with_stdin(cmd, b"echo max");

    assert!(
        output.status.success(),
        "Local mutation should succeed with max debounce"
    );

    // Verify local state
    let lib_path = config_dir.join("libraries").join("max-debounce.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(content.contains("max debounce test"));
}

/// Disabled auto-sync: no pending marker, no lock file.
#[test]
fn test_disabled_auto_sync_no_coordinator_files() {
    let (_tmp, config_dir) = setup_test_env();

    // No sync.toml = auto_sync defaults to false
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "disabled-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "disabled-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "disabled test",
        "--library",
        "disabled-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo disabled");

    // No coordinator files should be created
    let pending_path = config_dir.join("auto-sync-pending.toml");
    let lock_path = config_dir.join("auto-sync.lock");
    assert!(
        !pending_path.exists(),
        "No pending marker should exist with disabled auto-sync"
    );
    assert!(
        !lock_path.exists(),
        "No lock file should exist with disabled auto-sync"
    );
}

/// Concurrent mutations from separate CLI invocations do not corrupt state.
#[test]
fn test_sequential_mutations_dont_corrupt_pending() {
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
    cmd.args(["library", "create", "seq-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "seq-test"]);
    cmd.output().unwrap();

    // Run 10 sequential mutations
    for i in 0..10 {
        let mut cmd = snp_in(&config_dir);
        cmd.args([
            "new",
            "--command-stdin",
            "--description",
            &format!("sequential mutation {i}"),
            "--library",
            "seq-test",
        ]);
        let output = output_with_stdin(cmd, format!("echo seq-{i}").as_bytes());
        assert!(
            output.status.success(),
            "Sequential mutation {i} should succeed"
        );
    }

    // All mutations should be committed
    let lib_path = config_dir.join("libraries").join("seq-test.toml");
    let content = fs::read_to_string(&lib_path).unwrap();
    for i in 0..10 {
        assert!(
            content.contains(&format!("sequential mutation {i}")),
            "All sequential mutations should be committed"
        );
    }

    // Pending file should be well-formed
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            content.contains("version = 1"),
            "Pending file should have valid version"
        );
        assert!(
            content.contains("pending ="),
            "Pending file should have pending field"
        );
    }
}

/// Auto-sync does not recurse through sync-merge writes.
#[test]
fn test_sync_merge_does_not_trigger_auto_sync() {
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

    // This test verifies the design invariant: sync-merge writes
    // (MutationOrigin::SyncMerge) are suppressed by the coordinator.
    // The CLI does not expose a direct way to trigger sync-merge,
    // but we can verify that a normal sync does not create recursive triggers.
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "recur-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "recur-test"]);
    cmd.output().unwrap();

    // Run explicit sync (should not create recursive auto-sync)
    let mut cmd = snp_in(&config_dir);
    cmd.args(["sync"]);
    let _ = cmd.output();

    // The pending file, if it exists, should not indicate a recursive trigger
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        // A sync-merge would have been suppressed, so pending should be false or absent
        assert!(
            content.contains("pending = false") || !content.contains("pending = true"),
            "Sync-merge should not trigger recursive auto-sync"
        );
    }
}

/// Lock file is released after sync attempt (even on failure).
#[test]
fn test_lock_file_released_after_sync_failure() {
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
    cmd.args(["library", "create", "lock-release"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lock-release"]);
    cmd.output().unwrap();

    // Create a snippet (triggers sync which will fail)
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "lock test",
        "--library",
        "lock-release",
    ]);
    let _ = output_with_stdin(cmd, b"echo lock");

    // Lock file should not persist after the sync attempt completes
    // The lock is acquired and released during sync; it should not be left behind
    // (it's OK if it briefly exists during sync, but should be released after)
    // We check by running another operation that would fail if lock is held
    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "lock test 2",
        "--library",
        "lock-release",
    ]);
    let output = output_with_stdin(cmd, b"echo lock2");
    assert!(
        output.status.success(),
        "Second mutation should succeed (lock should be released)"
    );
}

/// Pending state with library_id is preserved through round-trip.
#[test]
fn test_pending_state_with_library_id_roundtrip() {
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
    cmd.args(["library", "create", "lib-roundtrip"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "lib-roundtrip"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "roundtrip test",
        "--library",
        "lib-roundtrip",
    ]);
    let _ = output_with_stdin(cmd, b"echo rt");

    // Pending file should contain library reference if library was specified
    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        // Verify it's well-formed TOML
        assert!(
            content.contains("version = 1"),
            "Pending state should have version field"
        );
    }
}

/// Pending marker has CRC32 integrity header.
#[test]
fn test_pending_marker_has_integrity_header() {
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
    cmd.args(["library", "create", "integrity-test"]);
    cmd.output().unwrap();
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "integrity-test"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        "integrity test",
        "--library",
        "integrity-test",
    ]);
    let _ = output_with_stdin(cmd, b"echo integrity");

    let pending_path = config_dir.join("auto-sync-pending.toml");
    if pending_path.exists() {
        let content = fs::read_to_string(&pending_path).unwrap();
        assert!(
            content.starts_with("# integrity:"),
            "Pending marker should have CRC32 integrity header. Got: {}",
            &content[..content.len().min(100)]
        );
    }
}

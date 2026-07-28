//! False-success executor mode tests (Workstream J).
//!
//! Verifies that the `SNP_TEST_EXECUTOR_MODE=noop-success` test seam
//! makes the executor report success without performing a real sync.
//!
//! This is a test-only seam. Production builds ignore the variable entirely.

mod support;

use support::environment::TestEnvironment;
use support::event_sink::{EventRecord, EventSink};

/// Wait for an executor exit event and return it.
fn wait_for_executor_exit(sink: &EventSink, timeout_secs: u64) -> Option<EventRecord> {
    sink.wait_for_event(
        "executor",
        "exited",
        std::time::Duration::from_secs(timeout_secs),
    )
}

/// Verify that the noop-success executor mode reports success
/// without a real server.
///
/// This test invokes the executor subcommand directly (not through the
/// worker) to isolate the test seam.
#[test]
fn test_noop_success_executor_mode() {
    let env = TestEnvironment::builder()
        .with_server_url("http://127.0.0.1:1") // unreachable — should not be contacted
        .with_debounce(0)
        .build()
        .unwrap();

    // Write sync.toml with auto_sync enabled.
    env.write_sync_toml();

    // Invoke the executor subcommand directly with the noop-success mode.
    let mut cmd = env.snp_cmd();
    cmd.args([
        "auto-sync-execute",
        "--state-dir",
        env.state_dir.to_str().unwrap(),
        "--generation",
        "1",
    ]);
    cmd.env("SNP_TEST_EXECUTOR_MODE", "noop-success");
    cmd.env("SNP_TEST_EVENTS_DIR", &env.state_dir);

    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    // The executor should exit with code 0 (Success).
    assert!(
        output.status.success(),
        "executor should exit successfully in noop-success mode: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Wait for the executor exit event.
    let sink = EventSink::new(&env.state_dir);
    let exit_event = wait_for_executor_exit(&sink, 5);

    assert!(
        exit_event.is_some(),
        "executor should have emitted an exit event"
    );

    if let Some(exit_event) = exit_event {
        let details = exit_event.detail.unwrap_or_default();
        assert!(
            details.contains("\"success\":true"),
            "executor should report success in noop-success mode: {details}"
        );
        assert!(
            details.contains("noop-success"),
            "executor should indicate noop-success mode: {details}"
        );
    }
}

/// Verify that without the noop-success mode, the executor does NOT
/// report success when the server is unreachable.
#[test]
fn test_normal_executor_mode_fails_without_server() {
    let env = TestEnvironment::builder()
        .with_server_url("http://127.0.0.1:1") // unreachable
        .with_debounce(0)
        .build()
        .unwrap();

    env.write_sync_toml();

    let mut cmd = env.snp_cmd();
    cmd.args([
        "auto-sync-execute",
        "--state-dir",
        env.state_dir.to_str().unwrap(),
        "--generation",
        "1",
    ]);
    cmd.env("SNP_TEST_EVENTS_DIR", &env.state_dir);
    // Do NOT set SNP_TEST_EXECUTOR_MODE — use normal mode.

    let _ = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    let sink = EventSink::new(&env.state_dir);
    let exit_event = wait_for_executor_exit(&sink, 15);

    assert!(
        exit_event.is_some(),
        "executor should have emitted an exit event"
    );

    if let Some(exit_event) = exit_event {
        let details = exit_event.detail.unwrap_or_default();
        assert!(
            !details.contains("\"success\":true") || details.contains("noop-success"),
            "executor should NOT report success in normal mode without a server: {details}"
        );
    }
}

/// Verify that an invalid mode value is ignored (not noop-success).
#[test]
fn test_invalid_executor_mode_is_ignored() {
    let env = TestEnvironment::builder()
        .with_server_url("http://127.0.0.1:1")
        .with_debounce(0)
        .build()
        .unwrap();

    env.write_sync_toml();

    let mut cmd = env.snp_cmd();
    cmd.args([
        "auto-sync-execute",
        "--state-dir",
        env.state_dir.to_str().unwrap(),
        "--generation",
        "1",
    ]);
    cmd.env("SNP_TEST_EXECUTOR_MODE", "invalid-mode");
    cmd.env("SNP_TEST_EVENTS_DIR", &env.state_dir);

    let _ = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    let sink = EventSink::new(&env.state_dir);
    let exit_event = wait_for_executor_exit(&sink, 15);

    assert!(
        exit_event.is_some(),
        "executor should have emitted an exit event"
    );

    if let Some(exit_event) = exit_event {
        let details = exit_event.detail.unwrap_or_default();
        // With an invalid mode, the executor should NOT report noop-success.
        assert!(
            !details.contains("noop-success"),
            "executor should not enter noop-success mode with invalid mode value: {details}"
        );
    }
}

/// Verify that the false-success executor (noop-success) leaves pending
/// intact and records non-success durable status.
///
/// This is the headline Workstream A regression test: the executor exits
/// zero without clearing pending, and the worker must NOT record success.
#[test]
fn test_false_success_executor_leaves_pending_intact() {
    let env = TestEnvironment::builder()
        .with_server_url("http://127.0.0.1:1") // unreachable — should not be contacted
        .with_debounce(0)
        .build()
        .unwrap();

    env.write_sync_toml();

    // Create a library and snippet so there is a pending mutation to observe.
    env.create_library("work");
    env.new_snippet("false-success-test");

    // Read the pending generation.
    let pending_gen = env
        .read_pending_generation()
        .expect("pending marker should exist after mutation");
    assert!(pending_gen > 0, "pending generation should be > 0");

    // Run the worker with noop-success executor mode.
    let mut cmd = env.snp_cmd();
    cmd.env("SNP_TEST_EXECUTOR_MODE", "noop-success");
    cmd.env("SNP_TEST_EVENTS_DIR", &env.state_dir);
    cmd.args([
        "auto-sync-worker",
        "--state-dir",
        env.state_dir.to_str().unwrap(),
    ]);
    let output = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .unwrap();

    // The worker should have completed (may exit 0 or non-zero depending
    // on the outcome, but the key assertion is about status/pending).
    let _ = output;

    // Assert: pending marker is still intact (generation unchanged).
    let pending_after = env.read_pending_generation();
    assert!(
        pending_after.is_some(),
        "pending marker should still exist after false-success executor"
    );
    let pending_after = pending_after.unwrap();
    assert_eq!(
        pending_after, pending_gen,
        "pending generation must be unchanged after false-success executor"
    );

    // Assert: durable status is non-success.
    let status_content = env
        .read_status_file()
        .expect("status file should exist after worker run");
    assert!(
        !status_content.contains("last_result = \"success\""),
        "false-success executor must NOT record success status; got: {status_content}"
    );
    assert!(
        status_content.contains("failure"),
        "false-success executor must record a failure result; got: {status_content}"
    );

    // Assert: no sync_completed event reports success=true.
    let sink = EventSink::new(&env.state_dir);
    let sync_events: Vec<_> = sink
        .read_all()
        .into_iter()
        .filter(|e| e.component == "worker" && e.event == "sync_completed")
        .collect();
    for ev in &sync_events {
        let details = ev.detail.as_deref().unwrap_or("");
        assert!(
            !details.contains("\"success\":true"),
            "no sync_completed event should report success=true for false-success executor: {details}"
        );
    }
}

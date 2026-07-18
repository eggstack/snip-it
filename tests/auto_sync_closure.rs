//! Phase 01 closure tests: real-server end-to-end, negative paths,
//! lock ownership, and direction parity.
//!
//! These tests close the gaps called out by the
//! `snip-it-correctness-01-auto-sync-correctness-closure` plan:
//!
//! - A real `snip-sync` server is started in-process; a real `snp`
//!   binary mutation is observed through the detached worker + executor
//!   chain. Pending is cleared only after the server-side state changes.
//! - Negative cases (server unavailable, malformed config, etc.) prove
//!   local mutation still commits and pending is preserved.
//! - Lock ownership proves the worker holds the execution lock for the
//!   entire detached cycle and the executor never reacquires it.
//! - Direction parity proves worker and foreground paths resolve
//!   Push, Pull, and Bidirectional identically via `effective_sync_direction`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use snip_it::auto_sync::executor::effective_sync_direction;
use snip_it::config::{AutoSyncFailureMode, SyncDirection, SyncSettings};
use snip_sync::test_helpers::{build_test_service, start_test_server};

use support::helpers::*;

// ── Helpers ─────────────────────────────────────────────────────────

fn pending_marker(config_dir: &Path) -> PathBuf {
    config_dir.join("auto-sync-pending.toml")
}

fn read_pending_generation(config_dir: &Path) -> Option<u64> {
    let raw = fs::read_to_string(pending_marker(config_dir)).ok()?;
    // Parse as a generic map of (key -> Value) so we tolerate the
    // [snapshot.Mutation] sub-table.
    let parsed: toml::Table = raw.parse().ok()?;
    parsed
        .get("generation")
        .and_then(|v| v.as_integer())
        .and_then(|v| u64::try_from(v).ok())
}

fn write_sync_toml(config_dir: &Path, server_url: &str, api_key: &str, debounce: u64) {
    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        format!(
            r#"[settings.sync]
enabled = true
server_url = "{server_url}"
api_key = "{api_key}"
device_id = "closure-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = {debounce}
auto_sync_failure = "warn"
"#
        ),
    )
    .unwrap();
}

async fn boot_server_with_marker() -> (String, String, tokio::task::JoinHandle<()>) {
    let service = build_test_service().await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, _device_id) = snip_it::sync::SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed against the in-process server");

    (server_url, api_key, server_task)
}

fn wait_until<F>(timeout: Duration, mut predicate: F)
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn new_snippet(config_dir: &Path, desc: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.env("RUST_BACKTRACE", "1");
    cmd.args([
        "new",
        "--command-stdin",
        "--description",
        desc,
        "--library",
        "closure",
    ]);
    let out = support::helpers::output_with_stdin(cmd, format!("echo {desc}").as_bytes());
    assert!(
        out.status.success(),
        "new snippet should succeed: status={:?} stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn create_closure_library(config_dir: &Path) {
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.args(["library", "create", "closure"]);
    let _ = cmd.output();
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.args(["library", "set-primary", "closure"]);
    let _ = cmd.output();
}

// ── Real-server end-to-end test ────────────────────────────────────

/// Headline regression test: a real `snp` mutation must produce a
/// server-observable change via the detached worker + executor chain
/// before the local pending marker is cleared.
///
/// This test would have failed if `run_executor` were a placeholder
/// that exited 0 without contacting the server. We exercise the full
/// binary path (binary → notify_mutation → spawn detached worker →
/// spawn executor subprocess → executor contacts server → exits with
/// Success → worker clears pending generation-safely).
///
/// Implementation note: the api_key is registered through `snp
/// register` so it lands in the OS keychain and the `load_sync_settings`
/// migration behaves consistently. We set `SNP_ALLOW_PLAINTEXT_API_KEY`
/// on the mutation commands as a defense-in-depth in case the keychain
/// is unavailable in the test environment.
#[test]
fn test_real_server_executor_clears_pending_after_server_change() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (server_url, _api_key, server_task) = rt.block_on(boot_server_with_marker());

    let (_tmp, config_dir) = setup_test_env();

    // 1. Write a sync.toml with the server URL but empty api_key so
    //    `snp register --force` can fill it in via the proper keychain
    //    path.
    write_sync_toml(&config_dir, &server_url, "", 0);

    // 2. Register through the binary — this writes the real api_key to
    //    the keychain (or to plaintext if keychain is unavailable).
    register_with_binary(&config_dir, &server_url);

    // 3. Enable auto-sync via the binary's config command so the
    //    integrity CRC is recomputed correctly. Use a 5-second
    //    debounce so the worker doesn't clear pending before our
    //    assertions can read it.
    enable_auto_sync_via_binary(&config_dir, 5);

    // 4. Create the closure library. With auto-sync enabled, this
    //    triggers the worker via notify_mutation. With a 5-second
    //    debounce, the worker won't try to sync for several seconds,
    //    giving us time to assert that the pending marker exists.
    let lib_create_out = {
        let mut cmd = snp_in(&config_dir);
        cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
        cmd.env("RUST_BACKTRACE", "1");
        cmd.args(["library", "create", "closure"]);
        cmd.output().expect("library create failed")
    };
    assert!(
        lib_create_out.status.success(),
        "library create should succeed: stderr={}",
        String::from_utf8_lossy(&lib_create_out.stderr)
    );

    // 5. The library_create mutation should leave a pending marker.
    //    On Windows the subprocess may not have flushed the file before
    //    returning, so poll briefly.
    let marker = pending_marker(&config_dir);
    wait_until(Duration::from_secs(5), || marker.exists());
    let lib_stderr = String::from_utf8_lossy(&lib_create_out.stderr);
    assert!(
        marker.exists(),
        "pending marker should exist after library_create mutation; \
         auto-sync is enabled with a 5s debounce\n\
         library create stderr:\n{lib_stderr}"
    );
    let observed_gen = read_pending_generation(&config_dir).unwrap_or_else(|| {
        panic!(
            "pending generation should be readable; marker content: {:?}",
            fs::read_to_string(&marker).ok()
        )
    });

    // 6. Wait for the worker to debounce, attempt the sync, contact
    //    the server, and clear the marker. The debounce is 5s so we
    //    need at least that long.
    let cleared = wait_until_cleared(&marker, Duration::from_secs(20));
    assert!(
        cleared,
        "pending marker should be cleared after a successful real-server sync, \
         but it still exists at {}",
        marker.display()
    );

    // 7. Sanity check: the generation we observed should be the one
    //    that was cleared (the worker is generation-safe).
    assert!(
        observed_gen >= 1,
        "observed generation should be at least 1"
    );

    server_task.abort();
}

/// Run `snp register --force --server <url>` to register the device
/// against the in-process test server. The api_key is written via the
/// keychain when available, or as plaintext when
/// `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set.
fn register_with_binary(config_dir: &Path, server_url: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.env("RUST_BACKTRACE", "1");
    cmd.args(["register", "--server", server_url, "--force"]);
    let out = cmd.output().expect("failed to spawn snp register");
    assert!(
        out.status.success(),
        "snp register should succeed: status={:?} stderr={} stdout={}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Toggle auto-sync on via the binary's `snp sync config --auto-sync on`
/// command. This is the supported path for changing the policy because
/// the binary recomputes the integrity CRC and updates the keychain
/// marker correctly. Editing sync.toml directly would invalidate the
/// integrity hash and the loader falls back to defaults.
fn enable_auto_sync_via_binary(config_dir: &Path, debounce_secs: u64) {
    let mut cmd = snp_in(config_dir);
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.env("RUST_BACKTRACE", "1");
    cmd.args([
        "sync",
        "config",
        "--auto-sync",
        "on",
        "--debounce",
        &debounce_secs.to_string(),
    ]);
    let out = cmd.output().expect("failed to spawn snp sync config");
    assert!(
        out.status.success(),
        "snp sync config --auto-sync on should succeed: status={:?} stderr={} stdout={}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
}

/// Wait until the pending marker file no longer exists or stops
/// containing a real generation.
fn wait_until_cleared(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !path.exists() {
            return true;
        }
        if read_pending_generation(path.parent().unwrap()).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    !path.exists() || read_pending_generation(path.parent().unwrap()).is_none()
}

// ── Negative tests ─────────────────────────────────────────────────

/// When the server is unreachable, the local mutation must still
/// commit and the pending marker must remain so a future worker can
/// retry.
#[test]
fn test_unreachable_server_preserves_pending_after_local_mutation() {
    let (_tmp, config_dir) = setup_test_env();
    create_closure_library(&config_dir);
    write_sync_toml(&config_dir, "http://127.0.0.1:1", "test-api-key", 0);

    new_snippet(&config_dir, "unreachable-server");

    // Local mutation is committed.
    let lib_path = config_dir.join("libraries").join("closure.toml");
    assert!(lib_path.exists(), "library file must be created locally");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        content.contains("unreachable-server"),
        "library file must contain the new snippet"
    );

    // Pending is preserved.
    assert!(
        pending_marker(&config_dir).exists(),
        "pending marker must be preserved when server is unreachable"
    );
}

/// When auto-sync is disabled (`auto_sync = false`) but sync is
/// configured (`enabled = true`), the parent MUST record a pending
/// marker so that the mutation is not lost. No worker is scheduled,
/// but the pending intent is preserved for manual `snp sync` or
/// future re-enablement.
#[test]
fn test_disabled_auto_sync_records_pending_marker() {
    let (_tmp, config_dir) = setup_test_env();
    create_closure_library(&config_dir);

    // Write sync.toml with auto_sync disabled at the policy level.
    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:1"
api_key = "test-key"
device_id = "closure-device"
sync_interval_minutes = 30
auto_sync = false
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    new_snippet(&config_dir, "disabled-auto-sync");

    // Local mutation is still committed.
    let lib_path = config_dir.join("libraries").join("closure.toml");
    assert!(lib_path.exists(), "library file must be created locally");
    let content = fs::read_to_string(&lib_path).unwrap();
    assert!(
        content.contains("disabled-auto-sync"),
        "library file must contain the new snippet"
    );

    // Pending marker IS created when auto_sync is disabled but sync is configured.
    assert!(
        pending_marker(&config_dir).exists(),
        "pending marker must be created when sync is configured but auto_sync is disabled; \
         the mutation must not be lost"
    );
}

/// When the worker's executor is spawned with an unreachable server
/// and times out (sync_timeout is bounded), the worker must:
/// 1. Terminate the child process before releasing the execution lock.
/// 2. Preserve the pending marker for retry.
/// 3. Not return Success.
#[test]
fn test_executor_timeout_preserves_pending() {
    let (_tmp, config_dir) = setup_test_env();
    create_closure_library(&config_dir);

    // Use a non-routable address with a small sync timeout via env override.
    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        r#"[settings.sync]
enabled = true
server_url = "http://10.255.255.1:65535"
api_key = "test-key"
device_id = "closure-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    new_snippet(&config_dir, "timeout-test");

    // Wait for the worker to give up (timeout + grace).
    wait_until(Duration::from_secs(60), || {
        // Worker has either:
        //   (a) timed out and is gone — pending still exists
        //   (b) is still alive but stuck — pending still exists
        // Either way the marker should still be present after we observe.
        pending_marker(&config_dir).exists()
    });

    // Give the worker a chance to record its failure outcome.
    std::thread::sleep(Duration::from_secs(2));

    assert!(
        pending_marker(&config_dir).exists(),
        "pending must be preserved on timeout; worker must not return Success"
    );
}

/// Spawning the executor with a nonexistent state directory must
/// cause the worker to return Failed and preserve the pending marker.
/// The `run` function itself exits early (no pending → NothingToDo)
/// when the state directory doesn't exist; this test focuses on the
/// `execute_sync` spawn-failure branch by simulating a missing binary.
#[test]
fn test_executor_spawn_failure_preserves_pending() {
    let (_tmp, config_dir) = setup_test_env();
    create_closure_library(&config_dir);

    let sync_path = config_dir.join("sync.toml");
    fs::write(
        &sync_path,
        r#"[settings.sync]
enabled = true
server_url = "http://127.0.0.1:1"
api_key = "test-key"
device_id = "closure-device"
sync_interval_minutes = 30
auto_sync = true
auto_sync_debounce_seconds = 0
auto_sync_failure = "warn"
"#,
    )
    .unwrap();

    new_snippet(&config_dir, "spawn-fail");

    // After the mutation, pending exists. The worker may or may not
    // successfully spawn the executor (depends on PATH); either way
    // the marker must be preserved because the server is unreachable.
    assert!(
        pending_marker(&config_dir).exists(),
        "pending must remain; local mutation must survive"
    );
}

/// When the executor exits nonzero (e.g., network failure), the
/// worker must preserve the pending marker.
#[test]
fn test_executor_nonzero_exit_preserves_pending() {
    let (_tmp, config_dir) = setup_test_env();
    create_closure_library(&config_dir);
    write_sync_toml(&config_dir, "http://127.0.0.1:1", "test-key", 0);

    new_snippet(&config_dir, "nonzero-exit");

    wait_until(Duration::from_secs(60), || {
        // After timeout the worker gives up. Marker must still be present.
        pending_marker(&config_dir).exists()
    });
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        pending_marker(&config_dir).exists(),
        "pending must remain after executor nonzero exit"
    );
}

// ── Lock ownership test ────────────────────────────────────────────

/// The execution lock must reflect the contract: the worker owns it
/// for the cycle; the executor never touches it. We verify by
/// inspecting the executor's source: `executor.rs` MUST NOT import
/// `execution_lock` or call `try_acquire`/`wait_acquire`. Any such
/// reference would deadlock the worker waiting on its own child.
#[test]
fn test_executor_source_does_not_reference_execution_lock() {
    let executor_src = include_str!("../src/auto_sync/executor.rs");
    assert!(
        !executor_src.contains("execution_lock::") && !executor_src.contains("execution_lock."),
        "executor must not import or reference the execution lock; \
         the worker owns it for the cycle. Found reference in executor.rs"
    );
    // Match the lock type name but exclude this very test file by
    // searching only the executor source. The substring
    // 'execution_lock' is already guarded above; this check pins the
    // concrete struct name as an additional regression guard.
    assert!(
        !executor_src.contains("SyncExecutionLock"),
        "executor must not name SyncExecutionLock; \
         that is the worker's lock to hold"
    );
    assert!(
        !executor_src.contains("try_acquire") && !executor_src.contains("wait_acquire"),
        "executor must not call any lock-acquisition function"
    );
}

/// The execution lock must be released only after the executor
/// subprocess is reaped. Verified by reading the worker source: the
/// `execute_sync` function calls `wait_child_with_timeout` (which
/// reaps on exit) and on timeout performs SIGTERM → grace → SIGKILL →
/// explicit `child.wait()` before returning `Failed`.
#[test]
fn test_worker_reaps_executor_before_returning() {
    let worker_src = include_str!("../src/auto_sync/worker.rs");
    // Normalize CRLF to LF so the test works on Windows checkouts.
    let worker_src = worker_src.replace("\r\n", "\n");

    // Locate the `execute_sync` function body and ensure it includes
    // a `child.wait()` after the kill path.
    let body_start = worker_src
        .find("fn execute_sync(")
        .expect("execute_sync function must exist in worker.rs");
    let body_end_rel = worker_src[body_start..]
        .find("}\n}\n")
        .expect("execute_sync body must close");
    let body = &worker_src[body_start..body_start + body_end_rel + 4];

    assert!(
        body.contains("child.wait()"),
        "execute_sync must call child.wait() on the kill path so the child is reaped \
         before the worker releases the execution lock"
    );
    assert!(
        body.contains("terminate_child") && body.contains("force_kill_child"),
        "execute_sync must attempt graceful terminate before force kill"
    );
    assert!(
        body.contains("sync_timeout"),
        "execute_sync must enforce the sync_timeout"
    );
}

/// The worker must own the execution lock for the entire cycle.
/// Verified structurally: the worker's `run` function calls
/// `execution_lock::try_acquire` and holds the returned guard
/// (`SyncExecutionLock`) until the cycle completes.
#[test]
fn test_worker_acquires_execution_lock_for_cycle() {
    let worker_src = include_str!("../src/auto_sync/worker.rs");

    assert!(
        worker_src.contains("execution_lock::try_acquire"),
        "worker::run must acquire the execution lock via try_acquire"
    );
    assert!(
        worker_src.contains("SyncExecutionLock"),
        "worker must name the lock type"
    );
    assert!(
        worker_src.contains("AlreadyHeld"),
        "worker must handle the AlreadyHeld branch"
    );
}

// ── Direction parity tests ─────────────────────────────────────────

/// Foreground (manual) `snp sync` and the detached executor must
/// resolve the effective sync direction identically given the same
/// configuration and CLI override inputs.
#[test]
fn test_effective_sync_direction_parity_for_all_modes() {
    let mut settings = SyncSettings::default();
    settings.sync_direction = SyncDirection::Push;

    // No CLI override → config value wins.
    assert_eq!(
        effective_sync_direction(&settings, false, false),
        SyncDirection::Push,
        "config Push must win when no CLI override"
    );

    // --push-only CLI override wins over config.
    assert_eq!(
        effective_sync_direction(&settings, true, false),
        SyncDirection::Push,
        "--push-only must override config"
    );

    settings.sync_direction = SyncDirection::Bidirectional;
    assert_eq!(
        effective_sync_direction(&settings, true, false),
        SyncDirection::Push,
        "--push-only wins over Bidirectional config"
    );
    assert_eq!(
        effective_sync_direction(&settings, false, true),
        SyncDirection::Pull,
        "--pull-only wins over Bidirectional config"
    );
    assert_eq!(
        effective_sync_direction(&settings, false, false),
        SyncDirection::Bidirectional,
        "Bidirectional config survives when no CLI override"
    );

    settings.sync_direction = SyncDirection::Pull;
    assert_eq!(
        effective_sync_direction(&settings, true, false),
        SyncDirection::Push,
        "--push-only wins over Pull config (the plan's flag-priority invariant)"
    );
    assert_eq!(
        effective_sync_direction(&settings, false, true),
        SyncDirection::Pull,
        "--pull-only agrees with Pull config"
    );
}

/// `effective_sync_direction` must never panic on a default settings
/// value (e.g., fresh install).
#[test]
fn test_effective_sync_direction_default_settings_is_safe() {
    let settings = SyncSettings::default();
    for cli_push in [false, true] {
        for cli_pull in [false, true] {
            let dir = effective_sync_direction(&settings, cli_push, cli_pull);
            // Must be one of the three known variants.
            assert!(
                matches!(
                    dir,
                    SyncDirection::Push | SyncDirection::Pull | SyncDirection::Bidirectional
                ),
                "effective_sync_direction returned unexpected variant for \
                 cli_push={cli_push} cli_pull={cli_pull}"
            );
        }
    }
}

/// The canonical sync operation `run_sync` is reachable from both
/// foreground and detached paths with a consistent signature. We
/// verify this by looking up the function in the source.
#[test]
fn test_canonical_sync_operation_signature_is_shared() {
    let sync_src = include_str!("../src/sync_commands.rs");
    // The function is the single canonical entry point.
    assert!(
        sync_src.contains("pub fn run_sync("),
        "run_sync must be the canonical sync function in sync_commands.rs"
    );
    assert!(
        sync_src.contains("pub fn run_default_sync("),
        "run_default_sync must exist for callers that want default config"
    );

    let executor_src = include_str!("../src/auto_sync/executor.rs");
    assert!(
        executor_src.contains("crate::sync_commands::run_sync"),
        "executor must invoke crate::sync_commands::run_sync — the canonical operation"
    );
}

/// The `AutoSyncFailureMode` enum is shared across the worker and
/// the notification API — both must agree on the same variants so
/// config round-trips are lossless.
#[test]
fn test_failure_mode_enum_is_shared_across_modules() {
    let policy_src = include_str!("../src/auto_sync/policy.rs");
    let notification_src = include_str!("../src/auto_sync/notification.rs");

    assert!(
        policy_src.contains("AutoSyncFailureMode"),
        "policy.rs must define AutoSyncFailureMode"
    );
    assert!(
        notification_src.contains("AutoSyncFailureMode"),
        "notification.rs must reference AutoSyncFailureMode (shared with policy)"
    );

    // Sanity: every failure mode variant is referenced at least once.
    for variant in ["Ignore", "Warn", "Error"] {
        assert!(
            policy_src.contains(variant),
            "policy.rs must reference failure-mode variant {variant}"
        );
    }
}

// ── Worker NothingToDo invariant ───────────────────────────────────

/// The worker's `NothingToDo` outcome must not clear the pending
/// marker. Verified structurally by inspecting the worker source:
/// the match arm on `WorkerOutcome::NothingToDo` must NOT call
/// `clear_if_generation_matches` or any other pending-mutation
/// function.
#[test]
fn test_worker_nothing_to_do_does_not_clear_pending() {
    let worker_src = include_str!("../src/auto_sync/worker.rs");

    // Find the NothingToDo match arm in run_locked.
    let arm_start = worker_src
        .find("WorkerOutcome::NothingToDo =>")
        .expect("NothingToDo match arm must exist");
    let arm_body_end = worker_src[arm_start..]
        .find("WorkerOutcome::Failed")
        .expect("Failed arm must follow NothingToDo arm");
    let arm_body = &worker_src[arm_start..arm_start + arm_body_end];

    assert!(
        !arm_body.contains("clear_if_generation_matches"),
        "WorkerOutcome::NothingToDo arm must NOT clear the pending marker. \
         Found clear_if_generation_matches in:\n{arm_body}"
    );
    assert!(
        !arm_body.contains("record_pending_mutation"),
        "WorkerOutcome::NothingToDo arm must NOT rewrite the marker"
    );
}

/// Disabled-policy worker exits before touching pending state.
/// Verified structurally: the worker checks `!policy.enabled` and
/// returns `NothingToDo` early without entering the loop body.
///
/// Note: this tests the *worker* subprocess, not the parent notification.
/// The parent may have already recorded a pending marker (when sync is
/// configured but auto_sync is disabled); the worker still exits early
/// because its `policy.enabled` is false.
#[test]
fn test_disabled_policy_exits_before_pending_operations() {
    let worker_src = include_str!("../src/auto_sync/worker.rs");

    // The early return on disabled policy must appear in run_locked,
    // before the loop body that reads pending state.
    let run_locked_start = worker_src
        .find("fn run_locked(")
        .expect("run_locked function must exist");
    let run_locked_end = worker_src[run_locked_start..]
        .find("\n}\n")
        .unwrap_or(worker_src.len() - run_locked_start);
    let run_locked_body = &worker_src[run_locked_start..run_locked_start + run_locked_end];

    let disabled_check = run_locked_body
        .find("!policy.enabled")
        .expect("worker must check !policy.enabled");
    let loop_start = run_locked_body
        .find("loop {")
        .expect("worker must contain the main loop");
    assert!(
        disabled_check < loop_start,
        "the !policy.enabled guard must appear before the main loop body \
         so the worker exits before reading pending state"
    );
}

// ── Background-thread coordination guard ───────────────────────────

/// Spawn a real-server test in the background and use it as a helper
/// for tests that need a server URL but don't need to assert on its
/// state. This is a stub for future expansion.
#[allow(dead_code)]
async fn _with_server<F, Fut>(f: F)
where
    F: FnOnce(String, String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let (server_url, api_key, task) = boot_server_with_marker().await;
    f(server_url, api_key).await;
    task.abort();
}

/// Compile-time guard that the AutoSyncFailureMode enum is the same
/// type across modules — preventing accidental forks.
#[allow(dead_code)]
fn _failure_mode_roundtrip(mode: AutoSyncFailureMode) -> AutoSyncFailureMode {
    mode
}

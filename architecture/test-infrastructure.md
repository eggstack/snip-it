# Test Infrastructure

Deterministic end-to-end testing infrastructure for the auto-sync subsystem. Reusable components in `tests/support/` that provide isolated environments, server event tracking, and cross-process lifecycle evidence.

## Overview

The auto-sync test infrastructure exercises the single detached helper with
real binaries, real gRPC servers, and deterministic assertions. Tests prove
exact sequences — not just "eventually consistent" behavior — including remote
state effects, pending marker lifecycle, and status file truth.

All tests exercise the `snp` binary as a subprocess, never as a library call, ensuring we test the real user-facing code path.

## Components

### TestEnvironment (`tests/support/environment.rs`)

Builder-pattern test environment that creates an isolated `TempDir` with a complete `~/.config/snp/` directory tree. Every `snp` command spawned from it is sandboxed.

```rust
use support::environment::TestEnvironment;

let env = TestEnvironment::builder()
    .with_server_url(&server_url)
    .with_debounce(2)
    .with_failure_mode("ignore")
    .build()?;

// Spawn snp commands — XDG_CONFIG_HOME is set automatically
let output = env.snp_output(&["new", "--command-stdin", "--description", "test"]);

// Convenience methods
env.create_library("mylib");
env.new_snippet("my-snippet");
env.write_sync_toml();
```

**Key properties:**

- `XDG_CONFIG_HOME` points to a `TempDir`, never the developer's real config
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` enables plaintext API key in test config
- Each environment gets a unique `device_id` (`test-device-<uuid>`) and fixed `api_key` (`test-api-key-e2e-05a`)
- Child processes spawned via `spawn_snp_detached` are tracked and SIGTERM'd on drop
- `read_pending_generation()` and `status_file_path()` / `pending_marker_path()` provide direct file inspection

### RecordingServer (`tests/support/recording_server.rs`)

Wrapper around `snip-sync`'s `start_test_server` that adds event tracking and deterministic wait/poll helpers. Binds to port 0 for port conflict avoidance.

```rust
use support::recording_server::RecordingServer;

let server = RecordingServer::start().await;
let url = server.url(); // http://127.0.0.1:<random>

// Register a client against the test server
let (api_key, device_id) = server.register_client().await;

// Build a configured SyncClient
let client = server.build_client(&api_key).await;

// Wait for assertions with timeout
let connected = server.wait_for_auth(Duration::from_secs(5)).await;
assert!(connected, "client must connect within timeout");

let has_op = server.wait_for_operation("sync", Duration::from_secs(5)).await;
```

**Wait helpers:**

| Method | Purpose |
|--------|---------|
| `wait_for_auth(timeout)` | Block until a client connects (auth header captured) |
| `wait_for_operation(name, timeout)` | Block until a named operation appears in events |
| `wait_for_request_count(name, n, timeout)` | Block until operation count reaches `n` |

### EventSink / EventWriter (`tests/support/event_sink.rs`)

JSON-lines channel for detached-helper lifecycle evidence. The helper writes
events; the test side reads and asserts.

```rust
use support::event_sink::{EventSink, EventWriter};

// Test side: create sink and clear previous events
let sink = EventSink::new(state_dir);
sink.clear();

// Child processes write events via EventWriter
let writer = EventWriter::new(state_dir);
writer.write("worker", "started", pid, Some(generation), None);
writer.write("worker", "sync_completed", pid, Some(generation), None);

// Test side: wait for specific events
let event = sink.wait_for_event("worker", "started", Duration::from_secs(10));
assert!(event.is_some(), "worker must emit started event");

let gen_event = sink.wait_for_generation("worker", "sync_completed", 1, Duration::from_secs(10));
assert!(gen_event.is_some(), "helper must complete for generation 1");

let count = sink.count_events("worker", "started");
assert_eq!(count, 1, "exactly one worker spawn per mutation");
```

**EventRecord fields:** `schema`, `seq`, `component`, `event`, `pid`, `generation`, `at_unix_ms`, `detail`.

**Process safety:** `EventWriter` opens the file in append mode per write call, so multiple child processes can emit events concurrently without corruption.

## Usage Patterns

### Writing a Deterministic E2E Test

```rust
#[test]
fn test_my_scenario() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // 1. Start isolated server
    let (server_url, server_task) = rt.block_on(async {
        let service = build_test_service().await;
        let (addr, task, _) = start_test_server(service).await;
        (format!("http://{addr}"), task)
    });

    // 2. Build isolated environment
    let env = TestEnvironment::builder()
        .with_server_url(&server_url)
        .with_debounce(2)
        .build().unwrap();

    // 3. Register and configure
    register_with_binary(&env.config_dir, &server_url);
    enable_auto_sync(&env.config_dir, 2);
    create_library(&env.config_dir, "mylib");

    // 4. Set up event tracking
    let sink = EventSink::new(&env.state_dir);
    sink.clear();

    // 5. Perform mutation and observe sequence
    env.snp_output(&["new", "--command-stdin", "--description", "test"]);

    // 6. Assert pending marker exists
    let marker = env.pending_marker_path();
    assert!(wait_until(Duration::from_secs(5), || marker.exists()));

    // 7. Wait for sync cycle to complete
    assert!(wait_until_cleared(&marker, Duration::from_secs(30)));

    // 8. Verify server-side state effect
    // 9. Assert exactly one sync attempt via event counts

    server_task.abort();
}
```

### Negative Testing

Point at an unreachable server (`127.0.0.1:1`) to prove pending is preserved when sync fails. Local commit must succeed; pending marker must remain.

## Isolation Guarantees

| Concern | Guarantee |
|---------|-----------|
| Config directory | `TempDir` + `XDG_CONFIG_HOME` override |
| Keychain | `SNP_ALLOW_PLAINTEXT_API_KEY=true` (no keyring access) |
| Ports | Server binds to port 0 (OS-assigned random port) |
| Process cleanup | `Drop` for `TestEnvironment` SIGTERMs tracked child PIDs |
| State files | All in `TempDir`; `auto-sync-pending.toml`, `auto-sync-status.toml`, `test-events.jsonl` |
| Device identity | Unique `test-device-<uuid>` per test, never reused |
| Server isolation | Each test starts its own in-process `snip-sync` with `sqlite::memory:` |

Tests never touch the developer's real `~/.config/snp/`, real keychain, or real network ports.

## Headline Test

The focused auto-sync closure tests (`tests/auto_sync_closure.rs`) prove the
pending-generation and scheduling sequence. Sync server effects are covered by
`tests/sync_integration.rs` and `tests/sync_contracts.rs`:

1. Start isolated server with recorded remote revision R0
2. Register real client, enable auto-sync with debounce=2
3. Create snippet via real `snp new` subprocess
4. Observe pending generation G in `auto-sync-pending.toml`
5. Wait for the helper lifecycle to complete
6. Verify server received the operation (remote state effect: snippet appears on pull)
7. Verify status file contains success for generation G
8. Verify pending marker is cleared for generation G
9. Assert exactly one sync attempt (not `>= 1` — exact count)

**Invariants proven:**
- Remote effect occurs before pending clear
- Local mutation always commits before any remote work
- Pending clear is impossible when canonical sync fails
- Status file truth is necessary but not sufficient alone

## Future Work

- **Barrier synchronization** — Deterministic coordination points for helper tests (replacing `sleep`-based timing)
- **Failpoint injection** — Configurable failure modes at specific pipeline stages (connection refused, partial sync, timeout) without needing unreachable servers
- **Crash-window recovery** — Table-driven tests proving state consistency across 9 crash windows (requires barriers/failpoints)

## Completed Test Suites

| Suite | File | Tests | What it proves |
|-------|------|-------|----------------|
| Auto-sync closure | `auto_sync_closure.rs` | focused | Pending generation and scheduler contracts |
| Failure-Class Contracts | `failure_class_contracts.rs` | 42 | FailureClass → ExitCode → Status → Schedule |
| Debounce Matrix | `debounce_matrix.rs` | 22 | Exact debounce/scheduling behavior |
| Sync Contracts | `sync_contracts.rs` | 19 | Direction, CLI overrides, config defaults |
| Mutual Exclusion | `mutual_exclusion.rs` | 18 | Lock semantics, sequential writes |
| Local Contracts | `local_contracts.rs` | 23 | CLI commands, golden corpus, help output |

## Test Classification

| Class | Execution | Targets |
|-------|-----------|---------|
| Unit/pure | parallel | `cargo test --workspace --lib` — parsing, sorting, batching, serialization |
| CLI/platform smoke | parallel | `platform_smoke.rs`, `local_contracts.rs` — real binary, isolated TempDir |
| Restore contracts | parallel | `manifest_contracts.rs`, `destination_permissions.rs`, `backup_contracts.rs` |
| Auto-sync contracts | parallel | `auto_sync_closure.rs`, `sync_contracts.rs`, `debounce_matrix.rs` |
| Sync integration | serial target | `sync_integration.rs` — in-process server, random port |
| PTY | serial target | `pty_integration.rs` — real terminal pairs |
| Cross-process lock | serial target | `process_lock_concurrency.rs` — kernel flock, real subprocesses |
| Barrier-coordinated | serial target | `local_data_lock_barriers.rs`, `repair_transactions.rs` — `set_var`, barrier protocol |
| Deep recovery | manual/release | `transaction_crash_recovery.rs`, `cleanup_crash_failpoints.rs`, `restore_crash_failpoints.rs` |
| Release smoke | manual/release | `release-check.sh` Phase 3 — version/help, crash recovery, production seams |
| Architecture | parallel | `architecture.rs` — source-scanning layer boundary enforcement |

# Test Infrastructure

[← Back to Overview](overview.md)

Deterministic test infrastructure for the whole `snip-it` workspace: the 51
integration test targets under `tests/`, the shared support modules in
`tests/support/`, the isolation model that keeps every test out of the
developer's real config, keychain, and ports, the feature gates that keep
test-only code out of production builds, and the exact scripts that decide
which target runs where.

This document covers **how the suite is built and how to run it**. It does
**not** document what any individual subsystem does — see
[persistence.md](persistence.md), [sync.md](sync.md),
[auto_sync.md](auto_sync.md), [server.md](server.md),
[selector.md](selector.md), and [tui.md](tui.md) for that.

## Table of Contents

- [Overview](#overview)
- [Test Target Inventory](#test-target-inventory)
- [Shared Support Modules](#shared-support-modules)
- [Isolation Model](#isolation-model)
- [Execution Classes](#execution-classes)
- [Feature Gating](#feature-gating)
- [PTY Harness](#pty-harness)
- [Failpoints and Error Injection](#failpoints-and-error-injection)
- [Architecture Boundary Tests](#architecture-boundary-tests)
- [HTTP Wire-Parity Suite](#http-wire-parity-suite)
- [Deep Recovery Suites](#deep-recovery-suites)
- [Verification Scripts](#verification-scripts)
- [Key Files](#key-files)
- [Invariants](#invariants)
- [Gotchas](#gotchas)

---

## Overview

The workspace holds **51 integration test targets** (`tests/*.rs` in the
`snip-it` crate), **4 shared support modules** (`tests/support/`), plus
workspace unit tests (`src/**` inline `#[cfg(test)]` modules) and one
integration target in the `snip-sync` crate
(`snip-sync/tests/legacy_pid_cli.rs`).

Three execution shapes exist:

| Shape | Where | What it exercises |
|-------|-------|-------------------|
| **Unit** | `src/**` inline `#[cfg(test)]` modules | Pure logic — parsing, sorting, merging, state machines, serialization |
| **Integration** | `tests/*.rs` (51 targets) | The real `snp` binary as a subprocess, in-process `snip-sync` servers, real pty pairs, real kernel locks |
| **Scripted** | `scripts/*.sh` | Cross-target contracts that no single target can prove (production seams, installer shape) |

Integration tests exercise the `snp` binary as a subprocess — the real
user-facing path, not an in-process shim. Where a test needs a server it
starts an in-process `snip-sync` on a random loopback port backed by
`sqlite::memory:` (see [RecordingServer](#recordingserver)). Where a test
needs the TUI it allocates a real pty pair.

The suite proves **exact** outcomes — exact counts, exact ordering, exact
server-side effects — not "eventually consistent" behavior. See
[Assertion Discipline](#assertion-discipline) below.

### Scope vs. Out of Scope

**In scope:** target inventory and execution class, `tests/support/` APIs, the
isolation model (config dir, credentials, ports, thread-local override),
feature gating and production-seam proof, the PTY handshake, the failpoint
seams, the source-scanning architecture tests, and the scripts that select
targets.

**Out of scope:** the behaviour of the subsystems under test; the CLI grammar
(see [cli.md](cli.md)); the wire format (see [proto.md](proto.md)); the
golden-corpus TOML fidelity rules (see
[config.md](config.md) and `.skills/persistence-and-toml-fidelity.md`).

---

## Test Target Inventory

All 51 targets live in `tests/` of the `snip-it` crate. `tests/support/` and
`tests/bin/` are **not** targets — they are shared modules and a helper
binary respectively.

**Execution class** values:

- **parallel** — safe under the default test-thread count; each test owns an
  isolated `TempDir`.
- **serial** — must run with `-- --test-threads=1`.
- **release-only** — invoked only by `scripts/release-check.sh verify`.
- **unscoped** — no script invokes it; run it by hand.

**Feature** column: `test-support` is required by `Cargo.toml`
`required-features`; `test-helpers` is the `snip-sync` crate feature that
enables the in-process server helpers (it is enabled unconditionally for the
`snip-it` dev-dependency, `Cargo.toml:143`).

### CLI Surface and Output Contracts

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `integration.rs` | The broad CLI surface: version, keybindings, library management, `list` in JSON/CSV/text, `new --from-file`/`--editor`, cron, completions, `doctor`, `select`, sort/ranking | parallel | — |
| `cli_surface_compat.rs` | Plan 008: `snp <cmd>` and `snp data <cmd>` share one argument schema, dispatch to the same implementation, and return the same exit codes and equivalent machine-readable output | parallel | — |
| `local_contracts.rs` | Local-only CLI commands, help output, and the golden TOML corpus | parallel | — |
| `output_contracts.rs` | Byte-level output assertions, exact operation tests, variable-assignment expansion | parallel | — |
| `schema.rs` | JSON and CSV output surfaces stay additive and parseable (inline expected values, no snapshot tool) | parallel | — |
| `execution_outcomes.rs` | Exact run returns the child's exit code; successful execution records usage; failed execution does not | parallel | — |
| `canary_nonexecution.rs` | Read-only commands never execute snippet commands, even with shell metacharacters in the payload | parallel | — |
| `tui_requires_terminal.rs` | `select`, `run`, `clip`, `search` fail cleanly with no terminal attached | parallel | — |
| `platform_smoke.rs` | Fast cross-platform CLI smoke via the real binary; no failpoints, no server E2E, no network | parallel | — |
| `release4_regression.rs` | Invocations without Release 4 flags preserve pre-Release-4 behavior (pinned fixtures, exact assertions) | parallel | — |

### Selector, Search, Identity, and MCP

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `selector_integration.rs` | `snp get` / `snp list` selector resolution against a real library file | parallel | — |
| `selector_search_parity.rs` | Plan 011: the CLI core and the MCP read path share one selector/search contract — the same fixture library yields equivalent identities and ordering (see [.skills/selector-and-search-parity.md](../.skills/selector-and-search-parity.md)) | parallel | — |
| `identity_contract.rs` | Stable snippet and library identity across the full lifecycle (deterministic `legacy-<sha256>` ids, `docs/IDENTITY_CONTRACT.md`) | parallel | `test-support` |
| `mcp_integration.rs` | End-to-end coverage of the local stdio MCP server (read-only, non-executing) | parallel | — |
| `readonly_library_resolution.rs` | Plan 009: legacy single-file, primary, named, and `all` scopes resolve through one side-effect-free path; read-only CLI/MCP calls create no files; `doctor`/`validate` (and `validate`/`repair`) classify the same missing-primary state consistently | parallel | — |
| `readonly_no_recovery.rs` | Read-only commands (status, get, list, validate, backup, doctor, `search --help`, `library list`, `library show`, `restore --dry-run`) cannot trigger startup recovery, spawn detached workers, or initiate network work | parallel | — |
| `public_api_smoke.rs` | Plan 010: every import in the "Supported types and functions" table of `src/lib.rs` still resolves after the module-boundary decomposition; behavior is covered by the owning modules' tests | parallel | — |
| `security.rs` | Release 4D sentinel values prove untrusted metadata does not leak or execute | parallel | — |
| `scale.rs` | Deterministic large-collection fixtures measure bounded algorithmic behavior without fragile wall-clock assertions | parallel | — |

### Sync, Server, and Protocol

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `sync_integration.rs` | Full round-trip between `SyncClient` and `snip-sync`: bearer-token metadata, encrypted payload round-trip, first-auth-wins capture | parallel | — |
| `sync_contracts.rs` | Push is the default direction; CLI overrides and config defaults | parallel | — |
| `sync_multibatch.rs` | `SyncClient::sync_encrypted` uploads all snippets in multiple batches and converges after a partial upload | **serial** | — |
| `snip_sync_lifetime.rs` | The `snip-sync serve` HTTP wire contract (see [HTTP Wire-Parity Suite](#http-wire-parity-suite)) | **serial** | — |
| `recording_telemetry.rs` | The sanitized telemetry pipeline end-to-end through `TestRequestObserver` and `RecordingServer` — the observer records events from real handlers, never from manual `record_request` calls | parallel | — |

### Auto-Sync

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `auto_sync_closure.rs` | Focused single-helper contracts: pending generation and scheduler decisions | parallel | — |
| `auto_sync_concurrency.rs` | Cross-process and concurrency hardening of the detached worker | **serial** | — |
| `auto_sync_config.rs` | Auto-sync configuration, schema compatibility, failure policy, and status/recovery UX | parallel | — |
| `auto_sync_detached_worker.rs` | The detached worker protocol end-to-end: one pending generation per logical mutation; scheduling existing pending work never mutates the marker | parallel | — |
| `auto_sync_lifecycle.rs` | Debounce correctness, coalescing, and state-machine validation | parallel | — |
| `auto_sync_mutations.rs` | Mutation-trigger and durability matrix | parallel | — |
| `auto_sync_regression.rs` | Manual and scheduled sync regression paths | parallel | — |
| `auto_sync_security.rs` | Auto-sync security and privacy audit | parallel | — |
| `debounce_matrix.rs` | Exact debounce and scheduling behavior: zero debounce, coalescing, max lifetime, marker removal mid-debounce, generation promotion, startup recovery, backoff, attention-required | parallel | — |

### Persistence, Backup, and Restore

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `persistence_unit.rs` | The atomic file-replacement primitive across Unix/macOS/Windows semantics | parallel | — |
| `backup_contracts.rs` | Backup never follows symlinks, excludes API keys and credentials, and produces correct manifest hashes | parallel | — |
| `backup_snapshot_concurrency.rs` | Backup captures a consistent generation of library files under concurrent mutation, symlinks, non-regular files, and deterministic ordering | **serial** | — |
| `destination_permissions.rs` | `snp restore` applies the destination permission policy uniformly: new files `0o600` on Unix, existing modes preserved, sensitive kinds overridden | parallel | — |
| `edit_mutation_notify.rs` | `snp edit` byte-change-driven sync notification, per the outcome matrix (editor status × file changed) | parallel | — |
| `manifest_contracts.rs` | Restore manifest validation — 25 numbered rejection/success cases including unknown entry kind, schema version, traversal paths, Windows reserved names, UNC paths, duplicate destinations, and fixture integrity | parallel | — |
| `restore_security.rs` | Every rejection class of the backup-relative path validator rejects unsafe source artifacts | parallel | — |
| `restore_transactions.rs` | Restore transactionality: dry run produces zero writes, a successful restore records exactly one pending generation, a failed restore rolls back | parallel | — |

### Locking

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `mutual_exclusion.rs` | The auto-sync `execution_lock`: sequential acquisition, blocked second acquisition, stale lock with a dead PID is reclaimed, generation preservation across sequential mutations | parallel | — |
| `process_lock_concurrency.rs` | The kernel-backed process file lock: exactly one contender wins a simultaneous race, a killed owner releases through kernel teardown, repeated cycles leave only the canonical lock file | **serial** | `test-support` |
| `local_data_lock_barriers.rs` | `snp backup` sees a complete before-state or after-state while real writers are paused inside multi-file mutations at a barrier point | **serial** | `test-support` |

### PTY

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `pty_integration.rs` | The TUI cancellation contract over a real pty pair: `snp select` → exit 4, `snp run` → exit 0, `snp select --output-file` → exit 4 with no file written (see [PTY Harness](#pty-harness)) | **serial** | — |

### Recovery

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `repair_transactions.rs` | Exact transaction repair: `recover_transaction_by_id` operates on exactly one selected journal, `snp repair` collects and applies all typed actions, multiple-journal scenarios stay isolated | **serial** | `test-support` |
| `recovery_integration.rs` | `snp status` recovery reporting and startup-recovery behavior against a dead PID | parallel | — |
| `transaction_crash_recovery.rs` | Interrupted-transaction detection and repair across crash windows | **serial**, release-only | — |
| `cleanup_crash_failpoints.rs` | The typed cleanup state machine survives a process abort at every typed boundary and recovers to a clean final state | **unscoped** | — |
| `restore_crash_failpoints.rs` | The production transaction protocol: kill the real `snp restore` binary at production failpoints and verify exact recovery | **unscoped** | — |

### Architecture

| Target | Proves | Class | Feature |
|--------|--------|-------|---------|
| `architecture.rs` | The layered dependency rules in [`docs/LOGICAL_LAYERS.md`](../docs/LOGICAL_LAYERS.md) are enforced at the source level, plus the updater transport split (see [Architecture Boundary Tests](#architecture-boundary-tests)) | parallel | — |

### Target counts by concern

| Concern | Targets |
|---------|---------|
| CLI surface and output contracts | 10 |
| Selector, search, identity, MCP | 9 |
| Sync, server, protocol | 5 |
| Auto-sync | 9 |
| Persistence, backup, restore | 8 |
| Locking | 3 |
| PTY | 1 |
| Recovery | 5 |
| Architecture | 1 |
| **Total** | **51** |

The authoritative count is `ls tests/*.rs | wc -l` = **51**; `tests/support/`
and `tests/bin/` are not targets.

---

## Shared Support Modules

`tests/support/` holds four modules. 39 of the 51 targets opt in with
`mod support;` at the top of the file; the remaining 12 drive the binary
directly (they need no shared helpers):

| Target | Why it does not use `mod support;` |
|--------|------------------------------------|
| `architecture.rs` | Scans `src/` as text — no process or server needed |
| `identity_contract.rs` | In-process identity assertions over `TempDir` library files, with the config dir redirected per thread (see [Gotchas](#gotchas)) |
| `public_api_smoke.rs` | Uses the `snip_it` library directly, not the binary |
| `persistence_unit.rs` | Exercises the atomic-write primitive in-process |
| `pty_integration.rs` | Allocates its own pty pair and config dir |
| `snip_sync_lifetime.rs` | Launches the real `snip-sync serve` binary |
| `sync_integration.rs`, `sync_multibatch.rs` | Build a `SnipSyncService` directly |
| `mcp_integration.rs` | Drives the MCP server over its own stdio pipe |
| `integration.rs`, `output_contracts.rs`, `recovery_integration.rs` | Predate the shared helper module; each carries its own local `snp_cmd` / `setup_test_env` |

### `tests/support/mod.rs`

Declares the sub-modules and a `helpers` namespace of free functions that
predate the builder pattern. The helpers remain the most widely used entry
point:

```rust
// tests/support/mod.rs:12-38
pub fn snp_cmd() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snp"));
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    // Clear test-control variables by default so child processes don't
    // inherit them from the test runner's environment.
    cmd.env_remove("SNP_TEST_FAILPOINT");
    cmd.env_remove("SNP_TEST_EXECUTOR_MODE");
    cmd.env_remove("SNP_SKIP_WORKER_SPAWN");
    cmd.env_remove("SNP_TEST_EVENTS_DIR");
    cmd.env_remove("SNP_TEST_INJECT_ERROR");
    cmd.env_remove("SNP_TEST_MUTATION_BARRIER_DIR");
    cmd
}

pub fn setup_test_env() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let config_dir = tmp.path().join(".config").join("snp");
    fs::create_dir_all(&config_dir).unwrap();
    (tmp, config_dir)
}

pub fn snp_in(config_dir: &Path) -> Command {
    let mut cmd = snp_cmd();
    cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
    cmd
}
```

`helpers` also provides `output_with_stdin`, `create_sort_test_library`,
`write_sync_toml_auto_sync_enabled`, `write_sync_toml_auto_sync_ignore`,
`create_test_library_for_auto_sync`, and `golden_corpus`
(`tests/support/mod.rs:41-157`).

The `env_remove` block is deliberate: a test that needs a seam re-adds only
that one variable, so a stray value in the developer's shell cannot change a
test's behavior.

### `TestEnvironment`

`tests/support/environment.rs`. A builder-pattern environment that owns a
`TempDir` and a complete `~/.config/snp/` tree.

```rust
// tests/support/environment.rs:19-31
pub struct TestEnvironment {
    pub tmp: TempDir,
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
    pub home_dir: PathBuf,
    pub api_key: String,
    pub device_id: String,
    pub server_url: Option<String>,
    debounce: u64,
    failure_mode: String,
    child_pids: RefCell<Vec<u32>>,
    credential_file: Option<PathBuf>,
}
```

| Method | Purpose |
|--------|---------|
| `TestEnvironment::builder()` | Entry point; defaults `debounce = 0`, `failure_mode = "warn"` (`environment.rs:35-41`) |
| `.with_server_url(url)` | Server URL written into `sync.toml` |
| `.with_debounce(seconds)` | `auto_sync_debounce_seconds` in `sync.toml` |
| `.with_failure_mode(mode)` | `auto_sync_failure` in `sync.toml` |
| `.build()` | Creates the `TempDir`, `.config/snp/`, a unique `device_id` (`test-device-<uuid>`), a fixed `api_key` (`test-api-key-e2e-05a`), a credential file, and writes `sync.toml` (`environment.rs:249-279`) |
| `snp_cmd()` / `snp_args(&[&str])` | A `Command` for the real `snp` binary, sandboxed (below) |
| `snp_output(&[&str])` | Run to completion with null stdin, piped stdout/stderr |
| `snp_output_with_stdin(&[&str], &[u8])` | Run with bytes on stdin |
| `spawn_snp_detached(&[&str])` | Spawn a background `snp` and track its PID for cleanup |
| `write_sync_toml()` | (Re)write `sync.toml` with the environment's server URL, key, device, debounce, and failure mode |
| `create_library(name)` | `snp library create` + `snp library set-primary` |
| `new_snippet(desc)` | `snp new --command-stdin --description <desc>` with `echo <desc>` on stdin |
| `read_pending_generation()` | Parse `generation` out of `auto-sync-pending.toml` |
| `pending_marker_path()` / `status_file_path()` | Direct paths to `auto-sync-pending.toml` / `auto-sync-status.toml` |
| `read_status_file()` / `has_pending_marker()` | File-truth probes |
| `event_sink_path()` | `<state_dir>/test-events.jsonl` |

Every spawned command is sandboxed in `base_snp_cmd` (`environment.rs:43-66`):

```rust
// tests/support/environment.rs:43-58 (abridged)
fn base_snp_cmd(&self) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_snp"));
    cmd.env("XDG_CONFIG_HOME", self.home_dir.join(".config").to_str().unwrap());
    cmd.env("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    cmd.env_remove("SNP_TEST_FAILPOINT");
    cmd.env_remove("SNP_TEST_EXECUTOR_MODE");
    cmd.env_remove("SNP_SKIP_WORKER_SPAWN");
    cmd.env_remove("SNP_TEST_EVENTS_DIR");
    cmd.env_remove("SNP_TEST_INJECT_ERROR");
    cmd.env_remove("SNP_TEST_MUTATION_BARRIER_DIR");
    ...
}
```

`Drop` SIGTERMs any tracked child that is still alive
(`environment.rs:185-230`).

### `RecordingServer`

`tests/support/recording_server.rs`. A wrapper around the `snip-sync` crate's
`start_test_server` that adds event recording, deterministic wait helpers, and
failure-mode injection.

```rust
// tests/support/recording_server.rs:127-148
pub async fn start() -> Self {
    let observer = snip_sync::test_observer::InMemoryObserver::shared();
    let service = build_test_service_with_observer(Some(
        observer.clone() as Arc<dyn snip_sync::test_observer::TestRequestObserver>
    ))
    .await;
    let db = service.db.clone();
    let captured_auth_header = service.captured_auth_header.clone();
    let (addr, server_task, _captured) = start_test_server(service).await;
    ...
}
```

The underlying server binds to **loopback only on port 0**
(`snip-sync/src/test_helpers.rs:86-89`):

```rust
// snip-sync/src/test_helpers.rs:86-89
// Bind to loopback only -- tests must never expose a server on all interfaces.
let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
let addr = listener.local_addr().unwrap();
```

The service itself is pre-configured with `sqlite::memory:` and a 5-connection
pool (`snip-sync/src/test_helpers.rs:36-38`), so no database file is ever
created.

| Method | Purpose |
|--------|---------|
| `start()` | Bind and start the in-process server |
| `url()` / `addr()` | `http://127.0.0.1:<random>` and the raw `SocketAddr` |
| `db()` | The server's `Arc<Database>` for state inspection |
| `server_task()` | The `JoinHandle` (for `abort`) |
| `observer()` | The `InMemoryObserver` for telemetry assertions |
| `max_concurrent_requests()` | Peak in-flight requests seen by the observer |
| `events()` / `event_count(op)` / `has_operation(op)` | Recorded-event probes |
| `captured_auth_header()` | The first request's `authorization` metadata |
| `set_failure_mode(mode)` / `failure_mode()` | Inject `reject_auth`, `hang`, `reject_after`, `shutdown` |
| `total_request_count()` | Total requests recorded |
| `register_client()` | Registers a client and returns `(api_key, device_id)` |
| `build_client(&api_key)` | A configured `SyncClient` |
| `wait_for_auth(timeout)` | Block until a client connects |
| `wait_for_operation(op, timeout)` | Block until a named operation appears |
| `wait_for_request_count(op, n, timeout)` | Block until the operation count reaches `n` |
| `summary()` | A `RecordingSummary` with `count_for`, `success_count`, `failure_count` |
| `assert_exact_request_count(op, n)` and friends | Assertion wrappers over the above |
| `shutdown()` | Stop the server |

### `EventSink` / `EventWriter`

`tests/support/event_sink.rs`. A JSON-lines channel for detached-helper
lifecycle evidence. The **test side** reads (`EventSink`); the **child side**
writes (`EventWriter`).

> **Usage note.** No target in the current suite imports `support::event_sink`
> — the live lifecycle-event channel is the `SNP_TEST_EVENTS_DIR` sink in
> `src/auto_sync/test_events.rs`, which writes the same
> `test-events.jsonl` format. `EventSink` / `EventWriter` are the
> test-side reader/writer pair for that file and are available for new
> tests; do not go looking for an existing `EventSink` assertion in the
> suite and expect to find one.

```rust
// tests/support/event_sink.rs:31-48
pub struct EventRecord {
    pub schema: u32,
    pub seq: u64,
    pub component: String,
    pub event: String,
    pub pid: u32,
    pub generation: Option<u64>,
    pub at_unix_ms: u64,
    pub detail: Option<String>,
}
```

| Method | Purpose |
|--------|---------|
| `event_sink_path(state_dir)` | `<state_dir>/test-events.jsonl` (`event_sink.rs:55-57`) |
| `EventSink::new(state_dir)` | Test-side reader handle |
| `read_all()` | Every event recorded so far |
| `wait_for_event(component, event, timeout)` | Block until a matching event appears, or `None` on timeout |
| `wait_for_generation(component, event, gen, timeout)` | Same, plus a generation match |
| `count_events(component, event)` | Exact count |
| `clear()` | Truncate the file |
| `EventWriter::new(state_dir)` / `write(...)` | Child-side appender |

`EventWriter` opens the file in **append** mode per write, so several child
processes can emit concurrently without corrupting each other's lines.

Emission is compiled in only under the `test-support` feature
(`src/auto_sync/test_events.rs:15-24`) and fires **only when
`SNP_TEST_EVENTS_DIR` is set**, writing `<dir>/test-events.jsonl`
(`src/auto_sync/test_events.rs:44-47`). Production builds carry `#[inline(always)]`
no-op stubs (`src/auto_sync/test_events.rs:36-40`, `:50-54`, `:100-109`).

---

## Isolation Model

The isolation story has **three layers**: process-level (subprocess tests),
thread-level (in-process unit tests), and credential/network level (shared
by both).

### Process-Level: `TempDir` + `XDG_CONFIG_HOME`

Every integration test that shells out to `snp` redirects the config
directory. `setup_test_env()` returns a `TempDir` and the
`<tmp>/.config/snp` path; `snp_in(config_dir)` sets `XDG_CONFIG_HOME` to the
config directory's parent, so the binary resolves
`$XDG_CONFIG_HOME/snp` (`src/utils/config.rs:63-82`).

`TestEnvironment` does the same with `home_dir` as the `TempDir` root
(`environment.rs:44-48`).

**Rule:** a subprocess test must not write the real `~/.config/snp/`. There is
no per-platform `#[cfg]` branch in config resolution — macOS and Windows use
the same `~/.config` layout (`src/utils/config.rs:1-7`).

A target that calls the `snip_it` **library** (rather than the binary) is
not covered by `XDG_CONFIG_HOME` unless it sets it itself — see the
`identity_contract.rs` gap in [Gotchas](#gotchas).

### Thread-Level: `ScopedConfigDir` (the one a contributor must know)

This is the single most important isolation fact in the suite.

Unit tests run **in the same process** as each other, in parallel threads.
`save_library` derives its transaction-gate and local-data-lock paths from
the **config directory** (`src/library/persistence.rs:223-224`):

```rust
// src/library/persistence.rs:223-224
let sync_state_dir = crate::config::derive_sync_state_dir();
let transaction_dir = crate::local_data::transaction_dir();
crate::transaction::gate_mutation_on_interrupted_transactions(
    &sync_state_dir,
    &transaction_dir,
)?;
```

Before `ScopedConfigDir` existed, unit tests therefore wrote to the
developer's **real** `~/.config/snp/`: a real `audit.log`, a real
`usage.toml`, and the real `.transaction/` directory — and every parallel
test contended on one process-wide local-data lock file. That contention is
what made `test_serialization_matrix_makefile_leading_tabs_with_trailing_newline`
fail on a 30 s local-data-lock timeout.

The fix is a `#[cfg(test)]`-only, **thread-local** config-directory override:

```rust
// src/utils/config.rs:15-22
#[cfg(test)]
thread_local! {
    /// Per-thread config-directory override. Tests run in parallel threads
    /// inside one process, so an `XDG_CONFIG_HOME` mutation would race with
    /// every other test that resolves a path. A thread-local override keeps
    /// each test hermetic without touching process-global environment state.
    static CONFIG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

// src/utils/config.rs:35-57
#[cfg(test)]
#[derive(Debug)]
pub struct ScopedConfigDir {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl ScopedConfigDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let previous = CONFIG_DIR_OVERRIDE.with(|slot| slot.replace(Some(dir)));
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for ScopedConfigDir {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CONFIG_DIR_OVERRIDE.with(|slot| *slot.borrow_mut() = previous);
    }
}
```

`get_config_dir()` consults the override first (`src/utils/config.rs:64-67`).
`Drop` restores the previous value — including on assertion panic — so a
failing test cannot leak the redirect into the tests running beside it.
Nested overrides unwind LIFO.

**Usage pattern** (from `src/commands/run_cmd.rs:434-440`):

```rust
/// `run_exact` writes an audit-log entry and a usage counter, both of
/// which resolve their paths from the config directory. Point this thread
/// at a temp dir so the tests never mutate the developer's real
/// `~/.config/snp/` (and never contend on its local-data lock).
fn isolated_config_dir() -> (tempfile::TempDir, crate::utils::config::ScopedConfigDir) {
    let tmp = tempfile::TempDir::new().expect("test temp dir");
    let config_home = tmp.path().join(".config");
    std::fs::create_dir_all(&config_home).expect("test config home");
    let guard = crate::utils::config::ScopedConfigDir::new(config_home.join("snp"));
    (tmp, guard)
}
```

The same pattern appears in `src/logging.rs:601-607` for the audit-log test.

The canonical example is the `TestDir` helper in the library unit tests —
this is the helper that fixed the failing serialization-matrix test:

```rust
// src/library/tests.rs:9-37 (abridged)
/// Private temp directory that also points this thread's resolved config
/// directory at `<tmp>/.config/snp`.
///
/// `save_library` derives its gate and local-data-lock paths from the config
/// directory rather than from the target library path, so a bare `TempDir`
/// still let unit tests touch the developer's real `~/.config/snp/` and
/// contend on one process-wide lock file. Binding the override to the temp
/// dir's lifetime makes every test hermetic and independent.
struct TestDir {
    tmp: TempDir,
    _config_dir: crate::utils::config::ScopedConfigDir,
}
```

Note that the override is bound to the `TempDir`'s **lifetime**: the guard
field lives in the same struct as the temp dir, so the redirect is dropped
when the temp dir goes away.

**Caveat:** a thread-local override does **not** propagate to helper threads
spawned by the test. A test that spawns a worker thread and expects the
redirected config directory must set `XDG_CONFIG_HOME` instead. See
[config.md](config.md#test-only-config-directory-override).

### Credentials

`SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on **every** spawned test command
(`tests/support/mod.rs:14`, `tests/support/environment.rs:49`). It is the
seam that lets a test write a plaintext API key into `sync.toml` instead of
going through the OS keychain. **Never remove it** — the keychain is a real
system store and tests must not touch it.

`TestEnvironment` additionally writes a credential file and points
`SNP_TEST_CREDENTIAL_FILE` at it (`environment.rs:59-64`, `:258-260`) so the
parent *and* the detached worker resolve the same real key rather than
`@keychain`.

### Ports and Network

- In-process `snip-sync` servers bind `127.0.0.1:0` — loopback only, OS-assigned
  port (`snip-sync/src/test_helpers.rs:87`).
- `tests/snip_sync_lifetime.rs` launches the real `snip-sync serve` binary and
  reserves ports explicitly (`reserve_port`, `start_server_on_ports`).
- Negative tests point at `127.0.0.1:1` (unreachable) rather than a
  non-routable address, so the failure is fast and local.

### Edition 2024: `set_var` is `unsafe`

The workspace is edition 2024, where `std::env::set_var` is `unsafe`. Barrier
tests wrap the call explicitly:

```rust
// SAFETY: this is the only thread calling set_var, ...
unsafe {
    std::env::set_var("SNP_TEST_MUTATION_BARRIER_DIR", barrier_dir2);
}
```

(`tests/repair_transactions.rs:1170-1175`)

### Isolation summary

| Concern | Guarantee |
|---------|-----------|
| Config directory (subprocess) | `TempDir` + `XDG_CONFIG_HOME` override |
| Config directory (in-process unit) | `ScopedConfigDir` thread-local override, `#[cfg(test)]` only |
| Keychain | `SNP_ALLOW_PLAINTEXT_API_KEY=true` + `SNP_TEST_CREDENTIAL_FILE`; no keyring access |
| Ports | Server binds `127.0.0.1:0` — OS-assigned, loopback only |
| Process cleanup | `Drop` for `TestEnvironment` SIGTERMs tracked child PIDs |
| State files | All inside the `TempDir`: `auto-sync-pending.toml`, `auto-sync-status.toml`, `test-events.jsonl` |
| Device identity | Unique `test-device-<uuid>` per environment, never reused |
| Server database | `sqlite::memory:` — no database file |
| Event emission | JSON-lines only when `SNP_TEST_EVENTS_DIR` is set |

Tests never touch the developer's real `~/.config/snp/`, real keychain, or
real network ports.

---

## Execution Classes

### Parallel-safe

`cargo test --workspace --lib` is parallel-safe: each unit test owns an
isolated `TempDir` and, where it touches the config directory, a
`ScopedConfigDir` bound to its own thread.

Integration targets are parallel-safe by the same reasoning — each test
constructs its own `TempDir` via `setup_test_env()` or `TestEnvironment`.

### Serial targets and why

| Target | Why it must be serial |
|--------|-----------------------|
| `pty_integration` | Allocates real pty pairs and drives an interactive TUI; concurrent pty sessions contend for terminal resources and the harness's own output drains |
| `auto_sync_concurrency` | Cross-process timing: real detached workers, pending markers, and lock handoff across processes |
| `backup_snapshot_concurrency` | Real concurrent writers paused mid-mutation while a backup process takes the `LocalDataLock` |
| `process_lock_concurrency` | Kernel `flock` (Unix) / `LockFileEx` (Windows) contention across real subprocesses |
| `local_data_lock_barriers` | Barrier protocol: the writer blocks on `SNP_TEST_MUTATION_BARRIER_DIR` while the backup process contends for the lock |
| `repair_transactions` | Barrier protocol plus `unsafe set_var` on process-global environment state |
| `sync_multibatch` | Multi-batch upload ordering against one server |
| `snip_sync_lifetime` | Binds real TCP ports and asserts HTTP wire bytes |
| `transaction_crash_recovery` | Release-profile crash windows; each test kills a process at a boundary |
| `cleanup_crash_failpoints` | Aborts the real `snp restore` binary at each typed cleanup boundary (`tests/cleanup_crash_failpoints.rs:86-100`) |
| `restore_crash_failpoints` | Aborts the real `snp restore` binary at typed boundaries |

### Assertion Discipline

These rules apply to every target in the suite, not just the auto-sync ones.

- **Exact counts, not `>= 1`**: `assert_eq!(count, 1, ...)` on sync attempts,
  worker spawns, and server operations.
- **Server-side effects proven**: remote state (the snippet appears on pull,
  the request was recorded) is asserted, not just the client's exit code.
- **Pending-clear ordering**: the remote effect occurs *before* the pending
  marker is cleared; a pending clear is impossible when the canonical sync
  fails; status-file truth is necessary but not sufficient on its own.
- **Filesystem snapshots**: a read-only path is proven by comparing a
  recursive byte snapshot of the config directory before and after
  (`tests/readonly_library_resolution.rs:17-35`).

### Command Matrix

| Command | Feature | Threads |
|---------|---------|---------|
| `cargo test --workspace --lib` | — | parallel (default) |
| `cargo test --test platform_smoke --features test-support` | `test-support` | parallel |
| `cargo test --test destination_permissions --features test-support` | `test-support` | parallel |
| `cargo test --test auto_sync_closure --features test-support` | `test-support` | parallel |
| `cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1` | `test-support` | **serial** |
| `cargo test --test sync_multibatch --features test-support -- --test-threads=1` | `test-support` | **serial** |
| `cargo test --test snip_sync_lifetime -- --test-threads=1` | — | **serial** |
| `cargo test --test <name> --features test-support` for any other target | `test-support` | parallel unless listed above |
| `cargo test -p snip-sync --features test-helpers` | `test-helpers` | parallel |
| `cargo test --workspace --all-features -- --test-threads=1` | all | **serial** (full suite) |
| `cargo test --release --test transaction_crash_recovery --features test-support -- --test-threads=1` | `test-support` | **serial**, release profile |

---

## Feature Gating

### `test-support` (the `snip-it` crate)

`Cargo.toml:161-162` defines `test-support = []` — a no-op feature whose only
purpose is to switch `#[cfg(feature = "test-support")]` blocks on and off.

Under `test-support`:

- `src/test_failpoints.rs` compiles its real failpoint, error-injection, and
  barrier logic (`src/test_failpoints.rs:20-26`, `:38-48`, `:63-80`).
- `src/auto_sync/test_events.rs` compiles real event emission
  (`src/auto_sync/test_events.rs:15-33`).
- `src/lib.rs:65-69` re-exports the `transaction` module publicly so
  integration tests can call the recovery API:

```rust
// src/lib.rs:65-69
#[cfg(not(feature = "test-support"))]
pub(crate) mod transaction;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod transaction;
```

Without `test-support`, every one of those sites is a compile-time no-op —
the environment variable checks are **absent from the binary**, not merely
inert. See [Production Seam Proof](#production-seam-proof).

### `required-features` in `Cargo.toml`

```toml
# Cargo.toml:51-75
# Cross-process lock test helper binary. Only enabled under `test-support`
# so production builds do not include this code path.
[[bin]]
name = "process_lock_helper"
path = "tests/bin/process_lock_helper.rs"
required-features = ["test-support"]

# The repair_transactions integration test exercises the public transaction
# recovery API, which is only re-exported under the `test-support` feature.
# Gate the target so that `cargo test --workspace --all-targets` (without
# `--all-features`) can compile cleanly.
[[test]]
name = "repair_transactions"
path = "tests/repair_transactions.rs"
required-features = ["test-support"]

[[test]]
name = "process_lock_concurrency"
path = "tests/process_lock_concurrency.rs"
required-features = ["test-support"]

[[test]]
name = "local_data_lock_barriers"
path = "tests/local_data_lock_barriers.rs"
required-features = ["test-support"]
```

Exactly **four** entries carry `required-features`: the three targets above
plus the `process_lock_helper` binary. The remaining 48 targets compile with
default features.

> **Note.** `cargo clippy --workspace --all-targets -- -D warnings` (the
> `check.sh` step) does **not** pass `--all-features`. The gated targets are
> therefore *not* linted by that command; they are linted only when a target
> is built with `--features test-support`. `check.sh` covers
> `platform_smoke`, `destination_permissions`, and `auto_sync_closure` that
> way — but `repair_transactions`, `process_lock_concurrency`, and
> `local_data_lock_barriers` are **never** linted by `check.sh`. Run
> `cargo clippy --workspace --all-targets --all-features -- -D warnings`
> locally before landing changes to them.

### `process_lock_helper`

`tests/bin/process_lock_helper.rs` is a small binary that participates in the
kernel-lock race. `process_lock_concurrency.rs` spawns it via
`env!("CARGO_BIN_EXE_process_lock_helper")` and coordinates through
`SNP_TEST_LOCK_PATH` and `SNP_TEST_LOCK_BARRIER_DIR`
(`tests/process_lock_concurrency.rs:28-40`).

**Container caveat** (from the target's own header): the tests use real
`flock` (Unix), which requires `SYS_ADMIN` capability or `--privileged` in
Docker. Without it `flock` silently degrades to `fcntl` and mutual exclusion
is not guaranteed. On Windows `flock` is emulated via `LockFileEx` and no
special container privileges are needed.

### `test-helpers` (the `snip-sync` crate)

`snip-sync/Cargo.toml:35` defines `test-helpers = []`. Under it (or under
`cfg(test)`), `snip-sync` exposes:

```rust
// snip-sync/src/lib.rs:28-31
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_helpers;
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_observer;
```

`test_helpers` provides `build_test_service()`,
`build_test_service_with_observer(Option<Arc<dyn TestRequestObserver>>)`, and
`start_test_server(service)` (`snip-sync/src/test_helpers.rs:24-110`).
`test_observer` provides the `TestRequestObserver` trait and
`InMemoryObserver`, which retain sanitized request telemetry only — no API
key, authorization header, raw body, or plaintext payload
(`snip-sync/src/test_observer.rs:1-22`).

The `snip-it` crate pulls `snip-sync` in as a dev-dependency with the feature
already on (`Cargo.toml:143`):

```toml
snip-sync = { version = "0.1.6", path = "snip-sync", features = ["test-helpers"] }
```

### Production Seam Proof

`scripts/ci/test-production-seams.sh` is the executable proof that the
feature gates work. It builds `snp` **without** `test-support` into an
isolated target directory (`test-production-seams.sh:18-27`):

```bash
TARGET_DIR="target/production-seam"
BINARY="$TARGET_DIR/release/snp"
cargo build --release --no-default-features --target-dir "$TARGET_DIR"
```

Then it runs five scenarios against that binary with the test-only variables
set to *valid* values, and asserts that no test behavior activates:

| # | Variable | Assertion |
|---|----------|-----------|
| 1 | `SNP_TEST_FAILPOINT=restore-after-prepared` | Production `restore` does not abort (`:81-128`) |
| 2 | `SNP_SKIP_WORKER_SPAWN=1` | Production `library create` still completes and writes the file (`:131-147`) |
| 3 | `SNP_ALLOW_DIR_FSYNC_FAILURE=1` | The binary does not even *contain* the string — `grep -a -q` on the binary must fail — and the mutation still persists (`:150-170`) |
| 4 | `SNP_TEST_EVENTS_DIR=<dir>` | No `test-events.jsonl` is created (`:173-185`) |
| 5 | `SNP_TEST_MUTATION_BARRIER_DIR=<dir>` | The mutation does not block, and no `entered` file appears (`:188-213`) |

Scenario 3 is the strongest: it proves the opt-out is *compiled out*, not
merely ignored.

This script runs in `release-check.sh verify` (`release-check.sh:166`), not in
Linux CI.

---

## PTY Harness

`tests/pty_integration.rs` (1693 lines) drives the real TUI through a real
pty pair and asserts the cancellation contract:

- `snp select` → exit 4 on primary-selector cancel
- `snp run` → exit 0 on cancel (normal completion)
- `snp select --output-file` → exit 4 **and** the output file is not created

### The Paint-and-Quiet Handshake

**Before the rewrite** the harness slept a fixed interval before writing
keystrokes. Under load the keys arrived before the TUI had installed its
input reader; the keys were dropped, the TUI waited forever, and the run
ended only at the fixed timeout. This was the single failure in the full
serial suite.

The harness now observes the child's output and waits for the child to
**paint and then go quiet**, or to exit on its own.

```rust
// tests/pty_integration.rs:34-46
type PtyOutput = std::sync::Arc<std::sync::Mutex<Vec<u8>>>;

/// Upper bound on how long the TUI has to produce its first screenful before
/// the test gives up. Generous on purpose: this replaces a fixed 2 s sleep,
/// which was the only thing standing between a merely slow start and a hang.
const PTY_READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound on how long the child may take to exit after the last key.
const PTY_EXIT_TIMEOUT: Duration = Duration::from_secs(30);
```

```rust
// tests/pty_integration.rs:60-92
fn wait_for_pty_ready(
    child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    output: &PtyOutput,
    settle: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + PTY_READY_TIMEOUT;
    let mut last_len = 0usize;
    let mut quiet_since: Option<std::time::Instant> = None;

    while std::time::Instant::now() < deadline {
        let len = output.lock().unwrap().len();
        if len != last_len {
            // Any new bytes mean the screen is still changing.
            last_len = len;
            quiet_since = Some(std::time::Instant::now());
        } else if let Some(since) = quiet_since
            && since.elapsed() >= settle
        {
            return true;
        }

        if len == 0 && matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }

        std::thread::sleep(Duration::from_millis(20));
    }

    // Never settled. Send the keys anyway rather than failing a test whose
    // subject is not startup timing.
    eprintln!("warning: snp PTY output did not settle within {PTY_READY_TIMEOUT:?}");
    true
}
```

The mechanics:

1. A **drain thread** reads the master pty fd with `libc::poll` +
   `libc::read` on a non-blocking fd and appends into the shared
   `PtyOutput` (`tests/pty_integration.rs:99-127`, spawned at `:206-217`).
2. `wait_for_pty_ready` polls the shared buffer length every 20 ms.
3. **New bytes** reset the quiet timer — the screen is still changing.
4. **No new bytes for `settle`** → the child has painted and settled → send
   the keys.
5. **The child exited with zero output** → short-circuit and send the keys.
6. **The deadline passed** → warn on stderr and send the keys anyway. A test
   whose subject is *not* startup timing must not fail because startup was
   slow.

The `settle` window is the `initial_delay` argument, which the old code used
as a sleep. `run_snp_pty` passes 2 s; `run_snp_pty_with_delay` lets a test
widen it (`tests/pty_integration.rs:172-183`).

### Commands That Render Nothing

Not every command paints. `snp new --command-stdin` reads stdin and exits
silently — and its "keys" *are* the stdin bytes. Waiting for a screen that
never appears would deadlock the test. Two mechanisms cover this:

- the **quiet timer** fires after `settle` of zero output, and
- an **already-exited child** short-circuits outright (`len == 0 && try_wait()`
  returned a status).

This is called out explicitly in the function's own doc comment
(`tests/pty_integration.rs:56-59`).

### Exit Timeout and Drain Lifetime

The exit timeout is the **named constant** `PTY_EXIT_TIMEOUT` (30 s), not a
literal. On expiry the harness kills the child, waits, joins the drain
thread, and panics with the captured output (`tests/pty_integration.rs:234-248`).

The drain thread's own deadline is `PTY_READY_TIMEOUT + PTY_EXIT_TIMEOUT`
(`tests/pty_integration.rs:101`) so the drain cannot give up before the
output being waited for has been produced.

---

## Failpoints and Error Injection

`src/test_failpoints.rs` is a **test-only** module (`src/lib.rs:64` —
`pub(crate) mod test_failpoints;`) with three seams, each of which is a
compile-time no-op without `test-support`.

### `maybe_failpoint(name)`

```rust
// src/test_failpoints.rs:20-26
#[cfg(feature = "test-support")]
pub fn maybe_failpoint(name: &str) {
    if std::env::var("SNP_TEST_FAILPOINT").as_deref().ok() == Some(name) {
        tracing::error!(failpoint = name, "test failpoint triggered: aborting");
        std::process::abort();
    }
}
```

Aborts the process immediately — a true crash, not a clean error. The
production stub is `pub fn maybe_failpoint(_name: &str) {}`
(`src/test_failpoints.rs:29-31`).

**Named failpoints** (`src/test_failpoints.rs:101-122`):

| Constant | Value |
|----------|-------|
| `RESTORE_AFTER_PREPARED` | `restore-after-prepared` |
| `RESTORE_AFTER_BACKUPS_DURABLE` | `restore-after-backups-durable` |
| `RESTORE_AFTER_FIRST_INSTALL` | `restore-after-first-install` |
| `RESTORE_AFTER_INDEX_INSTALL` | `restore-after-index-install` |
| `RESTORE_AFTER_ALL_INSTALLS` | `restore-after-all-installs` |
| `RESTORE_AFTER_COMMITTED_LOCAL_BEFORE_PENDING` | `restore-after-committed-local-before-pending` |
| `RESTORE_AFTER_PENDING_BEFORE_JOURNAL_UPDATE` | `restore-after-pending-before-journal-update` |
| `RESTORE_AFTER_PENDING_BEFORE_CLEANUP` | `restore-after-pending-before-cleanup` |
| `RESTORE_DURING_FIRST_ROLLBACK` | `restore-during-first-rollback` |
| `RESTORE_DURING_SECOND_ROLLBACK` | `restore-during-second-rollback` |
| `CLEANUP_AFTER_STATE_BEFORE_VALIDATION` | `cleanup-after-state-before-validation` |
| `CLEANUP_AFTER_VALIDATION_BEFORE_STAGED` | `cleanup-after-validation-before-staged` |
| `CLEANUP_AFTER_STAGED_BEFORE_BACKUPS` | `cleanup-after-staged-before-backups` |
| `CLEANUP_AFTER_BACKUPS_BEFORE_ARTIFACT_ROOT` | `cleanup-after-backups-before-artifact-root` |
| `CLEANUP_AFTER_ARTIFACT_ROOT_BEFORE_JOURNAL` | `cleanup-after-artifact-root-before-journal` |
| `CLEANUP_AFTER_JOURNAL_BEFORE_PARENT_SYNC` | `cleanup-after-journal-before-parent-sync` |

### `maybe_injected_error(name)`

```rust
// src/test_failpoints.rs:38-48
#[cfg(feature = "test-support")]
pub fn maybe_injected_error(name: &str) -> crate::error::SnipResult<()> {
    if std::env::var("SNP_TEST_INJECT_ERROR").as_deref().ok() == Some(name) {
        return Err(crate::error::SnipError::runtime_error(
            "Injected test failure",
            Some(name),
        ));
    }
    Ok(())
}
```

Returns a recoverable error rather than aborting — used to drive rollback
paths (`tests/repair_transactions.rs:812`,
`tests/restore_crash_failpoints.rs:523`).

### `mutation_barrier(point)`

```rust
// src/test_failpoints.rs:63-80 (abridged)
#[cfg(feature = "test-support")]
pub fn mutation_barrier(point: &str) {
    let Ok(root) = std::env::var("SNP_TEST_MUTATION_BARRIER_DIR") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let expected = root.join("point");
    if std::fs::read_to_string(&expected).ok().as_deref() != Some(point) {
        return;
    }
    let _ = std::fs::write(root.join("entered"), point).inspect_err(|e| { ... });
    // ... then block until <dir>/release appears
}
```

The barrier protocol, driven by the test:

1. The test writes the expected point name to `<dir>/point` and clears stale
   `entered` / `release` files
   (`tests/local_data_lock_barriers.rs:31-38`).
2. The real process reaches that point, writes `<dir>/entered`, and blocks.
3. The test observes `entered` and runs the contending operation.
4. The test writes `<dir>/release`; the blocked process resumes
   (`tests/local_data_lock_barriers.rs:40-43`).

### Who Uses Which Seam

| Seam | Consumers |
|------|-----------|
| `SNP_TEST_FAILPOINT` | `tests/restore_crash_failpoints.rs:174`, `tests/cleanup_crash_failpoints.rs:94`, `tests/destination_permissions.rs:283` |
| `SNP_TEST_INJECT_ERROR` | `tests/repair_transactions.rs:812`, `tests/restore_crash_failpoints.rs:523`, `:707` |
| `SNP_TEST_MUTATION_BARRIER_DIR` | `tests/local_data_lock_barriers.rs:211`, `tests/repair_transactions.rs:1173` |
| `SNP_SKIP_WORKER_SPAWN` | `tests/edit_mutation_notify.rs:50`, `:268`; production no-op at `src/auto_sync/schedule.rs:189-198` |
| `SNP_TEST_EVENTS_DIR` | `src/auto_sync/test_events.rs:32`, `:45` |

---

## Architecture Boundary Tests

`tests/architecture.rs` (256 lines) is a **source-scanning** test. It does not
run the code under test — it reads the source text and asserts on it. This is
what makes it cheap enough to run in every CI pass.

### Layer lists

```rust
// tests/architecture.rs:15-29
const CORE_MODULES: &[&str] = &[
    "library/",
    "sort.rs",
    "output.rs",
    "usage.rs",
    "diagnostics.rs",
];

const SYNC_CLIENT_MODULES: &[&str] = &["sync.rs", "sync_commands.rs", "encryption.rs"];
```

Forbidden imports:

```rust
// tests/architecture.rs:35-52
const FORBIDDEN_FROM_CORE: &[&str] = &[
    "crate::commands", "crate::ui", "crate::logging", "crate::auto_sync",
    "crate::clipboard", "crate::sync_commands", "crate::sync",
];

const FORBIDDEN_FROM_SYNC_CLIENT: &[&str] = &[
    "crate::commands", "crate::ui", "crate::logging", "crate::auto_sync",
    "crate::clipboard",
];
```

Two documented exceptions are baked into the lists rather than the scanner:
`crate::config` is allowed from core (pure TOML-cache helpers only), and
`config/` is deliberately *not* listed as a sync-client module because of a
pre-Plan-010 cross-layer call (`tests/architecture.rs:23-34`).

The scanner reads a module directory by concatenating every `*.rs` file in
sorted order (`tests/architecture.rs:59-79`) and skips comment lines
(`:85-90`), so it does not false-positive on documentation.

The third test pins module visibility: `clipboard`, `diagnostics`,
`encryption`, `library`, `output`, `status_snapshot`, `sync_commands`, and
`utils` must be `pub(crate) mod`, not `pub mod`
(`tests/architecture.rs:131-161`).

### Updater transport pin

```rust
// tests/architecture.rs:200-218
#[test]
fn snp_updater_pins_lean_eggfetch_profile() {
    ...
    assert!(
        source.contains("eggfetch-core = { version = \"=0.2.2\""),
        "snp updater must pin eggfetch-core =0.2.2"
    );
    assert!(source.contains("\"standard-http1\""), ...);
    assert!(source.contains("\"redirects\""), ...);
}
```

The pin is `=0.2.2`, matching `Cargo.toml:132`:

```toml
eggfetch-core = { version = "=0.2.2", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
```

Three further tests pin the split:

- `snp_updater_does_not_shell_out_to_curl` (`tests/architecture.rs:172-193`) —
  `src/update.rs` must not contain `"curl"` or `curl_protocol`. The
  `snip-sync/src/update.rs` curl adapter is intentionally *not* scanned: it
  is a measured footprint tradeoff (see [update.md](update.md)).
- `snp_updater_delegates_redirects_and_total_timeout`
  (`tests/architecture.rs:220-256`) — `RedirectPolicy::strict` must be
  present, and `tokio::time::timeout` plus the superseded manual redirect
  helpers (`fn safe_get`, `fn is_redirect_status`, `fn redirect_location`,
  `fn redirect_target`, `follow_redirects(false)`) must be absent.

The rule this encodes: **split transports by measurement, don't unify.**
`snp update` uses the in-process lean `eggfetch-core`; `snip-sync update`
keeps the external `curl` because embedding a trial bloated the small server
by ~34%, past the 10% gate.

### Layer doc coupling

The constant lists in `tests/architecture.rs` and the tables in
[`docs/LOGICAL_LAYERS.md`](../docs/LOGICAL_LAYERS.md) must be changed
**together**. Adding a module to a layer means editing both.

---

## HTTP Wire-Parity Suite

`tests/snip_sync_lifetime.rs` (840 lines) launches the real `snip-sync serve`
binary and asserts the HTTP wire bytes. It is the lock on the server's
two-route EggServe leaf — do not change status codes, `Allow`/`Vary`, or the
security headers without updating this target.

| Test | Line | Proves |
|------|------|--------|
| `server_remains_healthy_beyond_30_seconds` | `:215-217` | The server survives past the 30 s total-connection-lifetime boundary. **Marked `#[ignore = "runs for 35+ seconds; invoke explicitly with --ignored"]`** — invoked by `release-check.sh:163` |
| `http_health_head_metrics_disabled_and_fallback_contract` | `:251-252` | `HEAD /health`, the metrics-disabled path, and the fallback registry |
| `http_metrics_auth_and_cors_preflight_contract` | `:357-358` | Basic auth on `/metrics`, the CORS preflight, and the `WWW-Authenticate` challenge |
| `http_allow_all_cors_is_available_for_loopback_server` | `:441-442` | `Access-Control-Allow-Origin: *` for the loopback server |

### The `WWW-Authenticate` assertion

`/metrics` returned 401 without the RFC 9110 §11.1 challenge, which made a
password-protected endpoint unreachable through standard HTTP auth. The
header is now sent and asserted twice — for the missing-credentials case and
the malformed-credentials case:

```rust
// tests/snip_sync_lifetime.rs:370-377
// RFC 9110 §11.1: a 401 must carry a challenge, otherwise clients cannot
// learn the scheme and browsers will not prompt for credentials.
assert!(
    unauthorized
        .to_ascii_lowercase()
        .contains("www-authenticate: basic realm=\"metrics\""),
    "{unauthorized}"
);
```

The second occurrence is at `tests/snip_sync_lifetime.rs:381-388`, for the
`Authorization: Basic !!!` (invalid) response.

### Shared assertion helpers

The target provides reusable helpers that other work should reuse rather than
re-derive: `response_headers`, `header_value`, `header_tokens`,
`response_body`, `assert_security_headers_present`,
`assert_no_security_headers`, `assert_vary_origin`, `assert_no_vary`, and
`assert_empty_fallback` (`tests/snip_sync_lifetime.rs:485-590`).

---

## Deep Recovery Suites

Three targets dig deeper than anything CI runs.

| Target | Lines | Scope | Invoked by |
|--------|-------|-------|------------|
| `transaction_crash_recovery.rs` | 753 | Interrupted-transaction detection and repair across crash windows | `scripts/release-check.sh verify` only (`release-check.sh:156-157`) |
| `cleanup_crash_failpoints.rs` | 402 | The typed cleanup state machine survives an abort at every typed boundary | **No script** |
| `restore_crash_failpoints.rs` | 876 | The production transaction protocol, verified by killing the real `snp restore` binary at production failpoints | **No script** |

**`transaction_crash_recovery.rs`** is the only one wired into a script. It
runs in the **release profile** — the release binary's optimizer and
`panic = "abort"` profile (`Cargo.toml:150-155`) can expose timing and
teardown behavior that a debug build does not.

**`cleanup_crash_failpoints.rs` and `restore_crash_failpoints.rs` are
invoked by no script at all** — not by `check.sh`, not by `release-check.sh`,
not by CI. They must be run explicitly:

```bash
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
```

**Do not assume CI covers them.** A change to the restore or cleanup state
machine needs one of these run locally before merge.

---

## Verification Scripts

### `scripts/check.sh`

The authoritative Linux-CI check list (`scripts/check.sh:15-46`):

| Step | Command |
|------|---------|
| Installer contract | `bash scripts/tests/installers.sh` |
| Format check | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --workspace --all-targets -- -D warnings` |
| Unit tests | `cargo test --workspace --lib` |
| Platform smoke | `cargo test --test platform_smoke --features test-support` |
| Destination permissions | `cargo test --test destination_permissions --features test-support` |
| Single-helper auto-sync contracts | `cargo test --test auto_sync_closure --features test-support` |
| Auto-sync concurrency | `cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1` |
| Multi-batch sync contracts | `cargo test --test sync_multibatch --features test-support -- --test-threads=1` |
| snip-sync HTTP socket contracts | `cargo build -p snip-sync --bin snip-sync` then `cargo test --test snip_sync_lifetime -- --test-threads=1` |

Six of the 51 targets run in CI. The other 45 run only under the
full-suite command, the release script, or by hand.

### `scripts/release-check.sh verify`

Requires a clean tree (`release-check.sh:130-134`). Phases:

1. Release workflow contract
2. `scripts/check.sh` (same as Linux CI)
3. Release build (`cargo build --workspace --release`)
4. Release smoke — `--version`, `--help`, then:
   - `cargo test --release --test transaction_crash_recovery --features test-support -- --test-threads=1`
   - `cargo test --release --test sync_multibatch -- --test-threads=1`
   - `cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1`
   - `bash scripts/ci/test-production-seams.sh`
   - `cargo test --test manifest_contracts --features test-support`
5. Package validation (`cargo package -p snip-proto|snip-sync|snip-it --locked`)

### `scripts/ci/test-production-seams.sh`

See [Production Seam Proof](#production-seam-proof).

### CI matrix

`.github/workflows/ci.yml`:

- **linux-correctness** — `bash scripts/check.sh` (35 min timeout).
- **platform-smoke** — macOS and Windows: `cargo check --workspace --all-targets`
  plus `cargo test --test platform_smoke --features test-support`. Windows
  additionally runs `pwsh -NoProfile -File scripts/tests/installers.ps1`.

---

## Key Files

| Path | Role |
|------|------|
| `tests/support/mod.rs` | Shared `helpers` namespace: `snp_cmd`, `setup_test_env`, `snp_in`, `output_with_stdin`, `golden_corpus` |
| `tests/support/environment.rs` | `TestEnvironment` builder — isolated `TempDir`, `XDG_CONFIG_HOME`, sandboxed `snp` commands, child cleanup |
| `tests/support/recording_server.rs` | `RecordingServer` — in-process `snip-sync` on a random loopback port with event recording and failure injection |
| `tests/support/event_sink.rs` | `EventSink` / `EventWriter` — JSON-lines lifecycle evidence channel (available; not imported by any current target) |
| `tests/bin/process_lock_helper.rs` | Helper binary that participates in the kernel-lock race (`required-features = ["test-support"]`) |
| `src/test_failpoints.rs` | Feature-gated failpoint, error-injection, and barrier seams |
| `src/auto_sync/test_events.rs` | Feature-gated JSON-lines event emission (`SNP_TEST_EVENTS_DIR`) |
| `src/utils/config.rs` | `get_config_dir()` and the `#[cfg(test)]` thread-local `ScopedConfigDir` override |
| `tests/architecture.rs` | Source-scanning layer-boundary enforcement and the updater transport pin |
| `tests/pty_integration.rs` | Real-pty TUI harness with the paint-and-quiet handshake |
| `tests/snip_sync_lifetime.rs` | Locked HTTP wire-parity assertions for the `snip-sync` leaf |
| `tests/readonly_library_resolution.rs` | Read-only resolution regression: legacy layout, `all` scope, no writes |
| `tests/public_api_smoke.rs` | The supported `snip_it` public surface still resolves |
| `Cargo.toml` | `required-features` gating for three targets and the helper bin; `test-support` feature |
| `snip-sync/Cargo.toml` | `test-helpers` feature for the in-process server helpers |
| `scripts/check.sh` | The Linux-CI check list |
| `scripts/release-check.sh` | The manual pre-release verification, including the deep suites |
| `scripts/ci/test-production-seams.sh` | Proof that test-only env vars are inert in production builds |

---

## Invariants

1. **Tests must not touch the developer's real `~/.config/snp/`, real
   keychain, or real network ports.** Subprocess tests use `TempDir` +
   `XDG_CONFIG_HOME`; unit tests use the `#[cfg(test)]` thread-local
   `ScopedConfigDir`. One target does not yet comply — see
   [Gotchas](#gotchas).
2. **`SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on every test command.** It is
   the seam that keeps the OS keychain out of the test path. Never remove it.
3. **Test-only code is compiled out of production builds.** `test-support`
   gates the failpoints, the event sink, and the public `transaction`
   re-export. `scripts/ci/test-production-seams.sh` proves the env-var
   checks are *absent from the binary*, not merely inert.
4. **`cargo test --workspace --lib` is parallel-safe.** Every unit test owns
   an isolated `TempDir` and, where it needs one, a thread-local config
   override.
5. **Serial targets are serial for a reason** — real pty pairs, kernel lock
   contention, barrier coordination, or shared ports. Do not parallelize them.
6. **Exactly four Cargo entries carry `required-features`**: the three
   `test-support` targets and the `process_lock_helper` bin.
7. **The exit timeout is a named constant.** `PTY_EXIT_TIMEOUT`, not a
   literal.
8. **Assertions are exact counts, not `>= 1`**, and server-side effects are
   proven, not inferred from the client's exit code.
9. **A read-only path is proven by a filesystem snapshot**, not by the
   absence of an error. Compare a recursive byte snapshot of the config
   directory before and after the call
   (`tests/readonly_library_resolution.rs:17-35`).
10. **Two deep suites run in no script.** `cleanup_crash_failpoints.rs` and
    `restore_crash_failpoints.rs` must be run by hand before merging a change
    to the restore or cleanup state machine.

---

## Gotchas

- **`ScopedConfigDir` is unreachable from integration tests.** It is
  `#[cfg(test)]` on the struct *and* both impls (`src/utils/config.rs:35`,
  `:41`, `:51`) *and* lives in a `pub(crate)` module
  (`src/lib.rs:70`). Only the crate's own unit tests can bind it; an
  integration test must redirect the config directory with
  `XDG_CONFIG_HOME` instead.
- **A thread-local override does not propagate to spawned threads.** Use
  `XDG_CONFIG_HOME` for a test that spawns a worker thread.
- **`set_var` is `unsafe` in edition 2024.** Barrier tests wrap it
  (`tests/repair_transactions.rs:1170-1175`).
- **`cargo clippy --workspace --all-targets` does not lint the three
  `required-features` targets.** `check.sh` does not pass `--all-features`;
  lint them explicitly before landing changes to
  `repair_transactions`, `process_lock_concurrency`, or
  `local_data_lock_barriers`.
- **Do not delete the `env_remove` block** in `snp_cmd()` /
  `base_snp_cmd()`. Without it a stray variable in the developer's shell
  silently changes test behavior.
- **`process_lock_concurrency.rs` needs `SYS_ADMIN` / `--privileged` in
  Docker.** Without it `flock` degrades to `fcntl` and mutual exclusion is
  not actually proven.
- **`snip_sync_lifetime.rs` has one `#[ignore]`d test** that takes 35+ s. It
  runs only via `release-check.sh` with `--ignored`.
- **`snip-sync` has its own integration target**
  (`snip-sync/tests/legacy_pid_cli.rs`), Linux-only
  (`#![cfg(target_os = "linux")]`). It is not one of the 51.
- **The `snip_it` library is `#[doc(hidden)]` in places.** Integration tests
  use it directly, but `public_api_smoke.rs` is the guard on the *supported*
  surface listed in `src/lib.rs`.
- **`tests/identity_contract.rs` now isolates the config directory.** *(Fixed
  in this session.)* It calls `save_library` in-process, which runs the
  mutation gate and acquires the local-data lock — both derived from the
  **config directory** (`src/library/persistence.rs:223-224` →
  `src/utils/config.rs:227-232` → `src/local_data.rs:398-400`). It
  previously set neither `XDG_CONFIG_HOME` nor a `ScopedConfigDir`, so the
  gate and the lock resolved against the developer's real `~/.config/snp/`:
  the library file was written inside a `TempDir`, but the gate could refuse
  the mutation (or roll back an interrupted transaction) depending on the
  developer's real state, and a `.transaction` directory was created in the
  real config. The target now uses a `TestDir` helper that binds
  `ScopedConfigDir` for the calling thread, and the target is gated on
  `test-support` because that override is exported from the crate root only
  under that feature.
- **The PTY harness's `settle` window is a quiet period, not a delay.**
  `run_snp_pty` passes 2 s; the old meaning of that argument (sleep before
  writing keys) is gone.
- **`support::event_sink` has no current consumer.** The suite's live
  lifecycle-event channel is `SNP_TEST_EVENTS_DIR` →
  `src/auto_sync/test_events.rs`. The `EventSink` / `EventWriter` types in
  `tests/support/event_sink.rs` read and write that same file shape but no
  target imports them today.
- **`cargo test --workspace --all-features -- --test-threads=1`** is the
  full-suite command — it is what the fix commits were verified against. It
  is not run by CI.

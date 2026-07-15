# Auto-Sync Architecture

Deep-dive reference for the auto-sync subsystem. For the full sync protocol and merge strategy, see [sync.md](sync.md).

## Overview

Auto-sync is an optional, opt-in background synchronization mechanism (Release 5A–5D). It is **disabled by default** and must be explicitly enabled via `snp sync config --auto-sync on`.

When enabled, mutation commands (`new`, `edit`, `import`, `delete`, `library create/delete`) trigger a detached one-shot worker that performs the remote sync after the local change is committed. The core invariant: **local mutations always succeed before any remote work begins.**

## Canonical Data Flow

```text
mutation command
  -> validate input
  -> atomic local commit (save_library / save_snippets)
  -> audit + local success
  -> notify_mutation(kind, origin)
  -> AutoSyncPolicy::resolve()
  -> origin check (SyncMerge -> suppress)
  -> mark_pending(state_dir, snapshot) -> PendingState{generation, ...}
  -> spawn_dispatch(state_dir)
     -> try_acquire(state_dir) -> WorkerLock
     -> spawn::spawn_worker(current_exe, "auto-sync-worker", state-dir, nonce)
        -> setsid() (Unix) / DETACHED_PROCESS | CREATE_NO_WINDOW (Windows)
        -> stdin/stdout/stderr -> null
        -> parent process exits immediately, orphan detaches
  -> return AutoSyncNotificationResult::Scheduled{generation}

detached worker process (snp auto-sync-worker --state-dir ... --nonce ...)
  -> policy = AutoSyncPolicy::resolve(...)
  -> try_acquire(state_dir) -> WorkerLock (RAII)
  -> nonce_already_used? -> exit NothingToDo
  -> read_state_from_dir(state_dir) -> PendingState
  -> execute_sync(state_dir, pending, policy)
     -> run_default_sync() inside OnceLock<tokio::runtime>
        bounded by sync_timeout (default 30s)
     -> on success: clear_if_generation_matches(state_dir, generation)
     -> on failure: record_failure(state_dir, generation, classification)
  -> exit WorkerOutcome
```

**Key point:** All mutation commands use a single central API (`notify_mutation` / `notify_local_mutation`). No command spawns its own worker or schedules its own pending state.

## Module Layout

The auto-sync subsystem lives under `src/auto_sync/` as a directory module:

- `policy.rs` — `AutoSyncPolicy`, `MutationKind`, `MutationOrigin`, `FailureClass`, debounce/timeout constants
- `pending.rs` — durable `PendingState` (schema v2, CRC32 integrity), v1 → v2 migration
- `lock.rs` — `WorkerLock` RAII, `WorkerLockContents` (`pid`/`started_at_unix_ms`/`nonce`), `process_alive`
- `spawn.rs` — `spawn_worker`, `apply_platform_detach` (libc `setsid` on Unix, `DETACHED_PROCESS|CREATE_NO_WINDOW` on Windows)
- `worker.rs` — `run`, `try_schedule`, `execute_sync`, `startup_recover`, `WorkerOutcome`, `SpawnResult`
- `notification.rs` — `notify_mutation`, `notify_local_mutation`, `clear_pending_after_explicit_sync`, `startup_recover_pending`, `MutationContext`, `AutoSyncNotificationResult`, `derive_state_dir`
- `mod.rs` — pub re-exports + `paths::{state_dir, pending_marker, worker_lock}` helpers used by `snp doctor`

## Key Types

### AutoSyncPolicy

```rust
pub struct AutoSyncPolicy {
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub max_retries: u32,
    pub sync_timeout: Duration,
}
```

Resolved once per command invocation from `SyncSettings`. Worker uses an internal copy created from `get_sync_settings()` at startup; the parent uses a short-lived resolved copy.

### MutationKind

```rust
pub enum MutationKind {
    SnippetCreate,
    SnippetUpdate,
    SnippetDelete,
    Import,
    LibraryChange,
    PremadeInstall,
    SyncConflictWrite,
    AccountConfig,  // Never triggers sync
}
```

### MutationOrigin

```rust
pub enum MutationOrigin {
    User,       // User-initiated mutation
    Import,     // Import operation
    SyncMerge,  // Sync merge — NEVER triggers auto-sync (prevents loops)
    Recovery,   // Recovery operation
}
```

### AutoSyncNotificationResult

```rust
pub enum AutoSyncNotificationResult {
    Disabled,
    Suppressed,
    Scheduled { generation: u64 },
    SchedulingFailed { generation: Option<u64> },
}
```

### PendingState (Schema v2)

```rust
pub struct PendingState {
    pub generation: u64,
    pub snapshot: PendingSnapshot,
    pub created_at_unix_ms: u64,
}
```

### WorkerOutcome

```rust
pub enum WorkerOutcome {
    Success,
    Failed,
    NothingToDo,
}
```

## Trigger Matrix

| Operation | MutationKind | Remote-syncable | Auto-sync event | Notes |
|---|---:|---:|---:|---|
| new snippet | SnippetCreate | yes | yes | after save |
| edit command | SnippetUpdate | yes | yes | after editor save |
| output-only edit | SnippetUpdate | no | no | local-only field |
| delete/tombstone | SnippetDelete | yes | yes | after tombstone save |
| import dry-run | — | no | no | read-only |
| import merge no-op | — | no | no | no event |
| import create/replace | Import | yes | one | logical transaction |
| set primary library | — | no | no | local-only metadata |
| sync merge write | SyncConflictWrite | already sync | no | recursion suppressed |
| library create | LibraryChange | yes | yes | after library created |
| library delete | LibraryChange | yes | yes | after library deleted |
| premade get | — | local copy | no | no trigger |
| `snp sync` (explicit) | — | — | clears pending | prevents duplicate delayed sync |

## Detached Worker Model

Auto-sync no longer uses an in-process debounce coordinator. Instead, the parent mutation command:

1. Records a monotonic pending generation.
2. Acquires an advisory worker lock (`auto-sync-worker.lock`).
3. Re-execs the current binary as `snp auto-sync-worker --state-dir <dir> --nonce <nonce>` with platform-detached flags (`setsid` on Unix, `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows) and `stdin`/`stdout`/`stderr` routed to `null`.
4. Returns to the user immediately.

The detached worker:

- Acquires the same lock (or exits with `NothingToDo` if another worker holds it).
- Detects duplicate nonces via `auto-sync-worker.<nonce>.done` sentinel files.
- Runs a single sync attempt bounded by `sync_timeout` (default 30s).
- Clears pending on success (conditional on observed generation), preserves on failure for recovery.
- Exits with `WorkerOutcome::{Success, Failed, NothingToDo}` mapped to internal exit codes (0/0/0).

The parent never waits for the worker. There is no IPC, no in-process debounce state, no shared Tokio runtime across the fork boundary.

## Durable Pending State

**File:** `~/.config/snp/auto-sync-pending.toml`

```toml
schema = 2
generation = 1
created_at_unix_ms = 1700000000000

[snapshot.Mutation]
kind = "snippet_create"

integrity = "crc32:441c462e"
```

- Schema `2` — v1 markers migrate transparently on load.
- Monotonic `generation` increments per `mark_pending`; conditional clear keyed on observed generation prevents stale workers from clobbering newer state.
- `created_at_unix_ms` records when the marker was written; >5 minute age → cleared by `startup_recover_pending`.
- `integrity` is `crc32:<hex>` over the serialized snapshot — rejects tampered files.
- Written atomically via `tempfile + rename + fsync`.
- No secrets, commands, or snippet content in the file.
- 0o600 permissions on Unix.

## Cross-Process Locking

**File:** `~/.config/snp/auto-sync-worker.lock`

```toml
pid = 12345
started_at_unix_ms = 1700000000000
nonce = "abc-12345-def"
```

- Atomic acquisition via `OpenOptions::create_new(true)` — only one parent wins.
- Stale detection: `kill -0 pid` on Unix (process dead → stale), plus >5 minute age.
- Stale locks are reclaimed transparently by `try_acquire`.
- Restrictive permissions (0o600 on Unix).
- Worker lock is **released** at worker exit (RAII `Drop` removes the file).
- Parent lock after successful spawn is `mem::forget`-ed (the file outlives the parent) so the worker can detect it via `inspect`.

## Worker Lifecycle

```text
Parent (snp new)
  |
  |-- mark_pending(state_dir) -> generation=N
  |-- try_acquire(state_dir)  -> WorkerLock (held)
  |-- spawn_worker(current_exe, "auto-sync-worker", state_dir, nonce)
  |     |-- setsid() / DETACHED_PROCESS | CREATE_NO_WINDOW
  |     |-- stdin/stdout/stderr -> null
  |     |-- fork+exec child
  |-- mem::forget(WorkerLock) so lock file outlives parent
  |-- return Scheduled{generation=N} to mutation command

Child (snp auto-sync-worker --state-dir ... --nonce ...)
  |
  |-- AutoSyncPolicy::resolve(get_sync_settings())
  |-- try_acquire(state_dir)
  |     |-- AlreadyHeld -> NothingToDo (another worker owns it)
  |-- nonce_already_used? -> NothingToDo
  |-- read_state_from_dir(state_dir) -> PendingState
  |-- execute_sync(state_dir, &pending, &policy)
  |     |-- if !policy.enabled -> clear_if_generation_matches, NothingToDo
  |     |-- run_with_timeout(run_default_sync, sync_timeout)
  |     |-- on Ok -> Success, record_success
  |     |-- on Err -> Failed, record_failure (preserves pending)
  |-- exit(0)  (WorkerOutcome mapping is internal; parent never sees it)
```

## Retry and Backoff

There is no in-process retry loop. The detached worker attempts one sync, bounded by `sync_timeout`. If that attempt fails:

- The pending marker is preserved (generation unchanged).
- The next `mark_pending` from a future mutation will increment the generation, signaling that any later worker should re-attempt.
- `startup_recover_pending` clears stale pending state (>5 min) on next parent startup.

For users who want stronger delivery guarantees, manual `snp sync` and `snp cron` remain the canonical recovery paths.

## Failure Policy

| Mode | Behavior |
|------|----------|
| `Ignore` | Silent — debug-level log only, no user-facing output |
| `Warn` (default) | Stderr warning: `warning: auto-sync scheduling failed; pending work preserved for recovery` |
| `Error` | Stderr error: `error: auto-sync scheduling failed; pending work preserved for recovery`, nonzero exit code, but local mutation remains committed |

These messages fire only when the **parent** fails to record the pending marker or spawn the worker. Worker-side failures are logged to `~/.config/snp/logs/` and surface via `snp doctor` diagnostics, not stderr — the user is no longer present when the worker runs.

## Local-Only Fields

The `output` field is local-only — not in `ProtoSnippet`, never uploaded or downloaded. Edits that change only the `output` field do NOT trigger auto-sync.

The `favorite` and `folders` fields are also local-only — preserved when server wins the merge conflict.

## Explicit Sync Precedence

When `--sync` flag is used (on `run`, `clip`, `search`, or TUI delete):

1. Explicit sync runs immediately via `run_default_sync()`
2. Pending auto-sync state is cleared via `clear_pending_after_explicit_sync()`
3. No duplicate delayed sync for the same mutation generation

## Design Decisions

### Architecture: Detached One-Shot Worker (Corrective)

Replaces the in-process coordinator with a hidden `auto-sync-worker` subcommand re-execed by the parent. The parent never blocks waiting for the worker; the worker is fully independent once spawned.

Alternatives evaluated:
- **In-process debounce** (predecessor) — added visible latency to mutation commands and held the parent process hostage during network round-trips.
- **Persistent daemon** — disproportionate for a CLI tool with no existing long-running process; would require lifecycle, IPC, and uninstall handling.
- **Detached helper process** (chosen) — re-exec is portable, zero-cost IPC (no IPC), and reuses the same `snp` binary's sync code path.

### Sync Target: Global

`run_default_sync` syncs all configured libraries. The `MutationContext::library_id` field is retained for forward compatibility but currently unused.

### Delivery Guarantees: Best-Effort

Auto-sync is convenience, not durable delivery. The durable pending marker survives crash/restart, and `startup_recover_pending` clears stale state (>5 minutes). Manual `snp sync` and `snp cron` remain the recovery path for missed syncs.

### Process Detachment

- Unix: `libc::setsid()` puts the worker in a new session, ensuring it does not die when the parent exits.
- Windows: `DETACHED_PROCESS | CREATE_NO_WINDOW` flags on `CreateProcess`.
- All file descriptors are released; stdin/stdout/stderr are routed to `null` so the worker cannot interfere with a TTY.

## Doctor Integration

`snp doctor --compatibility` inspects auto-sync state using the new path helpers:
- `paths::state_dir()` — directory containing all auto-sync artifacts.
- `paths::pending_marker()` — full path to the pending TOML.
- `paths::worker_lock()` — full path to the worker lock TOML.

Diagnostics emitted:
- `compat.auto_sync.enabled` / `compat.auto_sync.disabled` — policy state.
- `compat.auto_sync.pending_active` / `compat.auto_sync.pending_stale` / `compat.auto_sync.pending_unreadable` — pending marker status.
- `compat.auto_sync.lock_held` / `compat.auto_sync.lock_stale` / `compat.auto_sync.lock_unreadable` — worker lock status.
- Liveness probe uses `lock::process_alive(pid)` which calls `kill -0` on Unix and a placeholder always-true on Windows.

## Safety Invariants

1. Auto-sync is disabled by default.
2. Local mutation commits before any remote work begins.
3. Remote failure never rolls back or corrupts a successful local mutation.
4. SyncMerge origin never triggers auto-sync (prevents loops).
5. All syncable mutations use one notification API.
6. Triggers occur strictly after commit.
7. No command payload, credentials, or encryption material in coordinator artifacts (pending marker, lock file, worker argv, worker env).
8. Manual and scheduled sync remain independent.
9. Local state survives every remote/scheduling failure.
10. No auto-sync fields enter snippet TOML, ProtoSnippet, or import/export schema.
11. Worker is fully detached — its lifetime is not coupled to the parent's TTY.
12. Cross-process safety: stale locks reclaimed, dead processes detected via `kill -0`, no permanent deadlock.
13. Pending state generation is monotonic and conditional — stale workers cannot clobber fresh state.
14. Pending marker integrity-checked via CRC32; tampered files fail closed.

## Hidden Subcommand

`auto-sync-worker` is registered with `hide = true` in the clap CLI — it does not appear in `--help` output and is intended only for internal use by the parent process. It accepts `--state-dir <path>` and `--nonce <id>` and exits with `WorkerOutcome` mapped to internal exit codes (currently 0 for all outcomes — failures are logged, not propagated).
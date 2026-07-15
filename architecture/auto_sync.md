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
  -> pending::record_pending_mutation(state_dir, snapshot) -> PendingState{generation, ...}
     (the ONLY writer/incrementer of the pending generation)
  -> worker::schedule_existing_pending(state_dir)
     -> spawn::spawn_worker(current_exe, "auto-sync-worker", --state-dir)
        -> setsid() (Unix) / DETACHED_PROCESS | CREATE_NO_WINDOW (Windows)
        -> stdin/stdout/stderr -> null
        -> parent process exits immediately, orphan detaches
  -> return AutoSyncNotificationResult::Scheduled{generation}

detached worker process (snp auto-sync-worker --state-dir ...)
  -> policy = AutoSyncPolicy::resolve(...)
  -> try_acquire(state_dir) -> WorkerLock (RAII) [parent never acquired it]
  -> read_state_from_dir(state_dir) -> PendingState
  -> debounce loop:
       -> reload marker; if newer generation appeared -> restart debounce
       -> sleep until observed_timestamp + debounce, clamped by max_lifetime
  -> execute_sync(state_dir, observed_generation, observed_snapshot, policy)
     -> run_default_sync() inside OnceLock<tokio::runtime>
        bounded by sync_timeout via tokio::time::timeout (default 30s)
     -> on success: pending::clear_if_generation_matches(state_dir, observed_generation)
     -> on failure: leave pending intact (recovery will retry)
  -> exit WorkerOutcome
```

**Key point:** All mutation commands use a single central API (`notify_mutation` / `notify_local_mutation`). No command spawns its own worker or schedules its own pending state. **Release 5 corrective:** the API split guarantees that only `record_pending_mutation` increments the generation; `schedule_existing_pending` never mutates the marker. The parent never holds the worker lock — every spawned worker races for the lock and exactly one wins.

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

1. Records a monotonic pending generation (via `pending::record_pending_mutation`).
2. Schedules a worker (via `worker::schedule_existing_pending`).
3. **Release 5 corrective:** the parent does **not** acquire the worker lock — the lock is the worker's responsibility.
4. Re-execs the current binary as `snp auto-sync-worker --state-dir <dir>` with platform-detached flags (`setsid` on Unix, `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows) and `stdin`/`stdout`/`stderr` routed to `null`.
5. Returns to the user immediately.

The detached worker:

- Acquires the worker lock itself (or exits with `NothingToDo` if another worker holds it).
- Reads pending state, then runs a **debounce loop**: it reloads the marker every ≤250ms, restarts the deadline if a newer generation has appeared, and waits up to `policy.debounce + max_lifetime` (default 5 minutes).
- Runs a single sync attempt bounded by `sync_timeout` (default 30s), wrapped in `tokio::time::timeout` to drop abandoned work cleanly.
- Clears pending on success via `pending::clear_if_generation_matches(state_dir, observed_generation)`. **Release 5 corrective:** clearing is conditional on the observed generation, so a stale worker cannot clobber newer state.
- On failure or `NothingToDo`, the marker is preserved for `startup_recover_pending`.
- A newer generation that appears during sync is detected on the next loop iteration and triggers a follow-up cycle.
- Exits with `WorkerOutcome::{Success, Failed, NothingToDo}` mapped to internal exit codes (0/0/0).

The parent never waits for the worker. There is no IPC, no in-process debounce state, no shared Tokio runtime across the fork boundary. The worker creates its own Tokio runtime internally.

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
- Monotonic `generation` increments per `record_pending_mutation` (the only writer). **Release 5 corrective:** `mark_pending` is a legacy alias preserved for the `doctor --compatibility` interface, but every production call site has been migrated to `record_pending_mutation` so generation ownership is unambiguous.
- `clear_if_generation_matches(observed_generation)` is the only clear path; stale workers cannot clobber newer state.
- `created_at_unix_ms` records when the marker was written; >5 minute age → cleared by `startup_recover_pending` only after re-checking the lock state.
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

- Atomic acquisition via `OpenOptions::create_new(true)` — only one worker wins.
- **Release 5 corrective:** the parent never acquires the lock. The lock exists for the worker; the parent only inspects it (via `lock::process_alive`) to detect liveness.
- Stale detection: `kill -0 pid` on Unix (process dead → stale), plus >5 minute age.
- Stale locks are reclaimed transparently by `try_acquire`.
- Restrictive permissions (0o600 on Unix).
- Worker lock is **released** at worker exit (RAII `Drop` removes the file).
- Each spawned worker generates a fresh nonce in its lock entry; workers spawned concurrently race for the lock and exactly one wins.

## Worker Lifecycle

```text
Parent (snp new)
  |
  |-- record_pending_mutation(state_dir, snapshot) -> PendingState{generation=N, ...}
  |     (only writer/incrementer; conditional write)
  |-- schedule_existing_pending(state_dir)
  |     -> spawn::spawn_worker(current_exe, "auto-sync-worker", --state-dir)
  |           |-- setsid() / DETACHED_PROCESS | CREATE_NO_WINDOW
  |           |-- stdin/stdout/stderr -> null
  |           |-- fork+exec child
  |           (parent does NOT acquire the worker lock)
  |-- return Scheduled{generation=N} to mutation command

Child (snp auto-sync-worker --state-dir ...)
  |
  |-- AutoSyncPolicy::resolve(get_sync_settings())
  |-- try_acquire(state_dir)
  |     |-- AlreadyHeld -> NothingToDo (another worker owns it)
  |-- read_state_from_dir(state_dir) -> PendingState{generation, timestamp, snapshot}
  |-- DEBOUNCE LOOP (bounded by max_lifetime, default 5 minutes):
  |     |-- compute_deadline(observed_timestamp, policy.debounce, start, max_lifetime)
  |     |-- wait_for_quiet(state_dir, observed_generation, deadline, ...)
  |           (reloads marker every ≤250ms; if a newer generation appears,
  |            this iteration observes the change via reload and may restart
  |            the loop with the new generation)
  |     |-- execute_sync(state_dir, observed_generation, observed_snapshot, policy)
  |           |-- if !policy.enabled -> WorkerOutcome::NothingToDo
  |           |-- run_async_with_timeout(policy.sync_timeout)
  |                 -> run_default_sync on a worker-owned Tokio runtime,
  |                    cancelled via tokio::time::timeout on expiry
  |     |-- if Success && newer_generation_observed -> continue loop (follow-up)
  |     |-- if Success && no newer generation -> clear_if_generation_matches, exit
  |     |-- if Failed -> preserve pending, exit
  |-- exit(0)  (WorkerOutcome mapping is internal; parent never sees it)
```

## Retry and Backoff

There is no in-process retry loop. The detached worker attempts one sync per generation, bounded by `sync_timeout`. The debounce loop may cycle multiple times in one worker invocation:

- If sync succeeds but a newer generation is now on disk, the worker loops again to service the newer work.
- If sync fails, the worker exits; the pending marker is preserved (generation unchanged).
- The next `record_pending_mutation` from a future mutation will increment the generation, signaling that the next worker should service the new work.
- `startup_recover_pending` clears stale pending state (>5 min) on next parent startup, then re-spawns a worker to retry.

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

1. Capture the observed pending generation via `observe_pending_generation()` before sync.
2. Run explicit sync immediately via `run_default_sync()`.
3. Clear the pending marker via `clear_pending_after_explicit_sync(observed_generation, sync_succeeded)` — **Release 5 corrective:** clearing is conditional on the observed generation, so a mutation that arrived during the sync is preserved for the next worker.

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
7. No command payload, credentials, or encryption material in worker artifacts (pending marker, lock file, worker argv, worker env).
8. Manual and scheduled sync remain independent.
9. Local state survives every remote/scheduling failure.
10. No auto-sync fields enter snippet TOML, ProtoSnippet, or import/export schema.
11. Worker is fully detached — its lifetime is not coupled to the parent's TTY.
12. Cross-process safety: stale locks reclaimed, dead processes detected via `kill -0`, no permanent deadlock.
13. Pending state generation is monotonic and conditional — stale workers cannot clobber fresh state.
14. Pending marker integrity-checked via CRC32; tampered files fail closed.

## Hidden Subcommand

`auto-sync-worker` is registered with `hide = true` in the clap CLI — it does not appear in `--help` output and is intended only for internal use by the parent process. It accepts `--state-dir <path>` and `--nonce <id>` and exits with `WorkerOutcome` mapped to internal exit codes (currently 0 for all outcomes — failures are logged, not propagated).
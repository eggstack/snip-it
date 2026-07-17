# Auto-Sync Architecture

Deep-dive reference for the auto-sync subsystem. For the full sync protocol and merge strategy, see [sync.md](sync.md).

## Overview

Auto-sync is an optional, opt-in background synchronization mechanism (Release 5A–5F). It is **disabled by default** and must be explicitly enabled via `snp sync config --auto-sync on`.

When enabled, mutation commands (`new`, `edit`, `import`, `delete`, `library create/delete`) trigger a detached one-shot worker that performs the remote sync after the local change is committed. The architecture uses two subprocess roles: a **debounce worker** (`auto-sync-worker`) that manages timing and a **killable sync executor** (`auto-sync-execute`) that performs the actual sync. All sync operations — worker, manual `snp sync`, explicit `--sync`, and cron — share a single `SyncExecutionLock` to prevent concurrent sync. The core invariant: **local mutations always succeed before any remote work begins.**

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
  -> try_acquire(state_dir) -> SyncExecutionLock (RAII) [parent never acquired it]
  -> read_state_from_dir(state_dir) -> PendingState
  -> debounce loop:
       -> reload marker; if newer generation appeared -> restart debounce
       -> sleep until observed_timestamp + debounce, clamped by max_lifetime
  -> execute_sync(state_dir, observed_generation, observed_snapshot, policy)
     -> spawn executor subprocess (snp auto-sync-execute --state-dir ...)
     -> wait_child_with_timeout(child, policy.sync_timeout)
        -> on success: map ExecutorExitCode to WorkerOutcome
        -> on timeout: SIGTERM -> wait 2s -> SIGKILL -> wait -> WorkerOutcome::Failed
     -> on success with no newer generation: clear_if_generation_matches
     -> on success with newer generation: continue loop (follow-up cycle)
     -> release execution lock
  -> exit WorkerOutcome
```

**Key point:** All mutation commands use a single central API (`notify_mutation` / `notify_local_mutation`). No command spawns its own worker or schedules its own pending state. **Release 5 corrective:** the API split guarantees that only `record_pending_mutation` increments the generation; `schedule_existing_pending` never mutates the marker. The parent never holds the execution lock — every spawned worker races for the lock and exactly one wins. **Release 5F:** the worker spawns an executor subprocess for the actual sync; on timeout the executor is killed via SIGTERM/SIGKILL.

## Module Layout

The auto-sync subsystem lives under `src/auto_sync/` as a directory module:

- `policy.rs` — `AutoSyncPolicy`, `MutationKind`, `MutationOrigin`, `FailureClass`, debounce/timeout constants
- `pending.rs` — durable `PendingState` (schema v2, CRC32 integrity), v1 → v2 migration, `ConditionalClearResult`
- `pending_lock.rs` — `PendingTxnGuard` RAII, short-lived transaction lock for pending-marker operations, unique temp paths, atomic writes, directory fsync
- `lock.rs` — `WorkerLock` RAII, `WorkerLockContents` (`pid`/`started_at_unix_ms`/`nonce`), `process_alive`, ownership-checked drop
- `execution_lock.rs` — `SyncExecutionLock` RAII, shared execution lock for all sync operations, `try_acquire`, `wait_acquire`, `ExecutionLockError`
- `executor.rs` — `ExecutorExitCode`, `effective_sync_direction`, `run_executor` entry point
- `spawn.rs` — `spawn_worker`, `spawn_executor`, `apply_platform_detach` (libc `setsid` on Unix, `DETACHED_PROCESS|CREATE_NO_WINDOW` on Windows)
- `worker.rs` — `run`, `try_schedule`, `execute_sync`, `startup_recover`, `WorkerOutcome`, `SpawnResult`
- `notification.rs` — `notify_mutation`, `notify_local_mutation`, `clear_pending_after_explicit_sync`, `startup_recover_pending`, `MutationContext`, `AutoSyncNotificationResult`, `derive_state_dir`, `SubcommandTag`, `should_attempt_auto_sync_recovery`
- `mod.rs` — pub re-exports + `paths::{state_dir, pending_marker, pending_txn_lock, worker_lock, execution_lock}` helpers used by `snp doctor`

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

### ConditionalClearResult

```rust
pub enum ConditionalClearResult {
    Cleared,
    Missing,
    GenerationChanged { current: u64 },
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

### ExecutionLockError

```rust
pub enum ExecutionLockError {
    Io(std::io::Error),
    AlreadyHeld { pid: u32, started_at_unix_ms: u64, nonce: String },
    Timeout { owner_pid: u32, owner_started_at: u64 },
}
```

Returned by `try_acquire` (non-blocking) and `wait_acquire` (blocking with timeout). `AlreadyHeld` means another process holds a live lock; `Timeout` means the lock was still held after the wait period.

### ExecutorExitCode

```rust
#[repr(i32)]
pub enum ExecutorExitCode {
    Success = 0,
    NotConfigured = 2,
    AuthFailure = 3,
    NetworkTimeout = 4,
    ConflictPartial = 5,
    LocalPersistence = 6,
    InternalError = 7,
}
```

Standardized exit codes for the executor subprocess. The worker maps these to `WorkerOutcome`. Code 1 is reserved for the general CLI error path.

### SubcommandTag

```rust
pub enum SubcommandTag {
    Mutation,
    Sync,
    Cron,
    Register,
    AutoSyncWorker,
    AutoSyncExecute,
}
```

Used by `should_attempt_auto_sync_recovery` to classify commands at startup. Only `Mutation` (and `None`) commands attempt auto-sync recovery; `Sync`, `Cron`, `Register`, and internal subprocess tags suppress it.

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

Auto-sync uses a two-process-per-cycle model: a detached debounce worker and a killable sync executor. The parent mutation command:

1. Records a monotonic pending generation (via `pending::record_pending_mutation`).
2. Schedules a worker (via `worker::schedule_existing_pending`).
3. **Release 5 corrective:** the parent does **not** acquire the execution lock — the lock is the worker's responsibility.
4. Re-execs the current binary as `snp auto-sync-worker --state-dir <dir>` with platform-detached flags (`setsid` on Unix, `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows) and `stdin`/`stdout`/`stderr` routed to `null`.
5. Returns to the user immediately.

The detached worker:

- Acquires the `SyncExecutionLock` itself (or exits with `NothingToDo` if another sync holds it). **Phase 01 invariant:** the worker is the *only* component that holds this lock during a detached cycle.
- Reads pending state, then runs a **debounce loop**: it reloads the marker every ≤250ms, restarts the deadline if a newer generation has appeared, and waits up to `policy.debounce + max_lifetime` (default 5 minutes).
- Spawns an executor subprocess (`snp auto-sync-execute`) that **does not acquire the execution lock** — the worker is already holding it for the cycle. The executor simply invokes the canonical sync operation (`crate::sync_commands::run_sync`).
- Supervises the executor with `wait_child_with_timeout(policy.sync_timeout)` (default 30s). On timeout, sends SIGTERM, waits 2 seconds, then SIGKILL. **Phase 01 invariant:** the executor is reaped before the lock is released.
- Clears pending on success via `pending::clear_if_generation_matches(state_dir, observed_generation)`. **Phase 01 invariant:** clearing is conditional on the observed generation, so a stale worker cannot clobber newer state.
- On failure, the marker is preserved for `startup_recover_pending`; the worker exits with `Failed`.
- On `NothingToDo` (no pending state, lock contention, max-lifetime exceeded, or policy disabled), the marker is preserved — pending is only cleared on a real successful comparison.
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
- Monotonic `generation` increments per `record_pending_mutation` (the only writer). **Release 5E corrective:** `mark_pending` is module-private; all generation writes go through `record_pending_mutation` under `PendingTxnGuard`.
- `clear_if_generation_matches(observed_generation)` returns typed `ConditionalClearResult` (Cleared/Missing/GenerationChanged); stale workers cannot clobber newer state. **Release 5E corrective:** the read-compare-delete is atomic under `PendingTxnGuard`.
- `created_at_unix_ms` records when the marker was written; >5 minute age → cleared by `startup_recover_pending` only after re-checking the lock state.
- `integrity` is `crc32:<hex>` over all behavior-driving fields (schema, generation, created_at_unix_ms, snapshot) — rejects tampered or corrupted files.
- Written atomically via unique temp file per transaction (`pending_lock::unique_temp_path`) + rename + fsync + directory fsync.
- No secrets, commands, or snippet content in the file.
- 0o600 permissions on Unix.

## Cross-Process Locking

### Pending Transaction Lock

**File:** `~/.config/snp/auto-sync-pending.lock`

Short-lived lock protecting read-modify-write operations on the pending marker. Distinct from the worker execution lock.

```toml
pid = 12345
nonce = "abc-12345-def"
created_at_unix_ms = 1700000000000
```

- Atomic acquisition via `OpenOptions::create_new(true)`.
- **Release 5E corrective:** ownership-checked drop — only removes lock if PID and nonce still match.
- Dead-owner reclaim via `kill -0`; live owners never stolen regardless of age.
- Bounded retry with 1-5ms jitter up to 500ms.
- 0o600 permissions on Unix.

### Worker Execution Lock

**File:** `~/.config/snp/auto-sync-worker.lock`

Long-lived lock protecting the worker lifecycle (debounce + sync execution).

```toml
pid = 12345
started_at_unix_ms = 1700000000000
nonce = "abc-12345-def"
```

- Atomic acquisition via `OpenOptions::create_new(true)` — only one worker wins.
- **Release 5 corrective:** the parent never acquires the lock. The lock exists for the worker; the parent only inspects it (via `lock::process_alive`) to detect liveness.
- Stale detection: `kill -0 pid` on Unix (process dead → stale). **Release 5E corrective:** live PID means owned regardless of age — no age-based stale classification.
- **Release 5E corrective:** `Drop` reads the current lock record and removes it only when PID and nonce match the guard. An old guard never removes a replacement owner's lock.
- Restrictive permissions (0o600 on Unix).
- Each spawned worker generates a fresh nonce in its lock entry; workers spawned concurrently race for the lock and exactly one wins.

### Sync Execution Lock (Release 5F)

**File:** `~/.config/snp/auto-sync-execution.lock`

Shared lock preventing concurrent sync operations across all callers. Unlike the worker lock (which guards the worker lifecycle), this lock guards the actual sync operation — whether performed by the detached worker, manual `snp sync`, explicit `--sync` flag, or cron.

```toml
pid = 12345
started_at_unix_ms = 1700000000000
nonce = "abc-12345-def"
```

- Atomic acquisition via `OpenOptions::create_new(true)`.
- **Worker:** uses `try_acquire` — if the lock is busy, exits with `NothingToDo` (preserves pending for later).
- **Foreground callers** (`snp sync`, `--sync` flag): uses `wait_acquire` with a bounded timeout (default 30s) — polls every 250ms, returns `Timeout` error if still busy.
- Ownership-checked `Drop`: only removes the lock file if PID and nonce match.
- Stale detection: dead PIDs (via `kill -0`) are reclaimed automatically.
- 0o600 permissions on Unix. No secrets, commands, or snippet content.

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
  |           (parent does NOT acquire the execution lock)
  |-- return Scheduled{generation=N} to mutation command

Child (snp auto-sync-worker --state-dir ...)
  |
  |-- AutoSyncPolicy::resolve(get_sync_settings())
  |-- if !policy.enabled -> NothingToDo, exit early (pending preserved)
  |-- execution_lock::try_acquire(state_dir)
  |     |-- AlreadyHeld -> NothingToDo (another sync is in progress; pending preserved)
  |-- read_state_from_dir(state_dir) -> PendingState{generation, timestamp, snapshot}
  |-- DEBOUNCE LOOP (bounded by max_lifetime, default 5 minutes):
  |     |-- compute_deadline(observed_timestamp, policy.debounce, start, max_lifetime)
  |     |-- wait_for_quiet(state_dir, observed_generation, deadline, ...)
  |           (reloads marker every ≤250ms; if a newer generation appears,
  |            this iteration observes the change via reload and may restart
  |            the loop with the new generation)
  |     |-- execute_sync(state_dir, policy)
  |           |-- spawn::spawn_executor(state_dir)
  |           |     (snp auto-sync-execute --state-dir <dir>, NOT detached,
  |           |      does NOT acquire the execution lock — worker owns it)
  |           |-- wait_child_with_timeout(child, policy.sync_timeout)
  |           |     |-- on exit: map ExecutorExitCode -> WorkerOutcome
  |           |         (Success=0, NotConfigured=2, AuthFailure=3,
  |           |          NetworkTimeout=4, ConflictPartial=5,
  |           |          LocalPersistence=6, InternalError=7)
  |           |     |-- on timeout: SIGTERM -> 2s grace -> SIGKILL -> reap -> Failed
  |     |-- if Success && newer_generation_observed -> continue loop (follow-up)
  |     |-- if Success && no newer generation -> clear_if_generation_matches, exit
  |     |-- if Failed -> preserve pending, exit
  |     |-- if NothingToDo -> preserve pending, exit (no clearing)
  |-- release execution lock
  |-- exit(0)  (WorkerOutcome mapping is internal; parent never sees it)
```

### Executor Subprocess (Release 5F, Phase 01 invariant)

The worker spawns a child process (`snp auto-sync-execute`) instead of running sync in-process. This provides:

1. **Killable sync work:** On timeout, the worker sends SIGTERM then SIGKILL to the child. Unlike `tokio::time::timeout` (which cannot cancel a `spawn_blocking` task), killing a child process guarantees the sync work terminates.
2. **Worker-owned execution lock:** The executor does **not** acquire the `SyncExecutionLock` — the worker is already holding it for the cycle. All other sync entry points (`snp sync`, `snp sync --push-only`/`--pull-only`, `run --sync`, `clip --sync`, `search --sync`, post-selection `--sync`) acquire that same lock via `wait_acquire` to serialize with the worker.
3. **Canonical sync invocation:** The executor invokes `crate::sync_commands::run_sync(...)` directly — the same function used by foreground `snp sync`. There is no second sync implementation.
4. **Direction correctness:** The executor resolves the effective sync direction via `effective_sync_direction()`, which applies CLI flag overrides (`--push-only`, `--pull-only`) to the config setting. Detached sync uses the configured direction (no CLI override); foreground sync accepts explicit CLI overrides.
5. **Standardized exit codes:** The executor exits with codes from `ExecutorExitCode` (0=success, 2=not configured, 3=auth, 4=network/timeout, 5=conflict, 6=local persistence, 7=internal).

## Retry and Backoff

There is no in-process retry loop. The detached worker attempts one sync per generation, spawning an executor subprocess and supervising it with a timeout. The debounce loop may cycle multiple times in one worker invocation:

- If sync succeeds but a newer generation is now on disk, the worker loops again to service the newer work.
- If sync fails, the worker exits; the pending marker is preserved (generation unchanged).
- The next `record_pending_mutation` from a future mutation will increment the generation, signaling that the next worker should service the new work.
- `startup_recover_pending` clears stale pending state (>5 min) on next parent startup, then re-spawns a worker to retry.
- On timeout, the executor subprocess is terminated (SIGTERM then SIGKILL) before the execution lock is released.

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
2. Acquire the `SyncExecutionLock` via `wait_acquire` with bounded timeout (30s default).
3. Run explicit sync immediately via `run_default_sync()`.
4. Clear the pending marker via `clear_pending_after_explicit_sync(observed_generation, sync_succeeded)` — **Release 5 corrective:** clearing is conditional on the observed generation, so a mutation that arrived during the sync is preserved for the next worker.
5. Release the execution lock.

## Design Decisions

### Architecture: Two-Process-Per-Cycle (Release 5F)

Replaces the in-process coordinator (Release 5D) with a two-subprocess model: a detached debounce worker and a killable sync executor. The parent never blocks waiting for the worker; the worker never runs sync in-process.

Alternatives evaluated:
- **In-process debounce** (predecessor) — added visible latency to mutation commands and held the parent process hostage during network round-trips.
- **Persistent daemon** — disproportionate for a CLI tool with no existing long-running process; would require lifecycle, IPC, and uninstall handling.
- **Detached one-shot worker with in-process sync** (Release 5D) — re-exec is portable and zero-cost IPC, but `tokio::time::timeout` around `spawn_blocking` does not cancel the underlying thread. Sync work could outlive the timeout.
- **Detached worker + killable executor subprocess** (chosen, Release 5F) — re-exec is portable; the executor is a real child process that can be SIGTERM/SIGKILLed on timeout; shared `SyncExecutionLock` prevents concurrent sync across all callers.

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
- `paths::pending_txn_lock()` — full path to the pending transaction lock.
- `paths::worker_lock()` — full path to the worker lock TOML.
- `paths::execution_lock()` — full path to the execution lock TOML.

Diagnostics emitted:
- `compat.auto_sync.enabled` / `compat.auto_sync.disabled` — policy state.
- `compat.auto_sync.pending_active` / `compat.auto_sync.pending_stale` / `compat.auto_sync.pending_unreadable` — pending marker status.
- `compat.auto_sync.lock_held` / `compat.auto_sync.lock_stale` / `compat.auto_sync.lock_unreadable` — worker lock status.
- `compat.auto_sync.execution_lock_held` / `compat.auto_sync.execution_lock_stale` — execution lock status.
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
14. Pending marker integrity-checked via CRC32 over all behavior-driving fields; tampered files fail closed.
15. **Release 5E:** Pending marker mutations serialized via `PendingTxnGuard`; unique temp files per transaction.
16. **Release 5E:** Worker lock ownership-checked on drop; old owners cannot remove replacement locks.
17. **Release 5E:** Live worker locks never stolen due to age; dead-owner reclaim via `kill -0` only.
18. **Release 5F:** All sync operations share one `SyncExecutionLock`; no concurrent sync is possible.
19. **Release 5F:** Executor subprocess terminated (SIGTERM then SIGKILL) before execution lock released.
20. **Release 5F:** No `spawn_blocking` cancellation claim; sync work runs in a killable child process.
21. **Release 5F:** Startup recovery suppressed for sync-related commands (`sync`, `cron`, `register`, internal subprocesses).
22. **Phase 01:** Worker `NothingToDo` never clears the pending marker; only `Success` performs `clear_if_generation_matches`.
23. **Phase 01:** Disabled-policy worker exits with `NothingToDo` *before* touching pending state.
24. **Phase 01:** Executor subprocess never references the execution lock; the worker owns it for the entire cycle.

## Hidden Subcommands

`auto-sync-worker` and `auto-sync-execute` are registered with `hide = true` in the clap CLI — they do not appear in `--help` output and are intended only for internal use by the parent process.

- `auto-sync-worker` accepts `--state-dir <path>` and exits with `WorkerOutcome` mapped to internal exit codes (currently 0 for all outcomes — failures are logged, not propagated).
- `auto-sync-execute` accepts `--state-dir <path>` and performs the actual sync operation, exiting with `ExecutorExitCode` status codes.
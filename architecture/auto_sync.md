# Auto-Sync Architecture

Deep-dive reference for the auto-sync subsystem. For the full sync protocol and merge strategy, see [sync.md](sync.md).

## Overview

Auto-sync is an optional, opt-in background synchronization mechanism (Release 5A–5F). It is **disabled by default** and must be explicitly enabled via `snp sync config --auto-sync on`.

When enabled, mutation commands (`new`, `edit`, `import`, `delete`, `library create/delete`) trigger a detached one-shot worker that performs the remote sync after the local change is committed. The architecture uses two subprocess roles: a **debounce worker** (`auto-sync-worker`) that manages timing and a **killable sync executor** (`auto-sync-execute`) that performs the actual sync. All sync operations — worker, manual `snp sync`, explicit `--sync`, and cron — share a single `SyncExecutionLock` to prevent concurrent sync. The core invariant: **local mutations always succeed before any remote work begins.** The executor timeout (30s default, configurable via `AutoSyncPolicy::sync_timeout`) is independent of the debounce window.

## Canonical Data Flow

```text
mutation command
  -> validate input
  -> atomic local commit (save_library / save_snippets)
  -> audit + local success
  -> notify_mutation(kind, origin)
  -> AutoSyncPolicy::resolve()
  -> origin check (SyncMerge -> suppress)
  -> sync_configured check: if false -> Disabled (no sync account configured)
  -> pending::record_pending_mutation(state_dir, snapshot) -> PendingState{generation, ...}
     (the ONLY writer/incrementer of the pending generation)
  -> if auto_sync disabled: return PendingRecorded{generation} (intent preserved, no worker scheduled)
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

**Key point:** All mutation commands use a single central API (`notify_mutation` / `notify_local_mutation`). No command spawns its own worker or schedules its own pending state. `schedule_sync()` is the sole scheduling authority — it centralizes all spawn decisions and prevents worker storms. **Release 5 corrective:** the API split guarantees that only `record_pending_mutation` increments the generation; `schedule_existing_pending` never mutates the marker. The parent never holds the execution lock — every spawned worker races for the lock and exactly one wins. **Release 5F:** the worker spawns an executor subprocess for the actual sync; on timeout the executor is killed via SIGTERM/SIGKILL.

## Module Layout

The auto-sync subsystem lives under `src/auto_sync/` as a directory module:

- `policy.rs` — `AutoSyncPolicy` (typed policy loading distinguishes `NotConfigured` from config failure), `MutationKind`, `MutationOrigin`, `FailureClass` (11 variants with distinct exit codes), `RetryDisposition`, `transient_backoff()`, debounce/timeout constants
- `pending.rs` — durable `PendingState` (schema v2, CRC32 integrity), v1 → v2 migration, `ConditionalClearResult`
- `pending_lock.rs` — `PendingTxnGuard` RAII, short-lived transaction lock for pending-marker operations, unique temp paths, atomic writes, directory fsync
- `lock.rs` — `WorkerLock` RAII, `WorkerLockContents` (`pid`/`started_at_unix_ms`/`nonce`), `process_alive`, ownership-checked drop
- `execution_lock.rs` — `SyncExecutionLock` RAII, shared execution lock for all sync operations, `try_acquire`, `wait_acquire`, `ExecutionLockError`
- `executor.rs` — `ExecutorExitCode`, `classify_sync_error()`, `effective_sync_direction`, `run_executor` entry point
- `spawn.rs` — `spawn_worker`, `spawn_executor`, `apply_platform_detach` (libc `setsid` on Unix, `DETACHED_PROCESS|CREATE_NO_WINDOW` on Windows)
- `worker.rs` — `run`, `execute_sync`, `startup_recover`, `WorkerOutcome`, `SpawnResult`, `Clock` trait for deterministic testing
- `status.rs` — `AutoSyncStatus` (durable status persistence with CRC32 integrity over behavior-driving fields), `record_success()`, `record_failure()`, `compute_config_fingerprint()`, `release_deferral_on_config_change()`, secret redaction
- `schedule.rs` — `schedule_sync()` (centralized scheduling decision), `ScheduleDecision` enum, `Caller` enum, worker storm prevention
- `notification.rs` — `notify_mutation`, `notify_local_mutation`, `clear_pending_after_explicit_sync`, `startup_recover_pending`, `MutationContext`, `AutoSyncNotificationResult`, `derive_state_dir`, `SubcommandTag`, `should_attempt_auto_sync_recovery`
- `mod.rs` — pub re-exports + `paths::{state_dir, pending_marker, pending_txn_lock, worker_lock, execution_lock, status_file}` helpers used by `snp doctor`

## Key Types

### AutoSyncPolicy

```rust
pub struct AutoSyncPolicy {
    pub sync_configured: bool,  // settings.enabled — sync account exists
    pub enabled: bool,          // settings.auto_sync && settings.enabled
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub max_retries: u32,
    pub sync_timeout: Duration,
    pub max_delay: Duration,
}
```

Resolved once per command invocation from `SyncSettings`. `sync_configured` indicates that a sync account is configured (`enabled = true` in sync.toml), regardless of whether auto-sync execution is enabled. The parent uses `sync_configured` to decide whether to record pending intent (preserving synchronization intent even when auto-sync is disabled). The worker uses `enabled` to decide whether to actually perform sync.

**Note:** `max_delay` is a separate config from `debounce`. `debounce` controls the quiet period (how long to wait after the last change), while `max_delay` caps the total elapsed time before forcing a sync attempt — even if changes continue to arrive. This prevents indefinite starvation under continuous mutations.

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
    PendingRecorded { generation: u64 },
    SchedulingFailed { generation: Option<u64> },
}
```

`PendingRecorded` is returned when sync is configured but auto-sync execution is disabled — the pending marker is created (preserving intent) but no worker is scheduled.

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

Standardized exit codes for the executor subprocess. The worker maps these to `WorkerOutcome`. Code 1 is reserved for the general CLI error path. Every failure class has a distinct exit code (11 distinct codes), enabling precise failure diagnosis from exit status alone.

Each exit code maps to a `FailureClass` via the `failure_class()` method on `ExecutorExitCode`. `FailureClass` maps back to an exit code via `from_failure_class()`. This bidirectional mapping is used by the durable status system to record the failure category for backoff and retry decisions.

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
- `created_at_unix_ms` records when the marker was written. **Phase 02:** startup recovery is read-only with respect to generation — valid pending work is recoverable regardless of age, not just within 5 minutes.
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
  |     |-- debounce(state_dir, observed_generation, policy.debounce, max_delay, clock) -> DebounceResult
  |           (reloads marker every ≤250ms; if a newer generation appears,
  |            this iteration observes the change via reload and may restart
  |            the loop with the new generation; returns the latest observed state)
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

The detached worker uses a **one-attempt-per-lifecycle** model: each worker invocation performs a single sync attempt, spawning an executor subprocess and supervising it with a timeout. However, failure outcomes are persisted as durable backoff state via `auto-sync-status.toml`, allowing the *next* scheduling decision to defer retry based on an exponential backoff schedule.

**Backoff schedule:** ~5s, ~15s, ~30s, ~60s, then exponential growth capped at 15 minutes with bounded jitter. Each failure class that permits retry records a `next_attempt_at_unix_ms` timestamp in the status file. The worker never sleeps to honor backoff — it simply exits with `Failed` and the `schedule_sync()` function defers spawning until the backoff window expires.

- If sync succeeds but a newer generation is now on disk, the worker loops again to service the newer work.
- If sync fails, the worker exits; the pending marker is preserved (generation unchanged) and the failure class determines whether backoff is recorded or attention is required.
- The next `record_pending_mutation` from a future mutation will increment the generation, signaling that the next worker should service the new work.
- `startup_recover_pending` always schedules a worker for valid pending work regardless of age; it no longer clears stale pending based on the 5-minute threshold alone.
- On timeout, the executor subprocess is terminated (SIGTERM then SIGKILL) before the execution lock is released.
- Configuration or credential changes can clear deferred disposition by resetting `next_attempt_at_unix_ms` to zero in the status file, allowing immediate retry.

For users who want stronger delivery guarantees, manual `snp sync` and `snp cron` remain the canonical recovery paths.

## Failure Policy

| Mode | Behavior |
|------|----------|
| `Ignore` | Silent — debug-level log only, no user-facing output |
| `Warn` (default) | Stderr warning: `warning: auto-sync scheduling failed; pending work preserved for recovery` |
| `Error` | Stderr error: `error: auto-sync scheduling failed; pending work preserved for recovery`, nonzero exit code, but local mutation remains committed |

These messages fire only when the **parent** fails to record the pending marker or spawn the worker. Worker-side failures are logged to `~/.config/snp/logs/` and surface via `snp doctor` diagnostics, not stderr — the user is no longer present when the worker runs.

### Failure Class Retry Dispositions

Each `FailureClass` maps to a specific retry disposition:

| Failure Class | Retry Disposition |
|---|---|
| `DeferredDisabled` | WaitForConfigurationChange |
| `DeferredNotConfigured` | WaitForConfigurationChange |
| `TransientNetwork` | RetryAfter(exponential backoff) |
| `TransientTimeout` | RetryAfter(exponential backoff) |
| `Authentication` | RequiresAttention |
| `Configuration` | RequiresAttention |
| `CredentialStore` | RequiresAttention |
| `Conflict` | RequiresAttention |
| `Partial` | RequiresAttention |
| `LocalPersistence` | RequiresAttention |
| `Internal` | RetryAfter with bounded budget (3 attempts then RequiresAttention) |

`WaitForConfigurationChange` clears the deferred disposition when the user updates sync settings or credentials. `RequiresAttention` surfaces via `snp doctor` and is not retried automatically. `Internal` retries are bounded to 3 attempts before escalating to `RequiresAttention`, preventing infinite loops on persistent internal errors.

## Durable Status

**File:** `~/.config/snp/auto-sync-status.toml`

A bounded, private, secret-free artifact that records the outcome of the most recent sync attempt and drives backoff/retry decisions. Unlike the pending marker (which tracks intent), status tracks *results*.

```toml
pending_generation = 1
last_attempt_generation = 1
last_attempt_at_unix_ms = 1700000000000
last_success_at_unix_ms = 1700000000000
last_result = "success"
last_failure_class = "none"
consecutive_failures = 0
next_attempt_at_unix_ms = 0
executor_exit_code = 0
attention_required = false
message = ""
config_fingerprint = 0
integrity = "crc32:441c462e"
```

**Schema fields:**

- `pending_generation` — generation of the pending marker at last attempt.
- `last_attempt_generation` — generation that was actually synced.
- `last_attempt_at_unix_ms` — wall-clock time of the last attempt.
- `last_success_at_unix_ms` — wall-clock time of the last successful sync.
- `last_result` — `"success"` or `"failed"`.
- `last_failure_class` — the `FailureClass` variant name (e.g. `"TransientNetwork"`, `"Internal"`), or `"none"` on success.
- `consecutive_failures` — count of back-to-back failures; resets to 0 on success.
- `next_attempt_at_unix_ms` — earliest time the next attempt may be scheduled (backoff window). Zero means no deferral.
- `executor_exit_code` — raw exit code from the last executor run.
- `attention_required` — `true` when the failure class maps to `RequiresAttention`.
- `message` — human-readable summary of the last outcome (truncated, secrets redacted).
- `config_fingerprint` — hash of non-secret structural config (server URL, enabled flags, direction, API key presence). Used by `release_deferral_on_config_change()` to detect when credential/config changes should release deferred failures. Zero means no fingerprint recorded.
- `integrity` — `crc32:<hex>` over all behavior-driving fields; rejects tampered or corrupted files.

**Invariants:**

- Written atomically via unique temp file + rename + fsync + directory fsync.
- Status write failure must **not** clear pending — a write failure leaves the existing status file intact and does not affect the pending marker.
- No command text, API keys, encryption keys, or raw server responses are stored. Messages are sanitized: control characters stripped, Bearer tokens and API key values redacted.
- 0o600 permissions on Unix.

## Schedule Decision

The `schedule_sync()` function is the **sole** entry point for all worker scheduling decisions. It prevents worker storms by evaluating whether a new worker should be spawned, deferred, or skipped entirely. Every code path that wants to spawn a worker must go through this function — there are no per-mutation spawn paths.

```rust
pub enum ScheduleDecision {
    /// Conditions are met — spawn a worker immediately.
    SpawnNow,
    /// A worker is already active (execution lock held) — skip.
    AlreadyActive,
    /// Backoff window has not expired — defer until `next_attempt_at_unix_ms`.
    DeferredUntil(u64),
    /// Auto-sync is disabled — do not schedule.
    Disabled,
    /// Failure class requires user attention — do not schedule.
    RequiresAttention,
    /// No pending work exists — nothing to do.
    NoPending,
    /// Sync is not configured (no account) — do not schedule.
    NotConfigured,
}
```

**Checks performed (in order):**

1. Pending marker exists and contains valid work.
2. Policy is enabled (`settings.auto_sync && settings.enabled`).
3. Execution lock is not held (`try_acquire` or inspection).
4. Backoff status: `next_attempt_at_unix_ms` in status file has elapsed.
5. Failure class retry allowance: `Internal` failures have a 3-attempt budget; `RequiresAttention` failures are not retried automatically.
6. Config-change detection: if `RequiresAttention` is due to `Authentication`, `CredentialStore`, or `Configuration`, check if the config fingerprint has changed since the failure. If so, release the deferral and allow a new attempt.

Only `SpawnNow` invokes the process spawner (`spawn::spawn_worker`). All other variants are terminal for that scheduling call — no process is created. This function is called from notification handlers, startup recovery, and any path that previously called `schedule_existing_pending` directly.

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

### Clock Trait for Testability

The debounce and worker logic depends on wall-clock time (`Instant::now`, `thread::sleep`). To enable deterministic testing without real sleeps, the implementation uses a `Clock` trait that abstracts time sources. Production code uses a `RealClock`; tests inject a `ManualClock` that advances time programmatically. This avoids flaky time-dependent tests and allows precise control over debounce deadlines and max-delay boundaries in unit tests.

### Sync Target: Global

`run_default_sync` syncs all configured libraries. The `MutationContext::library_id` field is retained for forward compatibility but currently unused.

### Delivery Guarantees: Best-Effort

Auto-sync is convenience, not durable delivery. The durable pending marker survives crash/restart, and `startup_recover_pending` always schedules a worker for valid pending work regardless of age. Manual `snp sync` and `snp cron` remain the recovery path for missed syncs. Pending intent is preserved even when auto-sync execution is disabled — re-enabling auto-sync or running manual sync recovers accumulated work.

## Phase 02: Debounce and Max-Delay Semantics

### DebounceResult

The `debounce()` function returns a `DebounceResult` enum:

```rust
pub enum DebounceResult {
    /// Quiet period elapsed with no new generations — safe to sync
    /// the observed generation and snapshot.
    Ready { generation: u64, snapshot: PendingSnapshot },
    /// max_delay elapsed while changes were still arriving. Returns
    /// the latest observed state so the caller can sync it.
    MaxDelayReached { generation: u64, snapshot: PendingSnapshot },
    /// No pending work found (marker cleared or missing).
    NothingToDo,
}
```

`Ready` fires when the quiet period (`debounce`) elapses without a new generation appearing. `MaxDelayReached` fires when the total elapsed time hits `max_delay`, even if changes continue to arrive — this prevents indefinite starvation under continuous mutations.

### Separate debounce vs max_delay

- **`debounce`** (quiet period): how long to wait after the *last* observed change before syncing. Resets every time a newer generation appears during the wait.
- **`max_delay`** (bounded latency): absolute upper bound on total debounce time. When hit, the worker syncs the latest observed state immediately, regardless of whether the quiet period has elapsed.

These are configured independently: `snp sync config --debounce 5 --max-delay 60`.

### Preflight check before executor spawn

Between debounce completion and executor spawn, the worker performs a preflight check:

1. Re-reads the pending marker to confirm no newer generation appeared during the debounce-to-spawn window.
2. If a newer generation is found, the worker loops back to debounce rather than spawning a stale executor.
3. This closes a race condition where a mutation could arrive in the tiny window between debounce returning `Ready` and the executor subprocess starting.

### Clock trait for deterministic testing

All time-dependent operations (`Instant::now`, `thread::sleep`) go through a `Clock` trait:

```rust
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}
```

Production uses `RealClock`; tests inject `ManualClock` to advance time without real sleeps. This enables deterministic testing of debounce deadlines, max-delay boundaries, and worker lifecycle timing.

### Process Detachment

- Unix: `libc::setsid()` puts the worker in a new session, ensuring it does not die when the parent exits.
- Windows: `DETACHED_PROCESS | CREATE_NO_WINDOW` flags on `CreateProcess`.
- All file descriptors are released; stdin/stdout/stderr are routed to `null` so the worker cannot interfere with a TTY.

## Pending Discard (Phase 04A)

The `snp sync discard-pending` command allows users to abandon synchronization intent. This is distinct from disabling auto-sync — disabled policy preserves pending intent; discard deliberately removes it.

```bash
snp sync discard-pending [--force] [--generation <N>]
```

1. Reads the current pending generation.
2. Requires confirmation unless `--force` is passed (non-interactive contexts require `--force`).
3. Calls `clear_if_generation_matches` with the observed generation.
4. Refuses if the generation changed during confirmation (requires retry).
5. Exits with code 2 on generation mismatch, code 3 on corruption/inaccessibility.

This operation must never delete local snippet data — it only removes the pending synchronization marker.

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
- Liveness probe uses `lock::process_alive(pid)` which calls `kill -0` on Unix and `GetExitCodeProcess` API checks on Windows (not a placeholder).

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
12. Cross-process safety: stale locks reclaimed, dead processes detected via `kill -0` (Unix) or `GetExitCodeProcess` (Windows), no permanent deadlock.
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
25. **Phase 02:** Startup recovery never increments generation and schedules a worker for valid pending work regardless of age — unless the execution lock is already held (active sync in progress), in which case scheduling is skipped.
26. **Phase 02:** Debounce returns the latest observed state, not the first state seen.
27. **Phase 02:** Pre-executor preflight check closes the race between debounce completion and executor spawn.
28. **Phase 02:** Disabled auto-sync execution does not silently erase synchronization intent — pending marker is created when sync is configured, regardless of auto-sync setting.
29. **Phase 03:** Status write failure must never clear pending — a write failure leaves the existing status file intact and does not affect the pending marker.
30. **Phase 03:** Backoff is persisted across CLI process restarts via `auto-sync-status.toml` — deferred disposition survives process death and system reboot.
31. **Phase 03:** New mutations do not spawn per-mutation workers — debounce + centralized `schedule_sync()` decision prevent worker storms.

## Status Snapshot (Phase 04A)

The `snp status` command provides a read-only view of auto-sync state via the `StatusSnapshot` module (`src/status_snapshot.rs`). It aggregates pending markers, execution locks, status persistence, and sync configuration into a single structured output.

**Key properties:**
- Strictly read-only — never mutates any artifact.
- `TopLevelSyncState` is derived via `derive_top_level()` with a defined precedence order (corrupt → live execution → pending states → configured states).
- JSON output uses schema version 1 with serde's externally-tagged enum representation.
- Human output is conditionally formatted based on top-level state.
- Diagnostics are severity-sorted (Info < Warning < Error) with machine-readable codes.

See [status.md](status.md) for the full deep-dive.

## Recovery Commands (Phase 04A)

### `snp sync retry`

Force an immediate sync attempt, bypassing backoff and schedule decisions. Acquires the execution lock, runs sync, records outcome, and clears pending on success.

```bash
snp sync retry [--library <name>]
```

**Difference from `snp sync`:** `retry` captures and clears the pending marker (like explicit `--sync`), while `snp sync` is the general-purpose sync command. Both acquire the execution lock and serialize with auto-sync workers.

### `snp sync clear-failure`

Clear the failure state from `auto-sync-status.toml`, resetting `attention_required`, `consecutive_failures`, and `next_attempt_at_unix_ms`. Does not trigger a sync — only clears the disposition so the next scheduling attempt can proceed.

```bash
snp sync clear-failure
```

### `snp sync discard-pending`

Remove the pending sync marker, abandoning synchronization intent. See [Pending Discard](#pending-discard-phase-04a) above.

```bash
snp sync discard-pending [--force] [--generation <N>]
```

### `snp sync repair`

Diagnose and repair corrupt auto-sync state. Without `--apply`, lists detected issues as dry-run actions. With `--apply`, executes repairs: quarantines corrupt files, removes stale locks, deletes orphaned temp files, fixes permissions.

```bash
snp sync repair [--dry-run] [--apply]
```

**Repair actions:**
- Corrupt status file → quarantine to `.quarantine.{timestamp}/`, optionally recreate empty
- Stale execution/worker lock (dead PID) → quarantine and remove
- Stale pending transaction lock → quarantine and remove
- Orphaned temp files (`snp-sync-tmp.*`, `.quarantine.*`) → remove
- Incorrect permissions (not `0o600`) → fix on Unix

## Hidden Subcommands

`auto-sync-worker` and `auto-sync-execute` are registered with `hide = true` in the clap CLI — they do not appear in `--help` output and are intended only for internal use by the parent process.

- `auto-sync-worker` accepts `--state-dir <path>` and exits with `WorkerOutcome` mapped to internal exit codes (currently 0 for all outcomes — failures are logged, not propagated).
- `auto-sync-execute` accepts `--state-dir <path>` and performs the actual sync operation, exiting with `ExecutorExitCode` status codes.
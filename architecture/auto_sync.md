# Auto-Sync Architecture

[← Back to Overview](overview.md)

Auto-sync is an optional, disabled-by-default facility that runs a **detached
one-shot helper** after a successful local mutation. The parent command records
durable pending intent, decides whether a worker should run, and returns; the
helper owns the network work. This document covers the scheduling, locking,
pending-marker, and worker-cycle mechanics. It does **not** cover the canonical
sync protocol, merge, or conflict rules (see [sync.md](sync.md)), the read-only
status projection (see [status.md](status.md)), the transaction journal and
mutation gate (see [persistence.md](persistence.md)), or the kernel lock
primitive itself (see [process_file_lock.md](process_file_lock.md)).

## Runtime model

```text
mutation command (parent)
  -> local atomic commit
  -> notify_local_mutation(kind, origin)
       -> pending::record_pending_mutation(state_dir, snapshot)  // generation G+1
       -> schedule::schedule_and_spawn(state_dir, policy, Caller::Mutation)
            -> schedule_sync(): SpawnNow / DeferredUntil / RequiresAttention /
                               NoPending / Disabled / NotConfigured
            -> spawn_worker_if_needed() -> execution_lock::spawn_worker(state_dir)
                 -> re-exec current binary as `auto-sync-worker --state-dir <dir>`
                 -> setsid() (Unix) / DETACHED_PROCESS|CREATE_NO_WINDOW (Windows)
                 -> stdin/stdout/stderr -> null
  -> return Scheduled{generation} immediately

snp auto-sync-worker (child, detached, one-shot)
  -> resolve AutoSyncPolicy from live sync settings
  -> execution_lock::try_acquire(state_dir)   // AlreadyHeld -> NothingToDo
  -> run_locked(): holds the execution lock for the WHOLE cycle
       -> read pending marker      // read error -> record_failure, not a no-op
       -> debounce                 // <= 250 ms slices, re-read each slice
       -> preflight_check()        // typed PreflightError
       -> execute_sync(): run_sync_with_limits(.., Some(SyncRunLimits)) directly
       -> clear_if_generation_matches(G) on success / record_failure(G) on error
       -> follow-up cycle only on Success with a strictly newer generation
  -> release lock (RAII Drop), exit(0)
```

There is no daemon, queue database, IPC channel, or service-manager
integration. The helper is opportunistic and bounded by both the worker
lifetime (`max_lifetime`) and `auto_sync_timeout_seconds` per attempt. The sync
client caps requests, retries, and retry sleeps by the remaining attempt
deadline; local filesystem operations are not force-cancelled.

## Module layout (`src/auto_sync/`)

Eleven files — ten submodules declared at `src/auto_sync/mod.rs:5-14` plus
`mod.rs` itself:

| Module | Responsibility (one line) |
|--------|---------------------------|
| `mod.rs` | Module wiring, re-exports, and the `paths` module of stable path helpers for doctor/diagnostics (`src/auto_sync/mod.rs:28-53`) |
| `policy.rs` | `AutoSyncPolicy`, `MutationKind`/`MutationOrigin`, `RetryDisposition`, `transient_backoff`; re-exports `FailureClass` from `sync_failure.rs` |
| `pending.rs` | Durable pending marker: `PendingState`/`PendingSnapshot`, schema-v2 TOML + CRC32, generation increments, conditional clear, `PendingError` |
| `pending_lock.rs` | `PendingTxnGuard` — short-lived mutex serializing pending-marker read-modify-write sections |
| `execution_lock.rs` | `SyncExecutionLock` and `WorkerLock` wrappers over the kernel lock, plus `spawn_worker()` and platform detachment |
| `lock.rs` | Re-export shim over the `execution_lock` worker-lock types (backward compatibility) |
| `schedule.rs` | `schedule_sync()` — the sole scheduling authority; `ScheduleDecision`, `ScheduleError`, `Caller`, `schedule_and_spawn` / `schedule_existing_pending` |
| `notification.rs` | Parent-side `notify_mutation` / `notify_local_mutation`, explicit-sync helpers, `StartupRecoveryPolicy` |
| `worker.rs` | Detached helper cycle: debounce, typed preflight, direct canonical sync, exact-generation clear, bounded follow-up loop |
| `status.rs` | Durable `AutoSyncStatus`: attempt result, backoff, attention flag, secret-free config fingerprint |
| `test_events.rs` | Test-only JSON-lines lifecycle events (compile-time no-op in production builds) |

Hidden command: `auto-sync-worker --state-dir <path>` (`WORKER_SUBCOMMAND`) is
internal, hidden from help, and suppressed from startup-recovery recursion
(`StartupRecoveryPolicy::SuppressInternal`).

## Policy (`policy.rs`, `config/sync_settings.rs`)

```rust
pub struct AutoSyncPolicy {
    pub sync_configured: bool,  // settings.enabled
    pub enabled: bool,          // settings.auto_sync && settings.enabled
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub sync_timeout: Duration,
    pub max_lifetime: Duration, // sole pre-sync debounce window cap
}
```

Resolved once per invocation via `AutoSyncPolicy::resolve(&settings)`
(`src/auto_sync/policy.rs:39-53`). `should_trigger()` returns `enabled`
(`:50-52`). Defaults: disabled, 2 s debounce, `Warn`, 30 s timeout, 300 s
max-lifetime (`src/auto_sync/policy.rs:55-66`). Clamps: debounce 0–300 s
(`MAX_DEBOUNCE_SECS`, `src/auto_sync/policy.rs:6`), max-delay 0–600 s, timeout
5–120 s. Malformed values clamp — they never disable sync.

`AutoSyncFailureMode` (`Ignore` / `Warn` default / `Error`) only controls
parent-side scheduling-failure rendering. It never rolls back the local
commit and never changes the exit code — the mode selects only the `tracing`
level of the policy log line (`src/auto_sync/notification.rs:242-254`).

### Mutation taxonomy

```rust
pub enum MutationKind {  // 9 variants
    SnippetCreate, SnippetUpdate, SnippetDelete, SnippetRun,
    Import, LibraryChange, PremadeInstall, SyncConflictWrite,
    AccountConfig,  // is_syncable_mutation() == false; never triggers
}
pub enum MutationOrigin { User, Import, SyncMerge, Recovery }
// SyncMerge.should_suppress() == true — sync-origin writes never recurse.
```

`is_syncable_mutation()` excludes only `AccountConfig`
(`src/auto_sync/policy.rs:84-87`); `MutationOrigin::should_suppress()` matches
only `SyncMerge` (`:98-102`).

**Produced variants.** Only five `MutationKind` values have a production call
site: `SnippetCreate` (`new_cmd.rs:650`), `SnippetUpdate` (`edit_cmd.rs:107`
and `:130`), `SnippetDelete` (`commands/mod.rs:444`), `Import`
(`import_cmd.rs:473`, `restore_cmd.rs:1657`, `transaction.rs:919`), and
`LibraryChange` (`library_cmd.rs:29`, `:64`). `SnippetRun`, `PremadeInstall`,
and `SyncConflictWrite` have **no producer** — they exist so the taxonomy stays
stable, and `SyncConflictWrite`/`SyncMerge` suppression is a backstop rather
than the primary loop guard (the sync merge path never notifies at all).

### Failure classes and retry disposition

`FailureClass` (4 variants) is defined in `src/sync_failure.rs:24-33` and
re-exported unchanged by `policy.rs` (`src/auto_sync/policy.rs:107`).

| Class | `as_code()` | `is_deferred()` | Automatic retry |
|-------|-------------|-----------------|-----------------|
| `Transient` | `transient` | no | yes, exponential backoff |
| `Configuration` | `configuration` | yes | no — waits for a config-fingerprint change |
| `LocalFailure` | `local_failure` | no | no — requires operator attention |
| `Internal` | `internal` | no | yes while `consecutive_failures < 3`, then attention |

`from_code` additionally accepts legacy codes (`deferred_disabled`,
`deferred_not_configured`, `authentication`, `credential_store` → Configuration;
`transient_network`, `transient_timeout`, `partial` → Transient; `conflict`,
`local_persistence` → LocalFailure) and falls back to `Internal`
(`src/sync_failure.rs:130-145`).

`RetryDisposition` (`src/auto_sync/policy.rs:116-125`) is derived by
`FailureClass::retry_disposition(consecutive_failures)` (`:128-143`):
`RetryAfter(Duration)` (Transient; Internal while count < 3),
`WaitForConfigurationChange` (Configuration), `RequiresAttention`
(LocalFailure; Internal at count ≥ 3), `NoAutomaticRetry`. Note that
`NoAutomaticRetry` is **declared but never returned** by `retry_disposition` —
it exists for the "only an explicit `snp sync` can retry" semantics and is
matched defensively by the scheduler (`src/auto_sync/schedule.rs:101-103`).

Backoff schedule (`transient_backoff`, `src/auto_sync/policy.rs:146-158`):
5 s → 15 s → 30 s → 60 s → exponential, capped at 15 min, plus 0–20 % jitter.

Retry behaviour is driven by durable backoff state in `auto-sync-status.toml` —
distinct from `SyncRetryConfig.max_retries`, which controls per-RPC gRPC
attempts inside one sync operation.

## Durable pending state (`pending.rs`, `pending_lock.rs`)

`auto-sync-pending.toml` (`PENDING_FILE_NAME`, `src/auto_sync/pending.rs:8`),
schema v2 (`SCHEMA_VERSION`, `:9`):

```toml
schema = 2
generation = 7
created_at_unix_ms = 1700000000000

[snapshot.Mutation]
kind = "snippet_create"

integrity = "crc32:441c462e"
```

- `integrity` is `crc32:<hex>` over schema + generation + creation timestamp +
  serialized snapshot (+ `source_transaction_id` when set). v1 markers
  (`kind`, `created_at_unix_ms`) migrate transparently on read
  (`src/auto_sync/pending.rs:182-215`).
- Atomic write: unique temp file (PID + nanoseconds) + rename + directory
  fsync (`atomic_write_bytes`, `DurableUserData`); 0o600 on Unix.
- No secrets, commands, or snippet content — only the mutation kind.
- `record_pending_mutation` is the **only** API that increments the
  generation (`src/auto_sync/pending.rs:73-94`); scheduling paths must never
  mutate it. `ensure_pending_for_transaction` gives restore exactly-once
  semantics — `Created` / `Reused` without increment / `Conflict` preserving
  newer work (`src/auto_sync/pending.rs:127-163`).
- `clear_if_generation_matches` is the automatic acknowledgement boundary:
  read-compare-delete under `PendingTxnGuard`. `Cleared` on equality,
  `GenerationChanged{current}` preserves newer work, `Missing` (already
  cleared) is success (`src/auto_sync/pending.rs:237-259`). Clear errors
  preserve recoverability and record failure. `record_success` wraps the same
  boundary; the pending-side `record_failure` only logs and preserves intent
  (`src/auto_sync/pending.rs:272-303`).
- `PendingError` variants: `Io`, `Serialize`, `Deserialize`, `IntegrityMismatch`,
  `NotFound`, `Lock`, `Corrupted`, `Scheduling` (`src/auto_sync/pending.rs:330-344`).
  `Scheduling` is explicitly *not* data corruption — the marker is preserved
  for retry.

### Generation rules

Generations are **monotonic**. A lower generation normally means corrupt
rollback: fail closed, preserve the marker, spawn nothing. The single
exception is a cleared-and-recreated marker — observed as generation exactly
`1` with a strictly **newer** `created_at_unix_ms` after an explicit sync
cleared the old marker:

```rust
fn adopt_generation_reset(current: &PendingState, latest: PendingState) -> Option<PendingState> {
    let is_reset = latest.generation == 1 && latest.created_at_unix_ms > current.created_at_unix_ms;
    if is_reset {
        tracing::info!(
            observed_generation = current.generation,
            recreated_generation = latest.generation,
            "pending marker was cleared and recreated during debounce; adopting new generation"
        );
        Some(latest)
    } else {
        None
    }
}
```

(`src/auto_sync/worker.rs:202-214`). It is applied in exactly three places: the
debounce loop (both the deadline-reached and the mid-sleep re-read branches,
`src/auto_sync/worker.rs:243-262` and `:287-308`), the preflight
(`src/auto_sync/worker.rs:378-387`), and the post-sync follow-up check
(`src/auto_sync/worker.rs:151-161`). Any other lower generation — and any
lower generation with an equal-or-older timestamp — stays corrupt.

`STALE_PENDING_THRESHOLD_MS` (formerly 5 minutes) was **removed**: pending
intent is no longer discarded by age, and `startup_recover`
(`src/auto_sync/worker.rs:567-584`) reschedules whatever marker it finds.

### Pending transaction lock

`pending_lock.rs` (`auto-sync-pending.lock`, `PENDING_TXN_LOCK_NAME`,
`src/auto_sync/pending_lock.rs:17`) serializes concurrent CLI processes for the
minimum read-modify-write section only (10 s timeout,
`PENDING_TXN_LOCK_TIMEOUT`, `src/auto_sync/pending.rs:17`): kernel-authoritative
`flock`/`LockFileEx` via `process_file_lock`, bounded retry polling the kernel
lock every 100 ms, live owners never reclaimed, `Drop` releases without
unlinking, 0o600. Distinct from the long-lived execution lock below.

## Scheduling (`schedule.rs`)

`schedule_sync(state_dir, policy, caller)` (`src/auto_sync/schedule.rs:62-132`)
is the **sole scheduling authority** — startup recovery, post-mutation, and
explicit retry all go through it.

```rust
pub enum ScheduleDecision {
    SpawnNow,
    DeferredUntil(u64),
    Disabled,
    RequiresAttention(FailureClass),
    NoPending,
    NotConfigured,
}
pub enum ScheduleError {
    Pending(pending::PendingError),
    Spawn(execution_lock::SpawnError),
}
```

Decision order inside `schedule_sync`:

1. `!policy.sync_configured` → `NotConfigured` (`schedule.rs:67-69`).
2. `!policy.should_trigger()` → `Disabled` (`:71-73`).
3. Pending marker missing → `NoPending`; **any other pending read error →
   `Err(ScheduleError::Pending(..))`** (`:76-80`). A corrupt marker is never
   collapsed into `NoPending`/`SpawnNow`/success.
4. Everything below is guarded by `caller != Caller::ExplicitRetry`
   (`schedule.rs:83`) — an explicit retry bypasses the backoff window, the
   attention check, and the corrupt-status check alike.
   1. Backoff active (`next_attempt_at_unix_ms` in the future) →
      `DeferredUntil(t)` (`:84-93`).
   2. `attention_required && consecutive_failures > 0` → the last failure
      class's `retry_disposition` decides (`:98-122`). `RequiresAttention` /
      `NoAutomaticRetry` / `WaitForConfigurationChange` produce
      `RequiresAttention(class)` — unless the class is deferred *and* the
      config fingerprint changed, which releases the deferral and falls through
      to `SpawnNow` (`:104-115`). `RetryAfter` falls through to `SpawnNow`
      (the worker re-reads the backoff window itself).
   3. A corrupt status file → `RequiresAttention(FailureClass::Internal)`
      (`:124-126`).
5. Otherwise → `SpawnNow` (`:131`).

**Execution-lock nuance.** The scheduler does not *block* on the execution
lock, but it does *probe* it in one case: releasing a config-change deferral
calls `status::release_deferral_on_config_change`
(`schedule.rs:108-111`), which does a **non-blocking `try_acquire`** of the
execution lock before rewriting the status file (`src/auto_sync/status.rs:339`);
if a worker owns it, the release is skipped and the deferral stands. This is
the only execution-lock interaction on the scheduling path, and it never
competes with a running worker.

`Caller::{StartupRecovery, Mutation, ExplicitRetry}` selects logging and
whether backoff may be bypassed (explicit retry only). `schedule_and_spawn`
(`schedule.rs:163-171`) and `schedule_existing_pending` (`:150-158`) are the
only translators of `SpawnNow` into a spawn, and both funnel through
`spawn_worker_if_needed` (`:173-187`) — the single production call site of
`execution_lock::spawn_worker` (`schedule.rs:181`; the only other reference in
`src/` is the definition at `src/auto_sync/execution_lock.rs:614`). A
structural unit test pins that count, though it only scans `schedule.rs`
itself, so a call added elsewhere in the workspace would not be caught by it
(`schedule.rs:476-504`). `schedule_existing_pending` schedules already-recorded
intent (e.g. restore) without touching the generation. Under `test-support`,
`SNP_SKIP_WORKER_SPAWN` suppresses the spawn (production: never checked —
`src/auto_sync/schedule.rs:189-205`).

## Worker (`worker.rs`, `execution_lock.rs`)

`run(state_dir)` (`src/auto_sync/worker.rs:61-73`): resolve policy → `try_acquire`
(`AlreadyHeld` → `NothingToDo`, preserving pending) → `run_locked` loop.

`run_locked` (`src/auto_sync/worker.rs:75-173`) holds the execution lock for
the **entire** cycle — debounce, sync, and follow-up — via `let _lock = lock`
(`:76`); it is released by RAII `Drop` when the function returns.

1. **Lifetime cap.** `start.elapsed() >= policy.max_lifetime` → `NothingToDo`
   (`src/auto_sync/worker.rs:84-86`).
2. **Read pending.** `read_state_from_dir` → `NotFound` → `NothingToDo`.
   **Any other read error is a recorded failure**, classified by
   `failure_class_for_pending_error` and attributed to
   `UNKNOWN_PENDING_GENERATION` (`0`), because the marker's own generation is
   unknown (`src/auto_sync/worker.rs:87-104`, `:371`). This is the path that
   was previously mis-reported as a no-op: a temporarily unreadable marker
   (NFS hiccup, `EACCES` during a concurrent rename) now increments
   `consecutive_failures` and engages backoff instead of letting the next
   mutation re-spawn immediately.
3. **Debounce.** Deadline = observed `created_at + debounce`, capped by
   `start + max_lifetime` (`compute_deadline`, `src/auto_sync/worker.rs:179-191`).
   Sleeps in ≤ 250 ms slices, re-reading the marker each time; newer
   generations restart the deadline, marker removal cancels, corruption fails
   the cycle as `Internal`. The cleared-and-recreated reset is adopted as a
   fresh full window (`src/auto_sync/worker.rs:216-314`).
4. **Preflight.** Re-reads the marker.

   ```rust
   pub enum PreflightError {
       NothingToDo,
       Failed { class: FailureClass, message: String },
   }
   ```

   (`src/auto_sync/worker.rs:323-332`), returned by
   `preflight_check(state_dir, observed) -> Result<PendingState, PreflightError>`
   (`:373-395`). The typed error keeps three semantically different conditions
   apart:

   | Condition | Result |
   |-----------|--------|
   | Marker removed between debounce and preflight | `NothingToDo` → exit 0 |
   | Lower generation that is the cleared-and-recreated case | `Ok(recreated)` — adopted as the new target, the cycle continues |
   | Lower generation that cannot be adopted | `Failed { class: Internal }` |
   | Marker unreadable or corrupt | `Failed { class }` — `Transient` for `Io`/`Lock`, `LocalFailure` for `Deserialize`/`IntegrityMismatch`/`Corrupted` (`src/auto_sync/worker.rs:348-365`) |

   **Both failure shapes go through `record_sync_failure`** — status file,
   backoff, `sync_completed` event, nonzero exit — so corruption can never be
   reported as a no-op cycle (`src/auto_sync/worker.rs:131-144`). Before
   commit `8d5c209` all three conditions were collapsed into `NothingToDo`
   (exit 0): corruption never incremented `consecutive_failures`, never
   engaged backoff, and was invisible in `snp status`.
5. **Execute.** Honours configured direction, builds a `new_current_thread`
   runtime (the client keeps `rt-multi-thread`), and calls
   `crate::sync_commands::run_sync_with_limits(&settings, None, push_only,
   pull_only, &runtime, Some(crate::sync::SyncRunLimits { deadline,
   request_timeout }))` directly — never a child sync process
   (`src/auto_sync/worker.rs:409-454`). A runtime-construction failure is
   recorded as `Internal` (`:431-438`).
6. **Acknowledge.** Success → `clear_if_generation_matches(observed)`:
   `Cleared` / `GenerationChanged` (newer work preserved) / `Missing`
   (already cleared) all record success; a clear I/O failure records
   `LocalFailure` (`src/auto_sync/worker.rs:455-500`).
7. **Follow-up.** Only `Success` loops (`follow_up_allowed`,
   `src/auto_sync/worker.rs:175-177`). The loop re-reads the marker: a strictly
   newer generation continues; an adoptable reset continues; an unadoptable
   rollback records `Internal`; a reload failure records `Internal`
   (`src/auto_sync/worker.rs:145-171`).

### Failure recording and backoff

`record_sync_failure` (`src/auto_sync/worker.rs:510-532`) is the single
failure sink for the worker. It:

1. computes `consecutive = next_consecutive_failures(state_dir)` — the durable
   count from the status file, `+1`, or `1` when the status is missing/corrupt
   (`src/auto_sync/worker.rs:544-549`);
2. computes `backoff = transient_backoff(consecutive)`;
3. calls `status::record_failure(state_dir, generation, class, -1, consecutive,
   now + backoff, message, config_fingerprint)` — the `-1` exit code means
   "detached helper, no executor process";
4. emits the `sync_completed` test event with `success: false`;
5. returns `WorkerOutcome::Failed`.

A status write failure is logged and never clears pending. A failure never
clears the pending marker either — the marker is the durable intent, and the
status file carries the retry state.

## Locks: what each one guards

| Lock | File | Acquired by | Guards | Acquisition mode |
|------|------|-------------|--------|------------------|
| `SyncExecutionLock` | `auto-sync-execution.lock` | The detached worker (whole cycle); manual `snp sync`, explicit `--sync`, cron | All actual sync work — no two syncs can run concurrently | Worker: `try_acquire` (`AlreadyHeld` → cheap exit, `src/auto_sync/execution_lock.rs:143-147`). Foreground: `wait_acquire`, 30 s default (`:152-159`, timeout from `:24`) |
| `WorkerLock` (legacy) | `auto-sync-worker.lock` | **No production path acquires it** | Nothing at runtime — inspection/diagnostics only | `try_acquire_worker` (`src/auto_sync/execution_lock.rs:277-281`) and `wait_acquire_worker` (`:291-298`) exist but are reached only through the `lock.rs` re-export shim (`src/auto_sync/lock.rs:3-7`); the only live consumers are the status snapshot (`src/status_snapshot.rs:310-330`) and the `snp sync repair` permission-hardening path (`src/commands/sync_cmd.rs:900-902`) |
| `PendingTxnGuard` | `auto-sync-pending.lock` | Parent mutation commands and the worker | The pending marker's read-modify-write section only | `wait_acquire`, 10 s timeout (`src/auto_sync/pending_lock.rs:91-98`, `src/auto_sync/pending.rs:17`) |

### Acquisition order

The order is fixed and never inverted:

1. **Parent mutation path**: local commit → `LocalDataLock`/transaction gate
   (see [persistence.md](persistence.md)) → *then* `PendingTxnGuard` inside
   `record_pending_mutation` → then release → then `schedule_and_spawn`.
   The parent **never** holds the execution lock.
2. **Worker path**: `SyncExecutionLock` (via `try_acquire`) is taken **first**,
   before any pending-marker read; the `PendingTxnGuard` is taken *inside*
   each pending read-modify-write section while the execution lock is held.
   The worker therefore holds the execution lock across its pending-marker
   critical sections, but never holds the pending lock while waiting for the
   execution lock.
3. **Explicit sync path**: observe pending generation → `wait_acquire`
   (execution lock) → sync → generation-scoped clear (which takes the
   `PendingTxnGuard` inside `clear_if_generation_matches`).

No path acquires the pending lock and then the execution lock. The worker
lock is never part of any of these orders because nothing acquires it.

### Kernel authority

Mutual exclusion comes from the kernel (`flock` Unix / `LockFileEx` Windows
via `process_file_lock`); on-disk TOML (`pid`, `started_at`, `nonce`,
`purpose`) is diagnostic only and may be stale. Stale metadata is overwritten
by the next acquirer — no inspect-then-rename race. `Drop` releases the kernel
lock and never unlinks, so the lock file persists and may hold a dead owner's
metadata (`src/process_file_lock.rs:190-200`). Liveness is `kill(pid, 0)`:
only `ESRCH` proves absence (`EPERM`/unknown = live)
(`src/utils/process.rs:14-25`); Linux start tokens use `/proc/<pid>/stat`
field 22. Non-Unix treats unknown PIDs as alive (conservative non-stealing).

## Detachment

`spawn_worker` re-execs `current_exe` with `auto-sync-worker --state-dir <dir>`
(`src/auto_sync/execution_lock.rs:614-641`); stdio is nulled (stderr optionally
appended to `SNP_AUTO_SYNC_WORKER_LOG` for debugging). Unix detaches with
`libc::setsid()` in `pre_exec` (new session, no controlling terminal, outlives
the parent); Windows uses `DETACHED_PROCESS | CREATE_NO_WINDOW`. Parent
scheduling failures never change the exit code — the local commit stays
successful. `failure_mode` selects only the `tracing` level of the policy log
line (`Ignore`: debug, `Warn`: warn, `Error`: error,
`src/auto_sync/notification.rs:242-254`); a `warning:` on stderr is printed
unconditionally for a scheduling failure
(`src/auto_sync/notification.rs:43-55`). Worker-side failures go to logs and
`doctor`, never the departed parent's stderr.

## Budgets

| Budget | Setting | Default / clamp | Meaning |
|--------|---------|-----------------|---------|
| Debounce (quiet period) | `auto_sync_debounce_seconds` | 2 s, clamped 0–300 (`AUTO_SYNC_DEBOUNCE_MIN`/`MAX`, `src/config/sync_settings.rs:27-29`) | Coalesces rapid mutations |
| Max delay (starvation cap) | `auto_sync_max_delay_seconds` | 300 s, clamped 0–600 (`src/config/sync_settings.rs:31-33`) | Sole pre-sync window cap; worker exits at `start + max_lifetime` |
| Attempt timeout | `auto_sync_timeout_seconds` | 30 s, clamped 5–120 (`src/config/sync_settings.rs:35-39`) | Per-attempt network/retry budget (`SyncRunLimits`); expiry records `Transient`, preserves pending, never force-cancels local I/O |

Clamping lives in `SyncSettings::auto_sync_debounce` /
`auto_sync_max_delay` / `auto_sync_timeout`
(`src/config/sync_settings.rs:202-228`); a malformed value clamps rather than
disabling sync.

Manual sync and cron keep their existing (unbounded) timeout behaviour.

## Trigger matrix (`notification.rs` + call sites)

`notify_mutation(kind, origin)` resolves policy and calls
`notify_local_mutation` (`src/auto_sync/notification.rs:57-68`), which
delegates to `notify_local_mutation_with_dir` and performs one generation
increment and one scheduling attempt
(`src/auto_sync/notification.rs:83-133`). Results
(`AutoSyncNotificationResult`):

| Variant | Meaning |
|---------|---------|
| `Disabled` | No sync account configured **and** no `sync.toml` in the state dir (`notification.rs:88-95`). A broken-but-present `sync.toml` still records pending, so the mutation is not lost. |
| `Suppressed` | `SyncMerge` origin (`notification.rs:99-106`), or a spawn that was not performed because the schedule did not return `SpawnNow` (`:121`). |
| `PendingRecorded { generation: u64 }` | Generation is durable, but the policy does not trigger (`notification.rs:112-116`). |
| `Scheduled { generation: u64 }` | Worker spawned (`notification.rs:118-120`). |
| `SchedulingFailed { generation: Option<u64> }` | Pending preserved, local commit intact (`notification.rs:122-124`, `:130`). `None` means the pending marker itself could not be written; `Some(g)` means the marker is durable but the spawn failed. |

| Command | `MutationKind` / `Origin` | Triggers? |
|---------|---------------------------|-----------|
| `snp new` (all sources) | `SnippetCreate` / `User` | Yes, after atomic save (`new_cmd.rs:650`) |
| `snp edit` (editor, changed only) | `SnippetUpdate` / `User` | Yes, after the editor closes — including editor-failure-with-changes (`edit_cmd.rs:107`, `:130`) |
| `snp edit --output` / `--clear-output` (by filter or ID) | — | **No** (no notify call; `output` is local-only, nothing remote to sync) |
| TUI delete | `SnippetDelete` / `User` | Yes, after tombstone save (`commands/mod.rs:444`); with `--sync`, explicit sync instead |
| `snp import pet` (create / merge-changed / replace) | `Import` / `Import` | Yes, after the library file **and** config registration (`import_cmd.rs:473`) |
| `snp import pet` (dry-run, strict abort, parse failure, no-op merge) | — | **No** |
| `snp library create` / `snp library delete` | `LibraryChange` / `User` | Yes, after manager success (`library_cmd.rs:29`, `:64`) |
| `snp library set-primary` / `list` / `show` | — | **No** (local-only metadata / read-only) |
| `snp premade get` | — | **No** (local copy of remote data) |
| `snp sync` (manual) / `--sync` paths | — | Clears pending (observe → sync → generation-scoped clear), never schedules |
| Sync merge writes | — | **No** — the merge path never notifies at all. `MutationKind::SyncConflictWrite` and `MutationOrigin::SyncMerge` have **no production producer**; the origin suppression (`src/auto_sync/policy.rs:99-102`) is the backstop if a future call site adds one. |
| `AccountConfig` mutations | `AccountConfig` | **No** (`is_syncable_mutation() == false`) |
| Restore commit | `Import` | `ensure_pending_for_transaction` + `schedule_existing_pending` only — no new generation (`restore_cmd.rs:1653-1665`, `:1720-1728`) |
| Crash recovery of a committed-local transaction | `Import` | `ensure_pending_for_transaction` only (`transaction.rs:915-921`) |

Each command notifies strictly after its authoritative commit point (after
`save_library`, after the library file **and** config registration for import,
after manager success for libraries). Backup failure, dry-run, cancel,
failure, and no-op paths emit nothing; machine-facing stdout stays free of
sync diagnostics.

## Explicit `--sync` precedence (`commands/mod.rs`)

`run_explicit_sync(runtime)` — shared by `run`/`clip`/`search` and TUI delete
paths: observe pending generation **before** sync
(`notification::observe_pending_generation`,
`src/auto_sync/notification.rs:173-182`) → `wait_acquire` (execution lock,
30 s) → `run_default_sync` → `clear_pending_after_explicit_sync`
(generation-scoped, success-gated; `src/auto_sync/notification.rs:160-168`).
A mutation arriving mid-sync keeps its newer generation; no duplicate delayed
sync fires for the same generation. Manual `snp sync` follows the same
observe-then-clear shape in `sync_cmd.rs`.

## Doctor integration

`snp doctor` reads the same artifacts indirectly: it calls
`status_snapshot::capture_snapshot()` (which resolves them through
`auto_sync::paths` with `process_alive` liveness probes) and maps each
`StatusDiagnostic` to a dotted code via `map_snapshot_diagnostic`
(`src/commands/doctor_cmd.rs:70-134`):

- `compat.sync.checked` (Info) summary carrying the top-level sync state.
- `sync.config.load_failed` / `sync.config.not_configured` — configuration
  state (`CONFIG_LOAD_FAILED` is downgraded Error → Warning under
  `--compatibility`).
- `sync.pending.corrupt` / `sync.pending.inaccessible`.
- `sync.execution.{dead_stale,malformed}` and
  `sync.worker_lock.{dead_stale,malformed}` — lock inspection
  (`Inaccessible` maps to the same `malformed` code).
- `sync.status.corrupt` and `sync.attention.*` — the attention code is selected
  by **substring match on the diagnostic message**, which the snapshot builds as
  `"Sync requires attention: {last_failure_class}"`. Because
  `last_failure_class` is only ever one of `transient`, `configuration`,
  `local_failure`, `internal`, only **two** of the **six** distinct attention
  codes are reachable through this path: `sync.attention.configuration` (and
  the fallback for `transient` / `internal`) and `sync.attention.local_persistence`.
  `sync.attention.authentication`, `sync.attention.credential_store`,
  `sync.attention.conflict`, and `sync.attention.partial` are unreachable.
  See [status.md](status.md#diagnostic-codes) for the full mapping.

See [status.md](status.md) for the `snp status` projection over the same
artifacts.

## Durable status and test seams (`status.rs`, `test_events.rs`)

`auto-sync-status.toml` (`STATUS_FILE_NAME`, `src/auto_sync/status.rs:17`;
schema 1, CRC32, 0o600, ≤ 512-char message) records each attempt
independently of pending intent: generations, attempt/success timestamps,
result + `FailureClass` codes, consecutive failures,
`next_attempt_at_unix_ms`, exit code, `attention_required`, and a secret-free
config fingerprint. Status write failure never clears pending
(`src/auto_sync/status.rs:8-10`).

`test_events.rs` emits JSON-lines lifecycle events to
`$SNP_TEST_EVENTS_DIR/test-events.jsonl` only under the `test-support`
feature; production builds compile the emitter to a no-op and the env check is
absent from the binary (`src/auto_sync/test_events.rs:30-53`, `:100-109`).
Each line is `{schema, seq, component, event, pid, generation, at_unix_ms,
detail}` (`:70-79`). The worker emits `worker`/`started` on entry
(`src/auto_sync/worker.rs:63`), `worker`/`sync_started` before the sync
(`:413-419`), and `worker`/`sync_completed` after every attempt
(`:534-542`).

---

## Key Files

| File | Subject |
|------|---------|
| `src/auto_sync/mod.rs` | Module wiring, re-exports, `paths` helpers for doctor/diagnostics |
| `src/auto_sync/policy.rs` | Effective policy, mutation taxonomy, retry disposition, backoff schedule |
| `src/auto_sync/pending.rs` | Durable pending marker, generation rules, conditional clear |
| `src/auto_sync/pending_lock.rs` | Short-lived pending-marker mutex |
| `src/auto_sync/execution_lock.rs` | Execution/worker lock wrappers, `spawn_worker`, platform detachment |
| `src/auto_sync/lock.rs` | Backward-compat re-export shim for the worker lock |
| `src/auto_sync/schedule.rs` | Sole scheduling authority, typed decisions and errors |
| `src/auto_sync/notification.rs` | Parent-side notify API, explicit-sync helpers, startup-recovery policy |
| `src/auto_sync/worker.rs` | Detached helper cycle: debounce, typed preflight, sync, ack, follow-up |
| `src/auto_sync/status.rs` | Durable attempt status, backoff state, config fingerprint |
| `src/auto_sync/test_events.rs` | Test-only JSON-lines lifecycle events |
| `src/sync_failure.rs` | `FailureClass` taxonomy (sync-client layer) |
| `src/process_file_lock.rs` | Kernel-backed cross-process lock primitive |
| `src/status_snapshot.rs` | Status projection over the auto-sync artifacts, consumed by `snp status` and doctor |
| `src/commands/status_cmd.rs` | `snp status` CLI rendering of the snapshot |
| `src/commands/doctor_cmd.rs` | Snapshot → dotted diagnostic-code mapping |
| `.skills/transactions-and-auto-sync.md` | Skill-level invariants for this subsystem |

---

## Invariants

1. Disabled by default; `sync_configured` stays true while auto-sync is off so
   manual sync keeps working and pending intent survives re-enable.
2. **Local mutation commits before any remote work; a remote or scheduling
   failure never rolls back or corrupts the local commit.**
3. The parent never holds the execution lock. The scheduler never *blocks* on
   it; its single non-blocking probe is the config-change deferral release
   (`src/auto_sync/status.rs:339`).
4. `schedule_sync` is the sole scheduling authority and `spawn_worker_if_needed`
   is the sole spawn site — structurally pinned by a unit test that counts
   `spawn_worker` references in `schedule.rs` (`src/auto_sync/schedule.rs:476-504`;
   the test scans that one file, not the whole workspace).
5. Pending generations are monotonic; a lower generation fails closed except
   the cleared-and-recreated case (generation exactly `1` **and** a strictly
   newer `created_at_unix_ms`), which is adopted as new work.
6. `clear_if_generation_matches` is the only automatic acknowledgement
   boundary; stale clears preserve newer work; `Missing` is already-cleared
   success.
7. `SyncMerge` origin never triggers (no feedback loops).
8. Secrets, commands, and snippet content never enter pending markers, lock
   files, worker argv/env, status, or logs (secret-free fingerprint only).
9. Pending intent survives a crash; startup recovery reschedules existing
   intent rather than discarding it by age (no staleness threshold remains),
   and startup recovery is suppressed for read-only, explicit-sync,
   internal-worker, and configuration commands.
10. All sync operations (automatic, manual, explicit `--sync`, cron) share one
    `SyncExecutionLock`; the worker holds it for the full cycle.
11. The worker drives sync only through the canonical
    `run_sync_with_limits(.., Some(limits))` entry point; it never mutates
    libraries directly, and local I/O is never force-cancelled by a timeout.
12. `auto-sync-worker` adds no public CLI surface (hidden subcommand).
13. Scheduling errors stay typed (`Pending` vs `Spawn`); backoff/attention
    state is durable and config-change aware.
14. A failed cycle is a **recorded failure**, never a silent no-op: a corrupt
    or unreadable pending marker reaches `record_sync_failure`, increments
    `consecutive_failures`, and engages backoff. A corrupt *status* file
    degrades to a default status rather than blocking the attempt.
15. Tests assert exact notification counts, prove server-side effects, verify
    pending-clear ordering, and emit events only under `SNP_TEST_EVENTS_DIR`.

---

## Gotchas

- **Do not collapse a corrupt marker into `NothingToDo`.** `preflight_check`
  returns a typed `PreflightError` precisely so that "the marker is gone"
  (clean exit) stays distinct from "the marker is corrupt" and "the marker
  could not be read" (recorded failures). `src/auto_sync/worker.rs:316-333`.
- **The pending *read* error path is a failure path too**, not just preflight.
  `src/auto_sync/worker.rs:87-104`.
- **`record_pending_mutation` is the only generation incrementer.** Scheduling,
  spawning, and the worker must never bump or rewrite the marker.
  `src/auto_sync/pending.rs:63-94`.
- **The lower-generation exception is narrow.** Generation exactly `1` *and* a
  strictly newer creation timestamp. Any other lower generation — including
  generation `1` with an equal-or-older timestamp — is corruption.
  `src/auto_sync/worker.rs:202-214`.
- **The `WorkerLock` (`auto-sync-worker.lock`) is dead weight at runtime.**
  Nothing in production acquires it; only the status snapshot and the repair
  permission path touch it. Do not add an acquirer without changing the
  mutual-exclusion story in this document.
- **The execution lock is held for the whole worker cycle**, including the
  debounce sleep. A long debounce blocks foreground `snp sync` until its
  30 s `wait_acquire` timeout expires.
- **`Drop` releases the kernel lock but never unlinks the lock file.** A
  leftover lock file with a dead PID is normal, not corruption.
  `src/process_file_lock.rs:190-200`.
- **Only `ESRCH` proves a PID is gone.** `EPERM` and unknown errors mean the
  process is alive. `src/utils/process.rs:23-25`.
- **Status is informative, not authoritative.** It drives retry scheduling but
  is never the source of truth for whether pending work exists, and a status
  write failure never clears pending. `src/auto_sync/status.rs:8-10`.
- **`config_fingerprint` is a `DefaultHasher` digest**, so its numeric value
  changes across Rust toolchain releases. Only the comparison against the
  stored value is meaningful. `src/auto_sync/status.rs:319-329`.

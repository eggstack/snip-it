# Auto-Sync Architecture

[← Back to Overview](overview.md)

Auto-sync is optional and disabled by default. After a successful local
mutation, the command records durable pending intent and attempts to detach
one `snp auto-sync-worker` helper. The parent never waits for network work.
Canonical sync itself is documented in [sync.md](sync.md).

## Runtime model

```text
mutation command (parent)
  -> local atomic commit
  -> notify_local_mutation(kind, origin)
  -> pending::record_pending_mutation(state_dir, snapshot)  // generation G+1
  -> schedule::schedule_and_spawn(state_dir, policy, Caller::Mutation)
       -> schedule_sync(): SpawnNow / DeferredUntil / RequiresAttention /
                           NoPending / Disabled / NotConfigured
       -> execution_lock::spawn_worker(state_dir)           // sole spawn site
            -> re-exec current binary as `auto-sync-worker --state-dir <dir>`
            -> setsid() (Unix) / DETACHED_PROCESS|CREATE_NO_WINDOW (Windows)
            -> stdin/stdout/stderr -> null
  -> return Scheduled{generation} immediately

snp auto-sync-worker (child, detached, one-shot)
  -> resolve AutoSyncPolicy from live sync settings
  -> execution_lock::try_acquire(state_dir)                 // AlreadyHeld -> NothingToDo
  -> read pending marker -> debounce -> preflight
  -> execute_sync(): run_sync_with_limits(.., Some(SyncRunLimits)) directly
  -> clear_if_generation_matches(G) on success / record_failure(G) on error
  -> follow-up cycle only on Success with a strictly newer generation
  -> release lock (RAII Drop), exit(0)
```

There is no daemon, queue database, IPC channel, or service-manager
integration. The helper is opportunistic and bounded by both the worker
lifetime and `auto_sync_timeout_seconds` per attempt. The sync client caps
requests, retries, and retry sleeps by the remaining attempt deadline;
local filesystem operations are not force-cancelled.

## Module layout (`src/auto_sync/`)

| Module | Owns |
|--------|------|
| `policy.rs` | `AutoSyncPolicy`, `MutationKind`/`MutationOrigin`, `RetryDisposition`, `transient_backoff` (re-exports `FailureClass` from `sync_failure.rs`) |
| `pending.rs` | Durable generation marker (`PendingState`, v2 TOML + CRC32, monotonic increments, conditional clear) |
| `pending_lock.rs` | `PendingTxnGuard`: short-lived transaction lock for pending read-modify-write sections |
| `execution_lock.rs` | `SyncExecutionLock` + `WorkerLock`, `spawn_worker()`, platform detachment, kernel-backed ownership |
| `lock.rs` | Re-export shim over `execution_lock` worker-lock types (backward compatibility) |
| `schedule.rs` | `schedule_sync()` — sole scheduling authority; `Caller`; `schedule_and_spawn` / `schedule_existing_pending` |
| `notification.rs` | Parent-side `notify_mutation` / `notify_local_mutation`, explicit-sync helpers, `StartupRecoveryPolicy` |
| `worker.rs` | Debounce, preflight, direct canonical sync, exact-generation clear, bounded follow-up loop |
| `status.rs` | Durable `AutoSyncStatus` result/backoff/attention state (secret-free, integrity-checked) |
| `test_events.rs` | Test-only JSON-lines lifecycle events (compile-time no-op in production) |
| `mod.rs` (`paths`) | Stable path helpers for doctor/diagnostics (`state_dir`, `pending_marker`, `pending_txn_lock`, `worker_lock`, `execution_lock`, `status_file`) |

Hidden command: `auto-sync-worker --state-dir <path>` (`WORKER_SUBCOMMAND`)
is internal, hidden from help, and suppressed from startup-recovery
recursion (`StartupRecoveryPolicy::SuppressInternal`).

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

Resolved once per invocation via `AutoSyncPolicy::resolve(&settings)`.
Defaults: disabled, 2 s debounce, `Warn`, 30 s timeout; `Default` impl
matches. Clamps: debounce 0–300 s (`MAX_DEBOUNCE_SECS`), max-delay 0–600 s,
timeout 5–120 s. Malformed values clamp — they never disable sync.

`AutoSyncFailureMode` (`Ignore` / `Warn` default / `Error`) only controls
parent-side scheduling-failure rendering. It never rolls back the local
commit; `Error` yields a distinct post-commit nonzero exit.

Retry behaviour is driven by durable backoff state in
`auto-sync-status.toml` — distinct from `SyncRetryConfig.max_retries`,
which controls per-RPC gRPC attempts inside one sync operation.

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

`FailureClass` (4 variants) is defined in `sync_failure.rs` and re-exported
unchanged. `RetryDisposition`: `RetryAfter(Duration)` (Transient, Internal
< 3), `WaitForConfigurationChange` (Configuration), `RequiresAttention`
(LocalFailure, Internal ≥ 3), `NoAutomaticRetry` (explicit `snp sync` only).
Backoff schedule: 5 s → 15 s → 30 s → 60 s → exponential, capped at 15 min
(+0–20% jitter).

## Durable pending state (`pending.rs`, `pending_lock.rs`)

`auto-sync-pending.toml`, schema v2:

```toml
schema = 2
generation = 7
created_at_unix_ms = 1700000000000

[snapshot.Mutation]
kind = "snippet_create"

integrity = "crc32:441c462e"
```

- `integrity` is `crc32:<hex>` over schema + generation + creation
  timestamp + serialized snapshot (+ `source_transaction_id` when set).
  v1 markers (`kind`, `created_at_unix_ms`) migrate transparently.
- Atomic write: unique temp file (PID + nanoseconds) + rename + directory
  fsync (`atomic_write_bytes`, `DurableUserData`); 0o600 on Unix. No
  secrets, commands, or snippet content.
- `record_pending_mutation` is the **only** API that increments the
  generation; scheduling paths must never mutate it.
  `ensure_pending_for_transaction` gives restore exactly-once semantics
  (`Created` / `Reused` without increment / `Conflict` preserving newer
  work).
- Generations are **monotonic**. A lower generation normally means corrupt
  rollback: fail closed, preserve the marker, spawn nothing. Exception —
  cleared-and-recreated marker (observed as generation 1, or any strictly
  lower generation, with a strictly **newer** `created_at_unix_ms` after an
  explicit sync cleared the old marker): adopted as new work
  (`adopt_generation_reset`, in debounce, preflight, and the follow-up
  check). Equal-or-older timestamps stay corrupt.
- `clear_if_generation_matches` is the automatic acknowledgement boundary:
  read-compare-delete under `PendingTxnGuard`. `Cleared` on equality,
  `GenerationChanged{current}` preserves newer work,
  `Missing` (already cleared) is success. Clear errors preserve
  recoverability and record failure. `record_success` wraps the same
  boundary; pending-side `record_failure` only logs and preserves intent.

`pending_lock.rs` (`auto-sync-pending.lock`) serializes concurrent CLI
processes for the minimum read-modify-write section only (10 s timeout):
atomic `create_new`, bounded retry with 1–5 ms jitter, dead-owner reclaim,
ownership-checked `Drop`, 0o600. Distinct from the long-lived execution
lock below.

## Scheduling (`schedule.rs`)

`schedule_sync(state_dir, policy, caller)` is the **sole scheduling
authority** — startup recovery, post-mutation, and explicit retry all go
through it. It never probes the execution lock; worker acquisition is the
sole execution authority, so redundant spawns exit cheaply.

| Decision | Meaning |
|----------|---------|
| `SpawnNow` | Spawn immediately (only variant that spawns) |
| `DeferredUntil(ms)` | Backoff active (`next_attempt_at_unix_ms` in the future) |
| `RequiresAttention(class)` | No automatic retry (config/local/internal-escalated) |
| `NoPending` | No marker — nothing to do |
| `Disabled` / `NotConfigured` | Policy state |

Corrupt pending is a typed `ScheduleError::Pending` — never collapsed into
`NoPending`/`SpawnNow`/success; spawn failures are `ScheduleError::Spawn`.
`Caller::{StartupRecovery, Mutation, ExplicitRetry}` selects logging and
whether backoff may be bypassed (explicit retry only). Configuration-class
deferral is released when the config fingerprint (server URL hash, flags,
`credential_revision` — no secrets) changes. `schedule_and_spawn` is the
sole translator of `SpawnNow` into `spawn_worker` (structurally pinned by a
unit test); `schedule_existing_pending` schedules already-recorded intent
(e.g. restore) without touching the generation. Under `test-support`,
`SNP_SKIP_WORKER_SPAWN` suppresses the spawn (production: never checked).

## Worker (`worker.rs`, `execution_lock.rs`)

`run(state_dir)`: resolve policy → `try_acquire` (`AlreadyHeld` →
`NothingToDo`, preserving pending) → `run_locked` loop:

1. **Debounce.** Deadline = observed `created_at + debounce`, capped by
   `start + max_lifetime`. Sleeps in ≤ 250 ms slices, re-reading the marker
   each time; newer generations restart the deadline, marker removal
   cancels, corruption fails the cycle as `Internal`. The
   cleared-and-recreated reset is adopted as a fresh full window.
2. **Preflight.** Re-reads the marker; rollback without a newer timestamp
   fails, removal means nothing to do.
3. **Execute.** Honours configured direction, builds a `new_current_thread`
   runtime (client keeps `rt-multi-thread`), and calls
   `run_sync_with_limits(.., Some(SyncRunLimits{deadline, request_timeout}))`
   directly — never a child sync process. The helper holds the execution
   lock for the complete cycle.
4. **Acknowledge.** Success → `clear_if_generation_matches(observed)`:
   `Cleared` / `GenerationChanged` (newer work preserved) / `Missing`
   (already cleared) all record success; clear I/O failure records
   `LocalFailure`. Failure → `record_failure` with
   `FailureClass::from_error`, incremented consecutive count, and
   `transient_backoff` as `next_attempt_at_unix_ms`; status write failure
   never clears pending. `test_events::emit` records `sync_completed`.
5. **Follow-up.** Only `Success` with a strictly newer generation loops
   (bounded by `max_lifetime`); a failed helper exits without an immediate
   retry even if newer work appeared. Rollback after sync records
   `Internal`.

## Locks: kernel authority, diagnostic metadata

| Lock | File | Held by | Scope |
|------|------|---------|-------|
| `SyncExecutionLock` | `auto-sync-execution.lock` | Worker for the whole cycle; manual `snp sync`, explicit `--sync` (`wait_acquire`, 30 s), cron | All sync work — no concurrent sync possible |
| `WorkerLock` (legacy name) | `auto-sync-worker.lock` | Same kernel primitive via `execution_lock` (`lock.rs` shim) | Retained for compatibility/diagnostics |
| `PendingTxnGuard` | `auto-sync-pending.lock` | Parent mutation commands | Minimum pending critical section only |

Mutual exclusion comes from the kernel (`flock` Unix / `LockFileEx`
Windows via `process_file_lock`); on-disk TOML (`pid`, `started_at`,
`nonce`, `purpose`) is diagnostic only and may be stale. Stale metadata is
overwritten by the next acquirer — no inspect-then-rename race. `Drop`
releases without unlinking and only removes the file when PID + nonce still
match (never deletes a replacement owner's lock). Liveness is
`kill(pid, 0)`: only `ESRCH` proves absence (`EPERM`/unknown = live);
Linux start tokens use `/proc/<pid>/stat` field 22. Non-Unix treats
unknown PIDs as alive (conservative non-stealing).

## Detachment

`spawn_worker` re-execs `current_exe` with
`auto-sync-worker --state-dir <dir>`; stdio is nulled (stderr optionally
appended to `SNP_AUTO_SYNC_WORKER_LOG` for debugging). Unix detaches with
`libc::setsid()` in `pre_exec` (new session, no controlling terminal, outlives
the parent); Windows uses `DETACHED_PROCESS | CREATE_NO_WINDOW`. Parent
scheduling failures render per `failure_mode` (`Ignore`: log only, `Warn`:
stderr warning, `Error`: stderr + nonzero exit, local commit intact);
worker-side failures go to logs and `doctor`, never the departed parent's
stderr.

## Budgets

| Budget | Setting | Default / clamp | Meaning |
|--------|---------|-----------------|---------|
| Debounce (quiet period) | `auto_sync_debounce_seconds` | 2 s, 0–300 | Coalesces rapid mutations |
| Max delay (starvation cap) | `auto_sync_max_delay_seconds` | 300 s, 0–600 | Sole pre-sync window cap; worker exits at `start + max_lifetime` |
| Attempt timeout | `auto_sync_timeout_seconds` | 30 s, 5–120 | Per-attempt network/retry budget (`SyncRunLimits`); expiry records `Transient`, preserves pending, never force-cancels local I/O |

Manual sync and cron keep their existing (unbounded) timeout behaviour.

## Trigger matrix (`notification.rs` + call sites)

`notify_mutation(kind, origin)` resolves policy and calls
`notify_local_mutation`: record one generation, then `schedule_and_spawn`.
Results: `Disabled` (no sync account and no `sync.toml`), `Suppressed`
(SyncMerge origin), `PendingRecorded` (generation durable, policy off),
`Scheduled{generation}`, `SchedulingFailed{generation}` (pending preserved,
local commit intact).

| Command | `MutationKind` / `Origin` | Triggers? |
|---------|---------------------------|-----------|
| `snp new` (all sources) | `SnippetCreate` / `User` | Yes, after atomic save |
| `snp edit` (editor, changed only) | `SnippetUpdate` / `User` | Yes, after editor closes (including editor-failure-with-changes) |
| `snp edit --output` / `--clear-output` (by filter or ID) | — | **No** (no notify call; `output` is local-only, nothing remote to sync) |
| TUI delete | `SnippetDelete` / `User` | Yes, after tombstone save (with `--sync`, explicit sync instead) |
| `snp import pet` (create / merge-changed / replace) | `Import` / `Import` | Yes, after library file + config registration |
| `snp import pet` (dry-run, strict abort, parse failure, no-op merge) | — | **No** |
| `snp library create` / `snp library delete --force?` | `LibraryChange` / `User` | Yes, after manager success |
| `snp library set-primary` / `list` | — | **No** (local-only metadata / read-only) |
| `snp premade get` | — | **No** (local copy of remote data) |
| `snp sync` (manual) / `--sync` paths | — | Clears pending (`observe` → sync → generation-scoped clear), never schedules |
| Sync merge / restore replay writes | `SyncConflictWrite` / `SyncMerge` | **No** (origin suppression prevents loops) |
| `AccountConfig` mutations | `AccountConfig` | **No** (`is_syncable_mutation() == false`) |
| Restore commit | existing pending | `schedule_existing_pending` only — no new generation |

Each command notifies strictly after its authoritative commit point (after
`save_library`, after the library file **and** config registration for
import, after manager success for libraries). Backup failure, dry-run,
cancel, failure, and no-op paths emit nothing; machine-facing stdout stays
free of sync diagnostics.

## Explicit `--sync` precedence (`commands/mod.rs`)

`run_explicit_sync(runtime)` — shared by `run`/`clip`/`search` and TUI
delete paths: observe pending generation **before** sync → `wait_acquire`
(execution lock, 30 s) → `run_default_sync` → `clear_pending_after_explicit_sync`
(generation-scoped, success-gated). A mutation arriving mid-sync keeps its
newer generation; no duplicate delayed sync fires for the same generation.
Manual `snp sync` follows the same observe-then-clear shape in
`sync_cmd.rs`.

## Doctor integration

`snp doctor --compatibility` reads state read-only via `auto_sync::paths`
(`state_dir`, `pending_marker`, `pending_txn_lock`, `worker_lock`,
`execution_lock`, `status_file`) with `process_alive` liveness probes:

- `compat.auto_sync.enabled` / `.disabled` — policy state.
- `compat.auto_sync.pending_active` / `.pending_stale` (> 5 min,
  `startup_recover_pending` clears) / `.pending_unreadable`.
- `compat.auto_sync.lock_held` / `.lock_stale` / `.lock_unreadable`.

See [status.md](status.md) for the `snp status` projection over the same
artifacts.

## Status + test seams (`status.rs`, `test_events.rs`)

`auto-sync-status.toml` (schema 1, CRC32, 0o600, ≤ 512-char message) records
each attempt independently of pending intent: generations, attempt/success
timestamps, result + `FailureClass` codes, consecutive failures,
`next_attempt_at_unix_ms`, exit code, `attention_required`, and a
secret-free config fingerprint. Status write failure never clears pending.
`test_events.rs` emits JSON-lines lifecycle events to
`$SNP_TEST_EVENTS_DIR/test-events.jsonl` only under `test-support`;
production builds compile it to a no-op (the env check is absent).

## Safety invariants

1. Disabled by default; `sync_configured` stays true while auto-sync is off
   so manual sync keeps working and pending intent survives re-enable.
2. Local mutation commits before any remote work; remote/scheduling failure
   never rolls back or corrupts it.
3. Parent never holds the execution lock; scheduler never probes it — worker
   acquisition is the sole execution authority.
4. `schedule_sync` is the sole spawn authority; exactly one production call
   site (`spawn_worker_if_needed`).
5. Pending generations monotonic; lower generation fails closed except the
   cleared-and-recreated (lower generation + strictly newer timestamp)
   adoption.
6. `clear_if_generation_matches` is the only automatic ack boundary; stale
   clears preserve newer work; `Missing` is already-cleared success.
7. `SyncMerge` origin never triggers (no feedback loops).
8. Secrets, commands, and snippet content never enter pending markers, lock
   files, worker argv/env, status, or logs (secret-free fingerprint only).
9. Pending marker survives crash; stale markers (> 5 min) clear on startup
   recovery; startup recovery is suppressed for read-only, explicit-sync,
   internal-worker, and configuration commands.
10. All sync operations (automatic, manual, explicit `--sync`, cron) share
    one `SyncExecutionLock`; worker holds it for the full cycle.
11. Worker only calls `run_default_sync`; it never mutates libraries
    directly, and local I/O is never force-cancelled by a timeout.
12. `auto-sync-worker` adds no public CLI surface (hidden subcommand).
13. Scheduling errors stay typed (`Pending` vs `Spawn`); backoff/attention
    state is durable and config-change aware.
14. Tests assert exact notification counts, prove server-side effects, verify
    pending-clear ordering, and emit events only under
    `SNP_TEST_EVENTS_DIR`.

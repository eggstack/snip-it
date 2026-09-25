# Transactions & Auto-Sync Skill

## Purpose
Guide agents through the crash-recovery transaction subsystem (`src/transaction.rs`,
`src/local_data.rs`, `src/process_file_lock.rs`) and the auto-sync machinery
(`src/auto_sync/`). These are the largest and most invariant-heavy modules — read
this before changing either.

## Directory Model (two directories — never mix them)

| Directory | Path | Contents |
|-----------|------|----------|
| `sync_state_dir` | `~/.config/snp/` | `auto-sync-pending.toml`, `auto-sync-status.toml`, locks |
| `transaction_dir` | `<sync_state_dir>/.transaction/` | `txn-<uuid>.toml` journals, `transaction.lock`, durable backups, staged files |

- Pending APIs receive `sync_state_dir`; transaction APIs receive `transaction_dir`.
- `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` needs BOTH.
- Callers derive them via `crate::auto_sync::notification::derive_state_dir()` and
  `.join(".transaction")` (e.g. `src/commands/mod.rs:178`, `src/commands/restore_cmd.rs`).

## Transaction State Machine

```
Prepared → Committing{next_commit_position} → CleaningUp{outcome, next_step} → journal removed
Prepared → RollingBack{next_rollback_position} → CleaningUp{...} → journal removed
Committing → CommittedLocal{pending} → CleaningUp → journal removed (restore pending-finalization path)
Prepared → Failed(error_message) (persisted terminal; recovery refuses without repair)
```

- Absence of a journal is the true terminal indicator; terminal states are never persisted.
- `BackupsDurable`, `Committed`, `RolledBack` are legacy recovery-only states (older journals); new code paths don't produce them.
- `Committing{next_commit_position}` uses completed-position semantics: progress is
  persisted only AFTER each verified atomic write, never before.
- Rollback restores files in reverse order with hash verification after each action.
- Interruptible states for recovery: `Prepared`, `BackupsDurable`, `Committing`,
  `CommittedLocal`, `RollingBack`, `CleaningUp` (the latter two legacy states included).
- Restore uses the explicit sequence: `begin_transaction` →
  `advance_to_backups_durable` → `advance_to_committing` → `advance_to_committed_local`
  → `commit_transaction`.

## Lock Hierarchy & Process Locks

Acquisition order: **`LocalDataLock` → `TransactionLock` → writes**. Never invert.

- Cross-process exclusion has two mechanisms: kernel-backed `flock`/`LockFileEx` via `src/process_file_lock.rs` (auto-sync execution/worker/pending locks, server singleton) and owned-record + PID-reclaim locks (`TransactionLock`, `LocalDataLock`: `create_new` + `ProcessIdentity::observe` + quarantine). The kernel alone is authoritative where it applies — persistent lock file metadata may be stale and is diagnostic only.
- Linux process start tokens use `/proc/<pid>/stat` field 22 (`starttime`). Unix
  `kill(pid, 0)` probes treat `EPERM` and unknown errors as a live process; only
  `ESRCH` proves absence.
- Ownership verification compares the OBSERVED start token at `existing.pid` against
  the persisted token (prevents misclassifying a live owner as PID reuse).
- `Drop` releases the kernel lock WITHOUT unlinking the file; unlink only happens when
  nonce AND start_token both match (prevents old owners deleting replacement locks).
- Malformed lock files are quarantined (`.quarantine.<uuid>`), never silently deleted.
- `save_library_internal()` skips gate+lock for internal callers already holding locks.

## Mutation Gate

`gate_mutation_on_interrupted_transactions()` MUST be called before any local mutating
operation:
- Single interrupted journal → auto-rollback.
- Multiple or incomplete journals → refuse and direct the user to `snp repair`.
- It inspects the canonical pending marker in `sync_state_dir` while cleaning up
  artifacts in `transaction_dir`.

## Auto-Sync Machinery (`src/auto_sync/`)

Runtime model: local atomic commit → record pending generation G → detach one
`snp auto-sync-worker` → worker acquires `SyncExecutionLock`, debounces, runs
`run_sync_with_limits` directly, clears ONLY generation G on success.

Hard rules:
- The detached worker holds `SyncExecutionLock` for the entire bounded cycle. Manual
  sync, explicit `--sync`, and cron acquire the same lock. The scheduler NEVER probes
  the execution lock.
- Pending generations are monotonic. A lower generation observed during debounce or
  preflight is corrupt state: preserve the marker, log, do not spawn work. Exception:
  a lower generation carrying a strictly newer creation timestamp is a marker that an
  explicit sync cleared and a new mutation re-recorded; debounce adopts it.
- `schedule_sync`, `schedule_and_spawn`, and `schedule_existing_pending` return typed
  local scheduling errors. Never collapse pending-read or spawn failures into
  `NoPending`/`SpawnNow`/a successful notification.
- Configuration failures defer until config change (fingerprint) or explicit retry;
  transient failures retain pending intent and use durable exponential backoff in
  `auto-sync-status.toml`. There is no in-memory retry counter (`max_retries` was
  removed — do not re-add).
- Worker uses `Builder::new_current_thread()`; the client keeps tokio's
  `rt-multi-thread` feature for this detached-worker path.

## Module Map

| File | Role |
|------|------|
| `src/transaction.rs` | Journal, states, begin/commit/rollback, recovery scan |
| `src/local_data.rs` | `LocalDataLock`, dir derivation, cross-process barriers |
| `src/process_file_lock.rs` | Kernel flock/LockFileEx primitive |
| `src/migration.rs` | `SchemaVersion`; `write_schema_version` must use `toml::Table` to preserve array-of-tables |
| `auto_sync/schedule.rs` | Centralized scheduling decisions (`schedule_sync`) |
| `auto_sync/pending.rs` / `pending_lock.rs` | Monotonic pending generation marker |
| `auto_sync/execution_lock.rs` | `SyncExecutionLock` (wait_acquire / try_acquire) |
| `auto_sync/worker.rs` | Detached helper cycle |
| `auto_sync/status.rs` / `notification.rs` | Durable status/backoff; pending clear after manual sync |
| `auto_sync/policy.rs` | Backoff policy; consumes `FailureClass` defined in `src/sync_failure.rs` |

## Test Seams

- Crash failpoints live in `src/test_failpoints.rs` behind `test-support` feature;
  deep-recovery tests (`transaction_crash_recovery.rs`, `cleanup_crash_failpoints.rs`,
  `restore_crash_failpoints.rs`) run manually/release only.
- Barrier-coordinated tests (`local_data_lock_barriers.rs`, `repair_transactions.rs`)
  need `--features test-support` AND `--test-threads=1` (`set_var` needs `unsafe` in edition 2024).
- `process_lock_concurrency.rs` spawns real subprocesses — serial target.
- Test-only env vars (`SNP_TEST_FAILPOINT`, `SNP_SKIP_WORKER_SPAWN`, `SNP_TEST_EVENTS_DIR`, `SNP_TEST_MUTATION_BARRIER_DIR`, `SNP_ALLOW_DIR_FSYNC_FAILURE`) are inert in production builds — proven by `scripts/ci/test-production-seams.sh`.

## Related Docs
- `architecture/persistence.md` — verified deep-dive on transactions/durability
- `architecture/auto_sync.md` — verified deep-dive on worker contracts
- `docs/PERSISTENCE_INVENTORY.md` — every persisted artifact with durability class

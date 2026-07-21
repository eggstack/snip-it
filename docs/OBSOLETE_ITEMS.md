# Obsolete and Transitional Items — Phase 06A Workstream J

## 1. Old "Coordinator" Terminology

The "coordinator" was the in-process debounce state machine from Release 5B/5C that was replaced by the detached worker model (Release 5D-5F). The term persists in test file names, architecture docs, and plan docs.

### Source Code (.rs files)
**No coordinator references in `src/`** — clean. The auto-sync module uses `worker`, `executor`, `schedule`, `notification` terminology.

### Test Files
| File | Issue |
|------|-------|
| `tests/auto_sync_coordinator.rs` | **Entire file named after obsolete concept.** Contains tests like `test_disabled_auto_sync_no_coordinator_files` and references to "coordinator" in comments. Should be renamed to `auto_sync_worker.rs` or similar. |
| `tests/auto_sync_mutations.rs:502` | Comment: "No corrupted coordinator state blocks future commands" — stale reference. |
| `tests/auto_sync_config.rs:240,267` | Comments: "created by the coordinator" and "create lock file via the coordinator" — should reference "worker" or "spawn system". |

### Architecture Docs
| File | Line | Issue |
|------|------|-------|
| `architecture/overview.md:290` | Table: "Auto-sync policy, coordinator, debounce, triggers" — should say "worker" |
| `architecture/auto_sync.md:522` | "Replaces the in-process coordinator (Release 5D)" — acceptable as historical context but could be tightened |
| `architecture/sync.md:198,408,471` | Historical references to coordinator — acceptable as design rationale |

### Plan Documents
Multiple plan docs reference coordinator — these are historical artifacts and can remain, but should not be treated as current architecture references.

**Action**: Rename `tests/auto_sync_coordinator.rs`, update 3 test comments, update `architecture/overview.md` table entry.

---

## 2. Direct Worker Spawns Outside Central Scheduler

**Status: CLEAN.** The structural test `test_spawn_worker_only_called_from_scheduler` (`src/auto_sync/schedule.rs:482`) pins the invariant that `spawn::spawn_worker` is only called from `schedule_and_spawn`. No ad-hoc spawn paths exist.

---

## 3. Duplicate Sync Wrappers

### `sync.rs` vs `sync_commands.rs`
These serve different purposes and are **not duplicates**:
- `sync.rs`: gRPC client (`SyncClient`) — low-level transport, retry, encryption
- `sync_commands.rs`: Orchestration — merge logic, conflict resolution, `run_sync()`, `run_premade_sync()`, `run_default_sync()`

**Status: CLEAN.** No overlapping functions.

### Duplicate Retry Config
| Location | Default | Purpose |
|----------|---------|---------|
| `src/sync.rs:30` | `DEFAULT_MAX_RETRIES = 3` | gRPC client-level retry (per-request) |
| `src/auto_sync/policy.rs:7` | `DEFAULT_MAX_RETRIES = 1` | Auto-sync policy-level retry (per-cycle) |

These are **distinct retry layers** (transport vs. orchestration) and both are correct. However:
- `policy.rs:19` has `pub max_retries: u32` that is **always set to `DEFAULT_MAX_RETRIES`** and never read by the worker or executor. The worker uses `sync_timeout` and exit codes, not `max_retries`. This field is **dead**.

**Action**: Remove `max_retries` from `AutoSyncPolicy` and `DEFAULT_MAX_RETRIES` from `policy.rs`.

---

## 4. Duplicate Policy Loaders

**Status: CLEAN.** `AutoSyncPolicy::resolve()` is the single policy loader. No alternative loaders exist.

---

## 5. False Timeout/Cancellation Comments

### Cancellation Comments
All cancellation references in `src/` are **correct** — they refer to:
- User cancelling TUI selection (`q`, `Esc`, `Ctrl-C`)
- Clipboard clear cancellation (atomic generation counter)
- Output file cleanup on cancel
- Shell buffer restoration on cancellation

**Status: CLEAN.** No false cancellation claims.

### Timeout Comments
All timeout references in `src/` are **correct** — they refer to:
- Clipboard operation timeouts (real)
- gRPC connect/request timeouts (real)
- Execution lock wait timeout (real)
- Worker sync_timeout (real)
- Debounce poll timeout (real)
- Run command timeout (real)

The specific comment about `tokio::time::timeout` around `spawn_blocking` not cancelling the underlying thread (referenced in `architecture/auto_sync.md:527`) was the motivation for the executor subprocess model — this is historical design rationale, not a stale false claim.

**Status: CLEAN.**

---

## 6. Unused `max_retries` / Stale Fields

### `AutoSyncPolicy.max_retries`
- Defined at `src/auto_sync/policy.rs:19`
- Set to `DEFAULT_MAX_RETRIES` (1) in `resolve()` and `Default`
- **Never read** by any production code. The worker uses `sync_timeout` and executor exit codes for retry decisions.
- Referenced only in tests (`policy.rs` unit tests)

**Action**: Remove `max_retries` from `AutoSyncPolicy` struct, remove `DEFAULT_MAX_RETRIES` constant.

### `SyncRetryConfig.max_retries`
- Defined at `src/sync.rs:39`
- Used by `retry_grpc!` macro and `sync_with_retry` method
- **Actively used** — this is the gRPC transport retry config.

**Status: KEEP.** This is a different retry layer.

### `STALE_LOCK_THRESHOLD_SECS`
- Defined at `src/auto_sync/lock.rs:9` as `5 * 60`
- **Never used** — staleness is determined by `process_alive()` (PID check), not by age threshold.

**Action**: Remove unused constant.

---

## 7. Legacy Aliases

### `pub use` statements
| Location | Item | Status |
|----------|------|--------|
| `src/lib.rs:40` | `pub use error::{SnipError, SnipResult}` | Legitimate re-export for public API |
| `src/config.rs:7` | `pub use crate::utils::config::get_sync_config_path` | Legitimate re-export |
| `src/auto_sync/mod.rs:16-23` | Re-exports of notification, pending, policy types | Legitimate module API |
| `src/ui/mod.rs:14-15` | Re-exports of theme and variables | Legitimate module API |
| `src/utils/mod.rs:16` | Re-exports of variables | Legitimate module API |

### `pub type` statements
| Location | Item | Status |
|----------|------|--------|
| `src/error.rs:293` | `pub type SnipResult<T>` | Standard error alias |
| `src/encryption.rs:97` | `pub type CryptoResult<T>` | Standard error alias |

**Status: CLEAN.** All `pub use` and `pub type` are legitimate API surface, not transitional aliases.

---

## 8. Temporary Debug `eprintln!`

All `eprintln!` calls in `src/` are **intentional production output**:
- `src/logging.rs`: Panic handler, log init warnings — correct
- `src/main.rs`: Runtime creation failure, signal handler failure, CLI error display — correct
- `src/commands/doctor_cmd.rs`: Doctor report output — correct (doctor is a diagnostic command)
- `src/commands/import_cmd.rs`: Import report output — correct
- `src/commands/premade_cmd.rs`: "Sync not enabled" messages — correct
- `src/commands/shell_cmd.rs`: "skipping: bash not found" — correct
- `src/commands/edit_cmd.rs`, `register_cmd.rs`, `cron_cmd.rs`: User-facing messages — correct

**No temporary debug `eprintln!` found.** All uses are intentional CLI output.

---

## 9. Stale Release 5 Labels

### Architecture Docs
| File | Lines | Issue |
|------|-------|-------|
| `architecture/auto_sync.md` | 7, 50, 229, 263, 264, 286, 304-306, 310, 374, 515, 520, 522, 527-528, 645-651 | ~30+ references to "Release 5A–5F", "Release 5 corrective", "Phase 01" |
| `architecture/sync.md` | 194, 359, 414, 462-467, 471, 564 | ~10 references to "Release 5C/5D/5E/5F" |

These are **historical design annotations** that explain *why* the architecture is the way it is. They are useful as provenance but can be confusing as "current" labels.

**Action**: Convert inline "Release 5X" annotations to a single historical note at the top of each file, or move to `## History` sections. Do not delete — they provide valuable design provenance.

---

## 10. Dead Lock Types

### Three lock types exist:
| Lock | File | Purpose | Used by |
|------|------|---------|---------|
| `WorkerLock` | `lock.rs` | Prevents concurrent workers | Worker subprocess, parent inspect |
| `ExecutionLock` | `execution_lock.rs` | Prevents concurrent sync operations | Worker, executor, manual sync, cron |
| `PendingTxnGuard` | `pending_lock.rs` | Serializes pending marker read-modify-write | `pending.rs` internal |

**Status: CLEAN.** All three locks serve distinct purposes:
- `WorkerLock`: mutual exclusion between debounce workers (one active worker at a time)
- `ExecutionLock`: mutual exclusion between sync operations (any sync path)
- `PendingTxnGuard`: transactional safety for the pending marker file

No dead lock types.

---

## Summary of Actionable Items

| # | Item | Priority | Effort |
|---|------|----------|--------|
| 1 | Rename `tests/auto_sync_coordinator.rs` and update 3 stale "coordinator" comments in tests | High | Low |
| 2 | Update `architecture/overview.md` table entry (coordinator → worker) | High | Trivial |
| 3 | Remove `max_retries` from `AutoSyncPolicy` (dead field) | High | Low |
| 4 | Remove `STALE_LOCK_THRESHOLD_SECS` from `lock.rs` (unused constant) | Medium | Trivial |
| 5 | Tidy Release 5 labels in architecture docs (move to History sections) | Low | Medium |
| 6 | Feature-gate `sync`, `tui`, `auto-sync`, `clipboard`, `self-update`, `bundled-themes` | Future | High |

Items 1-4 are straightforward removals. Item 5 is cosmetic. Item 6 is the feature boundary work from Workstream I.

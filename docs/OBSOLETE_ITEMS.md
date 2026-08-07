# Obsolete and Transitional Items — Phase 06A Workstream J

## 1. Old "Coordinator" Terminology

The "coordinator" was the in-process debounce state machine from Release 5B/5C that was replaced by the detached worker model (Release 5D-5F). The term persists in test file names, architecture docs, and plan docs.

### Source Code (.rs files)
**No coordinator references in `src/`** — clean. The auto-sync module uses `worker`, `executor`, `schedule`, `notification` terminology.

### Test Files
| File | Issue |
|------|-------|
| `tests/auto_sync_coordinator.rs` | **Renamed** — file no longer exists under this name. |
| `tests/auto_sync_mutations.rs:502` | Comment: "No corrupted coordinator state" — **Fixed** to "pending state". |
| `tests/auto_sync_lifecycle.rs:1,218,240,349` | Module doc and comments reference "coordinator" — **Fixed** to "worker"/"lifecycle". |
| `tests/auto_sync_config.rs:240,267` | Comments: "created by the coordinator" — **Fixed** to "worker". |

### Architecture Docs
| File | Line | Issue |
|------|------|-------|
| `architecture/overview.md:318` | Table: "Auto-sync policy, coordinator, debounce, triggers" — **Fixed** to "worker" |
| `architecture/auto_sync.md:522` | "Replaces the in-process coordinator (Release 5D)" — acceptable as historical context but could be tightened |
| `architecture/sync.md:198,408,471` | Historical references to coordinator — acceptable as design rationale |

### Plan Documents
Multiple plan docs reference coordinator — these are historical artifacts and can remain, but should not be treated as current architecture references.

**Action**: Rename `tests/auto_sync_coordinator.rs`, update 3 test comments, update `architecture/overview.md` table entry. **Status: Done.**

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

**Status: CLEAN.** The `AutoSyncPolicy.max_retries` field and `DEFAULT_MAX_RETRIES` in `policy.rs` were **removed** in Phase 06A. Only the gRPC transport retry config remains.

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

## 6. Unused `max_retries` / Stale Fields (Removed)

### `AutoSyncPolicy.max_retries`
- **REMOVED** in Phase 06A. Was never read by any production code.
- Retry behavior is now driven entirely by durable backoff state in `auto-sync-status.toml`.

### `SyncRetryConfig.max_retries`
- Defined at `src/sync.rs:39`
- Used by `retry_grpc!` macro and `sync_with_retry` method
- **Actively used** — this is the gRPC transport retry config.

**Status: KEEP.** This is a different retry layer.

### `STALE_LOCK_THRESHOLD_SECS`
- **REMOVED** in Phase 06A. Was unused; lock staleness is handled by timeout logic and `kill -0` process liveness checks.

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
| `WorkerLock` | `execution_lock.rs` | Prevents concurrent workers | Worker subprocess, parent inspect |
| `SyncExecutionLock` | `execution_lock.rs` | Prevents concurrent sync operations | Worker, executor, manual sync, cron |
| `PendingTxnGuard` | `pending_lock.rs` | Serializes pending marker read-modify-write | `pending.rs` internal |

**Status: CLEAN.** All three locks serve distinct purposes:
- `WorkerLock`: mutual exclusion between debounce workers (one active worker at a time)
- `ExecutionLock`: mutual exclusion between sync operations (any sync path)
- `PendingTxnGuard`: transactional safety for the pending marker file

No dead lock types.

---

## Summary of Actionable Items

| # | Item | Priority | Effort | Status |
|---|------|----------|--------|--------|
| 1 | Rename `tests/auto_sync_coordinator.rs` and update 3 stale "coordinator" comments in tests | High | Low | **Done** |
| 2 | Update `architecture/overview.md` table entry (coordinator → worker) | High | Trivial | **Done** |
| 3 | Remove `max_retries` from `AutoSyncPolicy` (dead field) | High | Low | **Done** |
| 4 | Remove `STALE_LOCK_THRESHOLD_SECS` from `lock.rs` (unused constant) | Medium | Trivial | **Done** |
| 5 | Tidy Release 5 labels in architecture docs (move to History sections) | Low | Medium | Open |
| 6 | Feature-gate `sync`, `tui`, `auto-sync`, `clipboard`, `self-update`, `bundled-themes` | N/A | N/A | **Removed** — empty feature labels removed in Phase 10; binary is monolithic |

Items 1-4 are completed. Item 5 is cosmetic. Item 6 is the feature boundary work from Workstream I.

# Persistence Architecture

[← Back to Overview](overview.md)

## Table of Contents

- [Overview](#overview)
- [Atomic Write Primitive](#atomic-write-primitive)
- [Process File Lock](#process-file-lock)
- [Lock Hierarchy](#lock-hierarchy)
- [Transaction Boundary](#transaction-boundary)
- [Local-Data Lock](#local-data-lock)
- [Validation Framework](#validation-framework)
- [Backup Format](#backup-format)
- [Restore Semantics](#restore-semantics)
- [Repair Command](#repair-command)
- [Migration Framework](#migration-framework)
- [Identity Contract](#identity-contract)
- [Security Properties](#security-properties)
- [Key Files](#key-files)
- [Invariants](#invariants)
- [Gotchas](#gotchas)

---

## Overview

snip-it uses a layered persistence architecture centered on editable TOML files. Atomic writes are standardized, stable identity is defined, and validation/backup/restore/repair workflows are supported.

The persistence stack has four layers:

1. **Atomic write primitive** — crash-safe file replacement with durability classes
2. **Transaction boundary** — multi-file coordination with journaling
3. **Validation, backup, restore, repair** — data integrity workflows
4. **Migration framework** — schema versioning and evolution

All user-facing data lives under `~/.config/snp/` (XDG-compliant, no per-platform branch — see [utils/config.md](utils/config.md)). See [library.md](library.md) for the snippet data model and [utils/atomic.md](utils/atomic.md) for the write primitive's finer points.

**What this document does not cover.** The *sync protocol*, conflict resolution, and merge semantics live in [sync.md](sync.md). The *auto-sync worker*, pending-generation marker, and execution lock live in [auto_sync.md](auto_sync.md). The *kernel* cross-process lock primitive is documented here ([Process File Lock](#process-file-lock)) and detailed in [process_file_lock.md](process_file_lock.md). The *library data model* is in [library.md](library.md). The *CLI surface* for backup/restore/repair/validate is in [commands/backup_cmd.md](commands/backup_cmd.md), [commands/restore_cmd.md](commands/restore_cmd.md), [commands/repair_cmd.md](commands/repair_cmd.md), and [commands/validate_cmd.md](commands/validate_cmd.md).

---

## Atomic Write Primitive

### Location

`src/utils/atomic.rs`

### API

Public functions:

| Function | Purpose |
|----------|---------|
| `write_private_atomic(path, content, prefix)` | Simple atomic write with `0o600` permissions on Unix |
| `atomic_replace(target, bytes, options)` | Enhanced atomic replace with durability classes, permission control, and target validation |
| `atomic_write_bytes(path, bytes, durability)` | Convenience wrapper over `atomic_replace` with default options |
| `hash_file(path)` | Canonical SHA-256 file helper (`transaction::hash_file` delegates here) |

### Durability Classes

```rust
pub enum Durability {
    DurableUserData,       // fsync before rename, parent dir sync
    SensitiveConfig,       // 0o600 permissions, symlink rejection
    RecoverableMetadata,   // no fsync
    EphemeralCoordination, // no fsync, no dir sync
}
```

| Class | fsync file | fsync dir | Permissions | Symlink reject |
|-------|-----------|-----------|-------------|----------------|
| `DurableUserData` | Yes | Yes (hard failure) | `0o600` temp file | No |
| `SensitiveConfig` | Yes | Yes (hard failure) | `0o600` | Yes |
| `RecoverableMetadata` | No | Yes (best-effort) | `0o600` temp file | No |
| `EphemeralCoordination` | No | No | `0o600` temp file | No |

On Unix the temp file is always created `0o600`, so a replaced file keeps
`0o600` unless `preserve_permissions` restores the previous mode.

Exact fsync semantics (`src/utils/atomic.rs`):

- **File fsync** — `file.sync_all()` runs only for `DurableUserData` and `SensitiveConfig` (`atomic.rs:402-408`). The other two classes get `file.flush()` only, which pushes to the kernel buffer but not to the device.
- **Directory fsync** — `sync_parent_dir()` hard-fails the write for `DurableUserData`/`SensitiveConfig` when the parent-dir fsync errors (`atomic.rs:228-255`). `RecoverableMetadata` downgrades a directory-fsync failure to a warning (`atomic.rs:257-264`). `EphemeralCoordination` skips the directory fsync entirely (`atomic.rs:195-197`).
- **Temp-file mode** — `0o600` for *every* class on Unix (`atomic.rs:383-387`); `write_private_atomic` likewise creates its temp at `0o600` (`atomic.rs:292`).
- **Windows** — `std` offers no creation-time permission knob, so the temp file inherits the parent directory's ACL. Windows installs rely on the user-private `%APPDATA%` directory ACLs instead of a file-level DACL (`atomic.rs:273-277`).

`SNP_ALLOW_DIR_FSYNC_FAILURE=1` downgrades a directory-fsync error to a warning, but only in `test-support` builds (`atomic.rs:217-226`). Production builds always fail closed on a durability error — the seam is inert outside tests.

### Options

```rust
pub struct AtomicWriteOptions {
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub reject_symlink: bool,
}
```

`AtomicWriteOptions::for_durability(d)` creates defaults: `reject_symlink = true` only for `SensitiveConfig` (`atomic.rs:61-68`).

### Algorithm

`atomic_replace` executes this sequence:

1. Resolve parent directory (create if missing via `create_dir_all`)
2. Validate target — reject directories, FIFOs, sockets, block/character devices, and optionally symlinks
3. Snapshot original permissions if `preserve_permissions` is set
4. Create UUID-named temp file in the same directory (`0o600` on Unix for every class)
5. Write bytes, flush to kernel buffer
6. For `DurableUserData` and `SensitiveConfig`, call `sync_all` on the file
7. Atomic `rename` over the target
8. Restore original permissions if `preserve_permissions` was set
9. Sync parent directory — for `DurableUserData`/`SensitiveConfig` a failure fails the write; for `RecoverableMetadata` it is logged and ignored; `EphemeralCoordination` skips it
10. On any failure, `TempFileGuard` cleans up the temp file

Target validation (`validate_target`, `atomic.rs:112-167`) rejects directories, FIFOs, sockets, and device nodes. A symlink is **replaced, never followed** — the final `rename` swaps the directory entry itself, so an allowed symlink is a valid target (`atomic.rs:126-131`).

`write_private_atomic` syncs the parent directory with `SensitiveConfig` semantics (`atomic.rs:326-327`), so its directory-fsync failures are hard errors. That sync is Unix-only — on non-Unix platforms the `sync_parent_dir` call is compiled out, so the rename entry is never flushed on Windows.

### Report

`atomic_replace` returns `AtomicWriteReport`:

```rust
pub struct AtomicWriteReport {
    pub target_existed: bool,
    pub bytes_written: u64,
    pub parent_sync_supported: Option<bool>,
}
```

### Write Path in LibraryManager

Every library TOML write goes through `write_private_atomic()`: `save_library()` (a free function in `src/library/persistence.rs`) for snippet libraries and `LibraryManager` for `libraries.toml`. The simple atomic write is used because it fsyncs the file, syncs the parent directory, and leaves the result `0o600` instead of briefly exposing library content through a world-readable temp file.

### Tests

`tests/persistence_unit.rs` exercises the full atomic write pipeline including durability classes, permission preservation, symlink rejection, and temp file cleanup on failure.

---

## Process File Lock

### Location

`src/process_file_lock.rs`

### Purpose

A single authoritative cross-process mutual-exclusion primitive built on the operating system's advisory file-lock facility. The post-11L corrective pass replaced the previous create-new / quarantine / PID-liveness reclaim model with a kernel-backed design so that:

- Mutual exclusion is owned by the kernel, not by PID-file inspection.
- The persistent lock file remains on disk and may contain stale metadata.
- `Drop` releases the kernel lock and closes the file but does not unlink or rename.
- A killed owner releases automatically through kernel process teardown.

### Authority

The kernel alone arbitrates. `flock(fd, LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` over a fixed byte range on Windows guarantee that at most one process holds the lock at a time. Unsupported platforms return `ProcessFileLockError::UnsupportedPlatform` rather than silently weakening exclusion.

### Acquisition Order

1. Ensure the parent directory exists.
2. Open the persistent lock file in read/write/create mode.
3. Attempt the kernel lock nonblocking.
4. If busy, read owner metadata best-effort for diagnostics and return `Busy`.
5. Tighten permissions to `0o600` where supported (warn-only on failure).
6. Truncate the file and write the new identity record (PID, nonce, start token, acquired timestamp).
7. `sync_all`, then return the guard.

If metadata publication fails after kernel acquisition, the kernel lock is released before the error is returned, and no caller is left believing it owns the lock.

### Identity Metadata

`LockIdentity` is for diagnostics only. It must never authorize lock stealing or canonical-file deletion. A contender that observes a busy kernel lock with empty, malformed, or legacy metadata must treat it as a live owner.

On Linux, process start identity is parsed from `/proc/<pid>/stat` using field
22 (`starttime`) after locating the final closing parenthesis of `comm`, which
also handles spaces and parentheses in process names. Unix liveness probes
treat `ESRCH` as absent and `EPERM` or unknown errors conservatively as alive.

### Wrappers

Three thin wrappers in `src/auto_sync/` consume the primitive and preserve the existing public types and error variants:

- `auto_sync::execution_lock::SyncExecutionLock` — wraps the sync execution lock with `try_acquire` and `wait_acquire` semantics.
- `auto_sync::execution_lock::WorkerLock` — wraps the worker lock acquisition (merged from former `lock.rs`).
- `auto_sync::pending_lock::PendingTxnGuard` — short-lived pending-marker mutex.

`snip-sync`'s `server_lock::ServerLock` provides the same primitive for the server singleton at `<state_dir>/snip-sync.server.lock`. The server holds it for the full runtime; a crash releases the kernel lock automatically.

### Lifecycle Invariants

- No `.quarantine.*` files are produced during normal ownership transitions.
- The persistent file is never unlinked or renamed by `Drop`.
- A repeated cycle of acquire/drop on the same path leaves only the canonical file.

> **Distinct from the owned-record locks.** The kernel lock above is *not* the lock that serializes local data mutations. `LocalDataLock` and `TransactionLock` are owned-record locks (PID + nonce + start token + PID-reclaim) implemented in `src/local_data.rs` and `src/transaction.rs`. They are described in [Lock Hierarchy](#lock-hierarchy), [Local-Data Lock](#local-data-lock), and [TransactionLock](#transactionlock).

---

## Lock Hierarchy

snip-it uses **three** distinct lock mechanisms. Confusing them is the source of most lock-ordering bugs.

| Lock | Kind | Location | Authority |
|------|------|----------|-----------|
| `LocalDataLock` | Owned record (`create_new` + PID reclaim) | `<config>/.transaction/local-data.lock` | Reclaimable owned record |
| `TransactionLock` | Owned record (`create_new` + PID reclaim) | `<config>/.transaction/transaction.lock` | Reclaimable owned record |
| `ProcessFileLock` | Kernel (`flock` / `LockFileEx`) | auto-sync execution/worker/pending locks, `snip-sync` server singleton | Kernel only |

### Acquisition Order

**`LocalDataLock` → `TransactionLock` → destination writes.** Never invert.

This ordering is stated in the source itself: `src/commands/restore_cmd.rs:1133` reads
`// Lock hierarchy: LocalDataLock -> TransactionLock -> destination writes.`

The mutation gate runs *before* either lock is taken. The gate itself may acquire the transaction lock internally (via `recover_transaction_by_id` → `acquire_transaction_lock`, `src/transaction.rs:782`), but it releases it before returning — so the gate never holds the transaction lock while another lock is being acquired, and there is no inversion.

### Call-Site Map

| Call site | Gate | `LocalDataLock` | `TransactionLock` |
|-----------|------|-----------------|-------------------|
| `library::persistence::save_library` | Yes (`persistence.rs:266-269`) | Yes (`persistence.rs:274`) | — |
| `commands::save_snippets` | Yes (`commands/mod.rs:212-215`) | Yes (`commands/mod.rs:218`) | — |
| `LibraryManager` mutators | Yes (`manager.rs:666-672`) | Yes (`manager.rs:657-660`) | — |
| `commands::edit_cmd` | Yes (`edit_cmd.rs:73`) | Yes (`edit_cmd.rs:101`) | — |
| `commands::restore_cmd` | Yes (`restore_cmd.rs:1142-1145`) | Yes (`restore_cmd.rs:1146`) | Yes (`restore_cmd.rs:1147`) |
| `commands::backup_cmd` | — | Yes (`backup_cmd.rs:78`) | — |
| `usage` | — | Yes (`usage.rs:74`) | — |
| `config::sync_settings` | — | Yes (`config/sync_settings.rs:473`) | — |
| `transaction::recover_transaction_by_id` | — | — | Yes, first (`transaction.rs:782`) |

Every `LibraryManager` mutator follows the same shape — `gate_mutation()` then `acquire_local_data_lock()` — at `manager.rs:184-185, 364/378, 430-431, 500-501, 524-525, 537-538, 556-557, 571-572, 585-586, 612-613, 653-654`. `gate_mutation()` runs *before* any in-memory mutation so a refused call leaves the manager clean for a retry after `snp repair` (`manager.rs:662-672`). Two mutators deviate: `create_library` takes the lock at `:378`, after `ensure_library_mode()` at `:376` (gate at `:364`), and `save_premade_library` takes the local-data lock only, with no gate (`manager.rs:747-749`).

Two commands acquire no lock of their own and inherit the table's rows **transitively**, so look them up here before concluding they are unlocked:
- `snp repair` takes the transaction lock only inside `recover_transaction_by_id` (`transaction.rs:782`) and the local-data lock only inside `UsageIndex::save()` (`usage.rs:74`).
- `snp import` reaches both through `save_library` (`persistence.rs:258-279`) and, for a new library, `add_existing_library` (`manager.rs:602-603`).

`snp edit` is the one deliberate exception to "gate, then lock": the gate runs at `edit_cmd.rs:73`, the editor session runs unlocked, and the local-data lock is taken only for the short post-editor observe-and-notify window (`edit_cmd.rs:97-101`). The interactive session cannot hold the lock; only the read-compare-notify critical section needs to be serialized against other writers.

### Why the Local-Data Lock Exists

`LocalDataLock` is the *snapshot-consistency* lock, not the *mutation* lock. Backup captures a set of files while they may be being rewritten; holding the local-data lock for the duration of the snapshot guarantees backup sees either the complete before-state or the complete after-state, never a mixed one (`backup_cmd.rs:74-78`, `persistence.rs:271-274`).

---

## Transaction Boundary

### Location

`src/transaction.rs`

### Purpose

Coordinates multi-file operations (library create/delete, bulk import, restore, repair) with crash-safe journaling. The transaction lock prevents concurrent mutations.

### Directory Model

The transaction subsystem uses two distinct directories:

- **`sync_state_dir`** (canonical config directory, e.g. `~/.config/snp/`): Where the pending sync marker (`auto-sync-pending.toml`) lives. Pending APIs must receive this directory.
- **`transaction_dir`** (`<sync_state_dir>/.transaction`): Where journals, locks, durable backups, and staged files live. Transaction APIs must receive this directory.

This separation ensures the pending marker is never written to the `.transaction` subdirectory, which would cause it to be missed by the canonical pending path. The `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` function requires both directories: it inspects the canonical pending marker (in `sync_state_dir`) while cleaning up transaction artifacts (in `transaction_dir`).

The canonical helper for the second directory is `local_data::transaction_dir()` (`src/local_data.rs:398-400`), which returns `<config_dir>/.transaction`. Prefer it over re-deriving the `.transaction` suffix by hand.

### State Machine

```
Prepared → Committing{next_commit_position} → CleaningUp{outcome, next_step} → (journal removed)
Prepared → RollingBack{next_rollback_position} → CleaningUp{outcome, next_step} → (journal removed)
Committing → CommittedLocal{pending} → CleaningUp → (journal removed; restore pending-finalization path)
Prepared → Failed(error_message) (persisted terminal; recovery refuses without repair)
```

New transactions never persist terminal `Committed` or `RolledBack` states. The journal is removed during cleanup, making the absence of a journal the true terminal indicator. Legacy `Committed`, `RolledBack`, and `BackupsDurable` journals (from older versions) are handled during recovery. `CommittedLocal` is live on the restore path (see below) and also handled for legacy journals via `FinalizeCommittedLocal`.

The persisted enum is `TransactionState` (`transaction.rs:306-354`); `is_interruptible()` (`transaction.rs:363-373`) lists the interruptible states: `Prepared`, `BackupsDurable`, `Committing`, `CommittedLocal`, `RollingBack`, `CleaningUp`.

### Components

#### TransactionJournal

Persisted as `txn-<uuid>.toml` in the `.transaction` subdirectory of the state directory:

```rust
pub struct TransactionJournal {
    pub id: String,                    // UUID
    pub operation: String,             // e.g. "library_delete", "bulk_import"
    pub created_at_unix_ms: i64,
    pub staged_files: Vec<StagedFile>,
    pub state: TransactionState,
}
```

#### StagedFile

```rust
pub struct StagedFile {
    pub original_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub staged_path: PathBuf,
    pub sha256: String,
    pub existed_before: bool,
    pub action: StagedAction,
    pub original_hash: String,
    pub new_hash: String,
    pub durable_staged_path: Option<PathBuf>,
    pub original_metadata: OriginalFileMetadata,
}
```

`staged_path` is the live destination. `backup_path` holds the original bytes used for rollback, `durable_staged_path` a private durable copy of the intended bytes, and `original_metadata` the pre-transaction permission/metadata snapshot.

`StagedAction` (`transaction.rs:165-175`) is `Replace | Create | Delete | NoOp`.

#### TransactionLock

File-create guard ensuring exclusive access. `acquire_transaction_lock(state_dir, operation)` creates `transaction.lock` via `create_new(true)`. The lock file contains a TOML record with `schema_version`, `pid`, `nonce`, `created_at_unix_ms`, `operation`, and `start_token` fields. On acquisition, if the lock already exists, the system checks PID liveness via `ProcessIdentity::observe(existing.pid)` — dead owners are reclaimed, live owners cause an error. **Ownership verification**: the system observes the process at `existing.pid` and compares the observed start token with the persisted start token, not the contender's own start token. This prevents a live owner from being classified as PID reuse. Ownership is verified on `Drop`: the lock file is only removed if the stored nonce, PID, and start token match the guard's nonce, PID, and start token, preventing old owners from removing a replacement owner's lock. Malformed locks are quarantined (renamed to `transaction.lock.quarantine.<uuid>`) rather than silently deleted.

**Bounded retry ladder.** A record that is *empty, unparseable, or only partially written* goes through a bounded retry ladder before quarantine, because "a live writer called `create_new` but has not finished writing" is an observable state, and a partially written record is indistinguishable from a malformed one; quarantining immediately would rename away a live owner's lock and admit a second writer into the critical section.

The ladder has three arms (`transaction.rs:1151-1201`):

| Arm | Condition | Bound | Action |
|-----|-----------|-------|--------|
| Readable | `toml::from_str` succeeds | — | Resets **both** counters (`transaction.rs:1152-1160`) |
| Empty | parse fails **and** `content.trim().is_empty()` | quarantine when `empty_retries > 200`, or `> 50` and the file is older than 100 ms | `quarantine_stale_lock` (`transaction.rs:1161-1180`) |
| Malformed / partially written | parse fails and content is non-empty | quarantine when `malformed_retries > 200` | `quarantine_stale_lock` (`transaction.rs:1181-1200`) |

Both retry arms sleep 1 ms between attempts. A *readable* record resets both counters, so a waiter that sees scattered empty reads over a long legitimate acquisition never eventually quarantines a perfectly good lock.

**Transient contention is platform-gated.** `is_transient_lock_contention` (`transaction.rs:1029-1047`) returns `true` only on Windows for `PermissionDenied`: a just-deleted file can briefly surface `PermissionDenied` in a pending-delete state. On Unix the same condition is *stable* (lock file owned by another UID, or a non-traversable `.transaction` directory), so retrying it spins forever with no error and no timeout. The `#[cfg(windows)]` / `#[cfg(not(windows))]` split is the fix for a hang that applied the Windows workaround unconditionally.

**Deadline.** `LOCK_ACQUIRE_TIMEOUT` is 30 s (`transaction.rs:1023-1027`). The deadline is checked in the "lock removed or transient read" arm (`transaction.rs:1131-1140`) and returns `Transaction lock acquisition timed out`. The empty and malformed arms are bounded by their retry counters instead, not by the deadline.

**A failed `write_all` no longer leaves a 0-byte lock file behind.** If the record write fails after `create_new` succeeded, the file this call created is removed before the error is returned (`transaction.rs:1097-1104`); otherwise every later acquirer would spin through the empty-content ladder on a file that can never become valid.

**Quarantine is rename-based and GC'd.** `quarantine_stale_lock` (`transaction.rs:1268`) renames the lock to `transaction.lock.quarantine.<uuid>`; a `NotFound` from a concurrent writer's quarantine is treated as success. Old quarantine files are pruned by `prune_quarantine_files` with `QUARANTINE_RETENTION` = 7 days (`transaction.rs:1300-1301`).

### API

| Function | Description |
|----------|-------------|
| `acquire_transaction_lock(state_dir, operation)` | Acquire exclusive lock, error if held or after the 30 s deadline |
| `begin_transaction(state_dir, operation, affected_files)` | Create journal in `Prepared` state |
| `commit_transaction(state_dir, journal)` | Begin cleanup phase; removes backups and journal |
| `rollback_transaction(state_dir, journal)` | Restore files from backups in reverse order |
| `check_interrupted_transactions(state_dir)` | Find journals in interruptible states on startup |
| `scan_transaction_journals(transaction_dir)` | Authoritative discovery of *every* journal, including corrupt ones |
| `classify_journal_recovery(transaction_dir, journal)` | Map a journal to a `RecoveryClass` |
| `recover_transaction_by_id(sync_state_dir, transaction_dir, id, expected)` | Exact, lock-held, re-validated recovery of one journal |

### The Mutation Gate

`gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` (`transaction.rs:2510-2614`) is the mandatory pre-mutation check. Its policy:

1. **Corrupt journals** → fail closed. Mutations are refused with a pointer to `snp repair` (`transaction.rs:2516-2535`).
2. **`UnsafeFailed` journals** → fail closed; the journal is preserved for manual investigation (`transaction.rs:2547-2562`).
3. **Zero actionable journals** → terminal journals with no artifacts are removed one at a time through exact recovery, each under its own transaction lock with re-validation (`transaction.rs:2570-2591`).
4. **Exactly one actionable journal** → auto-recover it (auto-rollback for `Prepared`/`RollingBack`, finalize for `CommittedLocal`, resume cleanup for `CleaningUp`) (`transaction.rs:2593-2601`).
5. **Two or more actionable journals** → refuse and direct the user to `snp repair` (`transaction.rs:2603-2613`).

`check_interrupted_transactions` is a **scan helper, not the gate**. Calling the scan and proceeding anyway reproduces the corruption it exists to prevent.

`RecoveryClass` (`transaction.rs:402-418`): `Rollback`, `FinalizeCommittedLocal`, `ResumeCleanup`, `CleanupLegacyCommitted`, `CleanupLegacyRolledBack`, `RemoveTerminalJournal`, `UnsafeFailed`.

### Crash Recovery

`scan_transaction_journals()` scans the `.transaction` subdirectory of the state directory for `txn-*.toml` files and reports both parsed journals and corrupt ones. Journals in interruptible states (`Prepared`, `BackupsDurable`, `Committing`, `CommittedLocal`, `RollingBack`, `CleaningUp`) represent interrupted operations. The `snp repair` command detects these and offers automatic rollback.

`recover_transaction_by_id` (`transaction.rs:771-846`) is the canonical transaction-specific recovery API. It validates the transaction ID, **acquires the transaction lock first**, loads and classifies the journal *under* the lock, compares the actual classification against the expected one *under* the lock (a mismatch returns `stale repair action` and asks the user to re-run `snp repair`), and only then executes the recovery path — all while holding the lock.

### Journal Lifecycle

1. `begin_transaction` writes journal via `write_private_atomic` in `Prepared` state
2. Caller performs live replacements, advancing through `Committing { next_commit_position }` per file — progress persisted only after each verified atomic write
3. After all writes complete, advance to `CleaningUp { outcome: Commit, next_step }` — records pending sync intent, then removes backups and journal
4. If interrupted between begin and cleanup completion, the scanner finds the orphan in any interruptible state (`Prepared`, `BackupsDurable`, `Committing`, `CommittedLocal`, `RollingBack`, `CleaningUp`). Legacy `CommittedLocal` and `BackupsDurable` journals from older versions are handled during recovery.

### Commit Progress Semantics

`Committing { next_commit_position }` uses completed-position semantics: `next_commit_position == N` means positions `0..N` have already been installed and verified; position `N` is next. Progress is persisted only after install and verification, never before.

### Rollback Order

`RollingBack { next_rollback_position }` uses rollback-order coordinates: `rollback_order = (0..files.len()).rev()`. `next_rollback_position == N` means positions `0..N` have been rolled back. Each rollback action verifies the pre-transaction bytes or expected absence after completion.

### Artifact Path Validation

All transaction artifact paths (backup, staged, destination) are validated by `validate_contained_path` (`transaction.rs:1520`) before use. The validation has three layers:

1. **Lexical containment** (`lexically_within`, `transaction.rs:1594`): Both paths must be absolute. `Component::ParentDir` is explicitly rejected during normalization via `normalize_absolute_without_parent` (`transaction.rs:1625`) — any `..` component causes immediate rejection. This catches traversal even when the path doesn't exist yet (e.g. `<artifact-root>/../../outside.bin`).
2. **Symlinked prefix rejection** (`reject_symlinked_existing_prefixes`, `transaction.rs:1649`): Walks from root toward child using `symlink_metadata` (not `fs::metadata`, so symlinks are not followed). If any existing intermediate component is a symlink, the path is rejected. This catches `<root>/link/missing.bin` where `link` is a symlink to outside and the final file is absent.
3. **Canonical containment**: For existing paths, canonicalize both root and path and verify containment as defense in depth.

The helper runs for every transaction state (not just on disk existence), and backup references are revalidated immediately before reading in rollback.

Per-transaction artifacts live under `.transaction/artifacts/<txn-id>/` with `backups/` and `staged/` subdirectories (`transaction_artifact_dir`, `transaction.rs:1737`; used by `restore_cmd.rs:1179-1193`).

### CommittedLocal State (restore path + legacy)

`CommittedLocal { pending }` is the restore pending-finalization state (`src/commands/restore_cmd.rs` advances through `advance_to_committed_local` before `commit_transaction`). New non-restore transactions transition directly from `Committing` to `CleaningUp`. Older-version `CommittedLocal` journals are handled during recovery via `FinalizeCommittedLocal`, which completes the pending intent recording.

---

## Local-Data Lock

### Location

`src/local_data.rs`

### Purpose

`LocalDataLock` is the *snapshot-consistency* lock: it serializes config mutations against backup snapshot capture, so a backup never observes a half-applied multi-file change. It lives in the same `.transaction` directory as the journal and the transaction lock, but is a separate file with a separate record.

### Record

```rust
pub struct LocalDataLockInfo {
    pub schema_version: u32,
    pub pid: u32,
    pub nonce: String,
    pub created_at_unix_ms: i64,
    pub start_token: Option<String>,
}
```

`LocalDataLockInfo` derives `PartialEq, Eq` precisely so the reclaim path can compare the record it observed against the record currently on disk (`local_data.rs:35`).

### Acquisition

`acquire_local_data_lock(state_dir)` (`local_data.rs:111`) uses the same owned-lock protocol as the transaction lock: `create_new(true)` on `local-data.lock`, immediately write the TOML record, `sync_all` (warn-only on failure), and return the guard.

- **Deadline** — 30 s from entry (`local_data.rs:128`). Exceeding it while the owner is alive returns `Local data lock held` / `Timed out waiting for local data lock after 30 seconds.` (`local_data.rs:282-287`).
- **Backoff** — 10 ms doubling to a 1 s cap while a live owner holds the lock (`local_data.rs:288-289`).
- **Transient contention** — the same platform-gated `is_transient_lock_contention` as the transaction lock (`local_data.rs:174, 184`).

### Bounded Retry Ladder

The local-data lock uses the same three-arm ladder as the transaction lock (`local_data.rs:199-258`):

| Arm | Condition | Bound | Action |
|-----|-----------|-------|--------|
| Readable | parse succeeds | — | Resets both counters (`local_data.rs:200-208`) |
| Empty | parse fails and content is empty | quarantine when `empty_retries > 200`, or `> 50` and the file is older than 100 ms | `quarantine_local_data_lock(&lock_path, None)` (`local_data.rs:209-241`) |
| Malformed / partially written | parse fails and content is non-empty | quarantine when `malformed_retries > 200` | `quarantine_local_data_lock(&lock_path, None)` (`local_data.rs:242-257`) |

The empty arm additionally checks the 30 s deadline and returns a typed error naming the likely cause — a writer interrupted between creating and writing the lock file (`local_data.rs:229-238`).

**Before this ladder existed, a 0-byte `local-data.lock` hung every mutation forever.** A 0-byte file can arise from a failed `write_all` or from SIGKILL between `create_new` and `write_all`. Because the record can never become valid, no reclaim path was reachable and every later acquirer spun indefinitely with no error and no timeout.

**A failed `write_all` deletes the file it created.** The record write removes the just-created 0-byte lock before returning the error (`local_data.rs:152-163`), so the poison file never reaches the ladder.

### Ownership-Verified Reclaim

Stale-lock reclaim renames the lock file. Reading the record and checking the owner's liveness are **not atomic**, so a writer that decided to reclaim could rename away a *live* lock that another writer had already reclaimed and re-acquired in the meantime — silently putting two processes inside the critical section.

`quarantine_local_data_lock(lock_path, observed)` (`local_data.rs:322-354`) therefore re-reads the record and compares it to the one the caller observed before renaming. If the record changed, or is no longer parseable, the rename is **skipped** and the current owner is left alone. `observed = None` is used only for the empty/malformed arms, where there is no owner identity to compare — the same trade-off `Drop` makes for unparseable content.

Both reclaim call sites pass the observed record: dead owner (`local_data.rs:268`) and PID reuse via start-token mismatch (`local_data.rs:297`).

Quarantine files are named `local-data.lock.quarantine.<uuid>` and pruned by `prune_quarantine_files` with the same 7-day retention (`local_data.rs:356-370`).

### Relationship to Backup Snapshot Capture

`snp backup` acquires the local-data lock for the duration of snapshot capture (`backup_cmd.rs:74-78`), reading the library generation before and after the copy so a concurrent mutation is detected as a torn snapshot. The mirror-image rule holds on the write side: `save_library` (`persistence.rs:271-274`), `save_snippets` (`commands/mod.rs:217-218`), and every `LibraryManager` mutator (`manager.rs:657-660`) take the same lock, so backup sees either the complete before-state or the complete after-state of all local data, never a mixed state.

---

## Validation Framework

### Location

`src/commands/validate_cmd.rs`

### Diagnostic Model

```rust
pub struct ValidationDiagnostic {
    pub code: String,           // e.g. "E-DUP-ID", "W-ID-EMPTY"
    pub severity: Severity,     // Info | Warning | Error
    pub path: Option<PathBuf>,
    pub library: Option<String>,
    pub snippet_id: Option<String>,
    pub message: String,
    pub repairability: Repairability,  // Auto | Manual | Unrepairable
}
```

```rust
pub struct ValidationReport {
    pub schema_version: String,
    pub tool_version: String,
    pub strict_mode: bool,
    pub dry_run: bool,
    pub total_libraries: usize,
    pub total_snippets: usize,
    pub diagnostics: Vec<ValidationDiagnostic>,
}
```

### Check Categories

| Code | Severity | Description |
|------|----------|-------------|
| `E-FILE-READ` | Error | Failed to read library file |
| `E-TOML-PARSE` | Error | TOML syntax error |
| `E-DUP-ID` | Error | Duplicate snippet IDs within a library |
| `E-CMD-EMPTY` | Error | Snippet has empty command |
| `E-INDEX-MISSING-FILE` | Error | Library registered in index but file missing |
| `E-PRIMARY-MISSING` | Error | Primary library file does not exist |
| `I-FILE-EMPTY` | Info | Library file is empty |
| `W-ID-EMPTY` | Warning | Snippet has empty ID (load assigns IDs) |
| `W-DESC-EMPTY` | Warning | Snippet has empty description |
| `W-SAME-ID-DIVERGENT` | Warning | Same ID appears with different content |
| `W-DUP-CHECK-SKIPPED` | Warning | Same-ID divergent-content check skipped (raw TOML re-parse failed) |
| `W-EXACT-DUP` | Warning | Exact duplicate snippet (same description + command) |
| `W-ORPHAN-FILE` | Warning | File in `libraries/` not in index |
| `W-NO-PRIMARY` | Warning | No primary library set |
| `W-LIB-UNREADABLE` | Warning | Library file unreadable; excluded from the usage-orphan check |
| `W-LIB-INDEX-UNREADABLE` | Warning | Library index unreadable; usage-orphan check skipped |
| `W-USAGE-ORPHAN` | Warning | Usage entry references deleted snippet |
| `W-INSECURE-PERMS` | Warning | Config file has world-readable/group-writable bits |
| `W-CORRUPT-BAK` | Warning | Corrupt backup file exists |

### Strict Mode

In strict mode, designated warning codes are elevated to errors: `W-ID-EMPTY`, `W-DESC-EMPTY`, `W-SAME-ID-DIVERGENT`, `W-EXACT-DUP`.

### Output

- Human-readable to stderr (grouped by severity)
- JSON to stdout (`--json` flag)
- Exit code 2 if any errors, 0 otherwise

---

## Backup Format

### Location

`src/commands/backup_archive.rs` (manifest, path validation, atomic staging); `src/commands/backup_cmd.rs` owns the CLI orchestration, lock acquisition, and output reporting.

### Manifest

TOML or JSON (format depends on `BackupFormat`):

```rust
pub struct BackupManifest {
    pub schema: u32,
    pub created_at_unix_ms: i64,
    pub snip_it_version: String,
    pub layout: String,                    // "directory" or "archive"
    pub files: Vec<BackupManifestEntry>,
}

pub struct BackupManifestEntry {
    pub path: String,
    pub kind: BackupEntryKind,         // library | index | usage | sync_config
    pub size: u64,
    pub sha256: String,
}
```

### Default Inclusions

| Kind | Source | Required |
|------|--------|----------|
| `library` | `~/.config/snp/libraries/*.toml` | Yes (if exists) |
| `index` | `~/.config/snp/libraries.toml` | Yes (if exists) |
| `usage` | `~/.config/snp/usage.toml` | Optional (`--include-usage`) |
| `sync_config` | `~/.config/snp/sync.toml` | Optional (`--include-sync-state`), API key redacted |

### Default Exclusions

- API keys, encryption keys, credentials
- Lock files, logs, caches, temp files
- Pending mutation markers, auto-sync status
- Theme files, premade libraries
- Transaction journals

### Secret Redaction

`redact_sync_config()` redacts `api_key`, `ApiKey`, and `api-key` lines, replacing values with `<redacted>`.

### Backup Locations

| Flag | Location |
|------|----------|
| Default | `~/.config/snp/backups/<timestamp>/` |
| `--output <path>` | User-specified directory |

### Snapshot Consistency

The backup acquires the local-data lock before reading any file (`backup_cmd.rs:74-78`) and records the library generation before and after the capture so a concurrent mutation is detectable. Files are staged into a `<final>.staging.<uuid>` directory and renamed into place, so a partial backup is never visible at the final path (`backup_cmd.rs:65-72`).

### Integrity

Each file in the backup has a SHA-256 digest recorded in the manifest. Restore verifies checksums before applying.

---

## Restore Semantics

### Location

`src/commands/restore_cmd.rs`

### Modes

| Mode | Behavior |
|------|----------|
| `DryRun` | Preview planned actions without changes |
| `Merge` | Combine with existing data, report conflicts |
| `Replace` | Full replacement with pre-restore backup |

### Restore Flow

1. Gate against foreign interrupted transactions (`restore_cmd.rs:1142-1145`)
2. Acquire `LocalDataLock` (`restore_cmd.rs:1146`)
3. Acquire the transaction lock (`restore_cmd.rs:1147`)
4. Begin transaction journal (`begin_transaction`, `restore_cmd.rs:1168`)
5. Load and validate manifest (`manifest.toml` or `manifest.json`)
6. Validate every source artifact (checksum, size, symlink rejection)
7. Validate every destination path (traversal, reserved names, kind constraints)
8. Parse incoming TOML files before any live write
9. Validate duplicate snippet IDs (`validate_library_no_duplicate_ids`)
10. Load every affected current file
11. Compute full restore plan in memory (detect conflicts, produce deterministic report)
12. Create durable backups for every existing destination under `.transaction/artifacts/<txn-id>/backups/` (`restore_cmd.rs:1179-1186`)
13. Create durable staged files under `.transaction/artifacts/<txn-id>/staged/` (`restore_cmd.rs:1191-1193`)
14. fsync files and required parent directories according to durability class
15. Populate all journal fields, including hashes and action; advance to `BackupsDurable` (`restore_cmd.rs:1295`)
16. Advance to `Committing { next_commit_position: 0 }` (`restore_cmd.rs:1304`) and perform live replacements via `atomic_replace` — `Durability::DurableUserData` with `preserve_permissions` for existing destinations, `Durability::SensitiveConfig` for newly created ones — persisting `Committing { next_commit_position }` only after each verified write (`restore_cmd.rs:1561`)
17. Advance to `CommittedLocal { pending: PendingFinalization }` — records pending sync intent atomically (`restore_cmd.rs:1636, 1699`)
18. Mark journal committed only after all live writes succeed and pending intent is recorded (`commit_transaction`, `restore_cmd.rs:1713, 1732`)
19. Release transaction lock
20. Release `LocalDataLock`
21. Schedule auto-sync once, after commit, if policy permits
22. Clean backups and journal according to retention policy

Restore uses `save_library_internal` (not `save_library`) for the libraries it rewrites: it already holds both locks, and the global mutation gate would otherwise see restore's *own* journal as interrupted and roll it back mid-flight.

### Merge Strategy

For each library file already present:
- If content is identical → skip
- Load both versions, merge snippets by ID
- Prefer newer `updated_at` for conflicting IDs
- Add new snippets from backup that don't exist locally
- Report all conflict resolutions

### Replace Strategy

- Full file copy for each backup entry
- Pre-restore backup created before any writes
- `sync.toml` restored with redacted API key (user must re-register)

### Conflict Report

```rust
pub struct RestoreConflict {
    pub library: String,
    pub kind: String,      // "updated", "kept_existing", "added", "replaced", "redacted_key"
    pub detail: String,
}
```

---

## Repair Command

### Location

`src/commands/repair_cmd.rs`

### Purpose

Conservative, backed-up, idempotent repair. Validates configuration and library files, identifies safe repair candidates, and applies fixes only when explicitly requested.

### Repair Items

```rust
pub struct RepairItem {
    pub action: RepairAction,   // typed action (usage/index/ids/timestamps/transaction …)
    pub category: String,       // "usage", "transaction", "config", "primary", "ids", "timestamps", "unsafe"
    pub problem: String,
    pub fix: String,
    pub safe: bool,             // Whether safe for auto-apply
    pub target_path: Option<PathBuf>,
}
```

### Categories

| Category | Safe | Description |
|----------|------|-------------|
| `usage` | Yes | Prune orphaned usage entries |
| `transaction` | Yes | Roll back interrupted transactions |
| `ids` | No | Regenerate empty/duplicate IDs (requires library context) |
| `timestamps` | No | Fix zero timestamps (requires library context) |
| `primary` | Yes (single lib) | Auto-assign primary when only one library exists |
| `primary` | No (multiple) | Prompt user to choose primary |
| `config` | No | TOML corruption requiring manual inspection |
| `unsafe` | No | Corrupt journal or unsafe transaction artifacts; journal preserved for manual quarantine |

### Modes

- `--dry-run`: Analyze and print planned repairs
- `--apply`: Create pre-repair backup, apply safe repairs, emit report
- Neither: Print validation summary only

### Backup Before Repair

`snp repair --apply` always creates a timestamped backup at `~/.config/snp/backups/repair-<timestamp>-<uuid>/` before any mutations (`repair_cmd.rs:214-216, 872-905`). The backup copies the whole `libraries/` directory plus `libraries.toml` and `usage.toml`.

Transaction recovery inside repair goes through `recover_transaction_by_id`, so each journal is classified, re-validated, and recovered under the transaction lock.

---

## Migration Framework

### Location

`src/migration.rs`

### Schema Versioning

```rust
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    pub const LEGACY: SchemaVersion = SchemaVersion(0);
    pub const CURRENT: SchemaVersion = SchemaVersion(1);
}
```

Version 0 represents unversioned/legacy files. Version 1 is the current schema. The version is stored as `schema_version = <N>` in the TOML top-level table (key constant `SCHEMA_KEY`, `migration.rs:99`).

`get_schema_version` parses the file as a `toml::Table` so a document with multiple root keys is handled; a missing key, a missing file, an empty file, or an unparseable file all read back as `LEGACY` (`migration.rs:113-147`).

### Migration Trait

```rust
pub trait Migration {
    fn source(&self) -> SchemaVersion;
    fn target(&self) -> SchemaVersion;
    fn analyze(&self, path: &Path) -> SnipResult<MigrationPlan>;
    fn apply(&self, plan: &MigrationPlan, path: &Path) -> SnipResult<MigrationOutput>;
}
```

### Migration Operations

```rust
pub enum MigrationOperation {
    RenameField { table, from, to },
    AddField { table, name, default },
    RemoveField { table, name },
    Transform { description },
}
```

### Key Functions

| Function | Description |
|----------|-------------|
| `get_schema_version(path)` | Read `schema_version` from TOML file |
| `needs_migration(path)` | Check if file version < `CURRENT` |
| `write_schema_version(path, version)` | Write version using `toml::Table` for roundtripping |
| `run_migrations(path, migrations)` | Apply migration chain in order |

### Roundtripping

`write_schema_version` parses the file as `toml::Table`, inserts the version key, and serializes back (`migration.rs:153-180`). This preserves array-of-tables structure and other TOML constructs that naive string insertion would corrupt.

**`toml::Table`, not `toml::Value`.** `toml::Value` collapses array-of-tables — it is the exact failure the `toml::Table` choice exists to prevent.

### Migration Chain

`run_migrations` iterates through registered migrations in order. Each migration's `source` must match the current version. The chain advances from `LEGACY` → `CURRENT`.

Each migration is **analyzed first**; the plan is only applied when it has operations. A best-effort pre-migration backup (`<path>.toml.pre-migration.bak`) is written once, before the first mutating step. If any step fails mid-chain, the file is restored from that backup, so the schema is never left at an intermediate version with half-applied data (`migration.rs:188-191, 205-217, 225-245`).

### Current Usage

`write_schema_version`, `needs_migration`, and `run_migrations` currently have **no production call sites** — they are exercised only by `src/migration.rs`'s own tests. They remain the canonical schema-versioning API and the template any future schema change must follow.

---

## Identity Contract

### Location

`docs/IDENTITY_CONTRACT.md`

### Snippet Identity

- Opaque string identifier, assigned deterministically on load for legacy snippets
- UUID v4 for snippets created by import
- Never regenerated for a given snippet
- Retained across edit, move, export, sync, and restore
- New ID assigned on import (existing IDs discarded)
- Deduplication on load: first duplicate keeps original, later duplicates get deterministic replacement IDs

### ID Assignment Points

1. **`load_library()`** — assigns deterministic `legacy-<sha256hex>` IDs to empty IDs and makes duplicate IDs unique on load
2. **`commands::import_cmd`** — assigns UUID for imported snippets
3. **`doctor_cmd`** — reports planned ID regeneration (diagnostic only)

Note: `Snippet::new()` creates a snippet with an empty `id`. The deterministic ID is assigned when the library is next loaded. The next normal `save_library()` persists the normalized IDs.

### Deterministic Legacy IDs

For legacy snippets with missing IDs, `load_library()` generates deterministic provisional IDs using SHA-256 (`src/library/persistence.rs:128-157`). The hashed preimage is:

```
"snip-it-legacy-id-v1\0"
+ description + "\0"
+ command + "\0"
+ (tag + "\0")*
+ output + "\0"
+ (folder + "\0")*
+ favorite-as-one-byte + "\0"
+ device_id + "\0"
+ occurrence.to_le_bytes()
```

For a **duplicate explicit ID** the preimage is (`persistence.rs:160-190`):

```
"snip-it-duplicate-id-v1\0"
+ original_id + "\0"
+ description + "\0"
+ command + "\0"
+ (tag + "\0")*
+ output + "\0"
+ (folder + "\0")*
+ favorite-as-one-byte + "\0"
+ device_id + "\0"
+ occurrence.to_le_bytes()
```

Both are rendered as `legacy-<64 lowercase hex chars>`. Note that `folders`, `favorite`, and `device_id` participate in the digest even though they are local-only / sync-metadata fields — see [library.md](library.md) for the local-only vs synced split.

**Every ID the file already carries is reserved before any provisional ID is
minted.** A snippet that already holds `legacy-<hash(content, 0)>` does not
consume occurrence index 0 for its content, so without the reservation a later
sibling with the same content and an empty ID would be handed the very same
ID — two snippets sharing one identity, one of them unreachable by ID and
collapsed into a single row on the server. The reservation pass makes the
outcome independent of the order the two appear in the file.

This ensures repeated loads of identical file content produce identical IDs without rewriting the user's TOML file.

### Library Identity

- Primary key: `filename` (without `.toml` extension) in `libraries.toml`
- Server ID: Optional `library_id` for sync linkage
- Primary flag: `is_primary` boolean — exactly one library is primary
- Filename is immutable after creation (no rename command)

### Lifecycle Rules

| Operation | Snippet ID | Library ID |
|-----------|-----------|------------|
| Edit | Retains | N/A |
| Move between libraries | Retains | N/A |
| Export (native) | Includes | N/A |
| Import (native reimport) | New UUID assigned | N/A |
| Import (external, no ID) | New UUID assigned | N/A |
| Delete | `deleted=true`, retains as tombstone | Removed from index |
| Recreate | New UUID (never reuses deleted) | New entry |
| Restore | Retains (duplicates resolved at load) | New if collision |
| Sync | Same ID across devices | `library_id` linkage |

---

## Security Properties

- All sensitive files created with 0o600 permissions
- Config directory created with 0o700 permissions
- Kernel-backed locks (the auto-sync execution lock, the sync-server singleton) use the process file lock (`flock`/`LockFileEx`) for atomic acquisition; the kernel state is the sole mutual-exclusion authority and ownership is never reused based on PID liveness inspection
- Owned-record locks (`transaction.lock`, `local-data.lock`) do **not** use a kernel lock: they are created with `O_EXCL` (`create_new(true)`) plus a TOML owner record (`transaction.rs:1085-1088`, `local_data.rs:140-143`). A stale owner is reclaimed by observing the recorded PID and comparing start tokens (`transaction.rs:1205-1247`, `local_data.rs:261-299`)
- Auto-sync lock ownership uses start-token + nonce diagnostics only; the kernel state is the sole mutual-exclusion authority
- Atomic writes: temp-file-then-rename with `validate_target` (rejects FIFOs, sockets, devices)
- Transaction artifact path validation rejects `Component::ParentDir` during lexical normalization and rejects missing children below symlinked intermediate components
- Transaction journals use UUID-based filenames and O_EXCL locks
- Backup checksums: SHA-256 per file, verified before restore
- Backup redaction: API keys stripped from sync.toml copies
- Owned-record lock reclaim is **ownership-verified** for the local-data lock: the record on disk is re-read and compared before the rename (`local_data.rs:322-354`), so a live lock is never renamed away by a racing reclaimer
- Lock records are `0o600`-scoped by construction: the config directory is `0o700` and the temp/record files inherit that
- `SNP_ALLOW_DIR_FSYNC_FAILURE` is inert in production builds (`atomic.rs:217-226`)

---

## Key Files

| File | Subject |
|------|---------|
| `src/utils/atomic.rs` | Atomic write primitive, durability classes |
| `src/utils/toml_helpers.rs` | Escape helpers for hand-written TOML only |
| `src/utils/tempfile_guard.rs` | RAII temp-file cleanup used by the atomic write path |
| `src/utils/config.rs` | Config-dir resolution, `ScopedConfigDir` test override, `derive_sync_state_dir` |
| `src/process_file_lock.rs` | Kernel-backed cross-process file lock primitive |
| `src/auto_sync/execution_lock.rs` | Auto-sync execution lock and worker lock wrappers |
| `src/auto_sync/pending_lock.rs` | Auto-sync pending-marker mutex wrapper |
| `snip-sync/src/server_lock.rs` | snip-sync server singleton kernel lock |
| `snip-sync/src/process.rs` | Legacy PID parser and identity checks for compatibility stop/restart |
| `src/transaction.rs` | Transaction boundary, journaling, transaction lock, rollback, mutation gate |
| `src/local_data.rs` | `LocalDataLock`, snapshot-consistency lock, directory derivation |
| `src/commands/validate_cmd.rs` | Validation framework, diagnostic model, 12+ check categories |
| `src/commands/backup_archive.rs` | Backup manifest, path validation, secret redaction, SHA-256 integrity, atomic staging |
| `src/commands/backup_cmd.rs` | Backup CLI orchestration, lock acquisition, output reporting |
| `src/commands/restore_cmd.rs` | Restore modes (DryRun/Merge/Replace), conflict resolution |
| `src/commands/repair_cmd.rs` | Conservative repair, safe/unsafe classification, pre-repair backup |
| `src/library/persistence.rs` | Library load/save, deterministic ID normalization, backup |
| `src/library/manager.rs` | `libraries.toml` registry, primary selection, canonical read-only resolver |
| `src/migration.rs` | Schema versioning, migration trait, TOML roundtripping |
| `docs/IDENTITY_CONTRACT.md` | Snippet and library identity lifecycle rules |
| `docs/PERSISTENCE_INVENTORY.md` | Every persisted artifact with its durability class |
| `tests/persistence_unit.rs` | Atomic write and durability class tests |
| `tests/local_contracts.rs` | Golden-corpus byte-fidelity contract |
| `tests/process_lock_concurrency.rs` | Cross-process kernel-lock concurrency tests |
| `tests/edit_mutation_notify.rs` | `snp edit` byte-change-driven mutation notification tests |
| `tests/identity_contract.rs` | Identity lifecycle contract tests |

---

## Invariants

1. **The save path never post-processes `toml::to_string_pretty` output.** There is exactly one serialization-then-write site and nothing runs between them (`persistence.rs:249-257`).
2. **`write_schema_version` uses `toml::Table`, never `toml::Value`** (`migration.rs:165`) — `toml::Value` collapses array-of-tables.
3. **Every local mutation is gated** by `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)`, and the gate receives *both* directories.
4. **One interrupted journal → auto-rollback; multiple or incomplete journals → refuse** and direct the user to `snp repair` (`transaction.rs:2593-2613`).
5. **Lock acquisition order is `LocalDataLock` → `TransactionLock` → writes** (`restore_cmd.rs:1133`). Never invert.
6. **The gate runs before either lock is taken**, and it releases the transaction lock before returning — so the gate can never invert the order.
7. **Journals live in `<config>/.transaction/`.** Transaction APIs take `.transaction`; pending-marker APIs take the state dir.
8. **Absence of a journal is the true terminal indicator.** Terminal states are never persisted by new code paths.
9. **Commit progress is persisted only after each verified atomic write**, never before.
10. **Rollback restores files in reverse order** with hash verification after each action.
11. **A malformed library or `libraries.toml` fails closed** (best-effort backup + error). A missing or empty file yields defaults. These are different cases and must stay different.
12. **Backup captures a coherent snapshot** — it holds the local-data lock, and every mutation path takes the same lock.
13. **Pre-repair and pre-restore backups are created automatically.** Do not remove them.
14. **Quarantined lock files are retained for 7 days** and pruned opportunistically.

---

## Gotchas

- **A lock that cannot be parsed is not automatically stale.** An empty or partially written record may belong to a live owner. Both locks use a bounded retry ladder before quarantining; quarantining on the first malformed read renames away a live lock and admits a second writer.
- **`PermissionDenied` is transient only on Windows.** On Unix it is stable, so the transaction lock's `is_transient_lock_contention` returns `false` there. Applying the Windows workaround unconditionally caused the CLI to hang with no error and no timeout — including `snp repair`, `snp restore`, and the mutation gate.
- **A failed `write_all` must delete the 0-byte lock file it created.** Otherwise every later acquirer spins on a record that can never become valid.
- **The local-data lock's reclaim is ownership-verified; the transaction lock's is not.** `quarantine_local_data_lock` re-reads and compares the record before renaming (`local_data.rs:322-354`); `quarantine_stale_lock` (`transaction.rs:1268`) renames without re-checking. Do not assume the two are symmetric.
- **`check_interrupted_transactions` is a scan helper, not the gate.** Calling the scan and proceeding anyway reproduces the corruption it exists to prevent.
- **`save_library_internal` skips the gate and the lock.** It is valid only for callers that already hold both (restore). Do not call it from a new external path without that guarantee.
- **Adding a cleanup/normalization pass after `toml::to_string_pretty` silently corrupts user data.** The serializer already picks correct quoting and escaping for tabs, trailing whitespace, and CRLF.
- **`quote_strings_containing_backslashes` must never touch serialized output.** It cannot distinguish a triple-quoted multi-line string from an ordinary double-quoted one, and its single-quoted output preserves `\t` as a literal two-character pair. It survives only for callers that hand-write TOML.
- **Reading is not the same as resolving.** `resolve_selector_readonly` never migrates; `resolve_selector` calls `ensure_library_mode()` and can rewrite metadata. Picking the wrong one reintroduces recovery side effects — see [library.md](library.md).
- **Unit tests must not write to the real `~/.config/snp/`.** `save_library` derives its gate and local-data-lock paths from the *config directory*, not from the target library path (`persistence.rs:264-265`), and `ScopedConfigDir` (`src/utils/config.rs:35-57`, `#[cfg(any(test, feature = "test-support"))]`) binds a per-test, thread-local config directory.

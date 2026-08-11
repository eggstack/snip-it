# Persistence Architecture

[← Back to Overview](overview.md)

## Table of Contents

- [Overview](#overview)
- [Atomic Write Primitive](#atomic-write-primitive)
- [Process File Lock](#process-file-lock)
- [Transaction Boundary](#transaction-boundary)
- [Validation Framework](#validation-framework)
- [Backup Format](#backup-format)
- [Restore Semantics](#restore-semantics)
- [Repair Command](#repair-command)
- [Migration Framework](#migration-framework)
- [Identity Contract](#identity-contract)
- [Key Files](#key-files)

---

## Overview

snip-it uses a layered persistence architecture centered on editable TOML files. Atomic writes are standardized, stable identity is defined, and validation/backup/restore/repair workflows are supported.

The persistence stack has four layers:

1. **Atomic write primitive** — crash-safe file replacement with durability classes
2. **Transaction boundary** — multi-file coordination with interrupted-operation markers
3. **Validation, backup, restore, repair** — data integrity workflows
4. **Migration framework** — schema versioning and evolution

All user-facing data lives under `~/.config/snp/` (XDG-compliant). See [utils.md](utils.md) for path resolution and [library.md](library.md) for the snippet data model.

---

## Atomic Write Primitive

### Location

`src/utils/atomic.rs`

### API

Two public functions:

| Function | Purpose |
|----------|---------|
| `write_private_atomic(path, content, prefix)` | Simple atomic write with `0o600` permissions on Unix |
| `atomic_replace(target, bytes, options)` | Enhanced atomic replace with durability classes, permission control, and target validation |

### Durability Classes

```rust
pub enum Durability {
    DurableUserData,       // fsync before rename, parent dir sync
    SensitiveConfig,       // 0o600 permissions, symlink rejection
    RecoverableMetadata,   // no fsync, default permissions
    EphemeralCoordination, // no fsync, no dir sync
}
```

| Class | fsync file | fsync dir | Permissions | Symlink reject |
|-------|-----------|-----------|-------------|----------------|
| `DurableUserData` | Yes | Yes (best-effort) | Default | No |
| `SensitiveConfig` | No | Yes (best-effort) | `0o600` | Yes |
| `RecoverableMetadata` | No | Yes (best-effort) | Default | No |
| `EphemeralCoordination` | No | No | Default | No |

### Options

```rust
pub struct AtomicWriteOptions {
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub reject_symlink: bool,
}
```

`AtomicWriteOptions::for_durability(d)` creates defaults: `reject_symlink = true` only for `SensitiveConfig`.

### Algorithm

`atomic_replace` executes this sequence:

1. Resolve parent directory (create if missing via `create_dir_all`)
2. Validate target — reject directories, FIFOs, sockets, block/character devices, and optionally symlinks
3. Snapshot original permissions if `preserve_permissions` is set
4. Create UUID-named temp file in the same directory
5. For `SensitiveConfig` on Unix, set `0o600` on the temp file
6. Write bytes, flush to kernel buffer
7. For `DurableUserData`, call `sync_all` on the file
8. Atomic `rename` over the target
9. Restore original permissions if `preserve_permissions` was set
10. Sync parent directory (best-effort, logged on failure)
11. On any failure, `TempFileGuard` cleans up the temp file

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

`LibraryManager::save_library()` calls `write_private_atomic()` for all library TOML writes. The simple atomic write is used because library files are `DurableUserData` with default permissions and the `0o600` temp file prevents brief world-readable exposure.

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

The kernel alone arbitrates. `flock(fd, LOCK_EX | LOCK_NB)` on Unix and `LockFileEx` over a fixed byte range on Windows guarantee that at most one process holds the lock at a time. Unsupported platforms return [`ProcessFileLockError::UnsupportedPlatform`](../../src/process_file_lock.rs) rather than silently weakening exclusion.

### Acquisition Order

1. Ensure the parent directory exists.
2. Open the persistent lock file in read/write/create mode.
3. Attempt the kernel lock nonblocking.
4. If busy, read owner metadata best-effort for diagnostics and return `Busy`.
5. After the kernel lock succeeds, truncate the file.
6. Write the new identity record (PID, nonce, start token, acquired timestamp).
7. `sync_all` and tighten permissions to `0o600` where supported.
8. Return the guard.

If metadata publication fails after kernel acquisition, the kernel lock is released before the error is returned, and no caller is left believing it owns the lock.

### Identity Metadata

[`LockIdentity`](../../src/process_file_lock.rs) is for diagnostics only. It must never authorize lock stealing or canonical-file deletion. A contender that observes a busy kernel lock with empty, malformed, or legacy metadata must treat it as a live owner.

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

---

## Transaction Boundary

### Location

`src/transaction.rs`

### Purpose

Coordinates multi-file operations (restore, library create/delete, bulk import, repair) with a minimal interrupted-operation marker. Individual file replacement is always atomic via `atomic_replace`. Multi-file operations are fail-closed on interruption and may require `snp repair`; they are not transparently database-style transactional across all files.

### Directory Model

The transaction subsystem uses two distinct directories:

- **`sync_state_dir`** (canonical config directory, e.g. `~/.config/snp/`): Where the pending sync marker (`auto-sync-pending.toml`) lives. Pending APIs must receive this directory.
- **`transaction_dir`** (`<sync_state_dir>/.transaction`): Where the interrupted-operation marker, locks, durable backups, and staged files live. Transaction APIs must receive this directory.

This separation ensures the pending marker is never written to the `.transaction` subdirectory, which would cause it to be missed by the canonical pending path. The `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` function requires both directories: it inspects the canonical pending marker (in `sync_state_dir`) while cleaning up transaction artifacts (in `transaction_dir`).

### InterruptedOperation Marker Model

New multi-file operations use a minimal `InterruptedOperation` marker instead of the full transaction state machine. The marker is detection/repair metadata, not a restartable commit program counter. If the process dies while the marker exists:

- Read-only diagnostics may inspect state
- Normal new mutations fail closed (`gate_mutation_on_interrupted_transactions`)
- The user is directed to `snp repair`
- Repair validates affected paths/backups and resolves the marker

Old-style transaction journals from previous versions are still detected for backward compatibility but are no longer created by new operations.

### Components

#### InterruptedOperation

Persisted as `interrupted-operation.toml` in the `.transaction` subdirectory of the state directory:

```rust
pub struct InterruptedOperation {
    /// Schema version for forward compatibility.
    pub schema_version: u32,                    // currently 1
    /// Human-readable operation name (e.g. "restore").
    pub operation: String,
    /// Unix timestamp (ms) when the operation was created.
    pub created_at_unix_ms: i64,
    /// Files affected by this operation.
    pub affected_paths: Vec<PathBuf>,
    /// Backup files created for this operation (parallel to affected_paths).
    /// Empty path means no backup was created for this file.
    pub backup_paths: Vec<PathBuf>,
    /// Original file metadata captured before live writes (parallel to affected_paths).
    /// Used to preserve permissions across rollback.
    pub original_metadata: Vec<OriginalFileMetadata>,
    /// Artifact directory containing staged files for this operation.
    pub artifact_dir: PathBuf,
}
```

The marker file is written via `write_private_atomic` and is always read with symlink rejection.

#### TransactionLock

File-create guard ensuring exclusive access. `acquire_transaction_lock(state_dir)` creates `transaction.lock` via `create_new(true)`. The lock file contains a TOML record with `pid`, `nonce`, `created_at_unix_ms`, `schema_version`, `operation`, and `start_token` fields. On acquisition, if the lock already exists, the system checks PID liveness via `ProcessIdentity::observe(existing.pid)` — dead owners are reclaimed, live owners cause an error. **Ownership verification**: the system observes the process at `existing.pid` and compares the observed start token with the persisted start token, not the contender's own start token. This prevents a live owner from being classified as PID reuse. Ownership is verified on `Drop`: the lock file is only removed if the stored nonce AND start_token match the guard's nonce and start_token, preventing old owners from removing a replacement owner's lock. Malformed locks are quarantined (renamed to `.quarantine.<uuid>`) rather than silently deleted.

### API

| Function | Description |
|----------|-------------|
| `write_interrupted_operation(state_dir, op)` | Write marker via atomic write |
| `read_interrupted_operation(state_dir)` | Read marker, `Ok(None)` if absent |
| `remove_interrupted_operation(state_dir)` | Remove marker from disk |
| `rollback_interrupted_operation(state_dir, op)` | Restore files from backups, clean artifacts, remove marker |
| `recover_interrupted_operation(state_dir)` | Read marker + rollback (repair entry point) |
| `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` | Refuse mutations if marker or old journals exist |
| *Legacy (backward-compatible recovery):* | |
| `acquire_transaction_lock(state_dir)` | Acquire exclusive lock, error if held |
| `begin_transaction(state_dir, operation, affected_files)` | Create journal in `Prepared` state (legacy) |
| `commit_transaction(state_dir, journal)` | Mark `Committed`, remove backups and journal (legacy) |
| `rollback_transaction(journal)` | Restore files from backups in reverse order (legacy) |
| `check_interrupted_transactions(state_dir)` | Find journals in interruptible states (legacy) |

### Crash Recovery

**New model (InterruptedOperation marker):**
1. `gate_mutation_on_interrupted_transactions()` checks for a marker via `read_interrupted_operation(transaction_dir)`
2. If a marker is present, the next mutation fails closed with an error directing the user to `snp repair`
3. `snp repair` calls `recover_interrupted_operation(state_dir)` which reads the marker, rolls back each affected file from its backup, cleans up the artifact directory, and removes the marker

**Legacy model (old journals, backward compatibility):**
1. After checking for new-style markers, `gate_mutation_on_interrupted_transactions()` falls back to scanning for old-style `txn-*.toml` journals
2. Corrupt journals cause immediate failure
3. Classifiable journals are handled according to their recovery class (automatic rollback, deferred, or manual)
4. `snp repair` handles old journals via the existing scanner/classifier

### Marker Lifecycle

1. Caller creates durable backups for every affected file and captures original metadata
2. Caller creates the `InterruptedOperation` struct with paths to affected files, backups, metadata, and artifact directory
3. `write_interrupted_operation(state_dir, &op)` atomically persists the marker
4. Caller performs live replacements via `atomic_replace` with `Durability::DurableUserData`
5. On success: cleanup artifact directory and `remove_interrupted_operation(state_dir)`
6. If interrupted between step 3 and step 5, the marker remains on disk and blocks subsequent mutations until resolved via `snp repair`

### Artifact Path Validation

All transaction artifact paths (backup, staged, destination) are validated by `validate_contained_path` before use. The validation has three layers:

1. **Lexical containment** (`lexically_within`): Both paths must be absolute. `Component::ParentDir` is explicitly rejected during normalization via `normalize_absolute_without_parent` — any `..` component causes immediate rejection. This catches traversal even when the path doesn't exist yet (e.g. `<artifact-root>/../../outside.bin`).
2. **Symlinked prefix rejection** (`reject_symlinked_existing_prefixes`): Walks from root toward child using `symlink_metadata` (not `fs::metadata`, so symlinks are not followed). If any existing intermediate component is a symlink, the path is rejected. This catches `<root>/link/missing.bin` where `link` is a symlink to outside and the final file is absent.
3. **Canonical containment**: For existing paths, canonicalize both root and path and verify containment as defense in depth.

### Backward Compatibility with Old Journals

Old-style `TransactionJournal` structs (`txn-<uuid>.toml`) from versions prior to Phase 14G are still detected and handled:

- `gate_mutation_on_interrupted_transactions()` checks for new markers first, then falls back to old journal scanning
- `snp repair` handles both marker-based and old journal-based recovery
- Legacy transaction state machine code is retained in `transaction.rs` for old journal recovery but is no longer called by new production code
- `advance_to_committed_local` is retained for backward-compatible recovery of old `CommittedLocal` journals
- `BackupsDurable` transaction state is retained for backward-compatible recovery of old journals

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
| `W-EXACT-DUP` | Warning | Exact duplicate snippet (same description + command) |
| `W-ORPHAN-FILE` | Warning | File in `libraries/` not in index |
| `W-NO-PRIMARY` | Warning | No primary library set |
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

`src/commands/backup_cmd.rs`

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
    pub kind: String,                      // "library", "index", "usage", "sync_config"
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
- Transaction journals, interrupted operation markers

### Secret Redaction

`redact_sync_config()` redacts `api_key`, `ApiKey`, and `api-key` lines, replacing values with `<redacted>`.

### Backup Locations

| Flag | Location |
|------|----------|
| Default | `~/.config/snp/backups/<timestamp>/` |
| `--output <path>` | User-specified directory |

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

1. Acquire `LocalDataLock` (backup coordination)
2. Acquire transaction lock (`acquire_transaction_lock`)
3. Begin transaction journal (`begin_transaction`)
4. Load and validate manifest (`manifest.toml` or `manifest.json`)
5. Validate every source artifact (checksum, size, symlink rejection)
6. Validate every destination path (traversal, reserved names, kind constraints)
7. Parse incoming TOML files before any live write
8. Validate duplicate snippet IDs (`validate_library_no_duplicate_ids`)
9. Load every affected current file
10. Compute full restore plan in memory (detect conflicts, produce deterministic report)
11. Create durable backups for every existing destination
12. Create durable staged files containing exact intended bytes
13. fsync files and required parent directories according to durability class
14. Populate all journal fields, including hashes and action
15. Atomically persist `BackupsDurable`
16. Perform live replacements via `atomic_replace` with `Durability::DurableUserData`, persisting `Committing { next_commit_position }` only after each verified write
17. Advance to `CommittedLocal { pending_generation, pending_recorded }` — records pending sync intent atomically
18. Mark journal committed only after all live writes succeed and pending intent is recorded (`commit_transaction`)
19. Release transaction lock
20. Release `LocalDataLock`
21. Schedule auto-sync once, after commit, if policy permits
22. Clean backups and journal according to retention policy

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
    pub category: String,   // "index", "primary", "usage", "ids", "transaction", "timestamps"
    pub problem: String,
    pub fix: String,
    pub safe: bool,         // Whether safe for auto-apply
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

### Modes

- `--dry-run`: Analyze and print planned repairs
- `--apply`: Create pre-repair backup, apply safe repairs, emit report
- Neither: Print validation summary only

### Backup Before Repair

`snp repair --apply` always creates a timestamped backup at `~/.config/snp/backups/repair-<timestamp>/` before any mutations.

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

Version 0 represents unversioned/legacy files. Version 1 is the current schema. The version is stored as `schema_version = <N>` in the TOML top-level table.

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

`write_schema_version` parses the file as `toml::Table`, inserts the version key, and serializes back. This preserves array-of-tables structure and other TOML constructs that naive string insertion would corrupt.

### Migration Chain

`run_migrations` iterates through registered migrations in order. Each migration's `source` must match the current version. The chain advances from `LEGACY` → `CURRENT`.

---

## Identity Contract

### Location

`docs/IDENTITY_CONTRACT.md`

### Snippet Identity

- Opaque string identifier, assigned deterministically on load for legacy snippets
- UUID v4 for new snippets created via `Snippet::new()` or import
- Never regenerated for a given snippet
- Retained across edit, move, export, sync, and restore
- New ID assigned on import (existing IDs discarded)
- Deduplication on load: first duplicate keeps original, later duplicates get deterministic replacement IDs

### ID Assignment Points

1. **`load_library()`** — assigns deterministic `legacy-<sha256hex>` IDs to empty IDs and deduplicates on load
2. **`commands::import_cmd`** — assigns UUID for imported snippets
3. **`doctor_cmd`** — reports planned ID regeneration (diagnostic only)

Note: `Snippet::new()` creates a snippet with an empty `id`. The deterministic ID is assigned when the library is next loaded. The next normal `save_library()` persists the normalized IDs.

### Deterministic Legacy IDs

For legacy snippets with missing IDs, `load_library()` generates deterministic provisional IDs using SHA-256:

- **Missing ID**: `legacy-<sha256("snip-it-legacy-id-v1\0" + description + "\0" + command + "\0" + tags + "\0" + output + "\0" + occurrence)>`
- **Duplicate ID**: `legacy-<sha256("snip-it-duplicate-id-v1\0" + original_id + "\0" + description + "\0" + command + "\0" + tags + "\0" + output + "\0" + occurrence)>`

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
- Lock files use the kernel-backed process file lock for atomic acquisition; ownership is never reused based on PID liveness inspection
- Auto-sync lock ownership uses start-token + nonce diagnostics only; the kernel state is the sole mutual-exclusion authority
- Atomic writes: temp-file-then-rename with validate_target (rejects FIFOs, sockets, devices)
- Transaction artifact paths validated with lexical containment, symlinked prefix rejection, and canonical containment
- Interrupted operation marker written atomically via `write_private_atomic`, read with symlink rejection
- Transaction locks use UUID-based filenames and O_EXCL locks
- Backup checksums: SHA-256 per file, verified before restore
- Backup redaction: API keys stripped from sync.toml copies

## Key Files

| File | Subject |
|------|---------|
| `src/utils/atomic.rs` | Atomic write primitive, durability classes, temp file guard |
| `src/process_file_lock.rs` | Kernel-backed cross-process file lock primitive |
| `src/auto_sync/execution_lock.rs` | Auto-sync execution lock and worker lock wrappers |
| `src/auto_sync/pending_lock.rs` | Auto-sync pending-marker mutex wrapper |
| `snip-sync/src/server_lock.rs` | snip-sync server singleton kernel lock |
| `snip-sync/src/process.rs` | PID record parser, atomic publication, identity-checked cleanup |
| `src/transaction.rs` | Transaction boundary, `InterruptedOperation` marker, lock, rollback (legacy journal recovery retained for old data) |
| `src/commands/validate_cmd.rs` | Validation framework, diagnostic model, 12+ check categories |
| `src/commands/backup_cmd.rs` | Backup manifest, secret redaction, SHA-256 integrity |
| `src/commands/restore_cmd.rs` | Restore modes (DryRun/Merge/Replace), conflict resolution |
| `src/commands/repair_cmd.rs` | Conservative repair, safe/unsafe classification |
| `src/migration.rs` | Schema versioning, migration trait, TOML roundtripping |
| `docs/IDENTITY_CONTRACT.md` | Snippet and library identity lifecycle rules |
| `tests/persistence_unit.rs` | Atomic write and durability class tests |
| `tests/process_lock_concurrency.rs` | Cross-process kernel-lock concurrency tests |
| `tests/edit_mutation_notify.rs` | `snp edit` byte-change-driven mutation notification tests |
| `tests/identity_contract.rs` | Identity lifecycle contract tests |

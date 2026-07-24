# Persistence Architecture

[← Back to Overview](overview.md)

## Table of Contents

- [Overview](#overview)
- [Atomic Write Primitive](#atomic-write-primitive)
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

snip-it uses a layered persistence architecture centered on editable TOML files. Phase 07A standardizes atomic writes, defines stable identity, and adds validation/backup/restore/repair workflows.

The persistence stack has four layers:

1. **Atomic write primitive** — crash-safe file replacement with durability classes
2. **Transaction boundary** — multi-file coordination with journaling
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

## Transaction Boundary

### Location

`src/transaction.rs`

### Purpose

Coordinates multi-file operations (library create/delete, bulk import, restore, repair) with crash-safe journaling. The transaction lock prevents concurrent mutations.

### State Machine

```
Prepared → BackupsDurable → Committing{next_commit_position} → CommittedLocal{pending_generation, pending_recorded} → Committed
Prepared → Failed(error_message)
```

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
    pub destination: PathBuf,
    pub action: StagedAction,
    pub existed_before: bool,
    pub original_hash: Option<String>,
    pub intended_hash: Option<String>,
    pub durable_backup_path: Option<PathBuf>,
    pub durable_staged_path: Option<PathBuf>,
    pub original_permissions: Option<PortablePermissions>,
}
```

`durable_staged_path` is a private durable copy of the intended bytes, never the live destination. `durable_backup_path` is a durable copy of the original bytes for rollback.

#### TransactionLock

File-create guard ensuring exclusive access. `acquire_transaction_lock(state_dir)` creates `transaction.lock` via `create_new(true)`. The lock file contains a TOML record with `pid`, `nonce`, `created_at_unix_ms`, `schema_version`, `operation`, and `start_token` fields. On acquisition, if the lock already exists, the system checks PID liveness via `ProcessIdentity::observe(existing.pid)` — dead owners are reclaimed, live owners cause an error. **Phase 11C fix**: ownership verification observes the process at `existing.pid` and compares the observed start token with the persisted start token, not the contender's own start token. This prevents a live owner from being classified as PID reuse. Ownership is verified on `Drop`: the lock file is only removed if the stored nonce AND start_token match the guard's nonce and start_token, preventing old owners from removing a replacement owner's lock. Malformed locks are quarantined (renamed to `.quarantine.<uuid>`) rather than silently deleted.

### API

| Function | Description |
|----------|-------------|
| `acquire_transaction_lock(state_dir)` | Acquire exclusive lock, error if held |
| `begin_transaction(state_dir, operation, affected_files)` | Create journal in `Prepared` state |
| `commit_transaction(state_dir, journal)` | Mark `Committed`, remove backups and journal |
| `rollback_transaction(journal)` | Restore files from backups in reverse order |
| `check_interrupted_transactions(state_dir)` | Find journals in `Prepared` state on startup |

### Crash Recovery

`check_interrupted_transactions()` scans the `.transaction` subdirectory of the state directory for `txn-*.toml` files in interruptible states (`Prepared`, `BackupsDurable`, `Committing`, `RollingBack`). These represent interrupted operations. The `snp repair` command detects these and offers automatic rollback.

### Journal Lifecycle

1. `begin_transaction` writes journal via `write_private_atomic` in `Prepared` state
2. Caller creates durable backups and staged files, then advances to `BackupsDurable`
3. Caller performs live replacements, advancing through `Committing { next_commit_position }` per file — progress persisted only after each verified atomic write
4. After all writes complete, advance to `CommittedLocal { pending_generation, pending_recorded }` — records pending sync intent atomically
5. `commit_transaction` marks `Committed`, cleans up backups and journal
6. If interrupted between begin and commit, `check_interrupted_transactions` finds the orphan in any interruptible state (including `CommittedLocal`, which completes the pending intent recording)

### Commit Progress Semantics

`Committing { next_commit_position }` uses completed-position semantics: `next_commit_position == N` means positions `0..N` have already been installed and verified; position `N` is next. Progress is persisted only after install and verification, never before.

### Rollback Order

`RollingBack { next_rollback_position }` uses rollback-order coordinates: `rollback_order = (0..files.len()).rev()`. `next_rollback_position == N` means positions `0..N` have been rolled back. Each rollback action verifies the pre-transaction bytes or expected absence after completion.

### CommittedLocal State

`CommittedLocal { pending_generation, pending_recorded }` eliminates the crash window between durable restore commit and pending-sync intent recording. After all live writes are committed, the journal transitions to `CommittedLocal` with the pending generation number. The pending sync intent is then recorded. If a crash occurs between `CommittedLocal` and `Committed`, recovery completes the pending intent recording.

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
- Transaction journals

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

- UUID v4 string, generated by `uuid::Uuid::new_v4()`
- Never regenerated for a given snippet
- Retained across edit, move, export, sync, and restore
- New ID assigned on import (existing IDs discarded)
- Deduplication on load: duplicate IDs get new UUIDs

### ID Assignment Points

1. **`load_library()`** — assigns UUID to empty IDs and deduplicates on load
2. **`commands::import_cmd`** — assigns UUID for imported snippets
3. **`doctor_cmd`** — reports planned ID regeneration (diagnostic only)

Note: `Snippet::new()` creates a snippet with an empty `id`. The UUID is assigned when the library is next loaded.

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

## Security Properties (Phase 09A)

- All sensitive files created with 0o600 permissions
- Config directory created with 0o700 permissions
- Lock files use O_EXCL (create_new) for atomic acquisition
- Auto-sync lock ownership verified via nonce (pid-nanos-seq) to prevent PID reuse theft; transaction lock also uses nonce + PID liveness check for ownership verification and stale-lock reclaim
- Atomic writes: temp-file-then-rename with validate_target (rejects FIFOs, sockets, devices)
- Transaction journals use UUID-based filenames and O_EXCL locks
- Backup checksums: SHA-256 per file, verified before restore
- Backup redaction: API keys stripped from sync.toml copies

## Key Files

| File | Subject |
|------|---------|
| `src/utils/atomic.rs` | Atomic write primitive, durability classes, temp file guard |
| `src/transaction.rs` | Transaction boundary, journaling, lock, rollback |
| `src/commands/validate_cmd.rs` | Validation framework, diagnostic model, 12+ check categories |
| `src/commands/backup_cmd.rs` | Backup manifest, secret redaction, SHA-256 integrity |
| `src/commands/restore_cmd.rs` | Restore modes (DryRun/Merge/Replace), conflict resolution |
| `src/commands/repair_cmd.rs` | Conservative repair, safe/unsafe classification |
| `src/migration.rs` | Schema versioning, migration trait, TOML roundtripping |
| `docs/IDENTITY_CONTRACT.md` | Snippet and library identity lifecycle rules |
| `tests/persistence_unit.rs` | Atomic write and durability class tests |
| `tests/identity_contract.rs` | Identity lifecycle contract tests |

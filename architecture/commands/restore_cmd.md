# snp restore

[← Back to Overview](../overview.md) · [Command index](mod.md) · [backup_cmd.md](backup_cmd.md) · [backup_archive.md](backup_archive.md) · [repair_cmd.md](repair_cmd.md)

`snp restore` installs a backup snapshot produced by `snp backup` back into the configuration directory. It is the only command that writes library files from an external source, and it is the **most safety-critical command in the maintenance set**: it runs a full transaction with rollback, takes both locks, and drives a pending-marker finalization so auto-sync picks the restored content up.

It does **not** cover: restoring the OS keychain, restoring the `snip-sync` server, restoring files that were not in the backup manifest, or recovering from a corrupted backup (it refuses rather than guessing).

## CLI arguments

`snp restore` and `snp data restore` share one struct (`src/commands/restore_cmd.rs:22-36`):

```rust
pub struct RestoreArgs {
    /// Path to the backup directory
    #[arg(value_name = "BACKUP_DIR")]
    pub backup: PathBuf,
    /// Restore mode
    #[arg(long, value_enum, default_value = "merge")]
    pub mode: RestoreMode,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}
```

| Flag | Type | Default | Effect |
|---|---|---|---|
| `BACKUP_DIR` (positional) | `PathBuf` | — | The backup directory produced by `snp backup`. Must contain `manifest.toml`. |
| `--mode <MODE>` | `RestoreMode` | `merge` | `merge`, `replace`, or `dry-run`. See [Modes](#modes). |
| `--json` | flag | off | Emit the restore report as JSON on **stdout**. |

`RestoreMode` (`src/commands/restore_cmd.rs:16-20`):

```rust
pub enum RestoreMode {
    DryRun,
    Merge,
    Replace,
}
```

## Modes

### `dry-run`

Reads and validates the backup, then prints the planned actions and **returns before acquiring any lock, creating a journal, or writing anything** (`src/commands/restore_cmd.rs:1091-1130`). The dry-run JSON report contains `mode`, `manifest_schema`, `manifest_version`, `files_in_backup`, and a per-file `path`/`kind`/`size`/`action` list (`:1093-1112`).

### `merge` (default)

- **Libraries:** merges snippet-by-snippet by ID. If the incoming snippet's `updated_at` is **strictly greater**, the incoming snippet wins and a conflict of kind `updated` is recorded. Otherwise the existing snippet is kept and a `kept_existing` conflict is recorded. A snippet present only in the backup is appended (`added`) (`src/commands/restore_cmd.rs:810-855`).
- **Index (`libraries.toml`), usage (`usage.toml`), sync config (`sync.toml`):** if the destination already exists, the local file is **preserved** and the entry is marked `NoOp` (`restore_cmd.rs:1250-1280`). `sync.toml` additionally records `sync.toml (local config preserved)` in `report.skipped` (`:1270-1272`).
- **Identical library content:** the library is skipped entirely and recorded as `<name>.toml (identical)` in `report.skipped` (`restore_cmd.rs:803-808`).

### `replace`

- **Libraries:** the backup file replaces the destination wholesale; a `replaced` conflict is recorded when the destination existed (`restore_cmd.rs:861-877`).
- **Index / usage / sync config:** replaced unconditionally (the `merge`-mode skip does not apply).
- **Pre-restore backup:** a copy of the current `<config>/libraries/*.toml` and `libraries.toml` is taken to `<config>/backups/pre-restore-<YYYYMMDD_HHMMSS>/` before any live write (`restore_cmd.rs:728-774`, called at `:1308-1313`).
- **Strict index coverage:** in replace mode every library artifact in the manifest must be referenced by the index, otherwise restore fails before any lock is taken (`restore_cmd.rs:481-494`).

### Merge vs replace at a glance

| Aspect | `merge` | `replace` |
|---|---|---|
| Library conflict resolution | per-snippet by ID, newer `updated_at` wins | whole file replaced |
| Existing `libraries.toml` / `usage.toml` | preserved (`NoOp`) | replaced |
| Existing `sync.toml` | preserved (`NoOp`) | replaced |
| Pre-restore backup of live config | no | yes |
| Index must cover all artifacts | no | yes |
| Writes library files | yes (merged content) | yes |

## Restore flow

`run(backup, mode, json)` (`src/commands/restore_cmd.rs:956-1762`).

### Phase A — validate before touching anything

All of this happens **before** any lock, transaction, or live write.

1. **Backup path must exist** (`:957-962`).
2. **Load the manifest** (`:964-965` → `load_manifest`, `:542-570`). `manifest.toml` is tried first, then `manifest.json`.
3. **Contract validation** (`:971` → `validate_manifest_contract`, `:239-353`):
   - `schema` must be `1`; `layout` must be `"directory"` (`:241-254`);
   - at most one `index`, one `usage`, one `sync_config` entry (`:256-287`);
   - every path re-validated for traversal / Windows reserved names (`:289-292`);
   - no two entries may map to the same destination key, using portable separator normalization (`:294-312`);
   - `sha256` must be exactly 64 hex characters and `size` must not exceed the 10 MiB ceiling (`:320-350`).
4. **Path validation** for every entry (`:973-976`).
5. **Artifact checks** for every entry (`:978-1057`): file exists; not a symlink (`symlink_metadata`); is a regular file; size ≤ `MAX_RESTORE_SOURCE_SIZE` (10 MiB, `:189`); manifest-declared size equals the actual size; and for `library` entries, **no duplicate snippet IDs** (`validate_library_no_duplicate_ids`, `:521-539`).
6. **Checksum verification** (`:1059-1074`): recompute SHA-256 and compare with the manifest. A mismatch is a hard error.
7. **Semantic validation** (`:1080` → `validate_manifest_semantics`, `:381-514`): parse the index, reject duplicate or case-folded-colliding library names, reject more than one primary, reject index entries without a matching artifact, reject artifacts not indexed (replace mode only), and reject library path aliases that map to the same destination.

### Phase B — dry-run exit

If `mode == DryRun`, print the plan and return (`:1091-1130`).

### Phase C — gate, lock, journal

```rust
// 6. Gate against foreign interrupted transactions, then acquire locks.
//    Lock hierarchy: LocalDataLock -> TransactionLock -> destination writes.
let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
let transaction_dir = sync_state_dir.join(".transaction");
crate::transaction::gate_mutation_on_interrupted_transactions(
    &sync_state_dir,
    &transaction_dir,
)?;
let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;
let _lock = crate::transaction::acquire_transaction_lock(&transaction_dir, "restore")?;
```

(`src/commands/restore_cmd.rs:1132-1147`.) This is the canonical statement of the lock hierarchy in the codebase:

**`LocalDataLock` → `TransactionLock` → destination writes.**

Two distinct directories are used and they are not interchangeable:

| Directory | Contents | Which APIs take it |
|---|---|---|
| `sync_state_dir` (config root) | the auto-sync pending marker `auto-sync-pending.toml` | pending-marker APIs (`ensure_pending_for_transaction`) |
| `transaction_dir` (`<config>/.transaction`) | journals, locks, durable staged files, per-transaction backups | transaction APIs (`begin_transaction`, `acquire_transaction_lock`) |

The mutation gate runs **before** the locks. One interrupted journal is auto-rolled-back; multiple or incomplete journals cause the gate to **refuse** and point the operator at `snp repair` (see `architecture/persistence.md`).

### Phase D — prepare

1. **Collect affected files** and map each manifest entry to its destination (`:1149-1165`):
   - `library` → `<config>/libraries/<stem>.toml`
   - `index` → `<config>/libraries.toml`
   - `usage` → `<config>/usage.toml`
   - `sync_config` → `<config>/sync.toml`
2. **Begin the transaction** (`:1167-1168`) → journal in `Prepared` state.
3. **Failpoint** `RESTORE_AFTER_PREPARED` (`:1169-1171`).
4. **Create the artifact tree** `<config>/.transaction/artifacts/<txn-id>/{backups,staged}` (`:1179-1197`).
5. **For each affected file**: copy the current content to `backups/<i>.bak` via `copy_sync_verify` (`:1202-1206`), then compute the intended replacement bytes and write them to `staged/<i>.new` via `write_sync_verify` (`:1285-1288`). Library merge computation happens **here, during preparation** — not in the live commit loop (`:1224-1248`).
6. **Persist `BackupsDurable`** (`:1295`) — the journal now contains every rollback path and every commit path. **Failpoint** `RESTORE_AFTER_BACKUPS_DURABLE` (`:1296-1298`).
7. **Persist `Committing` with 0 completed positions** (`:1304`) — progress is persisted **after** each verified write, so a crash never causes recovery to skip an unwritten destination.

### Phase E — commit

Inside a closure whose error triggers rollback (`:1307`):

1. **Replace mode:** take the pre-restore backup (`:1308-1313`).
2. **Ensure `<config>/libraries/` exists** (`:1316-1319`).
3. **Install each file** from its durable staged path (`:1328-1556`):
   - `library` → `install_library_file` (`:888-953`): atomic replace, apply the destination permission policy, re-hash the **live** destination and compare to the intended hash, verify metadata, verify permissions, then `report.files_restored += 1`.
   - `index` / `usage` → atomic replace with `SensitiveConfig` for new files or `DurableUserData` + `preserve_permissions` for existing ones; metadata is restored and verified only for pre-existing files (`:1385-1507`).
   - `sync_config` → always `SensitiveConfig` (0o600); a `redacted_key` conflict is always recorded telling the operator to re-run `snp register` (`:1509-1553`).
4. **Persist commit progress after each verified write** (`:1558-…`).

### Phase F — commit-to-pending finalization

Only when `report.files_restored > 0` (`:1631`):

1. `advance_to_committed_local(..., PendingFinalization::NotRecorded)` — marks the transaction committed locally; a crash here is recoverable and creates **no** pending generation (`:1636-1640`). **Failpoint** `RESTORE_AFTER_COMMITTED_LOCAL_BEFORE_PENDING` (`:1644-1646`).
2. `ensure_pending_for_transaction(&sync_state_dir, &journal.id, PendingSnapshot::Mutation { kind: MutationKind::Import })` — idempotently records the pending marker in the **canonical state dir** (`:1653-1665`). **Failpoint** `RESTORE_AFTER_PENDING_BEFORE_JOURNAL_UPDATE` (`:1693-1695`).
3. `advance_to_committed_local(..., finalization)` where `finalization` is `Recorded { generation }` or `CoveredByExisting { generation }` (`:1667-1703`). A conflicting newer pending generation is **preserved** without incrementing (`:1674-1688`).
4. **Failpoint** `RESTORE_AFTER_JOURNAL_PENDING_BEFORE_CLEANUP` (`:1708-1710`).
5. `commit_transaction` — remove the journal and backups (`:1713`).
6. **Schedule the auto-sync worker** for the already-recorded pending work; a scheduling failure is a **warning**, not an error, and the pending marker is preserved for a later retry (`:1718-1729`).

When nothing was restored, the transaction is simply committed (`:1730-1733`).

## The conflict report

```rust
pub struct RestoreConflict {
    pub library: String,
    pub kind: String,
    pub detail: String,
}

pub struct RestoreReport {
    pub mode: String,
    pub files_restored: usize,
    pub conflicts: Vec<RestoreConflict>,
    pub skipped: Vec<String>,
    pub pre_restore_backup: Option<String>,
}
```

(`src/commands/restore_cmd.rs:164-180`.)

| `kind` | Raised when | Source |
|---|---|---|
| `updated` | incoming snippet has a strictly greater `updated_at` | `restore_cmd.rs:823-834` |
| `kept_existing` | existing snippet's `updated_at` is greater or equal | `restore_cmd.rs:835-846` |
| `added` | snippet ID not present in the destination | `restore_cmd.rs:847-854` |
| `replaced` | replace mode overwrote an existing library | `restore_cmd.rs:863-869` |
| `redacted_key` | `sync.toml` was restored from a redacted backup | `restore_cmd.rs:1545-1551` |

`report.skipped` holds human-readable strings, not structured records: `"<name>.toml (identical)"`, `"sync.toml (local config preserved)"`.

## Destination permission handling

```rust
pub enum DestinationClass {
    NewPrivate,
    ExistingPreserved,
    Restore,
}
```

(`src/commands/restore_cmd.rs:38-54`.) `for_destination(existed_before, is_restore)` (`:59-67`) picks `NewPrivate` for a file that did not exist, otherwise `Restore` (restore always passes `is_restore = true`, `restore_cmd.rs:901`, `:1397`, `:1460`).

| Class | Unix mode | Non-Unix |
|---|---|---|
| `NewPrivate` | `0o600` | best-effort |
| `ExistingPreserved` / `Restore` | original mode from `OriginalFileMetadata::unix_mode`, defaulting to `0o600` | original readonly flag, best-effort |

`apply_permissions` (`:77-122`) sets the mode; `verify_permissions` (`:128-161`) re-stats the file and compares `mode & 0o777` against the expected value, erroring on mismatch ("Destination permission verification failed"). New files created with `Durability::SensitiveConfig` get `0o600` **at creation**; their original metadata is deliberately **not** re-applied, because that would overwrite `0o600` with the default `0o644` (`restore_cmd.rs:1409-1417`).

## Crash / restore failpoint suite

The restore path is instrumented with failpoints from `src/test_failpoints.rs:100-123`:

| Failpoint constant | Value | Position | Crash point it models |
|---|---|---|---|
| `RESTORE_AFTER_PREPARED` | `restore-after-prepared` | `restore_cmd.rs:1169` | journal is `Prepared`; no backups or staged files exist yet |
| `RESTORE_AFTER_BACKUPS_DURABLE` | `restore-after-backups-durable` | `restore_cmd.rs:1296` | backups and staged files are durable; journal is `BackupsDurable`; no live write yet |
| `RESTORE_AFTER_FIRST_INSTALL` | `restore-after-first-install` | `restore_cmd.rs:1569` | exactly one destination has been written and verified |
| `RESTORE_AFTER_INDEX_INSTALL` | `restore-after-index-install` | `restore_cmd.rs:1574` | the `libraries.toml` index has been installed |
| `RESTORE_AFTER_ALL_INSTALLS` | `restore-after-all-installs` | `restore_cmd.rs:1599` | every destination is installed; commit is not yet finalized |
| `RESTORE_AFTER_COMMITTED_LOCAL_BEFORE_PENDING` | `restore-after-committed-local-before-pending` | `restore_cmd.rs:1644` | `CommittedLocal(NotRecorded)` is durable; **no pending marker must exist** |
| `RESTORE_AFTER_PENDING_BEFORE_JOURNAL_UPDATE` | `restore-after-pending-before-journal-update` | `restore_cmd.rs:1693` | the pending marker is durable; the journal still says `NotRecorded` |
| `RESTORE_AFTER_JOURNAL_PENDING_BEFORE_CLEANUP` | `restore-after-journal-pending-before-cleanup` | `restore_cmd.rs:1708` | the journal records the final pending state; artifacts still exist |

Each of these is a distinct recovery state that `snp repair` must classify and resolve (`repair_cmd.rs:483-553`).

## Output and exit codes

| Mode | Stream | Content |
|---|---|---|
| `--json` | stdout | the serialized `RestoreReport` (`:1736-1740`) |
| default | stderr | `Restore complete (mode: …)`, files restored, conflict list, skipped list, pre-restore backup path (`:1741-1759`) |

`snp restore` returns `CliOutcome::Success` (exit `0`) on success (`src/main.rs:449-452`). Failures are `SnipError`s mapped through `src/outcome.rs:48-72`.

## Interaction with the transaction journal and locks

| Concern | Behavior |
|---|---|
| Mutation gate | `gate_mutation_on_interrupted_transactions(&sync_state_dir, &transaction_dir)` — called **before** locking (`restore_cmd.rs:1142-1145`). |
| Local-data lock | `acquire_local_data_lock(&transaction_dir)` — held for the whole restore (`restore_cmd.rs:1146`). |
| Transaction lock | `acquire_transaction_lock(&transaction_dir, "restore")` — held for the whole restore (`restore_cmd.rs:1147`). |
| Journal | `begin_transaction` → `Prepared` → `BackupsDurable` → `Committing` → `CommittedLocal` → committed (`:1168`, `:1295`, `:1304`, `:1636`, `:1699`, `:1713`). |
| Artifacts | `<config>/.transaction/artifacts/<txn-id>/{backups,staged}` (`:1179-1197`). |
| Pending marker | Written to `sync_state_dir`, **not** `transaction_dir` (`:1653-1655`). |

## Key Files

| Path | Role |
|---|---|
| `src/commands/restore_cmd.rs` | This command: modes, validation, transaction, install, report. |
| `src/commands/backup_archive.rs` | Manifest format, path validation, hashing, redaction. |
| `src/commands/backup_cmd.rs` | The producer of the backup this command consumes. |
| `src/transaction.rs` | Gate, transaction lock, journal state machine, `copy_sync_verify` / `write_sync_verify`. |
| `src/local_data.rs` | Local-data lock. |
| `src/test_failpoints.rs` | The restore failpoint constants. |
| `architecture/persistence.md` | Journal states, lock hierarchy, durability classes. |
| `architecture/commands/repair_cmd.md` | The recovery command for interrupted restores. |

## Invariants

- **Nothing is written until every validation passes.** Manifest contract, path safety, artifact type/size, checksum, and index semantics are all checked before the first lock is taken (`restore_cmd.rs:964-1080`).
- The lock order is always `LocalDataLock → TransactionLock → destination writes` (`restore_cmd.rs:1142-1147`).
- The gate runs before the locks, so an interrupted foreign transaction refuses the restore rather than racing it.
- Every staged artifact is written, synced, and verified **before** the journal is persisted at `BackupsDurable`; commit progress is persisted **after** each verified write.
- Each installed destination is re-hashed **from the live file** and compared to the intended hash; an I/O error during hashing is propagated, never treated as a hash match.
- New files are created `0o600`; pre-existing files keep their original mode, and the mode is verified after the write.
- The pending marker is written **once** and idempotently (`ensure_pending_for_transaction`), so a crash-and-retry never produces two pending generations.
- A restored `sync.toml` always records a `redacted_key` conflict.

## Gotchas

- **The default mode is `merge`, not `replace`.** `merge` preserves an existing `libraries.toml`, `usage.toml`, and `sync.toml`; it merges library contents per snippet.
- Merge resolution compares **`updated_at` only**, and strictly — equal timestamps keep the existing snippet (`restore_cmd.rs:823-846`). It is not the sync-side conflict tuple `(updated_at, device_id, SHA-256)`.
- A restored `sync.toml` contains the **redacted** API key. The command says so in a `redacted_key` conflict, but the file is still written. Re-run `snp register` afterwards.
- The 10 MiB per-file ceiling applies to both the manifest-declared size and the real artifact size. A large library cannot be restored even if the manifest lies about it.
- The **pre-restore backup** is only taken in `replace` mode, and it copies only `libraries/*.toml` and `libraries.toml` — not `usage.toml` or `sync.toml` (`restore_cmd.rs:728-774`).
- `dry-run` reads and hashes every artifact (steps 1–7 run before the dry-run branch), so it is not free on a large backup.
- The transaction lock's stale-lock quarantine is **not ownership-verified** (`src/transaction.rs:1268`), unlike the local-data lock's, which re-reads the lock record and refuses to quarantine when it has changed or become unparseable (`src/local_data.rs:326-354`, rename at `:356-370`). A stale transaction lock can therefore in principle be renamed out from under a live owner.

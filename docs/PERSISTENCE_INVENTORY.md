# Persistence Inventory

> **Evergreen contract.** This document catalogs every persisted artifact in snip-it,
> with its durability class. Durability classes are implemented in
> `src/utils/atomic.rs`.

For each artifact: canonical path derivation, owner module/layer, schema/version, user-editable vs private, secret classification, durability class, max supported size, atomicity method, permissions/ACL expectation, symlink/non-regular-file policy, corruption handling, unknown-field policy, backup inclusion default, migration owner, and synchronization relevance.

**Reading the "Durability class" column.** The four classes
(`DurableUserData`, `SensitiveConfig`, `RecoverableMetadata`,
`EphemeralCoordination`) live in `src/utils/atomic.rs` and are applied only
through `atomic_replace` / `atomic_write_bytes`. `write_private_atomic` takes
**no** class — it always fsyncs the file and fsyncs the parent directory on
Unix only. Artifacts written through it are therefore listed as "None
applied", not as a class. See
[Durability Classes](#durability-classes) below and
[`.skills/persistence-and-toml-fidelity.md`](../.skills/persistence-and-toml-fidelity.md)
for the save path.

## Artifacts

### 1. Snippet Libraries

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/libraries/<name>.toml` (library mode) or `~/.config/snp/snippets.toml` (legacy single-file) |
| **Owner module** | `src/library/` (data structures, load/save), `src/commands/mod.rs` (load_snippets/save_snippets for legacy) |
| **Schema** | TOML with `[[snippets]]` table (pet-compatible), fields: id, description, command, output, tag, folders, favorite, created_at, updated_at, device_id, deleted |
| **User-editable** | Yes (primary user asset) |
| **Secret classification** | None |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | Unbounded (practical: ~10MB per library) |
| **Atomicity method** | `utils::atomic::write_private_atomic` (temp file + rename) |
| **Permissions** | `0o600` on Unix (private) |
| **Symlink policy** | Created atomically, no symlink support |
| **Corruption handling** | Backup before save into `<library dir>/backups/<stem>.<timestamp>.toml.bak` (max 10 per library); a failed parse copies the file to `<name>.toml.corrupt.bak` beside it |
| **Unknown-field policy** | Serde `#[serde(deny_unknown_fields)]` NOT used; unknown fields silently ignored |
| **Backup inclusion** | Default (primary content) |
| **Migration owner** | `LibraryManager::migrate_from_single_file` |
| **Sync relevance** | Primary sync payload |

### 2. Library Index (`libraries.toml`)

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/libraries.toml` |
| **Owner module** | `src/library/` (`LibraryManager::save_config`) |
| **Schema** | TOML with `[[libraries]]` array (filename, library_id, is_primary, last_sync, server_id) plus a top-level `generation` counter |
| **User-editable** | No (managed by tool) |
| **Secret classification** | None |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | ~1KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Fail-closed: best-effort backup + error, never synthesize a writable empty index |
| **Unknown-field policy** | Serde default (ignored) |
| **Backup inclusion** | Yes (required for restore) |
| **Migration owner** | `LibraryManager` |
| **Sync relevance** | Not synced |

### 3. Sync Configuration

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/sync.toml` |
| **Owner module** | `src/config/` (`SyncSettings`) |
| **Schema** | TOML with server_url, api_key (keychain), device_id, direction, auto_sync, etc. |
| **User-editable** | Partially (server_url, direction) |
| **Secret classification** | API key is secret — normally held in the OS keychain with an `@keychain` marker in the file; plaintext storage is refused by default |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | ~1KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Falls back to defaults |
| **Unknown-field policy** | Serde default (ignored) |
| **Backup inclusion** | Only with `--include-sync-state`; the copied file has its `api_key` line replaced with `"<redacted>"` |
| **Migration owner** | `config.rs` |
| **Sync relevance** | Configuration only |

### 4. Usage Metadata

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/usage.toml` |
| **Owner module** | `src/usage.rs` |
| **Schema** | TOML with per-snippet usage data (use_count, last_used) |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | ~100KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` (via save path) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Load is fail-open: a missing, unreadable, or invalid file yields an empty index |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No (optional, via --include-usage) |
| **Migration owner** | `usage.rs` |
| **Sync relevance** | Not synced |

### 5. Theme Selection

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/themes.toml` |
| **Owner module** | `src/ui/theme.rs` |
| **Schema** | TOML with active theme name |
| **User-editable** | Yes (via env var SNP_THEME or config) |
| **Secret classification** | None |
| **Durability class** | SensitiveConfig |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write (`atomic_write_bytes`, `Durability::SensitiveConfig`) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Falls back to default theme (Cyber Red) |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No |
| **Migration owner** | `ui/theme.rs` |
| **Sync relevance** | Not synced |

### 6. Theme Files

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/themes/<name>.toml` |
| **Owner module** | `src/ui/theme.rs` |
| **Schema** | Halloy-compatible TOML |
| **User-editable** | Yes |
| **Secret classification** | None |
| **Durability class** | None applied (plain file write, not the atomic layer) |
| **Max size** | ~100KB per theme |
| **Atomicity method** | Plain `fs::write` when seeding a bundled theme (write-if-missing; not atomic) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Skipped on load failure |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No |
| **Migration owner** | `ui/theme.rs` |
| **Sync relevance** | Not synced |

### 7. Pending Sync Markers

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/auto-sync-pending.toml` |
| **Owner module** | `src/auto_sync/pending.rs` |
| **Schema** | TOML marker with monotonic generation + creation timestamp |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | DurableUserData |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write (`atomic_write_bytes`, `Durability::DurableUserData`) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Treated as stale if invalid |
| **Unknown-field policy** | N/A (private format) |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/pending.rs` |
| **Sync relevance** | Drives sync scheduling |

### 8. Library Sync-Recovery Marker

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/libraries/<library>.sync_recovery` |
| **Owner module** | `src/sync_commands.rs` (`write_recovery_marker`) |
| **Schema** | TOML marker: schema, local_library_name, local_library_id, server_library_id, created_at_unix_ms, phase (`Creating`/`RemoteCreated`/`Linked`) |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | DurableUserData |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write (`atomic_replace`, `Durability::DurableUserData`) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | An unreadable marker fails closed — the library is not relinked automatically |
| **Unknown-field policy** | Serde default (ignored) |
| **Backup inclusion** | No |
| **Migration owner** | N/A (no schema versioning beyond the `schema` field) |
| **Sync relevance** | Guards cross-library relink after a partially completed server-side create |

### 9. Auto-Sync Status

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/auto-sync-status.toml` |
| **Owner module** | `src/auto_sync/status.rs` |
| **Schema** | TOML with last_attempt, consecutive_failures, backoff, etc. |
| **User-editable** | No |
| **Secret classification** | None (secrets redacted) |
| **Durability class** | DurableUserData |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write (`atomic_write_bytes`, `Durability::DurableUserData`) with CRC32 integrity over the behaviour-driving fields |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Rebuilt from defaults |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/status.rs` |
| **Sync relevance** | Not synced |

### 10. Worker/Execution Locks

| Property | Value |
|----------|-------|
| **Canonical path** | `auto-sync-execution.lock` / `auto-sync-worker.lock` / `auto-sync-pending.lock` in the state dir |
| **Owner module** | `src/auto_sync/execution_lock.rs` (+ `lock.rs` re-export shim), `src/auto_sync/pending_lock.rs` |
| **Schema** | Kernel-backed lock file (diagnostic PID/nonce/start-token metadata; kernel is authoritative) |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | None applied — lock files are created with `create_new`, not through the atomic-write layer |
| **Max size** | ~1KB |
| **Atomicity method** | File creation with create_new |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Staleness decided by process liveness (`is_stale`); 30s default acquire timeout |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/lock.rs` |
| **Sync relevance** | Not synced |

### 11. Logs and Audit Trail

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/logs/snp.log` (tracing log) and `~/.config/snp/audit.log` (audit trail) |
| **Owner module** | `src/logging.rs` |
| **Schema** | Structured log text (`snp.log`); pipe-delimited audit records (`audit.log`) |
| **User-editable** | No |
| **Secret classification** | None (secrets redacted in production) |
| **Durability class** | None applied — both files are appended, not written through the atomic layer |
| **Max size** | `snp.log`: daily rotation (`tracing_appender::rolling::daily`). `audit.log`: rotated at 10 MB, rotated files retained 30 days |
| **Atomicity method** | Appender-based (tracing-appender); audit entries are appended synchronously |
| **Permissions** | Log directory `0o700` on Unix; `audit.log` `0o600`; `snp.log` uses the appender's default file mode |
| **Corruption handling** | Rotation |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | No |
| **Migration owner** | `logging.rs` |
| **Sync relevance** | Not synced |

### 12. Backup Snapshots

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/backups/<timestamp>/` (directory) |
| **Owner module** | `src/commands/backup_cmd.rs` (+ `src/commands/backup_archive.rs`) |
| **Schema** | `manifest.toml` + `libraries/*.toml` + `libraries.toml` + optional `usage.toml` + optional `sync.toml` (with the `api_key` line redacted) |
| **User-editable** | No (tool-generated) |
| **Secret classification** | None by default; `--include-sync-state` copies `sync.toml` with the `api_key` line replaced by `"<redacted>"` |
| **Durability class** | None applied — the manifest is written with `write_private_atomic`; the directory itself is staged and renamed into place |
| **Max size** | ~100MB |
| **Atomicity method** | Directory staged then renamed into place; in-memory snapshot prevents inconsistent writes |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | SHA-256 checksums in manifest verify each file |
| **Unknown-field policy** | Version-gated manifest (`manifest.schema`) |
| **Backup inclusion** | No (is a backup) |
| **Migration owner** | N/A — the manifest carries its own format version |
| **Sync relevance** | Not synced |

### 13. Transaction Journals

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/.transaction/txn-<uuid>.toml` (alongside `transaction.lock`) and `~/.config/snp/.transaction/artifacts/<txn-id>/` for staged files and durable backups |
| **Owner module** | `src/transaction.rs` |
| **Schema** | TOML manifest of staged operations (source, destination, backup, hash, action) |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | ~10KB |
| **Atomicity method** | Atomic write (`write_private_atomic`) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Recovery on startup: prepared journals are rolled back |
| **Unknown-field policy** | Serde default (unknown fields ignored) |
| **Backup inclusion** | No |
| **Migration owner** | N/A (no schema versioning) |
| **Sync relevance** | Not synced |

### 14. Import Outputs

| Property | Value |
|----------|-------|
| **Canonical path** | User-specified output path (the `--report-file` JSON report) |
| **Owner module** | `src/commands/import_cmd.rs` (no export command exists in this crate) |
| **Schema** | JSON import report |
| **User-editable** | Yes (destination) |
| **Secret classification** | None |
| **Durability class** | User-controlled |
| **Max size** | Unbounded |
| **Atomicity method** | Report file written with `write_private_atomic`; the destination path is user-specified |
| **Permissions** | `0o600` on Unix (set by the atomic writer) |
| **Corruption handling** | N/A |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | N/A |
| **Migration owner** | `import_cmd.rs` |
| **Sync relevance** | Import triggers sync |

### 15. Migration Metadata

| Property | Value |
|----------|-------|
| **Canonical path** | None (in-memory, or inline as `schema_version` in the library TOML's top-level table) |
| **Owner module** | `src/library/` (layout migration) |
| **Schema** | N/A |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | N/A |
| **Max size** | N/A |
| **Atomicity method** | N/A (state inferred) |
| **Permissions** | N/A |
| **Corruption handling** | Migration idempotent |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | N/A |
| **Migration owner** | `LibraryManager::migrate_from_single_file` |
| **Sync relevance** | Not synced |

### 16. Premade Libraries

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/premade/<name>.toml` |
| **Owner module** | `src/library/` (`save_premade_library`) |
| **Schema** | Same as snippet libraries |
| **User-editable** | No (downloaded from server) |
| **Secret classification** | None |
| **Durability class** | None applied (`write_private_atomic` takes no class) |
| **Max size** | ~1MB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Re-download from server |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No (re-downloadable) |
| **Migration owner** | `premade_cmd.rs` |
| **Sync relevance** | Synced from server |

## Durability Classes

The authoritative definition — per-class fsync behaviour, permissions, and
symlink policy — is
[architecture/persistence.md § Durability Classes](../architecture/persistence.md#durability-classes).
The two facts this inventory depends on:

- All four classes apply `mode(0o600)` to the temp file; they differ **only** in
  fsync behaviour. `DurableUserData` and `SensitiveConfig` both fsync the file
  and **require** a parent-directory fsync; `SensitiveConfig` additionally
  rejects a symlinked target. `RecoverableMetadata` does not fsync the file and
  downgrades a parent-directory fsync failure to a warning.
  `EphemeralCoordination` skips both fsyncs and currently has **no production
  caller**.
- A class is applied only through `atomic_replace` / `atomic_write_bytes`.
  `write_private_atomic` takes no class — it always fsyncs the file and fsyncs
  the parent directory on Unix only.

## Key Findings

1. Durable writes are split across two paths: `write_private_atomic` (no class,
   always file fsync) and `atomic_replace` / `atomic_write_bytes` (class
   applied). The latter is used for `themes.toml`,
   `auto-sync-pending.toml`, `auto-sync-status.toml`, the library sync-recovery
   marker, restore destinations, and `snp select --output`.
2. There is no `fs::write` or truncate-in-place for user data, with one
   exception: bundled theme files under `themes/` are seeded with a plain
   `fs::write` (write-if-missing).
3. Backup is a timestamped directory built from an in-memory snapshot, staged
   and renamed into place.
4. The API key uses keychain integration — `sync.toml` holds an `@keychain`
   marker and plaintext storage is refused by default. The file that does hold
   the settings is written `0o600`.
5. `validate_target` rejects directories, FIFOs, sockets, and block/char
   devices. Symlinks are rejected only when the caller opts in
   (`Durability::SensitiveConfig`); otherwise the symlink is *replaced*, not
   followed. `write_private_atomic` does not call `validate_target` at all.
6. `atomic_replace` applies the durability class: `DurableUserData` and
   `SensitiveConfig` both fsync the file **and require** a parent-directory
   fsync (a best-effort parent sync applies only to `RecoverableMetadata`).

> **Known live defect (Windows).** `atomic_replace` renames the target first and
> only then calls `sync_parent_dir`, while `parent_dir_sync_durable` returns
> `Err` on non-Unix. On Windows a `DurableUserData`/`SensitiveConfig`
> `atomic_replace` therefore replaces the target and *then* returns an error,
> never returning the `AtomicWriteReport`. `write_private_atomic` is not
> affected — its parent-directory sync is `#[cfg(unix)]`-gated.


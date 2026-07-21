# Persistence Inventory

This document catalogs every persisted artifact in snip-it. It is the authoritative reference for Phase 07A durability work.

For each artifact: canonical path derivation, owner module/layer, schema/version, user-editable vs private, secret classification, durability class, max supported size, atomicity method, permissions/ACL expectation, symlink/non-regular-file policy, corruption handling, unknown-field policy, backup inclusion default, migration owner, and synchronization relevance.

## Artifacts

### 1. Snippet Libraries

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/libraries/<name>.toml` (library mode) or `~/.config/snp/snippets.toml` (legacy single-file) |
| **Owner module** | `src/library.rs` (data structures, load/save), `src/commands/mod.rs` (load_snippets/save_snippets for legacy) |
| **Schema** | TOML with `[[snippets]]` table (pet-compatible), fields: id, description, command, output, tag, folders, favorite, created_at, updated_at, device_id, deleted |
| **User-editable** | Yes (primary user asset) |
| **Secret classification** | None |
| **Durability class** | DurableUserData |
| **Max size** | Unbounded (practical: ~10MB per library) |
| **Atomicity method** | `utils::atomic::write_private_atomic` (temp file + rename) |
| **Permissions** | `0o600` on Unix (private) |
| **Symlink policy** | Created atomically, no symlink support |
| **Corruption handling** | Backup before save (`.toml.bak`), corrupted parse creates `.toml.corrupt.bak` |
| **Unknown-field policy** | Serde `#[serde(deny_unknown_fields)]` NOT used; unknown fields silently ignored |
| **Backup inclusion** | Default (primary content) |
| **Migration owner** | `LibraryManager::migrate_from_single_file` |
| **Sync relevance** | Primary sync payload |

### 2. Library Index (`libraries.toml`)

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/libraries.toml` |
| **Owner module** | `src/library.rs` (`LibraryManager::save_config`) |
| **Schema** | TOML with `[[libraries]]` array (filename, library_id, is_primary, last_sync, server_id) |
| **User-editable** | No (managed by tool) |
| **Secret classification** | None |
| **Durability class** | DurableUserData |
| **Max size** | ~1KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Uses defaults on parse failure |
| **Unknown-field policy** | Serde default (ignored) |
| **Backup inclusion** | Yes (required for restore) |
| **Migration owner** | `LibraryManager` |
| **Sync relevance** | Not synced |

### 3. Sync Configuration

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/sync.toml` |
| **Owner module** | `src/config.rs` (`SyncSettings`) |
| **Schema** | TOML with server_url, api_key (keychain), device_id, direction, auto_sync, etc. |
| **User-editable** | Partially (server_url, direction) |
| **Secret classification** | API key is secret (stored in keychain when available) |
| **Durability class** | SensitiveConfig |
| **Max size** | ~1KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Falls back to defaults |
| **Unknown-field policy** | Serde default (ignored) |
| **Backup inclusion** | No (contains keychain refs) |
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
| **Durability class** | RecoverableMetadata |
| **Max size** | ~100KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` (via save path) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Recreated on next use |
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
| **Durability class** | RecoverableMetadata |
| **Max size** | ~1KB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
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
| **Durability class** | RecoverableMetadata |
| **Max size** | ~100KB per theme |
| **Atomicity method** | File copy (premade download) or atomic write |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Skipped on load failure |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No |
| **Migration owner** | `ui/theme.rs` |
| **Sync relevance** | Not synced |

### 7. Pending Sync Markers

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/auto-sync-pending/<device_id>.pending` |
| **Owner module** | `src/auto_sync/pending.rs` |
| **Schema** | Simple marker file with generation number |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | EphemeralCoordination |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Treated as stale if invalid |
| **Unknown-field policy** | N/A (private format) |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/pending.rs` |
| **Sync relevance** | Drives sync scheduling |

### 8. Auto-Sync Status

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/auto-sync-status.toml` |
| **Owner module** | `src/auto_sync/status.rs` |
| **Schema** | TOML with last_attempt, consecutive_failures, backoff, etc. |
| **User-editable** | No |
| **Secret classification** | None (secrets redacted) |
| **Durability class** | RecoverableMetadata |
| **Max size** | ~1KB |
| **Atomicity method** | Atomic write with CRC32 integrity |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Rebuilt from defaults |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/status.rs` |
| **Sync relevance** | Not synced |

### 9. Worker/Execution Locks

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/auto-sync-locks/<device_id>.lock` |
| **Owner module** | `src/auto_sync/lock.rs`, `src/auto_sync/execution_lock.rs` |
| **Schema** | Simple lock file |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | EphemeralCoordination |
| **Max size** | ~1KB |
| **Atomicity method** | File creation with create_new |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Staleness timeout (30s default) |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | No |
| **Migration owner** | `auto_sync/lock.rs` |
| **Sync relevance** | Not synced |

### 10. Logs

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/logs/snp.log` |
| **Owner module** | `src/logging.rs` |
| **Schema** | Structured log text |
| **User-editable** | No |
| **Secret classification** | None (secrets redacted in production) |
| **Durability class** | EphemeralCoordination |
| **Max size** | Rotated (10MB max) |
| **Atomicity method** | Appender-based (tracing-appender) |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Rotation |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | No |
| **Migration owner** | `logging.rs` |
| **Sync relevance** | Not synced |

### 11. Backup Manifests/Archives

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/backups/<name>.tar.gz` (planned Phase 07A) |
| **Owner module** | New: `src/backup.rs` (Phase 07A) |
| **Schema** | TOML manifest + archive |
| **User-editable** | No (tool-generated) |
| **Secret classification** | None (excludes secrets) |
| **Durability class** | DurableUserData |
| **Max size** | ~100MB |
| **Atomicity method** | Atomic write of archive |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Checksum verification |
| **Unknown-field policy** | Version-gated |
| **Backup inclusion** | No (is a backup) |
| **Migration owner** | Phase 07A |
| **Sync relevance** | Not synced |

### 12. Transaction Journals (Planned Phase 07A)

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/transaction-journals/<id>.journal` |
| **Owner module** | New: `src/transaction.rs` (Phase 07A) |
| **Schema** | TOML manifest of staged operations |
| **User-editable** | No |
| **Secret classification** | None |
| **Durability class** | EphemeralCoordination |
| **Max size** | ~10KB |
| **Atomicity method** | Atomic write |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Refuse automatic recovery |
| **Unknown-field policy** | Strict |
| **Backup inclusion** | No |
| **Migration owner** | Phase 07A |
| **Sync relevance** | Not synced |

### 13. Import/Export Outputs

| Property | Value |
|----------|-------|
| **Canonical path** | User-specified output path |
| **Owner module** | `src/commands/import_cmd.rs`, export logic |
| **Schema** | Pet-compatible or native TOML |
| **User-editable** | Yes (destination) |
| **Secret classification** | None |
| **Durability class** | User-controlled |
| **Max size** | Unbounded |
| **Atomicity method** | User responsibility |
| **Permissions** | Default |
| **Corruption handling** | N/A |
| **Unknown-field policy** | N/A |
| **Backup inclusion** | N/A |
| **Migration owner** | `import_cmd.rs` |
| **Sync relevance** | Import triggers sync |

### 14. Migration Metadata

| Property | Value |
|----------|-------|
| **Canonical path** | None (in-memory or inline in libraries.toml) |
| **Owner module** | `src/library.rs` (layout migration) |
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

### 15. Premade Libraries

| Property | Value |
|----------|-------|
| **Canonical path** | `~/.config/snp/premade/<name>.toml` |
| **Owner module** | `src/library.rs` (`save_premade_library`) |
| **Schema** | Same as snippet libraries |
| **User-editable** | No (downloaded from server) |
| **Secret classification** | None |
| **Durability class** | RecoverableMetadata |
| **Max size** | ~1MB |
| **Atomicity method** | `utils::atomic::write_private_atomic` |
| **Permissions** | `0o600` on Unix |
| **Corruption handling** | Re-download from server |
| **Unknown-field policy** | Serde default |
| **Backup inclusion** | No (re-downloadable) |
| **Migration owner** | `premade_cmd.rs` |
| **Sync relevance** | Synced from server |

## Durability Classes

- **DurableUserData**: Primary user-authored content (libraries, index). Must survive crashes. Requires fsync.
- **SensitiveConfig**: Configuration with secrets (sync.toml). Private permissions, no backup of secrets.
- **RecoverableMetadata**: Can be recreated or is low-value (usage, theme, status). Crash-safe but less critical.
- **EphemeralCoordination**: Locks, pending markers, logs. Transient; staleness/timeout handles corruption.

## Key Findings

1. All durable writes already use `write_private_atomic` (temp file + rename with `0o600`)
2. No `fs::write` or truncate-in-place for user data
3. Backup is timestamped and capped at 10 per library
4. API key uses keychain integration; plaintext fallback in sync.toml is protected by `0o600`
5. No symlink/device/FIFO rejection in current atomic primitive (add in Phase 07A)
6. No `sync_all()` or `fsync()` on rename (Phase 07A enhancement)

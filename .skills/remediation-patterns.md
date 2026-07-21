# Remediation Patterns for snp

## Key Patterns Used in Remediation

### 1. Security: Keychain Integration (keyring crate)
- `keyring = "3"` for cross-platform OS keychain access
- `Entry::new(service, user)` to create credential entries
- `entry.set_password()` / `entry.get_password()` for storage/retrieval
- Graceful fallback: if keychain unavailable, store plaintext with warning
- Migration: detect plaintext on load, move to keychain, save marker

### 2. Security: Rate Limiting
- Rate limit check BEFORE auth check (cheaper operation first)
- Use server-controlled keys (IP address) not client-controlled (device_id)
- Pattern: `rate_limiter.allow(&key, limit, window).await`

### 3. Security: CORS Configuration
- Read env vars at server startup: `std::env::var("CORS_ALLOW_ALL")`
- `CorsLayer::new().allow_origin(Any)` for permissive mode
- Log configuration for debugging

### 4. Bug Fixes: Race Conditions
- Use generation counters (`AtomicU64`) instead of `AtomicBool`
- Increment counter on each new schedule
- Sleeping thread checks if its generation matches current counter
- Prevents stale timers from affecting new operations

### 5. Bug Fixes: Error Propagation
- Return `Err()` instead of silent defaults on data loss conditions
- Backup files before returning errors so callers can recover
- Use `?` operator for propagation in callers

### 6. Bug Fixes: Data Integrity
- Check for existing entries before inserting (prevent duplicates)
- Validate input parameters (e.g., interval >= 1)
- Use tie-breaking for concurrent updates (device_id as tiebreaker)

### 7. Code Quality: Extract Repeated Patterns
- Identify copy-pasted auth+rate-limit blocks
- Extract into helper method: `authenticate_and_rate_limit(&self, api_key)`
- Reduces code duplication and ensures consistency

### 8. Code Quality: Module Splitting
- Move independent types to appropriate modules (e.g., Variable struct)
- Break large files into submodules with re-exports
- Maintain public API via re-exports in mod.rs

### 9. Performance: SQL Optimization
- Replace correlated subqueries with JOINs
- Use `LEFT JOIN ... GROUP BY` for counts
- Add indexes for frequently queried columns

### 10. Removing Dead Public Items
- Audit public API surface before releasing (see `docs/PUBLIC_API.md`)
- Remove unused fields, constants, and functions from public types
- For removed items: verify no callers exist in the workspace (`rg <item_name>`)
- Apply `#[non_exhaustive]` to public enums to prevent future breakage from variant additions
- Document removed items in `docs/OBSOLETE_ITEMS.md` with rationale
- Common pattern: a field/method was added speculatively but never wired up — delete it before it becomes a stability commitment

### 11. Clippy Compliance
- Use `sort_by_key` instead of `sort_by` for simple key extraction
- Collapse nested `if` into match arm guards where practical
- Use `#[allow(clippy::...)]` for complex patterns that can't be collapsed

### 12. Atomic write with durability classes
- Use `atomic_replace` with `AtomicWriteOptions::for_durability()` instead of raw `fs::write` for all user-data files
- Match durability class to data criticality:
  - `DurableUserData` for libraries
  - `SensitiveConfig` for credentials
  - `RecoverableMetadata` for caches
  - `EphemeralCoordination` for locks

### 13. Transaction journals for multi-file ops
- Use `transaction.rs` for any operation touching 2+ files
- Begin → stage → commit removes the journal
- On startup, check for interrupted journals via `check_interrupted_transactions`
- Never schedule sync before local transaction is consistent

### 14. Schema versioning for migrations
- Use `migration.rs` with `SchemaVersion` ordinal type
- `write_schema_version` uses `toml::Table` (not `toml::Value`) to preserve array-of-tables structure
- Always test idempotency: second run should be a no-op

### 15. Validation-first repair
- Always run validation before repair
- Safe repairs: rebuild index, fix primary selection, remove orphans, generate missing IDs
- Refuse: same-ID divergence, partially parseable TOML, corrupt pending intent, multiple primary candidates

### 16. Backup before destructive operations
- `backup_cmd` creates secret-free snapshots with SHA-256 checksums
- Default excludes API keys, locks, logs
- `restore_cmd --replace` creates automatic pre-restore backup
- `repair --apply` creates pre-repair backup

## Testing Approach
- Unit tests for individual functions
- Integration tests with TempDir for file system operations
- Server tests with `sqlite::memory:` for database isolation
- Run `cargo clippy --all-targets -- -D warnings` before committing
- Run `cargo fmt --check` to verify formatting

## Phase 06A Dead Items

The following dead items were identified and removed during the API tightening audit:

- **`AutoSyncPolicy.max_retries`** — field was never read; backoff is now durable and retry-count-based via `auto-sync-status.toml`. Do not re-add; use `schedule_sync()` backoff decisions instead.
- **`STALE_LOCK_THRESHOLD_SECS`** — constant was unused; lock staleness is handled by timeout logic and `kill -0` process liveness checks. Do not re-add; use timeout-based staleness detection.
- **`encryption::ct_eq`** — constant-time equality helper was unreferenced; replaced by downstream crate functionality. Do not re-add.

Public enums now carry `#[non_exhaustive]` to allow future variant additions without breaking downstream callers.

## Phase 07A Patterns

The following patterns were introduced in Phase 07A:

### Durability Classes (`src/utils/atomic.rs`)
| Class | Use case | fsync |
|-------|----------|-------|
| `DurableUserData` | Libraries, snippets | fsync parent + file |
| `SensitiveConfig` | Credentials, sync settings | fsync parent + file |
| `RecoverableMetadata` | Caches, status | file only |
| `EphemeralCoordination` | Locks, temp state | no fsync |

### Transaction Journal (`src/transaction.rs`)
- Operations touching 2+ files must use `Transaction::begin()`
- Journal lives at `<config>/transaction.journal` with operation list + file checksums
- On crash recovery: `check_interrupted_transactions()` rolls back incomplete transactions
- Journal is removed only after successful commit

### Schema Migrations (`src/migration.rs`)
- `SchemaVersion` ordinal type tracks schema state
- `write_schema_version` preserves TOML array-of-tables structure
- Migrations are idempotent: re-running applies no changes
- `current_schema_version()` reads from `snippets.toml` header

### Validation (`src/commands/validate_cmd.rs`)
- Read-only checks: orphan detection, primary selection, ID uniqueness, TOML parseability
- Run validation before any repair operation
- Report severity: Warning vs Error; only Errors block repair

### Backup (`src/commands/backup_cmd.rs`)
- Creates snapshot directory with `manifest.toml` + SHA-256 checksums
- Excludes: `api_key`, `lock`, `logs`, `themes`
- `restore_cmd --replace` auto-creates pre-restore backup
- `repair --apply` auto-creates pre-repair backup

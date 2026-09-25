# Remediation Patterns for snp

## Key Patterns Used in Remediation

### 1. Security: Keychain Integration (keyring crate)
- `keyring = "4"` for cross-platform OS keychain access (default `v1` features)
- `Entry::new(service, user)` to create credential entries
- `entry.set_password()` / `entry.get_password()` for storage/retrieval
- Graceful fallback: if keychain unavailable, store plaintext with warning
- Migration: detect plaintext on load, move to keychain, save marker
- Tests bypass the OS keychain via `SNP_ALLOW_PLAINTEXT_API_KEY=true`

### 2. Security: Rate Limiting
- Rate limit check BEFORE auth check (cheaper operation first)
- Use server-controlled keys (IP address) not client-controlled (device_id)
- Pattern: `rate_limiter.allow(&key, max_requests, window).await` (`snip-sync/src/rate_limiter.rs:45` takes `&self, key: &str, max_requests: usize, window: Duration -> bool`); canonical entry is `authenticate_and_rate_limit[_with_duration]` (`snip-sync/src/lib.rs`)

### 3. Security: CORS Configuration
- EggServe leaf service only — do NOT reintroduce Axum/Tower-HTTP `CorsLayer`. HTTP policy lives in `snip-sync/src/http.rs` (`CorsPolicy`, `service()`); `CORS_ALLOW_ALL` is strictly parsed at startup (`snip-sync/src/main.rs`) and ignored on non-loopback binds. Keep `Vary: origin` / `Allow: GET, HEAD` wire parity (locked in `tests/snip_sync_lifetime.rs`).

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
- Conflict order is `(updated_at, device_id, SHA-256(synced fields))` — never role-dependent server-wins; deletion beats live content even with an older timestamp

### 7. Code Quality: Extract Repeated Patterns
- Identify copy-pasted auth+rate-limit blocks
- Extract into helper method: `authenticate_and_rate_limit(&self, api_key)`
- Reduces code duplication and ensures consistency

### 8. Code Quality: Module Splitting
- Keep `commands/mod.rs` (`load/save_snippets`, `run_snippet_selection`) and `ui/mod.rs` re-exports stable — they affect all TUI commands; any function moved to a submodule needs a re-export for `commands/` callers
- Break large files into submodules with re-exports; maintain public API via re-exports in mod.rs

### 9. Performance: SQL Optimization
- Prefer measured, local changes; keep `snip-sync` update on external `curl` (the embedded-transport trial bloated the server +34%, past the 10% gate — see `tests/architecture.rs`). Add indexes for frequently queried columns.

### 10. Removing Dead Public Items
- Audit public API surface before releasing (see `docs/PUBLIC_API.md`)
- Remove unused fields, constants, and functions from public types
- For removed items: verify no callers exist in the workspace (`rg <item_name>`) and record rationale in the CHANGELOG
- Apply `#[non_exhaustive]` to public enums to prevent future breakage from variant additions
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
- Use free functions in `transaction.rs` (`begin_transaction` / `advance_to_*` / `commit_transaction`) for any operation touching 2+ files — there is no `Transaction::begin()` API
- Journals live as `txn-<uuid>.toml` files in `<config>/.transaction/`
- Gate every local mutation on `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` (`src/transaction.rs:2441`): one journal = auto-rollback, multiple/incomplete = refuse and direct to `snp repair`. `check_interrupted_transactions` is a scan helper, not the gate
- Never schedule sync before local transaction is consistent

### 14. Schema versioning for migrations
- Use `migration.rs` with `SchemaVersion` (`LEGACY(0)`/`CURRENT(1)`); read via `get_schema_version()`
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
- Serial targets (`*_concurrency`, `sync_multibatch`, `*_barriers`, `pty_integration`, `repair_transactions`) need `--test-threads=1`; `repair_transactions`/`process_lock_concurrency`/`local_data_lock_barriers` (+ helper bin) need `--features test-support`
- Run `cargo clippy --workspace --all-targets -- -D warnings` (NOT `--all-features`) before committing
- Run `cargo fmt --all -- --check` to verify formatting
- `sync.rs` RPCs retry through the single `retry_grpc_unified!` macro — never reintroduce a closure-based generic retry helper (fails with "captured variable cannot escape `FnMut`")

## Phase 06A Dead Items (Removed)

The following dead items were identified and **removed** during the API tightening audit:

- **`AutoSyncPolicy.max_retries`** — **REMOVED.** Field was never read; backoff is now durable and retry-count-based via `auto-sync-status.toml`. Do not re-add; use `schedule_sync()` backoff decisions instead.
- **`STALE_LOCK_THRESHOLD_SECS`** — **REMOVED.** Constant was unused; lock staleness is handled by timeout logic and `kill -0` process liveness checks. Do not re-add; use timeout-based staleness detection.
- **`encryption::ct_eq`** — **Removed from public API.** The constant-time equality helper was unreferenced by production code; it survives only as a `#[cfg(test)]` test helper in `encryption.rs`. Keep it out of the public surface.

Public enums now carry `#[non_exhaustive]` to allow future variant additions without breaking downstream callers.

## Phase 07A Patterns

The following patterns were introduced in Phase 07A:

### Durability Classes (`src/utils/atomic.rs`)
| Class | Use case | fsync |
|-------|----------|-------|
| `DurableUserData` | Libraries, snippets | fsync file + fsync parent |
| `SensitiveConfig` | Credentials, sync settings | fsync parent only, 0o600 perms, symlink rejection |
| `RecoverableMetadata` | Caches, status | fsync parent only |
| `EphemeralCoordination` | Locks, temp state | no fsync |

### Transaction Journal (`src/transaction.rs`)
- Operations touching 2+ files must use the `begin_transaction` / `advance_to_*` / `commit_transaction` free functions
- Journals live as `txn-<uuid>.toml` files in `<config>/.transaction/` (alongside locks, durable backups, and staged files)
- On crash recovery: `gate_mutation_on_interrupted_transactions()` rolls back a single interrupted journal, refuses on multiple/incomplete
- Journal is removed only after successful commit

### Schema Migrations (`src/migration.rs`)
- `SchemaVersion` (`LEGACY(0)`/`CURRENT(1)`) tracks schema state
- `write_schema_version` preserves TOML array-of-tables structure
- Migrations are idempotent: re-running applies no changes
- `get_schema_version()` reads from the file header

### Validation (`src/commands/validate_cmd.rs`)
- Read-only checks: orphan detection, primary selection, ID uniqueness, TOML parseability
- Run validation before any repair operation
- Report severity: Warning vs Error; only Errors block repair

### Backup (`src/commands/backup_cmd.rs`)
- Creates snapshot directory with `manifest.toml` + SHA-256 checksums
- Excludes: `api_key`, `lock`, `logs`, `themes`
- `restore_cmd --replace` auto-creates pre-restore backup
- `repair --apply` auto-creates pre-repair backup

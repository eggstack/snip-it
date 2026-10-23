# Remediation Patterns for snp

## Key Patterns Used in Remediation

### 1. Security: Keychain Integration (keyring crate)
- `keyring = "4"` for cross-platform OS keychain access (default `v1` features)
- `Entry::new(service, user)` to create credential entries
- `entry.set_password()` / `entry.get_password()` for storage/retrieval
- No plaintext downgrade: a production save **refuses** when the keychain is unavailable rather than falling back (`src/config/sync_settings.rs` surfaces the keychain write error; it is never downgraded to a plaintext key)
- Migration: detect plaintext on load, move to keychain, save marker
- Tests bypass the OS keychain via `SNP_ALLOW_PLAINTEXT_API_KEY=true` — that seam is `test-support`-gated and compiled out of production builds

### 2. Security: Rate Limiting
- Rate limit check BEFORE the database auth lookup (cheaper operation first); only the local empty-key and max-length checks precede it
- Use server-controlled keys (IP address) not client-controlled (device_id): the key comes from `peer_rate_limit_key` (`snip-sync/src/lib.rs`), which honours `x-forwarded-for` only from a configured trusted proxy
- Pattern: `rate_limiter.allow(&key, max_requests, window).await` (`snip-sync/src/rate_limiter.rs:46` takes `&self, key: &str, max_requests: usize, window: Duration -> bool`); canonical entry is `authenticate_and_rate_limit[_with_duration]` (`snip-sync/src/lib.rs`)

### 3. Security: CORS Configuration
- EggServe leaf service only — do NOT reintroduce Axum/Tower-HTTP `CorsLayer`. HTTP policy lives in `snip-sync/src/http.rs` (`CorsPolicy`, `service()`); `CORS_ALLOW_ALL` is strictly parsed at startup (`snip-sync/src/main.rs`) and ignored on non-loopback binds. Keep `Vary: origin` / `Allow: GET, HEAD` wire parity (locked in `tests/snip_sync_lifetime.rs`).

### 4. Bug Fixes: Race Conditions
- Use generation counters instead of a bare `AtomicBool` flag
- In-memory variant: `AtomicU64` incremented on each new operation, and the sleeping worker compares its captured generation against the current one (`src/clipboard.rs` — `CLIPBOARD_GENERATION`)
- Durable variant: the pending generation is an on-disk `u64` in `src/auto_sync/pending.rs`; the clear step only proceeds when the observed generation still matches
- Prevents stale timers from affecting new operations

### 5. Bug Fixes: Error Propagation
- Return `Err()` instead of silent defaults on data loss conditions
- Backup files before returning errors so callers can recover
- Use `?` operator for propagation in callers

### 6. Bug Fixes: Data Integrity
- Check for existing entries before inserting (prevent duplicates)
- Validate input parameters on load (e.g. `sync_limit` uses a positive-only deserializer; auto-sync debounce, max delay, and timeout are clamped to documented ranges in `src/config/sync_settings.rs`)
- Conflict order is `(updated_at, device_id, SHA-256(synced fields))` — never role-dependent server-wins; deletion beats live content even with an older timestamp

### 7. Code Quality: Extract Repeated Patterns
- Identify copy-pasted auth+rate-limit blocks
- Extract into helper method: `authenticate_and_rate_limit(&self, api_key: &str, rate_limit_key: &str)`
- Reduces code duplication and ensures consistency

### 8. Code Quality: Module Splitting
- Keep `commands/mod.rs` (`load/save_snippets`, `run_snippet_selection`) and `ui/mod.rs` re-exports stable — they affect all TUI commands; any function moved to a submodule needs a re-export for `commands/` callers
- Break large files into submodules with re-exports; maintain public API via re-exports in mod.rs

### 9. Performance: SQL Optimization
- Prefer measured, local changes; keep `snip-sync` update on external `curl` — the embedded-transport trial bloated the server +34%, past the 10% gate (`snip-sync/src/update.rs` module docs). `tests/architecture.rs` pins the `snp` half only (no `curl` in `src/update.rs`, lean `eggfetch-core` profile); `snip-sync/src/update.rs` is deliberately excluded from that scan
- Add indexes for frequently queried columns (`snip-sync/src/db.rs` indexes users by API-key prefix, snippets by user/library/updated_at, libraries by user)

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
- Never use raw `fs::write` for user-data files. Two helpers cover the two shapes:
  - `write_private_atomic(path, content, temp_prefix)` — always fsyncs the file; fsyncs the parent directory only under `#[cfg(unix)]`. This is the library/snippet save path (`src/library/persistence.rs`, `src/commands/mod.rs`)
  - `atomic_replace(target, bytes, &AtomicWriteOptions::for_durability(class))` — class-driven fsync, permission control, and target validation
- Match durability class to data criticality:
  - `DurableUserData` — sync recovery markers, auto-sync status and pending markers, `select --output` files, restored files, rollback restores
  - `SensitiveConfig` — theme files and restored credential files (adds symlink rejection on top of `DurableUserData`)
  - `RecoverableMetadata` — no production caller yet; currently exercised only by `src/utils/atomic.rs` tests

### 13. Transaction journals for multi-file ops
- Use free functions in `transaction.rs` (`begin_transaction` / `advance_to_*` / `commit_transaction`) for any operation touching 2+ files — there is no `Transaction::begin()` API
- Journals live as `txn-<uuid>.toml` files in `<config>/.transaction/`
- Gate every local mutation on `gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)` (`src/transaction.rs:2510`): one *actionable* journal = auto-recover (rollback, finalize committed-local, resume cleanup), multiple actionable = refuse and direct to `snp repair`. Corrupt journals and `Failed`-state journals refuse even when only one journal exists. `check_interrupted_transactions` is a scan helper, not the gate
- Never schedule sync before local transaction is consistent

### 14. Schema versioning for migrations
- The persisted library format carries **no schema version**; the dead versioning framework (`src/migration.rs`) was removed
- Introducing a schema version is a **schema change**: it requires a migration that is idempotent (a second run must be a no-op) and a version bump

### 15. Validation-first repair
- Always run validation before repair (`src/commands/validate_cmd.rs` emits a `Severity` and a `Repairability` per diagnostic)
- Safe (auto-applied by `repair --apply`): prune orphaned usage entries; transaction-journal recovery (rollback / finalize committed-local / resume cleanup / clean up legacy committed or rolled-back journals / remove a terminal journal); remove orphaned transaction artifact directories
- Refused (reported for manual handling only): library-index and config load failures including TOML syntax errors, snippet ID repair (empty or duplicate IDs), zero timestamps, primary-library selection (missing file, no primary, multiple candidates), corrupt transaction journals, unsafe artifact-inspection failures, and journals in the `Failed` state

### 16. Backup before destructive operations
- `backup_cmd` creates secret-free snapshots with SHA-256 checksums
- There is no exclusion list — the snapshot is a fixed file set: `libraries/*.toml` and `libraries.toml`, plus `usage.toml` only with `--include-usage` and `sync.toml` only with `--include-sync-state` (API key redacted before it is hashed and written). Locks, logs, and themes are absent because they are not part of that set
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

## Dead public items (removed — do not re-introduce)

Removed during the API tightening audit (closure record
`plans/closure/cli-library-sync-consolidation/`):

- **`AutoSyncPolicy.max_retries`** — **REMOVED.** Field was never read; backoff is now durable and retry-count-based via `auto-sync-status.toml`. Do not re-add; use `schedule_sync()` backoff decisions instead.
- **`STALE_LOCK_THRESHOLD_SECS`** — **REMOVED.** Constant was unused; lock staleness is handled by timeout logic and `kill -0` process liveness checks. Do not re-add; use timeout-based staleness detection.
- **`encryption::ct_eq`** — **Removed from public API.** The constant-time equality helper was unreferenced by production code; it survives only as a `#[cfg(test)]` test helper in `encryption.rs`. Keep it out of the public surface.

Public enums that may gain variants carry `#[non_exhaustive]` so additions don't
break downstream callers. This is **not** blanket: only the enums in
`src/error.rs`, `src/outcome.rs`, `src/sync_failure.rs`, `src/sort.rs`,
`src/encryption.rs`, `src/config/sync_settings.rs`, `src/lib.rs`,
`src/auto_sync/{worker,policy,execution_lock}.rs`, `src/commands/mod.rs`, and
`src/utils/atomic.rs` are marked. Enums in `src/status_snapshot.rs`,
`src/selector.rs`, `src/transaction.rs`, and `src/auto_sync/pending.rs` are
**not** — mark a new public enum when you add it rather than assuming the crate
already does. Verify the current set with
`rg -n 'non_exhaustive' src/ --glob '*.rs'` rather than trusting this list.

## Durability and validation patterns

The following patterns are established in source and covered by the
`persistence`-focused integration targets. `tests/architecture.rs` does not
enforce them — it pins layer dependencies, the `pub` surface, and the `snp`
updater transport only.

### Durability Classes (`src/utils/atomic.rs`)
All four classes create the temp file with `mode(0o600)` on Unix (the target
inherits it); they differ **only** in fsync behavior.

| Class | Use case | file fsync | parent-dir fsync | extra |
|-------|----------|------------|------------------|-------|
| `DurableUserData` | Recovery markers, auto-sync status/pending, restored files | yes | required (hard error on failure) | — |
| `SensitiveConfig` | Theme files, restored credential files | yes | required (hard error on failure) | symlink rejection (its only behavioral difference from `DurableUserData`) |
| `RecoverableMetadata` | Reconstructible metadata — no production caller yet | no | best-effort (warns, continues, returns `Ok(Some(false))`) | — |

`write_private_atomic` takes no class argument: it always fsyncs the file and
fsyncs the parent directory only under `#[cfg(unix)]`.

> Known live defect (do not "fix" casually): on Windows `atomic_replace`
> renames the target first and then calls `sync_parent_dir` unconditionally,
> while `parent_dir_sync_durable` returns `Err` on non-Unix. A durable-class
> `atomic_replace` on Windows therefore replaces the target and *then* returns
> an error, never returning the `AtomicWriteReport`. `write_private_atomic` is
> unaffected (its parent-dir fsync is `#[cfg(unix)]`-gated).

### Transaction Journal (`src/transaction.rs`)
- Operations touching 2+ files must use the `begin_transaction` / `advance_to_*` / `commit_transaction` free functions
- Journals live as `txn-<uuid>.toml` files in `<config>/.transaction/` (alongside locks, durable backups, and staged files)
- On crash recovery: `gate_mutation_on_interrupted_transactions()` auto-recovers a single actionable journal and refuses on multiple actionable journals, on corrupt journals, and on `Failed`-state journals
- Journal is removed only after successful commit

### Schema Migrations
- The persisted library format carries **no schema version**; the dead versioning framework (`src/migration.rs`) was removed as unreferenced code
- Adding a schema version is a schema change: ship a migration that is idempotent (re-running applies no changes) together with a version bump
- A migration that rewrites a library file must preserve TOML array-of-tables structure — operate on `toml::Table`, never `toml::Value`

### Validation (`src/commands/validate_cmd.rs`)
- Read-only checks: orphan detection, primary selection, ID uniqueness, TOML parseability
- Run validation before any repair operation
- Report severity as `Info` / `Warning` / `Error`, plus a `Repairability` of `Auto` / `Manual` / `Unrepairable`; only `Error` blocks repair

### Backup (`src/commands/backup_cmd.rs`)
- Creates snapshot directory with `manifest.toml` + SHA-256 checksums
- Captures `libraries/*.toml` and `libraries.toml`; `usage.toml` only with `--include-usage`, `sync.toml` only with `--include-sync-state` (API key redacted)
- `restore_cmd --replace` auto-creates pre-restore backup
- `repair --apply` auto-creates pre-repair backup

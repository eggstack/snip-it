# Architecture Review Skill

## Purpose
Guide agents through reviewing snip-it architecture documents against actual code.

## Review Process

### 1. Read the Architecture Document
```bash
cat architecture/<module>.md
```

### 2. Trace Claims to Code
For each claim in the document:
- Verify file paths exist
- Check struct definitions match
- Verify function signatures
- Confirm behavioral descriptions

### 3. Interrogate the Code
Look for:
- **Bugs**: Logic errors, edge cases, error handling gaps
- **Design Issues**: Tight coupling, unclear responsibilities, dead code
- **Security Concerns**: Especially in encryption, sync, server modules
- **Performance Issues**: Unnecessary allocations, O(n²) algorithms
- **Test Coverage Gaps**: Missing tests for critical paths

### 4. Write Findings
Report findings directly (session summary or PR description) with:
- Document Accuracy (verified correct + discrepancies)
- Bugs & Issues (with file:line locations)
- Design Issues
- Security Concerns
- Performance Issues
- Priority Ranking table (critical/high/medium/low)
- Recommendations

## Key Files to Check

`architecture/overview.md` § Deep-Dive Index is the authoritative module → doc
map — 61 deep dives, 62 documents in `architecture/` counting `overview.md`
itself. The table below is the review *shortcut* — the highest-risk
source files, grouped by the skill that covers them.

| Area | Primary Source Files | Skill / Doc |
|------|---------------------|-------------|
| CLI entry + dispatch | `src/main.rs`, `src/outcome.rs`, `src/usage.rs` | `architecture/cli.md`, `outcome.md` |
| Commands | `src/commands/` (one module per command) | `architecture/commands/*.md` |
| Data model + persistence | `src/library/{model,persistence,manager}.rs`, `src/migration.rs` | `.skills/persistence-and-toml-fidelity.md` |
| Config + credentials | `src/config/{mod,sync_settings,toml_cache}.rs`, `src/utils/config.rs` | `.skills/keychain-integration.md` |
| Selector + search parity | `src/selector.rs` | `.skills/selector-and-search-parity.md` |
| Sync client + merge | `src/sync.rs`, `src/sync_commands.rs`, `src/sync_failure.rs` | `.skills/sync-module.md` |
| Auto-sync | `src/auto_sync/` (11 files), `src/status_snapshot.rs` | `.skills/transactions-and-auto-sync.md` |
| Transactions + locks | `src/transaction.rs`, `src/local_data.rs`, `src/process_file_lock.rs` | `.skills/transactions-and-auto-sync.md` |
| Server | `snip-sync/src/` | `.skills/server-module.md` |
| Proto | `snip-proto/` | `architecture/proto.md` |
| MCP | `src/mcp/` | `.skills/selector-and-search-parity.md` |
| TUI | `src/ui/` (see `tui.md` for the event loop, `ui.md` for theme/highlight) | `.skills/ui-module.md` |
| Encryption | `src/encryption.rs` | `.skills/encryption-module.md` |
| Updater | `src/update.rs` + `snip-sync/src/update.rs` | `architecture/update.md` |
| Backup / restore / repair | `src/commands/{backup_cmd,backup_archive,restore_cmd,repair_cmd,validate_cmd}.rs` | `architecture/persistence.md` |
| Diagnostics / doctor | `src/diagnostics.rs`, `src/commands/{doctor_cmd,doctor_report,pet_analysis}.rs` | `architecture/diagnostics.md` |
| Cross-cutting | `src/clipboard.rs`, `src/logging.rs`, `src/error.rs`, `src/output.rs`, `src/sort.rs`, `src/utils/` | `architecture/utils.md`, `core.md` |
| Tests | `tests/`, `tests/support/`, `src/test_failpoints.rs` | `architecture/test-infrastructure.md` |

The two most invariant-heavy modules — `transaction.rs` + `local_data.rs` and
`auto_sync/` — are the easiest to miss if you scope a review from the CLI
surface. Check them on any change to local mutation or sync triggering.

## Common Patterns to Verify

### Security
- **Path canonicalization**: Output paths and editor paths should be canonicalized before use
- **TLS verification**: `src/sync.rs:1374` sets `.domain_name(host)` on `ClientTlsConfig` — verify it is still set after any TLS touch, don't regress it (deep dive: `architecture/sync.md`)
- **Shell execution**: `src/commands/run_cmd.rs:107-113` intentionally uses `$SHELL` with `/bin/sh` fallback (test asserts both accepted) — don't "harden" it to hardcoded `/bin/sh` (deep dive: `architecture/commands/run_cmd.md`)
- **Atomic file operations**: Use `fs::OpenOptions::create_new(true)` to prevent TOCTOU races

### Error Handling
- **Error propagation**: Functions should return `Result` and propagate errors via `?`
- **SnipError constructors**: Use `SnipError::io_error()`, `toml_error()`, `sync_failure()` etc. (`src/error.rs`); `CryptoError` converts via `impl From<CryptoError> for SnipError` (`src/error.rs:316`)
- **Silent failures**: Check for `let _ = ...` patterns that suppress errors without logging

### Sync
- **Deleted snippets**: `deleted: true` snippets should be filtered from TUI display (in `get_snippet_data()`, `src/commands/mod.rs:285`)
- **Conflict resolution**: Live conflicts order by `(updated_at, device_id, SHA-256(synced fields))` — never reintroduce role-dependent `>=` server-wins comparisons; deletion wins over live content regardless of timestamp (deep dive: `architecture/sync.md`)
- **Push-only counter**: `completed` should increment regardless of `has_failures` (`src/sync_commands.rs:838,841`)

### Known Historical Fixes (verify they're still in place)
- Encryption keys are zeroized after use — `architecture/encryption.md`
- Clipboard debug→warn for auto-clear failures — `architecture/clipboard.md`
- Visual mode `y` copies commands (not descriptions) — `architecture/tui.md`
- Premade writes are guarded by canonical containment, not the raw joined path — `architecture/commands/premade_cmd.md`
- Health RPC verifies database connectivity via `db.ping()` — `architecture/server.md`
- `CryptoError` converts to `SnipError::SyncFailure` via the `From` impl (`src/error.rs:316`)
- `From<io::Error>` auto-conversion with kind-based operation strings (`src/error.rs:238`)

## Evergreen-reference checklist

When reviewing public API changes or architecture docs, verify against the
evergreen refs listed in `docs/README.md` (which classifies every doc as contract
vs historical snapshot). `docs/archive/` holds the snapshots — never treat one as
a contract:

1. **Public API inventory** (`docs/PUBLIC_API.md`): Every public item is accounted for and justified
2. **Logical layers** (`docs/LOGICAL_LAYERS.md`): No internal types leak through public re-exports. This file is **executable** — `tests/architecture.rs` scans `src/` and fails on a lower layer gaining a higher-layer dependency. Update the doc and the test's constant lists together
3. **Canonical operations**: each behavior-critical operation should have a single documented entry point. The historical analysis is in `docs/archive/CANONICAL_OPERATIONS.md`, but its `path:line` citations are stale — verify the current entry point in `architecture/` and in the source
4. **Dead items**: Previously removed items (`AutoSyncPolicy.max_retries`, `STALE_LOCK_THRESHOLD_SECS`, a *public* `encryption::ct_eq`) stay removed from source and are not re-introduced. Verify no re-introduction — `ct_eq` still exists as a **private** helper (`src/encryption.rs:134`) and must stay private
5. **`#[non_exhaustive]`**: mark new public enums that may gain variants. It is **not** blanket — see the note in `.skills/remediation-patterns.md` for which modules already carry it

## Verification Checklist

1. **Security items**: Verify path canonicalization, TLS `domain_name`, shell `$SHELL`-fallback intent
2. **Core bugs**: Verify atomic saves, deleted flag filtering, error propagation
3. **Clipboard**: Verify generation counter pattern, audit logging, error handling
4. **Config**: Verify keychain error handling, migration atomicity
5. **Sync**: Verify `run_sync()` and `run_premade_sync()` return errors properly
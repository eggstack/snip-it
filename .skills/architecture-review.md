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

| Module | Primary Source Files |
|--------|---------------------|
| overview | `src/main.rs`, project root |
| cli | `src/main.rs`, `src/commands/` |
| clipboard | `src/clipboard.rs` |
| config | `src/config/`, `src/utils/config.rs` |
| core | `src/library/`, `src/error.rs` |
| encryption | `src/encryption.rs` |
| logging | `src/logging.rs` |
| proto | `snip-proto/` |
| server | `snip-sync/src/` |
| sync | `src/sync.rs`, `src/sync_commands.rs` |
| ui | `src/ui/` |
| utils | `src/utils/` |

## Common Patterns to Verify

### Security
- **Path canonicalization**: Output paths and editor paths should be canonicalized before use
- **TLS verification**: `src/sync.rs` sets `.domain_name(host)` on `ClientTlsConfig` — verify it is still set after any TLS touch, don't regress it
- **Shell execution**: `src/commands/run_cmd.rs:111` intentionally uses `$SHELL` with `/bin/sh` fallback (test asserts both accepted) — don't "harden" it to hardcoded `/bin/sh`
- **Atomic file operations**: Use `fs::OpenOptions::create_new(true)` to prevent TOCTOU races

### Error Handling
- **Error propagation**: Functions should return `Result` and propagate errors via `?`
- **SnipError constructors**: Use `SnipError::io_error()`, `toml_error()`, `sync_failure()` etc. (`src/error.rs`); `CryptoError` converts via `impl From<CryptoError> for SnipError` (`src/error.rs:316-330`)
- **Silent failures**: Check for `let _ = ...` patterns that suppress errors without logging

### Sync
- **Deleted snippets**: `deleted: true` snippets should be filtered from TUI display (in `get_snippet_data()`)
- **Conflict resolution**: Live conflicts order by `(updated_at, device_id, SHA-256(synced fields))` — never reintroduce role-dependent `>=` server-wins comparisons; deletion wins over live content regardless of timestamp
- **Push-only counter**: `completed` should increment regardless of `has_failures`

### Known Historical Fixes (verify they're still in place)
- Encryption keys are zeroized after use: encrypt path calls `key.zeroize()` (`src/encryption.rs:243`), decrypt path uses `drop(std::mem::take(&mut key))` (`src/encryption.rs:269`)
- Clipboard debug→warn for auto-clear failures
- Visual mode `y` copies commands (not descriptions) - check `src/ui/mod.rs`
- Premade TOCTOU: read from `canonical_path` not original `path`
- Health RPC verifies database connectivity via `db.ping()`
- `CryptoError` converts to `SnipError::SyncFailure` via the `From` impl (`error.rs`)
- `From<io::Error>` auto-conversion with kind-based operation strings (`error.rs`)

## Phase 06A Checklist

When reviewing public API changes or architecture docs, verify against the evergreen refs (check doc headers — `SECURITY_AUDIT`/`FEATURE_BOUNDARIES` are historical snapshots, not contracts):

1. **Public API inventory** (`docs/PUBLIC_API.md`): Every public item is accounted for and justified
2. **Logical layers** (`docs/LOGICAL_LAYERS.md`): No internal types leak through public re-exports
3. **Canonical operations** (`docs/CANONICAL_OPERATIONS.md`): Each operation has a single, documented entry point
4. **Dead items**: Previously removed items (`AutoSyncPolicy.max_retries`, `STALE_LOCK_THRESHOLD_SECS`, public `encryption::ct_eq`) stay removed from source and are not re-introduced. Verify no re-introduction.
5. **`#[non_exhaustive]`**: All public enums that may gain variants are marked `#[non_exhaustive]`

## Verification Checklist

1. **Security items**: Verify path canonicalization, TLS `domain_name`, shell `$SHELL`-fallback intent
2. **Core bugs**: Verify atomic saves, deleted flag filtering, error propagation
3. **Clipboard**: Verify generation counter pattern, audit logging, error handling
4. **Config**: Verify keychain error handling, migration atomicity
5. **Sync**: Verify `run_sync()` and `run_premade_sync()` return errors properly
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
Output to `plans/<module>_review.md` with:
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
| config | `src/config.rs`, `src/utils/config.rs` |
| core | `src/library.rs`, `src/error.rs` |
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
- **TLS verification**: When using TLS, ensure `domain_name(host)` is set on `ClientTlsConfig`
- **Shell execution**: Prefer hardcoded `/bin/sh` over reading from `$SHELL` env var
- **Atomic file operations**: Use `fs::OpenOptions::create_new(true)` to prevent TOCTOU races

### Error Handling
- **Error propagation**: Functions should return `Result` and propagate errors via `?`
- **From<String> for SnipError**: Enables error conversion from String to SnipError for sync operations
- **Silent failures**: Check for `let _ = ...` patterns that suppress errors without logging

### Sync
- **Deleted snippets**: `deleted: true` snippets should be filtered from TUI display (in `get_snippet_data()`)
- **Timestamp merge**: Server wins on equal timestamps (`>=` not `>`)
- **Push-only counter**: `completed` should increment regardless of `has_failures`

### Known Historical Fixes (verify they're still in place)
- Encryption `drop(key)` now zeroizes via `std::mem::take` before drop (`encryption.rs:229`)
- Clipboard debug→warn for auto-clear failures
- Visual mode `y` copies commands (not descriptions) - check `src/ui/mod.rs`
- Premade TOCTOU: read from `canonical_path` not original `path`
- Health RPC verifies database connectivity via `db.ping()`
- `CryptoError` integrates with `SnipError` via `From` impl (`error.rs:203-210`)
- `From<io::Error>` auto-conversion with kind-based operation strings (`error.rs`)

## Phase 06A Checklist

When reviewing public API changes or architecture docs, verify:

1. **Public API inventory** (`docs/PUBLIC_API.md`): Every public item is accounted for and justified
2. **Logical layers** (`docs/LOGICAL_LAYERS.md`): No internal types leak through public re-exports
3. **Canonical operations** (`docs/CANONICAL_OPERATIONS.md`): Each operation has a single, documented entry point
4. **Dead items** (`docs/OBSOLETE_ITEMS.md`): Removed items (`AutoSyncPolicy.max_retries`, `STALE_LOCK_THRESHOLD_SECS`, `encryption::ct_eq`) are gone from source and not referenced
5. **`#[non_exhaustive]`**: All public enums that may gain variants are marked `#[non_exhaustive]`
6. **Feature boundaries** (`docs/FEATURE_BOUNDARIES.md`): Feature-gated items are correctly gated and documented

## Verification Checklist

1. **Security items** (SEC-1 through SEC-6): Verify path canonicalization, TLS verification, shell hardening
2. **Core bugs** (CORE-1 through CORE-11): Verify atomic saves, deleted flag filtering, error propagation
3. **Clipboard** (CLIP-1 through CLIP-3): Verify generation counter pattern, audit logging, error handling
4. **Config** (CONFIG-1, CONFIG-2, CONFIG-4): Verify keychain error handling, migration atomicity
5. **Sync** (CMD-10, CMD-11): Verify `run_sync()` and `run_premade_sync()` return errors properly
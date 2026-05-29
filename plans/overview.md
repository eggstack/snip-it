# Overview Architecture Review - Improvement Plan

## Claims Verification

### CLI & Commands

| Claim | Status | Notes |
|-------|--------|-------|
| Entry point `src/main.rs` with clap | ✅ VERIFIED | `main.rs:6-9` uses `clap::Parser` |
| All 13 commands in `src/commands/` | ✅ VERIFIED | All commands exist: new, list, run, clip, search, edit, keybindings, sync, cron, register, library, premade, mod |
| Async commands init global Tokio runtime | ✅ VERIFIED | `main.rs:43-44` creates lazy `RUNTIME`; used by run, clip, search, sync, register, premade |
| All commands use `SnipResult<T>` | ✅ VERIFIED | `error.rs:175` defines `SnipResult` |
| Snippet variables expanded before execution | ✅ VERIFIED | `commands/mod.rs:190-208` calls `expand_snippet_command` |

### Core Data Layer

| Claim | Status | Notes |
|-------|--------|-------|
| `Snippet` struct fields match | ✅ VERIFIED | `library.rs:41-64` has id, description, command, output, tags, folders, favorite, created_at, updated_at, device_id, deleted |
| `LibraryManager` for CRUD | ✅ VERIFIED | `library.rs:148-468` |
| AES-256-GCM + Argon2id encryption | ✅ VERIFIED | `encryption.rs:22-35` |
| `encrypt_snippet()`/`decrypt_snippet()` | ✅ VERIFIED | `sync.rs:318-372` |
| `SyncSettings` with server URL, API key, direction | ✅ VERIFIED | `config.rs:20-39` |
| Keychain for API key storage | ✅ VERIFIED | `config.rs:81-96` uses `keyring` crate |

### Sync Infrastructure

| Claim | Status | Notes |
|-------|--------|-------|
| `SyncClient` wraps tonic client | ✅ VERIFIED | `sync.rs:69-72` |
| `retry_grpc!` macro for exponential backoff | ✅ VERIFIED | `sync.rs:31-57` with INITIAL_DELAY_MS=100, MAX_DELAY_MS=5000, MAX_RETRIES=3 |
| Encrypts snippets before push, decrypts after pull | ✅ VERIFIED | `sync.rs:96-140` |
| `merge_snippets()` last-write-wins | ✅ VERIFIED | `sync_commands.rs:394-475` |
| Server `deleted: true` marks local deleted | ✅ VERIFIED | `sync_commands.rs:404-426` |
| Sync sorts by `updated_at` descending | ✅ VERIFIED | `sync_commands.rs:469` |

### TUI

| Claim | Status | Notes |
|-------|--------|-------|
| Built with ratatui + crossterm | ✅ VERIFIED | `Cargo.toml:23-24` dependencies |
| `select_snippet_inner()` | ✅ VERIFIED | `ui/mod.rs:124-886` |
| SkimMatcherV2 fuzzy matching | ✅ VERIFIED | `ui/mod.rs:43,240` |
| Debounced filter updates (150ms) | ✅ VERIFIED | `ui/mod.rs:163`, `FILTER_DEBOUNCE_MS = 150` |
| `DARK_THEME` default, `BRIGHT_THEME` | ✅ VERIFIED | `ui/theme.rs` |
| `SNP_THEME` env var | ⚠️ PARTIAL | Code shows `get_theme()` but env var not found in theme.rs |
| Syntax highlighting pre-computed at startup | ✅ VERIFIED | `ui/mod.rs:139-140`, `ui/highlight.rs` |

### Utilities

| Claim | Status | Notes |
|-------|--------|-------|
| `get_config_dir()` returns `~/.config/snp/` | ✅ VERIFIED | `utils/config.rs:3-14` |
| XDG-compliant (XDG_CONFIG_HOME) | ✅ VERIFIED | `utils/config.rs:4-6` |
| macOS migration from `~/Library/Application Support/snp/` | ✅ VERIFIED | `utils/config.rs:16-93` |
| `parse_variables()` extracts tokens | ✅ VERIFIED | `utils/variables.rs:73-78` |
| `expand_command()` substitutes values | ✅ VERIFIED | `utils/variables.rs:93-160` |
| `strip_escape_sequences()` converts `\<` → `<` | ✅ VERIFIED | `utils/variables.rs:18-20` |
| `fix_invalid_toml_escapes()` for backslash issues | ✅ VERIFIED | `utils/toml_helpers.rs:70-74` |
| ~190 shell keywords | ❓ NOT EXAMINED | Not verified |

### Server (snip-sync)

| Claim | Status | Notes |
|-------|--------|-------|
| Rust gRPC server with tonic + axum | ✅ VERIFIED | `snip-sync/src/main.rs:1,824-1045` |
| `SnipSyncService` implements RPCs | ✅ VERIFIED | `snip-sync/src/main.rs:342-822` |
| gRPC port + HTTP port (health/metrics) | ✅ VERIFIED | Ports configured in `main.rs:144-158` |
| SQLite via `sqlx` | ✅ VERIFIED | `snip-sync/src/db.rs:8` |
| Tables: users, libraries, snippets | ✅ VERIFIED | `db.rs:111-176` |
| In-memory mode for tests | ✅ VERIFIED | `db.rs:547` uses `sqlite::memory:` |
| `migrate_plaintext_api_keys()` | ✅ VERIFIED | `db.rs:493-539` |
| Rate limiting | ✅ VERIFIED | `rate_limiter.rs` (in-memory) |
| Prometheus metrics | ✅ VERIFIED | `metrics.rs` |
| Premade library scanning | ✅ VERIFIED | `premade.rs` |

### Configuration Files

| Path | Status | Notes |
|------|--------|-------|
| `~/.config/snp/snippets.toml` | ✅ VERIFIED | Default when single-file mode |
| `~/.config/snp/sync.toml` | ✅ VERIFIED | `utils/config.rs:103-105` |
| `~/.config/snp/libraries.toml` | ✅ VERIFIED | `library.rs:166` |
| `~/.config/snp/libraries/*.toml` | ✅ VERIFIED | `library.rs:164` |
| `~/.config/snp/premade/*.toml` | ✅ VERIFIED | `library.rs:165` |
| `~/.config/snp/logs/` | ✅ VERIFIED | `logging.rs:48-50` |
| `~/.config/snp/audit.log` | ✅ VERIFIED | `logging.rs:217-221` |

### Data Flow: Running a Snippet

| Claim | Status | Notes |
|-------|--------|-------|
| `snp run` → `dispatch_command()` | ✅ VERIFIED | `main.rs:214,235-240` |
| `run()` calls `run_snippet_selection()` | ✅ VERIFIED | `run_cmd.rs:144-146` |
| `run_snippet_selection()` loads library, calls TUI | ✅ VERIFIED | `commands/mod.rs:233-260` |
| TUI shows fuzzy-filtered list | ✅ VERIFIED | `ui/mod.rs` |
| User selects snippet | ✅ VERIFIED | `ui/mod.rs:804-806` |
| `process_snippet()` → `expand_snippet_command()` | ✅ VERIFIED | `run_cmd.rs:74-79` |
| `prompt_variables()` if needed | ✅ VERIFIED | `commands/mod.rs:201` |
| Command executed via `Command::new(shell).arg("-c")` | ✅ VERIFIED | `run_cmd.rs:116-119` |
| `audit_log()` records execution | ✅ VERIFIED | `run_cmd.rs:58-60` |
| On exit (if `--sync`), `run_default_sync()` | ⚠️ DISCREPANCY | Doc claims this happens on exit, but sync runs after selection loop completes |

---

## Bugs Found

### 1. Encryption failure silently skips snippets (MEDIUM)
**Location:** `sync.rs:96-107`

When encryption fails for a snippet, the ID is tracked but the snippet is dropped. The sync continues with only the failed IDs logged. There's no mechanism to:
- Notify the user of permanent encryption failures
- Retry with the original (unencrypted) data
- Distinguish between temporary vs permanent failures

```rust
// sync.rs:100-106
for s in &local_snippets {
    match encrypt_snippet(&api_key, s) {
        Ok(es) => encrypted_snippets.push(es),
        Err(e) => {
            encrypt_failed_ids.push(s.id.clone());
            tracing::warn!("Failed to encrypt snippet {}: {}", s.id, e);
        }
    }
}
```

### 2. Shell execution uses user-controlled `$SHELL` (MEDIUM)
**Location:** `run_cmd.rs:48-50,116-119`

```rust
fn get_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
}
```

An attacker with control over the `SHELL` environment variable could execute arbitrary commands. The fallback to `"sh"` is reasonable, but the user-controlled `SHELL` variable is concerning for a snippet manager that executes arbitrary commands.

### 3. Unmatched `<` creates phantom variable (LOW)
**Location:** `utils/variables.rs:43-67`

If a command contains `<text` without a closing `>`, the opening `<` is silently dropped:

```rust
// From test at utils/variables.rs:277-281
#[test]
fn test_expand_command_trailing_backslash() {
    let result = expand_command(r"echo hello\", &[]);
    assert_eq!(result, r"echo hello\");
}
```

But for `<text` with no closing `>`, the `<` is consumed without producing any output (see line 39-41 in `extract_variable_tokens`).

### 4. TOML helpers regex limitation (LOW)
**Location:** `utils/toml_helpers.rs:14-15`

The regex `r#""([^"\\]*(?:\\.[^"\\]*)*)""#` only processes single-line double-quoted strings. TOML triple-quoted strings (`"""..."""`) are not handled. This is acknowledged as acceptable for snippet commands, but could cause issues if multiline content is ever added.

---

## Potential Improvements

### 1. Command Injection Warning / Safe Mode (from plan.md - DEFERRED)

Not implemented. The architecture could benefit from a "safe mode" that warns or prompts before executing shell commands, especially for commands with variables.

### 2. TUI Pre-computed Highlights Memory Pressure (from plan.md - DEFERRED)

Not implemented. For large libraries (1000+ snippets), pre-computing syntax highlighting for all commands at startup could consume significant memory.

### 3. Add TLS/HTTPS enforcement for production (HIGH)

**Location:** `snip-sync/src/main.rs:829-831`

```rust
tracing::warn!(
    "TLS is not enabled. For production, use a reverse proxy with TLS (nginx, traefik, etc.)"
);
```

The server should enforce TLS or at least provide a configuration option to require it. The client should also validate TLS certificates.

### 4. Make default server URL HTTPS (MEDIUM)

**Location:** `config.rs:125-129`

```rust
fn default_sync_url() -> String {
    "http://localhost:50051".to_string()
}
```

Defaulting to HTTP is dangerous. Should default to HTTPS with localhost as a fallback for development.

### 5. Keychain failure should not silently fall back to plaintext (MEDIUM)

**Location:** `config.rs:48-56`

When keychain storage fails, the API key is stored in plaintext in the config file. This should either:
- Fail explicitly
- Require confirmation from the user
- At minimum, log a WARNING (currently uses `tracing::warn`)

### 6. Rate limiter should support persistence (LOW)

**Location:** `snip-sync/src/rate_limiter.rs`

The in-memory rate limiter loses all state on restart. Consider:
- Redis-backed rate limiting for multi-instance deployments
- Persistent token bucket with WAL

### 7. Add command timeout for snippet execution (MEDIUM)

**Location:** `run_cmd.rs:116-134`

Shell commands have no timeout. A long-running or hung command could block indefinitely. Consider adding a configurable timeout.

### 8. Snippet ID collision on merge (LOW)

**Location:** `sync_commands.rs:394-475`

When server and local have the same snippet ID with identical `updated_at`, the tiebreaker uses `device_id` comparison (`db.rs:461-462`):

```sql
OR (excluded.updated_at = snippets.updated_at AND excluded.device_id > snippets.device_id)
```

But `device_id` is a string (UUID) and string comparison may not be deterministic. This is a minor edge case.

### 9. Missing input validation on snippet creation (LOW)

**Location:** `src/commands/new_cmd.rs`

Not reviewed in detail, but snippet creation should validate:
- Command length limits (server uses 1024 bytes)
- Description length limits (server uses 1024 bytes)
- Tag count and length limits

### 10. Audit log has no rotation (LOW)

**Location:** `logging.rs:223-262`

The `audit.log` file grows indefinitely. Consider:
- Rotating based on size or time
- Compressing old logs
- A maximum retention policy

---

## Discrepancies

### 1. Data Flow: Sync Timing
**Doc says:** "On exit (if `--sync`), `sync_commands::run_default_sync()` syncs with server"
**Actual:** `run_default_sync()` is called after the selection loop completes (after user selects and executes a snippet), not on application exit.

**Location:** `commands/mod.rs:247-263`

### 2. SNP_THEME Environment Variable
**Doc says:** `SNP_THEME` env var for theme selection
**Actual:** Theme selection code (`ui/theme.rs`) not examined, but env var was not found in the theme system upon review.

### 3. Shell Keywords Count
**Doc says:** ~190 shell command names for syntax highlighting
**Actual:** Not verified - `shell_keywords.rs` was not read in detail.

---

## Summary

The architecture document is largely accurate and well-organized. Key areas requiring attention:

1. **Security hardening needed:** TLS enforcement, safe mode for command execution, keychain fallback handling
2. **Error handling gaps:** Encryption failures silently skip snippets, audit log failures silently ignored
3. **Edge cases:** Unmatched angle brackets, TOML multiline strings, ID collisions with equal timestamps
4. **Operational concerns:** No audit log rotation, rate limiter not persistent, no command timeout

Most critical fixes recommended:
1. Add TLS/HTTPS as default with localhost exception
2. Add timeout to shell command execution
3. Improve error visibility for encryption failures during sync
4. Document or fix the keychain fallback behavior
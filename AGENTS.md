# AGENTS.md

## Build & Test Commands

```bash
# Build release binary
cargo build --release

# Run all tests (library, integration, server)
cargo test

# Run only unit tests
cargo test --lib

# Run only integration tests
cargo test --test integration

# Run only server (snip-sync) tests
cargo test -p snip-sync

# Lint with clippy
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --check

# Auto-format
cargo fmt
```

## Project Structure

```
snip-it/
├── Cargo.toml          # Main crate: binary "snp" (Rust 1.81+)
├── src/
│   ├── main.rs         # CLI entry point, clap command dispatch
│   ├── clipboard.rs    # Cross-platform clipboard (copypasta / clipboard-win)
│   ├── config.rs       # Sync settings (SyncSettings, SyncDirection)
│   ├── encryption.rs   # AES-256-GCM + Argon2id key derivation
│   ├── error.rs        # SnipError enum, SnipResult type alias
│   ├── library.rs      # Snippet/Snippets structs, LibraryManager
│   ├── logging.rs      # Tracing-based logging, audit log
│   ├── sync.rs         # gRPC client for snip-sync server
│   ├── sync_commands.rs# Sync orchestration, merge logic
│   ├── ui/              # TUI (ratatui), fuzzy search, themes
│   │   ├── mod.rs       # Main TUI loop, re-exports
│   │   ├── theme.rs     # Theme system, dark/bright themes
│   │   ├── highlight.rs # Syntax highlighting for commands
│   │   └── variables.rs # Variable prompting UI
│   ├── commands/       # One module per CLI subcommand
│   │   ├── mod.rs      # Shared helpers: expand_snippet_command, get_library_path
│   │   ├── run_cmd.rs  # Snippet execution via shell
│   │   ├── clip_cmd.rs # Copy to clipboard
│   │   ├── search_cmd.rs
│   │   ├── new_cmd.rs
│   │   ├── list_cmd.rs
│   │   ├── edit_cmd.rs
│   │   ├── sync_cmd.rs
│   │   ├── register_cmd.rs
│   │   ├── library_cmd.rs
│   │   ├── premade_cmd.rs
│   │   ├── cron_cmd.rs
│   │   └── keybindings_cmd.rs
│   └── utils/
│       ├── mod.rs
│       ├── config.rs       # get_config_dir, get_snippets_path, macOS migration
│       ├── variables.rs    # Variable parsing/expansion (<name=default>)
│       ├── toml_helpers.rs # TOML backslash escape handling
│       └── shell_keywords.rs
├── snip-proto/         # Protobuf definitions, tonic-generated gRPC code
│   ├── build.rs
│   ├── src/lib.rs
│   └── src/snip_proto.rs
├── snip-sync/          # Sync server (gRPC + HTTP/axum)
│   ├── src/main.rs     # Server entry, SnipSyncService impl, axum health/metrics
│   ├── src/db.rs       # SQLite (sqlx) — users, libraries, snippets tables
│   ├── src/rate_limiter.rs
│   ├── src/metrics.rs  # Prometheus metrics
│   └── src/premade.rs  # Premade library file scanning
├── tests/
│   └── integration.rs  # CLI integration tests using TempDir
├── plan.md             # Remediation plan for code review findings
└── AGENTS.md           # This file
```

## Key Patterns

### Error Handling
- All errors use `SnipError` enum (`src/error.rs`)
- Constructor helpers: `SnipError::io_error()`, `SnipError::toml_error()`, etc.
- Return type: `SnipResult<T> = Result<T, SnipError>`
- IO errors auto-convert via `From<io::Error>`

### Async (Tokio)
- A global `RUNTIME: LazyLock<Runtime>` is created lazily on first access
- Only async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`) trigger initialization
- Sync operations use `runtime.block_on()` to call async gRPC client methods

### TOML Handling
- TOML backslashes (`\<`, `\>`) are problematic in double-quoted strings
- `fix_invalid_toml_escapes()` converts double-quoted to single-quoted (raw literals)
- `quote_strings_containing_backslashes()` does the reverse on save
- Both live in `src/utils/toml_helpers.rs`
- **Important:** These only handle single-line strings. Triple-quoted (`"""`) TOML strings are not processed (acceptable since snippet commands are single-line)

### Snippet Variables
- Syntax: `<name>` or `<name=default>` in command strings
- `\<` and `\>` are literal angle brackets (escape sequences)
- Parsed by `utils/variables.rs::parse_variables()` and `extract_variable_tokens()`
- Expanded by `utils/variables.rs::expand_command()`
- UI prompt in `ui/variables.rs::prompt_variables_inner()`
- **Known edge case:** Unmatched `<` without `>` creates phantom variable and drops the `<` character

### TUI Architecture
- Single-loop event-driven TUI in `ui/mod.rs::select_snippet_inner()`
- Syntax highlighting is pre-computed once at startup (not in draw loop)
- Fuzzy matching via `fuzzy-matcher` (skim algorithm)
- Debounced filter updates (150ms)
- Theme: dark (default) or bright, controlled by `SNP_THEME` env var

### Sync Merge Strategy
- Last-write-wins based on `updated_at` timestamp
- Server `deleted: true` snippets are excluded from merge (destructive — see plan.md #3)
- Local-only fields (`output`, `folders`, `favorite`) are preserved when server wins
- Snippets are sorted by `updated_at` descending after merge

### Database (snip-sync)
- SQLite via `sqlx` with in-memory support for tests
- Tables: `users`, `libraries`, `snippets`
- API keys hashed with Argon2id
- Schema created inline in `Database::connect()` (no migration framework)
- `migrate_plaintext_api_keys()` backfills hashes for legacy data

## Configuration Files

- `~/.config/snp/snippets.toml` — Main snippet storage (or per-library in `libraries/`)
- `~/.config/snp/sync.toml` — Sync settings (server URL, API key, direction)
- `~/.config/snp/libraries.toml` — Library metadata
- `~/.config/snp/libraries/*.toml` — Individual library files
- `~/.config/snp/premade/*.toml` — Downloaded premade libraries
- `~/.config/snp/logs/` — Rolling log files (daily rotation)
- `~/.config/snp/audit.log` — Audit log for snippet operations

## Deferred Items

Optional items remaining in `plan.md` (see plan.md for full details):
- **Command injection warning** (safe mode for snippet execution)
- **TUI pre-computed highlights memory pressure** (lazy computation for large libraries)

## Architecture Review Summary

During the 2026-05-29 architecture review, 8 bugs were fixed and 80+ items were identified for future improvement. See `plan.md` for the full consolidated remediation plan organized into 4 implementation waves:

- **WAVE 1 (Security-Critical):** 6 items - SEC-1, SEC-2, SEC-3, SEC-5, SEC-6, CLI-1 - **COMPLETED**
- **WAVE 2 (Core Bugs):** 17 items - CORE-1 through CORE-11, CLIP-1 through CLIP-3, CONFIG-1, CONFIG-2, CONFIG-4 - **COMPLETED**
- **WAVE 3 (Improvements):** 24 items - SEC-7, SEC-8, SEC-9, CMD-3, CMD-10, CMD-11, LIB-1 through LIB-6, LOG-2, LOG-3, LOG-5, LOG-6, LOG-7, SERVER-3, SERVER-4, SERVER-5, SERVER-6, SERVER-8, PROTO-1, PROTO-2, TUI-1 - **COMPLETED**
- **WAVE 4 (Low Priority):** 40+ items - all TUI, CMD, CONFIG, LOG, LIB, SYNC, OV items - **DEFERRED**

Each wave can be implemented in parallel by separate agents. Within a wave, items are organized by module (UI, Commands, Library, Config, etc.) to minimize context switching.

## Implementation Notes (2026-05-30)

The following bugs were fixed during architecture review and subsequent work. For detailed fixes, see `plan.md` "Completed in Prior Work" section.

### High Priority Fixes (Historical)
1. **Encryption ineffective `drop(key)`** (`src/encryption.rs:176,195`): Removed no-op `drop(key)` calls after key was already moved into cipher
2. **Clipboard debug→warn** (`src/clipboard.rs:37`): Changed `tracing::debug` to `tracing::warn` for auto-clear failures
3. **Clipboard redundant drop** (`src/clipboard.rs:42`): Removed redundant `std::mem::drop(handle)` - thread continues regardless
4. **TUI visual mode copy bug** (`src/ui/mod.rs:672`): Visual mode `y` now copies commands (not descriptions) to match single-select behavior
5. **Sync merge equal timestamps** (`src/sync_commands.rs:429`): Changed `>` to `>=` so server wins on equal timestamps
6. **Push-only counter bug** (`src/sync_commands.rs:306-323`): `completed` now increments regardless of `has_failures`
7. **Premade TOCTOU** (`snip-sync/src/premade.rs:199`): Now reads from `canonical_path` instead of original `path`
8. **Health check DB ping** (`snip-sync/src/main.rs:343-352`): Health RPC now verifies database connectivity via `db.ping()`

### Additional Fixes (2026-05-30)
9. **CORE-2: deleted flag not filtered in TUI** (`src/commands/mod.rs:159-184`): `get_snippet_data()` now filters out `deleted: true` snippets
10. **CMD-10: sync error propagation** (`src/commands/sync_cmd.rs:185-192`): `run_sync()` now returns `Result<(), String>` and errors propagate to caller
11. **CMD-11: premade sync return value** (`src/commands/premade_cmd.rs:144-153`): `run_premade_sync()` now returns error on failure instead of always `Ok(())`
12. **Added `From<String>` for SnipError** (`src/error.rs`): Enables error conversion from String to SnipError for sync operations

### Known Scope Constraints
- `output` field not encrypted during sync (proto definition lacks field - cannot add without breaking change)
- `\<` escape inconsistency in variables.rs is a documented known edge case per AGENTS.md
- Many CLI documentation discrepancies (e.g., `--clip` behavior, cron intervals) are doc bugs not code bugs

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation

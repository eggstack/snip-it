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
│   ├── ui.rs           # TUI (ratatui), fuzzy search, themes
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
- UI prompt in `ui.rs::prompt_variables_inner()`
- **Known edge case:** Unmatched `<` without `>` creates phantom variable and drops the `<` character

### TUI Architecture
- Single-loop event-driven TUI in `ui.rs::select_snippet_inner()`
- Syntax highlighting is pre-computed once at startup (not in draw loop)
- Fuzzy matching via `fuzzy-matcher` (skim algorithm)
- Debounced filter updates (150ms)
- Theme: dark (default) or bright, controlled by `SNP_THEME` env var
- **Note:** `ACTIVE_THEME` uses `Mutex<Theme>` but `Theme` is `Copy` — `LazyLock` would be simpler

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

## Remediation Plan

The consolidated remediation plan is in `plan.md`. It contains all items from the architecture reviews in `plans/`, organized into parallel implementation waves.

**Current status:** Plan generated and verified. Implementation not yet started.

## Known Issues (Quick Reference)

For full details and fix instructions, see `plan.md`.

| Severity | Issue | Location |
|----------|-------|----------|
| P0 | Argon2 memory cost 64 KiB (OWASP min: 19 MiB) | `encryption.rs:32`, `db.rs:12` |
| P0 | API key stored as plaintext in `sync.toml` | `config.rs:19` |
| P0 | CORS `CORS_ALLOW_ALL` env var referenced but never read | `snip-sync/src/main.rs` |
| P0 | Registration rate limit bypassable via client-controlled `device_id` | `snip-sync/src/main.rs:333` |
| P1 | Sync fall-through: `list_libraries` failure doesn't stop sync | `sync_cmd.rs:224-251` |
| P1 | Encryption failures cause permanent snippet loss (last_sync advances) | `sync.rs:96-107`, `sync_commands.rs:300` |
| P1 | `set_primary()` silently succeeds on nonexistent filename | `library.rs:346-352` |
| P1 | `add_server_library()` creates duplicate metadata entries | `library.rs:387-413` |
| P1 | `load_snippets()` returns empty on parse error (data loss risk) | `commands/mod.rs:102-141` |
| P1 | Clipboard auto-clear race condition clears new content | `clipboard.rs:30-42` |
| P1 | `shutdown_logging` logs after dropping file writer guard | `logging.rs:93-98` |
| P2 | `ui.rs` is 1416 lines — should be split into submodules | `src/ui.rs` |

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation

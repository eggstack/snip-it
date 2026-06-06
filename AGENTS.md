# AGENTS.md

## Build & Test Commands

```bash
# Build release binary
cargo build --release

# Run all tests (unit + integration)
cargo test

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

# Code coverage (requires cargo-llvm-cov)
cargo llvm-cov --workspace --html
```

**Note:** `cargo test --lib` does not work because this is a binary-only crate.
To run unit tests without integration tests, the project would need to be restructured
as a workspace with a library crate.

## Project Structure

```
snip-it/
├── Cargo.toml          # Main crate: binary "snp" (Rust 1.88+)
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
│   │   ├── theme.rs     # Theme system, dark/bright themes, Halloy TOML parsing, ThemeManager
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
- Theme picker: press `e` in normal mode to open; `j`/`k` to preview live, `i` to filter, `Enter` to commit, `e`/`q`/`Esc` to cancel. INS sub-mode mirrors the snippet browser INS UX.
- Theme: Halloy-compatible TOML at `~/.config/snp/themes/<name>.toml`; active theme persisted in `~/.config/snp/themes.toml`. `SNP_THEME` env var still works for backward compat.

### Bundled Themes
- 50 Halloy themes live in `themes/` and are LZMA-compressed and base64-encoded at build time by `scripts/build_themes.py` into `src/ui/_generated_bundled_themes.rs`.
- `build.rs` re-invokes the script when the source themes directory is newer than the generated file.
- The default theme (`Cyber Red`) is hardcoded in the binary via `include_str!` as a fallback if `themes.toml` is missing.
- Decoding uses the pure-Rust `lzma-rs` crate (no C toolchain).

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

## Design Decisions

### No Command Filtering (by design)
- Snippet commands are executed as-is via the user's shell — no sanitization, filtering, or guardrails.
- This is intentional: the tool targets power users who explicitly do not want safety restrictions.
- Any "safe mode" or metacharacter filtering is explicitly rejected as a design decision.
- Users are responsible for the commands they store and execute.

## Deferred Items

- **TUI pre-computed highlights memory pressure** (lazy computation for large libraries)

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation
- Utils tests cover escape sequences, nested brackets, chained backslashes
- Sync tests cover device conflict detection
- Total: 168 unit tests + 20 integration tests + 30 server tests = 218 tests passing

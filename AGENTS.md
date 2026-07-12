# AGENTS.md

## Build & Test Commands

```bash
# Build release binary
cargo build --release

# Build the entire workspace (snip-it, snip-proto, snip-sync)
cargo build --workspace

# Run all tests across the workspace (unit + integration + server)
cargo test --workspace

# Run only the main snp crate's tests
cargo test -p snip-it

# Run only CLI integration tests
cargo test --test integration

# Run only sync integration tests (async, needs test-helpers feature)
cargo test --test sync_integration

# Run PTY end-to-end tests (must run single-threaded, needs portable-pty)
cargo test --test pty_integration -- --test-threads=1

# Run only server (snip-sync) tests
cargo test -p snip-sync

# Run only snip-proto tests
cargo test -p snip-proto

# Lint with clippy (across the workspace)
cargo clippy --workspace --all-targets -- -D warnings

# Format check (all crates)
cargo fmt --all -- --check

# Auto-format
cargo fmt

# Code coverage (requires cargo-llvm-cov)
cargo llvm-cov --workspace --html
```

**Note:** The main `snip-it` crate is still a binary-only crate, so `cargo test --lib -p snip-it`
does not work. Use `cargo test --workspace` (or `cargo test -p snip-it` for the binary's
unit + integration tests) instead. The `snip-proto` and `snip-sync` crates are proper
library / binary crates and can be tested individually with `-p`.

## Project Structure

```
snip-it/
├── Cargo.toml          # Main crate: binary "snp" (Rust 1.94+)
├── build.rs            # Re-invokes build_themes.py when themes/ changes
├── src/
│   ├── main.rs         # CLI entry point, clap command dispatch
│   ├── lib.rs          # Library re-exports for integration tests
│   ├── proto.rs        # Proto wrapper (re-exports snip_proto types)
│   ├── clipboard.rs    # Cross-platform clipboard (arboard / clipboard-win)
│   ├── config.rs       # Sync settings (SyncSettings, SyncDirection)
│   ├── encryption.rs   # AES-256-GCM + Argon2id key derivation
│   ├── error.rs        # SnipError enum, SnipResult type alias
│   ├── library.rs      # Snippet/Snippets structs, LibraryManager
│   ├── logging.rs      # Tracing-based logging, audit log
│   ├── sync.rs         # gRPC client for snip-sync server
│   ├── sync_commands.rs# Sync orchestration, merge logic
│   ├── ui/              # TUI (ratatui), fuzzy search, themes
│   │   ├── mod.rs       # Main TUI loop, re-exports
│   │   ├── state.rs     # SelectState, FilterState, SortMode, is_ctrl_key
│   │   ├── theme.rs     # Theme system, Halloy TOML parsing, ThemeManager, bundled themes
│   │   ├── highlight.rs # Syntax highlighting for commands
│   │   ├── variables.rs # Variable prompting UI
│   │   └── _generated_bundled_themes.rs # LZMA-compressed bundled themes (build-time)
│   ├── commands/       # One module per CLI subcommand
│   │   ├── mod.rs      # Shared helpers: expand_snippet_command, get_library_path
│   │   ├── run_cmd.rs  # Snippet execution via shell
│   │   ├── clip_cmd.rs # Copy to clipboard
│   │   ├── search_cmd.rs
│   │   ├── select_cmd.rs # Non-executing selection to stdout (pet compat)
│   │   ├── shell_cmd.rs  # Shell integration generation (snp shell init)
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
│       ├── shell_keywords.rs # ~190 shell command names for highlighting
│       └── tempfile_guard.rs # RAII temp file cleanup
├── snip-proto/         # Protobuf definitions, tonic-generated gRPC code
│   ├── build.rs        # Generates src/snip_proto.rs from proto/sync.proto (needs protoc only for regeneration)
│   ├── src/lib.rs
│   └── src/snip_proto.rs
├── snip-sync/          # Sync server (gRPC + HTTP/axum)
│   ├── src/main.rs     # Server entry, SnipSyncService impl, axum health/metrics
│   ├── src/lib.rs      # Service impl, config, constants (test-helpers feature)
│   ├── src/db.rs       # SQLite (sqlx) — users, libraries, snippets tables
│   ├── src/rate_limiter.rs
│   ├── src/metrics.rs  # Prometheus metrics
│   ├── src/premade.rs  # Premade library file scanning
│   ├── src/paths.rs    # Platform path helpers (config, data, state, cert, pid)
│   ├── src/bootstrap.rs # First-run layout and config creation
│   ├── src/cli.rs      # Clap CLI definitions (Command enum)
│   ├── src/cert.rs     # Dev certificate generation (via openssl)
│   ├── src/editor.rs   # Editor resolution ($EDITOR, PATH search)
│   └── src/process.rs  # PID file management and process lifecycle
├── tests/
│   ├── integration.rs      # CLI integration tests using TempDir
│   ├── pty_integration.rs  # PTY end-to-end tests (portable-pty, --test-threads=1)
│   └── sync_integration.rs # gRPC sync integration tests (real server in-process)
├── scripts/
│   └── build_themes.py # LZMA-compresses themes/ into src/ui/_generated_bundled_themes.rs
├── themes/             # 50 Halloy TOML theme files (source of truth for bundled themes)
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
- **Edge case:** Unmatched `<` without a matching `>` is treated as a literal `<` in the output (no variable substitution, character preserved).

### TUI Architecture
- Single-loop event-driven TUI in `ui/mod.rs::select_snippet_inner()`
- State types in `ui/state.rs`: `SelectState`, `FilterState`, `SortMode`
- Syntax highlighting is pre-computed once at startup (not in draw loop)
- Fuzzy matching via `fuzzy-matcher` (skim algorithm)
- Debounced filter updates (150ms)
- Theme picker: press `e` in normal mode to open; `j`/`k` to preview live, `i` to filter, `Enter` to commit, `e`/`q`/`Esc` to cancel. INS sub-mode mirrors the snippet browser INS UX.
- Theme: Halloy-compatible TOML at `~/.config/snp/themes/<name>.toml`; active theme persisted in `~/.config/snp/themes.toml`. `SNP_THEME` env var still works for backward compat.

### Selection Outcome Architecture
- **Three-layer outcome model:**
  1. `SnippetSelection` (ui/mod.rs): `Selected(usize, Option<String>)`, `Delete(usize)`, `Cancelled`
  2. `SelectionOutcome` (lib.rs): `Selected` or `Cancelled` — returned by `run_snippet_selection()`
  3. `CommandOutcome` (lib.rs): `Success` or `Cancelled` — returned by command `run()` functions, mapped to exit codes in `main.rs`
- **Cancellation flow:** TUI returns `SnippetSelection::Cancelled` → `run_snippet_selection()` returns `SelectionOutcome::Cancelled` → `select_cmd` maps to `CommandOutcome::Cancelled` → exit code 4
- **Conservative callers:** `run_cmd`, `clip_cmd`, `search_cmd` ignore `SelectionOutcome` (treat cancellation as normal completion, exit 0)
- **Ctrl+C:** Handled same as `q`/`Esc` in normal mode (sets `sel.selected = filtered.len()` → returns `Cancelled`). SIGINT signal path also returns `Cancelled` via TERMINATE atomic flag.
- **Variable prompt cancel:** Also maps to `SelectionOutcome::Cancelled` → exit 4 for `select`

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

### snip-sync CLI
- Binary defaults to `serve` when no subcommand given (backward compatible)
- `CONFIG_PATH` env var overrides platform config dir
- PID file written at `state_dir()/snip-sync.pid`, cleaned on shutdown
- `croncheck` spawns detached child process; uses lock file to prevent races
- Cert generation shells out to `openssl` (not a Rust crypto crate)

### Shell Integration (`snp shell init`)
- `src/commands/shell_cmd.rs` generates Bash, Zsh, and Fish integration code
- CLI: `snp shell init <bash|zsh|fish>` prints generated code to stdout
- Two public functions per shell: `snp_select_raw` and `snp_select_expanded`
- Shell functions call `snp select --query <buffer> --raw/--expanded --output-file <tmpfile>`
- Temp-file transport for lossless multiline handling (avoids `$(...)` trailing-newline stripping)
- `--query` is an alias for `--filter` on the `select` command (pre-fills TUI search)
- `snp select` returns `CommandOutcome` (Success/Cancelled); `Cancelled` maps to exit 4 at the CLI boundary in `main.rs`
- Shell adapters check exit status before file emptiness, and propagate nonzero exit codes from operational failures
- Cancellation (exit code 4): original buffer restored, temp file cleaned up
- `--output-file` rejects symlinks and directories
- No keybindings installed by default; generated code includes binding examples in comments
- No `eval` on selected content; `eval` only for sourcing trusted generated init code
- `command -v snp` at widget invocation time (not at source time) for graceful degradation

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
- `snip-sync` has a `test-helpers` feature gate for in-process server testing; `snip-it`'s dev-dependencies enable it automatically
- `tests/sync_integration.rs` spins up a real `snip-sync` server in-process via `test_helpers` — these are async `#[tokio::test]` and need the `test-helpers` feature
- PTY tests (`tests/pty_integration.rs`) use `portable-pty` crate and must run with `--test-threads=1` — they create real PTY pairs and inject keystrokes via raw fd writes
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation
- Utils tests cover escape sequences, nested brackets, chained backslashes
- Sync tests cover device conflict detection
- snip-sync has 78 tests (unit + integration)

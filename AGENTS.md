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

# Release 2A command ingestion and shell-helper tests
cargo test --test integration command_stdin
cargo test --lib new_cmd
cargo test --lib bash_new

# Release 2B file and editor creation tests
cargo test --test integration from_file
cargo test --test integration editor
cargo test --lib new_cmd

# Release 2C golden corpus and shell init tests
cargo test --test integration golden_corpus
cargo test --test integration cross_source
cargo test --test integration shell_init

# Release 2 closure pass: editor tempfile + cross-source tests
cargo test --lib new_cmd
cargo test --lib shell_cmd
cargo test --test integration -- new_editor new_from_file command_stdin golden_corpus multiline

# Release 3B pet import tests
cargo test --test integration import_pet
cargo test --lib import

# Release 3C doctor tests
cargo test --test integration doctor

# Release 3C doctor and diagnostics tests
cargo test -p snip-it -- diagnostics
cargo test --test integration doctor

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
│   ├── auto_sync.rs    # Auto-sync coordinator, debounce, durable pending, PID-file locking
│   ├── config.rs       # Sync settings (SyncSettings, SyncDirection)
│   ├── encryption.rs   # AES-256-GCM + Argon2id key derivation
│   ├── error.rs        # SnipError enum, SnipResult type alias
│   ├── diagnostics.rs  # Shared diagnostic model (SourceSpan, CompatibilityDiagnostic, DoctorReport, PetImportReport)
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
│   ├── sort.rs         # Shared sort/rank model (SnippetSort enum, rank_snippets)
│   ├── output.rs       # Output/notes presentation model, sanitization, preview helpers
│   ├── usage.rs        # Local-only usage metadata (UsageIndex, usage.toml)
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
│   │   ├── import_cmd.rs    # Pet import (snp import pet <path>)
│   │   ├── doctor_cmd.rs     # Compatibility diagnostics (snp doctor)
│   │   ├── pet_analysis.rs  # Shared pet TOML analysis (doctor + import)
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
- **Save path:** snip-it writes `toml::to_string_pretty` output directly without post-processing. The earlier `quote_strings_containing_backslashes` post-processing pass was removed because it silently corrupted tabs, trailing whitespace, and CRLF: its regex could not distinguish triple-quoted multi-line strings from ordinary double-quoted strings, and its single-quoted output preserved TOML escape sequences like `\t` as literal two-character pairs. The helper is retained as a public utility for callers that hand-write TOML.
- **Corpus constraint:** The golden command corpus includes tabs, trailing spaces, and CRLF line endings. These byte sequences survive the full save/load pipeline (`toml::to_string_pretty` + `load_library` via `fix_invalid_toml_escapes` + `toml::from_str`) and are tested through all acquisition sources (stdin, file, editor, positional).

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

### Sorting and Ranking (Release 4A)
- `SnippetSort` enum in `src/sort.rs`: Relevance (default), Recent, LastUsed, MostUsed, Description, Command
- `rank_snippets()` provides deterministic, stable sorting with tie-break chain:
  1. Primary key (sort mode)
  2. Favorites-first grouping (orthogonal modifier)
  3. Fuzzy relevance (for Relevance mode)
  4. Normalized description
  5. Original index (stable)
- CLI flags `--sort <mode>` and `--favorites-first` on: run, clip, search, select, list
- TUI interactive sort keybinds (n/o/a/z) still work; CLI --sort sets the initial mode
- `--sort` affects list JSON/CSV output ordering
- Usage tracking via `src/usage.rs`: `UsageIndex` persists to `~/.config/snp/usage.toml`
- Usage recorded on successful `run` and `clip` operations
- Usage data is local-only, not synchronized
- Missing/corrupt usage data fails open to zero usage
- TUI and list surfaces share real usage metadata for `last-used` and `most-used` sort modes
- `UsageIndex` is loaded once per selection session and passed to the TUI via `SnippetListParams`
- Default relevance tie behavior is compatibility-first: usage data has no effect unless `--sort last-used` or `--sort most-used` is explicitly selected

### Output / Notes Presentation (Release 4B)
- `OutputPresentation` in `src/output.rs` provides safe rendering of the `output` field
- `sanitize_for_terminal()` strips ANSI/OSC sequences without mutating stored values
- `summary(max_chars)` returns a single-line truncated preview; `display()` returns full sanitized content
- `display_bounded(max_lines)` truncates multiline output with a line-count note
- `for_scoring()` returns a bounded substring for fuzzy search (512 char budget)
- Default `list` output hides empty output fields; `--search-output` includes output in fuzzy matching
- TUI preview shows output below command with `--- Output / Notes ---` separator
- `snp edit --output`, `--output-stdin`, `--clear-output` for structured output editing
- `--filter` is required when using output edit flags; matches by description or command substring
- JSON and CSV output always include the `output` field exactly as stored
- `select`, `run`, and `clip` emit/command act on `command` only, never `output`
- Output is stored descriptive metadata, not automatically captured execution output
- **Output sync contract**: `output` is local-only — not in `ProtoSnippet`, not uploaded or downloaded. Sync merge preserves the local value when remote data wins.

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

### Auto-Sync Policy (Release 5A)

- `AutoSyncPolicy` struct in `src/auto_sync.rs` — effective policy resolved once per command invocation
- `AutoSyncFailureMode` enum in `src/config.rs` — Ignore, Warn (default), Error
- `MutationKind` enum in `src/auto_sync.rs` — classifies mutations for sync triggers
- Configuration in `sync.toml`: `auto_sync`, `auto_sync_debounce_seconds`, `auto_sync_failure`
- CLI: `snp sync config --show| --auto-sync on|off | --debounce <secs> | --failure ignore|warn|error`
- Auto-sync is disabled by default; local mutations always commit before remote work begins
- Remote failure never rolls back local state
- Existing manual `snp sync`, `snp cron`, and daemon workflows are unchanged
- `MutationKind::AccountConfig` never triggers sync; all other kinds are syncable when enabled
- Debounce range: 0-300 seconds (clamped); default: 2 seconds
- `error` failure mode sets nonzero exit code but local mutation remains committed

### Auto-Sync Coordinator (Release 5B)

- `AutoSyncCoordinator` struct in `src/auto_sync.rs` — debounce engine, durable pending markers, PID-file locking
- `AutoSyncRequest` — contains `library_id`, `mutation_kind`, `requested_at`; no snippet content or secrets
- `MutationOrigin` enum — `User`, `Import`, `SyncMerge`, `Recovery`; `SyncMerge` never triggers auto-sync
- `AutoSyncStatus` enum — `Disabled`, `Pending`, `Running`, `Succeeded`, `Failed`
- `FailureClass` enum — `Network`, `Auth`, `Conflict`, `Unknown`; classified from `SnipError`
- Debounce state machine: Idle → Pending → Running with follow-up support
- Maximum debounce: 300 seconds (bounded deadline prevents indefinite postponement)
- Follow-up debounce: 1 second after sync completes with pending work
- Durable pending marker at `~/.config/snp/auto-sync-pending.toml` with CRC32 integrity
- PID-file lock at `~/.config/snp/auto-sync.lock` with stale detection via `kill -0`
- Lock permissions: 0o600 (restrictive)
- `run_auto_sync()` wraps `sync_commands::run_default_sync` with lock acquisition, retry/backoff, and failure handling
- `recover_stale_pending()` clears pending state older than 5 minutes on startup
- Retry: configurable `max_retries` (default 1), exponential backoff with caps (1s initial, 30s max)
- Failure policy rendering: `Warn`/`Error` modes emit user-facing stderr messages via `eprintln!`
- **Architecture:** In-process coordinator (Option A) — the mutation command owns debounce and sync execution. Option B (detached helper process) was evaluated and rejected due to added complexity (IPC, process supervision, cross-platform detachment) for marginal benefit.
- **Sync target:** Global — `library_id` field is vestigial; `run_default_sync` syncs all configured libraries. Per-library targeting deferred until the sync protocol supports it.
- `snp doctor --compatibility` inspects auto-sync state: pending markers, lock files, stale locks, config settings
- **Release 5C:** All syncable mutation commands are wired via the central notification API

### Auto-Sync Mutation Trigger Integration (Release 5C)

- Central mutation notification API: `notify_mutation(kind, origin)` and `notify_local_mutation(policy, context)`
- `MutationContext` struct: `{ kind, origin, library_id }` — carries classification without snippet content
- `AutoSyncNotificationResult` enum: `Disabled`, `Suppressed`, `Executed(AutoSyncStatus)`
- `clear_pending_after_explicit_sync()` — clears pending state after successful manual sync
- Commands wire trigger after their authoritative commit point (local atomic write succeeds)
- Trigger matrix: `new` (SnippetCreate), `edit` editor (SnippetUpdate), TUI delete (SnippetDelete), `import pet` (Import, once per import), `library create/delete` (LibraryChange)
- Local-only fields (`output`) do NOT trigger sync — output-only edits are excluded
- Explicit sync (`--sync` flag, `snp sync`) clears pending auto-sync state to prevent duplicate delayed sync
- Sync-origin writes (`MutationOrigin::SyncMerge`) never trigger auto-sync (prevents feedback loops)
- `run_auto_sync()` creates its own Tokio runtime internally — callers don't need to pass one
- Dry-run, cancel, failure, and no-op paths emit no notification

### snip-sync CLI
- Binary defaults to `serve` when no subcommand given (backward compatible)
- `CONFIG_PATH` env var overrides platform config dir
- PID file written at `state_dir()/snip-sync.pid`, cleaned on shutdown
- `croncheck` spawns detached child process; uses lock file to prevent races
- Cert generation shells out to `openssl` (not a Rust crypto crate)

### Creation and Shell Integration (`snp new`, `snp shell init`)
- `src/commands/new_cmd.rs` resolves positional, interactive, multiline, `--command-stdin`, `--from-file`, and `--editor` sources before using the shared save pipeline.
- `--command-stdin` validates UTF-8, rejects NUL bytes, preserves supplied trailing newlines, and caps input at 16 MiB.
- `--from-file` reads exact file content (valid UTF-8, 16 MiB limit, no NUL bytes, rejects directories). Symlinks are followed; the resolved target must be a regular file. Broken symlinks, directories, FIFOs, sockets, and device nodes are rejected.
- `--editor` opens `$VISUAL` (if set), then `$EDITOR`, then `vim`. The editor spec is parsed with `shell-words` so values like `code --wait`, `nvim -f`, or `"/path with spaces/bin/code" --wait` work without invoking a shell. Temp files use `tempfile::Builder` in the OS temp directory with `0600` permissions and RAII cleanup.
- All exact sources (stdin, file, editor) share `validate_exact_command_bytes()` for: 16 MiB cap, valid UTF-8, no NUL bytes, no empty/whitespace-only content.
- Stdin ingestion requires `--description`; do not mix metadata prompts with command stdin.
- Captured command bodies are data: never evaluate, execute, echo, or log them during ingestion.
- Editor errors only identify the editor executable and exit status — never the command body.
- `src/commands/shell_cmd.rs` generates Bash, Zsh, and Fish integration code
- CLI: `snp shell init <bash|zsh|fish>` prints generated code to stdout
- Four public functions per shell: `snp_select_raw`, `snp_select_expanded`, `snp_new_current`, and `snp_new_previous`
- Shell functions call `snp select --query <buffer> --raw/--expanded --output-file <tmpfile>`
- Capture helpers use shell-native current-buffer/history APIs and never parse history files.
- Capture helpers forward metadata as argument arrays/lists, preserve the buffer, and install no keybindings by default.
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

### Pet Import (`snp import pet`)
- `src/commands/import_cmd.rs` implements `snp import pet <path>` with options: `--library`, `--merge`, `--replace`, `--dry-run`, `--strict`, `--report human|json`, `--report-file`
- Import modes: `Create` (default, fails if destination exists), `Merge` (skip exact duplicates), `Replace` (backup then overwrite)
- Source is never modified; validated for UTF-8, file size (16 MiB cap), no NUL bytes
- Entry conversion: `convert_entry()` maps pet fields to native `Snippet` with fresh UUIDs and timestamps
- Duplicate detection: exact duplicate = same command + same description; semantic warnings for same-command-different-description and same-description-different-command
- Library name derived from source filename via `derive_library_name()` (sanitize, lowercase, replace non-alphanumeric with hyphens)
- Atomic writes via `save_library()`; library manager config updated via `add_existing_library()`
- Human report to stderr, JSON to stdout; `--report-file` for persistent reports
- Security: no command execution, no variable expansion, no source modification, no sync side effects

### Shared Pet Analysis (`src/commands/pet_analysis.rs`)
- Common module used by both `import_cmd` and `doctor_cmd` for consistent analysis
- Provides: `read_source_file()`, `parse_pet_toml()`, `detect_unknown_fields()`, `analyze_entry()`, `is_exact_duplicate()`, `same_command_different_description()`, `same_description_different_command()`, `detect_duplicates()`
- `KNOWN_SNIPPET_FIELDS` constant defines recognized pet snippet fields (canonical + aliases)
- Ensures doctor and import produce identical diagnostic codes for the same input

### Compatibility Diagnostics (`snp doctor`)
- `src/commands/doctor_cmd.rs` implements `snp doctor` with four modes: `--pet-file <path>`, `--compatibility`, `--library <name>`, and `--check-shell <bash|zsh|fish>`
- `--pet-file` performs read-only analysis of pet snippet files: TOML parse, unknown fields, missing required fields, empty commands, choice variables, duplicates, output fields, normalization preview, malformed placeholders, unsupported concepts, destination naming conflicts, and recommended import command
- `--compatibility` audits the installed snp environment: binary version, config directory, library directory, primary library, sync config, shell availability (bash/zsh/fish), shell init syntax validation, editor configuration, legacy paths, canonical Pet TOML loading, Release 1 select availability, Release 2 acquisition flags, and Release 3 choice-variable parser
- `--library <name>` analyzes a specific library file (resolved from `~/.config/snp/libraries/` or literal path)
- `--check-shell <shell>` validates `snp shell init` output syntax for the specified shell
- Options: `--strict` (treat warnings as errors), `--report human|json` (output format)
- Exit codes: 0 (no errors), 1 (operational failure), 2 (error diagnostics found)
- Human-readable report to stderr; JSON to stdout
- Never mutates source, destination, config, or library state
- Uses the shared diagnostic model from `src/diagnostics.rs` for consistency with import
- `src/diagnostics.rs` provides `SourceSpan`, `CompatibilityDiagnostic`, `DiagnosticSeverity`, `DoctorReport`, `PetImportReport` with stable machine-readable codes (E-/W-/I- prefix convention)

## Configuration Files

- `~/.config/snp/snippets.toml` — Main snippet storage (or per-library in `libraries/`)
- `~/.config/snp/sync.toml` — Sync settings (server URL, API key, direction)
- `~/.config/snp/libraries.toml` — Library metadata
- `~/.config/snp/libraries/*.toml` — Individual library files
- `~/.config/snp/premade/*.toml` — Downloaded premade libraries
- `~/.config/snp/logs/` — Rolling log files (daily rotation)
- `~/.config/snp/audit.log` — Audit log for snippet operations
- `~/.config/snp/usage.toml` — Local usage metadata (use_count, last_used_at per snippet)

## Design Decisions

### No Command Filtering (by design)
- Snippet commands are executed as-is via the user's shell — no sanitization, filtering, or guardrails.
- This is intentional: the tool targets power users who explicitly do not want safety restrictions.
- Any "safe mode" or metacharacter filtering is explicitly rejected as a design decision.
- Users are responsible for the commands they store and execute.

## Deferred Items

- **TUI pre-computed highlights memory pressure** (lazy computation for large libraries)
- **Release 4C: Optional external libraries** — deferred (zero demand, sufficient `snp import pet --merge` workflow, high implementation cost). See `plans/pet-compat-release-4c-external-libraries.md`.

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- `snip-sync` has a `test-helpers` feature gate for in-process server testing; `snp`'s dev-dependencies enable it automatically
- `tests/sync_integration.rs` spins up a real `snip-sync` server in-process via `test_helpers` — these are async `#[tokio::test]` and need the `test-helpers` feature
- PTY tests (`tests/pty_integration.rs`) use `portable-pty` crate and must run with `--test-threads=1` — they create real PTY pairs and inject keystrokes via raw fd writes
- Golden command corpus tests (`tests/integration.rs`) verify exact-text preservation across all acquisition sources (stdin, file, editor, positional) with 24 edge cases including tabs, trailing spaces, CRLF, Unicode, shell metacharacters, multiline, trailing newlines, and variable placeholders
- Shell init tests (`tests/integration.rs`) verify `snp shell init bash|zsh|fish` output contains all four public functions and passes syntax validation when the target shell is available
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation
- Utils tests cover escape sequences, nested brackets, chained backslashes
- Sync tests cover device conflict detection
- snip-sync has 78 tests (unit + integration)
- Pet import tests (`tests/integration.rs`) cover: default create, explicit library, collision, merge, dry-run, source untouched, JSON report, invalid inputs, strict/permissive modes, replace with backup, command preservation, choice variables, mixed aliases, help text, flag conflicts
- Pet import unit tests (`src/commands/import_cmd.rs`) cover: library name derivation, entry conversion, command preservation, normalization recording, and output diagnostics
- Sort integration tests (`tests/integration.rs`) cover: description sort, command sort, recent sort, favorites-first, default relevance, CSV sort, invalid sort value, help text for sort flags on all 5 commands
- Pet analysis unit tests (`src/commands/pet_analysis.rs`) cover: source file validation, TOML parsing, field detection, entry analysis, duplicate detection, malformed variable detection, and library name sanitization
- Doctor integration tests (`tests/integration.rs`) cover: valid file analysis, JSON output, nonexistent file, choice variables, compatibility audit, no-mode error, strict mode with errors, help text, source non-mutation, malformed TOML, warnings-only exit code, JSON stdout-only, human no-mutation, library mode, check-shell, compatibility completeness, malformed choices, unknown metadata fields, import dry-run consistency, no command execution, no variable expansion, no API key leakage, config non-mutation, required/default variables, duplicates with output, multiline commands, mixed field aliases, edge cases, empty file, normalization preview, malformed variable detection, canonical Pet TOML loading check, and library state non-mutation
- Doctor unit tests (`src/commands/doctor_cmd.rs`) cover: library name sanitization, and malformed variable detection
- Auto-sync coordinator tests (`src/auto_sync.rs`) cover: policy resolution, debounce state transitions, rapid mutation coalescing, maximum delay bound, mutation during running, disabled policy, sync-origin suppression, pending state round-trip, lock acquire/release, stale lock detection, lock permissions, failure classification, failure policy mapping, zero debounce, multiple cycles, stale pending recovery, no secrets in serialization, request creation, status equality, Debug impl, derive_state_dir, run_auto_sync disabled/lock behavior, retry/backoff computation, retry policy fields, timeout clamping, shutdown/signal lifecycle, stale lock recovery, crash recovery, no secrets in debug output, bounded pending state size, lock file no command bodies, retry zero config, integration cycles/disabled/no-recursive-trigger, and Release 5C notification API tests (disabled policy, sync-merge suppression, user/import/AccountConfig origins, all mutation kinds, library ID, clear-after-explicit-sync, result Debug/PartialEq, MutationContext construction)
- Diagnostics unit tests (`src/diagnostics.rs`) cover: counts, report constructors, version, severity serialization, diagnostic serialization, source span, span skip-none, diagnostic ordering, severity ranking, stable code convention, strict-mode classification, bounded messages, recommendation generation, empty counts, and full PetImportReport roundtrip
- Output presentation unit tests (`src/output.rs`) cover: empty, single-line, multiline, summary truncation, ANSI sanitization, OSC hyperlinks, control character stripping, scoring budget
- Output integration tests (`tests/integration.rs`) cover: JSON preservation, CSV preservation, default display, edit set/clear/stdin, search-output flag, multiline roundtrip, tab/special char roundtrip, no-eval security, conflict flags, ANSI preservation, help text

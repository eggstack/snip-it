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
│   ├── auto_sync/       # Two-process-per-cycle model (Release 5F)
│   │   ├── mod.rs        # Pub re-exports + paths::{state_dir, pending_marker, worker_lock, execution_lock}
│   │   ├── policy.rs     # AutoSyncPolicy, MutationKind, MutationOrigin, FailureClass
│   │   ├── pending.rs    # PendingState (schema v2), CRC32 integrity, v1→v2 migration
│   │   ├── pending_lock.rs # PendingTxnGuard RAII, short-lived transaction lock
│   │   ├── lock.rs       # WorkerLock RAII, WorkerLockContents, process_alive (kill -0)
│   │   ├── execution_lock.rs # SyncExecutionLock RAII, try_acquire, wait_acquire, ExecutionLockError
│   │   ├── executor.rs   # ExecutorExitCode, effective_sync_direction, run_executor
│   │   ├── spawn.rs      # spawn_worker, spawn_executor (setsid / DETACHED_PROCESS | CREATE_NO_WINDOW)
│   │   ├── worker.rs     # run, try_schedule, execute_sync, WorkerOutcome, SpawnResult
│   │   └── notification.rs # notify_mutation, notify_local_mutation, startup_recover_pending, SubcommandTag
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

- `AutoSyncPolicy` struct in `src/auto_sync/policy.rs` — effective policy resolved once per command invocation
- `AutoSyncFailureMode` enum in `src/config.rs` — Ignore, Warn (default), Error
- `MutationKind` enum in `src/auto_sync/policy.rs` — classifies mutations for sync triggers
- Configuration in `sync.toml`: `auto_sync`, `auto_sync_debounce_seconds`, `auto_sync_failure`
- CLI: `snp sync config --show| --auto-sync on|off | --debounce <secs> | --failure ignore|warn|error`
- Auto-sync is disabled by default; local mutations always commit before remote work begins
- Remote failure never rolls back local state
- Existing manual `snp sync`, `snp cron`, and daemon workflows are unchanged
- `MutationKind::AccountConfig` never triggers sync; all other kinds are syncable when enabled
- Debounce range: 0-300 seconds (clamped); default: 2 seconds
- `error` failure mode sets nonzero exit code but local mutation remains committed

### Auto-Sync Detached Worker (Release 5D corrective)

- Replaces the in-process `AutoSyncCoordinator` with a hidden `snp auto-sync-worker` subcommand re-execed by the parent. The worker is fully detached via `setsid` on Unix and `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows, with `stdin`/`stdout`/`stderr` routed to `null` (or to the file named by the optional `SNP_AUTO_SYNC_WORKER_LOG` env var, used by tests). The parent returns immediately after spawning — no in-process latency for the user.
- Module layout under `src/auto_sync/`: `policy.rs`, `pending.rs`, `pending_lock.rs`, `lock.rs`, `execution_lock.rs`, `executor.rs`, `spawn.rs`, `worker.rs`, `notification.rs`, `mod.rs`.
- `WorkerLock` RAII (`src/auto_sync/lock.rs`): atomic acquisition via `OpenOptions::create_new(true)`; `WorkerLockContents { pid, started_at_unix_ms, nonce }`; stale detection via `kill -0 pid` only (live PID means owned, regardless of age). **Release 5E corrective:** ownership-checked drop — only removes the lock if PID and nonce still match. 0o600 permissions on Unix. **Release 5 corrective:** the parent never acquires the lock — it is the worker's responsibility.
- `PendingTxnGuard` (`src/auto_sync/pending_lock.rs`): short-lived transaction lock for pending-marker operations. Atomic acquire via `create_new(true)`; PID+nonce+timestamp in lock file; ownership-checked drop. Bounded retry with jitter (1-5ms) up to 500ms. Dead-owner reclaim via `kill -0`; live owners never stolen. Separate from `WorkerLock` — held only for the minimum read/modify/write critical section.
- `PendingState` schema v2 (`src/auto_sync/pending.rs`): monotonic `generation`, `created_at_unix_ms`, `snapshot` (Mutation/Nil), CRC32 `integrity` field over all behavior-driving fields (schema, generation, timestamp, snapshot). Conditional `clear_if_generation_matches(observed_generation)` returns `ConditionalClearResult` (Cleared/Missing/GenerationChanged). v1 markers migrate transparently on load. Unique temp files per transaction via `pending_lock::unique_temp_path`.
- **Release 5 corrective API split:** `pending::record_pending_mutation` is the only writer/incrementer of the pending generation. `worker::schedule_existing_pending` reads the marker but never mutates it. `mark_pending` is module-private. The parent path is now strictly `record → schedule` with no opportunity for a stale-worker-clears-newer-state race.
- `spawn_worker` (`src/auto_sync/spawn.rs`): re-execs `std::env::current_exe()` with `--state-dir`, detached flags, null stdio (unless `SNP_AUTO_SYNC_WORKER_LOG` is set). Returns child pid.
- `WorkerOutcome` (`src/auto_sync/worker.rs`): `Success` / `Failed` / `NothingToDo`. Mapped to internal exit code 0 — outcomes are logged, not propagated.
- `notify_mutation(kind, origin)` → `notify_local_mutation(policy, context)` → `pending::record_pending_mutation(state_dir, snapshot)` → `worker::schedule_existing_pending(state_dir)` → `spawn::spawn_worker(...)`. No lock acquisition on the parent side.
- **Release 5 corrective worker loop:** `worker::run_locked` reads the marker, acquires the `SyncExecutionLock` via `try_acquire` (exits `NothingToDo` if busy), computes a debounce deadline relative to `observed_timestamp`, then enters `wait_for_quiet` which reloads the marker every ≤250ms and detects newer generations during the wait. On expiry it runs `execute_sync` which spawns an executor subprocess (`snp auto-sync-execute`) and supervises it with `wait_child_with_timeout(policy.sync_timeout)`. On timeout, the worker sends SIGTERM, waits 2 seconds, then SIGKILL. On success with no newer generation it calls `clear_if_generation_matches(observed_generation)`; on success with a newer generation it loops again (follow-up cycle). Max lifetime is 5 minutes.
- `startup_recover_pending()` runs at startup for non-worker subcommands. Preserves pending markers (does not auto-discard); re-schedules a worker if recent pending state is found.
- `clear_pending_after_explicit_sync(observed_generation, sync_succeeded)` runs after `snp sync` or `--sync` flag. Generation-safe: clearing is conditional on the caller-supplied observed generation. Captured via `observe_pending_generation()` before the explicit sync.
- `paths::{state_dir, pending_marker, worker_lock, execution_lock}` helpers expose stable paths to `snp doctor --compatibility`.
- `snp doctor --compatibility` inspects auto-sync state using `lock::process_alive` (kill -0 on Unix) for liveness probes.
- Security: no command payloads, credentials, or encryption material in worker argv, env, pending markers, or lock files. All artifacts written with 0o600 on Unix.
- Worker creates its own Tokio runtime internally — the parent does not pass one.
- **Sync target:** Global — `library_id` field is vestigial; `run_default_sync` syncs all configured libraries.
- **Architecture:** Two-process-per-cycle model (Release 5F). A detached debounce worker spawns a killable sync executor subprocess. The earlier in-process coordinator was evaluated and removed: it added visible latency to mutation commands and held the parent process hostage during network round-trips. The Release 5D single-process model used `tokio::time::timeout` around `spawn_blocking`, which cannot cancel the underlying thread. Release 5F replaces this with a real child process that can be SIGTERM/SIGKILLed on timeout.

### Auto-Sync Integration Hardening and Closure (Release 5D)

- Architecture reconciliation: canonical data flow documented in `architecture/auto_sync.md`
- All mutation commands route through central `notify_mutation()` — no ad-hoc auto-sync logic exists outside the coordinator
- Trigger matrix reconciled across implementation, tests, and documentation (12 command types)
- Local-first durability: local commits always succeed before remote work; failed sync never rolls back local state
- Cross-process safety: PID+nonce worker lock with stale detection; no permanent deadlock; no unbounded sync storm
- Security: no command payloads, credentials, or encryption material in lock files, pending markers, or status files
- Pending marker bounded, versioned (schema v2), CRC32 integrity-checked, symlink-resistant atomic creation
- Manual/scheduled sync behavior unchanged; explicit sync clears pending to prevent duplicate delayed sync
- Documentation reconciled: README, USER_GUIDE, AGENTS.md, CHANGELOG, PET_COMPATIBILITY, architecture docs
- `architecture/auto_sync.md` deep-dive updated for detached worker model; `architecture/overview.md` updated with auto-sync section
- `docs/PET_COMPATIBILITY.md` updated: R5 marked as Implemented (was Planned)
- `docs/CLI_EXITCODE_STREAM_POLICY.md` updated: auto-sync error exit code documented

### Auto-Sync Transactional Pending State (Release 5E corrective)

- `PendingTxnGuard` (`src/auto_sync/pending_lock.rs`): short-lived transaction lock serializing concurrent CLI processes on the pending marker. Atomic acquire via `create_new(true)`; PID+nonce+timestamp in lock file; ownership-checked drop (only removes if PID and nonce match). Bounded retry with 1-5ms jitter up to 500ms. Dead-owner reclaim via `kill -0`; live owners never stolen. Distinct from `WorkerLock` (the long-lived sync execution lock).
- Transactional generation increment: `record_pending_mutation` acquires `PendingTxnGuard` for the entire read-modify-write critical section. Unique temp files per transaction via `pending_lock::unique_temp_path()` prevent shared-temp-path races. Atomic rename + directory fsync.
- Atomic conditional clear: `clear_if_generation_matches` acquires `PendingTxnGuard` before reading, comparing, and deleting. Returns typed `ConditionalClearResult` (Cleared/Missing/GenerationChanged) instead of generic bool.
- Full-state CRC32 integrity: CRC covers all behavior-driving fields (schema, generation, timestamp, snapshot) — not just the snapshot. Detects corruption to any control field.
- Worker lock ownership safety: `WorkerLock::drop` reads the current lock record and removes only if PID and nonce match. Live PIDs are never stolen due to age. Dead owners reclaimed via `kill -0` only.
- API cleanup: `mark_pending` is module-private (not exported). All generation writes go through `record_pending_mutation`. All clears go through the transactional conditional-clear primitive.
- Stale/abandoned artifact recovery: dead pending txn locks reclaimed; live locks never stolen; unique temp files cleaned by age; pending markers never deleted because old; lock cleanup is ownership-checked.

### Auto-Sync Unified Execution (Release 5F)

- All sync operations — detached worker, manual `snp sync`, explicit `--sync` flag, and cron — acquire `SyncExecutionLock` (`src/auto_sync/execution_lock.rs`) before performing actual sync. This prevents concurrent sync operations from interfering with each other.
- `SyncExecutionLock`: atomic acquisition via `create_new(true)`, ownership-checked drop, stale detection via `kill -0`. Foreground callers use `wait_acquire` with 30s bounded timeout (polls every 250ms); detached workers use `try_acquire` (exit `NothingToDo` if busy).
- Worker spawns killable executor subprocess (`snp auto-sync-execute`, `src/auto_sync/executor.rs`) instead of running sync in-process. Executor is NOT detached — the worker waits on it and can SIGTERM/SIGKILL on timeout.
- `ExecutorExitCode` standardized codes: 0=success, 2=not configured, 3=auth failure, 4=network/timeout, 5=conflict/partial, 6=local persistence, 7=internal error. Worker maps these to `WorkerOutcome`.
- `effective_sync_direction()`: CLI flags (`--push-only`, `--pull-only`) override config direction; used by executor.
- Timeout terminates the executor process (SIGTERM → 2s grace → SIGKILL) before execution lock released. This replaces the Release 5D `tokio::time::timeout` around `spawn_blocking`, which could not cancel the underlying thread.
- `SubcommandTag` and `should_attempt_auto_sync_recovery()`: startup recovery suppressed for `sync`, `cron`, `register`, and internal subprocess commands (`auto-sync-worker`, `auto-sync-execute`). Only mutation commands and default (no subcommand) attempt recovery.
- `paths::execution_lock()` added for doctor/diagnostics integration.

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
- Auto-sync tests (`src/auto_sync/`): unit tests in each submodule cover policy resolution (`policy.rs`), PendingState schema v2 roundtrip, generation monotonicity, conditional clear, CRC32 integrity, v1→v2 migration, atomic write, 0o600 permissions, no-secrets guarantee (`pending.rs`), PendingTxnGuard acquire/drop, contention, dead-owner reclaim, ownership-checked drop, permissions, no-secrets, unique temp paths (`pending_lock.rs`), WorkerLock RAII, atomic acquire, dead-PID reclaim, live-owner non-reclaim, ownership-checked drop, nonce uniqueness, lock file permissions (`lock.rs`), `spawn_worker` plumbing and detach flags (`spawn.rs`), `WorkerOutcome` mapping, `SpawnResult` matrix, `execute_sync`, `startup_recover`, `run_locked` debounce and follow-up cycle, executor supervision with timeout (`worker.rs`), and `notify_local_mutation` for disabled policy, sync-merge suppression, and result variants (`notification.rs`).
- Execution lock tests (`src/auto_sync/execution_lock.rs`): acquire/release, contention (double-acquire fails), dead-PID reclaim, live-owner non-reclaim by age, ownership-checked drop, nonce uniqueness, lock file permissions (0o600), no-secrets guarantee, inspect roundtrip, malformed/empty lock treated as stale, timeout error display, wait-acquire success, wait-acquire timeout, wait-acquire resolves after drop, and contents roundtrip.
- Executor subprocess tests (`src/auto_sync/executor.rs`): exit code values, exit code display, from_exit_status, to_exit_status roundtrip, effective_sync_direction (push-only, pull-only, config fallback, CLI overrides config), and executor subcommand name.
- Command classification tests (`src/auto_sync/notification.rs`): recovery allowed for None and Mutation, blocked for Sync, Cron, Register, AutoSyncWorker, AutoSyncExecute; SubcommandTag equality and debug.
- Auto-sync integration tests (`tests/auto_sync_coordinator.rs`, `tests/auto_sync_concurrency.rs`, `tests/auto_sync_mutations.rs`, `tests/auto_sync_regression.rs`, `tests/auto_sync_security.rs`, `tests/auto_sync_config.rs`, `tests/integration.rs`) cover: pending marker creation, disabled policy, stdin/file creation triggers, output-only edit exclusion, library create/delete triggers, import dry-run exclusion, import success trigger, failed sync local preservation, explicit sync interaction, sequential mutation handling, schema v2 format (`schema = 2`, `generation`, `integrity = "crc32:..."`), pending marker with library ID, integrity header presence, cross-process safety, concurrency hardening, mutation trigger matrix, and no-secrets security guarantees.
- Auto-sync integration tests (`tests/integration.rs`) cover: pending marker creation, disabled policy, stdin/file creation triggers, output-only edit exclusion, library create/delete triggers, import dry-run exclusion, import success trigger, failed sync local preservation, and explicit sync interaction
- Auto-sync end-to-end subprocess tests (`tests/auto_sync_detached_worker.rs`) cover the full Release 5 corrective contract: generation increments exactly once per mutation, `schedule_existing_pending` never mutates the marker, parent never holds the worker lock, workers race for the lock, mutations during sync are preserved across the follow-up cycle, stale worker clearing cannot clobber newer generations, startup recovery preserves valid pending work, explicit sync clears via observed generation, worker argv/env contains no payloads or credentials, CLI exits promptly after mutation, and the worker completes its lifecycle independently.
- Diagnostics unit tests (`src/diagnostics.rs`) cover: counts, report constructors, version, severity serialization, diagnostic serialization, source span, span skip-none, diagnostic ordering, severity ranking, stable code convention, strict-mode classification, bounded messages, recommendation generation, empty counts, and full PetImportReport roundtrip
- Output presentation unit tests (`src/output.rs`) cover: empty, single-line, multiline, summary truncation, ANSI sanitization, OSC hyperlinks, control character stripping, scoring budget
- Output integration tests (`tests/integration.rs`) cover: JSON preservation, CSV preservation, default display, edit set/clear/stdin, search-output flag, multiline roundtrip, tab/special char roundtrip, no-eval security, conflict flags, ANSI preservation, help text

# Internal Architecture Inventory

A concise map of the snp internal architecture for contributors working on pet-compatibility features. For deeper module documentation, see the `architecture/` directory (start with `architecture/overview.md`).

## Module Map

### Core Data (`src/library.rs`)
- `Snippets` container struct — serde `rename="snippets"` with `alias="Snippets"` (pet compat)
- `Snippet` struct — fields: `id`, `description`, `command`, `tags` (rename=`"tag"`), `output`, `folders`, `favorite`, `created_at`, `updated_at`, `device_id`, `deleted`
- Serde aliases on most fields enable bidirectional compatibility with pet's TOML format (`Description`, `Tags`, `Tag`, `Command`, `Output`, etc.)
- `LibraryManager` — multi-library support via `libraries.toml` registry, path resolution
- Key functions: `load_library()`, `save_library()`, `backup_library()`, `deduplicate_ids()` (inline in `load_library`)
- `validate_library_name()` — path traversal protection
- 29 unit tests covering pet format compat, roundtrip, atomic writes, path traversal rejection

### CLI (`src/main.rs`, `src/commands/`)
- Clap derive CLI with `Option<Commands>` — no subcommand defaults to `run` (TUI selector)
- 16 command modules, each with a public `run()` entry function
- `RUNTIME: LazyLock<Runtime>` — lazy Tokio async runtime, only initialized by async commands
- `dispatch_command()` — top-level match on `Option<Commands>`
- Shared helpers in `src/commands/mod.rs`:
  - `get_library_path()` — resolves named library or primary library path
  - `expand_snippet_command()` — parses variables, prompts UI if needed, returns `ExpandedCommand`
  - `run_snippet_selection()` — loads library, runs TUI loop, calls process callback, handles delete/selected outcomes
  - `get_snippet_data()` — extracts parallel arrays for TUI, filters deleted snippets
  - `load_snippets()` / `save_snippets()` — legacy single-file operations
  - `ExpandedCommand` enum — `Cancel`, `Skip`, `Expanded(String)`

### Creation Pipeline (`src/commands/new_cmd.rs`)
- `CommandSource` separates positional, interactive, multiline, stdin, file, and editor command bodies before persistence.
- `read_command_stdin()` reads at most 16 MiB, validates UTF-8, rejects NUL bytes, and preserves all other bytes including trailing newlines.
- `read_command_from_file()` follows symlinks and validates the resolved target is a regular file (directories, FIFOs, sockets, device nodes rejected).
- `read_command_from_editor()` resolves `$VISUAL` → `$EDITOR` → `vim`. The editor spec is parsed with `shell-words` (no shell invoked). Temp files use `tempfile::Builder` in the OS temp dir with `0600` permissions and RAII cleanup.
- All exact sources (stdin, file, editor) share `validate_exact_command_bytes()` for: 16 MiB cap, valid UTF-8, no NUL bytes, no empty/whitespace-only content.
- `--command-stdin` requires `--description` and never reuses command stdin for metadata prompts.
- Positional, prompt, stdin, file, and editor sources converge on `Snippet::new()` and the existing `save_library()` / `save_snippets()` backup and atomic-write paths.
- Stdin command bodies are never echoed or logged as part of ingestion; the command is data and is not executed.
- Editor errors identify only the editor executable and exit status — never the command body.

### Shell Integration (`src/commands/shell_cmd.rs`)
- `snp shell init` generates Bash, Zsh, and Fish code at runtime.
- Four public functions are generated per shell: `snp_select_raw`, `snp_select_expanded`, `snp_new_current`, and `snp_new_previous`.
- Selection uses the existing temp-file output contract; capture helpers pass command text to `snp new --command-stdin` without evaluation.
- Current-buffer helpers use Readline, ZLE, or `commandline`; previous helpers use native `fc` or Fish `history search` and never parse history files.
- No keybindings are installed automatically. Metadata is forwarded as shell argument arrays/lists, and capture helpers preserve the current buffer.

### Pet Import (`src/commands/import_cmd.rs`)
- `snp import pet <path>` — first-class import command for pet snippet files.
- Domain model: `PetImportOptions`, `PetImportReport`, `ImportMode` (Create/Merge/Replace), `ReportFormat` (Human/Json).
- Source loading: `read_source_file()` validates UTF-8, file size (16 MiB cap), no NUL bytes, rejects directories.
- Entry conversion: `convert_entry()` maps pet fields to native `Snippet` with fresh UUIDs and timestamps.
- Duplicate detection: exact (same command + description), semantic warnings for same-command-different-description and same-description-different-command.
- Library name derivation: `derive_library_name()` sanitizes source filename (lowercase, replace non-alphanumeric with hyphens).
- Atomic writes via `save_library()`; library config updated via `add_existing_library()`.
- Dry-run mode: all parsing and validation without file mutation.
- Strict mode: any error-severity diagnostic aborts the entire import.
- Human report to stderr, JSON to stdout; optional `--report-file` for persistent reports.
- Security: no command execution, no variable expansion, no source modification, no sync side effects.

### Compatibility Diagnostics (`src/commands/doctor_cmd.rs`, `src/diagnostics.rs`)
- `snp doctor --pet-file <path>` — read-only analysis of pet snippet files without creating a destination library
- `snp doctor --compatibility` — audits installed snp environment (binary, config, libraries, sync, shells)
- Shared diagnostic model in `src/diagnostics.rs`: `DiagnosticSeverity`, `CompatibilityDiagnostic`, `DoctorReport`, `PetImportReport`
- Diagnostic codes are stable and machine-readable (e.g., `entry.empty_command`, `compat.config_dir.ok`)
- Human-readable report to stderr; JSON to stdout; `--strict` treats warnings as errors
- Exit codes: 0 (no errors), 1 (operational failure), 2 (error diagnostics found)
- Reuses the same source validation, TOML parsing, and entry analysis as `import_cmd` for consistency
- Security: doctor never mutates source, destination, config, or library state
- External library support (R4-C) is deferred: no runtime behavior, no config surface, no provenance tracking. See `plans/pet-compat-release-4c-external-libraries.md` for rationale.

### Error Handling (`src/error.rs`)
- `SnipError` enum (`#[non_exhaustive]`): `Io`, `Toml`, `Clipboard`, `Command`, `Runtime`
- `SnipResult<T>` type alias
- Constructor helpers: `io_error()`, `toml_error()`, `clipboard_error()`, `command_error()`, `runtime_error()`
- `From<io::Error>` auto-conversion with kind-based operation strings
- `From<CryptoError>` for encryption failures
- All errors → `exit(1)` in `main()` — no exit code distinction

### TUI (`src/ui/`)
- `select_snippet_inner()` — main event loop (1955 lines in `mod.rs`)
- State types in `state.rs`: `SelectState`, `FilterState`, `SortMode`, `is_ctrl_key()`
- `TerminalGuard` — RAII mouse capture cleanup
- `MATCHER: LazyLock<SkimMatcherV2>` — pre-initialized fuzzy matcher
- `FilterRequest` — incremental filter narrowing optimization
- Theme system (`theme.rs`): Halloy TOML at `~/.config/snp/themes/<name>.toml`, bundled themes via LZMA
- Variable prompting UI (`variables.rs`): `VariablePromptResult` enum
- Syntax highlighting (`highlight.rs`): pre-computed once at startup
- `SnippetSelection` enum — `Selected(idx, copy_flag)`, `Delete(idx)`, `Cancelled`
- Signal handling: `TERMINATE` atomic flag, registered via `signal-hook` on Unix

### Sort and Ranking (`src/sort.rs`)
- `SnippetSort` enum: `Relevance` (default), `Recent`, `LastUsed`, `MostUsed`, `Description`, `Command`
- `SnippetSort` derives `clap::ValueEnum` with `#[value(rename_all = "kebab-case")]`
- `SortOptions` struct: `mode: SnippetSort`, `favorites_first: bool`
- `rank_snippets()` — deterministic sort with tie-break chain:
  1. Primary key (sort mode)
  2. Favorites-first grouping (orthogonal modifier)
  3. Fuzzy relevance (for Relevance mode)
  4. Normalized description (case-insensitive)
  5. Original index (stable)
- CLI flags `--sort` and `--favorites-first` on: run, clip, search, select, list
- TUI interactive sort via `n` key (sort by newest/oldest/a-z/z-a/used/freq), `o` key (toggles favorites-first)
- TUI sort indicators: `[new]`, `[old]`, `[a-z]`, `[z-a]`, `[used]`, `[freq]`

### Usage Tracking (`src/usage.rs`)
- `UsageIndex` struct: `entries: Vec<UsageEntry>`
- `UsageEntry`: `id: String`, `use_count: u64`, `last_used_at: Option<i64>`
- `UsageData`: return type for `get_usage()` — zeroed defaults for unknown IDs
- Persistence: `~/.config/snp/usage.toml` (TOML array-of-tables)
- Atomic writes via `write_private_atomic()`
- Fail-open: missing/corrupt file returns empty index
- `record_use()` — increments count, sets timestamp
- `prune()` — removes entries for deleted snippets
- Recorded on: successful `run` and `clip` operations only
- Local-only: never synced, never written to library TOML, no command text logged

### Variable System (`src/utils/variables.rs`)
- `Variable` struct: `name: String`, `kind: VariableKind`, `default: Option<String>`
- `VariableKind` enum: `Required`, `DefaultValue(String)`, `Choices { values, default_index }`
- Pet choice syntax `<name=|_opt1_||_opt2_||>` detected via `is_choice_syntax()` and `extract_choices()`
- `extract_variable_tokens()` — handles escaped brackets (`\<`, `\>`), nested brackets (`<a<b>>`), chained backslashes
- `parse_variables()` — returns `Vec<Variable>`
- `expand_command()` — replaces tokens with values, preserves unmatched `<` as literals
- `strip_escape_sequences()` — converts `\<`→`<`, `\>`→`>`, `\\`→`\`
- `has_unmatched_angle_bracket()` — validation check
- Edge case: bare `<` without matching `>` is treated as literal (preserved in output)

### Config (`src/config.rs`)
- `SyncSettings` struct — `enabled`, `server_url`, `api_key` (zeroized on drop), `device_id`, `sync_interval_minutes`, `auto_sync`, `sync_direction`, `clipboard_auto_clear_seconds`, `sync_limit`
- API key serialization: `@keychain` marker triggers OS keychain storage via `keyring` crate
- `Debug` impl redacts API key
- `SyncDirection` enum (default, push-only, pull-only)
- TOML cache: `CachedToml` with `mtime`/`len` invalidation, max 100 entries
- `cached_read_toml()` — file content cache with CRC32 integrity header
- `invalidate_toml_cache()` — called after every write

### Sync (`src/sync.rs`, `src/sync_commands.rs`)
- gRPC client via `tonic` for snip-sync server
- `SyncRetryConfig` — exponential backoff (3 retries, 100ms–5s)
- `SyncClient` wraps `SnippetSyncClient<Channel>` with TLS
- Encrypted payload exchange (AES-256-GCM via `encryption` module)
- Merge strategy: last-write-wins by `updated_at` timestamp
- Local-only fields (`output`, `folders`, `favorite`) preserved when server wins
- `sync_commands.rs` — orchestration layer, `run_default_sync()`

### Auto-Sync (`src/auto_sync/`)

Optional post-mutation background synchronization (Release 5A–5F). Disabled by default; opt-in via `snp sync config --auto-sync on`. Two-process-per-cycle model: a detached debounce worker spawns a killable executor subprocess.

- **`AutoSyncPolicy`** (`policy.rs`) — effective policy resolved once per invocation from `SyncSettings`. Fields: `enabled`, `debounce`, `failure_mode`, `max_retries`, `sync_timeout`.
- **`PendingState`** (`pending.rs`) — durable pending marker (schema v2) with monotonic `generation`, `created_at_unix_ms`, CRC32 `integrity` over all behavior-driving fields. v1 markers migrate transparently. `ConditionalClearResult` enum (Cleared/Missing/GenerationChanged) returned by conditional clear.
- **`PendingTxnGuard`** (`pending_lock.rs`) — short-lived transaction lock serializing concurrent CLI processes on the pending marker. Atomic acquire via `create_new(true)`; ownership-checked drop; bounded retry with jitter; dead-owner reclaim via `kill -0`; unique temp files per transaction; atomic rename + directory fsync.
- **`WorkerLock`** (`lock.rs`) — RAII cross-process lock with PID+nonce. Stale detection via `kill -0` only (live PID means owned, regardless of age). Ownership-checked drop — only removes if PID and nonce match. Atomic acquire via `OpenOptions::create_new`. 0o600 permissions.
- **`SyncExecutionLock`** (`execution_lock.rs`) — shared execution lock for all sync operations. `try_acquire` (non-blocking, for workers) and `wait_acquire` (bounded timeout, for foreground callers). Ownership-checked drop, stale detection via `kill -0`.
- **`ExecutorExitCode`** (`executor.rs`) — standardized exit codes: 0=success, 2=not configured, 3=auth, 4=network/timeout, 5=conflict, 6=local persistence, 7=internal. `effective_sync_direction()` resolves CLI overrides.
- **`spawn_worker`** (`spawn.rs`) — re-execs `std::env::current_exe()` as `snp auto-sync-worker` with platform-detached flags (`setsid` on Unix, `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows) and `stdin`/`stdout`/`stderr` routed to `null`.
- **`spawn_executor`** (`spawn.rs`) — spawns `snp auto-sync-execute` as a child process (NOT detached) for killable sync execution.
- **`WorkerOutcome`** (`worker.rs`) — `Success` / `Failed` / `NothingToDo`. Mapped to internal exit code 0; outcome is logged, not propagated.
- **`MutationKind`** — enum classifying mutations: `SnippetCreate`, `SnippetUpdate`, `SnippetDelete`, `Import`, `LibraryChange`, `PremadeInstall`, `SyncConflictWrite`, `AccountConfig` (never triggers).
- **`MutationOrigin`** — `User`, `Import`, `SyncMerge` (suppresses trigger), `Recovery`.
- **`MutationContext`** — carries `kind`, `origin`, `library_id` without snippet content.
- **`AutoSyncNotificationResult`** — `Disabled`, `Suppressed`, `Scheduled { generation }`, `SchedulingFailed { generation }`.
- **`SubcommandTag`** / **`should_attempt_auto_sync_recovery()`** — classifies commands at startup; recovery suppressed for sync/cron/register/internal subprocesses.
- **Central API**: `notify_mutation(kind, origin)` — convenience function for commands. `notify_local_mutation(policy, context)` — full control.
- **`clear_pending_after_explicit_sync()`** — clears pending state after successful manual sync to prevent duplicate delayed sync.
- **`startup_recover_pending()`** — runs at startup for non-worker subcommands; preserves pending markers and re-schedules a worker if recent pending state is found.
- **`auto-sync-worker`** — hidden subcommand (clap `hide = true`) that runs debounce loop, spawns executor, supervises with timeout, then exits.
- **`auto-sync-execute`** — hidden subcommand that invokes `crate::sync_commands::run_sync` (the canonical sync operation); does NOT acquire `SyncExecutionLock` — the worker owns it for the cycle. Exits with `ExecutorExitCode`.
- **Trigger matrix**: all syncable mutation commands call `notify_mutation()` after their local atomic commit. Output-only edits (`snp edit --output`) are excluded (output is local-only).
- **Debounce**: configurable 0–300 seconds (default 2). Rapid mutations coalesce. Maximum deadline bound prevents indefinite postponement.
- **Failure modes**: `Ignore` (silent), `Warn` (stderr message), `Error` (nonzero exit code). Local mutation always committed regardless.
- **Sync target**: Global — `library_id` field is vestigial; `run_default_sync` syncs all configured libraries.
- **Three lock concepts**: `PendingTxnGuard` (short-lived, for marker transactions), `WorkerLock` (long-lived, for worker lifecycle), and `SyncExecutionLock` (shared, for actual sync operations). Never mixed.

### Encryption (`src/encryption.rs`)
- AES-256-GCM + Argon2id key derivation (OWASP: 16 MiB, 3 iterations, 4 threads)
- `encrypt()` / `decrypt()` — per-snippet encryption for sync
- `CryptoError` enum (separate from `SnipError`, converted via `From`)
- Key derivation cache: max 10K entries, SHA-256 cache keys
- `ZeroizeOnDrop` on key material
- Random 12-byte nonce per encryption, stored with ciphertext

### Clipboard (`src/clipboard.rs`)
- `pub(crate)` — internal only
- Cross-platform: `arboard` (macOS/Linux), `clipboard-win` (Windows)
- `copy_to_clipboard_auto()` — used by run/clip commands

### Utils (`src/utils/`)
- `mod.rs` — re-exports: `parse_variables`, `expand_command`, `strip_escape_sequences`, `extract_variables_for_display`, `has_unmatched_angle_bracket`
- `config.rs` — `get_config_dir()`, `get_snippets_path()`, `get_sync_config_path()`, macOS migration
- `variables.rs` — variable parsing/expansion
- `toml_helpers.rs` — `fix_invalid_toml_escapes()` (hand-written TOML token scanner for legacy files), `quote_strings_containing_backslashes()` (public utility, no longer called by snip-it's own save pipeline)
- `shell_keywords.rs` — ~190 shell command names for syntax highlighting
- `tempfile_guard.rs` — RAII temp file cleanup
- `atomic.rs` — `write_private_atomic()` (temp file + rename, 0600 permissions on Unix)

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` (derive) | CLI framework |
| `ratatui` + `crossterm` | TUI |
| `toml` + `serde` | TOML serialization |
| `fuzzy-matcher` (skim) | Fuzzy search |
| `keyring` | OS keychain for API keys |
| `tokio` | Async runtime (lazy-init) |
| `tonic` | gRPC client |
| `arboard` / `clipboard-win` | Clipboard |
| `lzma-rs` | Bundled theme decompression |
| `chrono` | Timestamps |
| `uuid` | Snippet ID generation |
| `argon2` + `aes-gcm` | Encryption |
| `sha2` | Key cache hashing |
| `zeroize` | Secure memory cleanup |
| `crc32fast` | Config integrity |
| `signal-hook` | Unix signal handling |
| `clap_complete` | Shell completions |
| `tracing` | Structured logging |
| `tempfile` | Atomic writes, editor temp files |
| `shell-words` | Editor command specification parsing (no shell invocation) |

## Test Infrastructure

- `tests/integration.rs` — CLI integration tests (45+ tests, `TempDir` + `XDG_CONFIG_HOME` override)
- `tests/pty_integration.rs` — PTY end-to-end tests (21 tests, `portable-pty` crate, runs with `--test-threads=1`)
- `tests/sync_integration.rs` — gRPC integration tests (4 async `#[tokio::test]`, in-process server via `test-helpers` feature)
- Inline `#[cfg(test)]` modules in every source file
- All test data is inline — no fixture files
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation
- Sort unit tests (`src/sort.rs`): 32 tests covering all sort modes, tie-breakers, favorites-first, edge cases
- Usage unit tests (`src/usage.rs`): 7 tests covering load/save, record, prune, corruption fail-open

## Data Flow: Run Command

```
main.rs::dispatch_command()
  → commands::run_cmd::run()
    → commands::run_snippet_selection()
      → get_library_path() → LibraryManager → resolve .toml path
      → library::load_library() → parse TOML, deduplicate IDs
      → get_snippet_data() → parallel arrays (filter deleted)
      → sort::rank_snippets() → apply --sort mode + --favorites-first
      → ui::select_snippet() → TUI event loop (fuzzy search, navigation)
      → User selects snippet
        → expand_snippet_command() → parse_variables()
        → If variables: ui::prompt_variables() → user input
        → expand_command() → replace tokens with values
        → process_snippet()
          → If copy flag: clipboard::copy_to_clipboard_auto()
          → If output field: redirect stdout to file (path-validated)
          → Else: spawn shell with $SHELL -c (or $COMSPEC /C on Windows)
          → wait_for_command() → optional timeout via $SNP_COMMAND_TIMEOUT
          → audit_log("execute" or "copy")
          → usage::UsageIndex::record_use() → persist to usage.toml
    → If --sync: sync_commands::run_default_sync()
  → Exit with status code
```

## Data Flow: Auto-Sync Mutation Trigger

```
mutation command (new/edit/delete/import/library)
  → validate input
  → atomic local commit (save_library / save_snippets)
  → audit log
  → auto_sync::notify_mutation(kind, origin)
    → load SyncSettings → resolve AutoSyncPolicy
    → if disabled → return Disabled (no-op)
    → if origin == SyncMerge → return Suppressed (no feedback loop)
    → pending::record_pending_mutation(state_dir, snapshot)
      → PendingState{generation: N+1, ...}
    → worker::schedule_existing_pending(state_dir)  [never mutates pending]
      → spawn::spawn_worker(current_exe, "auto-sync-worker", state_dir)
    → return AutoSyncNotificationResult::Scheduled{generation}

snp auto-sync-worker (detached child process)
  → execution_lock::try_acquire(state_dir) → SyncExecutionLock (or AlreadyHeld → exit NothingToDo)
  → read pending state (observed generation/timestamp)
  → debounce loop: sleep in ≤250ms increments, reload marker, restart on newer generation
  → execute_sync(state_dir, policy)
    → spawn::spawn_executor(state_dir) → child process (snp auto-sync-execute)
    → wait_child_with_timeout(child, policy.sync_timeout)
      → on exit: map ExecutorExitCode → WorkerOutcome
      → on timeout: SIGTERM → 2s grace → SIGKILL → WorkerOutcome::Failed
    → on success: pending::clear_if_generation_matches(state_dir, generation)
    → on failure: pending::record_failure(state_dir, generation, classification)
  → reload marker; if newer generation exists, run another cycle
  → release execution lock, exit(0)
```

Key invariants:
- Local mutation is always committed before auto-sync begins.
- Remote failure never rolls back local state.
- Sync-merge writes never trigger auto-sync (prevents feedback loops).
- Explicit manual sync clears pending auto-sync state (prevents duplicates).
- Parent never acquires the worker execution lock — only the detached worker does. The executor subprocess does NOT acquire it either; it invokes `run_sync` directly.
- Scheduling never mutates the pending marker; only `record_pending_mutation` increments generation.

## Exit Codes

- `0` — success (snippet executed/copied, or command completed)
- `1` — any error (all `SnipError` variants map to exit(1))
- `4` — user cancelled TUI interaction (`q`/`Esc`/Ctrl-C in `snp select` only; `run`/`clip`/`search` treat cancellation as exit 0)

## Configuration Files

| File | Purpose |
|------|---------|
| `~/.config/snp/snippets.toml` | Legacy single-file snippet storage |
| `~/.config/snp/libraries.toml` | Library registry (tracks all libraries) |
| `~/.config/snp/libraries/*.toml` | Individual library files |
| `~/.config/snp/premade/*.toml` | Downloaded premade libraries |
| `~/.config/snp/sync.toml` | Sync settings (server URL, API key, direction) |
| `~/.config/snp/themes/<name>.toml` | Custom themes (Halloy format) |
| `~/.config/snp/themes.toml` | Active theme preference |
| `~/.config/snp/usage.toml` | Local usage metadata (use count, last used timestamps) |
| `~/.config/snp/logs/` | Rolling log files (daily rotation) |
| `~/.config/snp/audit.log` | Audit log for snippet operations |
| `~/.config/snp/backups/` | Timestamped library backups (max 10/library) |

# Internal Architecture Inventory

A concise map of the snp internal architecture for contributors working on pet-compatibility features.

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
- 15 command modules, each with a public `run()` entry function
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
- `SnippetSelection` enum — `Selected(idx, copy_flag)`, `Delete(idx)`
- Signal handling: `TERMINATE` atomic flag, registered via `signal-hook` on Unix

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

- `tests/integration.rs` — CLI integration tests (34 tests, `TempDir` + `XDG_CONFIG_HOME` override)
- `tests/pty_integration.rs` — PTY end-to-end tests (10 tests, `portable-pty` crate, runs with `--test-threads=1`)
- `tests/sync_integration.rs` — gRPC integration tests (4 async `#[tokio::test]`, in-process server via `test-helpers` feature)
- Inline `#[cfg(test)]` modules in every source file
- All test data is inline — no fixture files
- Encryption tests verify roundtrip, tamper detection, wrong key rejection
- Sync merge tests cover: server wins, local wins, new snippets, deleted snippets, local-only preservation

## Data Flow: Run Command

```
main.rs::dispatch_command()
  → commands::run_cmd::run()
    → commands::run_snippet_selection()
      → get_library_path() → LibraryManager → resolve .toml path
      → library::load_library() → parse TOML, deduplicate IDs
      → get_snippet_data() → parallel arrays (filter deleted)
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
    → If --sync: sync_commands::run_default_sync()
  → Exit with status code
```

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
| `~/.config/snp/logs/` | Rolling log files (daily rotation) |
| `~/.config/snp/audit.log` | Audit log for snippet operations |
| `~/.config/snp/backups/` | Timestamped library backups (max 10/library) |

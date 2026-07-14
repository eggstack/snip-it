# Architecture Overview

This document provides a bird's-eye view of the snip-it codebase. Each section links to a detailed deep-dive document in this directory.

## Table of Contents

- [CLI & Commands](#cli--commands)
- [Core Data Layer](#core-data-layer)
- [Sync Infrastructure](#sync-infrastructure)
- [TUI & User Interface](#tui--user-interface)
- [Utilities](#utilities)
- [Server (snip-sync)](#server-snip-sync)

---

## CLI & Commands

The CLI is the primary interface for users. The entry point is `src/main.rs` which uses `clap` to define all subcommands.

**Commands** (`src/commands/`):
- [new_cmd.md](commands/new_cmd.md) — Unified snippet creation and exact stdin ingestion
- [list_cmd.md](commands/list_cmd.md) — Text-based snippet listing
- [run_cmd.md](commands/run_cmd.md) — TUI selection + shell execution
- [clip_cmd.md](commands/clip_cmd.md) — Copy snippet to clipboard
- [search_cmd.md](commands/search_cmd.md) — Fuzzy search with detail display
- [edit_cmd.md](commands/edit_cmd.md) — Open snippet config in `$EDITOR`
- [keybindings_cmd.md](commands/keybindings_cmd.md) — TUI keybindings reference
- [sync_cmd.md](commands/sync_cmd.md) — Bidirectional sync with server
- [cron_cmd.md](commands/cron_cmd.md) — Crontab generation for periodic sync
- [register_cmd.md](commands/register_cmd.md) — Device registration
- [library_cmd.md](commands/library_cmd.md) — Library management subcommands
- [premade_cmd.md](commands/premade_cmd.md) — Premade library access
- [mod.md](commands/mod.md) — Shared helpers (path resolution, library loading, snippet expansion)

**Command Patterns**:
- Async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`) initialize the global Tokio runtime on first use
- All commands use `SnipResult<T>` error handling
- Snippet variables (`<name>` or `<name=default>`) are expanded before execution

---

## Core Data Layer

**library.rs** — Snippet and library data structures + persistence

- `Snippet` struct: id, name, command, output, tags, folders, favorite, created_at, updated_at, deleted
- `Snippets` wrapper for TOML serialization
- `LibraryManager` for CRUD operations, backup, migration
- `LibraryMeta` / `LibraryConfig` for multi-library support

**encryption.rs** — End-to-end encryption for sync

- AES-256-GCM with Argon2id key derivation
- `encrypt_snippet()` / `decrypt_snippet()` for safe transmission
- Session-local key cache (`KEY_CACHE`) to avoid re-deriving keys for repeated salts
- `clear_key_cache()` at end of sync operations

**config.rs** — Sync settings

- `SyncSettings`: server URL, API key (keychain), direction (Push/Pull/Bidirectional), interval
- `save_sync_settings()` / `load_sync_settings()`

See [library.md](library.md) for detailed data model and persistence behavior.

---

## Sync Infrastructure

**sync.rs** — gRPC client for snip-sync server

- `SyncClient` wraps the tonic client
- `retry_grpc!` macro for exponential backoff
- Encrypts snippets before push, decrypts after pull

**sync_commands.rs** — Sync orchestration and merge logic

- `run_sync()` handles full bidirectional sync flow
- `merge_snippets()` implements last-write-wins with local-only field preservation
- Server `deleted: true` snippets mark local copies as deleted (data preserved)
- Sync sorts results by `updated_at` descending

See [sync.md](sync.md) for merge strategy details.

---

## TUI & User Interface

Built with `ratatui` + `crossterm`. Single-loop event-driven architecture.

**ui/mod.rs** — Main TUI loop

- `select_snippet_inner()` renders the interactive snippet list
- Fuzzy matching via `SkimMatcherV2` (skim algorithm)
- Debounced filter updates (150ms)
- State: filter, incremental search (`/`), sort mode, tag filter, visual mode

**ui/state.rs** — State types

- `SelectState` — selection index, list state, scroll state
- `FilterState` — sort mode and tag filter text
- `SortMode` — None, Newest, Oldest, AlphaAsc, AlphaDesc
- `is_ctrl_key()` helper

**ui/theme.rs** — Theming

- `Theme` struct: 10-color palette (primary, secondary, accent, background, text, border, selected_bg, muted, string_color, escape_color)
- 50 bundled Halloy TOML themes (LZMA-compressed at build time)
- Theme picker: `e` in normal mode; `j`/`k` preview, `i` filter, `Enter` save
- `DARK_THEME` / `BRIGHT_THEME` built-in fallbacks
- `SNP_THEME` env var or `COLORFGBG` auto-detection (legacy)
- Active theme persisted in `~/.config/snp/themes.toml`

**ui/highlight.rs** — Syntax highlighting for commands

- Variables (`<name>`), shell keywords, strings, flags, comments
- Pre-computed once at startup (not in draw loop)

**ui/variables.rs** — Variable prompt UI

- TUI for entering values for `<name>` or `<name=default>` variables
- Arrow keys/tab navigation, `q` to cancel

See [tui.md](tui.md) for keybindings, state machine, and interaction details.

---

## Utilities

**utils/config.rs** — Path resolution

- `get_config_dir()` → `~/.config/snp/` (XDG-compliant)
- `get_snippets_path()`, `get_sync_config_path()`
- macOS migration from `~/Library/Application Support/snp/`

**utils/variables.rs** — Variable parsing and expansion

- `parse_variables()` extracts `<name>` / `<name=default>` tokens
- `expand_command()` substitutes values
- `strip_escape_sequences()` converts `\<` → `<` and `\>` → `>`

**utils/toml_helpers.rs** — TOML string escape handling

- `fix_invalid_toml_escapes()` converts double-quoted → single-quoted for strings containing `\<` or `\>`
- `quote_strings_containing_backslashes()` reverses on save
- Only handles single-line strings

**utils/shell_keywords.rs** — ~190 shell command names for syntax highlighting

**utils/tempfile_guard.rs** — RAII guard for temporary file cleanup

---

## Server (snip-sync)

Rust gRPC server using `tonic` + `axum` (HTTP).

**snip-sync/src/main.rs** — Server entry

- `SnipSyncService` implements all RPCs from `sync.proto`
- gRPC port + HTTP port (for health/metrics)
- Config via `config.toml` / env vars
- Rate limiting, CORS, Prometheus metrics

**snip-sync/src/db.rs** — SQLite persistence

- In-memory mode for tests (`sqlite::memory:`)
- Tables: `users`, `libraries`, `snippets`
- `migrate_plaintext_api_keys()` for legacy hash backfill

**snip-sync/src/rate_limiter.rs** — In-memory rate limiter

**snip-sync/src/metrics.rs** — Prometheus metrics

- Requests, auth failures, rate limit hits, sync/library operations

**snip-sync/src/premade.rs** — Premade library file scanning

---

## Key Patterns

### Error Handling
- `SnipError` enum in `src/error.rs` with domain-specific variants
- `SnipResult<T> = Result<T, SnipError>`
- Constructor helpers: `io_error()`, `toml_error()`, `clipboard_error()`, `command_error()`, `runtime_error()`

### Async (Tokio)
- Global `RUNTIME: LazyLock<tokio::runtime::Runtime>` initialized lazily
- Only async commands trigger initialization
- `runtime.block_on()` for blocking calls to async gRPC methods

### TOML Handling
- Problem: `\<` and `\>` in double-quoted TOML strings cause parse failures
- Solution in `src/utils/toml_helpers.rs`: convert to single-quoted (raw literals) before parsing, reverse on save
- Triple-quoted strings not handled (acceptable since snippet commands are single-line)

### Sync Merge Strategy
- Last-write-wins based on `updated_at` timestamp
- Server `deleted: true` → local copy marked deleted (preserved)
- Local-only fields (`output`, `folders`, `favorite`) preserved when server wins

### Configuration Files

| Path | Purpose |
|------|---------|
| `~/.config/snp/snippets.toml` | Single-file snippet storage |
| `~/.config/snp/sync.toml` | Sync settings |
| `~/.config/snp/libraries.toml` | Library metadata |
| `~/.config/snp/libraries/*.toml` | Individual library files |
| `~/.config/snp/premade/*.toml` | Downloaded premade libraries |
| `~/.config/snp/logs/` | Rolling log files |
| `~/.config/snp/audit.log` | Audit log |

**Note:** External library paths (`[[external_libraries]]`) are not supported. All snippet libraries reside under `~/.config/snp/libraries/`. See `plans/pet-compat-release-4c-external-libraries.md` for the deferral decision.

### Data Flow: Running a Snippet

1. `snp run` → `main.rs::dispatch_command()` → `commands::run_cmd::run()`
2. `run()` calls `run_snippet_selection()` with `process_snippet` closure
3. `run_snippet_selection()` loads library, calls `ui::select_snippet()` for TUI
4. TUI shows fuzzy-filtered list; user selects snippet
5. `process_snippet()` calls `expand_snippet_command()` → `ui::prompt_variables()` if needed
6. Expanded command executed via `Command::new(shell).arg("-c")`
7. `audit_log()` records the execution
8. On exit (if `--sync`), `sync_commands::run_default_sync()` syncs with server
    
---

## Deep Dives

| File | Subject |
|------|---------|
| [commands/mod.md](commands/mod.md) | Shared command helpers and path resolution |
| [library.md](library.md) | Data structures, persistence, library management |
| [sync.md](sync.md) | Sync protocol, merge logic, conflict resolution |
| [tui.md](tui.md) | TUI architecture, keybindings, state machine |

# Architecture Overview

This document provides a bird's-eye view of the snip-it codebase. Each section links to a detailed deep-dive document in this directory.

For the target logical layer architecture (Domain/Core → Sync-Client → Application), see [docs/LOGICAL_LAYERS.md](../docs/LOGICAL_LAYERS.md).

## Table of Contents

- [Project Structure](#project-structure)
- [CLI & Commands](#cli--commands)
- [Core Data Layer](#core-data-layer)
- [Sync Infrastructure](#sync-infrastructure)
- [TUI & User Interface](#tui--user-interface)
- [Utilities](#utilities)
- [Server (snip-sync)](#server-snip-sync)
- [Key Patterns](#key-patterns)
- [Deep Dives](#deep-dives)

---

## Project Structure

```
snip-it/              Main crate — binary "snp" (src/main.rs)
snip-proto/           Protobuf definitions, tonic-generated gRPC code
snip-sync/            Sync server binary + library (gRPC + HTTP/axum)
tests/                Integration tests (~50 files)
scripts/              build_themes.py, check.sh, release-check.sh, ci/
themes/               50 Halloy TOML theme files
architecture/         This directory — module deep-dive docs
docs/                 Public API, threat model, security audit
premade-libraries/    Premade snippet library files
```

Three workspace crates: `snip-it` (main binary), `snip-proto` (protobuf), `snip-sync` (server).

---

## CLI & Commands

The CLI is the primary interface for users. The entry point is `src/main.rs` which uses `clap` to define 30+ subcommands.

**Entry Point**: [cli.md](cli.md) — CLI dispatch, argument parsing, startup recovery, logging initialization.

**Commands** (`src/commands/`):

| Command | Module | Purpose |
|---------|--------|---------|
| `new` | [new_cmd.md](commands/new_cmd.md) | Snippet creation (arg/stdin/file/editor/multiline) |
| `list` | [list_cmd.md](commands/list_cmd.md) | Text-based snippet listing (JSON/CSV/default) |
| `run` | [run_cmd.md](commands/run_cmd.md) | TUI selection + shell execution |
| `clip` | [clip_cmd.md](commands/clip_cmd.md) | Copy snippet to clipboard |
| `search` | [search_cmd.md](commands/search_cmd.md) | Fuzzy search with detail display |
| `edit` | [edit_cmd.md](commands/edit_cmd.md) | Open snippet in `$EDITOR`, manage output field |
| `select` | [select_cmd.md](commands/select_cmd.md) | Non-executing selection for shell integration |
| `get` | [get_cmd.md](commands/get_cmd.md) | Deterministic non-TUI snippet retrieval |
| `validate` | [validate_cmd.md](commands/validate_cmd.md) | Read-only data validation |
| `repair` | [repair_cmd.md](commands/repair_cmd.md) | Conservative, backed-up, idempotent repair |
| `restore` | [restore_cmd.md](commands/restore_cmd.md) | Restore from backup snapshot |
| `backup` | [backup_cmd.md](commands/backup_cmd.md) | Secret-free backup snapshot |
| `status` | [status_cmd.md](commands/status_cmd.md) | Auto-sync status display |
| `sync` | [sync_cmd.md](commands/sync_cmd.md) | Bidirectional sync, recovery commands |
| `cron` | [cron_cmd.md](commands/cron_cmd.md) | Crontab generation for periodic sync |
| `register` | [register_cmd.md](commands/register_cmd.md) | Device registration with sync server |
| `library` | [library_cmd.md](commands/library_cmd.md) | Library management subcommands |
| `premade` | [premade_cmd.md](commands/premade_cmd.md) | Browse/download premade libraries |
| `import` | [import_cmd.md](commands/import_cmd.md) | Pet snippet file import |
| `doctor` | [doctor_cmd.md](commands/doctor_cmd.md) | Compatibility diagnostics, pet analysis |
| `shell` | [shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation (bash/zsh/fish) |
| `keybindings` | [keybindings_cmd.md](commands/keybindings_cmd.md) | TUI keybindings reference |

**Shared Helpers**: [commands/mod.md](commands/mod.md) — path resolution, library loading, snippet expansion.

**Pet Analysis**: [pet_analysis.md](commands/pet_analysis.md) — pet file reading, field detection, import analysis.

**Command Patterns**:
- Async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`) initialize the global Tokio runtime on first use
- All commands use `SnipResult<T>` error handling
- Snippet variables (`<name>` or `<name=default>`) are expanded before execution

---

## Core Data Layer

**Core Types**: [core.md](core.md) — error handling, key abstractions, `SnipError` enum.

| Module | File | Purpose |
|--------|------|---------|
| `library` | [library.md](library.md) | `Snippet`, `Snippets`, `LibraryManager` — data structures and TOML persistence |
| `encryption` | [encryption.md](encryption.md) | AES-256-GCM + Argon2id end-to-end encryption |
| `config` | [config.md](config.md) | `SyncSettings`, API key keychain storage, CRC32 integrity |
| `selector` | [selector.md](selector.md) | `SnippetSelector` — deterministic non-TUI snippet resolution |
| `outcome` | [outcome.md](outcome.md) | `CliOutcome` — exit codes, machine output |
| `sort` | [sort.md](sort.md) | `SnippetSort` — 6 sort modes, 5-level tie-break chain |
| `usage` | [usage.md](usage.md) | `UsageIndex` — persistent per-snippet usage metadata |
| `output` | [output.md](output.md) | `OutputPresentation` — safe output field rendering |
| `transaction` | [persistence.md](persistence.md) | Transaction boundary with journal, lock, begin/commit/rollback |
| `migration` | [persistence.md](persistence.md) | Schema versioning (`SchemaVersion`), migration operations |
| `local_data` | [persistence.md](persistence.md) | Short-lived exclusive lock serializing TOML mutations |

---

## Sync Infrastructure

| Module | File | Purpose |
|--------|------|---------|
| `sync` | [sync.md](sync.md) | `SyncClient` (tonic gRPC), `retry_grpc!` macro, exponential backoff |
| `sync_commands` | [sync.md](sync.md) | Bidirectional sync orchestration, merge logic, conflict resolution |
| `auto_sync` | [auto_sync.md](auto_sync.md) | Single detached-helper model, debounce, scheduling, execution lock |
| `status_snapshot` | [status.md](status.md) | `StatusSnapshot`, `TopLevelSyncState` (8 variants), diagnostic codes |
| `proto` | [proto.md](proto.md) | Protobuf definitions, gRPC service spec (`SnippetSync` — 11 RPCs) |

**Auto-Sync Model** (see [auto_sync.md](auto_sync.md)):
- Detached worker (`snp auto-sync-worker`) holds `SyncExecutionLock` for the entire cycle
- Parent never holds the worker lock
- `schedule_sync()` is the sole scheduling authority
- Pending generations are monotonic; lower generation = corrupt state

**Merge Strategy** (see [sync.md](sync.md)):
- Last-write-wins based on `updated_at` timestamp
- Server `deleted: true` → local copy marked deleted (preserved)
- Local-only fields (`output`, `folders`, `favorite`) preserved when server wins

---

## TUI & User Interface

Built with `ratatui` + `crossterm`. Single-loop event-driven architecture.

| Module | File | Purpose |
|--------|------|---------|
| Main loop | [tui.md](tui.md) | Event loop, fuzzy search (`SkimMatcherV2`), keyboard navigation |
| Components | [ui.md](ui.md) | UI components, rendering, theme system |
| State | [tui.md](tui.md) | `SelectState`, `FilterState`, `SortMode` (TUI-internal) |
| Theme | [ui.md](ui.md) | `Theme` struct (10-color palette), 50 bundled Halloy themes |
| Highlight | [tui.md](tui.md) | Syntax highlighting (variables, shell keywords, strings, flags) |
| Variables | [tui.md](tui.md) | TUI for `<name>` / `<name=default>` variable prompts |
| Sort | [sort.md](sort.md) | `SnippetSort` — Relevance, Recent, LastUsed, MostUsed, Description, Command |
| Usage | [usage.md](usage.md) | `UsageIndex` — persistent per-snippet use count + last-used timestamps |

---

## Utilities

| Module | File | Purpose |
|--------|------|---------|
| `config` | [utils/config.md](utils/config.md) | Path resolution: `get_config_dir()`, XDG, macOS migration |
| `variables` | [utils/variables.md](utils/variables.md) | `parse_variables()`, `expand_command()`, `strip_escape_sequences()` |
| `toml_helpers` | [utils/toml_helpers.md](utils/toml_helpers.md) | TOML backslash escape handling (`\<`/`\>` in double-quoted strings) |
| `shell_keywords` | [utils/shell_keywords.md](utils/shell_keywords.md) | ~190 shell command names for syntax highlighting |
| `tempfile_guard` | [utils/tempfile_guard.md](utils/tempfile_guard.md) | RAII guard for temporary file cleanup |
| `atomic` | [utils/atomic.md](utils/atomic.md) | `write_private_atomic()`, `atomic_replace()` — durability-aware atomic writes |

**Cross-cutting utilities** (see [utils.md](utils.md) for full inventory):
- [clipboard.md](clipboard.md) — cross-platform clipboard access (arboard/clipboard-win)
- [logging.md](logging.md) — structured logging (`tracing`), audit trail, panic handler

---

## Server (snip-sync)

Rust gRPC server using `tonic` + `axum` (HTTP). See [server.md](server.md).

| Module | Purpose |
|--------|---------|
| `main.rs` | Server entry, CLI (serve/init/cert/edit/stop/restart/update) |
| `lib.rs` | `SnipSyncService` implementing all gRPC RPCs, `Config` |
| `db.rs` | SQLite persistence (users/libraries/snippets), Argon2id API key hashing |
| `rate_limiter.rs` | In-memory rate limiting |
| `metrics.rs` | Prometheus metrics (requests, auth failures, sync operations) |
| `premade.rs` | Premade library file scanning |
| `server_lock.rs` | Kernel-backed server singleton lock |
| `cert.rs` | TLS certificate generation |

**Protocol**: [proto.md](proto.md) — single `SnippetSync` service with 11 RPCs (GetSnippets, PushSnippets, Sync, Health, Register, library CRUD, premade access).

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

### Persistence
- Atomic writes via `utils/atomic.rs` with `TempFileGuard` for cleanup
- Transaction journaling for multi-file mutations
- Backup/restore with SHA-256 integrity verification

### Configuration Files

| Path | Purpose |
|------|---------|
| `~/.config/snp/snippets.toml` | Single-file snippet storage (legacy) |
| `~/.config/snp/libraries.toml` | Library metadata + sync links |
| `~/.config/snp/libraries/*.toml` | Individual library files |
| `~/.config/snp/premade/*.toml` | Downloaded premade libraries |
| `~/.config/snp/sync.toml` | Sync settings (CRC32 integrity header) |
| `~/.config/snp/themes/*.toml` | Halloy-compatible theme files |
| `~/.config/snp/themes.toml` | Active theme selection |
| `~/.config/snp/usage.toml` | Local usage metadata (not synced) |
| `~/.config/snp/auto-sync-status.toml` | Durable sync status |
| `~/.config/snp/auto-sync-pending.toml` | Pending mutation marker |
| `~/.config/snp/logs/` | Rolling log files |
| `~/.config/snp/audit.log` | Audit trail |
| `~/.config/snp/transaction-journals/` | Transaction journals |
| `~/.config/snp/backups/` | Backup snapshots |

**Note:** External library paths are not supported. All snippet libraries reside under `~/.config/snp/libraries/`.

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

### CLI & Commands

| File | Subject |
|------|---------|
| [cli.md](cli.md) | CLI entry point, argument parsing, dispatch, startup recovery |
| [commands/mod.md](commands/mod.md) | Shared command helpers and path resolution |
| [commands/new_cmd.md](commands/new_cmd.md) | Snippet creation (arg/stdin/file/editor/multiline) |
| [commands/list_cmd.md](commands/list_cmd.md) | Text-based snippet listing |
| [commands/run_cmd.md](commands/run_cmd.md) | TUI selection + shell execution |
| [commands/clip_cmd.md](commands/clip_cmd.md) | Copy snippet to clipboard |
| [commands/search_cmd.md](commands/search_cmd.md) | Fuzzy search with detail display |
| [commands/edit_cmd.md](commands/edit_cmd.md) | Open snippet in `$EDITOR` |
| [commands/select_cmd.md](commands/select_cmd.md) | Non-executing selection for shell integration |
| [commands/get_cmd.md](commands/get_cmd.md) | Deterministic non-TUI snippet retrieval |
| [commands/validate_cmd.md](commands/validate_cmd.md) | Read-only data validation |
| [commands/repair_cmd.md](commands/repair_cmd.md) | Conservative, backed-up, idempotent repair |
| [commands/restore_cmd.md](commands/restore_cmd.md) | Restore from backup snapshot |
| [commands/backup_cmd.md](commands/backup_cmd.md) | Secret-free backup snapshot |
| [commands/status_cmd.md](commands/status_cmd.md) | Auto-sync status display |
| [commands/sync_cmd.md](commands/sync_cmd.md) | Sync and config subcommands |
| [commands/cron_cmd.md](commands/cron_cmd.md) | Crontab generation for periodic sync |
| [commands/register_cmd.md](commands/register_cmd.md) | Device registration |
| [commands/library_cmd.md](commands/library_cmd.md) | Library management subcommands |
| [commands/premade_cmd.md](commands/premade_cmd.md) | Premade library access |
| [commands/import_cmd.md](commands/import_cmd.md) | Pet snippet file import |
| [commands/doctor_cmd.md](commands/doctor_cmd.md) | Diagnostics, pet analysis, environment audit |
| [commands/shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation (bash/zsh/fish) |
| [commands/keybindings_cmd.md](commands/keybindings_cmd.md) | TUI keybindings reference |
| [commands/pet_analysis.md](commands/pet_analysis.md) | Pet file reading, field detection, import analysis |

### Core Data

| File | Subject |
|------|---------|
| [core.md](core.md) | Core types, error handling, key abstractions |
| [library.md](library.md) | Data structures, persistence, library management |
| [config.md](config.md) | Sync settings, path resolution, keychain API key |
| [encryption.md](encryption.md) | AES-256-GCM end-to-end encryption |
| [selector.md](selector.md) | Deterministic non-TUI snippet resolution |
| [outcome.md](outcome.md) | `CliOutcome` — exit codes, machine output |

### Sync

| File | Subject |
|------|---------|
| [sync.md](sync.md) | Sync protocol, merge logic, conflict resolution |
| [auto_sync.md](auto_sync.md) | Auto-sync policy, worker, debounce, triggers |
| [status.md](status.md) | Status snapshot, `snp status` command, diagnostic codes |
| [proto.md](proto.md) | Protobuf definitions, gRPC service spec |

### UI

| File | Subject |
|------|---------|
| [tui.md](tui.md) | TUI architecture, keybindings, state machine |
| [ui.md](ui.md) | UI components, rendering, theme system |

### Server

| File | Subject |
|------|---------|
| [server.md](server.md) | snip-sync server architecture, gRPC/HTTP, database |

### Utilities

| File | Subject |
|------|---------|
| [utils.md](utils.md) | Utility module inventory |
| [utils/config.md](utils/config.md) | Config directory resolution, path helpers, macOS migration |
| [utils/variables.md](utils/variables.md) | Variable parsing and expansion |
| [utils/toml_helpers.md](utils/toml_helpers.md) | TOML escape sequence handling |
| [utils/shell_keywords.md](utils/shell_keywords.md) | Shell command names for syntax highlighting |
| [utils/tempfile_guard.md](utils/tempfile_guard.md) | RAII temporary file cleanup |
| [utils/atomic.md](utils/atomic.md) | Atomic file writes with durability guarantees |
| [persistence.md](persistence.md) | Atomic writes, transactions, validation, backup/restore/repair |
| [clipboard.md](clipboard.md) | Cross-platform clipboard access |
| [sort.md](sort.md) | Sort modes, ranking, tie-break chain |
| [usage.md](usage.md) | Local usage metadata, update policy, storage |
| [output.md](output.md) | Snippet output field rendering |
| [logging.md](logging.md) | Structured logging, audit trail, panic handler |

### Testing

| File | Subject |
|------|---------|
| [test-infrastructure.md](test-infrastructure.md) | Deterministic E2E test infrastructure |

> **Note**: See `docs/ARCHITECTURE_INVENTORY.md` for a comprehensive module inventory.

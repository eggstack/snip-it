# Architecture Overview

Bird's-eye view of the snip-it codebase. Each section summarizes a discrete
module or component and links to a dedicated deep-dive document for full
detail.

## Table of Contents

- [Project Layout](#project-layout)
- [Workspace Crates](#workspace-crates)
- [CLI & Command Dispatch](#cli--command-dispatch)
- [Command Modules](#command-modules)
- [Core Data Layer](#core-data-layer)
- [Sync Client](#sync-client)
- [Auto-Sync Subsystem](#auto-sync-subsystem)
- [Server (snip-sync)](#server-snip-sync)
- [Protocol (snip-proto)](#protocol-snip-proto)
- [TUI & User Interface](#tui--user-interface)
- [Utilities & Cross-Cutting Concerns](#utilities--cross-cutting-concerns)
- [Persistence & Durability](#persistence--durability)
- [Testing Infrastructure](#testing-infrastructure)
- [Configuration Files](#configuration-files)
- [Data Flow: Running a Snippet](#data-flow-running-a-snippet)
- [Key Patterns](#key-patterns)
- [Deep-Dive Index](#deep-dive-index)

---

## Project Layout

```
snip-it/              Main crate — binary "snp"
  src/                Application source (CLI, commands, TUI, sync, core)
  tests/              Integration tests (~49 files)
  architecture/       This directory — module deep-dive docs
  docs/               Public API docs, threat model, security audit
  .skills/            Specialized agent reference docs
snip-proto/           Protobuf definitions, tonic-generated gRPC code
snip-sync/            Sync server (gRPC + HTTP/axum) + library crate
scripts/              Build helpers, CI scripts, theme bundler
themes/               50 Halloy TOML theme files
premade-libraries/    Premade snippet library files
```

---

## Workspace Crates

| Crate | Type | Purpose |
|-------|------|---------|
| `snip-it` | Binary (`snp`) + library | Main application: CLI, TUI, sync client, core data model |
| `snip-proto` | Library | Protobuf definitions and tonic-generated gRPC stubs |
| `snip-sync` | Binary + library | Self-hosted sync server (gRPC + HTTP, SQLite, TLS) |

The library surface of `snip-it` exposes a stable public API (`Snippet`,
`Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library`,
atomic write utilities, `SnipError`, `SnippetSort`, `SyncSettings`, etc.).
Everything else (`commands`, `ui`, `auto_sync`, `sync`, `encryption`,
`logging`, `process_file_lock`, `selector`, `usage`) is
`#[doc(hidden)]` — public for binary/integration-test access but not part
of the supported external API.

---

## CLI & Command Dispatch

**Source**: `src/main.rs`
**Deep dive**: [cli.md](cli.md)

Entry point using `clap` with 30+ subcommands. A global `LazyLock<Runtime>`
provides Tokio only when an async command is invoked (`run`, `clip`, `search`,
`sync`, `register`, `premade`). Signal handlers are registered on Unix
(SIGINT + SIGTERM).

Command dispatch flows through `dispatch_command()` which maps each CLI
variant to its command module. `command_behavior()` determines both startup
recovery policy and logging/audit service level in a single match:
read-only commands suppress recovery, mutation commands allow it, sync
commands manage their own behavior.

**Exit codes** (stable): 0 success, 1 general error, 2 usage error,
3 not found, 4 cancelled, 5 ambiguous, 6 validation, 7 sync failure,
8 execution failure, 9 conflict/refused.

---

## Command Modules

**Source**: `src/commands/` (24 modules)
**Deep dives**: [commands/mod.md](commands/mod.md) and per-command files

| Command | Module | Purpose |
|---------|--------|---------|
| `new` | [new_cmd.md](commands/new_cmd.md) | Create snippets (arg/stdin/file/editor/multiline) |
| `list` | [list_cmd.md](commands/list_cmd.md) | Text listing (JSON/CSV/default) |
| `run` | [run_cmd.md](commands/run_cmd.md) | TUI selection + shell execution |
| `clip` | [clip_cmd.md](commands/clip_cmd.md) | Copy snippet to clipboard |
| `search` | [search_cmd.md](commands/search_cmd.md) | Fuzzy search with detail display |
| `edit` | [edit_cmd.md](commands/edit_cmd.md) | Open in `$EDITOR`, manage output field |
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
| `doctor` | [doctor_cmd.md](commands/doctor_cmd.md) | Diagnostics, pet analysis, environment audit |
| `shell` | [shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation (bash/zsh/fish) |
| `keybindings` | [keybindings_cmd.md](commands/keybindings_cmd.md) | TUI keybindings reference |
| `completions` | — | Shell completion generation (clap_complete) |
| `update` | — | Self-update via current installation method |
| `data` | — | Advanced data maintenance subgroup (validate/backup/restore/repair/status) |

**Shared helpers**: [commands/mod.md](commands/mod.md) — path resolution,
library loading, snippet expansion, output writing.

**Pet analysis**: [commands/pet_analysis.md](commands/pet_analysis.md) —
pet file reading, field detection, import analysis.

---

## Core Data Layer

| Module | Source | Deep Dive | Purpose |
|--------|--------|-----------|---------|
| `library` | `src/library.rs` | [library.md](library.md) | `Snippet`, `Snippets`, `LibraryManager` — data structures and TOML persistence |
| `error` | `src/error.rs` | [core.md](core.md) | `SnipError` enum, `SnipResult<T>`, `SyncFailureKind` |
| `config` | `src/config.rs` | [config.md](config.md) | `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode`, keychain API key |
| `encryption` | `src/encryption.rs` | [encryption.md](encryption.md) | AES-256-GCM + Argon2id end-to-end encryption |
| `selector` | `src/selector.rs` | [selector.md](selector.md) | `SnippetSelector` — deterministic non-TUI snippet resolution |
| `outcome` | `src/outcome.rs` | [outcome.md](outcome.md) | `CliOutcome` — exit codes, machine output |
| `sort` | `src/sort.rs` | [sort.md](sort.md) | `SnippetSort` — 6 sort modes, 5-level tie-break chain |
| `usage` | `src/usage.rs` | [usage.md](usage.md) | `UsageIndex` — persistent per-snippet usage metadata |
| `output` | `src/output.rs` | [output.md](output.md) | `OutputPresentation` — safe output field rendering |
| `migration` | `src/migration.rs` | [persistence.md](persistence.md) | Schema versioning (`SchemaVersion`), migration operations |
| `transaction` | `src/transaction.rs` | [persistence.md](persistence.md) | Transaction journal, lock, begin/commit/rollback state machine |
| `local_data` | `src/local_data.rs` | [persistence.md](persistence.md) | Short-lived exclusive lock serializing TOML mutations |
| `diagnostics` | `src/diagnostics.rs` | — | Internal diagnostics |
| `test_failpoints` | `src/test_failpoints.rs` | — | Test-only failpoint hooks (compiled with `test-support`) |

---

## Sync Client

**Source**: `src/sync.rs`, `src/sync_commands.rs`
**Deep dive**: [sync.md](sync.md)

The sync client communicates with the snip-sync server over gRPC (tonic)
with TLS. Snippets are encrypted with the user's API key before
transmission (AES-256-GCM).

**Key components**:
- `SyncClient` — tonic gRPC client with exponential backoff retries
- `retry_grpc!` macro — configurable retry with jitter
- `sync_encrypted()` — byte-bounded upload batches (3.5 MiB ceiling)
- `sync_commands::run_sync()` — full bidirectional sync orchestration

**Merge strategy**:
- Live conflicts: `(updated_at, device_id, SHA-256(synced fields))`
- Deletions win over live content (no-resurrection)
- Local-only fields (`output`, `folders`, `favorite`) preserved

**Failure classification**: `SyncFailureKind` (21 variants) maps to
`FailureClass` (4 variants: Transient, Configuration, LocalFailure, Internal).

---

## Auto-Sync Subsystem

**Source**: `src/auto_sync/` (11 modules)
**Deep dive**: [auto_sync.md](auto_sync.md)

Single detached-helper model. A background worker (`snp auto-sync-worker`)
runs the canonical sync operation after local mutations.

| Module | Purpose |
|--------|---------|
| `execution_lock.rs` | `SyncExecutionLock`, `WorkerLock` — kernel-backed exclusive ownership (includes merged worker lock and `spawn_worker`) |
| `lock.rs` | Worker lock re-exports from `execution_lock` for backward compatibility |
| `worker.rs` | Detached worker entry point, holds lock for entire cycle |
| `notification.rs` | Mutation notification, pending marker creation, startup recovery |
| `pending.rs` | `PendingState` — on-disk pending mutation marker |
| `pending_lock.rs` | Transaction-scoped pending marker lock |
| `policy.rs` | `AutoSyncPolicy`, `FailureClass`, `MutationKind`, retry disposition |
| `schedule.rs` | Debounce scheduling, `schedule_sync()` — sole scheduling authority |
| `status.rs` | `StatusSnapshot`, `TopLevelSyncState` (8 variants), diagnostic codes |
| `test_events.rs` | Test-only lifecycle event emission (JSON-lines) |

**Key invariants**:
- Parent never holds the worker lock
- `schedule_sync()` is the sole scheduling authority
- Pending generations are monotonic; lower generation = corrupt state
- Local mutations always commit before remote work

---

## Server (snip-sync)

**Source**: `snip-sync/src/` (17 modules)
**Deep dive**: [server.md](server.md)

Self-hosted Rust gRPC server using tonic + axum (HTTP).

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
| `orchestration.rs` | Service lifetime, graceful shutdown |
| `bootstrap.rs` | Server initialization |
| `cli.rs` | CLI argument parsing |
| `paths.rs` | Path resolution for server state |
| `process.rs` | Legacy PID parsing and stop/restart compatibility |
| `editor.rs` | Server-side config editing |
| `update.rs` | Server update command and package-manager integration |
| `test_helpers.rs` | Test-only helpers (gated on `test-helpers` feature) |
| `test_observer.rs` | Test-only event capture |

---

## Protocol (snip-proto)

**Source**: `snip-proto/proto/sync.proto`, `snip-proto/src/`
**Deep dive**: [proto.md](proto.md)

Single `SnippetSync` gRPC service with 11 RPCs:

| RPC | Purpose |
|-----|---------|
| `GetSnippets` | Fetch non-deleted snippets updated after a timestamp |
| `PushSnippets` | Upload local snippets (idempotent upsert) |
| `Sync` | Bidirectional merge: upload local, download remote changes |
| `Health` | Server health check |
| `Register` | Device/account registration |
| `CreateLibrary` | Create a new library on the server |
| `ListLibraries` | List account libraries |
| `DeleteLibrary` | Delete a library |
| `ListPremadeLibraries` | List available premade libraries |
| `GetPremadeLibrary` | Download a premade library |
| `SearchPremadeLibraries` | Search premade libraries by query |

---

## TUI & User Interface

**Source**: `src/ui/` (6 files)
**Deep dives**: [tui.md](tui.md), [ui.md](ui.md)

Built with `ratatui` + `crossterm`. Single-loop event-driven architecture.

| Module | File | Purpose |
|--------|------|---------|
| Main loop | `mod.rs` | Event loop, fuzzy search (`SkimMatcherV2`), keyboard navigation |
| State | `state.rs` | `SelectState`, `FilterState`, sort mode (TUI-internal) |
| Theme | `theme.rs` | `Theme` struct (10-color palette), 50 bundled Halloy themes |
| Highlight | `highlight.rs` | Syntax highlighting (variables, shell keywords, strings, flags) |
| Variables | `variables.rs` | TUI for `<name>` / `<name=default>` variable prompts |
| Bundled themes | `_generated_bundled_themes.rs` | Generated explicitly by `scripts/build_themes.py`; checked-in source is used by normal builds |

---

## Utilities & Cross-Cutting Concerns

| Module | Source | Deep Dive | Purpose |
|--------|--------|-----------|---------|
| `utils/config` | `src/utils/config.rs` | [utils/config.md](utils/config.md) | Path resolution: `get_config_dir()`, XDG, macOS migration |
| `utils/variables` | `src/utils/variables.rs` | [utils/variables.md](utils/variables.md) | `parse_variables()`, `expand_command()`, `strip_escape_sequences()` |
| `utils/toml_helpers` | `src/utils/toml_helpers.rs` | [utils/toml_helpers.md](utils/toml_helpers.md) | TOML backslash escape handling (`\<`/`\>` in double-quoted strings) |
| `utils/shell_keywords` | `src/utils/shell_keywords.rs` | [utils/shell_keywords.md](utils/shell_keywords.md) | ~190 shell command names for syntax highlighting |
| `utils/tempfile_guard` | `src/utils/tempfile_guard.rs` | [utils/tempfile_guard.md](utils/tempfile_guard.md) | RAII guard for temporary file cleanup |
| `utils/atomic` | `src/utils/atomic.rs` | [utils/atomic.md](utils/atomic.md) | `write_private_atomic()`, `atomic_replace()` — durability-aware atomic writes |
| `clipboard` | `src/clipboard.rs` | [clipboard.md](clipboard.md) | Cross-platform clipboard (arboard/clipboard-win) |
| `logging` | `src/logging.rs` | [logging.md](logging.md) | Structured logging (`tracing`), audit trail, panic handler |
| `process_file_lock` | `src/process_file_lock.rs` | — | Kernel-backed cross-process file lock (`flock`/`LockFileEx`) |
| `status_snapshot` | `src/status_snapshot.rs` | [status.md](status.md) | Status snapshot and diagnostic codes |
| `update` | `src/update.rs` | — | Cargo/Homebrew update checking and installation |

Full utility inventory: [utils.md](utils.md).

---

## Persistence & Durability

**Deep dive**: [persistence.md](persistence.md)

- **Atomic writes**: `utils/atomic.rs` with `TempFileGuard` for cleanup.
  Durability classes: `DurableUserData` (fsync file+dir),
  `SensitiveConfig` (0o600, symlink reject), `RecoverableMetadata` (no
  fsync), `EphemeralCoordination` (no fsync, no dir sync).
- **Transaction journaling**: `transaction.rs` — `Prepared → Committing →
  CleaningUp` state machine for multi-file mutations. Journals persist to
  disk so interrupted operations can be recovered on startup.
- **Local data lock**: `local_data.rs` — exclusive lock serializing TOML
  mutations against backup snapshot capture.
- **Schema migration**: `migration.rs` — `SchemaVersion` ordinal type with
  forward-only migration operations.
- **Backup/restore**: SHA-256 integrity verification, secret-free snapshots,
  merge/replace restore modes.
- **Mutation gate**: `gate_mutation_on_interrupted_transactions()` must be
  called before any local mutating operation. Single journal = auto-rollback;
  multiple/incomplete = refuse and direct to `snp repair`.

---

## Testing Infrastructure

**Deep dive**: [test-infrastructure.md](test-infrastructure.md)

~49 integration test files in `tests/`. Reusable components in
`tests/support/`: `TestEnvironment` (isolated TempDir), `RecordingServer`,
`EventSink`.

| Class | Execution | Targets |
|-------|-----------|---------|
| Unit/pure | parallel | `cargo test --workspace --lib` |
| CLI/platform smoke | parallel | `platform_smoke.rs`, `local_contracts.rs` |
| Restore contracts | parallel | `destination_permissions.rs`, `backup_contracts.rs` |
| Auto-sync contracts | parallel | `auto_sync_closure.rs`, `sync_contracts.rs`, `debounce_matrix.rs` |
| Sync integration | serial | `sync_integration.rs` — in-process server, random port |
| PTY | serial | `pty_integration.rs` — real terminal pairs |
| Cross-process lock | serial | `process_lock_concurrency.rs` — kernel flock |
| Barrier-coordinated | serial | `local_data_lock_barriers.rs`, `repair_transactions.rs` |
| Deep recovery | manual | `transaction_crash_recovery.rs`, failpoint tests |
| Release smoke | manual | `release-check.sh` Phase 3 — `manifest_contracts.rs`, crash, production seams |
| Architecture | parallel | `architecture.rs` — source-scanning layer boundary enforcement |

---

## Configuration Files

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
| `~/.config/snp/auto-sync-status.toml` | Durable sync status (not synced) |
| `~/.config/snp/auto-sync-pending.toml` | Pending mutation marker |
| `~/.config/snp/transaction-journals/` | Transaction journals |
| `~/.config/snp/backups/` | Backup snapshots |
| `~/.config/snp/logs/` | Rolling log files |
| `~/.config/snp/audit.log` | Audit trail |

External library paths are not supported. All snippet libraries reside
under `~/.config/snp/libraries/`.

---

## Data Flow: Running a Snippet

```
snp run [--filter FOO] [--sync]
  │
  ├─ main.rs::dispatch_command()
  │    └─ commands::run_cmd::run()
  │         ├─ load library via LibraryManager
  │         ├─ ui::select_snippet()  ← TUI (ratatui + crossterm)
  │         │    ├─ fuzzy filter (SkimMatcherV2)
  │         │    ├─ keyboard navigation
  │         │    └─ variable prompting if needed
  │         ├─ expand_snippet_command()  → shell expansion
  │         ├─ Command::new(shell).arg("-c").arg(cmd)  → execute
  │         ├─ audit_log()  → structured log
  │         └─ if --sync: sync_commands::run_default_sync()
  │              └─ sync::sync_encrypted()  → gRPC bidirectional merge
  │
  └─ on local mutation: auto_sync::notify_mutation()
       └─ spawn_worker()  → detached background sync
```

---

## Key Patterns

### Error Handling
- `SnipError` enum with domain-specific variants: `Io`, `Toml`, `Clipboard`,
  `Command`, `Runtime`, `SyncFailure`
- Constructor helpers: `io_error()`, `toml_error()`, `clipboard_error()`,
  `command_error()`, `runtime_error()`, `sync_failure()`
- `SyncFailureKind` (21 variants) for typed sync failure classification

### Async (Tokio)
- Global `RUNTIME: LazyLock<Runtime>` — only initialized by async commands
- `runtime.block_on()` for blocking calls to async gRPC methods
- Detached auto-sync worker creates its own multi-thread runtime

### TOML Handling
- `\<` and `\>` in double-quoted TOML strings cause parse failures
- Solution: convert to single-quoted (raw literals) before parsing, reverse
  on save — implemented in `utils/toml_helpers.rs`

### Encryption
- AES-256-GCM for snippet payload encryption
- Argon2id for key derivation from API key
- Key cache for repeated operations
- See [encryption.md](encryption.md) and [architecture/encryption.md](encryption.md)

### Process Locks
- Kernel-backed (`flock` Unix / `LockFileEx` Windows) for:
  - Server singleton (`server_lock.rs`)
  - Auto-sync execution lock (`execution_lock.rs`)
  - Transaction lock (`transaction.rs`)
  - Local data lock (`local_data.rs`)
- `Drop` releases the lock; persistent files may contain stale metadata

---

## Deep-Dive Index

### CLI & Commands

| File | Subject |
|------|---------|
| [cli.md](cli.md) | CLI entry point, argument parsing, dispatch, startup recovery |
| [commands/mod.md](commands/mod.md) | Shared command helpers and path resolution |
| [commands/new_cmd.md](commands/new_cmd.md) | Snippet creation |
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
| [commands/shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation |
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
| [sort.md](sort.md) | Sort modes, ranking, tie-break chain |
| [usage.md](usage.md) | Local usage metadata, update policy, storage |
| [output.md](output.md) | Snippet output field rendering |

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
| [utils/config.md](utils/config.md) | Config directory resolution, path helpers |
| [utils/variables.md](utils/variables.md) | Variable parsing and expansion |
| [utils/toml_helpers.md](utils/toml_helpers.md) | TOML escape sequence handling |
| [utils/shell_keywords.md](utils/shell_keywords.md) | Shell command names for syntax highlighting |
| [utils/tempfile_guard.md](utils/tempfile_guard.md) | RAII temporary file cleanup |
| [utils/atomic.md](utils/atomic.md) | Atomic file writes with durability guarantees |
| [persistence.md](persistence.md) | Atomic writes, transactions, validation, backup/restore/repair |
| [clipboard.md](clipboard.md) | Cross-platform clipboard access |
| [logging.md](logging.md) | Structured logging, audit trail, panic handler |

### Testing

| File | Subject |
|------|---------|
| [test-infrastructure.md](test-infrastructure.md) | Deterministic E2E test infrastructure |

### Reference

| File | Subject |
|------|---------|
| [review_plan.md](review_plan.md) | Architecture review process (historical) |
| `../docs/LOGICAL_LAYERS.md` | Target logical layer architecture |
| `../docs/ARCHITECTURE_INVENTORY.md` | Comprehensive module inventory |

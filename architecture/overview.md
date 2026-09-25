# Architecture Overview

Bird's-eye view of the snip-it codebase. Each section summarizes a discrete
module or component and links to a dedicated deep-dive document for full
detail.

## Table of Contents

- [Project Layout](#project-layout)
- [Workspace Crates](#workspace-crates)
- [Logical Layers](#logical-layers)
- [CLI & Command Dispatch](#cli--command-dispatch)
- [Command Modules](#command-modules)
- [Core Data Layer](#core-data-layer)
- [Sync Client](#sync-client)
- [Auto-Sync Subsystem](#auto-sync-subsystem)
- [Server (snip-sync)](#server-snip-sync)
- [Protocol (snip-proto)](#protocol-snip-proto)
- [TUI & User Interface](#tui--user-interface)
- [MCP (Model Context Protocol)](#mcp-model-context-protocol)
- [Self-Update](#self-update)
- [Diagnostics](#diagnostics)
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
snip-it/              Main crate — binary "snp" (+ lib for binary/test sharing)
  src/                Application source (CLI, commands, TUI, sync, core)
  tests/              Integration tests (~50 targets + shared support/)
  architecture/       This directory — module deep-dive docs
  docs/               Public refs (EXIT_CODES, PERSISTENCE_INVENTORY, THREAT_MODEL,
                      COMMAND_CONTRACTS, LOGICAL_LAYERS, ARCHITECTURE_INVENTORY, ...)
  .skills/            Specialized agent reference docs (sync, transactions, server, ...)
  plans/              Planning registry + roadmaps (see plans/registry.md)
snip-proto/           Protobuf definitions + checked-in tonic stubs (proto/sync.proto)
snip-sync/            Sync server (Tonic gRPC + EggServe HTTP/1 leaf runtime, SQLite)
scripts/              check.sh, release-check.sh, build_themes.py, installers
themes/               50 Halloy TOML theme files (bundled via build_themes.py)
premade-libraries/    Premade snippet library files
packaging/            Dist packaging; demo/ + assets/ hold demos/screenshots
```

---

## Workspace Crates

| Crate | Type | Purpose |
|-------|------|---------|
| `snip-it` | Binary (`snp`) + library | Main application: CLI, TUI, sync client, core data model |
| `snip-proto` | Library (v0.1.3) | Protobuf definitions and checked-in tonic-generated gRPC stubs |
| `snip-sync` | Binary + library (v0.1.6) | Self-hosted sync server (gRPC + HTTP, SQLite, TLS, metrics) |

The library surface of `snip-it` exposes a stable public API (`Snippet`,
`Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library`,
atomic write utilities, `SnipError`, `SnippetSort`, `SyncSettings`, etc.).
Everything else (`commands`, `ui`, `auto_sync`, `sync`, `logging`,
`process_file_lock`, `selector`, `usage`, `mcp`) is `#[doc(hidden)]` — public for
binary/integration-test access but not part of the supported external API.
Modules such as `library`, `encryption`, `clipboard`, `diagnostics`, `output`,
`migration`, `local_data`, `sync_commands`, `sync_failure`, `status_snapshot`,
`transaction`, and `utils` are `pub(crate)` (crate-internal only).

Publish order is manual local: `snip-proto` → `snip-sync` → `snip-it`;
proto changes require bumping its version in both dependents.

---

## Logical Layers

Source modules are organized into three logical layers with a strict
dependency direction, documented in [`../docs/LOGICAL_LAYERS.md`](../docs/LOGICAL_LAYERS.md)
and enforced by source-scanning tests in `tests/architecture.rs`:

```
┌─────────────────────────────────────────────┐
│           Application / CLI Layer           │
│  main.rs, commands/*, ui/*, auto_sync/*,    │
│  clipboard, logging, update, transaction,   │
│  local_data, migration, status_snapshot,    │
│  mcp/, diagnostics                          │
├─────────────────────────────────────────────┤
│           Sync-Client Layer                 │
│  sync.rs, sync_commands.rs, encryption.rs,  │
│  sync_failure.rs, config/ (sync settings) │
├─────────────────────────────────────────────┤
│           Domain / Core Layer               │
│  library/, sort.rs, usage.rs, output.rs,    │
│  diagnostics.rs, error.rs, selector.rs,     │
│  process_file_lock.rs, utils/*              │
└─────────────────────────────────────────────┘
```

**Dependency rule**: `application → sync-client → core` (and `application → core`).
No reverse dependencies: core must not import UI/commands/sync; sync-client must not
import commands/ui/logging/auto_sync.

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
commands manage their own behavior. The hidden `auto-sync-worker` subcommand
is suppressed from help and from startup-recovery recursion.

**Exit codes** (stable, see `docs/EXIT_CODES.md`): 0 success, 1 general error,
2 usage error, 3 not found, 4 cancelled, 5 ambiguous, 6 validation,
7 sync failure, 8 execution failure, 9 conflict/refused, 10 unsafe repairs pending.

---

## Command Modules

**Source**: `src/commands/` (26 files incl. `mod.rs`; 25 submodules)
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
| `update` | [update.md](update.md) | Self-update via current installation method |
| `data` | — | Advanced data maintenance subgroup (validate/backup/restore/repair/status) |

**Shared helpers**: [commands/mod.md](commands/mod.md) — path resolution,
library loading, snippet expansion, output writing, shared `run_snippet_selection`
TUI loop, explicit `--sync` orchestration.

**Backup archive I/O**: [commands/backup_archive.md](commands/backup_archive.md) —
directory vs single-file archive layout, manifest, SHA-256 integrity, secret redaction.

**Doctor reporting**: [commands/doctor_report.md](commands/doctor_report.md) —
human/JSON rendering for `snp doctor`, compatibility diagnostics.

**Pet analysis**: [commands/pet_analysis.md](commands/pet_analysis.md) —
pet file reading, field detection, import analysis.

---

## Core Data Layer

| Module | Source | Deep Dive | Purpose |
|--------|--------|-----------|---------|
| `library` | `src/library/` (mod/model/persistence/manager) | [library.md](library.md) | `Snippet`, `Snippets`, `LibraryManager` — structures + TOML persistence + read-only resolver |
| `error` | `src/error.rs` | [core.md](core.md) | `SnipError` enum, `SnipResult<T>`, `SyncFailureKind` |
| `config` | `src/config/` (mod/sync_settings/toml_cache) | [config.md](config.md) | `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode`, keychain API key, TOML cache |
| `encryption` | `src/encryption.rs` | [encryption.md](encryption.md) | AES-256-GCM + Argon2id end-to-end encryption |
| `selector` | `src/selector.rs` | [selector.md](selector.md) | `SnippetSelector` — deterministic non-TUI resolution; fuzzy `SearchFields`/`searchable_text` |
| `outcome` | `src/outcome.rs` | [outcome.md](outcome.md) | `CliOutcome` — exit codes, machine output |
| `sort` | `src/sort.rs` | [sort.md](sort.md) | `SnippetSort` — 6 sort modes, 5-level tie-break chain |
| `usage` | `src/usage.rs` | [usage.md](usage.md) | `UsageIndex` — persistent per-snippet usage metadata |
| `output` | `src/output.rs` | [output.md](output.md) | `OutputPresentation` — safe output-field rendering |
| `migration` | `src/migration.rs` | [persistence.md](persistence.md) | Schema versioning (`SchemaVersion`), forward-only migrations |
| `transaction` | `src/transaction.rs` | [persistence.md](persistence.md) | Transaction journal, lock, begin/commit/rollback state machine |
| `local_data` | `src/local_data.rs` | [persistence.md](persistence.md) | Short-lived exclusive lock serializing TOML mutations |
| `process_file_lock` | `src/process_file_lock.rs` | [process_file_lock.md](process_file_lock.md) | Kernel-backed cross-process file lock (`flock`/`LockFileEx`) |
| `diagnostics` | `src/diagnostics.rs` | [diagnostics.md](diagnostics.md) | Shared diagnostic/report types for doctor/validate/import |
| `status_snapshot` | `src/status_snapshot.rs` | [status.md](status.md) | Status snapshot and diagnostic codes |
| `sync_failure` | `src/sync_failure.rs` | [sync.md](sync.md) | `FailureClass` (4 variants) so `crate::sync` classifies without depending on auto_sync |
| `test_failpoints` | `src/test_failpoints.rs` | [test-infrastructure.md](test-infrastructure.md) | Test-only failpoint hooks (`test-support`) |

---

## Sync Client

**Source**: `src/sync.rs`, `src/sync_commands.rs`, `src/sync_failure.rs`
**Deep dive**: [sync.md](sync.md)

The sync client communicates with the snip-sync server over gRPC (tonic)
with TLS. Snippets are encrypted with the user's API key before
transmission (AES-256-GCM).

**Key components**:
- `SyncClient` — tonic gRPC client with exponential backoff retries
- `retry_grpc_unified!` macro + `RetryBackoff` — single retry policy with jitter
  (inline expansion so `self.client.<rpc>()` reborrows work; never a closure helper)
- `sync_encrypted()` — byte-bounded upload batches (3.5 MiB ceiling < server 4 MiB)
- `sync_commands::run_sync()` — full bidirectional sync orchestration + merge
- `sync_failure.rs` — `FailureClass` (Transient/Configuration/LocalFailure/Internal)

**Merge strategy**:
- Live conflicts: `(updated_at, device_id, SHA-256(synced fields))` — never role-dependent server-wins
- Deletions win over live content (no-resurrection); tombstones persist until acked
- Local-only fields (`output`, `folders`, `favorite`) preserved, excluded from fingerprint;
  `output` is not in `ProtoSnippet`, `snp edit --output` requires `--filter`

**Failure classification**: `SyncFailureKind` (21 variants) maps to
`FailureClass` (4 variants). Multi-batch `PushSnippets` errors preserve the original
kind via `add_batch_context()`. Scheduling errors are typed — never collapse
pending-read/spawn failures into `NoPending`/`SpawnNow`/success.

---

## Auto-Sync Subsystem

**Source**: `src/auto_sync/` (11 files incl. `mod.rs`; 10 submodules)
**Deep dive**: [auto_sync.md](auto_sync.md)

Single detached-helper model. A background worker (`snp auto-sync-worker`)
runs the canonical sync operation after local mutations. Disabled by default.

| Module | Purpose |
|--------|---------|
| `execution_lock.rs` | `SyncExecutionLock`, `WorkerLock` — kernel-backed exclusive ownership (merged worker lock + `spawn_worker`) |
| `lock.rs` | Worker lock re-exports for backward compatibility |
| `worker.rs` | Detached worker entry point, holds lock for entire cycle |
| `notification.rs` | Mutation notification, pending marker creation, startup recovery |
| `pending.rs` | `PendingState` — on-disk pending mutation marker (monotonic generations) |
| `pending_lock.rs` | Transaction-scoped pending marker lock |
| `policy.rs` | `AutoSyncPolicy`, retry disposition, `MutationKind`/`MutationOrigin` |
| `schedule.rs` | Debounce scheduling, `schedule_sync()` — sole scheduling authority |
| `status.rs` | `StatusSnapshot`, `TopLevelSyncState` (8 variants), diagnostic codes |
| `test_events.rs` | Test-only lifecycle event emission (JSON-lines when `SNP_TEST_EVENTS_DIR` set) |

**Key invariants**:
- Parent never holds the worker lock; scheduler never probes the execution lock
- `schedule_sync()` is the sole scheduling authority
- Pending generations are monotonic; lower generation = corrupt (preserve marker, no spawn),
  except lower-generation + strictly newer timestamp = cleared and re-recorded as new work
- Local mutations always commit before remote work; remote failure never rolls back local commit
- SyncMerge origin never triggers auto-sync (no loops)

---

## Server (snip-sync)

**Source**: `snip-sync/src/` (19 modules)
**Deep dive**: [server.md](server.md)

Self-hosted Rust gRPC server using tonic (gRPC) + EggServe (HTTP/1 leaf runtime).
Tonic and EggServe own separate listeners. Do not restore Axum/Tower-HTTP or a
generic router/middleware layer.

| Module | Purpose |
|--------|---------|
| `main.rs` | Server entry, CLI (serve/init/cert/edit/stop/restart/update/paths/croncheck) |
| `lib.rs` | `SnipSyncService` implementing all gRPC RPCs, `Config` |
| `db.rs` | SQLite persistence (users/libraries/snippets), Argon2id API key hashing |
| `http.rs` | One concrete two-route EggServe leaf service (`/health`, `/metrics`), Basic-auth, CORS, security headers |
| `orchestration.rs` | Service lifetime, typed graceful shutdown coordination |
| `bootstrap.rs` | Server initialization and service wiring |
| `startup.rs` | Startup checks and fail-closed config loading |
| `rate_limiter.rs` | In-memory per-key rate limiting |
| `metrics.rs` | Prometheus metrics (requests, auth failures, sync operations) |
| `premade.rs` | Premade library file scanning |
| `server_lock.rs` | Kernel-backed server singleton lock |
| `cert.rs` | TLS certificate generation |
| `cli.rs` | CLI argument parsing |
| `paths.rs` | Path resolution for server state |
| `process.rs` | Legacy PID parsing and stop/restart compatibility |
| `editor.rs` | Server-side config editing |
| `update.rs` | Server update (external `curl`; lean-profile size gate keeps it out of-process) |
| `test_helpers.rs` | Test-only helpers (`test-helpers` feature) |
| `test_observer.rs` | Test-only event capture |

Wire parity is locked in `tests/snip_sync_lifetime.rs`: router 404/405 are empty
with no content-type (metrics-disabled 404 keeps `"Not found"`), known routes answer
preflight/unsupported methods with `Allow: GET, HEAD`, every non-allow-all response
carries `Vary: origin`, preflight never carries security headers.

---

## Protocol (snip-proto)

**Source**: `snip-proto/proto/sync.proto`, `snip-proto/src/` (checked-in `snip_proto.rs`)
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

`protoc` is needed only for an explicit regen after editing `proto/sync.proto`.

---

## TUI & User Interface

**Source**: `src/ui/` (6 files)
**Deep dives**: [tui.md](tui.md), [ui.md](ui.md)

Built with `ratatui` + `crossterm`. Single-loop event-driven architecture.

| Module | File | Purpose |
|--------|------|---------|
| Main loop | `mod.rs` | Event loop, fuzzy search (`SkimMatcherV2`), keyboard navigation; re-exports theme/state |
| State | `state.rs` | `SelectState`, `FilterState`, sort mode (TUI-internal) |
| Theme | `theme.rs` | `Theme` struct (10-color palette), 50 bundled Halloy themes |
| Highlight | `highlight.rs` | Syntax highlighting (variables, shell keywords, strings, flags) |
| Variables | `variables.rs` | TUI for `<name>` / `<name=default>` variable prompts |
| Bundled themes | `_generated_bundled_themes.rs` | Generated by `python3 scripts/build_themes.py`; never edit by hand |

---

## MCP (Model Context Protocol)

**Source**: `src/mcp/` (mod/protocol/tools/client_install)
**Deep dive**: [mcp.md](mcp.md)

Read-only, stdio-only, non-executing MCP server for AI-agent access:

- `protocol.rs` — JSON-RPC 2.0 framing over stdio
- `tools.rs` — `snippets_search` / `snippet_get` / `snippets_list` (share `selector::searchable_text`)
- `client_install.rs` — client installation helpers
- `mod.rs` — stdio entry, capability advertisement

Search parity: `snp get --query`, `snp list --filter`, MCP `snippets_search` share
`searchable_text` (description + command always, tags by default, output/notes only
with opt-in). Folders/favorite/sync metadata/credentials never searchable.
`snippet_get` requires exactly one of ID (case-sensitive) / description / command
(case-insensitive).

---

## Self-Update

**Source**: `src/update.rs` (client), `snip-sync/src/update.rs` (server)
**Deep dive**: [update.md](update.md)

- `snp update` uses in-process `eggfetch-core =0.2.0` (`standard-http1`+`redirects`+
  `tls-rustls`+`tls-native-roots` only; strict redirects + native `Timeout.total`,
  initial-HTTPS guard). Transport tests live in `src/update.rs` (`test-support`)
  with a std-only loopback fixture.
- `snip-sync update` keeps external `curl`: the 0.2.0 lean-profile trial grew the
  server +34% past the 10% gate. Pinned in `tests/architecture.rs`.

---

## Diagnostics

**Source**: `src/diagnostics.rs`, `src/commands/doctor_cmd.rs`, `src/commands/doctor_report.rs`
**Deep dive**: [diagnostics.md](diagnostics.md)

Shared diagnostic/report types (`CompatibilityDiagnostic`, `PetImportReport`,
`DoctorReport`) rendered by `doctor`, `validate`, and import paths. No generic
finding DSL — each consumer keeps its own semantics over the shared index
inspection (`LibraryManager::inspect_library_index`).

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
| `utils/process` | `src/utils/process.rs` | [utils/process.md](utils/process.md) | Shared process-liveness (`is_process_alive`) and owned-lock-file removal |
| `utils/redact` | `src/utils/redact.rs` | [utils/redact.md](utils/redact.md) | Secret redaction for logs/backups/diagnostics (never log credentials) |
| `clipboard` | `src/clipboard.rs` | [clipboard.md](clipboard.md) | Cross-platform clipboard (arboard/clipboard-win); side effects via `copy_to_clipboard()` |
| `logging` | `src/logging.rs` | [logging.md](logging.md) | Structured logging (`tracing`), audit trail, panic handler |
| `process_file_lock` | `src/process_file_lock.rs` | [process_file_lock.md](process_file_lock.md) | Kernel-backed cross-process file lock (`flock`/`LockFileEx`) |
| `status_snapshot` | `src/status_snapshot.rs` | [status.md](status.md) | Status snapshot and diagnostic codes |
| `update` | `src/update.rs` | [update.md](update.md) | Cargo/Homebrew update checking (in-process eggfetch-core transport) |

Full utility inventory: [utils.md](utils.md).

---

## Persistence & Durability

**Deep dive**: [persistence.md](persistence.md)

- **Atomic writes**: `utils/atomic.rs` with `TempFileGuard` for cleanup.
  Durability classes: `DurableUserData` (fsync file+dir),
  `SensitiveConfig` (0o600, symlink reject), `RecoverableMetadata` (no
  fsync), `EphemeralCoordination` (no fsync, no dir sync).
- **Transaction journaling**: `transaction.rs` — `Prepared → Committing →
  CleaningUp` state machine for multi-file mutations. Journals persist under
  `<config>/.transaction/` (transaction APIs take `.transaction`; pending-marker
  APIs take the state dir) so interrupted operations recover on startup.
- **Local data lock**: `local_data.rs` — exclusive lock serializing TOML
  mutations against backup snapshot capture.
- **Schema migration**: `migration.rs` — `SchemaVersion` ordinal type with
  forward-only migration operations. Save path does NOT post-process
  `toml::to_string_pretty` (golden corpus must survive); `write_schema_version`
  uses `toml::Table` to preserve array-of-tables.
- **Backup/restore**: SHA-256 integrity verification, secret-free snapshots,
  merge/replace restore modes (see `commands/backup_archive.md`).
- **Mutation gate**: `gate_mutation_on_interrupted_transactions()` must be
  called before any local mutating operation. Single journal = auto-rollback;
  multiple/incomplete = refuse and direct to `snp repair`.

---

## Testing Infrastructure

**Deep dive**: [test-infrastructure.md](test-infrastructure.md)

~50 integration test targets in `tests/` (plus 4 shared modules in
`tests/support/`). Reusable components in
`tests/support/`: `TestEnvironment` (isolated TempDir + `XDG_CONFIG_HOME`
override), `RecordingServer`, `EventSink`. Never touch real config/keychain/ports.
`SNP_ALLOW_PLAINTEXT_API_KEY=true` on all test commands — never remove that seam.
`set_var` in tests needs `unsafe` (edition 2024).

| Class | Execution | Targets |
|-------|-----------|---------|
| Unit/pure | parallel | `cargo test --workspace --lib` |
| CLI/platform smoke | parallel | `platform_smoke.rs`, `local_contracts.rs` |
| Restore contracts | parallel | `destination_permissions.rs`, `backup_contracts.rs` |
| Auto-sync contracts | parallel | `auto_sync_closure.rs`, `sync_contracts.rs`, `debounce_matrix.rs` |
| Sync integration | serial | `sync_integration.rs` — in-process server, random port |
| PTY | serial | `pty_integration.rs` — real terminal pairs (`--test-threads=1`) |
| Cross-process lock | serial | `process_lock_concurrency.rs` — kernel flock (`test-support`) |
| Barrier-coordinated | serial | `local_data_lock_barriers.rs`, `repair_transactions.rs` (`test-support`) |
| Multi-batch sync | serial | `sync_multibatch.rs` (`--test-threads=1`) |
| Auto-sync concurrency | serial | `auto_sync_concurrency.rs` (`--test-threads=1`) |
| Deep recovery | manual | `transaction_crash_recovery.rs`, failpoint tests (`release-check.sh verify`) |
| Release smoke | manual | `release-check.sh` Phase 3 — `manifest_contracts.rs`, crash, production seams |
| Architecture | parallel | `architecture.rs` — source-scanning layer boundary enforcement |

`check.sh` passes `--features test-support` to focused targets (`platform_smoke`,
`destination_permissions`, `auto_sync_closure`, plus `-- --test-threads=1` for
`auto_sync_concurrency`, `sync_multibatch`). Only `repair_transactions`,
`process_lock_concurrency`, `local_data_lock_barriers` (plus `process_lock_helper`
bin) gate compilation on it.

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
| `~/.config/snp/.transaction/` | Transaction journals + locks |
| `~/.config/snp/.transaction/*.lock` | Transaction / pending locks |
| `~/.config/snp/backups/` | Backup snapshots |
| `~/.config/snp/logs/` | Rolling log files |
| `~/.config/snp/audit.log` | Audit trail |

External library paths are not supported. All snippet libraries reside
under `~/.config/snp/libraries/`. Malformed library/`libraries.toml` fails
closed (best-effort backup + error, never synthesize writable empty);
missing/empty files give defaults.

---

## Data Flow: Running a Snippet

```
snp run [--filter FOO] [--sync]
  │
  ├─ main.rs::dispatch_command()
  │    └─ commands::run_cmd::run()
  │         ├─ gate_mutation_on_interrupted_transactions()
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
       └─ spawn_worker()  → detached background sync (same SyncExecutionLock)
```

Do not sanitize snippet commands (by design). Clipboard side effects go through
`copy_to_clipboard()` in `clip_cmd.rs`.

---

## Key Patterns

### Error Handling
- `SnipError` enum with domain-specific variants: `Io`, `Toml`, `Clipboard`,
  `Command`, `Runtime`, `SyncFailure`
- Constructor helpers: `io_error()`, `toml_error()`, `clipboard_error()`,
  `command_error()`, `runtime_error()`, `sync_failure()`
- `SyncFailureKind` (21 variants) for typed sync failure classification; never
  log credentials (see `error.rs`, no credentials)
- `CliOutcome` → stable exit codes (see `docs/EXIT_CODES.md`)

### Async (Tokio)
- Global `RUNTIME: LazyLock<Runtime>` — only initialized by async commands
  (`run`, `clip`, `search`, `sync`, `register`, `premade`, `update`); local-only
  commands never init it. `run_snippet_selection` takes `Option<&Runtime>`
  (`None` when `do_sync` false). Detached worker uses `new_current_thread()`;
  keep client's `rt-multi-thread` feature. Keep `keyring = "4"` default features.

### TOML Handling
- `\<` and `\>` in double-quoted TOML strings cause parse failures
- Solution: convert to single-quoted (raw literals) before parsing, reverse
  on save — implemented in `utils/toml_helpers.rs`
- Save path does NOT post-process `toml::to_string_pretty` (golden corpus:
  tabs, trailing spaces, CRLF must survive)

### Encryption
- AES-256-GCM for snippet payload encryption; Argon2id for key derivation
- Argon2 parameter changes break all existing payloads (version first)
- Key cache for repeated operations
- See [encryption.md](encryption.md)

### Process Locks
- Kernel-backed (`flock` Unix / `LockFileEx` Windows) for:
  - Server singleton (`server_lock.rs`)
  - Auto-sync execution lock (`execution_lock.rs`)
  - Transaction lock (`transaction.rs`)
  - Local data lock (`local_data.rs`)
- `Drop` releases without unlinking; lock files may hold stale metadata
- `kill(pid,0)`: only `ESRCH` proves absence (`EPERM`/unknown = live);
  Linux start tokens use `/proc/<pid>/stat` field 22
- `.cargo/config.toml` sets Windows MSVC `/STACK:8388608` — large `Commands`
  enum overflows 1 MB default. Do not remove.

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
| [commands/backup_archive.md](commands/backup_archive.md) | Backup archive I/O, manifest, integrity |
| [commands/status_cmd.md](commands/status_cmd.md) | Auto-sync status display |
| [commands/sync_cmd.md](commands/sync_cmd.md) | Sync and config subcommands |
| [commands/cron_cmd.md](commands/cron_cmd.md) | Crontab generation for periodic sync |
| [commands/register_cmd.md](commands/register_cmd.md) | Device registration |
| [commands/library_cmd.md](commands/library_cmd.md) | Library management subcommands |
| [commands/premade_cmd.md](commands/premade_cmd.md) | Premade library access |
| [commands/import_cmd.md](commands/import_cmd.md) | Pet snippet file import |
| [commands/doctor_cmd.md](commands/doctor_cmd.md) | Diagnostics, pet analysis, environment audit |
| [commands/doctor_report.md](commands/doctor_report.md) | Doctor human/JSON rendering |
| [commands/shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation |
| [commands/keybindings_cmd.md](commands/keybindings_cmd.md) | TUI keybindings reference |
| [commands/pet_analysis.md](commands/pet_analysis.md) | Pet file reading, field detection, import analysis |

### Core Data

| File | Subject |
|------|---------|
| [core.md](core.md) | Core types, error handling, key abstractions |
| [library.md](library.md) | Data structures, persistence, library management |
| [config.md](config.md) | Sync settings, path resolution, keychain API key, TOML cache |
| [encryption.md](encryption.md) | AES-256-GCM end-to-end encryption |
| [selector.md](selector.md) | Deterministic non-TUI snippet resolution |
| [outcome.md](outcome.md) | `CliOutcome` — exit codes, machine output |
| [sort.md](sort.md) | Sort modes, ranking, tie-break chain |
| [usage.md](usage.md) | Local usage metadata, update policy, storage |
| [output.md](output.md) | Snippet output field rendering |
| [diagnostics.md](diagnostics.md) | Shared diagnostic/report types |
| [process_file_lock.md](process_file_lock.md) | Kernel-backed cross-process file lock |

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

### Integrations

| File | Subject |
|------|---------|
| [mcp.md](mcp.md) | Read-only stdio MCP server, tools, search parity |
| [update.md](update.md) | Self-update (eggfetch client, curl server), install methods |

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
| [utils/process.md](utils/process.md) | Process liveness and owned-lock-file removal |
| [utils/redact.md](utils/redact.md) | Secret redaction for logs/backups |
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
| `../docs/LOGICAL_LAYERS.md` | Target logical layer architecture |
| `../docs/ARCHITECTURE_INVENTORY.md` | Comprehensive module inventory |
| `../AGENTS.md` | Authoritative verify commands, gotchas, invariants |
| `../AGENTS.override.md` | Session pitfall notes |
| `../plans/registry.md` | Authoritative planning status |

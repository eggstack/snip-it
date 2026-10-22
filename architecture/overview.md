# Architecture Overview

Bird's-eye view of the snip-it codebase.

This document has two jobs:

1. **Orient a newcomer (or a reviewer) in ~10 minutes.** The module map, the
   capability map, the layer boundaries, and the invariants that matter
   everywhere.
2. **Index the deep dives.** Every discrete component links to a dedicated
   document in this directory that goes deep on it. The index is at the
   bottom — [Deep-Dive Index](#deep-dive-index).

> **How to use it.** Read §1–§4 for the shape of the system. Jump straight to
> the [Deep-Dive Index](#deep-dive-index) for the component you care about.
> Every deep dive is written so a reviewer can audit the code against it:
> real `file:line` citations, current behavior, and the traps.

---

## Table of Contents

- [1. System at a Glance](#1-system-at-a-glance)
- [2. Repository Layout](#2-repository-layout)
- [3. Workspace Crates](#3-workspace-crates)
- [4. Logical Layers](#4-logical-layers)
- [5. Module Map](#5-module-map)
- [6. Capability Map](#6-capability-map)
- [7. Cross-Cutting Invariants](#7-cross-cutting-invariants)
- [8. Primary Data Flows](#8-primary-data-flows)
- [9. On-Disk State](#9-on-disk-state)
- [10. Testing Surface](#10-testing-surface)
- [Deep-Dive Index](#deep-dive-index)

---

## 1. System at a Glance

**snip-it** (`snp`) is a terminal snippet manager: a fast fuzzy-search TUI over
locally stored snippet libraries, with clipboard delivery, shell integration,
variable expansion, an AI-agent (MCP) surface, and optional encrypted
multi-device sync against a self-hosted server.

> **On "end-to-end encryption":** snippet *bodies* are encrypted with
> AES-256-GCM under an Argon2id-derived key, and the server stores only
> ciphertext it cannot read. But the API key is sent as a `Bearer` token on
> every request (`src/sync.rs:306-314`), so anything that observes that token
> can re-derive the key. The guarantee is transport + at-rest protection, not
> server-blindness. See [encryption.md](encryption.md).

Four things make it unusual, and they are where most of the engineering
complexity lives:

1. **Everything is a local TOML file.** There is no database in the client.
   Snippets live in `~/.config/snp/libraries/*.toml`, and every mutation goes
   through an atomic-write + transaction-journal path.
2. **Sync is peer-merged on the client, never role-dependent.** Conflict
   resolution is `(updated_at, device_id, SHA-256(synced fields))`, and a
   deletion always beats live content. The server applies a *different*,
   weaker rule (see [§7](#7-cross-cutting-invariants)) — the no-resurrection
   guarantee lives only on the client. The server stores snippet bodies as
   ciphertext, but the API key reaches it as a bearer token — see the
   trust-boundary note above.
3. **Read-only paths are separated by a distinct resolver.** A plain `snp list`
   may not migrate metadata, create directories, or bump a generation. This is
   enforced by a separate read-only resolver (`resolve_selector_readonly` /
   `readonly_library_sources`) and is repeatedly load-bearing. The separation is
   per *call site*, not per command — and it has known edges: the macOS config
   migration and the `.corrupt` quarantine files are reachable from otherwise
   read-only commands. See [selector.md](selector.md) and
   [commands/validate_cmd.md](commands/validate_cmd.md).
4. **Locks are kernel-backed and hierarchical.** Cross-process exclusion uses
   `flock`/`LockFileEx`, with a strict acquisition order and a bounded retry
   ladder, so a crashed process degrades to quarantine rather than a hang.

### The three processes

| Process | What it is | Where it runs |
|---------|-----------|---------------|
| `snp` | The interactive CLI. One invocation, one user-visible action. | Foreground, in the user's terminal |
| `snp auto-sync-worker` | A **detached** helper spawned by `snp` after a local mutation. Hidden subcommand, `--state-dir` only, stdio detached from the parent. It holds the `SyncExecutionLock` for the whole cycle. | Background, orphaned from the parent |
| `snip-sync` | The self-hosted sync server. Tonic gRPC on one listener, the EggServe HTTP/1 leaf (`/health`, `/metrics`) on another. | Usually another machine |

```
  user terminal                     user machine                     sync host
  ┌──────────────┐                  ┌──────────────────┐             ┌────────────┐
  │ snp          │  mutation        │ ~/.config/snp/   │  gRPC+TLS   │ snip-sync  │
  │  └─ spawn ───┼─────────────────▶│ libraries/*.toml │◀────────────│  tonic     │
  │              │  detached        │ .transaction/    │  ciphertext │  EggServe  │
  │              │  worker          │ auto-sync-*.toml │  (AES-GCM)  │  SQLite    │
  └──────────────┘                  └──────────────────┘             └────────────┘
```

The worker is the **only** thing that talks to the server on the user's
behalf without the user asking. It exists so a mutation never blocks on the
network. The manual `snp sync run` path uses the *same* `SyncExecutionLock`,
so a manual sync and an auto-sync can never overlap.

---

## 2. Repository Layout

```
snip-it/                 Main crate — binary "snp" (+ a library for binary/test sharing)
  src/                   Application source (~58k LOC across 87 files)
  tests/                 51 integration test targets + tests/support/ (4 shared modules)
  architecture/          This directory — one deep-dive doc per component
  docs/                  Reference contracts (EXIT_CODES, PUBLIC_API, THREAT_MODEL, ...)
  .skills/               Task-shaped agent reference docs (sync, transactions, server, ...)
  plans/                 Planning registry + roadmaps (plans/registry.md is authoritative)
  scripts/               check.sh, release-check.sh, build_themes.py, installers
  themes/                50 Halloy TOML themes (bundled by build_themes.py)
  packaging/             Dist packaging; demo/ + assets/ hold demos and screenshots
snip-proto/              Protobuf schema + checked-in tonic stubs (proto/sync.proto)
snip-sync/               Sync server — tonic gRPC + EggServe HTTP/1 leaf + SQLite (~11k LOC)
```

---

## 3. Workspace Crates

| Crate | Type | Purpose |
|-------|------|---------|
| `snip-it` | Binary (`snp`) + library | The application: CLI, commands, TUI, sync client, core data model |
| `snip-proto` | Library | Protobuf schema and checked-in tonic-generated gRPC stubs |
| `snip-sync` | Binary + library | Self-hosted sync server (gRPC + HTTP, SQLite, TLS, metrics) |

**Publish order is manual and local** — `snip-proto` → `snip-sync` → `snip-it`.
No CI token publishes anything. A proto change requires bumping
`snip-proto`'s version in **both** dependents.

The `snip-it` library crate exposes a small stable surface (`Snippet`,
`Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library`,
atomic-write helpers, `SnipError`, `SnippetSort`, `SyncSettings`, …). Most
modules are `#[doc(hidden)]` — public so the binary and integration tests can
reach them, but **not** part of the supported external API. See
[public_api.md](public_api.md) and `docs/PUBLIC_API.md`.

---

## 4. Logical Layers

Three layers with a strict dependency direction, specified in
[`../docs/LOGICAL_LAYERS.md`](../docs/LOGICAL_LAYERS.md).

```
┌──────────────────────────────────────────────────────────────┐
│                    Application / CLI Layer                   │
│  main.rs, commands/*, ui/*, auto_sync/*, mcp/*, clipboard,    │
│  logging, update, transaction, local_data, migration,        │
│  status_snapshot, diagnostics                                │
├──────────────────────────────────────────────────────────────┤
│                      Sync-Client Layer                       │
│  sync.rs, sync_commands.rs, encryption.rs, sync_failure.rs,  │
│  config/ (sync settings)                                     │
├──────────────────────────────────────────────────────────────┤
│                        Domain / Core Layer                   │
│  library/, selector.rs, sort.rs, usage.rs, output.rs,        │
│  diagnostics.rs, error.rs, outcome.rs, process_file_lock.rs, │
│  utils/*                                                      │
└──────────────────────────────────────────────────────────────┘
```

**Rule:** `application → sync-client → core`, and `application → core`.
No reverse edges.

**How much is actually enforced:** `tests/architecture.rs` source-scans only
the core/sync-client boundary (`library/`, `sort.rs`, `output.rs`, `usage.rs`,
`diagnostics.rs` vs `sync.rs`/`sync_commands.rs`/`encryption.rs`; `config/`
is an explicit carve-out). Wider layer claims are convention, not a
mechanically enforced contract — treat them as such when reviewing.

---

## 5. Module Map

Every source module, what it owns, and where it is documented in depth.

### 5.1 Entry points & public surface

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| CLI entry | `src/main.rs` | clap surface, dispatch, startup recovery, signals, exit | [cli.md](cli.md) |
| Library crate root | `src/lib.rs` | public API surface, module visibility | [public_api.md](public_api.md) |
| Outcome | `src/outcome.rs` | `CliOutcome` → exit codes, machine output | [outcome.md](outcome.md) |
| Errors | `src/error.rs` | `SnipError`, `SnipResult`, `SyncFailureKind` | [core.md](core.md) |

### 5.2 Core data layer

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Library model | `src/library/{mod,model,persistence,manager}.rs` | `Snippet`, `Snippets`, TOML persistence, `LibraryManager`, read-only resolver | [library.md](library.md) |
| Selector | `src/selector.rs` | `SnippetSelector`, `SearchFields`, `searchable_text`, `resolve_exact_target` | [selector.md](selector.md) |
| Sorting | `src/sort.rs` | `SnippetSort`, 6 modes, 5-level tie-break chain | [sort.md](sort.md) |
| Usage | `src/usage.rs` | `UsageIndex` — local per-snippet usage metadata | [usage.md](usage.md) |
| Output | `src/output.rs` | `OutputPresentation` — safe output-field rendering | [output.md](output.md) |
| Diagnostics | `src/diagnostics.rs` | shared diagnostic/report types | [diagnostics.md](diagnostics.md) |
| Status snapshot | `src/status_snapshot.rs` | `StatusSnapshot`, `TopLevelSyncState`, diagnostic codes | [status.md](status.md) |
| Process file lock | `src/process_file_lock.rs` | kernel-backed cross-process lock (`flock`/`LockFileEx`) | [process_file_lock.md](process_file_lock.md) |

### 5.3 Persistence & durability

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Transaction journal | `src/transaction.rs` | journal, transaction lock, `Prepared→Committing→CleaningUp` state machine | [persistence.md](persistence.md) |
| Local data lock | `src/local_data.rs` | exclusive lock serializing TOML mutations vs backup capture | [persistence.md](persistence.md) |
| Migration | `src/migration.rs` | `SchemaVersion`, forward-only migrations | [persistence.md](persistence.md) |
| Atomic writes | `src/utils/atomic.rs` | durability-aware atomic write primitives | [utils/atomic.md](utils/atomic.md) |

### 5.4 Sync client

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Sync client | `src/sync.rs` | `SyncClient`, `retry_grpc_unified!`, byte-bounded batching, TLS | [sync.md](sync.md) |
| Sync orchestration | `src/sync_commands.rs` | `run_sync()`, `merge_snippets()`, library linking, remote recovery | [sync.md](sync.md) |
| Failure classes | `src/sync_failure.rs` | `FailureClass` (4 variants) | [sync.md](sync.md) |
| Encryption | `src/encryption.rs` | Argon2id + AES-256-GCM | [encryption.md](encryption.md) |
| Config | `src/config/{mod,sync_settings,toml_cache}.rs` | `SyncSettings`, keychain API key, TOML cache | [config.md](config.md) |

### 5.5 Auto-sync subsystem

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Auto-sync root | `src/auto_sync/mod.rs` | subsystem root, re-exports, policy plumbing | [auto_sync.md](auto_sync.md) |
| Execution lock | `src/auto_sync/execution_lock.rs` | `SyncExecutionLock` — exclusive ownership of the sync cycle | [auto_sync.md](auto_sync.md) |
| Worker lock | `src/auto_sync/lock.rs` | backward-compatible re-exports | [auto_sync.md](auto_sync.md) |
| Worker | `src/auto_sync/worker.rs` | detached worker entry point, holds the lock for the cycle | [auto_sync.md](auto_sync.md) |
| Notification | `src/auto_sync/notification.rs` | mutation notification, pending marker, startup recovery | [auto_sync.md](auto_sync.md) |
| Pending state | `src/auto_sync/pending.rs` | `PendingState` — on-disk marker, monotonic generations | [auto_sync.md](auto_sync.md) |
| Pending lock | `src/auto_sync/pending_lock.rs` | transaction-scoped pending-marker lock | [auto_sync.md](auto_sync.md) |
| Policy | `src/auto_sync/policy.rs` | `AutoSyncPolicy`, retry disposition, `MutationKind`/`MutationOrigin` | [auto_sync.md](auto_sync.md) |
| Schedule | `src/auto_sync/schedule.rs` | debounce, `schedule_sync()` — the sole scheduling authority | [auto_sync.md](auto_sync.md) |
| Status file | `src/auto_sync/status.rs` | `AutoSyncStatus` (durable file), `record_success`/`record_failure`, config fingerprint | [auto_sync.md](auto_sync.md) |
| Test events | `src/auto_sync/test_events.rs` | JSON-lines lifecycle events (`SNP_TEST_EVENTS_DIR`) | [auto_sync.md](auto_sync.md) |

### 5.6 Commands

One module per command; the canonical `*Args` struct lives beside its
handler. `snp data` is a **mirror group** over the same handlers — a separate
Clap subcommand group, not a Clap `alias` — and the single-path `handle_*`
functions in `main.rs` keep both spellings identical.

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Command helpers | `src/commands/mod.rs` | path resolution, load/save, expansion, `run_snippet_selection` | [commands/mod.md](commands/mod.md) |
| New | `src/commands/new_cmd.rs` | snippet creation | [commands/new_cmd.md](commands/new_cmd.md) |
| List | `src/commands/list_cmd.rs` | text listing | [commands/list_cmd.md](commands/list_cmd.md) |
| Run | `src/commands/run_cmd.rs` | TUI selection + shell execution | [commands/run_cmd.md](commands/run_cmd.md) |
| Clip | `src/commands/clip_cmd.rs` | clipboard delivery | [commands/clip_cmd.md](commands/clip_cmd.md) |
| Search | `src/commands/search_cmd.rs` | fuzzy search + detail display | [commands/search_cmd.md](commands/search_cmd.md) |
| Edit | `src/commands/edit_cmd.rs` | `$EDITOR` editing, `output` field management | [commands/edit_cmd.md](commands/edit_cmd.md) |
| Select | `src/commands/select_cmd.rs` | non-executing selection for shell integration | [commands/select_cmd.md](commands/select_cmd.md) |
| Get | `src/commands/get_cmd.rs` | deterministic non-TUI retrieval | [commands/get_cmd.md](commands/get_cmd.md) |
| Validate | `src/commands/validate_cmd.rs` | read-only data validation | [commands/validate_cmd.md](commands/validate_cmd.md) |
| Repair | `src/commands/repair_cmd.rs` | conservative, backed-up, idempotent repair | [commands/repair_cmd.md](commands/repair_cmd.md) |
| Restore | `src/commands/restore_cmd.rs` | restore from a backup snapshot | [commands/restore_cmd.md](commands/restore_cmd.md) |
| Backup | `src/commands/backup_cmd.rs` | secret-free backup snapshot | [commands/backup_cmd.md](commands/backup_cmd.md) |
| Backup archive | `src/commands/backup_archive.rs` | archive layout, manifest, SHA-256 integrity | [commands/backup_archive.md](commands/backup_archive.md) |
| Status | `src/commands/status_cmd.rs` | auto-sync status display | [commands/status_cmd.md](commands/status_cmd.md) |
| Sync | `src/commands/sync_cmd.rs` | sync + config subcommands | [commands/sync_cmd.md](commands/sync_cmd.md) |
| Cron | `src/commands/cron_cmd.rs` | crontab generation | [commands/cron_cmd.md](commands/cron_cmd.md) |
| Register | `src/commands/register_cmd.rs` | device/account registration | [commands/register_cmd.md](commands/register_cmd.md) |
| Library | `src/commands/library_cmd.rs` | library management | [commands/library_cmd.md](commands/library_cmd.md) |
| Premade | `src/commands/premade_cmd.rs` | premade library access | [commands/premade_cmd.md](commands/premade_cmd.md) |
| Import | `src/commands/import_cmd.rs` | pet snippet file import | [commands/import_cmd.md](commands/import_cmd.md) |
| Doctor | `src/commands/doctor_cmd.rs` | diagnostics, environment audit | [commands/doctor_cmd.md](commands/doctor_cmd.md) |
| Doctor report | `src/commands/doctor_report.rs` | human/JSON rendering | [commands/doctor_report.md](commands/doctor_report.md) |
| Pet analysis | `src/commands/pet_analysis.rs` | pet file reading, field detection | [commands/pet_analysis.md](commands/pet_analysis.md) |
| Shell | `src/commands/shell_cmd.rs` | shell integration code generation | [commands/shell_cmd.md](commands/shell_cmd.md) |
| Keybindings | `src/commands/keybindings_cmd.rs` | TUI keybindings reference | [commands/keybindings_cmd.md](commands/keybindings_cmd.md) |

### 5.7 User interface

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| TUI event loop | `src/ui/mod.rs` | event loop, fuzzy search, keyboard navigation | [tui.md](tui.md) |
| TUI state | `src/ui/state.rs` | `SelectState`, `FilterState` | [tui.md](tui.md) |
| TUI variables | `src/ui/variables.rs` | `<name>` / `<name=default>` prompt UI | [tui.md](tui.md) |
| Theme | `src/ui/theme.rs` | `Theme` palette, theme file loading | [ui.md](ui.md) |
| Highlight | `src/ui/highlight.rs` | syntax highlighting | [ui.md](ui.md) |
| Bundled themes | `src/ui/_generated_bundled_themes.rs` | **generated** — `python3 scripts/build_themes.py` | [ui.md](ui.md) |

### 5.8 MCP (AI agent surface)

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Entry | `src/mcp/mod.rs` | stdio entry, capability advertisement | [mcp.md](mcp.md) |
| Protocol | `src/mcp/protocol.rs` | JSON-RPC 2.0 framing over stdio | [mcp.md](mcp.md) |
| Tools | `src/mcp/tools.rs` | `snippets_search` / `snippet_get` / `snippets_list` | [mcp.md](mcp.md) |
| Client install | `src/mcp/client_install.rs` | client installation helpers | [mcp.md](mcp.md) |

### 5.9 Utilities & cross-cutting

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Utils root | `src/utils/mod.rs` | utility module inventory | [utils.md](utils.md) |
| Config paths | `src/utils/config.rs` | `get_config_dir()`, XDG, macOS migration | [utils/config.md](utils/config.md) |
| Variables | `src/utils/variables.rs` | `parse_variables()`, `expand_command()` | [utils/variables.md](utils/variables.md) |
| TOML helpers | `src/utils/toml_helpers.rs` | `\<`/`\>` escape handling | [utils/toml_helpers.md](utils/toml_helpers.md) |
| Shell keywords | `src/utils/shell_keywords.rs` | ~190 command names for highlighting | [utils/shell_keywords.md](utils/shell_keywords.md) |
| Tempfile guard | `src/utils/tempfile_guard.rs` | RAII cleanup | [utils/tempfile_guard.md](utils/tempfile_guard.md) |
| Atomic | `src/utils/atomic.rs` | `write_private_atomic()`, `atomic_replace()` | [utils/atomic.md](utils/atomic.md) |
| Process | `src/utils/process.rs` | `is_process_alive`, owned-lock-file removal | [utils/process.md](utils/process.md) |
| Redaction | `src/utils/redact.rs` | secret redaction for logs/backups | [utils/redact.md](utils/redact.md) |
| Clipboard | `src/clipboard.rs` | cross-platform clipboard | [clipboard.md](clipboard.md) |
| Logging | `src/logging.rs` | `tracing` setup, audit trail, panic handler | [logging.md](logging.md) |
| Update | `src/update.rs` | self-update via the current install method | [update.md](update.md) |

### 5.10 Server (`snip-sync`)

19 modules, ~11k LOC. One concrete EggServe HTTP/1 leaf — **no** Axum,
**no** Tower-HTTP, no generic router/middleware. Tonic keeps its own separate
listener.

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Service impl | `snip-sync/src/lib.rs` | `SnipSyncService` (all gRPC RPCs), `Config`, limit constants | [server.md](server.md) |
| Database | `snip-sync/src/db.rs` | SQLite persistence, Argon2id API-key hashing | [server.md](server.md) |
| HTTP leaf | `snip-sync/src/http.rs` | `/health` + `/metrics`, Basic-auth, CORS, security headers | [server.md](server.md) |
| Orchestration | `snip-sync/src/orchestration.rs` | service lifetime, typed graceful shutdown | [server.md](server.md) |
| Bootstrap | `snip-sync/src/bootstrap.rs` | server init and service wiring | [server.md](server.md) |
| Startup | `snip-sync/src/startup.rs` | startup checks, fail-closed config loading | [server.md](server.md) |
| Rate limiter | `snip-sync/src/rate_limiter.rs` | per-peer rate limiting | [server.md](server.md) |
| Metrics | `snip-sync/src/metrics.rs` | Prometheus counters | [server.md](server.md) |
| Premade | `snip-sync/src/premade.rs` | premade library scanning | [server.md](server.md) |
| Server lock | `snip-sync/src/server_lock.rs` | kernel-backed singleton lock | [server.md](server.md) |
| Cert | `snip-sync/src/cert.rs` | TLS certificate generation | [server.md](server.md) |
| CLI | `snip-sync/src/cli.rs` | argument parsing | [server.md](server.md) |
| Paths | `snip-sync/src/paths.rs` | server state path resolution | [server.md](server.md) |
| Process | `snip-sync/src/process.rs` | legacy PID parsing, stop/restart compat | [server.md](server.md) |
| Editor | `snip-sync/src/editor.rs` | server-side config editing | [server.md](server.md) |
| Update | `snip-sync/src/update.rs` | server update (external `curl`) | [update.md](update.md) |
| Main | `snip-sync/src/main.rs` | server entry + CLI | [server.md](server.md) |
| Test helpers | `snip-sync/src/test_helpers.rs` | test-only helpers (`test-helpers`) | [server.md](server.md) |
| Test observer | `snip-sync/src/test_observer.rs` | test-only event capture | [server.md](server.md) |

### 5.11 Protocol (`snip-proto`)

| Module | Source | Owns | Deep dive |
|--------|--------|------|-----------|
| Schema | `snip-proto/proto/sync.proto` | the `SnippetSync` gRPC service | [proto.md](proto.md) |
| Crate root | `snip-proto/src/lib.rs` | module wiring, re-exports of the generated stubs | [proto.md](proto.md) |
| Stubs | `snip-proto/src/snip_proto.rs` | **generated, checked in** — never edit by hand | [proto.md](proto.md) |

---

## 6. Capability Map

The same surface, organized by what a user is trying to do rather than by
module.

### 6.1 Capture & author

| Capability | Commands |
|------------|----------|
| Create a snippet | `snp new` (arg / stdin / file / editor / multiline) |
| Edit a snippet | `snp edit` (`$EDITOR`), `snp edit --output` (requires `--filter`) |
| Import from elsewhere | `snp import pet <PATH>` |
| Install curated content | `snp premade list\|get\|sync\|search\|update` |

### 6.2 Find & inspect

| Capability | Commands |
|------------|----------|
| Browse (never executes) | `snp list` (text / JSON / CSV) |
| Retrieve deterministically | `snp get` (ID / description / command, `--query`) |
| Fuzzy search with detail | `snp search` |
| Pick without executing | `snp select` (for shell integration) |
| Learn the keybindings | `snp keybindings` |
| Check data health | `snp validate`, `snp doctor` |
| Check sync health | `snp status` |

> `snp list` and `snp get` never execute snippet commands and never mutate
> snippet data. `snp validate` and `snp status` are read-only **with respect to
> snippet data**, but they can still write a `.corrupt` quarantine file or
> trigger the macOS config migration. See
> [commands/validate_cmd.md](commands/validate_cmd.md).

### 6.3 Execute & deliver

| Capability | Commands |
|------------|----------|
| Run a snippet | `snp run` — TUI select → expand variables → shell |
| Copy to clipboard | `snp clip` |
| Wire into the shell | `snp shell init bash\|zsh\|fish` |

### 6.4 Organize

| Capability | Commands |
|------------|----------|
| Manage libraries | `snp library list\|create\|delete\|set-primary\|show` |

### 6.5 Sync & multi-device

| Capability | Commands |
|------------|----------|
| Run a sync | `snp sync run` (`--push-only` / `--pull-only` / `--dry-run` / `--library`) |
| Tune auto-sync | `snp sync config` (auto-sync, debounce, max-delay, failure, timeout) |
| Recover from a failure | `snp sync retry`, `snp sync clear-failure`, `snp sync discard-pending`, `snp sync repair` |
| Register a device | `snp register` |
| Schedule sync | `snp cron` |

### 6.6 Data safety

| Capability | Commands |
|------------|----------|
| Snapshot | `snp backup` (secret-free, SHA-256 manifest) |
| Restore | `snp restore` (merge or replace) |
| Repair | `snp repair` (backed-up, idempotent) |

### 6.7 Integrate & operate

| Capability | Commands |
|------------|----------|
| Expose to AI agents | `snp mcp serve\|instructions\|install` |
| Shell completions | `snp completions bash\|zsh\|fish\|…` |
| Update | `snp update` (`--dry-run`, `--locked`) |

> `snp data` is a **mirror group**, not a separate implementation and not a
> Clap `alias`: `data validate|backup|restore|repair|status` route to the same
> handlers as the top-level commands. The single-path `handle_*` functions in
> `main.rs` keep the two spellings identical.

---

## 7. Cross-Cutting Invariants

These hold across many modules. They are the ones that break things quietly
when violated.

### Read-only paths must not mutate

A read-only command may not migrate metadata, create directories, or bump a
generation. Read paths use `resolve_selector_readonly` and
`library::readonly_library_sources`; the **mutating** `resolve_selector` and
`ensure_library_mode()` must not be reached from a read path. Picking the
wrong one reintroduces recovery side effects into plain `snp list`.

The split is not "read-only command" vs "mutating command" — it is per
*call site*. `resolve_exact_target` is in the **mutating** family (it calls
`resolve_selector`), so `snp run --id`, `snp clip --id`, and `snp edit --id`
**can** rewrite library metadata even though they are not writing snippet
data. `snp get` and MCP must never reach it. See [selector.md](selector.md).

### Gate every local mutation

`gate_mutation_on_interrupted_transactions()` runs before any local mutating
operation. One journal → auto-rollback. Multiple or incomplete journals →
refuse and direct the user to `snp repair`. Corrupt journals and
`RecoveryClass::UnsafeFailed` also refuse (fail-closed) **before** the
one-journal path. Journals live under `<config>/.transaction/`; transaction
APIs take `.transaction`, while pending-marker APIs take the state dir.

### Writes go through the atomic layer, with named durability classes

`atomic_replace` + `AtomicWriteOptions::for_durability()` is the only
sanctioned write for user data; `write_private_atomic` (no class argument)
handles snippet and library files. All four classes apply `mode(0o600)` to
the temp file; they differ in **fsync** behavior:

| Class | File fsync | Parent-dir fsync |
|-------|-------------|-----------------|
| `DurableUserData` | yes | required (hard error) |
| `SensitiveConfig` | yes (+ symlink rejection) | required (hard error) |
| `RecoverableMetadata` | no | best-effort (warns, continues) |
| `EphemeralCoordination` | no | none |

> **Windows hazard.** `parent_dir_sync_durable` returns `Err` on non-Unix
> (`src/utils/atomic.rs:180-186`), and `atomic_replace` calls
> `sync_parent_dir` unconditionally (`src/utils/atomic.rs:434`). So on Windows
> an `atomic_replace` with a durable class **replaces the target and then
> returns an error** — and never returns the `AtomicWriteReport`. A caller that
> treats `Err` as "not written" is wrong on Windows.
> `write_private_atomic` is unaffected (its dir sync is `#[cfg(unix)]`).

### Locks are kernel-backed and ordered

`flock` (Unix) / `LockFileEx` (Windows) via `process_file_lock.rs` — server
singleton, auto-sync execution/worker/pending locks. Owned-record +
PID-reclaim (`create_new` + `ProcessIdentity::observe` + quarantine) for the
transaction and local-data locks. The **kernel-backed** family releases on
`Drop` **without unlinking**, so lock files may hold stale metadata; the
**owned-record** family unlinks only when nonce, PID, **and** start token all
match. `kill(pid,0)`: only `ESRCH` proves absence — `EPERM`/unknown means
live. Linux start tokens use `/proc/<pid>/stat` field 22.

> **Known asymmetry.** The local-data lock's reclaim path re-reads and verifies
> ownership before quarantining; the **transaction lock's does not** — it
> renames by path alone (`src/transaction.rs:1268`). A reclaimer that judged an
> owner dead and then renamed can rename away a lock that was reclaimed and
> re-acquired in the interim, putting two processes inside the transaction
> critical section. See [process_file_lock.md](process_file_lock.md).

### Client conflicts are never role-dependent

Live conflicts order by `(updated_at, device_id, SHA-256(synced fields))`.
Deletion beats live content regardless of timestamp (no resurrection).
`output`, `folders`, and `favorite` are local-only and excluded from the
fingerprint — the wire type (`snip_proto::Snippet`, imported as `ProtoSnippet`
in `src/sync_commands.rs`) carries only `id`, `description`, `command`, `tags`,
`created_at`, `updated_at`, `device_id`, `deleted`, and `encrypted`.

**This is the CLIENT rule, and it is the only place it holds.** The server
uses a *different* rule: a SQL last-write-wins upsert on
`(updated_at, device_id, command, description)` with no SHA-256, where
`deleted` is an ordinary column (`snip-sync/src/db.rs:729-733`). A newer live
row therefore **overwrites a tombstone on the server** — the server does not
guarantee no resurrection. The no-resurrection guarantee lives in the client's
`choose_version` (`src/sync_commands.rs:1199`), so it only holds if the client
re-merges locally. See [server.md](server.md) and [sync.md](sync.md).

### The delta cursor is server-assigned, not client-chosen

The server's delta filter and the watermark it hands back run on
`snippets.visible_at`, a **server-assigned** cursor position, never on the
client-chosen `updated_at`. `visible_at` is clamped to the library's current
watermark on every write (`max(pushed, watermark)`) and never moves
backwards, so it is monotone by construction.

`updated_at` keeps the client's logical modification time verbatim: it is
what the server's last-write-wins upsert compares and what the client's
merge order uses. The two roles are deliberately separate — deriving the
cursor from `updated_at` let a device with a clock behind (or restored from
an old backup) push a row that no other device ever received, because the
row landed below the watermark that every delta pull filters on.

Existing databases are migrated in place (`migrate_snippet_visible_at`), and
the protocol is unchanged, so old and new clients interoperate in both
directions. See
[ADR-0001](../plans/adrs/ADR-0001-sync-delta-cursor.md) for the remaining
second-granularity wart.

### Server `snippets.id` is global, and a cross-library push is dropped

`snippets.id` is a global `TEXT PRIMARY KEY`, not per user/library. The upsert
is guarded by `WHERE excluded.user_id = snippets.user_id AND excluded.library_id
= snippets.library_id`, so pushing the same content-derived ID under a
different library updates nothing and inserts nothing — the snippet is dropped
for that library. The server probes the scope to tell this apart from a normal
last-write-wins dedupe (`snip-sync/src/db.rs:749-770`) and reports it as
`UpsertOutcome::ForeignScope` — rejected on the push path, `skipped_ids` on
the sync path. The write is still discarded, and because `last_sync` only
advances when `skipped_count == 0`, **a library that contains a content-copy
of a snippet in another library can never complete a sync until that content
changes.** See [server.md](server.md) and [sync.md](sync.md).

### Search has three surfaces and one definition

`snp get --query`, `snp list --filter`, and MCP `snippets_search` all share
`selector::searchable_text` — description + command always, tags by default,
output/notes only with `list --search-output` / MCP `search_output`.
Folders, favorite, sync metadata, and credentials are never searchable.
**Adding a field changes three surfaces with no compiler error.**

### The save path does not post-process TOML

`toml::to_string_pretty` output is written as-is. The golden corpus (tabs,
trailing spaces, CRLF) must survive save/load. `write_schema_version` uses
`toml::Table`, not `toml::Value`, to preserve array-of-tables.

### Snippet IDs are deterministic

`legacy-<sha256 hex>` derived from content + occurrence index — **not** UUID
v4. See `docs/IDENTITY_CONTRACT.md`.

Every ID a library file already carries is **reserved** before any
provisional ID is minted, so an ID that is already persisted is never handed
out again — not to a later snippet and not to an earlier one. Without that
reservation a snippet holding `legacy-<hash(content, 0)>` would not consume
occurrence index 0, and a sibling with the same content and an empty ID
would be handed the very same ID: two snippets sharing one identity, one of
them unreachable by ID and collapsed into a single row on the server.

### Tokio is initialized only where it is needed

The global `RUNTIME: LazyLock<Runtime>` is created only by async commands
(`run`, `clip`, `search`, `sync`, `register`, `premade`, `update`). Local-only
commands never initialize it. `run_snippet_selection` takes
`Option<&Runtime>` (`None` when `do_sync` is false). The detached worker uses
`new_current_thread()`.

### Uploads are byte-bounded

Client batches are sized by Prost `encoded_len()` — 3.5 MiB client ceiling
under the server's 4 MiB gRPC limit. `PushSnippets` is idempotent by snippet
identity. Multi-batch errors preserve the original `SyncFailureKind` through
`add_batch_context()`.

### Scheduling errors are typed

Pending-read and spawn failures are never collapsed into `NoPending`,
`SpawnNow`, or success. Pending generations are monotonic — a lower
generation is corrupt (preserve the marker, do not spawn), except
lower-generation + strictly newer timestamp, which clears and re-records the
marker as new work.

### Windows stack size

`.cargo/config.toml` sets MSVC link `/STACK:8388608` — the large `Commands`
enum overflows the 1 MB default. Do not remove it.

---

## 8. Primary Data Flows

### 8.1 Running a snippet

```
snp run [--filter FOO] [--sync]
  │
  ├─ main.rs::dispatch_command()
  │    └─ commands::run_cmd::run()
  │         ├─ gate_mutation_on_interrupted_transactions()
  │         ├─ load library via LibraryManager
  │         ├─ ui::select_snippet()        ← ratatui + crossterm
  │         │    ├─ fuzzy filter (SkimMatcherV2)
  │         │    ├─ keyboard navigation
  │         │    └─ variable prompting when the command has <name> placeholders
  │         ├─ expand_snippet_command()    → shell expansion
  │         ├─ Command::new(shell).arg(shell_arg_flag()).arg(cmd)
  │         │    └─ shell_arg_flag() = "-c" (Unix) / "/C" (Windows);
  │         │       $SHELL with a /bin/sh fallback — a test asserts both are accepted
  │         ├─ audit_log()                 → structured log
  │         └─ if --sync: sync_commands::run_default_sync()
  │              └─ sync::sync_encrypted()  → gRPC bidirectional merge
  │
  └─ on local mutation: auto_sync::notify_mutation()
       └─ spawn_worker()  → detached background sync (same SyncExecutionLock)
```

Snippet commands are **not** sanitized — by design. Removing CLI flags is a
breaking change (deprecate first). Clipboard writes normally go through
`copy_to_clipboard()` in `clip_cmd.rs`; two sites call
`clipboard::copy_to_clipboard_auto` directly (the TUI visual-range
multi-select, and `cron_cmd.rs`).

### 8.2 A local mutation (the durable path)

```
command mutates
  │
  ├─ gate_mutation_on_interrupted_transactions()      ← refuse if journals are broken
  ├─ acquire local-data lock  (owned record + PID reclaim + quarantine)
  ├─ transaction::begin_transaction()  → journal written to <config>/.transaction/
  ├─ stage files → commit → CleaningUp → journal removed
  └─ auto_sync::notify_mutation()
        ├─ write pending marker (monotonic generation)
        └─ schedule_sync()  → debounce → spawn detached worker
```

The journal state machine (`src/transaction.rs:307-354`) has nine variants —
`Prepared`, `BackupsDurable`, `Committing`, `CommittedLocal`, `Committed`,
`RollingBack`, `RolledBack`, `CleaningUp`, `Failed` — with two live paths:

```
Prepared → Committing{pos}  → CleaningUp{outcome, step} → (journal removed)
Prepared → RollingBack{pos} → CleaningUp{outcome, step} → (journal removed)
```

`BackupsDurable`, `Committed`, and `RolledBack` are **legacy** recovery-only
states (older journals); new code paths do not produce them. `Failed` is
defined and classified as `RecoveryClass::UnsafeFailed`, but is **never
written by any production path** — it exists only in the enum, the classifier,
and tests, so that gate arm is currently unreachable from new code.

`CleaningUp` is interruptible and restartable: each destructive step is
persisted before it runs, and recovery resumes from the last recorded
`next_step`. **The journal's absence is the terminal indicator** — a crash
mid-transaction leaves the journal behind, which is what the mutation gate
detects.

The local commit always completes before remote work begins. A remote failure
never rolls back a local commit.

### 8.3 A sync round trip

```
snp sync run  (or the detached auto-sync worker)
  │
  ├─ sync_commands::run_sync()
  │    ├─ pre-flight: resolve library, link server library if needed
  │    │     └─ on missing-library: atomic <library>.sync_recovery state
  │    ├─ build_upload_batches()  → byte-bounded via encoded_len()
  │    ├─ sync::sync_encrypted()  → gRPC + TLS + AES-256-GCM
  │    └─ merge_snippets()
  │          ├─ choose_version(): (updated_at, device_id, SHA-256) for live conflicts
  │          ├─ deletion wins outright; a local tombstone survives until acked
  │          └─ local-only fields (output/folders/favorite) are preserved
  │
  ├─ counters: pushed / failed / conflicts / skipped_count
  │     └─ skipped_count > 0 ⇒ the run FAILED (non-zero exit), not "conflict"
  └─ on success: advance last_sync, clear the pending marker
```

---

## 9. On-Disk State

Everything lives under `~/.config/snp/` (XDG). External library paths are not
supported — all libraries live in `~/.config/snp/libraries/`.

| Path | Purpose | Deep dive |
|------|---------|-----------|
| `snippets.toml` | Legacy single-file snippet storage | [persistence.md](persistence.md) |
| `libraries.toml` | Library metadata + sync links | [library.md](library.md) |
| `libraries/*.toml` | Individual library files | [library.md](library.md) |
| `premade/*.toml` | Downloaded premade libraries | [commands/premade_cmd.md](commands/premade_cmd.md) |
| `sync.toml` | Sync settings (CRC32 integrity header) | [config.md](config.md) |
| `themes/*.toml`, `themes.toml` | Halloy themes + active selection | [ui.md](ui.md) |
| `usage.toml` | Local usage metadata (never synced) | [usage.md](usage.md) |
| `auto-sync-status.toml` | Durable sync status (never synced) | [status.md](status.md) |
| `auto-sync-pending.toml` | Pending mutation marker | [auto_sync.md](auto_sync.md) |
| `.transaction/` | Transaction journals + locks | [persistence.md](persistence.md) |
| `backups/` | Backup snapshots | [commands/backup_cmd.md](commands/backup_cmd.md) |
| `logs/` | Rolling log files | [logging.md](logging.md) |
| `audit.log` | Audit trail | [logging.md](logging.md) |

Malformed `libraries.toml` / library files **fail closed** (best-effort
backup + error, never a synthesized writable empty). Missing or empty files
give defaults.

---

## 10. Testing Surface

51 integration test targets in `tests/`, plus 4 shared modules in
`tests/support/`: `TestEnvironment` (isolated TempDir + `XDG_CONFIG_HOME`
override), `RecordingServer`, `EventSink`. Tests never touch the real
config, keychain, or ports. `src/test_failpoints.rs` supplies the
`test-support`-gated `SNP_TEST_FAILPOINT` / `SNP_TEST_INJECT_ERROR` seams.

`SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on every spawned test command that
can reach the credential store — never remove that seam. (`pty_integration.rs`
and the `snp --version` spawn in `transaction_crash_recovery.rs` do not set
it; neither reaches sync settings.) `set_var` in tests needs `unsafe`
(edition 2024).

| Class | Execution | Representative targets |
|-------|-----------|------------------------|
| Unit / pure | parallel | `cargo test --workspace --lib` |
| CLI + platform smoke | parallel | `platform_smoke.rs`, `local_contracts.rs`, `cli_surface_compat.rs` |
| Contract suites | parallel | `sync_contracts.rs`, `auto_sync_closure.rs`, `backup_contracts.rs`, `selector_search_parity.rs`, `identity_contract.rs`, `schema.rs` |
| Sync integration | parallel | `sync_integration.rs` — in-process server, random port |
| PTY | **serial** | `pty_integration.rs` — real terminal pairs |
| Cross-process lock | **serial** | `process_lock_concurrency.rs` — kernel `flock` |
| Barrier-coordinated | **serial** | `local_data_lock_barriers.rs`, `repair_transactions.rs` |
| Multi-batch sync | **serial** | `sync_multibatch.rs` |
| Auto-sync concurrency | **serial** | `auto_sync_concurrency.rs` |
| Deep recovery | release only | `transaction_crash_recovery.rs` (`release-check.sh verify`) |
| Architecture | parallel | `architecture.rs` — source-scanning layer enforcement |

`check.sh` passes `--features test-support` to the focused targets; only
`repair_transactions`, `process_lock_concurrency`, `local_data_lock_barriers`,
`identity_contract` (and the `process_lock_helper` bin) gate compilation on it.

**Every test seam is `test-support`-gated and compiled out of production
builds.** The eight are `SNP_TEST_EVENTS_DIR`, `SNP_TEST_FAILPOINT`,
`SNP_TEST_INJECT_ERROR`, `SNP_TEST_MUTATION_BARRIER_DIR`,
`SNP_SKIP_WORKER_SPAWN`, `SNP_ALLOW_DIR_FSYNC_FAILURE`,
`SNP_ALLOW_PLAINTEXT_API_KEY`, and `SNP_TEST_CREDENTIAL_FILE`. Setting an env
var in a production binary does nothing —
`scripts/ci/test-production-seams.sh` proves it by grepping the production
binary for the variable name.

**Test isolation is not optional.** `save_library` derives the mutation gate
and the local-data-lock path from the **config directory**, not from the target
library path. Any test that calls it in-process must redirect the config dir —
either with `XDG_CONFIG_HOME` on a subprocess, or with the thread-local
`ScopedConfigDir` override (exported from the crate root under `test-support`).
Without that, a test writes to the developer's real `~/.config/snp/`, contends
on the real lock, and can be refused outright by the gate if an interrupted
transaction journal happens to be on disk.

Full detail: [test-infrastructure.md](test-infrastructure.md). Task-shaped
rules for writing and running tests — isolation, the eight seams, serial
targets, assertion discipline, what CI does *not* run — are in
[`../.skills/testing-and-seams.md`](../.skills/testing-and-seams.md).

---

## Deep-Dive Index

One document per discrete component. Each is written to be audited against.

**Picking a deep dive for a review.** Read the doc for the component you are
touching, then the "Gotchas" section at its end — that is where the traps live.

| If you are reviewing… | Start with | Then read |
|----------------------|-------------|-----------|
| A new subcommand or flag | [cli.md](cli.md), [commands/mod.md](commands/mod.md) | [outcome.md](outcome.md), the command's own doc |
| Anything that writes snippet data | [persistence.md](persistence.md) | [process_file_lock.md](process_file_lock.md), [utils/atomic.md](utils/atomic.md) |
| Anything that touches sync | [sync.md](sync.md) | [encryption.md](encryption.md), [auto_sync.md](auto_sync.md), [server.md](server.md) |
| Search, filtering, or selectors | [selector.md](selector.md) | [sort.md](sort.md), [mcp.md](mcp.md) |
| The TUI or themes | [tui.md](tui.md) | [ui.md](ui.md), [clipboard.md](clipboard.md) |
| The agent-facing surface | [mcp.md](mcp.md) | [selector.md](selector.md) (search parity) |
| Locking, crashes, or recovery | [persistence.md](persistence.md) | [auto_sync.md](auto_sync.md), [process_file_lock.md](process_file_lock.md) |
| Layer/dependency questions | [core.md](core.md) | `../docs/LOGICAL_LAYERS.md` (enforced by `tests/architecture.rs`) |

### CLI, core & library

| File | Subject |
|------|---------|
| [cli.md](cli.md) | CLI surface, dispatch, startup recovery, exit paths |
| [core.md](core.md) | Core types, `SnipError`, key abstractions |
| [outcome.md](outcome.md) | `CliOutcome` — exit codes and machine output |
| [public_api.md](public_api.md) | The library crate's supported public surface |
| [usage.md](usage.md) | Local usage metadata and update policy |

### Core data

| File | Subject |
|------|---------|
| [library.md](library.md) | Data structures, TOML persistence, `LibraryManager` |
| [selector.md](selector.md) | Deterministic resolution and fuzzy search fields |
| [sort.md](sort.md) | Sort modes, ranking, tie-break chain |
| [output.md](output.md) | Snippet output-field rendering |
| [diagnostics.md](diagnostics.md) | Shared diagnostic/report types |

### Persistence & locking

| File | Subject |
|------|---------|
| [persistence.md](persistence.md) | Atomic writes, transactions, validation, backup/restore/repair, migration, identity |
| [process_file_lock.md](process_file_lock.md) | Kernel-backed cross-process file lock |

### Sync

| File | Subject |
|------|---------|
| [sync.md](sync.md) | Sync client, merge logic, conflict resolution, failure classification |
| [encryption.md](encryption.md) | AES-256-GCM + Argon2id snippet-body encryption, and the trust boundary |
| [config.md](config.md) | Sync settings, keychain API key, TOML cache |
| [auto_sync.md](auto_sync.md) | Auto-sync policy, worker, debounce, pending markers, locks |
| [status.md](status.md) | Status snapshot, `snp status`, diagnostic codes |
| [proto.md](proto.md) | Protobuf schema and the gRPC service spec |

### Server

| File | Subject |
|------|---------|
| [server.md](server.md) | snip-sync: gRPC, HTTP leaf, database, lifecycle, security |

### User interface

| File | Subject |
|------|---------|
| [tui.md](tui.md) | TUI event loop, state machine, variables, keybindings |
| [ui.md](ui.md) | Theme system, syntax highlighting, bundled themes |
| [clipboard.md](clipboard.md) | Cross-platform clipboard access |

### Commands

| File | Subject |
|------|---------|
| [commands/mod.md](commands/mod.md) | Shared command helpers and path resolution |
| [commands/new_cmd.md](commands/new_cmd.md) | Snippet creation |
| [commands/list_cmd.md](commands/list_cmd.md) | Text-based snippet listing |
| [commands/get_cmd.md](commands/get_cmd.md) | Deterministic non-TUI retrieval |
| [commands/search_cmd.md](commands/search_cmd.md) | Fuzzy search with detail display |
| [commands/select_cmd.md](commands/select_cmd.md) | Non-executing selection |
| [commands/run_cmd.md](commands/run_cmd.md) | TUI selection + shell execution |
| [commands/clip_cmd.md](commands/clip_cmd.md) | Copy to clipboard |
| [commands/edit_cmd.md](commands/edit_cmd.md) | Edit in `$EDITOR` |
| [commands/validate_cmd.md](commands/validate_cmd.md) | Read-only data validation |
| [commands/repair_cmd.md](commands/repair_cmd.md) | Conservative repair |
| [commands/restore_cmd.md](commands/restore_cmd.md) | Restore from a backup snapshot |
| [commands/backup_cmd.md](commands/backup_cmd.md) | Secret-free backup snapshot |
| [commands/backup_archive.md](commands/backup_archive.md) | Archive layout, manifest, integrity |
| [commands/status_cmd.md](commands/status_cmd.md) | Auto-sync status display |
| [commands/sync_cmd.md](commands/sync_cmd.md) | Sync and config subcommands |
| [commands/cron_cmd.md](commands/cron_cmd.md) | Crontab generation |
| [commands/register_cmd.md](commands/register_cmd.md) | Device registration |
| [commands/library_cmd.md](commands/library_cmd.md) | Library management |
| [commands/premade_cmd.md](commands/premade_cmd.md) | Premade library access |
| [commands/import_cmd.md](commands/import_cmd.md) | Pet snippet file import |
| [commands/doctor_cmd.md](commands/doctor_cmd.md) | Diagnostics and environment audit |
| [commands/doctor_report.md](commands/doctor_report.md) | Doctor human/JSON rendering |
| [commands/pet_analysis.md](commands/pet_analysis.md) | Pet file reading and field detection |
| [commands/shell_cmd.md](commands/shell_cmd.md) | Shell integration code generation |
| [commands/keybindings_cmd.md](commands/keybindings_cmd.md) | TUI keybindings reference |

### Integrations & utilities

| File | Subject |
|------|---------|
| [mcp.md](mcp.md) | Read-only stdio MCP server, tools, search parity |
| [update.md](update.md) | Self-update — eggfetch client, `curl` server, install methods |
| [logging.md](logging.md) | Structured logging, audit trail, panic handler |
| [utils.md](utils.md) | Utility module inventory |
| [utils/config.md](utils/config.md) | Config directory resolution, path helpers |
| [utils/variables.md](utils/variables.md) | Variable parsing and expansion |
| [utils/toml_helpers.md](utils/toml_helpers.md) | TOML escape sequence handling |
| [utils/shell_keywords.md](utils/shell_keywords.md) | Shell command names for syntax highlighting |
| [utils/tempfile_guard.md](utils/tempfile_guard.md) | RAII temporary file cleanup |
| [utils/atomic.md](utils/atomic.md) | Atomic file writes with durability guarantees |
| [utils/process.md](utils/process.md) | Process liveness and owned-lock-file removal |
| [utils/redact.md](utils/redact.md) | Secret redaction for logs and backups |

### Testing

| File | Subject |
|------|---------|
| [test-infrastructure.md](test-infrastructure.md) | Deterministic E2E test infrastructure |

### Reference (outside this directory)

| File | Subject |
|------|---------|
| [`../docs/README.md`](../docs/README.md) | **Index of reference docs** — current contracts vs archived snapshots |
| [`../docs/USER_GUIDE.md`](../docs/USER_GUIDE.md) | End-user walkthrough: libraries, variables, import, shell integration, sync |
| [`../docs/COMMANDS.md`](../docs/COMMANDS.md) | Every `snp` subcommand and its flags (`snp --help` is authoritative) |
| [`../docs/LOGICAL_LAYERS.md`](../docs/LOGICAL_LAYERS.md) | Target layer architecture (enforced by `tests/architecture.rs`) |
| [`../docs/PUBLIC_API.md`](../docs/PUBLIC_API.md) | Stable public surface of `src/lib.rs` |
| [`../docs/EXIT_CODES.md`](../docs/EXIT_CODES.md) | Exit codes 0–10 and the `CliOutcome` mapping |
| [`../docs/CLI_EXITCODE_STREAM_POLICY.md`](../docs/CLI_EXITCODE_STREAM_POLICY.md) | stdout/stderr discipline and easy-to-miss exit cases |
| [`../docs/COMMAND_CONTRACTS.md`](../docs/COMMAND_CONTRACTS.md) | Per-command behavioral contract |
| [`../docs/JSON_SCHEMAS.md`](../docs/JSON_SCHEMAS.md) | Machine-readable JSON envelopes |
| [`../docs/LIBRARY_SCOPE.md`](../docs/LIBRARY_SCOPE.md) | `--library` resolution and the selector vs single-library paths |
| [`../docs/IDENTITY_CONTRACT.md`](../docs/IDENTITY_CONTRACT.md) | Deterministic snippet IDs, library identity |
| [`../docs/PERSISTENCE_INVENTORY.md`](../docs/PERSISTENCE_INVENTORY.md) | Every persisted artifact, path, durability class |
| [`../docs/MCP.md`](../docs/MCP.md) | Read-only stdio MCP tools and protocol |
| [`../docs/THREAT_MODEL.md`](../docs/THREAT_MODEL.md) | Assets, adversaries, trust boundaries |
| [`../docs/COMPATIBILITY.md`](../docs/COMPATIBILITY.md) | Backward-compatibility and deprecation rules |
| [`../docs/SUPPLY_CHAIN_POLICY.md`](../docs/SUPPLY_CHAIN_POLICY.md) | Dependency, license, `cargo-deny` policy |
| [`../docs/PET_COMPATIBILITY.md`](../docs/PET_COMPATIBILITY.md) | Behavioral matrix vs. `pet` import |
| [`../AGENTS.md`](../AGENTS.md) | Authoritative verify commands, gotchas, invariants |
| [`../AGENTS.override.md`](../AGENTS.override.md) | Session pitfall notes |
| [`../plans/registry.md`](../plans/registry.md) | Authoritative planning status |
| [`../.skills/`](../.skills) | Task-shaped agent skills (14) — start with `planning.md`, then `architecture-review.md`; `testing-and-seams.md` and `documentation-authoring.md` cover working in this repo |

# Logical Layer Architecture

**Created:** Phase 06A Workstream B & C
**Status:** Documentation-only (no file moves yet)

## Overview

The snip-it codebase is organized into three logical layers with strict dependency direction:

```
┌─────────────────────────────────────────────┐
│           Application / CLI Layer           │
│  Clap, TUI, clipboard, shell, editor,       │
│  worker/executor, lock/pending/status,      │
│  runtime, rendering, update                 │
├─────────────────────────────────────────────┤
│           Sync-Client Layer                 │
│  gRPC client, encryption framing,           │
│  sync request/options/report, merge,        │
│  direction, credential-provider,            │
│  typed sync errors                          │
├─────────────────────────────────────────────┤
│           Domain / Core Layer               │
│  Snippet/library models, identifiers,       │
│  TOML representations, variable parsing,    │
│  filtering/sorting, import/export,          │
│  usage metadata, validation,                │
│  local persistence                          │
└─────────────────────────────────────────────┘
```

**Dependency rule:** `application → sync-client → core` and `application → core`.
No reverse dependencies. No `core → app` or `core → sync-client`.

---

## Domain / Core Layer

Pure data models, persistence, and domain logic. No I/O beyond local filesystem.
No platform dependencies (no keyring, no tonic, no clipboard, no process spawning).

| Module | Responsibility |
|--------|---------------|
| `src/library.rs` | Snippet/Snippets/LibraryManager/LibraryMeta data structures, TOML persistence, backup |
| `src/sort.rs` | SnippetSort enum, rank_snippets(), SortOptions — deterministic ranking |
| `src/usage.rs` | UsageIndex, UsageData — local-only per-snippet usage tracking |
| `src/diagnostics.rs` | CompatibilityDiagnostic, PetImportReport, DoctorReport — import/doctor models |
| `src/output.rs` | OutputPresentation — safe terminal rendering of snippet output metadata |
| `src/error.rs` | SnipError, SnipResult, SyncFailureKind — typed error categories |
| `src/utils/variables.rs` | Variable parsing, expansion, <name=default> syntax |
| `src/utils/shell_keywords.rs` | Shell keyword detection for syntax highlighting |
| `src/utils/atomic.rs` | Atomic file writes with private permissions |
| `src/utils/config.rs` | Config directory paths, XDG resolution |
| `src/utils/toml_helpers.rs` | TOML escape fixing, backslash quoting |

**Core layer dependencies (allowed):**
- `crate::error` (core)
- `crate::utils::*` (core)
- `crate::config::cached_read_toml` / `invalidate_toml_cache` (shared utility, see note below)
- `crate::usage` (core)

**Core layer must NOT depend on:**
- `crate::clipboard`, `crate::ui`, `crate::logging`
- `crate::sync`, `crate::encryption`, `crate::proto`
- `tonic`, `keyring`, `arboard`, `ratatui`, `crossterm`
- `std::process::Command` (process spawning)

---

## Sync-Client Layer

Protocol client, encryption, sync orchestration. Depends on core but not on application.

| Module | Responsibility |
|--------|---------------|
| `src/encryption.rs` | AES-256-GCM + Argon2id encryption, key derivation, key cache |
| `src/proto.rs` | Prost-generated protobuf types (Snippet, SyncRequest, etc.) |
| `src/sync.rs` | SyncClient (tonic gRPC), retry logic, encrypt/decrypt snippets |
| `src/sync_commands.rs` | Sync orchestration, merge logic (last-write-wins), run_sync() |
| `src/config.rs` | SyncSettings, SyncDirection, API key (keychain), sync config persistence |

**Sync-client layer dependencies (allowed):**
- `crate::error`, `crate::library`, `crate::utils::*` (core)
- `crate::encryption`, `crate::proto` (sync-client)
- `tonic` (gRPC transport)
- `keyring` (credential storage — platform dependency, isolated to config.rs)

**Sync-client layer must NOT depend on:**
- `crate::clipboard`, `crate::ui`, `crate::logging`
- `crate::commands`, `crate::auto_sync`
- `ratatui`, `crossterm`, `arboard`

**Known issue:** `config.rs:save_sync_settings()` calls `crate::clipboard::invalidate_clipboard_settings_cache()`.
This is a reverse dependency (sync-client → application) that should be resolved by moving
the invalidation call to the caller or using a callback/event pattern.

---

## Application / CLI Layer

Everything that touches the terminal, spawns processes, or orchestrates user workflows.

| Module | Responsibility |
|--------|---------------|
| `src/main.rs` | CLI entry point, clap dispatch |
| `src/lib.rs` | Library crate exports |
| `src/commands/*` | 16 command modules (new, list, run, clip, select, search, edit, sync, register, library, premade, import, doctor, cron, shell, keybindings, status) |
| `src/clipboard.rs` | Cross-platform clipboard access (arboard/clipboard-win) |
| `src/logging.rs` | Structured logging with file rotation, audit log |
| `src/update.rs` | Self-update (crates.io, Homebrew, GitHub releases) |
| `src/status_snapshot.rs` | Read-only status projection for `snp status` and doctor |
| `src/ui/*` | TUI (ratatui + crossterm), themes, syntax highlighting, variable prompts |
| `src/auto_sync/*` | Auto-sync subsystem (policy, pending, lock, executor, worker, spawn, notification, status, schedule) |

**Application layer dependencies (allowed):**
- Everything in core and sync-client layers
- `ratatui`, `crossterm`, `arboard` (TUI and clipboard)
- `std::process::Command` (process spawning)
- `tokio` (async runtime)

---

## Dependency Violations Found

### 1. config.rs → clipboard (sync-client → application)

**Location:** `src/config.rs:521`
```rust
crate::clipboard::invalidate_clipboard_settings_cache();
```

**Impact:** Sync-client layer depends on application layer. The `config` module is
imported by core modules (`library.rs`) and sync modules, creating a transitive
dependency on clipboard.

**Resolution options:**
1. Move `invalidate_clipboard_settings_cache` to a shared utility or event bus
2. Have callers of `save_sync_settings()` handle clipboard invalidation
3. Use a trait/callback pattern so config doesn't know about clipboard

### 2. error.rs → encryption (core → sync-client)

**Location:** `src/error.rs:295-302`
```rust
impl From<crate::encryption::CryptoError> for SnipError {
    fn from(e: crate::encryption::CryptoError) -> Self { ... }
}
```

**Impact:** Core error type depends on sync-client encryption type. This is a minor
violation but acceptable for now since `CryptoError` is a simple enum with no
external dependencies. Could be resolved by moving `CryptoError` to core or using
a generic error variant.

---

## Target Directory Structure (Future)

When the crate is physically restructured:

```
src/
├── core/              # Domain / Core layer
│   ├── library.rs
│   ├── sort.rs
│   ├── usage.rs
│   ├── diagnostics.rs
│   ├── output.rs
│   ├── error.rs
│   └── utils/
├── sync_client/       # Sync-Client layer
│   ├── encryption.rs
│   ├── proto.rs
│   ├── sync.rs
│   ├── sync_commands.rs
│   └── config.rs
├── app/               # Application / CLI layer
│   ├── commands/
│   ├── clipboard.rs
│   ├── logging.rs
│   ├── update.rs
│   ├── status_snapshot.rs
│   ├── ui/
│   └── auto_sync/
├── platform/          # Platform-specific code (future)
│   └── keychain.rs
└── lib.rs             # Facade: pub fn run_cli() -> ExitCode
```

---

## Verification Checklist

- [x] `library.rs` depends only on: `config` (utility), `error`, `utils::*`
- [x] `sort.rs` depends only on: `usage`, `library` (data types only)
- [x] `usage.rs` depends only on: `error`, `utils::*`
- [x] `diagnostics.rs` has zero crate dependencies (pure data)
- [x] `output.rs` has zero crate dependencies (pure data)
- [x] `error.rs` depends only on: `encryption::CryptoError` (minor violation)
- [x] `encryption.rs` has zero crate dependencies (pure crypto)
- [ ] `config.rs` depends on `crate::clipboard` (violation — see above)
- [x] No core module uses `tonic`, `keyring`, `ratatui`, `crossterm`, `arboard`
- [x] No core module uses `std::process::Command`
- [x] No core module uses `crate::ui`, `crate::logging`, `crate::commands`

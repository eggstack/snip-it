# Logical Layer Architecture

**Status:** Enforced. `tests/architecture.rs` is the executable form of this
document — it scans `src/` and fails on a lower layer gaining a dependency on a
higher one. **When you add or move a module, update this file and the test's
constant lists together**, or one of them is lying.

## Overview

The snp-it codebase is organized into three logical layers with strict dependency direction:

```
┌─────────────────────────────────────────────┐
│           Application / CLI Layer           │
│  Clap, TUI, clipboard, shell, editor,       │
│  worker, lock/pending/status,               │
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

## What the test actually enforces

`tests/architecture.rs` checks exactly these lists. This is the authoritative
boundary; the tables below are explanatory.

| Enforced set | Members | Forbidden imports |
|--------------|---------|-------------------|
| **Core** | `library/` (the whole directory), `sort.rs`, `output.rs`, `usage.rs`, `diagnostics.rs` | `crate::commands`, `crate::ui`, `crate::logging`, `crate::auto_sync`, `crate::clipboard`, `crate::sync_commands`, `crate::sync` |
| **Sync-Client** | `sync.rs`, `sync_commands.rs`, `encryption.rs` | `crate::commands`, `crate::ui`, `crate::logging`, `crate::auto_sync`, `crate::clipboard` |

Two deliberate gaps, both annotated in the test source:

1. **`config/` is not in the sync-client list.** It carries a known cross-layer
   call — `save_sync_settings()` → `crate::clipboard::invalidate_clipboard_settings_cache()`
   (`src/config/sync_settings.rs:501`) — that predates the module split. Adding
   `config/` to `SYNC_CLIENT_MODULES` would fail on that exception. See
   "Known cross-layer exceptions".
2. **`crate::config` is allowed from Core**, because core modules use only its
   pure TOML-caching helpers (`cached_read_toml`, `invalidate_toml_cache`). The
   sync/keychain parts of `config` are not imported by core.

Modules not named in either list (`selector.rs`, `process_file_lock.rs`,
`utils/*`, `record_lock.rs`, `transaction.rs`, `local_data.rs`, `error.rs`,
`outcome.rs`, `sync_failure.rs`) are governed by convention, not by the
scanner. Keep them conservative anyway.

---

## Domain / Core Layer

Pure data models, persistence, and domain logic. No I/O beyond local filesystem.
No platform dependencies (no keyring, no tonic, no clipboard, no process spawning).

| Module | Responsibility |
|--------|---------------|
| `src/library/` | Split into `model.rs` (Snippet/Snippets/LibraryMeta), `persistence.rs` (load/save, ID normalization), `manager.rs` (`libraries.toml` registry, resolver) |
| `src/sort.rs` | SnippetSort enum, `rank_snippets()`, SortOptions — deterministic ranking |
| `src/usage.rs` | UsageIndex, UsageData — local-only per-snippet usage tracking |
| `src/diagnostics.rs` | CompatibilityDiagnostic, PetImportReport, DoctorReport — import/doctor models |
| `src/output.rs` | OutputPresentation — safe terminal rendering of snippet output metadata |
| `src/error.rs` | SnipError, SnipResult, SyncFailureKind — typed error categories |
| `src/outcome.rs` | CliOutcome enum, exit-code mapping, machine-output guard |
| `src/selector.rs` | SnippetSelector, ResolutionPolicy — deterministic non-TUI snippet resolution |
| `src/process_file_lock.rs` | Kernel-backed cross-process file lock (`flock`/`LockFileEx`) |
| `src/record_lock.rs` | Owned-record lock protocol (`create_new` + ownership record + conservative PID-reuse reclaim), with the wait policy as a parameter |
| `src/utils/variables.rs` | Variable parsing, expansion, `<name=default>` syntax |
| `src/utils/shell_keywords.rs` | Shell keyword detection for syntax highlighting |
| `src/utils/atomic.rs` | Atomic file writes with durability classes |
| `src/utils/config.rs` | Config directory paths, XDG resolution |
| `src/utils/toml_helpers.rs` | TOML escape fixing, backslash quoting |
| `src/utils/process.rs` | Process liveness checks, owned-lock-file removal |
| `src/utils/redact.rs` | Secret redaction for log, status, and error messages |
| `src/utils/tempfile_guard.rs` | RAII cleanup of temporary files |

**Core layer dependencies (allowed):**
- `crate::error` (core)
- `crate::utils::*` (core)
- `crate::config::cached_read_toml` / `invalidate_toml_cache` (shared utility, see note above)
- `crate::usage` (core)
- `crate::test_failpoints` (compile-time no-op in production; used by
  `library/persistence.rs` as the mutation barrier)

**Core layer must NOT depend on:**
- `crate::clipboard`, `crate::ui`, `crate::logging`, `crate::commands`
- `crate::auto_sync`, `crate::sync`, `crate::sync_commands`, `snip-proto` crate
- `tonic`, `keyring`, `arboard`, `ratatui`, `crossterm`
- `std::process::Command` (process spawning)

*Only the `crate::` path entries are scanner-enforced; the external-crate and
`std` entries are convention — see the Verification Checklist.*

---

## Sync-Client Layer

Protocol client, encryption, sync orchestration. Depends on core but not on application.

| Module | Responsibility |
|--------|---------------|
| `src/encryption.rs` | AES-256-GCM + Argon2id encryption, key derivation, key cache |
| `src/sync_failure.rs` | FailureClass — sync-client error classification consumed by auto-sync policy |
| `snip-proto` crate | Prost-generated protobuf types (Snippet, SyncRequest, etc.), checked-in stubs |
| `src/sync.rs` | SyncClient (tonic gRPC), `retry_grpc_unified!` retry, encrypt/decrypt snippets |
| `src/sync_commands.rs` | Sync orchestration, merge logic, `run_sync()` |
| `src/config/` | SyncSettings, SyncDirection, API key (keychain), sync config persistence |

**Sync-client layer dependencies (allowed):**
- `crate::error`, `crate::library`, `crate::utils::*` (core)
- `crate::encryption`, `crate::sync_failure`, `snip-proto` (sync-client)
- `tonic` (gRPC transport)
- `keyring` (credential storage — platform dependency, isolated to `config/sync_settings.rs`)

**Sync-client layer must NOT depend on:**
- `crate::clipboard`, `crate::ui`, `crate::logging`
- `crate::commands`, `crate::auto_sync`
- `ratatui`, `crossterm`, `arboard`

*Only the `crate::` path entries are scanner-enforced; the external-crate
entries are convention — see the Verification Checklist.*

---

## Application / CLI Layer

Everything that touches the terminal, spawns processes, or orchestrates user workflows.

| Module | Responsibility |
|--------|---------------|
| `src/main.rs` | CLI entry point, clap dispatch, `command_behavior()` policy table |
| `src/lib.rs` | Library crate exports (stable public surface) |
| `src/commands/` | 27 modules: `mod`, `new`, `list`, `run`, `clip`, `select`, `search`, `edit`, `get`, `delete`, `sync`, `register`, `library`, `premade`, `import`, `doctor`, `doctor_report`, `cron`, `shell`, `keybindings`, `status`, `validate`, `backup`, `backup_archive`, `restore`, `repair`, `pet_analysis` |
| `src/clipboard.rs` | Cross-platform clipboard access (arboard/clipboard-win) |
| `src/logging.rs` | Structured logging with file rotation, audit log |
| `src/update.rs` | Self-update via lean in-process `eggfetch-core` |
| `src/status_snapshot.rs` | Read-only status projection for `snp status` and doctor |
| `src/transaction.rs` | Transaction boundary with journal, begin/commit/rollback; delegates the lock protocol to `src/record_lock.rs` |
| `src/local_data.rs` | Local data lock coordination, `transaction_dir()`; delegates the lock protocol to `src/record_lock.rs` |
| `src/mcp/` | Read-only stdio MCP server (`mod`, `protocol`, `tools`, `client_install`) |
| `src/ui/` | TUI (ratatui + crossterm), themes, syntax highlighting, variable prompts |
| `src/auto_sync/` | Auto-sync subsystem (`execution_lock`, `lock`, `notification`, `pending`, `pending_lock`, `policy`, `schedule`, `status`, `test_events`, `worker`) |

**Application layer dependencies (allowed):**
- Everything in core and sync-client layers
- `ratatui`, `crossterm`, `arboard` (TUI and clipboard)
- `std::process::Command` (process spawning)
- `tokio` (async runtime)

---

## Known cross-layer exceptions

Two acknowledged deviations. Neither is fixed; both are documented so a future
agent doesn't "discover" them as bugs.

### 1. `config/` → `clipboard` (sync-client → application)

**Location:** `src/config/sync_settings.rs:501`
```rust
crate::clipboard::invalidate_clipboard_settings_cache();
```

**Impact:** The `config` module is imported by core and sync modules, so this
creates a transitive dependency on `clipboard` from below the application layer.
This is why `config/` is excluded from `SYNC_CLIENT_MODULES` in the test.

**Resolution options (none taken):**
1. Move `invalidate_clipboard_settings_cache` to a shared utility or event bus
2. Have callers of `save_sync_settings()` handle clipboard invalidation
3. Use a trait/callback pattern so config doesn't know about clipboard

### 2. `error.rs` → `encryption` (core → sync-client)

**Location:** `src/error.rs:316` — `impl From<CryptoError> for SnipError`

**Impact:** The core error type names a sync-client type. Accepted: `CryptoError`
is a simple enum with no external dependencies, and the conversion is what makes
crypto failures classify as `FailureClass::Internal` for retry policy. Could be
resolved by moving `CryptoError` to core or using a generic error variant.

---

## Verification Checklist

### Enforced by `tests/architecture.rs`

These are the assertions the scanner actually makes. The layer checks match
**crate-root `crate::` path prefixes** on any non-comment line, so they catch
a dependency, not a specific import form. External crates and `std` items are
**not** matched.

- [x] Core modules (`library/`, `sort.rs`, `output.rs`, `usage.rs`,
  `diagnostics.rs`) do not reference `crate::commands`, `crate::ui`,
  `crate::logging`, `crate::auto_sync`, `crate::clipboard`,
  `crate::sync_commands`, or `crate::sync`
- [x] Sync-client modules (`sync.rs`, `sync_commands.rs`, `encryption.rs`) do
  not reference `crate::commands`, `crate::ui`, `crate::logging`,
  `crate::auto_sync`, or `crate::clipboard`
- [x] `clipboard`, `diagnostics`, `encryption`, `library`, `output`,
  `status_snapshot`, `sync_commands`, and `utils` are `pub(crate)` (not `pub`)
  in `src/lib.rs`
- [x] The owned-record lock acquisition protocol exists in exactly one module in `src/`
- [x] `is_transient_lock_contention` is defined in exactly one module in `src/`
- [x] No reference is the operand of the final link of a raw-pointer cast in
  `src/` or `snip-sync/src/` — a `cfg`-gated platform build cannot see such an
  error on Linux, and the macOS-only `E0606` it causes reaches `main` unseen
- [x] `src/update.rs` does not shell out to `curl`
- [x] `Cargo.toml` pins `eggfetch-core = "=0.2.2"` with the
  `standard-http1` + `redirects` features
- [x] `src/update.rs` retains no manual redirect loop and no outer Tokio
  timeout

### Manual / not test-enforced

These hold today, but **no assertion in `tests/architecture.rs` checks them**.
The scanner matches `crate::` path prefixes only — it does not inspect
external crate usage, `std` items, or dependency *allowlists*. Keep them true
by review.

- [ ] `library/` depends only on: `config` (TOML cache only), `error`, `utils::*`,
  `test_failpoints` (mutation barrier only)
- [ ] `sort.rs` depends only on: `usage`, `library` (data types only)
- [ ] `usage.rs` depends only on: `error`, `utils::*`
- [ ] `diagnostics.rs` has zero crate dependencies (pure data)
- [ ] `output.rs` has zero crate dependencies (pure data)
- [ ] `outcome.rs` has zero crate dependencies (pure data + exit-code mapping)
- [ ] `encryption.rs` has zero crate dependencies (pure crypto)
- [ ] No core module uses `tonic`, `keyring`, `ratatui`, `crossterm`, `arboard`, or the `snip-proto` crate
- [ ] No core module uses `std::process::Command`

### Documented exceptions (facts about the code)

- [x] `config/` depends on `crate::clipboard` — accepted exception, see above
- [x] `error.rs` depends on `encryption::CryptoError` — accepted exception, see above

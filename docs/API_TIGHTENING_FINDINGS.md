# API Tightening Findings (Phase 06A)

Workstreams E, F, G, H analysis for the snip-it Rust project.

---

## Workstream E: Data Model Tightening

### Parallel Vectors (Index-Correlated)

`SnippetData` (`src/lib.rs:46-53`) uses **explicitly index-correlated parallel vectors**:

```rust
pub struct SnippetData {
    pub descriptions: Vec<String>,
    pub commands: Vec<String>,
    pub outputs: Vec<String>,
    pub tags: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub favorites: Vec<bool>,
}
```

This is documented in the comment: *"Contains parallel vectors of snippet metadata where index `i` corresponds to the same snippet across all fields."* This is a known pattern consumed by the TUI layer. **No structural fix needed** — the parallel vectors are a deliberate performance choice for the TUI selector (avoids cloning full `Snippet` structs per row).

### `SnippetId` Newtype

**Not present.** Snippet IDs are `String` (`src/library.rs:46`). IDs are UUIDs generated at load time via `uuid::Uuid::new_v4()` (`src/library.rs:698-701`). A `SnippetId` newtype would improve type safety but is a **low-priority item** — IDs are primarily used as opaque strings in TOML serialization and gRPC transport.

### Field Visibility

- **`Snippet`**: All 11 fields are `pub` (`src/library.rs:44-74`). Acceptable for a data-transfer struct with `#[derive(Serialize, Deserialize)]`.
- **`Snippets`**: Both fields (`snippets`, `folders`) are `pub` (`src/library.rs:29-36`).
- **`LibraryMeta`**: All 5 fields are `pub` (`src/library.rs:90-100`).
- **`LibraryConfig`**: `libraries` field is `pub` (`src/library.rs:82`).

All are in `pub(crate) mod library` (`src/lib.rs:30`), so they're hidden from external crates. **No immediate concern.**

### `#[non_exhaustive]` on Public Enums

Three enums have `#[non_exhaustive]`:
- `SnipError` (`src/error.rs:94`) ✅
- `CryptoError` (`src/encryption.rs:85`) ✅
- `SyncDirection` (`src/config.rs:472`) ✅

**Missing `#[non_exhaustive]`** (likely to grow):
- `ProcessResult` (`src/lib.rs:56`) — may gain new states
- `CommandOutcome` (`src/lib.rs:66`) — may gain error detail variants
- `SelectionOutcome` (`src/lib.rs:78`) — may gain partial-selection states
- `SnippetSort` (`src/sort.rs:34`) — will grow with new sort modes
- `AutoSyncFailureMode` (`src/config.rs:44`) — may gain new modes
- `FailureClass` (`src/auto_sync/policy.rs`) — 11 variants, may grow
- `ExecutorExitCode` (`src/auto_sync/executor.rs:24`) — may grow
- `WorkerOutcome` (`src/auto_sync/worker.rs:28`) — may grow
- `SpawnError` (`src/auto_sync/spawn.rs:16`) — may grow

---

## Workstream F: Error Boundary Cleanup

### Error Variant Count

**`SnipError`** (`src/error.rs:95-143`): **6 variants** — `Io`, `Toml`, `Clipboard`, `Command`, `Runtime`, `SyncFailure`. Well-structured, domain-organized.

**`SyncFailureKind`** (`src/error.rs:28-63`): **17 variants** — `NotConfigured`, `ConnectFailed`, `HealthCheckFailed`, `AuthenticationFailed`, `SyncRequestFailed`, `CreateLibraryFailed`, `GetPremadeLibraryFailed`, `RegistrationFailed`, `LibraryManagerInitFailed`, `LibraryModeInitFailed`, `LibrariesDirReadFailed`, `NoLibrariesToSync`, `SaveMergedLibraryFailed`, `PartialSyncFailure`, `PremadePartialFailure`, `EncryptionFailed`, `DecryptionFailed`. Typed classification, no string matching needed.

### String-Matching Error Patterns (Production Code)

**Critical instances** (non-test code only):

1. **`src/sync_commands.rs:639,708`** — `err_msg.contains("Library not found")`:
   ```rust
   let err_msg = e.to_string();
   if err_msg.contains("Library not found") {
       handle_library_not_found(...);
   }
   ```
   This pattern-matches on the **display string** of `SnipError::Runtime`. Fragile — if the error message changes, the fallback path breaks silently. **Should use a typed error variant or `SyncFailureKind`.**

2. **`src/commands/doctor_cmd.rs:87-101`** — Chains `.contains("authentication")`, `.contains("configuration")`, etc. on a lowercased diagnostic message:
   ```rust
   let lower = d.message.to_lowercase();
   let code = if lower.contains("authentication") { ... }
   ```
   This is a **doctor diagnostic classification** that parses human-readable messages. Acceptable for display-only diagnostic output, but brittle.

3. **`src/commands/sync_cmd.rs:881,906`** — `action.action.contains("recreate")` and `action.action.contains("fix permissions")` — These operate on **repair action strings** that are constructed locally (`src/commands/sync_cmd.rs` repair module). Acceptable but could use an enum.

### Layer-Owned Error Hierarchy

The hierarchy is clean:
- **I/O layer**: `SnipError::Io` with `From<io::Error>` auto-conversion
- **TOML layer**: `SnipError::Toml` with `SnipError::toml_error()` constructor
- **Sync layer**: `SnipError::SyncFailure` with typed `SyncFailureKind` (no string matching)
- **Clipboard**: `SnipError::Clipboard`
- **Command**: `SnipError::Command`
- **Catch-all**: `SnipError::Runtime` — **this is the problem child**, used for everything from validation to keychain errors to encryption failures. The `CryptoError` conversion maps into `Runtime` (`src/error.rs:296-301`).

### Error Mapping Strategy

Constructors: `SnipError::io_error()`, `SnipError::toml_error()`, `SnipError::clipboard_error()`, `SnipError::command_error()`, `SnipError::runtime_error()`, `SnipError::sync_failure()`.

The `Runtime` variant is overused as a catch-all. Keychain errors (`src/config.rs:403-415`) use `runtime_error("keychain entry", Some(&e.to_string()))` instead of a typed variant. The `SyncFailureKind` enum is excellent and should be the model for other domains.

---

## Workstream G: Blocking/Async/Runtime Ownership

### Tokio Runtime Initialization

```rust
// src/main.rs:23-28
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Runtime::new().unwrap_or_else(|e| {
        eprintln!("Failed to create async runtime: {e}...");
        std::process::exit(1);
    })
});
```

`LazyLock` (Rust 1.80+). **Thread-safe, no `unsafe`, initialized on first access.** The runtime is a global singleton passed explicitly via `&RUNTIME` to command handlers.

### Which Commands Trigger Async

Commands receiving `&RUNTIME`:
- `run_cmd::run()` — default (no subcommand) and `Commands::Run`
- `clip_cmd::run()` — `Commands::Clip`
- `search_cmd::run()` — `Commands::Search`
- `select_cmd::run()` — `Commands::Select`
- `sync_cmd::run()` — `Commands::Sync`
- `sync_cmd::run_retry()` — `SyncCommands::Retry`
- `register_cmd::run()` — `Commands::Register`
- `premade_cmd::run_list/get/sync/search/update()` — `Commands::Premade`

**Synchronous-only commands** (no runtime): `new`, `list`, `edit`, `keybindings`, `doctor`, `import`, `completions`, `shell`, `status`, `cron`.

### `runtime.block_on()` Usage

The pattern is `runtime.block_on(async_fn)` — called from synchronous command handlers. This is **correct** for a CLI binary. The runtime is created once and reused. No `spawn_blocking` calls exist anywhere in the codebase.

### Executor's Own Runtime

The executor (`src/auto_sync/executor.rs`) creates its **own** `tokio::runtime::Runtime` via `tokio::runtime::Runtime::new()` (inside the subprocess). This is correct — the executor runs as a separate process and needs its own runtime.

### Global Runtime Concern

The `RUNTIME` static is `pub(crate)` (in `src/main.rs`, not `src/lib.rs`). It's only accessible within the binary crate. External consumers of the library crate don't see it. **No leak into public API.**

---

## Workstream H: Test-Support Boundary

### `test_events` Module

`src/auto_sync/test_events.rs` provides three public functions:
- `pub fn enabled() -> bool` — checks `SNP_TEST_EVENTS_DIR` env var
- `pub fn sink_path() -> Option<PathBuf>` — returns path if env var set
- `pub fn emit(...)` — writes JSON-lines event if env var set

**Not gated by `#[cfg(test)]`** — these are called from production code:
- `src/auto_sync/worker.rs` (20+ call sites)
- `src/auto_sync/executor.rs` (6 call sites)

**This is intentional.** The functions are **zero-cost no-ops** when `SNP_TEST_EVENTS_DIR` is not set. The env var is only set in integration test harnesses (`tests/support/event_sink.rs`). In release builds, the branch is always false and the compiler optimizes it away.

The module is declared as `pub mod test_events` in `src/auto_sync/mod.rs:13`. While `auto_sync` is `pub`, the `test_events` submodule's functions are effectively dead code in release — `enabled()` always returns `false`, and `emit()` early-returns on the `sink_path()` None check.

### `#[cfg(test)]` Across `src/`

Found in **42 files** (test modules only). No `#[cfg(test)]` items leak into public API — all are contained within `mod tests` blocks.

### Test-Only Items in Public API

**None found.** All `#[cfg(test)]` blocks are standard `mod tests` modules. The `EnvGuard` struct in `test_events.rs` tests is private to the test module.

### Test Event Env Var Gating

`SNP_TEST_EVENTS_DIR` is checked at runtime via `std::env::var()` — not compile-time gated. This means:
- The env var check runs on every worker/executor lifecycle event
- In release builds: one `std::env::var()` call per event (cheap)
- In test builds: events are written to the specified directory

**No security concern** — the env var is not user-facing and only set in test contexts. The path is derived from the env var value, not user input.

---

## Summary

| Workstream | Status | Key Finding |
|---|---|---|
| **E: Data Model** | 🟡 Moderate | Parallel vectors are intentional. Missing `#[non_exhaustive]` on 9+ public enums likely to grow. `SnippetId` newtype is low priority. |
| **F: Error Boundaries** | 🟡 Moderate | `SnipError` is well-structured (6 variants). `SyncFailureKind` (17 variants) is excellent. **String-matching in `sync_commands.rs:639,708` is the highest-risk item** — should use typed error dispatch. `Runtime` variant is overused as catch-all. |
| **G: Async/Runtime** | 🟢 Good | `LazyLock` is correct. Runtime is binary-only, not leaked to library API. No `spawn_blocking`. Executor creates its own runtime (correct for subprocess). |
| **H: Test-Support** | 🟢 Good | `test_events` is zero-cost no-ops in release (env-var gated). No `#[cfg(test)]` items leak into public API. 42 test modules properly isolated. |

### Recommended Priority Actions

1. **HIGH**: Replace `err_msg.contains("Library not found")` in `src/sync_commands.rs:639,708` with typed `SyncFailureKind` dispatch
2. **MEDIUM**: Add `#[non_exhaustive]` to `ProcessResult`, `CommandOutcome`, `SelectionOutcome`, `SnippetSort`, `FailureClass`, `ExecutorExitCode`, `WorkerOutcome`, `SpawnError`, `AutoSyncFailureMode`
3. **LOW**: Consider typed error variants for keychain operations (currently `Runtime` catch-all)
4. **LOW**: Consider `SnippetId` newtype for compile-time ID safety

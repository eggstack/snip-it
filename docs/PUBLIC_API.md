# snip-it Public API Inventory

Generated: Phase 06A Workstream A

## Overview

The `snip-it` crate is a binary crate whose library half (`src/lib.rs`) is
re-exported for integration tests under `tests/`.  The comment in `lib.rs`
states that public modules form the "stable API surface for crates.io
consumers," but in practice the crate is not published to crates.io as a
library — it is a standalone binary.  The public surface exists because:

1. The `snp` binary lives in the same package but is a separate crate, so it
   can only see `pub` items from `lib.rs`.
2. Integration tests under `tests/` need access to `sync::SyncClient`, proto
   types, and other internals.
3. A handful of types are exposed for `#[derive(clap::ValueEnum)]` on CLI
   arguments (e.g., `sort::SnippetSort`).

Four modules are correctly marked `pub(crate)`: `clipboard`, `library`,
`sync_commands`, and `utils`.  Everything else is `pub`.

---

## Root-level exports (`src/lib.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub mod auto_sync` | **application-internal** | Auto-sync subsystem; used by binary + integration tests only |
| `pub mod commands` | **application-internal** | CLI command implementations; no external consumer |
| `pub mod config` | **provisional-public** | `SyncSettings`, `SyncDirection`, constants; could be useful for library consumers |
| `pub mod encryption` | **provisional-public** | AES-256-GCM encryption; reusable standalone |
| `pub mod error` | **stable-public** | `SnipError`, `SnipResult`, `SyncFailureKind` — core error types |
| `pub mod logging` | **application-internal** | Logging infrastructure; not intended for external use |
| `pub mod proto` | **integration-test-only** | Prost-generated gRPC types; needed by sync integration tests |
| `pub mod sync` | **integration-test-only** | `SyncClient`; used by sync integration tests |
| `pub mod ui` | **application-internal** | TUI interface; not for external consumers |
| `pub mod diagnostics` | **provisional-public** | Import/doctor report types; could be useful for tooling |
| `pub mod output` | **provisional-public** | Output field rendering; self-contained utility |
| `pub mod sort` | **stable-public** | `SnippetSort`, `SortOptions`, `rank_snippets`; used by CLI args and tests |
| `pub mod status_snapshot` | **application-internal** | Status projection for `snp status` and doctor |
| `pub mod usage` | **application-internal** | Local-only usage metadata; not for external use |
| `pub use error::{SnipError, SnipResult}` | **stable-public** | Re-exported for convenience |
| `pub struct SnippetData` | **application-internal** | Parallel vectors for TUI display; internal glue |
| `pub enum ProcessResult` | **application-internal** | TUI selection result; internal glue |
| `pub enum CommandOutcome` | **application-internal** | CLI-level exit code mapping; internal glue |
| `pub enum SelectionOutcome` | **application-internal** | Raw TUI selection result; internal glue |

---

## `error` module (`src/error.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub enum SyncFailureKind` | **stable-public** | Typed sync failure classification (17 variants) |
| `pub enum SnipError` | **stable-public** | `#[non_exhaustive]` error enum with 6 variants |
| `pub type SnipResult<T>` | **stable-public** | `Result<T, SnipError>` alias |
| `SnipError::io_error()` | **stable-public** | Constructor |
| `SnipError::toml_error()` | **stable-public** | Constructor |
| `SnipError::clipboard_error()` | **stable-public** | Constructor |
| `SnipError::command_error()` | **stable-public** | Constructor |
| `SnipError::runtime_error()` | **stable-public** | Constructor |
| `SnipError::sync_failure()` | **stable-public** | Constructor |
| `impl From<io::Error>` | **stable-public** | Auto-conversion |
| `impl From<CryptoError>` | **stable-public** | Auto-conversion |

---

## `sort` module (`src/sort.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub enum SnippetSort` | **stable-public** | 6 sort modes; `#[derive(clap::ValueEnum)]` for CLI |
| `pub struct SortOptions` | **stable-public** | `{ mode, favorites_first }` |
| `pub fn rank_snippets()` | **stable-public** | Deterministic sort with tie-break chain |

---

## `config` module (`src/config.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub use crate::utils::config::get_sync_config_path` | **provisional-public** | Re-export from utils |
| `pub const DEFAULT_SERVER_URL` | **provisional-public** | Used by CLI defaults |
| `pub const AUTO_SYNC_DEBOUNCE_MIN/MAX` | **provisional-public** | Bounds constants |
| `pub const AUTO_SYNC_MAX_DELAY_MIN/MAX` | **provisional-public** | Bounds constants |
| `pub const DEFAULT_SYNC_TIMEOUT_SECS` | **provisional-public** | Default timeout |
| `pub const MIN/MAX_SYNC_TIMEOUT_SECS` | **provisional-public** | Bounds constants |
| `pub enum AutoSyncFailureMode` | **provisional-public** | `Ignore`, `Warn`, `Error` |
| `pub enum SyncDirection` | **provisional-public** | `Push`, `Pull`, `Bidirectional` |
| `pub struct SyncSettings` | **provisional-public** | 14 fields; `#[non_exhaustive]` would be appropriate |
| `pub fn invalidate_toml_cache()` | **application-internal** | Cache management |
| `pub fn cached_read_toml()` | **application-internal** | Cached TOML reader |
| `pub fn save_sync_settings()` | **application-internal** | Persist sync settings |
| `pub fn load_sync_settings()` | **application-internal** | Load sync settings |
| `pub fn get_sync_settings()` | **application-internal** | Load with fallback |
| `SyncSettings::sync_limit_value()` | **provisional-public** | Accessor |
| `SyncSettings::auto_sync_debounce()` | **provisional-public** | Duration accessor |
| `SyncSettings::auto_sync_max_delay()` | **provisional-public** | Duration accessor |
| `SyncSettings::auto_sync_timeout()` | **provisional-public** | Duration accessor |
| `SyncSettings::sync_config_file_exists()` | **provisional-public** | File check |

---

## `encryption` module (`src/encryption.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub fn clear_key_cache()` | **provisional-public** | Session cache management |
| `pub enum CryptoError` | **provisional-public** | `#[non_exhaustive]` 4-variant error |
| `pub type CryptoResult<T>` | **provisional-public** | Result alias |
| `pub struct EncryptedPayload` | **provisional-public** | `{ salt, nonce, ciphertext }` |
| `pub fn encrypt()` | **provisional-public** | AES-256-GCM encryption |
| `pub fn decrypt()` | **provisional-public** | AES-256-GCM decryption |
| `pub fn ct_eq()` (#[cfg(test)]) | **dead-or-accidental** | Test-only constant-time comparison exposed as `pub` |

---

## `sync` module (`src/sync.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub struct SyncRetryConfig` | **integration-test-only** | Retry config for gRPC |
| `pub fn is_retryable_grpc_error()` | **integration-test-only** | Retry predicate |
| `pub struct SyncClient` | **integration-test-only** | gRPC client wrapper |
| `SyncClient::create()` | **integration-test-only** | Constructor |
| `SyncClient::sync_encrypted()` | **integration-test-only** | Encrypted sync |
| `SyncClient::health_check()` | **integration-test-only** | Health check |
| `SyncClient::register()` | **integration-test-only** | Device registration |
| `SyncClient::list_libraries()` | **integration-test-only** | List server libraries |
| `SyncClient::create_library()` | **integration-test-only** | Create server library |
| `SyncClient::list_premade_libraries()` | **integration-test-only** | List premade libraries |
| `SyncClient::get_premade_library()` | **integration-test-only** | Download premade library |
| `SyncClient::search_premade_libraries()` | **integration-test-only** | Search premade libraries |
| `pub fn encrypt_snippet()` | **integration-test-only** | Encrypt snippet for sync |
| `pub fn decrypt_snippet()` | **integration-test-only** | Decrypt snippet from sync |
| `pub fn detect_device_conflict()` | **integration-test-only** | Multi-device conflict detection |
| `pub(crate) fn add_api_key_metadata()` | **application-internal** | `pub(crate)` — correct |

---

## `sync_commands` module (`src/sync_commands.rs`)

*Note: This module is `pub(crate)` in `lib.rs`, but the items below are
`pub` within the module.  They are accessible to the binary crate but not
to external consumers.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub fn run_premade_sync()` | **application-internal** | Premade library sync |
| `pub fn run_sync()` | **application-internal** | Full sync operation |
| `pub fn run_default_sync()` | **application-internal** | Default bidirectional sync |

---

## `commands` module (`src/commands/mod.rs`)

*The entire `commands` module is `pub` in `lib.rs` but is `application-internal`.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub mod clip_cmd` through `pub mod sync_cmd` (18 submodules) | **application-internal** | All CLI command modules |
| `pub enum ExpandedCommand` | **application-internal** | Command expansion result |
| `pub fn get_config_path()` | **application-internal** | Config path resolution |
| `pub fn get_library_path()` | **application-internal** | Library path resolution |
| `pub fn init_library_manager()` | **application-internal** | Library manager init |
| `pub fn load_snippets()` | **application-internal** | Load snippets from file |
| `pub fn save_snippets()` | **application-internal** | Save snippets to file |
| `pub fn get_snippet_data()` | **application-internal** | Extract TUI display data |
| `pub fn expand_snippet_command()` | **application-internal** | Variable expansion |
| `pub fn run_snippet_selection()` | **application-internal** | TUI selection loop |

---

## `diagnostics` module (`src/diagnostics.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub enum DiagnosticSeverity` | **provisional-public** | `Info`, `Warning`, `Error` |
| `pub struct SourceSpan` | **provisional-public** | Byte-offset span |
| `pub struct CompatibilityDiagnostic` | **provisional-public** | Import/doctor diagnostic |
| `pub struct ImportDuplicate` | **provisional-public** | Duplicate entry record |
| `pub struct NormalizationRecord` | **provisional-public** | Field normalization record |
| `pub struct PetImportReport` | **provisional-public** | Import operation report |
| `pub struct DoctorReport` | **provisional-public** | Doctor check report |
| `pub fn version()` | **provisional-public** | Crate version string |
| `pub fn diagnostic_counts()` | **provisional-public** | Severity counting helper |

---

## `output` module (`src/output.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub const OUTPUT_SEARCH_BUDGET` | **provisional-public** | Fuzzy match budget constant |
| `pub struct OutputPresentation` | **provisional-public** | Output rendering wrapper |
| `pub fn sanitize_for_terminal()` | **provisional-public** | ANSI stripping utility |

---

## `usage` module (`src/usage.rs`)

| Item | Classification | Notes |
|------|---------------|-------|
| `pub struct UsageData` | **application-internal** | Usage metadata |
| `pub struct UsageIndex` | **application-internal** | Persistent usage index |
| `pub struct UsageEntry` | **application-internal** | Single usage entry |
| `UsageIndex::load()` | **application-internal** | Load from disk |
| `UsageIndex::save()` | **application-internal** | Save to disk |
| `UsageIndex::record_use()` | **application-internal** | Record a use |
| `UsageIndex::get_usage()` | **application-internal** | Query usage |
| `UsageIndex::prune()` | **application-internal** | Remove stale entries |
| `UsageIndex::entries()` | **application-internal** | All entries accessor |

---

## `status_snapshot` module (`src/status_snapshot.rs`)

*Entire module is `application-internal`.*

| Item | Classification | Notes |
|------|---------------|-------|
| All 15 structs/enums | **application-internal** | Status snapshot types |
| `pub fn capture_snapshot()` | **application-internal** | Snapshot builder |
| `pub fn sync_configuration_state()` | **application-internal** | Config state query |
| `pub fn pending_state_view()` | **application-internal** | Pending state query |
| `pub fn attempt_state_view()` | **application-internal** | Attempt state query |
| `pub fn execution_state_view()` | **application-internal** | Execution state query |
| `pub fn derive_top_level()` | **application-internal** | Top-level state derivation |
| `pub fn collect_diagnostics()` | **application-internal** | Diagnostic collection |

---

## `auto_sync` module (`src/auto_sync/mod.rs`)

*Entire module is `application-internal` — used by the binary for auto-sync
worker/executor subprocess management.*

### Re-exports from `mod.rs`

| Item | Classification | Notes |
|------|---------------|-------|
| `pub use notification::*` (6 items) | **application-internal** | Notification API |
| `pub use pending::{ConditionalClearResult, PendingSnapshot, PendingState}` | **application-internal** | Pending state types |
| `pub use policy::{AutoSyncPolicy, FailureClass, MutationKind, MutationOrigin, RetryDisposition}` | **application-internal** | Policy types |
| `pub use worker::WorkerOutcome` | **application-internal** | Worker result type |
| `pub mod paths` | **application-internal** | Path helpers for diagnostics |

### Submodule highlights (all `application-internal`)

| Submodule | Key public items |
|-----------|-----------------|
| `execution_lock` | `SyncExecutionLock`, `ExecutionLockContents`, `try_acquire`, `wait_acquire`, `process_alive` |
| `executor` | `ExecutorExitCode`, `ExecutorCommand`, `run_executor`, `classify_sync_error` |
| `lock` | `WorkerLock`, `WorkerLockContents`, `try_acquire`, `inspect`, `is_stale`, `process_alive` |
| `notification` | `MutationContext`, `AutoSyncNotificationResult`, `SubcommandTag`, `notify_mutation`, `should_attempt_auto_sync_recovery`, `startup_recover_pending` |
| `pending` | `PendingSnapshot`, `PendingState`, `ConditionalClearResult`, `record_pending_mutation`, `read_state`, `read_state_from_dir` |
| `pending_lock` | `PendingTxnGuard`, `PendingTxnLockError`, `acquire_pending_txn` |
| `policy` | `AutoSyncPolicy`, `FailureClass` (11 variants), `MutationKind`, `MutationOrigin`, `RetryDisposition`, `transient_backoff` |
| `schedule` | `ScheduleDecision`, `Caller`, `schedule_sync`, `schedule_sync_from_config`, `schedule_and_spawn` |
| `spawn` | `SpawnError`, `spawn_worker`, `spawn_executor`, `WORKER_SUBCOMMAND`, `EXECUTOR_SUBCOMMAND` |
| `status` | `AutoSyncStatus`, `StatusRead`, `read_status`, `write_status`, `record_success`, `record_failure`, `compute_config_fingerprint` |
| `test_events` | `enabled`, `sink_path`, `emit` |
| `worker` | `WorkerOutcome`, `SpawnResult`, `Clock`, `SystemClock`, `DebounceResult`, `run`, `debounce`, `preflight_check`, `startup_recover` |

---

## `proto` module (`src/proto.rs`)

*Prost-generated. All items are `integration-test-only`.*

| Item | Classification | Notes |
|------|---------------|-------|
| `GetSnippetsRequest`, `PushSnippetsRequest`, `PushSnippetsResponse` | **integration-test-only** | gRPC request/response types |
| `SyncRequest`, `SyncResponse` | **integration-test-only** | Core sync protocol types |
| `Snippet` (proto) | **integration-test-only** | Wire-format snippet |
| `SnippetList` | **integration-test-only** | Paginated snippet list |
| `HealthRequest`, `HealthResponse` | **integration-test-only** | Health check types |
| `RegisterRequest`, `RegisterResponse` | **integration-test-only** | Registration types |
| `CreateLibraryRequest/Response`, `ListLibrariesRequest/Response`, `DeleteLibraryRequest/Response` | **integration-test-only** | Library management types |
| `Library` | **integration-test-only** | Server library metadata |
| `ListPremadeLibrariesRequest/Response`, `GetPremadeLibraryRequest/Response`, `SearchPremadeLibrariesRequest/Response` | **integration-test-only** | Premade library types |
| `PremadeLibrary` | **integration-test-only** | Premade library metadata |
| `pub mod snippet_sync_client` | **integration-test-only** | Generated gRPC client |
| `pub mod snippet_sync_server` | **integration-test-only** | Generated gRPC server trait + impl |

---

## `logging` module (`src/logging.rs`)

*Entire module is `application-internal`.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub struct LogConfig` | **application-internal** | Logging configuration |
| `pub fn get_default_log_dir()` | **application-internal** | Log directory path |
| `pub fn init_default_logging()` | **application-internal** | Initialize tracing |
| `pub fn log_startup_info()` | **application-internal** | Log startup |
| `pub fn log_shutdown_info()` | **application-internal** | Log shutdown |
| `pub fn setup_panic_handler()` | **application-internal** | Panic hook |
| `pub fn log_config_operation()` | **application-internal** | Config operation logging |
| `pub fn log_clipboard_operation()` | **application-internal** | Clipboard operation logging |
| `pub fn audit_log()` | **application-internal** | Audit trail |

---

## `ui` module (`src/ui/mod.rs`)

*Entire module is `application-internal`.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub use theme::get_theme` | **application-internal** | Theme loader |
| `pub use variables::{VariablePromptResult, prompt_variables}` | **application-internal** | Variable prompt UI |

---

## `utils` module (`src/utils/mod.rs`)

*Module is `pub(crate)` in `lib.rs` — correctly hidden from external consumers.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub mod atomic` | **application-internal** | Atomic file writes |
| `pub mod config` | **application-internal** | Config path helpers |
| `pub mod shell_keywords` | **application-internal** | Shell keyword expansion |
| `pub mod tempfile_guard` | **application-internal** | Temp file cleanup |
| `pub mod toml_helpers` | **application-internal** | TOML escape handling |
| `pub mod variables` | **application-internal** | Variable parsing/expansion |
| Re-exports: `expand_command`, `parse_variables`, etc. | **application-internal** | Convenience re-exports |

---

## `library` module (`src/library.rs`)

*Module is `pub(crate)` in `lib.rs` — correctly hidden from external consumers.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub struct Snippets` | **application-internal** | Snippet collection |
| `pub struct Snippet` | **application-internal** | Individual snippet |
| `pub struct LibraryConfig` | **application-internal** | Libraries config |
| `pub struct LibraryMeta` | **application-internal** | Library metadata |
| `pub struct LibraryManager` | **application-internal** | Library management |
| `pub fn load_library()` | **application-internal** | Load from file |
| `pub fn save_library()` | **application-internal** | Save to file |
| `pub fn backup_library()` | **application-internal** | Create backup |

---

## `clipboard` module (`src/clipboard.rs`)

*Module is `pub(crate)` in `lib.rs` — correctly hidden from external consumers.*

| Item | Classification | Notes |
|------|---------------|-------|
| `pub fn copy_to_clipboard()` | **application-internal** | Clipboard copy |
| `pub fn copy_to_clipboard_auto()` | **application-internal** | Copy with auto-clear |
| `pub fn copy_to_clipboard_with_auto_clear()` | **application-internal** | Copy with explicit clear |
| `pub fn clear_clipboard()` | **application-internal** | Clear clipboard |
| `pub fn schedule_clipboard_clear()` | **application-internal** | Schedule auto-clear |
| `pub fn invalidate_clipboard_settings_cache()` | **application-internal** | Cache invalidation |

---

## Recommended Visibility Changes

### High priority (should narrow)

1. **`pub mod commands`** → `pub(crate)` — No external consumer needs CLI
   command implementations.  The 18 submodules and their helpers are purely
   binary-internal.

2. **`pub mod auto_sync`** → `pub(crate)` — The entire auto-sync subsystem
   (worker, executor, locks, pending, status, policy, schedule, spawn) is
   binary-internal plumbing.  The re-exports in `mod.rs` expose ~30 items
   that no external consumer needs.

3. **`pub mod logging`** → `pub(crate)` — Logging setup is binary-internal.

4. **`pub mod status_snapshot`** → `pub(crate)` — Status projection for
   `snp status` and doctor; purely binary-internal.

5. **`pub mod ui`** → `pub(crate)` — TUI is binary-internal.

6. **`pub mod usage`** → `pub(crate)` — Local-only usage tracking; no
   external consumer.

### Medium priority (should narrow or document as provisional)

7. **`pub mod sync`** — Currently `pub` solely for integration tests.  If
   the integration tests were in the same crate, this could be `pub(crate)`.
   Since they're in `tests/`, it must remain `pub` for now.  Mark as
   `integration-test-only` in docs.

8. **`pub mod proto`** — Same situation as `sync`.  Prost-generated types
   needed by integration tests.  Mark as `integration-test-only`.

9. **`config` module internal items** — `invalidate_toml_cache`,
   `cached_read_toml`, `save_sync_settings`, `load_sync_settings`,
   `get_sync_settings` are `pub` but only used by the binary and
   `pub(crate)` modules.  These should be `pub(crate)`.

### Low priority (acceptable as-is)

10. **`pub mod error`** — Core error types.  Appropriately `pub` and
    `#[non_exhaustive]`.

11. **`pub mod sort`** — Used by `#[derive(clap::ValueEnum)]` on CLI
    arguments.  Appropriately `pub`.

12. **`pub mod encryption`** — Self-contained crypto utilities.  Reasonably
    `pub` for potential library reuse.

13. **`pub mod diagnostics`** — Report types for import/doctor.  Reasonably
    `pub` for tooling.

14. **`pub mod output`** — Self-contained rendering utility.  Reasonably `pub`.

### Dead/accidental items

15. **`pub fn ct_eq()` in `encryption.rs`** — Marked `#[cfg(test)]` but
    declared `pub`.  This is dead code in release builds.  Should be
    `#[cfg(test)] pub(crate)` or removed.

16. **`pub fn quote_strings_containing_backslashes()` in `utils/toml_helpers.rs`** —
    Public helper that was used by earlier code but is no longer called in the
    save path.  The doc comment says it's "available for callers that
    hand-write TOML."  Consider whether it should remain public.

### Doc warnings to fix

17. **`error.rs:137`** — Broken intra-doc link to `FailureClass`.  Should be
    `[`FailureClass`](crate::auto_sync::policy::FailureClass)`.

18. **`output.rs:3`** — Doc links to private item `crate::library::Snippet`.
    The `library` module is `pub(crate)`, so this link only works with
    `--document-private-items`.  Rephrase or make the link unconditional.

---

## Summary Counts

| Classification | Count |
|---------------|-------|
| **stable-public** | ~25 items (error, sort, re-exports) |
| **provisional-public** | ~55 items (config, encryption, diagnostics, output) |
| **application-internal** | ~200+ items (commands, auto_sync, logging, status_snapshot, ui, usage, library, clipboard, utils) |
| **integration-test-only** | ~40 items (sync, proto) |
| **dead-or-accidental** | 2 items (ct_eq, quote_strings_containing_backslashes) |

The majority of the public surface (~250 items) is `application-internal`,
exposed only because the binary crate and library crate share the same
package.  Narrowing `commands`, `auto_sync`, `logging`, `status_snapshot`,
`ui`, and `usage` to `pub(crate)` would reduce the public surface by ~80%
and make the true external API (error, sort, encryption, config, diagnostics,
output) much clearer.

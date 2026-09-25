# Core: Errors & Public API

[← Back to Overview](overview.md)

## Overview

Two halves: the typed error system (`src/error.rs`, domain/core layer)
and the stable public API surface (`src/lib.rs`). Everything else in the
crate is either `#[doc(hidden)]` (binary/test access, not supported) or
`pub(crate)` (crate-internal).

## `SnipError` — `src/error.rs:109-159`

```rust
#[non_exhaustive]
pub enum SnipError {
    Io { operation: String, path: Option<PathBuf>, source: io::Error },
    Toml { operation: String, source: Box<dyn Error + Send + Sync> },
    Clipboard { operation: String, message: String },
    Command { command: String, args: Vec<String>, source: io::Error },
    Runtime { message: String, detail: Option<String> },
    SyncFailure { kind: SyncFailureKind, detail: Option<String> },
}
pub type SnipResult<T> = Result<T, SnipError>; // error.rs:314
```

- `Display` (`error.rs:161-225`): `Io` shows operation + source + path;
  `Command` **redacts secrets** via `utils::redact::redact_secrets` and
  truncates long args — bearer tokens / URL credentials never render.
- `Error::source` returns the inner error for `Io`/`Toml`/`Command`,
  `None` for `Clipboard`/`Runtime`/`SyncFailure`.
- `#[non_exhaustive]` allows future variants without breaking downstream
  `match` statements.

### Constructors — `src/error.rs:258-311`

```rust
SnipError::io_error(operation: &str, path: impl Into<PathBuf>, source: io::Error) -> Self
SnipError::toml_error(operation: &str, source: impl Error + Send + Sync + 'static) -> Self
SnipError::clipboard_error(operation: &str, message: impl Into<String>) -> Self
SnipError::command_error(command: &str, args: Vec<String>, source: io::Error) -> Self
SnipError::runtime_error(message: &str, detail: Option<&str>) -> Self
SnipError::sync_failure(kind: SyncFailureKind, detail: Option<&str>) -> Self
```

### Conversions

- `From<io::Error>` (`error.rs:238-255`) maps `ErrorKind` to an operation
  string (`NotFound` → "file not found", `PermissionDenied` →
  "permission denied", …), path `None`.
- `From<CryptoError>` (`error.rs:316-331`): `EncryptionFailed` →
  `SyncFailureKind::EncryptionFailed`; `DecryptionFailed` /
  `KeyDerivationFailed` / `InvalidData` → `DecryptionFailed`. Both land
  in `FailureClass::Internal`, preserving retry classification.

## `SyncFailureKind` (21 variants) — `src/error.rs:30-73`

```rust
pub enum SyncFailureKind {
    NotConfigured, ConnectFailed, HealthCheckFailed, AuthenticationFailed,
    SyncRequestFailed, CreateLibraryFailed, GetPremadeLibraryFailed,
    RegistrationFailed, LibraryManagerInitFailed, LibraryModeInitFailed,
    LibrariesDirReadFailed, NoLibrariesToSync, SaveMergedLibraryFailed,
    PartialSyncFailure, PremadePartialFailure, EncryptionFailed,
    DecryptionFailed, LibraryNotFound, Timeout, RequestTooLarge, ClockSkew,
}
```

Each variant maps directly to an auto-sync `FailureClass`
(`Transient`/`Configuration`/`LocalFailure`/`Internal`) **without string
matching**; `detail` carries the upstream message for display only.
Multi-batch `PushSnippets` errors preserve the original kind via
`add_batch_context()` (see `sync.md`). Scheduling errors stay typed —
never collapse pending-read/spawn failures into
`NoPending`/`SpawnNow`/success.

## Stable Public API vs Hidden vs Internal — `src/lib.rs`

Supported table (`lib.rs:13-20`):

| Module | Public items |
|--------|-------------|
| crate root | `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library` |
| crate root | `AtomicWriteOptions`, `AtomicWriteReport`, `Durability`, `atomic_replace`, `write_private_atomic` |
| `error` | `SnipError`, `SnipResult`, `SyncFailureKind` |
| `sort` | `SnippetSort`, `SortOptions`, `rank_snippets` |
| `config` | `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode`, related constants |
| `outcome` | `CliOutcome`, `exit_code::*`, `OutputContext` |

```rust
pub mod config; pub mod error; pub mod outcome; pub mod sort; // stable
#[doc(hidden)] pub mod auto_sync, commands, logging, mcp, process_file_lock,
    selector, sync, ui, usage;                                // lib.rs:33-51
pub(crate) mod clipboard, diagnostics, encryption, library, local_data,
    migration, output, status_snapshot, sync_commands, sync_failure,
    test_failpoints, transaction (test-support: #[doc(hidden)] pub), utils;
```

- **Stable**: documented for external use; changes need semver review.
- **`#[doc(hidden)]`**: public only because the `snp` binary and
  integration tests are separate crates in the same package. Internal
  worker codes stay hidden; `SnippetData`, `ProcessResult`,
  `SelectionOutcome` are `#[non_exhaustive]` + hidden TUI-loop types.
- **`pub(crate)`**: compile-enforced internal (library, encryption,
  output, transaction, …). Layer tests in `tests/architecture.rs`
  additionally forbid core ← sync-client ← application reversals.

## `Snippet` / `Snippets` / `LibraryManager` pointer

The data-model half of "core" lives in `src/library/` — full detail in
`library.md`. `core.md` owns only the error + API-surface contract;
`src/commands/mod.rs::load_snippets()` / `save_snippets()` are thin
wrappers over `library::load_library` / `save_library`.

## Invariants / gotchas (from AGENTS.md)

- **No credentials in errors**: `Command` display redacts secrets;
  `api_key` is `[REDACTED]` in `Debug` and zeroized on `Drop`; sync
  `detail` strings must never carry key material (see `utils/redact.md`).
- `SnipError::sync_failure` is the only typed path into auto-sync
  policy — do not invent string-matched failure kinds.
- `#[non_exhaustive]` on `SnipError`, `CliOutcome`, `SyncDirection`,
  `AutoSyncFailureMode`, `SnippetSort`, `CryptoError` — downstream code
  must wildcard-match.
- `sync.rs` RPCs take `&mut self`; the retry macro expands inline so
  `self.client.<rpc>()` reborrows work. Never reintroduce a
  closure-based generic retry helper.

## File / line refs

- `src/error.rs:30-101` (`SyncFailureKind` + `Display`), `:109-159`
  (`SnipError`), `:161-255` (`Display`/`Error`/`From<io>`), `:258-314`
  (constructors + `SnipResult`), `:316-331` (`From<CryptoError>`),
  `:333-496` (unit tests incl. secret-redaction proof).
- `src/lib.rs:1-25` (supported-API docs), `:27-80` (module visibility +
  re-exports), `:86-140` (hidden TUI-loop types).

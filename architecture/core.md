# Core Types, Errors & Outcomes

[← Back to Overview](overview.md) · [← Back to CLI](cli.md)

## Overview

This doc inventories the crate's core domain types, the single error type
(`SnipError`) and its constructor helpers, and the chain that carries a
failure from a domain error to a stable process exit code.

It does **not** cover the library registry and file layout (see
[library.md](library.md)), the persistence/TOML fidelity rules (see
[persistence.md](persistence.md)), the full crate visibility split (see
[public_api.md](public_api.md)), or the per-command exit-code mapping detail
(see [outcome.md](outcome.md)).

## Core domain types

### `Snippet` — `src/library/model.rs:29-60`

The unit of stored content. Every field has a serde default, so a partially
specified snippet still deserializes.

| Field | Type | Serde name | Notes |
|-------|------|-----------|-------|
| `id` | `String` | `id` (aliases `Id`, `ID`) | Deterministic `legacy-<sha256 hex>` — **not** UUID v4 (`docs/IDENTITY_CONTRACT.md`) |
| `description` | `String` | `description` (aliases `Description`, `name`) | |
| `output` | `String` | `output` (alias `Output`) | **Local-only** — excluded from sync fingerprints |
| `tags` | `Vec<String>` | `tag` (aliases `Tag`, `Tags`, `tags`) | Serialized as `tag`, not `tags` |
| `command` | `String` | `command` (aliases `Command`, `cmd`) | May contain `<name>` / `<name=default>` variables |
| `folders` | `Vec<String>` | `folders` | **Local-only** |
| `favorite` | `bool` | `favorite` | **Local-only** |
| `created_at` | `i64` | `created_at` | Unix epoch seconds |
| `updated_at` | `i64` | `updated_at` | Unix epoch seconds; conflict-order key |
| `device_id` | `String` | `device_id` | Conflict-order key (2nd component) |
| `deleted` | `bool` | `deleted` | Tombstone; deletion beats live content even with an older timestamp |

### `Snippets` — `src/library/model.rs:14-22`

| Field | Type | Serde name | Notes |
|-------|------|-----------|-------|
| `snippets` | `Vec<Snippet>` | `snippets` (alias `Snippets`) | Pet's canonical table name; the alias keeps existing libraries loadable |
| `folders` | `Vec<String>` | `folders` | Skipped when empty |

### `LibraryConfig` — `src/library/model.rs:65-73`

Stored in `libraries.toml`.

| Field | Type | Notes |
|-------|------|-------|
| `libraries` | `Vec<LibraryMeta>` | Registry of every known library file |
| `generation` | `u64` | Monotonic counter, incremented on every mutation; backup uses it to verify coherent snapshots |

### `LibraryMeta` — `src/library/model.rs:79-90`

| Field | Type | Notes |
|-------|------|-------|
| `filename` | `String` | Library file name (without `.toml`) |
| `library_id` | `String` | Server-side library ID when linked |
| `is_primary` | `bool` | Exactly one library is primary |
| `last_sync` | `Option<i64>` | Last successful sync timestamp |
| `server_id` | `Option<String>` | Server linkage |

### Read-only resolution types — `src/library/model.rs:105-148`

| Type | Fields | Purpose |
|------|--------|---------|
| `ResolvedLibrarySource` | `name: String`, `library_id: String`, `path: PathBuf` | One visible library, resolved without migration, directory creation, or metadata rewrites |
| `PrimaryState` | `Present { name }`, `FileMissing { name, path }`, `NoPrimary { count }`, `NoLibraries` (default) | Primary-library state observed by `inspect_library_index` |
| `LibraryIndexInspection` | `missing_files: Vec<(String, PathBuf)>`, `orphan_files: Vec<PathBuf)>`, `primary: PrimaryState` | Shared read-only consistency view; `doctor`, `validate`, `repair`, and `status` each render it into their own diagnostic type |

## Selection types

### `SnippetData` — `src/lib.rs:104-111`

`#[doc(hidden)]`. Parallel vectors of snippet metadata; index `i` refers to
the same snippet in every vector. Built by
`commands::get_snippet_data()` (`src/commands/mod.rs:243`) for the TUI.

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

### `ProcessResult` — `src/lib.rs:117-131`

`#[doc(hidden)]`, `#[non_exhaustive]`. Per-snippet loop control returned by
the TUI `process_fn` callback.

| Variant | Payload | Meaning |
|---------|---------|---------|
| `Cancel` | — | User cancelled the selection |
| `Continue` | — | No snippet selected; continue to the next prompt |
| `Done(String)` | expanded command | Selection succeeded |
| `Failed { exit_code: Option<i32>, message: String }` | child status | Child process exited nonzero |

`ProcessResult::is_done()` (`src/lib.rs:135`) is `true` only for
`Done(_)`. Usage metadata is recorded only when it is true
(`src/commands/run_cmd.rs:175-184`), so a failed run records nothing.

### `SelectionOutcome` — `src/lib.rs:147-157`

`#[doc(hidden)]`, `#[non_exhaustive]`. The raw result of the shared
TUI selection loop (`run_snippet_selection`, `src/commands/mod.rs:338`).

| Variant | Payload | Meaning |
|---------|---------|---------|
| `Selected` | — | A snippet was selected and processed |
| `Cancelled` | — | User cancelled the primary selector (q / Esc / Ctrl-C) |
| `ExecutionFailed { exit_code: Option<i32> }` | child status | Selection happened but child execution failed |

`SelectionOutcome` is deliberately preserved as UI-only state. Commands map
it directly to `CliOutcome` at the CLI boundary — there is no intermediate
command-outcome layer. See [outcome.md](outcome.md).

## `SnipError` — `src/error.rs:109-159`

The crate's single error type. `#[non_exhaustive]`, `Debug`.

| Variant | Payload | Constructor helper | Typical trigger |
|---------|---------|--------------------|-----------------|
| `Io` | `operation: String`, `path: Option<PathBuf>`, `source: io::Error` | `io_error(operation, path, source)` (`:260-266`) | File read/write, directory creation, config-dir access |
| `Toml` | `operation: String`, `source: Box<dyn Error + Send + Sync>` | `toml_error(operation, source)` (`:269-277`) | Malformed TOML, serialization failure |
| `Clipboard` | `operation: String`, `message: String` | `clipboard_error(operation, message)` (`:280-285`) | Clipboard access / content transfer failure |
| `Command` | `command: String`, `args: Vec<String>`, `source: io::Error` | `command_error(command, args, source)` (`:288-294`) | Spawning or running an external command |
| `Runtime` | `message: String`, `detail: Option<String>` | `runtime_error(message, detail)` (`:297-302`) | Everything else — missing library, bad state, invariant violation |
| `SyncFailure` | `kind: SyncFailureKind`, `detail: Option<String>` | `sync_failure(kind, detail)` (`:305-310`) | Any sync-layer failure; typed so classification never string-matches |

### Conversions

```rust
impl From<io::Error> for SnipError            // src/error.rs:238-255
impl From<crate::encryption::CryptoError> for SnipError  // src/error.rs:316-331
pub type SnipResult<T> = Result<T, SnipError>;           // src/error.rs:314
```

`From<io::Error>` infers the operation from the `io::ErrorKind`
(`src/error.rs:238-255`). `From<CryptoError>` maps both encryption and
decryption failures onto `SyncFailureKind::EncryptionFailed` /
`DecryptionFailed` — both classify as `FailureClass::Internal`, so the retry
policy is preserved (`src/error.rs:316-331`).

**No credentials are carried in `SnipError`.** Error messages are built from
operation names, paths, and upstream details only; API keys and keychain
material are never interpolated into an error.

### `SyncFailureKind` — 21 variants, `src/error.rs:30-73`

`SyncFailureKind` carries the typed classification; `detail` carries the
upstream message for display only and is never used for classification.

| Kind | Classification |
|------|----------------|
| `NotConfigured` | Configuration |
| `ConnectFailed` | Transient |
| `HealthCheckFailed` | Transient |
| `AuthenticationFailed` | Configuration |
| `SyncRequestFailed` | Transient |
| `CreateLibraryFailed` | Configuration |
| `GetPremadeLibraryFailed` | Transient |
| `RegistrationFailed` | Configuration |
| `LibraryManagerInitFailed` | LocalFailure |
| `LibraryModeInitFailed` | LocalFailure |
| `LibrariesDirReadFailed` | LocalFailure |
| `NoLibrariesToSync` | Internal |
| `SaveMergedLibraryFailed` | LocalFailure |
| `PartialSyncFailure` | Transient |
| `PremadePartialFailure` | Transient |
| `EncryptionFailed` | Internal |
| `DecryptionFailed` | Internal |
| `LibraryNotFound` | Configuration |
| `Timeout` | Transient |
| `RequestTooLarge` | Configuration |
| `ClockSkew` | Configuration |

The mapping to `FailureClass` is a total, variant-by-variant match in
`FailureClass::from_error()` (`src/sync_failure.rs:36-60`) — no string
matching for typed sync failures. `SnipError::Runtime` falls back to
substring heuristics over the combined message
(`src/sync_failure.rs:61-104`).

## Error → outcome → exit code

There is **no** `From<SnipError> for CliOutcome` conversion, and no
`SnipError` → exit-code table. The chain is explicit at two points:

```text
command handler returns Err(SnipError)
        │
        └──► main(): eprintln!("error: {e}"); exit 1      (src/main.rs:1026-1032)

command handler returns Ok(CliOutcome::X)
        │
        └──► main(): exit(X.exit_code())                  (src/main.rs:1019-1024)
```

- A handler that wants a **specific** code (3, 4, 5, 6, 7, 9) returns the
  matching `CliOutcome` variant instead of propagating the error.
- A handler that has no more specific code returns `Err(SnipError)`, which
  becomes exit **1**.
- Exit code **2** (`USAGE_ERROR`) comes only from Clap's own argument
  parsing — no command returns it.
- Exit code **10** (`UNSAFE_REPAIRS`) is **not** a `CliOutcome` variant. It
  is produced by the direct `exit_on_repair_status()` path in `main.rs`
  (`src/main.rs:375-388`), which calls `std::process::exit` inside the
  handler.

`PersistenceFailed` deliberately maps to code 1 (`GENERAL_ERROR`) rather than
a code of its own — persistence failures are operational errors, not a
distinct user-facing category (`src/outcome.rs:83`, `docs/EXIT_CODES.md`).

## Module visibility split (summary)

`src/lib.rs` declares three visibility tiers:

| Tier | Items | Meaning |
|------|-------|---------|
| **Supported public API** | `pub mod config`, `pub mod error`, `pub mod outcome`, `pub mod sort`; root re-exports `SnipError`, `SnipResult`, `LibraryConfig`, `LibraryMeta`, `Snippet`, `Snippets`, `load_library`, `save_library`, and the atomic-write helpers (`src/lib.rs:27-30`, `:72-80`) | Stable for external consumers |
| **`#[doc(hidden)]` public** | `pub mod auto_sync`, `commands`, `logging`, `mcp`, `process_file_lock`, `selector`, `sync`, `ui`, `usage`, `transaction`; `SnippetData`, `ProcessResult`, `SelectionOutcome` (`src/lib.rs:33-51`, `:67-69`, `:104-131`, `:147-157`) | `pub` so the binary and integration tests can reach them, but **not** part of the supported external API |
| **`pub(crate)` internal** | `mod clipboard`, `diagnostics`, `encryption`, `library`, `local_data`, `migration`, `output`, `status_snapshot`, `sync_commands`, `sync_failure`, `test_failpoints`, `utils`; `pub(crate) use snip_proto as proto` (`src/lib.rs:54-70`, `:43`) | Not reachable outside the crate. `library` is internal, but its types are re-exported at the root |

`transaction` is feature-dependent: `pub(crate) mod transaction` without the
`test-support` feature (`src/lib.rs:65-66`), and `#[doc(hidden)] pub mod
transaction` **with** it (`src/lib.rs:67-69`) — the feature flips an internal
module into a public one so integration tests can drive transaction
scenarios.

The full enumeration, the contract enforced by `tests/public_api_smoke.rs`,
and the stable-vs-internal policy live in **[public_api.md](public_api.md)**.

## Key Files

- `src/error.rs` — `SnipError` (`:109-159`), constructors (`:258-311`), `SyncFailureKind` (`:30-73`), `From` impls
- `src/sync_failure.rs` — `FailureClass` and `from_error()` classification
- `src/outcome.rs` — `CliOutcome` (`:18-40`), `exit_code` constants (`:46-70`), `exit_code()` (`:72-91`)
- `src/library/model.rs` — `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`, read-only resolution types
- `src/lib.rs` — crate root: module tiers, root re-exports, `SnippetData` / `ProcessResult` / `SelectionOutcome`
- `src/commands/mod.rs` — `get_snippet_data` (`:243`), `run_snippet_selection` (`:338`)
- `src/main.rs` — the `Err(SnipError)` → exit 1 boundary (`:1026-1032`) and the exit-10 path (`:375-388`)

## Invariants

- **One error type.** Every fallible crate function returns
  `SnipResult<T>` = `Result<T, SnipError>` (`src/error.rs:314`). There are no
  ad-hoc error enums in command modules.
- **`SnipError` is `#[non_exhaustive]`** — adding variants is allowed;
  renumbering or changing payloads is a breaking change for downstream
  matchers.
- **Typed sync failures never string-match.** `SyncFailureKind` →
  `FailureClass` is a total match (`src/sync_failure.rs:36-60`).
- **Errors carry no credentials.** No API key, token, or keychain value is
  ever placed in an error payload.
- **`CliOutcome` is `#[non_exhaustive]`** and exit codes are stable — never
  renumber 0–10 (`docs/EXIT_CODES.md`).
- **Local-only fields never reach the sync fingerprint.** `output`,
  `folders`, and `favorite` are excluded from the SHA-256 that drives
  conflict resolution (see [sync.md](sync.md)).

## Gotchas

- **`tags` serializes as `tag`.** The Rust field is `tags`, the TOML key is
  `tag` (`src/library/model.rs:37-45`); `tags`/`Tags`/`Tag` are accepted on
  read only.
- **Snippet IDs are deterministic, not random.** `legacy-<64 hex>` derived
  from content + occurrence index (`docs/IDENTITY_CONTRACT.md`). Do not
  treat `Snippet::id` as a UUID.
- **`output` is not in `ProtoSnippet`.** `snp edit --output` requires an
  exact selector or `--filter`.
- **`ProcessResult::is_done()` is the usage-recording gate.** A `Failed`
  child writes no usage metadata.
- **Exit 10 has no `CliOutcome` variant.** It only exists in
  `exit_on_repair_status` (`src/main.rs:375-388`); a caller that tries to
  match it on `CliOutcome` will not find it.
- **`SnipError::Runtime` classification is substring-based** and therefore
  looser than the typed `SyncFailure` path — prefer `sync_failure()` for
  anything the sync layer classifies.

# Public API Surface

[← Back to Overview](overview.md) · [← Back to Core](core.md)

## Overview


`snip-it` is a package with **two** targets: the `snp` binary (`src/main.rs`)
and the `snip_it` library (`src/lib.rs`). The library half exists for three
reasons: the `snp` binary is a separate crate in the same package and
can only see `pub` items; integration tests under `tests/` need to reach
internal machinery (`sync::SyncClient`, proto types, transaction helpers);
and a few types must be `pub` for `#[derive(clap::ValueEnum)]` on CLI
arguments.

This doc describes the **architecture** of that surface: the visibility tiers,
which contract is enforced and by which test, and which re-exports are
load-bearing for the TUI. It deliberately does **not** duplicate the
item-by-item inventory in
[`docs/PUBLIC_API.md`](../docs/PUBLIC_API.md), which lists every public item
and records recommended narrowing. Read that doc for the exhaustive table;
read this one for the rules.

## Visibility tiers


`src/lib.rs` splits the crate into three tiers, marked by explicit section
comments in the file.

### Tier 1 — Supported public API (stable)

Declared at `src/lib.rs:26-30`, re-exported at `src/lib.rs:72-80`.

```rust
pub mod config;
pub mod error;
pub mod outcome;
pub mod sort;

pub use error::{SnipError, SnipResult};
pub use library::{LibraryConfig, LibraryMeta, Snippet, Snippets, load_library, save_library};
pub use utils::atomic::{
    AtomicWriteOptions, AtomicWriteReport, Durability, atomic_replace, write_private_atomic,
};
```

| Root re-export | Source | Why it is public |
|---|---|---|
| `SnipError`, `SnipResult` | `src/error.rs` | The crate's single error type; every fallible API returns it |
| `LibraryConfig`, `LibraryMeta`, `Snippet`, `Snippets` | `src/library/model.rs` | The domain data model, re-exported out of the `pub(crate) mod library` |
| `load_library`, `save_library` | `src/library/persistence.rs:158`, `:217` | Read/write one library TOML file by path |
| `AtomicWriteOptions`, `AtomicWriteReport`, `Durability`, `atomic_replace`, `write_private_atomic` | `src/utils/atomic.rs` | The durability-preserving write primitives |

`pub mod config`, `pub mod error`, `pub mod outcome`, and `pub mod sort` are
the four stable modules:

| Module | Supported surface |
|---|---|
| `config` | `SyncSettings`, `AutoSyncFailureMode`, `SyncDirection`, `DEFAULT_SERVER_URL`, `AUTO_SYNC_DEBOUNCE_MAX`, … (`src/config/sync_settings.rs:24-132`) |
| `error` | `SnipError`, `SnipResult`, `SyncFailureKind` (`src/error.rs`) |
| `outcome` | `CliOutcome`, `exit_code::*`, `OutputContext`, `ColorPolicy`, `OutputMode` (`src/outcome.rs`) |
| `sort` | `SnippetSort`, `SortOptions`, `rank_snippets` (`src/sort.rs:40-54`, `:116-122`) |

**Stable means:** the item stays `pub`, keeps its name and signature, and
its variants are not removed. `#[non_exhaustive]` on `SnipError`,
`CliOutcome`, `SyncFailureKind`, `AutoSyncFailureMode`, and `SyncDirection`
lets *new* variants be added without breaking downstream matchers.

### Tier 2 — `#[doc(hidden)] pub` (implementation-only)

Declared at `src/lib.rs:32-51` and `src/lib.rs:67-69`.

| Module | Role | Why it is `pub` |
|---|---|---|
| `auto_sync` | Detached debounce worker, pending marker, policy | The binary spawns the worker; integration tests drive pending-state scenarios |
| `commands` | CLI command implementations (`src/commands/mod.rs:8-32`) | One module per subcommand plus shared helpers (`backup_archive`, `doctor_report`, `pet_analysis`); the binary calls every handler |
| `logging` | File logging + audit log | The binary initializes logging at startup |
| `mcp` | Local read-only MCP adapter | The `snp mcp serve` subcommand and MCP integration tests |
| `process_file_lock` | Kernel-backed cross-process lock | Used by the auto-sync lock and the server singleton |
| `selector` | Deterministic non-TUI resolution | `resolve_exact_target`, `searchable_text` — shared by CLI, MCP, and diagnostics |
| `sync` | gRPC client (`SyncClient`) | Sync integration tests need a real client |
| `ui` | Ratatui TUI | The binary renders the selector |
| `usage` | Local-only usage index | Sort modes and `run`/`clip` writers |
| `transaction` | Journal + rollback | `pub(crate)` by default; becomes `pub` **only** under the `test-support` feature (`src/lib.rs:65-69`) |

Root-level TUI glue types are also `#[doc(hidden)]`:

| Type | Role |
|---|---|
| `SnippetData` (`src/lib.rs:104-111`) | Parallel vectors for TUI display |
| `ProcessResult` (`src/lib.rs:117-131`) | Per-snippet loop control inside the selection callback |
| `SelectionOutcome` (`src/lib.rs:147-157`) | Raw TUI selection result |

**`#[doc(hidden)]` means:** the item is `pub` so the binary and integration
tests can reach it, but it is **not** part of the supported external API and
may change in any release. `#[doc(hidden)]` also keeps it out of the
generated rustdoc, which is the crate's way of saying "do not depend on
this".

### Tier 3 — `pub(crate)` (crate-internal)

Declared at `src/lib.rs:53-70`.

`clipboard`, `diagnostics`, `encryption`, `library`, `local_data`,
`migration`, `output`, `status_snapshot`, `sync_commands`, `sync_failure`,
`test_failpoints`, `utils`, plus `pub(crate) use snip_proto as proto`
(`src/lib.rs:43`).

`library` is internal, yet its data types (`Snippet`, `Snippets`,
`LibraryConfig`, `LibraryMeta`) and its `load_library` / `save_library`
functions are re-exported at the root (`src/lib.rs:77`) — the module is
private, the *types* are supported.

## The enforced contract


`tests/public_api_smoke.rs` is the executable contract for Tier 1. It
imports only from the supported surface:

```rust
use snip_it::config::{
    AUTO_SYNC_DEBOUNCE_MAX, AutoSyncFailureMode, DEFAULT_SERVER_URL, SyncDirection, SyncSettings,
};
use snip_it::error::{SnipError, SyncFailureKind};
use snip_it::outcome::{CliOutcome, OutputContext, exit_code};
use snip_it::sort::{SnippetSort, SortOptions, rank_snippets};
use snip_it::{
    AtomicWriteOptions, Durability, LibraryConfig, LibraryMeta, Snippet, Snippets, load_library,
};
```

What it proves (`tests/public_api_smoke.rs:20-92`):

| Test | Contract |
|---|---|
| `supported_root_types_roundtrip_through_toml` | `Snippet` / `Snippets` / `LibraryConfig` / `LibraryMeta` still resolve and round-trip; the serialized form still contains `[[snippets]]` — the serde renames and aliases have not drifted (`:21-47`) |
| `supported_load_library_reads_isolated_file` | `load_library(&Path)` reads a library file from an arbitrary path, in a `TempDir` — no config-dir or keychain access (`:49-61`) |
| `supported_config_sort_outcome_error_types_resolve` | `SyncSettings::default()` matches the documented constants; `rank_snippets` works with `None` usage; `CliOutcome::exit_code()` agrees with `exit_code::*`; `SnipError::runtime_error` is constructible; `AtomicWriteOptions::for_durability(Durability::…)` is constructible (`:63-92`) |

The test asserts **exact counts**, not `>= 1` — a shape change is a failure,
not a warning.

**What it does not cover.** The smoke test pins the *Tier 1* surface only.
Tier 2 (`#[doc(hidden)]`) modules are covered indirectly by the integration
tests that use them (`tests/` has 51 top-level targets), not by a single
contract test. A change to `#[doc(hidden)] pub` items will not be caught by
`public_api_smoke.rs` — it is caught by whichever integration test happens to
use the item, or not at all.

`tests/architecture.rs` is a second, separate guard: it pins layer boundaries
and the updater transport split (see [update.md](update.md)). It does not
check API visibility.

## Load-bearing re-exports


Two re-export sites change behavior for **every** TUI command at once. This
is why they are called out explicitly in `AGENTS.md`.

### `commands/mod.rs`

| Re-export | Line | Affects |
|---|---|---|
| `run_snippet_selection` | `src/commands/mod.rs:338` | `run`, `clip`, `search`, `select` — all four TUI commands call it (`src/commands/run_cmd.rs:349`, `src/commands/clip_cmd.rs:107`, `src/commands/search_cmd.rs:58`, `src/commands/select_cmd.rs:85`) |
| `load_snippets` | `src/commands/mod.rs:157` | The shared TOML load path (`src/commands/list_cmd.rs:63`, `src/commands/new_cmd.rs:630`) |
| `save_snippets` | `src/commands/mod.rs:206` | The shared TOML save path, including the mutation gate and local-data lock (`src/commands/new_cmd.rs:646`) |
| `load_readonly_snippets` | `src/commands/mod.rs:127` | The **read-only** resolution path used by `snp list` (`src/commands/list_cmd.rs:71`) — never migrates or rewrites metadata |
| `get_snippet_data` | `src/commands/mod.rs:243` | Builds the `SnippetData` parallel vectors for the TUI |
| `expand_snippet_command` | `src/commands/mod.rs:274` | Variable expansion shared by `run` and `clip` |

`run_snippet_selection` is the **single** TUI entry point: it resolves the
library, loads the usage index, builds the one-shot usage map, calls
`ui::select_snippet`, and optionally triggers a background sync
(`src/commands/mod.rs:338-496`). It takes `Option<&tokio::runtime::Runtime>`
and errors when `do_sync` is true but the runtime is `None`
(`src/commands/mod.rs:353-362`).

`load_readonly_snippets` is the read-only sibling of `load_snippets`. It
calls `library::readonly_library_sources()`
(`src/library/manager.rs:784-787`), which resolves the visible libraries
without creating library files or rewriting library metadata. Routing a
read-only command through the mutating `get_library_path` would reintroduce
recovery side effects — that is the reason the two functions are separate.

> One caveat to the "no migration" claim: `readonly_library_sources()`
> constructs a `LibraryManager` (`src/library/manager.rs:41-92`), and
> `LibraryManager::new()` calls `migrate_macos_config_dir()`
> (`src/utils/config.rs:166-179`). On macOS with a legacy config directory
> present, that migration **does** create directories and move files — so
> "read-only" means "does not mutate the snippet library", not "never touches
> the filesystem".

> `src/mcp/tools.rs:234` defines its **own** local `load_snippets`. It is a
> different function with a different signature (`Option<&str>` →
> `Vec<LoadedSnippet>`) and is unrelated to `commands::load_snippets`.

### `ui/mod.rs`

| Re-export | Line | Affects |
|---|---|---|
| `select_snippet` + `SnippetListParams` | `src/ui/mod.rs:527`, `:499-513` | The only TUI entry point, called from exactly one place — `src/commands/mod.rs:403` |
| `get_theme` | `src/ui/mod.rs:16` | Theme resolution for every TUI render |
| `prompt_variables` + `VariablePromptResult` | `src/ui/mod.rs:17` | Variable prompting inside the TUI |
| `get_terminate` | `src/ui/mod.rs:62-64` | The shared SIGINT/SIGTERM flag, also used by `main.rs`'s Unix signal handler (`src/main.rs:35`) |

Because `select_snippet` has exactly one call site, any change to the
selection contract — the params struct, the sort options, the usage slice,
or `allow_delete` — lands on all four TUI commands simultaneously.

## Stable vs internal — the decision rule


| Question | Answer |
|---|---|
| Is the item in `docs/PUBLIC_API.md`'s "Supported API (stable)" table? | It is Tier 1 and stable |
| Is it `#[doc(hidden)]`? | It is public for binary/test access only; **not** a supported external API |
| Is it `pub(crate)`? | It is internal; the binary cannot see it either |
| Is it a `pub` item inside a `pub(crate)` module (e.g. `library::model::Snippet`)? | It is reachable only through the root re-export; the module path itself is not supported |

The crate's own comment states the intent
(`src/lib.rs:74-76`): *"The library data types and atomic write utilities are
the supported public surface; the rest is exposed for crate-boundary
reasons."*

## Related


- [`docs/PUBLIC_API.md`](../docs/PUBLIC_API.md) — the exhaustive public-item
  inventory and recommended narrowing. This doc supplements it; it does not
  repeat it.
- [core.md](core.md) — the error type and domain types that form Tier 1.
- [cli.md](cli.md) — the binary entry point that consumes Tier 2.
## Key Files


- `src/lib.rs` — the whole contract: module tiers (`:26-70`), root re-exports (`:72-80`), hidden root types (`:99-158`)
- `src/library/mod.rs` — `pub(crate)` module whose public types are re-exported at the root
- `src/utils/atomic.rs` — the atomic-write surface re-exported at the root (`Durability` `:24`, `AtomicWriteOptions` `:45`, `AtomicWriteReport` `:86`, `write_private_atomic` `:278`, `atomic_replace` `:350`)
- `src/config/sync_settings.rs` — the supported `config` surface (`:24-132`, `:419-420`)
- `src/sort.rs` — the supported `sort` surface (`:40-54`, `:116-122`)
- `src/commands/mod.rs` — load-bearing re-exports (`:127`, `:157`, `:206`, `:243`, `:274`, `:338`)
- `src/ui/mod.rs` — load-bearing TUI re-exports (`:16-17`, `:62`, `:499-513`, `:527`)
- `tests/public_api_smoke.rs` — the executable Tier 1 contract
- `tests/architecture.rs` — layer-boundary and transport-split guard
- `docs/PUBLIC_API.md` — the exhaustive item-by-item inventory (not duplicated here)

## Invariants


- **Tier 1 items are additive-only.** Removing or renaming a Tier 1 item, or
  changing a `pub` field's type, breaks `tests/public_api_smoke.rs` and
  every downstream consumer.
- **`#[doc(hidden)]` is not a stability promise.** Tier 2 items may change
  or disappear in any release without a semver bump.
- **`pub(crate)` is invisible to the binary.** The `snp` binary is a
  separate crate; it can only reach `pub` items, so a `pub(crate)` module
  cannot be called from `main.rs` directly — it must be re-exported.
- **One contract test, one scope.** `tests/public_api_smoke.rs` guards
  Tier 1 only.
- **The root re-export list is the public contract.** Adding a `pub` item to
  `src/lib.rs` root is a deliberate act, not a side effect of refactoring.

## Gotchas


- **`transaction` flips visibility with a feature flag.** Without
  `test-support` it is `pub(crate)` (`src/lib.rs:65-66`); with it, it becomes
  `#[doc(hidden)] pub` (`src/lib.rs:67-69`). Code that must work in both
  configurations cannot assume either.
- **`pub mod library` does not exist.** The path is
  `snip_it::library::…` only through the root re-exports; the module itself
  is `pub(crate)` (`src/lib.rs:57`).
- **`snip_it::proto` is `pub(crate)`.** Protocol types come from the
  separate `snip-proto` crate, not from `snip_it::proto`
  (`src/lib.rs:43`).
- **`sync::detect_device_conflict()` no longer exists.** It was **removed**
  in commit `1920075`; there is no `detect_device_conflict` anywhere under
  `src/`, and `docs/PUBLIC_API.md` no longer lists it either (line 172 is now
  `SyncClient::get_premade_library()`). Conflict counts come from the sync
  module's own resolution path — do not reintroduce the helper.
- **`mcp::tools::load_snippets` is not `commands::load_snippets`.** Same
  name, different module, different signature.
- **Adding a `pub` item to `src/lib.rs` root changes the public API.** There
  is no `#![deny(missing_docs)]` gate — review the diff.


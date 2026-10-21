# Commands Module (`src/commands/mod.rs`)

[← Back to Overview](../overview.md)

## Overview

`src/commands/mod.rs` (641 lines) is the shared foundation for all 25 CLI
command submodules. It declares every `pub mod` (`backup_archive`,
`backup_cmd`, `clip_cmd`, `cron_cmd`, `doctor_cmd`, `doctor_report`,
`edit_cmd`, `get_cmd`, `import_cmd`, `keybindings_cmd`, `library_cmd`,
`list_cmd`, `new_cmd`, `pet_analysis`, `premade_cmd`, `register_cmd`,
`repair_cmd`, `restore_cmd`, `run_cmd`, `search_cmd`, `select_cmd`,
`shell_cmd`, `status_cmd`, `sync_cmd`, `validate_cmd`) and provides the
helpers every command builds on: config/library path resolution, legacy
TOML load/save, TUI data extraction, variable expansion, the shared
TUI selection loop, and the canonical explicit-sync path.

This module does **not** define CLI flags. The canonical `*Args` structs live
beside each handler (`NewArgs`, `RunArgs`, `ClipArgs`, `GetArgs`, …) and
`src/main.rs` dispatches into them.

## Shared plumbing types

- `ExpandedCommand` (`mod.rs:42`) — `Cancel` / `Skip` / `Expanded(String)`,
  returned by `expand_snippet_command`.
- `run_snippet_selection` generics — `process_fn:
  FnMut(&Snippet, Option<String>) -> SnipResult<ProcessResult>`, where the
  `Option<String>` is the TUI copy flag and `ProcessResult` is
  `Cancel | Continue | Done(msg) | Failed { exit_code, .. }`.

## The mutating / read-only split

This is the single most important invariant in the module. Two families of
path resolution exist and they are **not** interchangeable.

| Helper | Mutating? | Mechanism |
|--------|-----------|-----------|
| `get_library_path` (`mod.rs:91`) | **Yes** | calls `LibraryManager::ensure_library_mode()` (`mod.rs:96`), which migrates a legacy single-file checkout |
| `init_library_manager` (`mod.rs:150`) | **Yes** | `new()` + `ensure_library_mode()` (`mod.rs:152`) |
| `resolve_selector` (`src/selector.rs:681`) | **Yes** | `ensure_library_mode()` at `src/selector.rs:683` |
| `load_readonly_snippets` (`mod.rs:127`) | No | `library::readonly_library_sources` — no migration, no file creation |
| `resolve_selector_readonly` (`src/selector.rs:623`) | No | `library::readonly_library_sources` (`src/selector.rs:629`) |

`ensure_library_mode()` on a legacy single-file checkout creates `libraries/`,
copies the legacy `snippets.toml` (leaving the original behind as a second,
now-inert copy), writes `libraries.toml`, and bumps the generation. A
read-only path must therefore never route through `get_library_path`,
`init_library_manager`, or `resolve_selector`.

## Path resolution

### `get_config_path` (`mod.rs:52`)

Resolves the legacy single-file config path from the `--config` CLI
argument or the XDG default (`utils::config::get_snippets_path`).
A single `fs::metadata` call classifies the path: regular file → use it;
`NotFound` → create parents + `create_new` (tolerates a concurrent
`AlreadyExists` that is a file); directory/other → `runtime_error`;
other I/O errors → `io_error`.

Note that this helper **creates** a missing file, so it is not side-effect
free even though it looks like a pure resolver.

### `get_library_path` (`mod.rs:91`)

```rust
pub fn get_library_path(library_name: Option<String>) -> SnipResult<Option<PathBuf>>
```

Builds a `LibraryManager`, calls `ensure_library_mode()` (`mod.rs:96`), then
maps a filename to `<libraries_dir>/<filename>.toml` (`mod.rs:100-107`), or
returns the primary library path for `None` (`mod.rs:108-111`). An unknown
name yields `library_not_found(&name)` (`mod.rs:102`).

### `init_library_manager` (`mod.rs:150`)

The two-line `new()` + `ensure_library_mode()` constructor used by sync,
new, and library commands. Also mutating.

### `load_readonly_snippets` (`mod.rs:127`)

```rust
pub fn load_readonly_snippets(
    library: Option<&str>,
) -> SnipResult<(
    Vec<crate::library::ResolvedLibrarySource>,
    crate::library::Snippets,
)>
```

The canonical read-only loader. It calls
`crate::library::readonly_library_sources(library)` (`mod.rs:133`), which
performs no migration and creates no files or directories, then loads each
resolved source with `load_library` and concatenates the snippets
(`mod.rs:136-144`). Folders are unioned without duplicates (`mod.rs:139-142`).

It delegates the `"all"` scope to
`crate::library::readonly_library_sources` →
`LibraryManager::resolve_readonly_sources`
(`src/library/manager.rs:248-278`), which matches the literal itself and
returns every visible library. `LibraryScope::from_filter_arg`
(`src/selector.rs:65`) is the keyword owner for the *selector* paths
(`snp get`, exact-mode `run`/`clip`/`edit`); both layers agree that `"all"`
means "every visible library", so `--library all` means the same thing here
as on `snp get`, exact-mode `run`/`clip`/`edit`, and MCP.

**Consumers**: `snp list` (`src/commands/list_cmd.rs:71`), `snp get` via
`resolve_selector_readonly` (`src/selector.rs:629`), and the MCP
`snippets_list` / `snippets_search` / `snippet_get` tools.

## Legacy single-file TOML

### `load_snippets` (`mod.rs:157`)

Missing/empty file → `Snippets::default()` (`mod.rs:162-175`); otherwise
`fix_invalid_toml_escapes` then `toml::from_str` (`mod.rs:177-195`).
Parse failure writes a `.toml.bak` copy (best-effort, warned) and returns a
TOML error. Every outcome goes through `logging::log_config_operation`.

### `save_snippets` (`mod.rs:206`)

Legacy single-file save path (used for `--config` flows):

1. `get_config_path` (`mod.rs:207`).
2. **Gate** — `gate_mutation_on_interrupted_transactions(&sync_state_dir,
   &transaction_dir)` (`mod.rs:212-215`). One journal = auto-rollback;
   multiple/incomplete = refuse and direct the user to `snp repair`.
3. **Lock** — `acquire_local_data_lock(&transaction_dir)` (`mod.rs:218`),
   which serializes against backup snapshot capture.
4. Best-effort `backup_library` (warn-only on failure, `mod.rs:220-222`).
5. `toml::to_string_pretty` with **no post-processing** (the golden corpus
   must survive), written via `write_private_atomic` (`mod.rs:224-231`).
6. TOML cache invalidated (`mod.rs:233`), operation logged (`mod.rs:235`).

The same gate → lock → write ordering is used by
`library::save_library` (`src/library/persistence.rs:217-238`), which
`save_snippets` mirrors for the legacy path.

## TUI data and expansion

### `get_snippet_data` (`mod.rs:243`)

```rust
pub fn get_snippet_data(snippets: &crate::library::Snippets) -> (crate::SnippetData, Vec<usize>)
```

Filters out `deleted` tombstones (`mod.rs:244-249`) and returns parallel
`SnippetData` arrays (descriptions/commands/outputs/tags/folders/favorites)
plus the `original_indices` map back into `snippets.snippets`
(`mod.rs:250-270`). Consumed by the TUI `select_snippet` call inside
`run_snippet_selection`. **Tombstones are invisible to the TUI** — a deleted
snippet is never offered for selection.

### `expand_snippet_command` (`mod.rs:274`)

Parses `<name>` / `<name=default>` variables with `parse_variables`; with
none, returns the escape-stripped command (`mod.rs:279-283`). Otherwise
delegates to the interactive `ui::prompt_variables` modal and maps
`Cancel → Cancel`, `Back`/`Skip → Skip`, `Values → expand_command`
(`mod.rs:285-292`). Never executes anything.

`expand_command` (`src/utils/variables.rs:644`) walks the command and
substitutes each token. Because `extract_variable_tokens`
(`src/utils/variables.rs:178`) and the substitution scanner are two separate
parsers, the scanner resynchronizes its token cursor when they disagree
(`src/utils/variables.rs:718-744`) — a single disagreement stays local
instead of silently failing every later variable in the command.

## Sync

### `run_explicit_sync` (`mod.rs:301`)

The single canonical explicit-sync implementation shared by TUI `--sync`
paths and exact-selector `--sync` paths:

1. `observe_pending_generation()` first (`mod.rs:303`) — a mutation racing
   the sync is preserved.
2. `wait_acquire` the `SyncExecutionLock` (30 s, `mod.rs:304-307`);
   failure → `runtime_error("could not acquire sync lock")`.
3. `sync_commands::run_default_sync(runtime)` (`mod.rs:309`); warn-only on
   failure (`mod.rs:311-313`).
4. `clear_pending_after_explicit_sync(observed, succeeded)` (`mod.rs:314`)
   so a successful foreground sync suppresses a duplicate delayed auto-sync.

Must be called **after** the local mutation has committed.

### `run_snippet_selection` (`mod.rs:338`)

```rust
pub fn run_snippet_selection<F>(
    filter: Option<String>,
    library: Option<String>,
    do_sync: bool,
    allow_delete: bool,
    sort_opts: Option<SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
    mut process_fn: F,
) -> crate::error::SnipResult<crate::SelectionOutcome>
```

The shared TUI loop behind `snp run`, `snp clip`, `snp search`, and
`snp select`. It requires `runtime.is_some()` when `do_sync` is set, else
it errors before touching disk (`mod.rs:353-362`).

1. Resolve `lib_path` via `get_library_path` (`mod.rs:364`) — **this is the
   mutating resolver**; the TUI commands still migrate a legacy checkout.
   No library → print a hint and return `SelectionOutcome::Cancelled`
   (`mod.rs:365-370`).
2. `load_library` + `UsageIndex::load` (`mod.rs:371-373`); build a one-shot
   `usage_map` (O(1) per-snippet lookup, not O(k·m), `mod.rs:386-390`).
3. Loop: `get_snippet_data` → `ui::select_snippet` (fuzzy, sort,
   usage-aware, `mod.rs:403-416`) → match:
   - `Cancelled` / `None` → `Cancelled` (`mod.rs:419-422`, `mod.rs:477-480`).
   - `Delete(idx)` with `allow_delete` → `mark_snippet_deleted`
     (tombstone + `updated_at = max(now)+1`), `save_library`, audit
     `delete` (`mod.rs:423-434`); then explicit sync if `do_sync`, else
     `notify_mutation(SnippetDelete, User)` (`mod.rs:435-449`). Without
     `allow_delete`, `Delete` is ignored (`mod.rs:452`).
   - `Copied` → `Selected` (clipboard TUI shortcut, `mod.rs:453-456`).
   - `Selected(idx, copy_flag)` → `process_fn`; `Cancel → Cancelled`,
     `Continue → loop`, `Done → Selected`,
     `Failed{exit_code} → ExecutionFailed` (`mod.rs:457-475`).
4. After the loop, if `do_sync && selected_and_processed`, one trailing
   `run_explicit_sync` (warn-only on failure, `mod.rs:482-488`).

`mark_snippet_deleted` (`mod.rs:499`) sets `deleted = true` and bumps
`updated_at`, preserving the tombstone for sync (no resurrection).

## Mutation vs read-only

`mod.rs` itself mutates only through `save_snippets` (transaction-gated,
local-locked) and through the delete branch of `run_snippet_selection`
(`save_library`, which gates inside `src/library/persistence.rs:225-233`).
All other helpers are read-only. Callers decide: `run`/`clip`/`search`/`select`
TUI paths are read-local except the delete shortcut; `new`/`edit`/`import`/
`restore` are mutating.

**`gate_mutation_on_interrupted_transactions()`** is called by
`save_snippets` (`mod.rs:212`) and by `library::save_library`
(`src/library/persistence.rs:225`) — never by a read path.

## Auto-sync trigger

- `run_snippet_selection` delete branch: `notify_mutation(
  MutationKind::SnippetDelete, MutationOrigin::User)` when `do_sync`
  is false (`mod.rs:444-447`); explicit sync when true
  (`mod.rs:435-440`).
- `run_explicit_sync`: no `notify_mutation` — it *is* the sync
  (plus pending-clear). SyncMerge origin never re-triggers by policy.

## Error / exit mapping

Helpers return `SnipResult`; exit mapping lives in `src/outcome.rs` via the
caller: `SelectionOutcome::Cancelled → Success` (for `run`) or
`Cancelled` (exit 4, for `select`); `ExecutionFailed{exit_code} →
CliOutcome::ExecutionFailed` (exit 8, or the child code via
`src/main.rs:518` / `src/main.rs:542`). Missing runtime with `do_sync` is a
`runtime_error("sync requested but no runtime")` (`mod.rs:355-358`).

## Key invariants

- `run_snippet_selection` takes `Option<&Runtime>` — `None` whenever
  `do_sync` is false. Local-only commands never init the global Tokio
  runtime.
- Read-only callers use `load_readonly_snippets` / `resolve_selector_readonly`,
  never `get_library_path` / `init_library_manager` / `resolve_selector`.
- The retry/explicit-sync path never uses a closure-based generic retry
  helper (borrow rules); `sync.rs` keeps the inline `retry_grpc_unified!`
  macro for the same reason.
- Tombstones are preserved on delete; `updated_at` strictly increases.
- Save path never post-processes `toml::to_string_pretty`.
- `Drop` releases kernel locks without unlinking; lock files may hold
  stale metadata.

## Gotchas

- `get_config_path` creates a missing file, so "read the config" is not
  always free of side effects on the legacy path.
- `run_snippet_selection` uses the *mutating* resolver. A TUI command
  (`run`, `clip`, `search`, `select`) can still migrate a legacy
  single-file checkout even though it never writes a snippet.
- `get_snippet_data` silently drops tombstones, so a TUI index never
  matches the on-disk snippet count.
- The delete shortcut is a real library mutation hidden inside a
  "read-only-looking" TUI command.

## Key Files

- `src/commands/mod.rs` — this module: shared helpers, the shared TUI
  selection loop, and the canonical explicit-sync path.
- `src/selector.rs` — `LibraryScope`, `searchable_text`, `exact_selector`,
  `resolve_selector` (mutating) vs `resolve_selector_readonly`.
- `src/library/manager.rs` — `ensure_library_mode` (mutating, `:163`) and
  `readonly_library_sources` (read-only, `:784`) behind both resolvers.
- `src/library/persistence.rs` — `load_library` / `save_library`, the
  gate → lock → atomic-write persistence path.
- `src/utils/variables.rs` — `parse_variables`, `expand_command`,
  `strip_escape_sequences`.
- `src/outcome.rs` — `CliOutcome` and the exit-code table.
- `src/main.rs` — dispatch into the per-command `run()` functions.

## File / line references

- Module decls: `src/commands/mod.rs:8-32`
- `ExpandedCommand`: `:42`; `get_config_path`: `:52`
- `get_library_path`: `:91`; `load_readonly_snippets`: `:127`
- `init_library_manager`: `:150`; `load_snippets`: `:157`
- `save_snippets`: `:206` (gate `:212`, lock `:218`)
- `get_snippet_data`: `:243`; `expand_snippet_command`: `:274`
- `run_explicit_sync`: `:301`; `run_snippet_selection`: `:338`
- `mark_snippet_deleted`: `:499`

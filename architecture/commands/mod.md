# Commands Module (`src/commands/mod.rs`)

[← Back to Overview](../overview.md)

## Overview

`src/commands/mod.rs` (609 lines) is the shared foundation for all 25 CLI
command submodules. It declares every `pub mod` (`backup_archive`,
`backup_cmd`, `clip_cmd`, `cron_cmd`, `doctor_cmd`, `doctor_report`,
`edit_cmd`, `get_cmd`, `import_cmd`, `keybindings_cmd`, `library_cmd`,
`list_cmd`, `new_cmd`, `pet_analysis`, `premade_cmd`, `register_cmd`,
`repair_cmd`, `restore_cmd`, `run_cmd`, `search_cmd`, `select_cmd`,
`shell_cmd`, `status_cmd`, `sync_cmd`, `validate_cmd`) and provides the
helpers every command builds on: config/library path resolution, legacy
TOML load/save, TUI data extraction, variable expansion, the shared
TUI selection loop, and the canonical explicit-sync path.

## CLI surface

`mod.rs` defines no CLI flags itself. The canonical `*Args` structs live
beside each handler (`NewArgs`, `RunArgs`, `ClipArgs`, `GetArgs`, …) and
`src/main.rs` dispatches into them. Two shared plumbing types live here:

- `ExpandedCommand` (`mod.rs:42`) — `Cancel` / `Skip` / `Expanded(String)`,
  returned by `expand_snippet_command`.
- `run_snippet_selection` generics — `process_fn:
  FnMut(&Snippet, Option<String>) -> SnipResult<ProcessResult>`, where the
  `Option<String>` is the TUI copy flag and `ProcessResult` is
  `Cancel | Continue | Done(msg) | Failed { exit_code, .. }`.

## Flow / steps

### `get_config_path` (`mod.rs:52`)

Resolves the legacy single-file config path from the `--config` CLI
argument or the XDG default (`utils::config::get_snippets_path`).
A single `fs::metadata` call classifies the path: regular file → use it;
`NotFound` → create parents + `create_new` (tolerates a concurrent
`AlreadyExists` that is a file); directory/other → `runtime_error`;
other I/O errors → `io_error`.

### `get_library_path` (`mod.rs:91`) / `init_library_manager` (`mod.rs:118`)

`get_library_path(library_name)` builds a `LibraryManager`, calls
`ensure_library_mode()`, then maps a filename to
`<libraries_dir>/<filename>.toml`, or returns the primary library path
for `None`. `init_library_manager()` is the two-line
`new()` + `ensure_library_mode()` constructor used by sync, new, and
library commands.

### `load_snippets` / `save_snippets` (`mod.rs:125`, `mod.rs:175`)

Legacy single-file TOML path (kept for `--config` flows):

1. `load_snippets` — missing/empty file → `Snippets::default()`; otherwise
   `fix_invalid_toml_escapes` then `toml::from_str`. Parse failure writes
   a `.toml.bak` copy (best-effort, warned) and returns a TOML error.
   Every outcome goes through `logging::log_config_operation`.
2. `save_snippets` — gates on interrupted transactions
   (`gate_mutation_on_interrupted_transactions`, `mod.rs:180`), acquires
   the local-data lock (serializes against backup snapshot capture),
   best-effort `backup_library`, then `toml::to_string_pretty` with
   **no post-processing** (golden corpus must survive) written via
   `write_private_atomic`, cache invalidated, operation logged.

### `get_snippet_data` (`mod.rs:211`)

Filters out `deleted` tombstones and returns parallel `SnippetData`
arrays (descriptions/commands/outputs/tags/folders/favorites) plus the
`original_indices` map back into `snippets.snippets`. Consumed by the
TUI `select_snippet` call inside `run_snippet_selection`.

### `expand_snippet_command` (`mod.rs:242`)

Parses `<name>` / `<name=default>` variables; with none, returns the
escape-stripped command. Otherwise delegates to the interactive
`ui::prompt_variables` modal and maps `Cancel → Cancel`,
`Back/Skip → Skip`, `Values → expand_command`. Never executes anything.

### `run_explicit_sync` (`mod.rs:269`)

The single canonical explicit-sync implementation shared by TUI
`--sync` paths and exact-selector `--sync` paths:

1. `observe_pending_generation()` first (a mutation racing the sync is
   preserved).
2. `wait_acquire` the `SyncExecutionLock` (30 s); failure → runtime error.
3. `sync_commands::run_default_sync(runtime)`; warn-only on failure.
4. `clear_pending_after_explicit_sync(observed, succeeded)` so a
   successful foreground sync suppresses a duplicate delayed auto-sync.

Must be called **after** the local mutation has committed.

### `run_snippet_selection` (`mod.rs:306`)

Signature: `(filter, library, do_sync, allow_delete, sort_opts,
runtime: Option<&Runtime>, process_fn)`. Requires `runtime.is_some()`
when `do_sync` is set, else errors before touching disk.

1. Resolve `lib_path` via `get_library_path`; no library → print a hint
   and return `SelectionOutcome::Cancelled`.
2. `load_library` + `UsageIndex::load`; build a one-shot `usage_map`
   (O(1) per-snippet lookup, not O(k·m)).
3. Loop: `get_snippet_data` → `ui::select_snippet` (fuzzy, sort,
   usage-aware) → match:
   - `Cancelled` / `None` → `Cancelled`.
   - `Delete(idx)` with `allow_delete` → `mark_snippet_deleted`
     (tombstone + `updated_at = max(now)+1`), `save_library`, audit
     `delete`; then explicit sync if `do_sync`, else
     `notify_mutation(SnippetDelete, User)`. Without `allow_delete`,
     `Delete` is ignored.
   - `Copied` → `Selected` (clipboard TUI shortcut).
   - `Selected(idx, copy_flag)` → `process_fn`; `Cancel → Cancelled`,
     `Continue → loop`, `Done → Selected`,
     `Failed{exit_code} → ExecutionFailed`.
4. After the loop, if `do_sync && selected_and_processed`, one trailing
   `run_explicit_sync` (warn-only on failure).

`mark_snippet_deleted` (`mod.rs:467`) sets `deleted = true` and bumps
`updated_at`, preserving the tombstone for sync (no resurrection).

## Mutation vs read-only

`mod.rs` itself mutates only through `save_snippets` (transaction-gated,
local-locked) and through the delete branch of `run_snippet_selection`
(`save_library`, which gates inside `LibraryManager`). All other helpers
are read-only. Callers decide: `run`/`clip`/`search` TUI paths are
read-local except the delete shortcut; `new`/`edit`/`import`/`restore`
are mutating.

## Auto-sync trigger

- `run_snippet_selection` delete branch: `notify_mutation(
  MutationKind::SnippetDelete, MutationOrigin::User)` when `do_sync`
  is false; explicit sync when true (`mod.rs:404-417`).
- `run_explicit_sync`: no `notify_mutation` — it *is* the sync
  (plus pending-clear). SyncMerge origin never re-triggers by policy.

## Error / exit mapping

Helpers return `SnipResult`; exit mapping lives in `outcome.rs` via the
caller: `SelectionOutcome::Cancelled → Success` (for `run`) or
`Cancelled` (exit 4, for `select`); `ExecutionFailed{exit_code} →
CliOutcome::ExecutionFailed` (exit 8 or the child code). Missing runtime
with `do_sync` is a `runtime_error("sync requested but no runtime")`.

## Key invariants

- `run_snippet_selection` takes `Option<&Runtime>` — `None` whenever
  `do_sync` is false. Local-only commands never init the global Tokio
  runtime.
- The retry/explicit-sync path never uses a closure-based generic retry
  helper (borrow rules); `sync.rs` keeps the inline `retry_grpc_unified!`
  macro for the same reason.
- Tombstones are preserved on delete; `updated_at` strictly increases.
- Save path never post-processes `toml::to_string_pretty`.
- `Drop` releases kernel locks without unlinking; lock files may hold
  stale metadata.

## File / line references

- Module decls: `src/commands/mod.rs:8-32`
- `ExpandedCommand`: `:42`; `get_config_path`: `:52`
- `get_library_path`: `:91`; `init_library_manager`: `:118`
- `load_snippets`: `:125`; `save_snippets`: `:175` (gate `:180`, lock `:186`)
- `get_snippet_data`: `:211`; `expand_snippet_command`: `:242`
- `run_explicit_sync`: `:269`; `run_snippet_selection`: `:306`
- `mark_snippet_deleted`: `:467`; unit tests: `:484-609`

# library_cmd — Library Management Subcommands

[← Back to Overview](../overview.md)

## Overview

`src/commands/library_cmd.rs` (155 lines) manages snippet libraries
via `LibraryManager`: list, create, delete (confirm-guarded), set
primary, and show metadata. Creation/deletion are synced mutations;
listing/primary/show are local metadata operations.

## CLI surface

`snp library` (alias `lib`), `main.rs:187-207,729-737`:

| Form | Handler | Notes |
|------|---------|-------|
| `library list` (alias `l`) | `run_list()` (`:6`) | Prints `filename [+ (primary)]` |
| `library create <name>` (alias `c`) | `run_create(name)` (`:24`) | Validates + writes new `.toml` |
| `library delete <name> [--force]` (alias `d`) | `run_delete` (`:39`) | Confirm unless `--force`; non-tty requires `--force` |
| `library set-primary <name>` (alias `p`) | `run_set_primary` (`:74`) | Local pointer swap |
| `library show [name]` (alias `s`) | `run_show` (`:82`) | One library detail or all with `[linked]` |

## Flow / steps

- `run_list`: `LibraryManager::new → list_libraries`; empty →
  `No libraries found.`; else `Libraries:` + entries.
- `run_create`: `create_library(name)` (sanitizes, rejects
  duplicates/reserved, writes file + index) → notify →
  `Created library '<name>' at <path>`.
- `run_delete`: without `force`, non-terminal stdin → hard error
  (`Non-interactive delete … Use --force`); terminal → `[y/N]`
  confirm, anything but `y` → `Cancelled.` + `Ok`. Then
  `delete_library(name)` → notify → `Deleted library '<name>'`.
- `run_set_primary`: `set_primary(name)` → confirmation line. No
  notify (pointer-only change).
- `run_show`: named → filename, ID (`{not linked}` when empty),
  primary flag, last-sync timestamp (`%Y-%m-%d %H:%M`); unnamed →
  all libraries with `(primary)`/`[linked]` markers; unknown name →
  `runtime_error("Library not found")`.

`StringExt::if_empty` (`:120-132`) backs the `{not linked}` fallback.

## Mutation vs read-only

Mixed: `create`/`delete` mutate (index + files, gated inside
`LibraryManager::gate_mutation`); `list`/`set-primary`/`show` are
metadata-level (set-primary writes `libraries.toml` only, no snippet
content — hence no gate/notify implications beyond the index write).

## Auto-sync trigger

- `run_create` / `run_delete`: `notify_mutation(
  MutationKind::LibraryChange, MutationOrigin::User)` after success
  (`:29`, `:64`; Workstream B5).
- `run_set_primary` / `run_list` / `run_show`: none — primary
  pointers and reads create no sync intent.

## Error / exit mapping

`SnipResult<()>`: unknown/duplicate/invalid names, non-interactive
delete without `--force`, and manager I/O errors propagate (exit 1;
not-found maps toward exit 3 via `library_not_found` wording where
applicable). Cancellation (`n` at the prompt) is exit-0 `Ok`.

## Key invariants

- All libraries live under `~/.config/snp/libraries/`; external
  paths unsupported. Malformed `libraries.toml` fails closed
  (backup + error, never synthetic-empty).
- Missing-library recovery uses atomic `<library>.sync_recovery`
  state: one normalized remote-name match only, ambiguity fails.
- Deletion is refused non-interactively without `--force` — scripts
  must opt in explicitly.
- `last_sync` linkage resets atomically with relink + retry sync.

## File / line references

- Handlers: `src/commands/library_cmd.rs:6,24,39,74,82`
- Notifies: `:29,64`; guard: `:43-59`; manager: `src/library/manager.rs`
- Dispatch: `src/main.rs:187-207,729-737`

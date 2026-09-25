# search_cmd — Fuzzy Search with Detail Display

[← Back to Overview](../overview.md)

## Overview

`src/commands/search_cmd.rs` (61 lines) is the thinnest TUI command: it
opens the shared snippet selector and prints the chosen snippet's full
detail instead of executing or copying. Selection, sorting, filtering,
and `--sync` reuse `run_snippet_selection` verbatim.

## CLI surface

`SearchArgs` (`search_cmd.rs:7`), `snp search` (alias `s`):
`-f/--filter` (initial TUI filter), `--sync`, `-l/--library`,
`--sort` (default `Relevance`), `--favorites-first`. No exact-selector
flags (unlike `run`/`clip`/`get`/`edit`); no output-format flags — the
detail block is fixed. `main.rs:585-597` passes `config: None`
(the `--config` derivation below lives inside `run`).

## Flow / steps

`run()` (`search_cmd.rs:23`):

1. Derive `effective_library`: explicit `--library`, else the file stem
   of `--config` (with an explanatory `note:` to stderr so a stem that
   differs from a registered library name is not confusing).
2. `run_snippet_selection(filter, effective_library, do_sync,
   allow_delete=true, sort_opts, runtime, print_fn)`.
3. `print_fn` prints six lines — Description, Command, Output, Tags,
   Folders, Favorite — and returns `Done("")`. Cancel/skip semantics
   come from the shared loop untouched.

The TUI delete shortcut (`allow_delete=true`) is available, identical
to `run`/`clip`.

## Mutation vs read-only

Display-only: no `save_library`, no transaction gate, no output files.
The only write on this path is the shared-loop delete shortcut
(tombstone + save + audit), which is a property of the selector, not
of search itself.

## Auto-sync trigger

None of its own. `do_sync` flows into `run_snippet_selection`, which
performs the trailing `run_explicit_sync` after a selection (and
`notify_mutation(SnippetDelete, User)` after a delete when `do_sync`
is false). Search never notifies by itself.

## Error / exit mapping

`SnipResult<()>`; errors propagate (exit 1). Cancellation is silent
success via the shared loop. `--sync` without a runtime errors inside
`run_snippet_selection` before the TUI opens.

## Key invariants

- No search-specific selection logic: filter, fuzzy matching
  (`SkimMatcherV2`), navigation, and variable prompting all live in
  `ui` + `commands::mod`.
- `--config` is translated to a library *name* (stem), never passed as
  a file path into the selector pipeline.
- Keep the detail-print closure side-effect-free apart from stdout;
  execution stays in `run_cmd`, clipboard in `clip_cmd`.

## The delete shortcut

Because `allow_delete=true`, the search TUI offers the same `d`
(delete with confirm) flow as `run`/`clip`: the shared loop marks the
tombstone, saves, audits, and notifies `SnippetDelete/User` (or runs
the explicit sync when `--sync`). A search session can therefore
mutate despite search itself being display-only — review UIs that wrap
`search` should account for this (contrast `select`, which passes
`allow_delete=false`).

## Testing

No unit tests in `search_cmd.rs` (the module is a thin closure over
the shared loop). Coverage comes from TUI integration tests driving
`--filter` selection and cancellation, plus the shared-loop tests in
`commands::mod`. When changing the `--config`-stem derivation, add a
case asserting the stderr `note:` names the derived library.

## File / line references

- `SearchArgs`: `src/commands/search_cmd.rs:7`; `run`: `:23`
- Shared loop: `src/commands/mod.rs:306`
- Dispatch: `src/main.rs:585-597`

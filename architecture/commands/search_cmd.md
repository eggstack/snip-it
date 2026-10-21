# search_cmd — Fuzzy Search with Detail Display

[← Back to Overview](../overview.md)

## Overview

`src/commands/search_cmd.rs` (76 lines) is the thinnest TUI command: it
opens the shared snippet selector and prints the chosen snippet's full
detail instead of executing or copying. Selection, sorting, filtering,
and `--sync` reuse `run_snippet_selection` verbatim.

`search` is display-only — but the shared selector it drives can still
mutate (see *The delete shortcut* below).

## CLI surface

`SearchArgs` (`search_cmd.rs:7`), `snp search` (alias `s`):

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `-f/--filter` | `Option<String>` | – | Initial TUI filter |
| `--sync` | bool | `false` | Explicit sync after the session |
| `-l/--library` | `Option<String>` | – | Library scope; `all` is **rejected** (see below) |
| `--sort` | `SnippetSort` | `Relevance` | Sort mode for snippet ordering |
| `--favorites-first` | bool | `false` | Favorites rank first |

No exact-selector flags (unlike `run`/`clip`/`get`/`edit`); no
output-format flags — the detail block is fixed. `src/main.rs:612-625`
passes `config: None` (`src/main.rs:621`); the `--config` derivation below
lives inside `run`.

## Flow / steps

`run()` (`search_cmd.rs:24`):

1. **Reject `--library all`** (`search_cmd.rs:37-44`) — see
   *Library scope* below.
2. Derive `effective_library`: explicit `--library`, else the file stem
   of `--config` (with an explanatory `note:` to stderr so a stem that
   differs from a registered library name is not confusing,
   `search_cmd.rs:50-57`).
3. `run_snippet_selection(filter, effective_library, do_sync,
   allow_delete=true, sort_opts, runtime, print_fn)`
   (`search_cmd.rs:58-74`).
4. `print_fn` prints six lines — Description, Command, Output, Tags,
   Folders, Favorite (`search_cmd.rs:66-71`) — and returns `Done("")`.
   Cancel/skip semantics come from the shared loop untouched.

The TUI delete shortcut (`allow_delete=true`) is available, identical
to `run`/`clip`.

## Library scope

`search` **rejects** `--library all` with an accurate error
(`search_cmd.rs:37-44`):

```
Library 'all' is not supported by 'snp search'
The interactive selector operates on one library. Use 'snp list --library all'
to list every library, or 'snp get --library all' to fetch a snippet by id.
```

The reason is structural: the interactive selector edits and deletes
within a **single library file**, so it cannot take a cross-library scope.

Before the fix, `--library all` bypassed `LibraryScope::from_filter_arg`,
so the lookup failed with the misleading `Library 'all' does not exist` —
false, because `all` is the scope keyword owned by
`LibraryScope::from_filter_arg` (`src/selector.rs:65`) and it *does* work
for `snp get`, `snp list`, and MCP. The guard now says what is actually
true.

For every other value, `search` passes the name straight into
`run_snippet_selection`, which resolves it through the **mutating**
`get_library_path` (`src/commands/mod.rs:467`, `src/commands/mod.rs:91`).

## Mutation vs read-only

Display-only: no `save_library`, no transaction gate, no output files.
`search` never calls `gate_mutation_on_interrupted_transactions()`.

The only write on this path is the shared-loop delete shortcut
(tombstone + save + audit), which is a property of the selector, not
of search itself.

## Auto-sync trigger

None of its own. `do_sync` flows into `run_snippet_selection`, which
performs the trailing `run_explicit_sync` after a selection (and
`notify_mutation(SnippetDelete, User)` after a delete when `do_sync`
is false, `src/commands/mod.rs:547-551`). Search never notifies by
itself.

`src/main.rs:992` classifies `Commands::Search(_)` as a mutation command
(`StartupRecoveryPolicy::Allow` + `StartupServices::Logging`), because the
delete shortcut can mutate.

## Tokio runtime

`search` is one of the commands that may initialize the global runtime —
but only for `--sync`. `src/main.rs:623` passes
`args.sync.then_some(&RUNTIME)`, so the runtime is `Some` only when
`--sync` is set and `None` otherwise. `run_snippet_selection`
(`src/commands/mod.rs:456-465`) errors with
`runtime_error("sync requested but no runtime")` if `do_sync` is true and
the runtime is `None`.

## Error / exit mapping

`SnipResult<()>`; errors propagate (exit 1). Cancellation is silent
success via the shared loop. `--sync` without a runtime errors inside
`run_snippet_selection` before the TUI opens. The `--library all`
rejection is a `runtime_error` (exit 1 family).

## Key invariants

- No search-specific selection logic: filter, fuzzy matching
  (`SkimMatcherV2`), navigation, and variable prompting all live in
  `ui` + `commands::mod`.
- `--config` is translated to a library *name* (stem), never passed as
  a file path into the selector pipeline.
- Keep the detail-print closure side-effect-free apart from stdout;
  execution stays in `run_cmd`, clipboard in `clip_cmd`.
- The `--library all` guard must run **before** the shared loop, so the
  user gets the accurate message instead of `Library 'all' does not exist`.

## The delete shortcut

Because `allow_delete=true`, the search TUI offers the same `d`
(delete with confirm) flow as `run`/`clip`: the shared loop marks the
tombstone, saves, audits, and notifies `SnippetDelete/User` (or runs
the explicit sync when `--sync`). A search session can therefore
mutate despite search itself being display-only — review UIs that wrap
`search` should account for this (contrast `select`, which passes
`allow_delete=false`).

## Gotchas

- `--library all` is a hard error, not a silent fallback to the primary
  library.
- The stderr `note:` from the `--config` stem derivation is advisory only;
  the derived stem must match a registered library name or the shared loop
  reports a library-not-found error.
- `search` requires a tty. `ui::select_snippet` returns an
  `io::ErrorKind::Unsupported` error when stdin or stdout is not a
  terminal (`src/ui/mod.rs:648-655`).

## Testing

No unit tests in `search_cmd.rs` (the module is a thin closure over
the shared loop). Coverage comes from TUI integration tests driving
`--filter` selection and cancellation, plus the shared-loop tests in
`commands::mod`. When changing the `--config`-stem derivation, add a
case asserting the stderr `note:` names the derived library; when
changing the `all` guard, assert the error text names both supported
alternatives.

## Key Files

- `src/commands/search_cmd.rs` — the command: args, the `all` guard, the
  `--config` stem derivation, and the detail-print closure.
- `src/commands/mod.rs` — `run_snippet_selection`, the shared TUI loop.
- `src/selector.rs` — `LibraryScope::from_filter_arg`, the owner of the
  `all` keyword.
- `src/ui/mod.rs` — `select_snippet` and the tty guard.
- `src/main.rs` — dispatch (`src/main.rs:612-625`).

## File / line references

- `SearchArgs`: `src/commands/search_cmd.rs:7`; `run`: `:24`
- `all` guard: `:37-44`; `--config` derivation: `:50-57`
- Shared loop: `src/commands/mod.rs:399`
- Dispatch: `src/main.rs:612-625`

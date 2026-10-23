# list_cmd — Text-Based Snippet Listing

[← Back to Overview](../overview.md)

## Overview

`src/commands/list_cmd.rs` (265 lines) lists snippets as human text,
JSON, or CSV. Filtering runs through the canonical
`selector::score_fuzzy_matches` primitive over the `selector::searchable_text`
field contract — the same match primitive `snp get --query` and MCP
`snippets_search` use; ordering goes through `sort::rank_snippets` with
optional usage data. Never executes, edits, or syncs.

This command is **strictly read-only**: it resolves its library scope
through the canonical read-only loader and never migrates a legacy
single-file checkout.

## CLI surface

`ListArgs` (`list_cmd.rs:9`), `snp list` (alias `l`):

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `-f/--filter` | `Option<String>` | – | Fuzzy filter string |
| `-c/--config PATH` | `Option<PathBuf>` | – | Legacy single-file source (ignores `--library` with a warning) |
| `-l/--library` | `Option<String>` | – | Library scope; `all` lists every library |
| `--json` | bool | `false` | JSON array to stdout (conflicts with `--csv`) |
| `--csv` | bool | `false` | CSV to stdout (conflicts with `--json`) |
| `--search-output` | bool | `false` | Include output/notes field in matching |
| `--sort` | `SnippetSort` | `Relevance` | Sort mode for snippet ordering |
| `--favorites-first` | bool | `false` | Favorites rank first |

`src/main.rs:517-537` maps the json/csv flags to
`ListFormat::{Json,Csv,Default}` and builds
`SortOptions{mode, favorites_first}` (always `Some`).

## Flow / steps

`run()` (`list_cmd.rs:51`):

1. **Warn on flag conflict** — `--config` together with `--library` prints
   `warning: --library is ignored when --config is set` to stderr
   (`list_cmd.rs:57-59`). Not an error.
2. **Load** (`list_cmd.rs:60-87`):
   - `--config` given → `load_snippets_readonly_at(config)`
     (legacy single-file path, `list_cmd.rs:65`).
   - Otherwise → `crate::commands::load_readonly_snippets(library.as_deref())`
     (`list_cmd.rs:73`). If the resolved sources are empty, print
     `No library found. Create one with 'snp library create <name>'` and
     return `Ok(())` (`list_cmd.rs:74-85`). In the machine-readable formats
     an empty document is emitted as well (`[]` for JSON, the header row for
     CSV): an empty stdout is indistinguishable from a truncated stream, so a
     consumer must always be able to parse what it receives.
3. **Filter** (`list_cmd.rs:106-119`): with a filter, call the shared match
   primitive `selector::score_fuzzy_matches` (`list_cmd.rs:107-108`) over
   `SearchFields::with_output()` / `fuzzy_default()`
   (`list_cmd.rs:92-96`); without a filter, every non-deleted snippet is
   collected directly (`list_cmd.rs:111-119`). Deleted tombstones are
   excluded by the primitive in both cases (`src/selector.rs:221`,
   `src/selector.rs:228-230`).
4. **Sort** (`list_cmd.rs:125-156`): collect indices, load `UsageIndex`
   **only** for `LastUsed`/`MostUsed` (`list_cmd.rs:130-142`),
   `rank_snippets(indices, snippets, scores, usage, opts)` (`list_cmd.rs:143`),
   then reorder by rank map (stable, total order, `list_cmd.rs:150-155`).
5. **Render** (`list_cmd.rs:158-223`): JSON (`description/command/output/tags/folders/favorite`
   per item, pretty, `list_cmd.rs:159-182`); CSV (header + `csv_escape`,
   `list_cmd.rs:183-198`); Default (colored `-----` separators,
   description: command, `Output:` 80-char summary via `OutputPresentation`,
   `Tags:`, `list_cmd.rs:199-222`).

`csv_escape` (`list_cmd.rs:227`): prefixes `= + - @`-leading fields with `\t`
(formula-injection defense), then quotes fields containing
`, " \n \r \t` with `""` doubling.

## Filter semantics

`list --filter` routes through `selector::score_fuzzy_matches`
(`src/selector.rs:209`) — the same match primitive `snp get --query` and MCP
`snippets_search` use — so the three surfaces cannot drift on *which*
snippets a query selects.

- **An empty filter is "no filter."** `snp list --filter ""` returns exactly
  what `snp list` with no `--filter` returns: every non-deleted snippet, with
  no scores — *including* snippets whose description, command, and tags are
  all empty. **This is a visible behavior change.** Before the shared
  primitive was adopted, an empty filter dropped those snippets, because the
  old inline filter skipped any snippet with an empty searchable text before
  matching. A non-empty filter matches the `searchable_text` projection only,
  so a snippet with an empty projection still never matches a non-empty
  filter.
- **Cross-library order is global relevance over the merged set.** With
  `--library all --filter`, the libraries are merged first and the merged set
  is scored and ranked by `sort::rank_snippets` under `SnippetSort::Relevance`
  (`list_cmd.rs:143-149`). `snp get --query --library all` uses the other
  order family (library → description → snippet ID) by design; within a
  **single** library the two orders coincide.

## Read-only guarantee

`list` resolves through `commands::load_readonly_snippets`
(`list_cmd.rs:73`), which calls `library::readonly_library_sources`
(`src/commands/mod.rs:176`). That resolver performs no migration and
creates no files or directories.

**This is a behavior change.** Before the fix, `list` resolved through
`get_library_path` (`src/commands/mod.rs:92`), which calls the *mutating*
`ensure_library_mode()`. On a legacy single-file checkout a plain
`snp list` created `libraries/`, copied the legacy file (leaving the
original behind as a second, now-inert copy), wrote `libraries.toml`, and
bumped the generation — a migration caused purely by a read.

## Library scope

`--library all` is supported by `list`. The literal `"all"` is matched by
`LibraryManager::resolve_readonly_sources` (`src/library/manager.rs:295-336`),
which `load_readonly_snippets` reaches via
`library::readonly_library_sources` (`src/commands/mod.rs:176`), so it means
the same thing here as on `snp get`, exact-mode `run`/`clip`/`edit`, and MCP.
(The *selector* paths — `snp get`, exact-mode `run`/`clip`/`edit` — own the
keyword in `LibraryScope::from_filter_arg`, `src/selector.rs:65`; both layers
agree that `"all"` means "every visible library".)

Scope semantics of `readonly_library_sources`
(`src/library/manager.rs:879`):

| `--library` | Result |
|-------------|--------|
| absent | the primary library — or, in legacy single-file mode, the implicit `snippets` library |
| `all` | every visible library (snippets concatenated in resolution order) |
| a name | that named library, or `library_not_found(name)` |

A library actually *named* `all` can no longer be created:
`validate_library_name` rejects the reserved name
(`src/library/model.rs:247-252`).

## Mutation vs read-only

Read-only. No gate, no lock, no save, no clipboard, no runtime. The
command does not call `gate_mutation_on_interrupted_transactions()`.

`src/main.rs:941-955` classifies `Commands::List(_)` (`:946`) under
`StartupRecoveryPolicy::SuppressReadOnly` + `StartupServices::Minimal`, so
startup recovery is suppressed and no network service is started.

## Auto-sync trigger

None. No `notify_mutation`, no explicit sync. Listing is invisible to
auto-sync (usage is not even recorded here — only `run`/`clip` record).

## Error / exit mapping

`SnipResult<()>`; load/serialize errors propagate (exit 1). Empty
library or empty filter result prints nothing and still succeeds.
`--library ignored with --config` is a stderr warning, not an error.
JSON serialization failure is a `runtime_error` (`list_cmd.rs:175-180`).

## Key invariants

- Search parity with `get --query` and MCP `snippets_search` via
  `searchable_text` **and** the shared match primitive
  `selector::score_fuzzy_matches`; output/notes participates only with
  `--search-output` (bounded to 512 **bytes** for scoring — `OUTPUT_SEARCH_BUDGET`,
  `src/output.rs:11`; the bound is on `str::len()`, with a char-boundary backup).
- **An empty `--filter` is "no filter"**, not "match nothing": it returns
  exactly what `snp list` with no filter returns, including snippets with an
  empty description, command, and tags.
- Deleted snippets never list, in any format.
- Sort skips the usage-file read unless the mode needs it.
- CSV formula protection applies before quoting; JSON carries no IDs
  or sync metadata by design of this view.
- The read-only path must not be swapped back to `get_library_path`:
  `commands/mod.rs` and `ui/mod.rs` re-exports affect all TUI commands.

## Gotchas

- `--config` silently wins over `--library` (warning only). A caller that
  passes both gets the legacy file, not the named library.
- `--library all` concatenates libraries without a per-library separator in
  the JSON/CSV output, so snippets from different libraries are
  indistinguishable in the payload.
- `list --filter` is a fuzzy match over description + command + tags, not
  a substring match — unlike `snp edit --filter`, which is a
  case-insensitive substring match (`src/commands/edit_cmd.rs:179-180`).
- `--filter ""` used to drop snippets whose description, command, and tags
  were all empty. It now returns every non-deleted snippet, exactly like
  `snp list` with no filter.
- `--library all --filter` and `snp get --query --library all` order their
  results differently **by design** — global relevance over the merged set
  versus library → description → snippet ID. They coincide only within a
  single library.

## Key Files

- `src/commands/list_cmd.rs` — the command: args, filter, sort, and the
  three renderers.
- `src/commands/mod.rs` — `load_readonly_snippets`, the read-only loader
  shared with `snp get` and MCP.
- `src/selector.rs` — `SearchFields`, `searchable_text`, and
  `score_fuzzy_matches`, the shared search contract.
- `src/sort.rs` — `rank_snippets` and `SnippetSort` ordering modes.
- `src/library/manager.rs` — `readonly_library_sources` and
  `resolve_readonly_sources`, the scope resolver.
- `src/main.rs` — dispatch (`src/main.rs:517-537`) and the read-only
  startup classification (`src/main.rs:941-955`).

## File / line references

- `ListArgs`: `src/commands/list_cmd.rs:7`; `ListFormat`: `:34`
- `run`: `:49`; read-only load: `:73`; filter block: `:106-119`; sort block: `:125`
- renderers: `:158-223`; `csv_escape`: `:227`
- dispatch: `src/main.rs:517-537`

# list_cmd — Text-Based Snippet Listing

[← Back to Overview](../overview.md)

## Overview

`src/commands/list_cmd.rs` (252 lines) lists snippets as human text,
JSON, or CSV. Filtering is fuzzy over the canonical
`selector::searchable_text` contract; ordering goes through
`sort::rank_snippets` with optional usage data. Never executes,
edits, or syncs.

## CLI surface

`ListArgs` (`list_cmd.rs:9`), `snp list` (alias `l`):

| Flag | Meaning |
|------|---------|
| `-f/--filter` | Fuzzy filter string |
| `-c/--config PATH` | Legacy single-file source (ignores `--library` with a warning) |
| `-l/--library` | Library scope |
| `--json` | JSON array to stdout (conflicts with `--csv`) |
| `--csv` | CSV to stdout (conflicts with `--json`) |
| `--search-output` | Include output/notes field in matching (default off) |
| `--sort <mode>` | `SnippetSort`, default `Relevance` |
| `--favorites-first` | Favorites rank first |

`main.rs:479-498` maps json/csv flags to `ListFormat::{Json,Csv,Default}`
and builds `SortOptions{mode, favorites_first}` (always `Some`).

## Flow / steps

`run()` (`list_cmd.rs:50`):

1. Load: `--config` → `load_snippets`; else `get_library_path` →
   `load_library` (no library → hint + `Ok(())`).
2. Filter: deleted tombstones excluded; with a filter, score
   `searchable_text(s, SearchFields{include_tags:true,
   include_output:search_output})` via `shared_fuzzy_matcher()`,
   keeping matches + scores (empty searchable text never matches).
3. Sort (`:113-143`): collect indices, load `UsageIndex` **only** for
   `LastUsed`/`MostUsed`, `rank_snippets(indices, snippets, scores,
   usage, opts)`, then reorder by rank map (stable, total order).
4. Render: JSON (`description/command/output/tags/folders/favorite`
   per item, pretty); CSV (header + `csv_escape`); Default (colored
   `-----` separators, description: command, `Output:` 80-char summary
   via `OutputPresentation`, `Tags:`).

`csv_escape` (`:214`): prefixes `= + - @`-leading fields with `\t`
(formula-injection defense), then quotes fields containing
`, " \n \r \t` with `""` doubling.

## Mutation vs read-only

Read-only. No gate, no lock, no save, no clipboard, no runtime.

## Auto-sync trigger

None. No `notify_mutation`, no explicit sync. Listing is invisible to
auto-sync (usage is not even recorded here — only run/clip record).

## Error / exit mapping

`SnipResult<()>`; load/serialize errors propagate (exit 1). Empty
library or empty filter result prints nothing and still succeeds.
`--library ignored with --config` is a stderr warning, not an error.

## Key invariants

- Search parity with `get --query` and MCP `snippets_search` via
  `searchable_text`; output/notes participates only with
  `--search-output` (bounded to 512 chars for scoring).
- Deleted snippets never list, in any format.
- Sort skips the usage-file read unless the mode needs it.
- CSV formula protection applies before quoting; JSON carries no IDs
  or sync metadata by design of this view.

## File / line references

- `ListArgs`: `src/commands/list_cmd.rs:9`; `ListFormat`: `:35`
- `run`: `:50`; sort block: `:113`; renderers: `:145-212`
- `csv_escape`: `:214`; dispatch: `src/main.rs:479-498`

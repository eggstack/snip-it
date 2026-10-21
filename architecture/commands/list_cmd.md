# list_cmd — Text-Based Snippet Listing

[← Back to Overview](../overview.md)

## Overview

`src/commands/list_cmd.rs` (257 lines) lists snippets as human text,
JSON, or CSV. Filtering is fuzzy over the canonical
`selector::searchable_text` contract; ordering goes through
`sort::rank_snippets` with optional usage data. Never executes,
edits, or syncs.

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

`src/main.rs:479-499` maps the json/csv flags to
`ListFormat::{Json,Csv,Default}` and builds
`SortOptions{mode, favorites_first}` (always `Some`).

## Flow / steps

`run()` (`list_cmd.rs:51`):

1. **Warn on flag conflict** — `--config` together with `--library` prints
   `warning: --library is ignored when --config is set` to stderr
   (`list_cmd.rs:59-61`). Not an error.
2. **Load** (`list_cmd.rs:62-77`):
   - `--config` given → `load_snippets(&config)` (legacy single-file path).
   - Otherwise → `crate::commands::load_readonly_snippets(library.as_deref())`
     (`list_cmd.rs:71`). If the resolved sources are empty, print
     `No library found. Create one with 'snp library create <name>'` and
     return `Ok(())` (`list_cmd.rs:72-75`).
3. **Filter** (`list_cmd.rs:85-115`): deleted tombstones excluded
   (`list_cmd.rs:91`, `list_cmd.rs:113`); with a filter, score
   `searchable_text(s, SearchFields{include_tags:true,
   include_output:search_output})` via `shared_fuzzy_matcher()`
   (`list_cmd.rs:79-83`, `list_cmd.rs:93`). A snippet whose searchable text
   is empty can never match a non-empty filter (`list_cmd.rs:94-99`).
4. **Sort** (`list_cmd.rs:118-148`): collect indices, load `UsageIndex`
   **only** for `LastUsed`/`MostUsed` (`list_cmd.rs:122-134`),
   `rank_snippets(indices, snippets, scores, usage, opts)`, then reorder by
   rank map (stable, total order, `list_cmd.rs:142-147`).
5. **Render** (`list_cmd.rs:150-215`): JSON (`description/command/output/tags/folders/favorite`
   per item, pretty, `list_cmd.rs:151-174`); CSV (header + `csv_escape`,
   `list_cmd.rs:175-190`); Default (colored `-----` separators,
   description: command, `Output:` 80-char summary via `OutputPresentation`,
   `Tags:`, `list_cmd.rs:191-214`).

`csv_escape` (`list_cmd.rs:219`): prefixes `= + - @`-leading fields with `\t`
(formula-injection defense), then quotes fields containing
`, " \n \r \t` with `""` doubling.

## Read-only guarantee

`list` resolves through `commands::load_readonly_snippets`
(`list_cmd.rs:71`), which calls `library::readonly_library_sources`
(`src/commands/mod.rs:133`). That resolver performs no migration and
creates no files or directories.

**This is a behavior change.** Before the fix, `list` resolved through
`get_library_path` (`src/commands/mod.rs:91`), which calls the *mutating*
`ensure_library_mode()`. On a legacy single-file checkout a plain
`snp list` created `libraries/`, copied the legacy file (leaving the
original behind as a second, now-inert copy), wrote `libraries.toml`, and
bumped the generation — a migration caused purely by a read.

## Library scope

`--library all` is supported by `list`. The literal `"all"` is routed
through `LibraryScope::from_filter_arg` (`src/selector.rs:65`) inside
`load_readonly_snippets`, so it means the same thing here as on `snp get`,
exact-mode `run`/`clip`/`edit`, and MCP.

Scope semantics of `readonly_library_sources`
(`src/library/manager.rs:773`):

| `--library` | Result |
|-------------|--------|
| absent | the primary library — or, in legacy single-file mode, the implicit `snippets` library |
| `all` | every visible library (snippets concatenated in resolution order) |
| a name | that named library, or `library_not_found(name)` |

A library actually *named* `all` can no longer be created:
`validate_library_name` rejects the reserved name
(`src/library/model.rs:213-223`).

## Mutation vs read-only

Read-only. No gate, no lock, no save, no clipboard, no runtime. The
command does not call `gate_mutation_on_interrupted_transactions()`.

`src/main.rs:891` classifies `Commands::List(_)` under
`StartupRecoveryPolicy::SuppressReadOnly` + `StartupServices::Minimal`, so
startup recovery is suppressed and no network service is started.

## Auto-sync trigger

None. No `notify_mutation`, no explicit sync. Listing is invisible to
auto-sync (usage is not even recorded here — only `run`/`clip` record).

## Error / exit mapping

`SnipResult<()>`; load/serialize errors propagate (exit 1). Empty
library or empty filter result prints nothing and still succeeds.
`--library ignored with --config` is a stderr warning, not an error.
JSON serialization failure is a `runtime_error` (`list_cmd.rs:167-172`).

## Key invariants

- Search parity with `get --query` and MCP `snippets_search` via
  `searchable_text`; output/notes participates only with
  `--search-output` (bounded to 512 **bytes** for scoring — `OUTPUT_SEARCH_BUDGET`,
  `src/output.rs:11`; the bound is on `str::len()`, with a char-boundary backup).
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
  case-insensitive substring match (`src/commands/edit_cmd.rs:173-174`).

## Key Files

- `src/commands/list_cmd.rs` — the command: args, filter, sort, and the
  three renderers.
- `src/commands/mod.rs` — `load_readonly_snippets`, the read-only loader
  shared with `snp get` and MCP.
- `src/selector.rs` — `SearchFields` and `searchable_text`, the shared
  search contract.
- `src/sort.rs` — `rank_snippets` and `SnippetSort` ordering modes.
- `src/library/manager.rs` — `readonly_library_sources` and
  `resolve_readonly_sources`, the scope resolver.
- `src/main.rs` — dispatch (`src/main.rs:479-499`) and the read-only
  startup classification (`src/main.rs:891`).

## File / line references

- `ListArgs`: `src/commands/list_cmd.rs:9`; `ListFormat`: `:36`
- `run`: `:51`; read-only load: `:71`; sort block: `:118`
- renderers: `:150-215`; `csv_escape`: `:219`
- dispatch: `src/main.rs:479-499`

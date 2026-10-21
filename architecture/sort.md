# Sort and Ranking Module

[← Back to Overview](overview.md)

## Overview

`src/sort.rs` (1334 LOC, 47 unit tests) is the shared, deterministic ranking
model for snippets. One function — `rank_snippets()` — orders a slice of
snippet indices by a chosen mode, with an orthogonal favorites-first grouping
and a fixed tie-break chain. It serves the CLI (`--sort`, `--favorites-first`),
the selector's fuzzy path, and MCP search.

**What this document does not cover.** The interactive TUI has a **separate**
inline comparator over its own `ui::state::SortMode` (`src/ui/mod.rs:173`),
which mirrors the ordering rules but is not `rank_snippets` — see
[The TUI comparator](#the-tui-comparator). The library data model is in
[library.md](library.md); the search-field contract is in
[selector.md](selector.md); usage *recording* (as opposed to usage
*ordering*) lives in `src/usage.rs`.

---

## Key types — `src/sort.rs:37-92`

```rust
pub enum SnippetSort {
    #[default]
    Relevance,   // fuzzy score, descending
    Recent,      // updated_at desc, then created_at desc
    LastUsed,    // last_used_at desc
    MostUsed,    // use_count desc, then last_used_at desc
    Description, // lowercased description, ascending
    Command,     // lowercased command, ascending
}

pub struct SortOptions {
    pub mode: SnippetSort,
    pub favorites_first: bool,
}

struct RankedSnippet {
    index: usize,
    desc_lower: Option<String>,  // computed unless mode == Relevance
    cmd_lower: Option<String>,   // computed only for Command mode
    favorite: bool,
    updated_at: i64,
    created_at: i64,
    use_count: u64,
    last_used_at: Option<i64>,
    fuzzy_score: Option<i64>,
}
```

`SnippetSort` is `clap::ValueEnum` (kebab-case), `#[non_exhaustive]`
(`src/sort.rs:36`), `Copy`, and `Default = Relevance`.

### What each mode orders by

| Mode | Primary key | Secondary | Direction | Notes |
|---|---|---|---|---|
| `Relevance` (default) | fuzzy score (`SkimMatcherV2`) | — | **descending** | `None` score sorts **after** every `Some`; equal scores fall through to index order |
| `Recent` | `updated_at` | `created_at` | **descending** | updated wins; `created_at` only breaks an `updated_at` tie |
| `LastUsed` | `last_used_at` | — | **descending** | `None` (never used) sorts **after** every `Some` |
| `MostUsed` | `use_count` | `last_used_at` | **descending** | `last_used_at` only breaks a `use_count` tie; `None` last-used sorts after `Some` |
| `Description` | lowercased `description` | — | **ascending** | ASCII lowercase, so `A`–`Z` |
| `Command` | lowercased `command` | — | **ascending** | ASCII lowercase |

Implementation: `src/sort.rs:157-194`.

`desc_lower` is skipped entirely under `Relevance` and `cmd_lower` is
computed only for `Command` (`src/sort.rs:134`), so timestamp and usage
orderings pay no string-allocation cost.

---

## Key function — `src/sort.rs:113-228`

```rust
pub fn rank_snippets(
    indices: &[usize],                          // subset to order; need not be contiguous
    snippets: &[Snippet],
    fuzzy_scores: Option<&HashMap<usize, i64>>, // index → Skim score
    usage: Option<&[UsageData]>,                // parallel slice; missing → zeroed
    opts: &SortOptions,
) -> Vec<usize>                                 // sorted indices; input is never mutated
```

`RankedSnippet` is a projection of `(index, snippet, score, usage)` — the
original `snippets` slice is never reordered or mutated.

### The tie-break chain

The comparator (`src/sort.rs:147-225`) applies, in this order:

| Level | Key | Direction | Source |
|---|---|---|---|
| 1 | `favorite` | `true` before `false` | `:150-155` |
| 2 | primary key (the `SnippetSort` variant) | per mode, see the table above | `:157-194` |
| 3 | fuzzy score | descending; `Some` before `None` | `:198-208` — **skipped when the primary mode is `Relevance`** |
| 4 | lowercased description | ascending | `:212-217` — **skipped when the primary mode is `Relevance`** |
| 5 | original index | ascending | `:220-224` |

Two properties fall out of this shape:

- **Favorites-first is orthogonal.** It is compared *before* the primary key
  (`:150-155`, with `.reverse()` so `true` wins), so favorites form one
  contiguous block and the selected mode still orders *within* each block.
- **`Relevance` short-circuits.** Levels 3 and 4 are explicitly skipped for
  `Relevance` (`:198`, `:212`), so the `Relevance` order is exactly
  *fuzzy score → original index*. That is why `Relevance` with no scores
  degenerates to plain file order.

### Stability

`rank_snippets` uses `sort_by` (Rust's **stable** sort,
`src/sort.rs:146`) **and** ends the comparator with an ascending original
index (`:220-224`). The result is therefore deterministic regardless of the
sort algorithm's stability: *same inputs → same output order, always*.
Level 5 is the final, total tie-break.

---

## How `Relevance` gets its score

`Relevance` never computes a score itself. The score comes from the caller
and arrives through the `fuzzy_scores` map.

| Caller | Score source | Passed as |
|---|---|---|
| `SnippetSelector::resolve` — `src/selector.rs:431-437` | `score_fuzzy_matches(&snippets.snippets, query, SearchFields::fuzzy_default())` (`src/selector.rs:418-419`) | `Some(&fuzzy_scores)` |
| `snp list --filter` — `src/commands/list_cmd.rs:135` | inline `shared_fuzzy_matcher().fuzzy_match()` (`src/commands/list_cmd.rs:100`) | `filter.is_some().then_some(&fuzzy_scores)` (`src/commands/list_cmd.rs:138`) |
| MCP `snippets_search` — `src/mcp/tools.rs:111` | inline `shared_fuzzy_matcher().fuzzy_match()` (`src/mcp/tools.rs:103`) | `Some(&scores)` |

When the caller passes `None`, or the map has no entry for an index,
`fuzzy_score` is `None` (`src/sort.rs:125`) and the snippet sorts after every
scored snippet under `Relevance`.

**Empty query ⇒ no scores.** `score_fuzzy_matches` returns every non-deleted
index with an empty score map for an empty query (`src/selector.rs:176-183`).
Under `Relevance` that means all scores are `None` → all equal → levels 3 and
4 are skipped → level 5 (index ascending) decides. The selector's own filter
(`src/selector.rs:441-445`) then keeps every candidate because an empty query
matches everything.

`fuzzy_scores` are also a **tie-break at level 3** for every non-`Relevance`
mode — so `snp list --sort recent --filter git` orders by recency first and
fuzzy score only separates otherwise-identical recency.

---

## How `LastUsed` and `MostUsed` read usage

The `usage` argument is an **optional parallel slice** of `UsageData`, one
entry per snippet, indexed the same way as `indices`
(`src/sort.rs:104-105`).

```rust
let u = usage.and_then(|u| u.get(idx)).cloned().unwrap_or_default();  // :124
```

**A snippet with no usage record** gets `UsageData::default()`
(`src/usage.rs:31-34`) — `use_count: 0`, `last_used_at: None`. There is no
sentinel and no error: a never-used snippet simply sorts last under
`LastUsed` (`None` after every `Some`, `src/sort.rs:171-179`) and ties at
`use_count = 0` under `MostUsed` (`:180-191`).

`UsageData` is produced by `UsageIndex::get_usage(id)`
(`src/usage.rs:100-111`), which returns the same zeroed default when no
entry exists for the ID. The index is persisted at
`~/.config/snp/usage.toml` as a TOML array-of-tables (`[[entries]]`, keyed by
snippet ID — `src/usage.rs:11-16,42-54`) and loads **fail-open**
(`src/usage.rs:127-140`): a missing or unreadable file yields an empty index
(`:128-131`); a corrupt file is warned about, backed up, and reset to empty
(`:132-139`) — usage history is low-value data and never blocks loading.

Callers only build the usage slice when the mode needs it — `snp list`
skips the file read and parse for every mode except `LastUsed` and
`MostUsed` (`src/commands/list_cmd.rs:122-133`).

The usage index itself — on-disk format, update policy, and who writes it —
is documented in [usage.md](usage.md).

---

## Callers

| Caller | Line | Mode | `favorites_first` | Usage slice |
|---|---|---|---|---|
| `snp list` | `src/commands/list_cmd.rs:135` | `--sort` (default `Relevance`) | `--favorites-first` | only for `LastUsed` / `MostUsed` |
| `SnippetSelector::resolve` (fuzzy path) | `src/selector.rs:431` | `Relevance` | `false` (via `..Default::default()`, `src/selector.rs:427-430`) | `None` |
| MCP `snippets_search` | `src/mcp/tools.rs:111` | `Relevance` | `false` (explicit, `src/mcp/tools.rs:117`) | `None` |

---

## The TUI comparator

`src/ui/mod.rs:173-291` — `sort_filtered_indices()` — is a **separate**
implementation over `ui::state::SortMode` (`src/ui/state.rs:4-15`:
`None`, `Newest`, `Oldest`, `AlphaAsc`, `AlphaDesc`, `LastUsed`, `MostUsed`,
`Command`). It is *not* `rank_snippets`, and it differs in three ways a
reviewer must know:

| Aspect | `rank_snippets` | TUI comparator |
|---|---|---|
| Fuzzy score | tie-break at **level 3** | **primary key** (`src/ui/mod.rs:198-205`) |
| Sort algorithm | `sort_by` (stable) | `sort_unstable_by` (`src/ui/mod.rs:187`) — determinism comes from explicit index tie-breaks, not stability |
| Final index tie-break | always **ascending** | mode-dependent: `Newest` → index **descending** (`src/ui/mod.rs:215`), `Oldest` → index **ascending** (`src/ui/mod.rs:226`) |
| `Relevance` member | yes | **no** — `SortMode` has no `Relevance` variant |

`SortMode::Command` is initial-only, mapped from `--sort command` and never
produced by the interactive sort toggles (`src/ui/state.rs:12-14`).

The TUI comparator short-circuits entirely when there is nothing to sort by:
`sort_mode == None && !has_filter && !favorites_first` → early return
(`src/ui/mod.rs:183-185`), preserving file order.

---

## Related

- [selector.md](selector.md) — the search-field contract and the fuzzy
  scoring entry points this module ranks.
- [usage.md](usage.md) — the persisted usage records this module reads.
- [output.md](output.md) — `OutputPresentation::for_scoring()`, the 512-byte
  bound that keeps the `output` field out of pathological score inputs.
- [mcp.md](mcp.md) — the MCP surface that calls `rank_snippets` directly.

---

## Key Files

| Path | Role |
|---|---|
| `src/sort.rs` | `SnippetSort`, `SortOptions`, `rank_snippets`, the tie-break chain |
| `src/sort.rs:113-228` | `rank_snippets` — projection, comparator, final index tie-break |
| `src/sort.rs:230-1334` | 47 unit tests — every mode, favorites-first × usage, divergent metadata, relevance ties, determinism |
| `src/usage.rs` | `UsageIndex` / `UsageData` — the persisted usage records the sort reads |
| `src/usage.rs:31-34` | `UsageData` — `use_count` + `last_used_at`, `Default` = zeroed |
| `src/usage.rs:100-111` | `get_usage()` — zeroed default for an unknown snippet ID |
| `src/usage.rs:127-140` | `load_from()` — fail-open load with corrupt-file backup |
| `src/commands/list_cmd.rs:122-138` | Builds the optional usage slice and passes the optional fuzzy scores |
| `src/selector.rs:425-437` | `Relevance` ranking inside the selector's fuzzy path |
| `src/mcp/tools.rs:111-118` | `Relevance` ranking for MCP search (`favorites_first: false`) |
| `src/ui/mod.rs:173-291` | The separate TUI comparator |
| `src/ui/state.rs:4-15` | `ui::state::SortMode` — the TUI's own mode enum |

---

## Invariants

- The default sort mode is always `Relevance`; every explicit mode is
  deterministic — same inputs produce the same output order.
- `rank_snippets` never mutates the caller's snippet slice, never writes
  usage metadata, and never touches the library or TOML order. It is
  read-only.
- The tie-break chain is total: level 5 (original index, ascending) means
  there is never an unresolved tie.
- Deleted snippets never reach the sorter — callers pass live-only indices
  (`src/commands/list_cmd.rs:91,113`; `src/selector.rs:363`).
- `MostUsed` breaks `use_count` ties by `last_used_at` descending; a `None`
  last-used sorts after a `Some`; an all-`None` group falls through to index
  order.
- `Relevance` order is exactly *fuzzy score → original index*. Levels 3 and
  4 are skipped for it on purpose.
- Favorites-first is a grouping modifier, not a sort mode: it partitions
  first, then the selected mode orders within each group.

## Gotchas

- **The module docstring and the code disagree.** `src/sort.rs:7-13` lists
  the chain as *primary key → favorites → fuzzy → description → index*, but
  the code compares **favorites first** (`:150-155`). The code is
  authoritative; the docstring is stale.
- The docstring also lists the chain as 5 numbered items but omits that
  levels 3 and 4 are *skipped* for `Relevance` — the code comments at
  `:198` and `:212` state it explicitly.
- `usage` is indexed by **snippet index**, not by position in `indices`.
  Passing a slice built in a different order silently mis-attributes usage.
- `UsageData` for a missing record is `use_count: 0` / `last_used_at: None` —
  never an error, never skipped. A snippet with no record still appears in
  the ranking.
- `Recent` compares `updated_at` first and only falls back to `created_at`;
  two snippets updated in the same second are separated by `created_at`, not
  by usage or relevance.
- `Relevance` with an **empty** query is equivalent to file order, because
  there are no scores to sort on.
- `favorites_first` is `false` for both MCP search and the selector's fuzzy
  path. Only `snp list --favorites-first` (`src/commands/list_cmd.rs:30-31`)
  and the TUI turn it on — `snp search --favorites-first`
  (`src/commands/search_cmd.rs:19-20`) reaches the TUI comparator, not
  `rank_snippets`.
- The TUI and the CLI do **not** produce identical orders for the same data;
  the TUI makes fuzzy score primary and uses `sort_unstable_by`.
- `SnippetSort` is `#[non_exhaustive]` — a `match` on it needs a wildcard
  arm.
- `src/sort.rs` is ~230 lines of logic and ~1100 lines of tests; the test
  module starts at `src/sort.rs:230`.

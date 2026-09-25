# Sort and Ranking Module

[← Back to Overview](overview.md)

## Overview

Deterministic, stable snippet ranking for CLI (`--sort`,
`--favorites-first`) and the TUI (`n`/`o`/`a`/`z` keys). One function —
`rank_snippets()` in `src/sort.rs` (~230 lines of logic + ~1100 lines of
tests) — serves both the `list` command and the TUI's
`sort_filtered_indices()` with identical semantics.

## Key types — `src/sort.rs:37-92`

```rust
pub enum SnippetSort { Relevance, Recent, LastUsed, MostUsed, Description, Command }
pub struct SortOptions { pub mode: SnippetSort, pub favorites_first: bool }
struct RankedSnippet { index, desc_lower: Option<String>, cmd_lower: Option<String>,
    favorite: bool, updated_at: i64, created_at: i64,
    use_count: u64, last_used_at: Option<i64>, fuzzy_score: Option<i64> }
```

| Variant | Primary key | Direction |
|---------|------------|-----------|
| `Relevance` (default) | fuzzy score (`SkimMatcherV2`) | descending; `None` sorts after `Some` |
| `Recent` | `updated_at`, then `created_at` | descending |
| `LastUsed` | `last_used_at` (`UsageData`) | descending; `None` (never used) last |
| `MostUsed` | `use_count`, then `last_used_at` | descending |
| `Description` | lowercased description | ascending (A–Z) |
| `Command` | lowercased command | ascending (A–Z) |

`SnippetSort` is `clap::ValueEnum` (kebab-case), `#[non_exhaustive]`,
`Default = Relevance`. `desc_lower` is skipped under `Relevance` and
`cmd_lower` computed only for `Command` mode to avoid O(n) allocations.

## Key function — `src/sort.rs:113-228`

```rust
pub fn rank_snippets(
    indices: &[usize],                          // subset to order; need not be contiguous
    snippets: &[Snippet],
    fuzzy_scores: Option<&HashMap<usize, i64>>, // index → Skim score
    usage: Option<&[UsageData]>,                // parallel slice; missing → zeroed
    opts: &SortOptions,
) -> Vec<usize>                                 // sorted indices, never mutates input
```

### 5-level tie-break chain

1. **Favorites-first** (orthogonal modifier): `true` sorts before
   `false`, applied *before* the primary key so favorites form one block.
2. **Primary key** — the `SnippetSort` variant above.
3. **Fuzzy relevance** — tie-break when primary is not `Relevance`
   (`Some` beats `None`, higher first).
4. **Normalized description** — case-insensitive ascending; skipped for
   `Relevance` (spec: fuzzy score → index directly).
5. **Original index ascending** — guarantees stability for identical inputs.

## Filtered-relevance rule

- **TUI** (`sort_filtered_indices`): fuzzy score is the *primary* key;
  the explicit mode breaks ties — best text match first in an
  interactive selector.
- **`list`** (`rank_snippets` via `list_cmd`): fuzzy scores passed as
  `None`, so the explicit mode is the sole key — non-interactive output
  strictly respects the chosen order.
- No query → explicit mode fully determines ordering. Equal
  `Relevance` scores fall through to index order — usage metadata has no
  effect unless `--sort last-used` / `most-used` is explicit
  (compatibility-first default).

## Invariants / gotchas (from AGENTS.md)

- Default sort is always `Relevance`; all explicit sorts are
  deterministic (same inputs → same output).
- Sort is read-only: never writes usage metadata, never mutates the
  library or TOML order. Deleted snippets never reach it (callers pass
  live-only indices).
- `MostUsed` equal counts tie-break by `last_used_at` desc; `None`
  last-used sorts after `Some`; all-`None` falls to index order.

## File / line refs

- `src/sort.rs:37-61` (`SnippetSort`, `SortOptions`), `:94-119`
  (docs + signature), `:120-228` (ranking + tie-break), `:230-1334`
  (47 unit tests: all modes, favorites-first × usage, divergent
  metadata, relevance-tie, determinism).
- Callers: `src/commands/list_cmd.rs` (`rank_snippets`), `src/ui/mod.rs`
  (`sort_filtered_indices`), `src/selector.rs:425-437` (`Relevance`
  ranking for `resolve`).

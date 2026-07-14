# Sort and Ranking Module

## Purpose

Provides deterministic, stable sorting of snippets for both CLI (`--sort` flag) and TUI interactive sort. The module defines the sort-mode enum, options struct, and the core ranking function.

## Module: `src/sort.rs`

### Core Types

- `SnippetSort` — Enum of available sort modes (CLI-facing, clap-derivable).
- `SortOptions` — Combines a primary sort mode with optional modifiers (e.g., favorites-first).
- `RankedSnippet` — Internal helper that pairs a snippet index with precomputed sort keys.

### `SnippetSort` Variants

| Variant | Primary Key | Direction |
|---------|------------|-----------|
| `Relevance` (default) | Fuzzy score | Descending (highest first) |
| `Recent` | `updated_at`, then `created_at` | Descending |
| `LastUsed` | `last_used_at` from usage data | Descending |
| `MostUsed` | `use_count`, then `last_used_at` | Descending |
| `Description` | Lowercased description | Ascending (A–Z) |
| `Command` | Lowercased command | Ascending (A–Z) |

### Tie-Break Chain

Within equal primary keys, a deterministic 5-level tie-break ensures stable output:

1. **Primary key** — the selected `SnippetSort` variant
2. **Favorites-first** — when enabled, favorited snippets sort before non-favorited within each primary-key group
3. **Fuzzy relevance** — used as secondary when primary is not `Relevance` (e.g., `MostUsed` with equal counts)
4. **Normalized description** — case-insensitive alphabetical (skipped for `Relevance` mode)
5. **Original index** — ascending, guarantees stability for identical inputs

### `rank_snippets()`

```rust
pub fn rank_snippets(
    indices: &[usize],
    snippets: &[Snippet],
    fuzzy_scores: Option<&HashMap<usize, i64>>,
    usage: Option<&[UsageData]>,
    opts: &SortOptions,
) -> Vec<usize>
```

Returns sorted indices. The input `indices` need not be contiguous — the function handles filtered subsets.

## Integration Points

- **CLI flags** (`--sort`, `--favorites-first`): Available on `run`, `clip`, `search`, `select`, `list` commands. Parsed by clap via `clap::ValueEnum`.
- **TUI keybinds** (`n`/`o`/`a`/`z`): Toggle interactive sort modes in the selector. The TUI maintains its own `ui/state.rs::SortMode` enum and maps to `SnippetSort` when initializing.
- **Fuzzy matching**: `SkimMatcherV2` produces scores that are passed to `rank_snippets()` for `Relevance` mode.
- **Usage data**: `UsageData` is loaded from `usage.toml` and passed by slice for `LastUsed`/`MostUsed` modes.

## Invariants

- Default sort is always `Relevance` (unchanged from pre-Release-4 behavior).
- All explicit sorts are deterministic: same inputs always produce same output.
- Usage metadata is never written by the sort module — it is read-only.
- The sort module does not mutate the original snippet library or TOML order.

## Test Coverage

32 unit tests covering all sort modes, tie-break chains, edge cases (empty input, single element, all-equal, non-contiguous indices), favorites-first grouping, and default options. Integration tests verify CLI flags and TUI sort-through-selector behavior.

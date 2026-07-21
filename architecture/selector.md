# Snippet Selector Model

[← Back to CLI](cli.md)

## Overview

The selector module (`src/selector.rs`) provides deterministic non-TUI snippet
resolution. It is used by `snp get`, and by `run --id`, `clip --id`, and
`edit --id` to bypass the interactive TUI.

The selector never opens a TUI, never executes snippets, and never accesses
the clipboard.

## Key Types

### `SnippetSelector`

A deterministic snippet selector. All fields are optional except `resolution`
and `library`. At least one of `id`, `description_exact`, `command_exact`, or
`query` must be set.

```rust
pub struct SnippetSelector {
    pub id: Option<String>,              // Exact UUID match
    pub description_exact: Option<String>, // Exact description (case-insensitive)
    pub command_exact: Option<String>,     // Exact command (case-insensitive)
    pub query: Option<String>,            // Fuzzy query match
    pub library: LibraryScope,            // Library scope
    pub resolution: ResolutionPolicy,     // Multi-match policy
}
```

Uses a builder pattern:

```rust
let selector = SnippetSelector::new(ResolutionPolicy::Unique)
    .with_id("abc-123".to_string())
    .with_library(LibraryScope::AllLibraries);
```

### `ResolutionPolicy`

Controls behavior when a query matches multiple snippets:

```rust
pub enum ResolutionPolicy {
    Unique, // Return exactly one; fail if ambiguous or not found (default)
    First,  // Return the first result in stable order; never fail for ambiguity
    All,    // Return all matching results
}
```

### `SelectionResult`

The outcome of a resolution attempt:

```rust
pub enum SelectionResult {
    One(Box<SnippetMatch>),              // Exactly one match
    Many(Vec<SnippetMatch>),             // Multiple matches (policy = All)
    NotFound,                            // No match
    Ambiguous(Vec<SnippetIdentity>),     // Multiple matches, unique policy
}
```

### `SnippetMatch`

A matched snippet with its library context:

```rust
pub struct SnippetMatch {
    pub snippet: Snippet,
    pub library_path: PathBuf,
    pub library_name: String,
    pub library_id: String,
}
```

### `SnippetIdentity`

Lightweight identity information used in ambiguity reports:

```rust
pub struct SnippetIdentity {
    pub id: String,
    pub description: String,
    pub command: String,
    pub library_name: String,
}
```

### `LibraryScope`

Determines which libraries to search:

```rust
pub enum LibraryScope {
    Primary,         // Search the primary library only (default)
    Named(String),   // Search a specific named library
    AllLibraries,    // Search all libraries
}
```

## Resolution Priority

Selectors are applied in strict priority order:

1. **ID** (exact UUID match) — highest priority, skips all other fields
2. **Exact description** (case-insensitive string comparison)
3. **Exact command** (case-insensitive string comparison)
4. **Query** (fuzzy match using `skim` fuzzy matcher, ranked by relevance)

Only the first applicable selector is evaluated. If `id` is set, description,
command, and query are ignored.

## Resolution Policy Behavior

After matching, `resolve_matches()` applies the `ResolutionPolicy`:

- **Unique**: If exactly one match → `One`. If zero → `NotFound`. If more than
  one → `Ambiguous` (list of `SnippetIdentity` for error reporting).
- **First**: Always returns the first match in stable order. Never produces
  `Ambiguous`.
- **All**: Returns all matches as `Many`. Returns `NotFound` only if zero
  matches.

## Top-Level Entry Point

```rust
pub fn resolve_selector(selector: &SnippetSelector) -> SnipResult<SelectionResult>
```

This is the primary API. It:

1. Creates a `LibraryManager` and ensures library mode
2. Loads libraries based on `LibraryScope`
3. Calls `selector.resolve()` for each library
4. Aggregates results across libraries
5. Applies the resolution policy to the combined set

For `LibraryScope::AllLibraries`, matches from all libraries are collected
and the resolution policy is applied to the combined results.

## Deterministic Tie-Break

Fuzzy query results are ranked by the existing `rank_snippets()` infrastructure
(`src/sort.rs`) using `SnippetSort::Relevance` mode. This provides a stable,
deterministic ordering for identical fuzzy scores.

## Deleted Snippets

Deleted snippets (where `snippet.deleted == true`) are excluded from all
resolution modes. An ID lookup on a deleted snippet returns `NotFound`.

## Validation

`SnippetSelector::validate()` ensures at least one targeting field is set.
Calling `resolve()` without any of `id`, `description_exact`, `command_exact`,
or `query` produces an error.

## Integration with Commands

### `snp get`

The primary consumer. Builds a `SnippetSelector` from CLI flags and calls
`resolve_selector()`. Supports `--json`, `--raw`, `--expanded`, `--field`,
and `--var` for output formatting and variable expansion.

### `run --id`, `clip --id`, `edit --id`

These commands check for exact selector flags before entering the TUI selection
loop. If any exact selector is provided, they build a `SnippetSelector` with
`ResolutionPolicy::Unique`, resolve it, and proceed directly to the action
without opening the TUI.

## Tests

Unit tests in `src/selector.rs` cover:

- ID lookup (found, not found, duplicate)
- Description exact match (unique, ambiguous)
- Command exact match
- Fuzzy query match (Unique, First, All policies)
- Deleted snippet exclusion
- Library context propagation
- Validation (no fields set)

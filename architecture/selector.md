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

`LibraryScope::from_filter_arg()` / `from_owned_arg()` is the single
definition of `--library` / MCP `library` parsing: `None` → `Primary`,
`"all"` → `AllLibraries`, anything else → `Named`. `snp get`, exact-mode
`run`/`clip`/`edit` (via `exact_selector()` / `resolve_exact_target()`),
and MCP share it, so `"all"` handling cannot drift.

## Searchable-field contract (Plan 011)

```rust
pub struct SearchFields {
    pub include_tags: bool,
    pub include_output: bool,
}

pub fn searchable_text(snippet: &Snippet, fields: SearchFields) -> String;
pub fn score_fuzzy_matches(snippets: &[Snippet], query: &str, fields: SearchFields)
    -> (Vec<usize>, HashMap<usize, i64>);
pub fn matches_required_tags(snippet: &Snippet, required: &[String]) -> bool;
```

- `description` and `command` are always searched;
- `tags` (space-joined) are searched when `include_tags` is set — the default
  fuzzy contract (`SearchFields::fuzzy_default()`) used by `get --query`,
  `list --filter`, and MCP `snippets_search`;
- `output`/notes participate only when `include_output` is set
  (`list --search-output`, MCP `search_output: true`), bounded through
  `OutputPresentation::for_scoring()` (512 chars);
- `folders`, `favorite`, sync metadata, credentials, and keychain data are
  never searchable.

`score_fuzzy_matches()` centralizes deleted filtering and `SkimMatcherV2`
scoring (empty query matches every live snippet); callers rank with the
existing `rank_snippets()` `Relevance` mode. `matches_required_tags()` backs
MCP's explicit `tags` filter (every listed tag present, case-insensitive).

## Exact-target constructor and aggregate

```rust
pub fn exact_selector(library: Option<String>, id: Option<String>,
    description_exact: Option<String>, command_exact: Option<String>) -> SnippetSelector;
pub fn sort_matches(matches: &mut [SnippetMatch]);
pub fn finish_aggregate(matches: Vec<SnippetMatch>, resolution: &ResolutionPolicy)
    -> SnipResult<SelectionResult>;
```

`exact_selector()` is the canonical builder for `run`/`clip`/`edit` exact
paths and `snp get` exact fields (ID case-sensitive; description/command
case-insensitive). `finish_aggregate()` is the single definition of
cross-library ordering (library → description → ID) and policy application
(`All` → `Many`, `First` → first, `Unique` → `Ambiguous`). Both
`resolve_selector()` and `resolve_selector_readonly()` collect per-library
candidates with an `All`-policy collector so a multi-match inside one library
still contributes to the cross-library verdict (previously a `Unique`
per-library `Ambiguous` was dropped from `all`-scope results).

## Resolution Priority

Selectors are applied in strict priority order:

1. **ID** (exact case-sensitive UUID match) — highest priority, skips all other fields
2. **Exact description** (case-insensitive string comparison)
3. **Exact command** (case-insensitive string comparison)
4. **Query** (fuzzy over canonical `searchable_text` with
   `SearchFields::fuzzy_default()` — description, command, tags — ranked by relevance)

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

## Top-Level Entry Points

```rust
pub fn resolve_selector(selector: &SnippetSelector) -> SnipResult<SelectionResult>
pub fn resolve_selector_readonly(selector: &SnippetSelector) -> SnipResult<SelectionResult>
```

`resolve_selector()` is the mutating API. It:

1. Creates a `LibraryManager` and ensures library mode (may migrate legacy
   state)
2. Loads libraries based on `LibraryScope`
3. Calls `selector.resolve()` for each library
4. Aggregates results across libraries
5. Applies the resolution policy to the combined set

`resolve_selector_readonly()` (Plan 009) is the side-effect-free counterpart
for deterministic read paths (`snp get`, MCP). It consumes the canonical
`library::readonly_library_sources()` resolver, so legacy single-file
handling, primary resolution, and path construction cannot drift from the
library layer. A `Primary` scope with no visible source preserves the
historical "No primary library" error; an `all` scope with no visible
libraries resolves to `NotFound`.

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
`resolve_selector_readonly()`. Supports `--json`, `--raw`, `--expanded`, `--field`,
and `--var` for output formatting and variable expansion. Read-only: a legacy
single-file checkout is read in place without migration or file creation.

### `run --id`, `clip --id`, `edit --id`

These commands check for exact selector flags before entering the TUI selection
loop. If any exact selector is provided, they build the selector via the
canonical `exact_selector()` / `resolve_exact_target()` path
(`ResolutionPolicy::Unique`, shared `"all"` handling) and proceed directly
to the action without opening the TUI. `snp get` exact fields reuse the same
constructor; MCP `snippet_get` (ID / description / command) resolves through
`resolve_selector_readonly()` with `All` policy mapped to structured
`not_found` / `ambiguous` results.

### MCP read parity (Plan 011)

MCP is a thin adapter over the same helpers: `snippets_search` uses
`searchable_text()` + `matches_required_tags()` + `Relevance` ranking, and
`snippet_get` uses `resolve_selector_readonly()`. No execution, no mutation,
no interactive variable expansion. See `docs/MCP.md` for the tool schemas.

## Tests

Unit tests in `src/selector.rs` cover:

- ID lookup (found, not found, duplicate)
- Description exact match (unique, ambiguous)
- Command exact match
- Fuzzy query match (Unique, First, All policies)
- Deleted snippet exclusion
- Library context propagation
- Validation (no fields set)

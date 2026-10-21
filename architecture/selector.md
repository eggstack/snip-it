# Snippet Selector Model

[← Back to Overview](overview.md) · [← Back to CLI](cli.md)

## Overview

`src/selector.rs` (1208 LOC, 29 unit tests) is the deterministic,
**non-TUI** snippet-resolution layer. It turns CLI flags / MCP parameters into
a `SnippetSelector`, applies it to one or more libraries, and returns a
`SelectionResult`. It also owns the **search-field contract**
(`SearchFields` + `searchable_text`) that keeps `snp get --query`,
`snp list --filter`, and MCP `snippets_search` searching the same fields.

**What this document does not cover.** The interactive TUI selector lives in
[ui.md](ui.md) and [tui.md](tui.md) and has its own comparator
(see [sort.md](sort.md)). The snippet data model (`Snippet`, `Snippets`,
`LibraryMeta`) is in [library.md](library.md). The MCP wire schema and its
read-only stdio boundary are in [mcp.md](mcp.md). The `output` field's
presentation rules are in [output.md](output.md).

### The one rule that matters most

`src/selector.rs` exposes **three** resolution entry points, and they are not
interchangeable. Picking the mutating one on a read path re-introduces
recovery side effects into a surface that must not have them:

| Entry point | Library source | `ensure_library_mode()` | Use on |
|---|---|---|---|
| `resolve_selector_readonly()` — `src/selector.rs:623` | `library::readonly_library_sources()` (`:629`) | **no** | read paths: `snp get`, MCP, inspection |
| `resolve_selector()` — `src/selector.rs:681` | `LibraryManager` (`:682`) | **yes** (`:683`) | mutating / recovery-owning commands |
| `resolve_exact_target()` — `src/selector.rs:604` | delegates to `resolve_selector()` (`:611`) | **yes** (indirectly) | exact-mode `run` / `clip` / `edit` |

`ensure_library_mode()` may create `libraries/`, copy a legacy single-file
checkout, write `libraries.toml`, and bump the library generation. A read
path that triggers it mutates the user's library layout as a side effect of
merely looking something up.

---

## Data model

### `LibraryScope` — the `all` keyword

`src/selector.rs:48-56`

```rust
pub enum LibraryScope {
    #[default]
    Primary,        // primary library only
    Named(String),  // one named library
    AllLibraries,   // every registered library
}
```

`src/selector.rs:65-76`

```rust
pub fn from_filter_arg(library: Option<&str>) -> Self {
    match library {
        None => LibraryScope::Primary,
        Some("all") => LibraryScope::AllLibraries,
        Some(name) => LibraryScope::Named(name.to_string()),
    }
}

pub fn from_owned_arg(library: Option<String>) -> Self {
    Self::from_filter_arg(library.as_deref())
}
```

This is the **single definition** of the `all` keyword for the selector
family. Note that `None` means *primary*, **not** *all* — the two are
distinct scopes and `None` is never silently widened.

`"all"` is also a **reserved library name**: `validate_library_name`
(`src/library/model.rs:213-223`) rejects it, and `create_library` enforces it
(`src/library/manager.rs:366-367`), so `snp library create all` fails with

> Invalid library name
> Library name cannot be 'all' (reserved as the cross-library scope keyword). Use 'snp list --library all' to target every library.

The rationale is in the source comment (`model.rs:213-217`): a library
actually named `all` could never be addressed exclusively by the resolver
surfaces, and `--library all` silently returned the wrong scope on them
instead of an error.

### `SnippetSelector` — field semantics

`src/selector.rs:267-280`

```rust
pub struct SnippetSelector {
    pub id: Option<String>,              // exact snippet ID
    pub description_exact: Option<String>, // exact description, case-insensitive
    pub command_exact: Option<String>,   // exact command, case-insensitive
    pub query: Option<String>,           // fuzzy query
    pub library: LibraryScope,
    pub resolution: ResolutionPolicy,
}
```

All fields are optional except `library` and `resolution`. At least one of
`id`, `description_exact`, `command_exact`, `query` must be set — enforced by
`validate()` (`src/selector.rs:326-338`), which errors with

> No selector specified
> Provide --id, --description-exact, --command-exact, or --query

| Field | Match semantics | Implementation |
|---|---|---|
| `id` | **case-sensitive** byte equality on the whole ID | `s.id == id.as_str()` — `src/selector.rs:369` |
| `description_exact` | **case-insensitive** ASCII equality of the whole description | `s.description.eq_ignore_ascii_case(desc)` — `src/selector.rs:400` |
| `command_exact` | **case-insensitive** ASCII equality of the whole command | `s.command.eq_ignore_ascii_case(cmd)` — `src/selector.rs:410` |
| `query` | fuzzy subsequence match over `searchable_text` | `src/selector.rs:418-419` |
| `library` | scope: primary / named / all | `LibraryScope` — `src/selector.rs:48-77` |
| `resolution` | what to do with multiple matches | `ResolutionPolicy` — `src/selector.rs:217-225` |

Note the asymmetry: **`id` is case-sensitive, description and command are
case-insensitive.** This is the same rule MCP `snippet_get` must reproduce.

Builder methods: `new()` (`:284`), `with_id()` (`:296`), `with_description_exact()`
(`:302`), `with_command_exact()` (`:308`), `with_query()` (`:314`),
`with_library()` (`:320`).

### `ResolutionPolicy`

`src/selector.rs:215-225`

```rust
pub enum ResolutionPolicy {
    #[default]
    Unique, // exactly one; Ambiguous if >1; NotFound if 0
    First,  // first match in stable order; never Ambiguous
    All,    // every match
}
```

### `SelectionResult`

`src/selector.rs:251-260`

```rust
pub enum SelectionResult {
    One(Box<SnippetMatch>),           // exactly one match
    Many(Vec<SnippetMatch>),          // policy = All
    NotFound,                         // zero matches
    Ambiguous(Vec<SnippetIdentity>),  // >1 match under Unique
}
```

There is **no `None` variant** — "no match" is `NotFound`. `Ambiguous`
carries identities, not full snippets, so an ambiguity report never has to
clone a command body.

### `SnippetMatch` and `SnippetIdentity`

`src/selector.rs:229-247`

```rust
pub struct SnippetMatch {
    pub snippet: Snippet,
    pub library_path: PathBuf,
    pub library_name: String,  // filename without .toml
    pub library_id: String,    // server-side ID if linked
}

pub struct SnippetIdentity {
    pub id: String,
    pub description: String,
    pub command: String,
    pub library_name: String,
}
```

`SnippetIdentity` is exactly what an ambiguous-match report shows: ID,
description, command, and the owning library name.

---

## The three resolution paths

### `resolve_exact_target` — the mutating exact path

`src/selector.rs:604-612`

```rust
pub fn resolve_exact_target(
    library: Option<String>,
    id: Option<String>,
    description_exact: Option<String>,
    command_exact: Option<String>,
) -> SnipResult<SelectionResult> {
    let selector = exact_selector(library, id, description_exact, command_exact);
    resolve_selector(&selector)
}
```

`exact_selector()` (`src/selector.rs:579-597`) builds the selector with
`ResolutionPolicy::Unique` and `LibraryScope::from_owned_arg(library)`.
This is the single canonical constructor for the exact-mode `run`, `clip`,
and `edit` paths.

Because it delegates to `resolve_selector()`, **this path mutates** — it
constructs a `LibraryManager` and calls `ensure_library_mode()`. That is
correct for `run`/`clip`/`edit`, which are allowed to own recovery, but it
means `resolve_exact_target` must never be used from a read-only surface.

### `resolve_selector_readonly` — the side-effect-free path

`src/selector.rs:623-674`

- Maps the scope to the library-layer argument
  (`Primary → None`, `Named(n) → n`, `AllLibraries → Some("all")`) at
  `:624-628`.
- Calls `crate::library::readonly_library_sources(scope_arg.as_deref())`
  (`:629`) — the canonical resolver in
  `src/library/manager.rs:248-289`. It performs **no migration and creates no
  files or directories**; `None` → primary (or the legacy file in
  single-file mode), `Some("all")` → every visible library,
  `Some(name)` → that library or `library_not_found`.
- Empty source list (`:631-646`): `Primary` → error
  `"No primary library"`; `AllLibraries` → `NotFound`; `Named` → `NotFound`
  (the library layer already errored for unknown names).
- Exactly one source (`:648-652`): resolve directly against it.
- Multiple sources (`:654-673`): collect per-library candidates with an
  `All`-policy **collector** (`:658-661`) so a multi-match *inside* one
  library is not dropped as `Ambiguous` before the cross-library verdict,
  then `finish_aggregate()`.

### `resolve_selector` — the mutating path

`src/selector.rs:681-737`

- `LibraryManager::new()` + `ensure_library_mode()` (`:682-683`) — this is
  the step that can migrate, create, and rewrite metadata.
- `Primary` (`:686-699`): primary library path; error
  `"No primary library"` when absent.
- `Named(name)` (`:700-710`): `get_library_by_filename(name)` or
  `library_not_found(name)`.
- `AllLibraries` (`:711-735`): iterate `mgr.list_libraries()`, collect with
  the same `All`-policy collector, then `finish_aggregate()`.

### Side-effect matrix

| Operation | `resolve_selector_readonly` | `resolve_selector` |
|---|---|---|
| Creates `libraries/` dir | no | **yes** (`ensure_library_mode`) |
| Copies a legacy single-file checkout | no | **yes** |
| Writes `libraries.toml` / bumps generation | no | **yes** |
| Read-only consumers | `snp get`, MCP `snippet_get`, `snp list` (via `load_readonly_snippets`) | `run`/`clip`/`edit` exact paths |

---

## Library scope by surface — `all` diverges on purpose

The `all` keyword is **not** uniformly supported. Three surfaces accept it,
one rejects it with an accurate message, and one mutating path silently
fails with a *wrong* message.

| Surface | Entry | `--library all` |
|---|---|---|
| `snp get` | `exact_selector()` + `resolve_selector_readonly()` — `src/commands/get_cmd.rs:149,168` | **works** |
| `snp list` | `commands::load_readonly_snippets()` → `library::readonly_library_sources()` — `src/commands/list_cmd.rs:71` | **works** |
| MCP (`snippets_list`, `snippets_search`, `snippet_get`) | `LibraryScope::from_filter_arg()` + `resolve_selector_readonly()` — `src/mcp/tools.rs:146,160` | **works** |
| exact-mode `run` / `clip` / `edit` | `resolve_exact_target()` → `resolve_selector()` — `src/selector.rs:604` | **works** |
| `snp search` | explicit guard — `src/commands/search_cmd.rs:37-44` | **rejected** (see below) |
| `snp new`, `snp edit` (single-library mutating path) | `commands::get_library_path()` — `src/commands/mod.rs:91-113` | **fails** with `library_not_found` |

### Why `snp search --library all` is rejected

`src/commands/search_cmd.rs:37-44`:

> Library 'all' is not supported by 'snp search'
> The interactive selector operates on one library. Use 'snp list --library all' to list every library, or 'snp get --library all' to fetch a snippet by id.

This is a deliberate, accurate error, not a missing feature. The interactive
selector **edits and deletes within a single library file**, so a
cross-library scope has no coherent write target. Before this guard, the
lookup failed with `"Library 'all' does not exist"`, which was false —
`all` is the scope keyword, and it *does* work for `snp get`, `snp list`, and
MCP. `SnipError::Runtime` maps to exit code **1**
(`src/main.rs:1025-1030`).

### Why `commands::get_library_path` does not understand `all`

`src/commands/mod.rs:91-113` resolves a library by **filename** and never
consults `LibraryScope`:

```rust
let lib = mgr
    .get_library_by_filename(&name)
    .ok_or_else(|| crate::library::library_not_found(&name))?;
```

`get_library_by_filename("all")` finds nothing, so the caller gets
`library_not_found("all")` → *"Library 'all' does not exist. Use 'snp
library list' to see available libraries."*
(`src/library/model.rs:154-161`) — a misleading message, because `all` is
reserved, not missing. This path is used by the **single-library mutating**
commands (`snp new`, `snp edit`). Do not route a scope through it.

### `snp list` resolves read-only

`snp list` does **not** go through `get_library_path`. It calls
`commands::load_readonly_snippets()` (`src/commands/mod.rs:127-147`), which
uses `library::readonly_library_sources()` and concatenates the snippets of
every resolved source in resolution order.

The comment at `src/commands/list_cmd.rs:65-70` records why: routing `list`
through `get_library_path` called `ensure_library_mode()`, which created
`libraries/`, copied the legacy file (leaving the original behind as a
second, now-inert copy), wrote `libraries.toml`, and bumped the generation —
all from a plain `snp list`.

---

## Search-field contract

### `SearchFields`

`src/selector.rs:91-128`

```rust
pub struct SearchFields {
    pub include_tags: bool,
    pub include_output: bool,
}
```

| Constructor | `include_tags` | `include_output` | Used by |
|---|---|---|---|
| `Default` (`:98-105`) | ✅ | ❌ | generic default |
| `description_and_command()` (`:109-114`) | ❌ | ❌ | historical minimum |
| `fuzzy_default()` (`:117-119`) | ✅ | ❌ | alias of `default()`; used by `SnippetSelector::resolve` |
| `with_output()` (`:122-127`) | ✅ | ✅ | `snp list --search-output`, MCP `search_output: true` |

### `searchable_text`

`src/selector.rs:135-158`

Builds the canonical searchable text for a snippet: `description` and
`command` **always**, `tags` (space-joined) when `include_tags`, and
`output` (through `OutputPresentation::for_scoring()`, capped at 512
**bytes** with a UTF-8 char-boundary backup) when `include_output`. Empty
fields are skipped; parts are joined with a single space.

### `score_fuzzy_matches`

`src/selector.rs:166-198`

Returns `(matched_indices, scores)` where indices are positions in the
input slice and scores are `SkimMatcherV2` scores.

- **Empty query** → every non-deleted snippet is returned, with **no scores**
  (`:176-183`). Callers then rank by index order.
- **Non-empty query** → `matcher.fuzzy_match(&display, query)` over
  `searchable_text` (`:188-195`).
- Deleted snippets are skipped in both branches (`:178`, `:185`).
- Snippets whose searchable text is empty can never match (`:189-191`).

### `matches_required_tags`

`src/selector.rs:202-212` — every required tag must be present, compared
ASCII case-insensitively; an empty requirement list matches everything.
This backs MCP's explicit `tags` parameter only.

### The parity contract across the three surfaces

**What is shared is the field set (`searchable_text`), not the scoring
call.** Only `snp get --query` calls `score_fuzzy_matches()`; the other two
surfaces call `shared_fuzzy_matcher().fuzzy_match()` directly on the same
`searchable_text` output.

| Surface | Entry point | Field set | Scorer |
|---|---|---|---|
| `snp get --query` | `src/selector.rs:418-419` | `SearchFields::fuzzy_default()` — description + command + tags | `score_fuzzy_matches()` (`:166`) |
| `snp list --filter` | `src/commands/list_cmd.rs:86-108` | `include_tags: true`, `include_output: --search-output` (`:80-83`) | `shared_fuzzy_matcher().fuzzy_match()` inline (`:100`) |
| MCP `snippets_search` | `src/mcp/tools.rs:74-127` | `include_tags: true`, `include_output: search_output` (`:79-83`) | `shared_fuzzy_matcher().fuzzy_match()` inline (`:103`) |

Consequences a reviewer should hold onto:

- `snp get --query` has **no** `--search-output` flag — `output`/notes are
  never searched on that surface, and `SearchFields::fuzzy_default()` is
  hard-wired at `src/selector.rs:419`.
- `output`/notes join **only** via `SearchFields::with_output()`, i.e.
  `snp list --search-output` and MCP `search_output: true`.
- `folders`, `favorite`, sync metadata (`id`, `device_id`, timestamps,
  `deleted`), and credentials / keychain data are **never** searchable on
  any surface.
- `snp list --filter` filters out snippets with empty searchable text
  before matching (`src/commands/list_cmd.rs:94-97`).
- MCP `snippets_search` additionally applies the `tags` filter
  (`src/mcp/tools.rs:91-93`) — the CLI has no equivalent flag.

**Adding a field changes three surfaces with no compiler error.** Extend
`SearchFields` (not `searchable_text` directly), decide the default, move
the CLI flag and the MCP parameter together, and extend
`tests/selector_search_parity.rs`.

---

## Resolution priority

`SnippetSelector::resolve()` — `src/selector.rs:354-454` — applies
selectors in strict priority order. **Only the first applicable field is
evaluated**; if `id` is set, description, command, and query are ignored.

1. **ID** — exact, case-sensitive (`:366-394`). A duplicate ID yields
   `Ambiguous` rather than picking one (`:380-392`).
2. **Exact description** — case-insensitive (`:397-404`).
3. **Exact command** — case-insensitive (`:407-414`).
4. **Query** — fuzzy over `searchable_text` with
   `SearchFields::fuzzy_default()`, ranked by `SnippetSort::Relevance`
   (`:417-450`).

The policy is then applied by `resolve_matches()` (`:457-512`):

- `All` → `Many`; `NotFound` when empty (`:465-480`).
- `First` → the first match as `One`; never `Ambiguous` (`:481-489`).
- `Unique` → `One` / `NotFound` / `Ambiguous` (`:490-511`).

Deleted snippets are excluded from every mode, including ID lookup
(`:363`).

---

## Aggregate ordering

`finish_aggregate()` — `src/selector.rs:536-571` — is the single definition
of cross-library behavior. It first sorts with `sort_matches()`, then
applies the policy.

`sort_matches()` — `src/selector.rs:519-527` — orders by
**library name (lowercased) → description (lowercased) → snippet ID**.
`sort_by_cached_key` evaluates each lowercased key once instead of
re-lowercasing on every comparison.

Both `resolve_selector()` and `resolve_selector_readonly()` share this
path, so a cross-library `all` result is ordered identically whichever
resolver produced it.

---

## Consumers

| Consumer | Entry | Resolver | Scope |
|---|---|---|---|
| `snp get` | `src/commands/get_cmd.rs:149,168` | `resolve_selector_readonly` | primary / named / all |
| `snp list` | `src/commands/list_cmd.rs:71` | `load_readonly_snippets` → `readonly_library_sources` | primary / named / all |
| MCP `snippet_get` | `src/mcp/tools.rs:145-160` | `resolve_selector_readonly` | primary / named / all |
| exact-mode `run` / `clip` / `edit` | `resolve_exact_target()` — `src/selector.rs:604`, called from `src/main.rs:503,549,639` | `resolve_selector` (mutating) | primary / named / all |
| `snp search` | `src/commands/search_cmd.rs:37-44` | interactive TUI | primary / named only |

`snp get` reads the requested `--resolution` policy
(`src/commands/get_cmd.rs:45-46`); `exact_selector()` starts at `Unique` and
`get_cmd.rs:151` re-applies the requested policy.

MCP `snippet_get` requires **exactly one** of `id` (case-sensitive),
`description`, or `command` (case-insensitive)
(`src/mcp/tools.rs:131-144`), enforced in code and by a JSON-Schema
`oneOf` (`src/mcp/protocol.rs:292`).

---

## Related

- [sort.md](sort.md) — the `Relevance` ranking used by the selector's fuzzy
  path.
- [output.md](output.md) — the `output` field's presentation and its
  512-byte search bound.
- [library.md](library.md) — the `Snippet` data model and the library
  manager behind both resolvers.
- [mcp.md](mcp.md) — the read-only stdio MCP surface that consumes
  `resolve_selector_readonly`.
- [commands/list_cmd.md](commands/list_cmd.md) and
  [commands/get_cmd.md](commands/get_cmd.md) — the two read-only CLI
  consumers.
- [`.skills/selector-and-search-parity.md`](../.skills/selector-and-search-parity.md)
  — the agent-facing parity contract and the resolver-split checklist. Read
  it before changing search fields or library scope.
- [`docs/LIBRARY_SCOPE.md`](../docs/LIBRARY_SCOPE.md) — the authoritative
  scope table.

---

## Key Files

| Path | Role |
|---|---|
| `src/selector.rs` | The whole selector: scope parsing, field contract, resolver split, aggregation |
| `src/selector.rs:24-26` | `shared_fuzzy_matcher()` — the single `SkimMatcherV2` instance |
| `src/selector.rs:32-44` | `contains_ignore_ascii_case()` — allocation-free ASCII substring check |
| `src/commands/get_cmd.rs:149,168` | `snp get` builds a selector and resolves read-only |
| `src/commands/list_cmd.rs:65-108` | `snp list` read-only load + inline fuzzy filter |
| `src/commands/search_cmd.rs:37-44` | The `snp search --library all` rejection |
| `src/commands/mod.rs:91-113` | `get_library_path` — mutating, single-library, no `all` |
| `src/commands/mod.rs:127-147` | `load_readonly_snippets` — read-only multi-library load |
| `src/library/manager.rs:248-289` | `resolve_readonly_sources` — the library-layer scope resolver |
| `src/library/model.rs:154-161` | `library_not_found` — the message `get_library_path` produces for `all` |
| `src/library/model.rs:213-223` | `validate_library_name` — `all` is reserved |
| `src/mcp/tools.rs:74-160` | MCP `snippets_search` / `snippet_get` |
| `src/sort.rs:113-228` | `rank_snippets` — the `Relevance` ranking used by the query path |
| `src/output.rs:93-110` | `for_scoring()` — the 512-byte bound on the `output` field |

---

## Invariants

- **Read-only paths use `resolve_selector_readonly`, never
  `resolve_selector`.** The latter runs `ensure_library_mode()` and can
  migrate and rewrite metadata; picking the wrong one reintroduces recovery
  side effects into a read-only surface.
- `LibraryScope::from_filter_arg` / `from_owned_arg` is the single
  definition of the `all` keyword; `exact_selector` /
  `resolve_exact_target` is the single exact-target constructor.
- `all` is a **reserved library name** — `validate_library_name` rejects it.
- `id` matching is **case-sensitive**; description and command matching are
  **case-insensitive**. MCP `snippet_get` must match the same rule.
- Deleted snippets are excluded from every resolution mode.
- `searchable_text` is the single source of truth for *which fields*
  participate in a search. Deleted filtering, scoring, and ranking live with
  the callers.
- `folders`, `favorite`, sync metadata, credentials, and keychain data are
  never searchable on any surface.
- `output`/notes participate in search only under
  `SearchFields::with_output()`, bounded to 512 bytes by
  `OutputPresentation::for_scoring()`.

## Gotchas

- `from_filter_arg(None)` is **Primary**, not "all". Passing `None` when you
  meant every library silently narrows the scope.
- `resolve_exact_target` is a **mutating** path. It looks read-only but runs
  `ensure_library_mode()`.
- `commands::get_library_path` bypasses `LibraryScope` entirely. It is the
  one path where `--library all` produces a *misleading* "does not exist"
  error rather than a scope error.
- `SelectionResult` has **no `None` variant** — "not found" is `NotFound`.
- `Ambiguous` carries `SnippetIdentity` (id / description / command /
  library name), not `SnippetMatch` — there is no library path or library ID
  in an ambiguity report.
- Only the **first** applicable selector field is evaluated. Setting `id`
  silently disables description / command / query.
- The MCP `tags` filter has no CLI equivalent; `snp list --filter` cannot
  filter by tag.
- `snp get --query` cannot search `output`/notes — there is no
  `--search-output` on that surface.
- `score_fuzzy_matches()` with an **empty** query returns every live snippet
  with **no scores**; under `Relevance` that degenerates to file order.
- `AllLibraries` with **no** visible libraries resolves to `NotFound` on the
  readonly path (`:639`) but to an empty aggregate → `NotFound` on the
  mutating path — while `Primary` with no library is an *error*
  (`"No primary library"`).

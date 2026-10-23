# Snippet Selector Model

[← Back to Overview](overview.md) · [← Back to CLI](cli.md)

## Overview

`src/selector.rs` (1412 LOC, 33 unit tests) is the deterministic,
**non-TUI** snippet-resolution layer. It turns CLI flags / MCP parameters into
a `SnippetSelector`, applies it to one or more libraries, and returns a
`SelectionResult`. It also owns the **search-field contract**
(`SearchFields` + `searchable_text`) and the **match primitive**
(`score_fuzzy_matches`) that keep `snp get --query`, `snp list --filter`,
and MCP `snippets_search` selecting the same snippets for the same query.

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
| `resolve_selector_readonly()` — `src/selector.rs:678` | `library::readonly_library_sources()` (`:684`) | **no** | read paths: `snp get`, MCP, inspection |
| `resolve_selector()` — `src/selector.rs:736` | `LibraryManager` (`:737`) | **yes** (`:738`) | mutating / recovery-owning commands |
| `resolve_exact_target()` — `src/selector.rs:659` | delegates to `resolve_selector()` (`:666`) | **yes** (indirectly) | exact-mode `run` / `clip` / `edit` |

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
(`src/library/manager.rs:413-414`), so `snp library create all` fails with

> Invalid library name
> Library name cannot be 'all' (reserved as the cross-library scope keyword). Use 'snp list --library all' to target every library.

The rationale is in the source comment (`model.rs:242-246`): a library
actually named `all` could never be addressed exclusively by the resolver
surfaces, and `--library all` silently returned the wrong scope on them
instead of an error.

### `SnippetSelector` — field semantics

`src/selector.rs:310-323`

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
`validate()` (`src/selector.rs:369-381`), which errors with

> No selector specified
> Provide --id, --description-exact, --command-exact, or --query

| Field | Match semantics | Implementation |
|---|---|---|
| `id` | **case-sensitive** byte equality on the whole ID | `s.id == id.as_str()` — `src/selector.rs:412` |
| `description_exact` | **case-insensitive** ASCII equality of the whole description | `s.description.eq_ignore_ascii_case(desc)` — `src/selector.rs:443` |
| `command_exact` | **case-insensitive** ASCII equality of the whole command | `s.command.eq_ignore_ascii_case(cmd)` — `src/selector.rs:453` |
| `query` | fuzzy subsequence match over `searchable_text` | `src/selector.rs:460-462` |
| `library` | scope: primary / named / all | `LibraryScope` — `src/selector.rs:48-77` |
| `resolution` | what to do with multiple matches | `ResolutionPolicy` — `src/selector.rs:260-268` |

Note the asymmetry: **`id` is case-sensitive, description and command are
case-insensitive.** This is the same rule MCP `snippet_get` must reproduce.

Builder methods: `new()` (`:327`), `with_id()` (`:339`),
`with_description_exact()` (`:345`), `with_command_exact()` (`:351`),
`with_query()` (`:357`), `with_library()` (`:363`).

### `ResolutionPolicy`

`src/selector.rs:260-268`

```rust
pub enum ResolutionPolicy {
    #[default]
    Unique, // exactly one; Ambiguous if >1; NotFound if 0
    First,  // first match in stable order; never Ambiguous
    All,    // every match
}
```

### `SelectionResult`

`src/selector.rs:294-303`

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

`src/selector.rs:272-290`

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

`src/selector.rs:659-667`

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

`exact_selector()` (`src/selector.rs:634-652`) builds the selector with
`ResolutionPolicy::Unique` and `LibraryScope::from_owned_arg(library)`.
This is the single canonical constructor for the exact-mode `run`, `clip`,
and `edit` paths.

Because it delegates to `resolve_selector()`, **this path mutates** — it
constructs a `LibraryManager` and calls `ensure_library_mode()`. That is
correct for `run`/`clip`/`edit`, which are allowed to own recovery, but it
means `resolve_exact_target` must never be used from a read-only surface.

### `resolve_selector_readonly` — the side-effect-free path

`src/selector.rs:678-729`

- Maps the scope to the library-layer argument
  (`Primary → None`, `Named(n) → n`, `AllLibraries → Some("all")`) at
  `:679-683`.
- Calls `crate::library::readonly_library_sources(scope_arg.as_deref())`
  (`:684`) — the canonical resolver in
  `src/library/manager.rs:295-336`. It performs **no migration and creates no
  files or directories**; `None` → primary (or the legacy file in
  single-file mode), `Some("all")` → every visible library,
  `Some(name)` → that library or `library_not_found`.
- Empty source list (`:686-701`): `Primary` → error
  `"No primary library"` (`:689-692`); `AllLibraries` → `NotFound` (`:694`);
  `Named` → `NotFound` (`:698`) (the library layer already errored for
  unknown names).
- Exactly one source (`:703-707`): resolve directly against it.
- Multiple sources (`:709-726`): collect per-library candidates with an
  `All`-policy **collector** (`:713-716`) so a multi-match *inside* one
  library is not dropped as `Ambiguous` before the cross-library verdict,
  then `finish_aggregate()` (`:728`).

### `resolve_selector` — the mutating path

`src/selector.rs:736-792`

- `LibraryManager::new()` + `ensure_library_mode()` (`:737-738`) — this is
  the step that can migrate, create, and rewrite metadata.
- `Primary` (`:741-754`): primary library path; error
  `"No primary library"` when absent.
- `Named(name)` (`:755-765`): `get_library_by_filename(name)` or
  `library_not_found(name)`.
- `AllLibraries` (`:766-790`): iterate `mgr.list_libraries()`, collect with
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
| `snp list` | `commands::load_readonly_snippets()` → `library::readonly_library_sources()` — `src/commands/list_cmd.rs:73` | **works** |
| MCP (`snippets_list`, `snippets_search`, `snippet_get`) | list/search go through `load_snippets()` → `library::readonly_library_sources()` (`src/mcp/tools.rs:59,72,243-244`); `snippet_get` uses `LibraryScope::from_filter_arg()` + `resolve_selector_readonly()` (`src/mcp/tools.rs:136,150`) | **works** |
| exact-mode `run` / `clip` / `edit` | `resolve_exact_target()` → `resolve_selector()` — `src/selector.rs:659` | **works** |
| `snp search` | explicit guard — `src/commands/search_cmd.rs:37-44` | **rejected** (see below) |
| `snp new`, `snp edit` (single-library mutating path) | `commands::get_library_path()` — `src/commands/mod.rs:92-158` | **fails** with `library_not_found` |

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
(`src/main.rs:1089-1090`).

### Why `commands::get_library_path` does not understand `all`

`src/commands/mod.rs:92-158` resolves a library by **filename** and never
consults `LibraryScope`:

```rust
let lib = mgr
    .get_library_by_filename(&name)
    .ok_or_else(|| crate::library::library_not_found(&name))?;
```

`get_library_by_filename("all")` finds nothing, so the caller gets
`library_not_found("all")` → *"Library 'all' does not exist. Use 'snp
library list' to see available libraries."*
(`src/library/model.rs:183-190`) — a misleading message, because `all` is
reserved, not missing. This path is used by the **single-library mutating**
commands (`snp new`, `snp edit`). Do not route a scope through it.

### `snp list` resolves read-only

`snp list` does **not** go through `get_library_path`. It calls
`commands::load_readonly_snippets()` (`src/commands/mod.rs:170-190`), which
uses `library::readonly_library_sources()` and concatenates the snippets of
every resolved source in resolution order.

The comment at `src/commands/list_cmd.rs:67-72` records why: routing `list`
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
| `with_output()` (`:122-127`) | ✅ | ✅ | the opt-in contract; `snp list --search-output` and MCP `search_output: true` both construct it with `SearchFields::with_output()` (`src/commands/list_cmd.rs`, `src/mcp/tools.rs`) |

### `searchable_text`

`src/selector.rs:142-180`

Builds the canonical searchable text for a snippet: `description` and
`command` **always**, `tags` (space-joined) when `include_tags`, and
`output` (through `OutputPresentation::for_scoring()`, capped at 512
**bytes** with a UTF-8 char-boundary backup) when `include_output`. Empty
fields are skipped; parts are joined with a single space.

### `score_fuzzy_matches`

`src/selector.rs:209-241`

Returns `(matched_indices, scores)` where indices are positions in the
input slice and scores are `SkimMatcherV2` scores.

This is the **single match primitive** for all three search surfaces
(`snp get --query`, `snp list --filter`, MCP `snippets_search`). The field
projection lives in `searchable_text`; the match semantics live here, so the
three surfaces cannot disagree about *which* snippets a query selects. Only
the *ranking* of the returned indices is left to the callers
(`sort::rank_snippets`).

- **Empty query → "no filter".** Every non-deleted snippet is returned, with
  **no scores** (`:219-226`). That includes snippets whose searchable text is
  empty (no description, command, tags, or output): an empty query filters
  nothing.
- **Non-empty query** → `matcher.fuzzy_match(&display, query)` over
  `searchable_text` (`:227-239`).
- Deleted snippets are skipped in both branches (`:221`, `:228-230`).
- A snippet whose searchable text is empty never matches a non-empty query
  (`:232-234`).

### `matches_required_tags`

`src/selector.rs:245-255` — every required tag must be present, compared
ASCII case-insensitively; an empty requirement list matches everything.
This backs MCP's explicit `tags` parameter only.

### The parity contract across the three surfaces

**What is shared is the field set (`searchable_text`) *and* the match
primitive (`score_fuzzy_matches`).** All three surfaces call
`score_fuzzy_matches()`, so they cannot disagree about *which* snippets a
query selects. Only the *ranking* of the returned indices differs — see
[Aggregate ordering](#aggregate-ordering).

| Surface | Entry point | Field set | Match primitive |
|---|---|---|---|
| `snp get --query` | `src/selector.rs:461-462` | `SearchFields::fuzzy_default()` — description + command + tags | `score_fuzzy_matches()` (`src/selector.rs:209`) |
| `snp list --filter` | `src/commands/list_cmd.rs:107-108` | `SearchFields::with_output()` / `fuzzy_default()` (`src/commands/list_cmd.rs:92-96`) | `score_fuzzy_matches()` (`src/selector.rs:209`) |
| MCP `snippets_search` | `src/mcp/tools.rs:91` | `SearchFields::with_output()` / `fuzzy_default()` (`src/mcp/tools.rs:79-83`) | `score_fuzzy_matches()` (`src/selector.rs:209`) |

Consequences a reviewer should hold onto:

- `snp get --query` has **no** `--search-output` flag — `output`/notes are
  never searched on that surface, and `SearchFields::fuzzy_default()` is
  hard-wired at `src/selector.rs:462`.
- `output`/notes join **only** via `SearchFields::with_output()`, i.e.
  `snp list --search-output` and MCP `search_output: true`.
- `folders`, `favorite`, sync metadata (`id`, `device_id`, timestamps,
  `deleted`), and credentials / keychain data are **never** searchable on
  any surface.
- MCP `snippets_search` additionally applies the `tags` filter as an
  **intersection** with the query matches (`src/mcp/tools.rs:97-99`) — the
  CLI has no equivalent flag.
- An empty query is **"no filter"** on all three surfaces — see below.

#### The empty-query rule

An empty query is **"no filter"**: `snp get --query ""`, `snp list --filter
""`, and MCP `snippets_search` with `query: ""` each return **every
non-deleted snippet in scope**, with no scores — *including* snippets whose
searchable text is empty (no description, command, tags, or output). A
non-empty query matches the `searchable_text` projection only: a snippet with
an empty projection never matches, and deleted snippets never match any
query.

Rationale: two of the three surfaces already behaved this way, "no filter"
is the natural reading of an empty argument, and the alternative ("match
nothing searchable") would contradict `snp list` with no filter at all,
which returns every live snippet. When the documentation is silent, the
tiebreak is to preserve the behavior of the larger number of current
surfaces.

**Adding a field changes three surfaces with no compiler error.** Extend
`SearchFields` (not `searchable_text` directly), decide the default, move
the CLI flag and the MCP parameter together, and extend
`tests/selector_search_parity.rs` — which now **gates the build**, because it
runs in `scripts/check.sh`.

---

## Resolution priority

`SnippetSelector::resolve()` — `src/selector.rs:397-497` — applies
selectors in strict priority order. **Only the first applicable field is
evaluated**; if `id` is set, description, command, and query are ignored.

1. **ID** — exact, case-sensitive (`:409-437`). A duplicate ID yields
   `Ambiguous` rather than picking one (`:423-436`).
2. **Exact description** — case-insensitive (`:440-447`).
3. **Exact command** — case-insensitive (`:450-457`).
4. **Query** — fuzzy over `searchable_text` with
   `SearchFields::fuzzy_default()`, ranked by `SnippetSort::Relevance`
   (`:460-493`).

The policy is then applied by `resolve_matches()` (`:500-555`):

- `All` → `Many`; `NotFound` when empty (`:508-522`).
- `First` → the first match as `One`; never `Ambiguous` (`:524-531`).
- `Unique` → `One` / `NotFound` / `Ambiguous` (`:533-554`).

Deleted snippets are excluded from every mode, including ID lookup
(`:406`).

---

## Aggregate ordering

`finish_aggregate()` — `src/selector.rs:591-626` — is the single definition
of cross-library behavior. It first sorts with `sort_matches()`, then
applies the policy.

`sort_matches()` — `src/selector.rs:574-582` — orders by
**library name (lowercased) → description (lowercased) → snippet ID**.
`sort_by_cached_key` evaluates each lowercased key once instead of
re-lowercasing on every comparison.

Both `resolve_selector()` and `resolve_selector_readonly()` share this
path, so a cross-library `all` result is ordered identically whichever
resolver produced it.

### Cross-library ordering — two families, by design

The two surface families order a multi-library result differently, and the
difference is deliberate:

| Family | Surfaces | Cross-library order |
|---|---|---|
| **Selector family** — resolves per library, then aggregates | `snp get --query` (and `--resolution all`), `snp run/clip/edit --library all`, MCP `snippet_get` | library name → description → snippet ID (`sort_matches()`, `src/selector.rs:574-582`) |
| **Merged-set family** — merges the libraries first, then scores the merged set | `snp list --filter`, MCP `snippets_search` | global relevance (`sort::rank_snippets` under `SnippetSort::Relevance`, `src/sort.rs:116`) |

Within a **single** library the two orders coincide — same set, same
relevance order — because there is only one library to merge. Across more
than one library they can differ. That is intentional and is pinned by the
parity suite on each surface; it is **not** a defect.

Rationale: changing `get` would alter the documented `get --resolution all`
aggregate order and MCP `snippet_get`, which is outside this milestone's
scope.

---

## Consumers

| Consumer | Entry | Resolver | Scope |
|---|---|---|---|
| `snp get` | `src/commands/get_cmd.rs:149,168` | `resolve_selector_readonly` | primary / named / all |
| `snp list` | `src/commands/list_cmd.rs:73` | `load_readonly_snippets` → `readonly_library_sources` | primary / named / all |
| MCP `snippet_get` | `src/mcp/tools.rs:135-150` | `resolve_selector_readonly` | primary / named / all |
| exact-mode `run` / `clip` / `edit` | `resolve_exact_target()` — `src/selector.rs:659`, called from `src/main.rs:541,587,689` | `resolve_selector` (mutating) | primary / named / all |
| `snp search` | `src/commands/search_cmd.rs:37-44` | interactive TUI | primary / named only |

`snp get` reads the requested `--resolution` policy
(`src/commands/get_cmd.rs:45-46`); `exact_selector()` starts at `Unique` and
`get_cmd.rs:151` re-applies the requested policy.

MCP `snippet_get` requires **exactly one** of `id` (case-sensitive),
`description`, or `command` (case-insensitive)
(`src/mcp/tools.rs:121-134`), enforced in code and by a JSON-Schema
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
| `src/commands/list_cmd.rs:73-119` | `snp list` read-only load + the shared match primitive |
| `src/commands/search_cmd.rs:37-44` | The `snp search --library all` rejection |
| `src/commands/mod.rs:92-158` | `get_library_path` — mutating, single-library, no `all` |
| `src/commands/mod.rs:170-190` | `load_readonly_snippets` — read-only multi-library load |
| `src/library/manager.rs:295-336` | `resolve_readonly_sources` — the library-layer scope resolver |
| `src/library/model.rs:183-190` | `library_not_found` — the message `get_library_path` produces for `all` |
| `src/library/model.rs:208-254` | `validate_library_name` — `all` is reserved |
| `src/mcp/tools.rs:72-196` | MCP `snippets_search` / `snippet_get` |
| `src/sort.rs:116-231` | `rank_snippets` — the `Relevance` ranking used by the query path |
| `src/output.rs:99-115` | `for_scoring()` — the 512-byte bound on the `output` field |

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
  participate in a search; `score_fuzzy_matches` is the single source of
  truth for *which snippets* a query selects. Ranking lives with the
  callers.
- **An empty query is "no filter" on every search surface.** It matches
  every non-deleted snippet, including snippets whose searchable text is
  empty. A non-empty query matches the searchable-text projection only.
- Cross-library order differs by surface family **by design**: the selector
  family orders by library → description → snippet ID; the merged-set
  family orders by global relevance. The two coincide within a single
  library.
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
  with **no scores** — *including* snippets whose searchable text is empty.
  An empty query is "no filter", not "match nothing"; under `Relevance` it
  degenerates to input order.
- `AllLibraries` with **no** visible libraries resolves to `NotFound` on the
  readonly path (`:694`) but to an empty aggregate → `NotFound` on the
  mutating path — while `Primary` with no library is an *error*
  (`"No primary library"`).

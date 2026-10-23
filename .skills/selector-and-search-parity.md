# Selector, Search Parity & MCP Skill

## Purpose
Guide agents through `src/selector.rs` (deterministic non-TUI resolution),
`src/mcp/` (read-only stdio MCP server), and the **search-parity contract**
that binds `snp get --query`, `snp list --filter`, and MCP `snippets_search`
to one field set and one match primitive.

This is the highest-risk-to-break, lowest-visibility invariant in the repo.
There is **no compiler error** when parity breaks. The parity suite
(`tests/selector_search_parity.rs`) now **gates the build** — it runs in
`scripts/check.sh` — so a parity regression fails CI, but only for the
surfaces the suite actually covers. Read this before touching search,
filters, or any new consumer of snippet text.

---

## The parity contract (one field set, one match primitive, three consumers)

Two functions bind `snp get --query`, `snp list --filter`, and MCP
`snippets_search` to one behavior:

- `selector::searchable_text(snippet, fields)` (`src/selector.rs:142`) is the
  single source of truth for *which fields participate* in a search. It only
  decides field membership.
- `selector::score_fuzzy_matches(snippets, query, fields)`
  (`src/selector.rs:209`) is the **single match primitive** that all three
  surfaces call. It decides *which snippets a query selects* — the deleted
  filter, the empty-query rule, and the fuzzy match itself. Because all
  three call it, they cannot disagree about the match set. Only the
  *ranking* of the returned indices is left to the callers.

| Field | Always searched | Conditional |
|-------|-----------------|-------------|
| `description` | yes | — |
| `command` | yes | — |
| `tags` | — | `include_tags` (default **true**) |
| `output` / notes | — | `include_output` (default **false**) |
| `folders`, `favorite` | **never** | — |
| sync metadata (`id`, `device_id`, timestamps, `deleted`) | **never** | — |
| credentials / keychain data | **never** | — |

### Contract item 1 — an empty query is "no filter"

An **empty** query matches **every non-deleted snippet**, with **no
scores** — including snippets whose searchable text is empty (no
description, command, tags, or output). The rule is identical on all three
surfaces:

| Surface | Empty argument | Result |
|---------|----------------|--------|
| `snp get --query ""` | empty query | every non-deleted snippet in scope |
| `snp list --filter ""` | empty filter | exactly what `snp list` with no `--filter` returns |
| MCP `snippets_search` | `query: ""` | every non-deleted snippet in scope |

A **non-empty** query matches the `searchable_text` projection only: a
snippet with an empty projection never matches, and deleted snippets never
match any query.

Rationale: two of the three surfaces already behaved this way; "no filter"
is the natural reading of an empty argument; and the alternative ("match
nothing searchable") would contradict `snp list` with no filter at all,
which returns every live snippet. When the documentation is silent, the
tiebreak is to preserve the behavior of the larger number of current
surfaces.

### Contract item 2 — cross-library order differs by surface family, by design

| Family | Surfaces | Cross-library order |
|---|---|---|
| **Selector family** — resolves per library, then aggregates | `snp get --query` (and `--resolution all`), `snp run/clip/edit --library all`, MCP `snippet_get` | library name → description → snippet ID (`selector::sort_matches`) |
| **Merged-set family** — merges the libraries first, then scores the merged set | `snp list --filter`, MCP `snippets_search` | global relevance (`sort::rank_snippets` under `SnippetSort::Relevance`) |

Within a **single** library the two orders coincide — same set, same
relevance order — because there is only one library to merge. Across more
than one library they can differ. That is intentional and is pinned by the
parity suite on each surface; it is **not** a defect.

Rationale: changing `get` would alter the documented `get --resolution all`
aggregate order and MCP `snippet_get`, which is outside this milestone's
scope.

### `SearchFields` presets (`src/selector.rs:91-128`)

| Constructor | tags | output | Used by |
|-------------|------|--------|---------|
| `SearchFields::default()` | ✅ | ❌ | generic default |
| `description_and_command()` | ❌ | ❌ | historical minimum only |
| `fuzzy_default()` | ✅ | ❌ | alias of `default()` |
| `with_output()` | ✅ | ✅ | `snp list --search-output`, MCP `search_output: true` |

Every surface builds `SearchFields` through these named constructors — never
through a struct literal. The default path is `fuzzy_default()`; the opt-in
path (`snp list --search-output`, MCP `search_output: true`) is
`with_output()`.

`output` is bounded for scoring via `OutputPresentation::for_scoring()`
(512 chars) — do not score raw `snippet.output`.

### Adding a searchable field — the checklist

1. Add the field to `SearchFields` (not to `searchable_text` directly).
2. Decide its default. **Changing a default silently changes three surfaces.**
3. Add the opt-in surface: a CLI flag (`snp list --search-output`) and the MCP
   `search_output` parameter must move together.
4. Extend `tests/selector_search_parity.rs` — this is the only thing that
   catches a regression. **The parity suite gates the build**: it runs in
   `scripts/check.sh` (`--features test-support`), so the CI workflows
   inherit the gate through that same script. A regression that breaks
   parity now fails CI — but there is still **no compiler error** when
   parity breaks.
5. Confirm the field is not in the never-searchable list above.

---

## Resolver split: mutating vs side-effect-free

`src/selector.rs` exposes two resolvers. **Picking the wrong one is a real
correctness bug**, not a style issue.

| Function | Effect | Use for |
|----------|--------|---------|
| `resolve_selector(&selector)` (`src/selector.rs:736`) | Constructs `LibraryManager`, calls `ensure_library_mode()` — **can migrate, create, and rewrite metadata** | Commands that already own recovery/gating |
| `resolve_selector_readonly(&selector)` (`src/selector.rs:678`) | Uses `readonly_library_sources()` — no migration, no directory creation, no metadata rewrite | Read-only commands, MCP, inspection |
| `resolve_exact_target(...)` (`src/selector.rs:659`) | Builds an exact selector and delegates to the **mutating** resolver | Mutating callers needing ID/description/command matching |

- MCP is read-only, so it must never reach the mutating `resolve_selector`.
  Only `snippet_get` calls `resolve_selector_readonly` directly
  (`src/mcp/tools.rs:150`); `snippets_list` and `snippets_search` go
  `load_snippets()` (`src/mcp/tools.rs:224`) → `library_sources()`
  (`src/mcp/tools.rs:243`) → `readonly_library_sources()`. Both routes are
  read-only, but they are not the same route.
- Guards: `tests/readonly_no_recovery.rs` and
  `tests/readonly_library_resolution.rs` assert the readonly path performs no
  recovery. Both are **serial-ish and feature-gated** — check
  `scripts/check.sh` before assuming CI ran them.
- `ensure_library_mode()` is the mutating step. If you find yourself wanting it
  on a read path, that is the bug, not the fix.

---

## Library scope: two paths, don't mix

`"all"` has **two** independent definitions — do not assume one covers both
paths:

- **Selector path** — `LibraryScope::from_filter_arg`
  (`src/selector.rs:65-71`) → `LibraryScope::AllLibraries`. Used by `snp get`,
  exact-mode `run`/`clip`/`edit`, and MCP `snippet_get`.
- **Read-only library path** — `LibraryManager::resolve_readonly_sources`
  (`src/library/manager.rs:295-336`) matches the literal `"all"` itself, in
  three separate arms (`:303`, `:313`, `:325`). Used by `snp list`,
  `load_readonly_snippets`, and MCP `snippets_list` / `snippets_search`.

They agree today, but they are separate code — changing one does not change
the other. (The `from_filter_arg` doc comment calls itself "the single
definition of `"all"` handling"; that is true only of the selector path.)

- **Selector path** understands `"all"`. Used by `snp get`, exact-mode
  `run`/`clip`/`edit`, and MCP `snippet_get`.
- **Read-only library path** understands `"all"` independently. Used by
  `snp list`, `load_readonly_snippets`, and MCP `snippets_list` /
  `snippets_search`.
- **`commands::get_library_path`** (`src/commands/mod.rs:92`) does **not**:
  it calls `get_library_by_filename("all")` and returns `library_not_found`.
  Used by the single-library mutating commands (`snp new`, `snp edit`).
- **`snp search` rejects `"all"` on purpose.** Its interactive selector edits
  and deletes within a single library file, so it cannot take a cross-library
  scope (`src/commands/search_cmd.rs:37-44`). It returns an error naming the
  supported alternatives (`snp list --library all`, `snp get --library all`).

So `snp get --library all` and `snp list --library all` work, and
`snp search --library all` is a deliberate, accurate error. `all` is also a
**reserved library name** — `validate_library_name` rejects it
(`src/library/model.rs:247-252`), so `snp library create all` fails.

Read-only commands must resolve through `commands::load_readonly_snippets`
(`src/commands/mod.rs:170-190`) → `library::readonly_library_sources`, never
through the mutating `get_library_path`; `ensure_library_mode()` must not run
on a read path.

See [`../docs/LIBRARY_SCOPE.md`](../docs/LIBRARY_SCOPE.md) for the
authoritative scope table.

---

## MCP surface (read-only, stdio-only, non-executing)

`snp mcp serve` speaks newline-delimited JSON-RPC 2.0 over stdio.

### Hard boundaries — do not erode

- **stdio only.** No HTTP, no socket, no daemon, no background thread that
  outlives the process. Advertised capabilities are exactly `{"tools":{}}`
  (`src/mcp/protocol.rs:239`).
- **Read-only.** `src/mcp/tools.rs` only resolves and serializes. There is no
  spawn, no exec, no write path, no mutation. If you add one, you have broken
  the contract that makes this surface safe to expose to an agent.
- **Non-executing.** A snippet's `command` is returned as text; it is never run
  and never shell-expanded.
- **stdout is protocol-only.** All diagnostics go to stderr, or they corrupt the
  JSON-RPC stream.
- **1 MiB request bound** — `MAX_MESSAGE_BYTES` (`src/mcp/protocol.rs:15`).
- Protocol revisions: `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`.
- Methods: `initialize`, `notifications/initialized`, `ping`, `tools/list`,
  `tools/call`. EOF shuts down cleanly.

### Tools (exactly three — `src/mcp/protocol.rs:324-326`)

| Tool | Parameters |
|------|-----------|
| `snippets_list` | `library`, `limit` (1–1000, default 100) |
| `snippets_search` | `query` (required), `library`, `limit`, `tags`, `search_output` |
| `snippet_get` | exactly one of `id` (case-**sensitive**) / `description` / `command` (case-**insensitive**), plus `library` |

`snippet_get` enforces exactly-one via a JSON-Schema `oneOf`
(`src/mcp/protocol.rs:292`). `search_output` here is the MCP spelling of
`snp list --search-output` — same `SearchFields::with_output()`.

### MCP ↔ CLI parity obligations

1. `snippets_search` uses `searchable_text` **and** `score_fuzzy_matches` —
   the same fields and the same match primitive as `get --query` and
   `list --filter`.
2. `snippet_get` matching uses the same case rules as `snp get` flags.
3. MCP must not gain a tool that mutates or executes.

Contract of record: [`../docs/MCP.md`](../docs/MCP.md).
Deep dive: [`../architecture/mcp.md`](../architecture/mcp.md),
[`../architecture/selector.md`](../architecture/selector.md).

---

## Tests that protect this

| Target | Protects |
|--------|----------|
| `tests/selector_search_parity.rs` | field-membership and match parity across all three surfaces (`get --query`, `list --filter`, MCP `snippets_search`); runs in `scripts/check.sh`, so it gates the build |
| `tests/selector_integration.rs` | selector matching and ambiguity behavior |
| `tests/mcp_integration.rs` | tool dispatch, params, read-only boundary |
| `tests/readonly_no_recovery.rs` | readonly resolver performs no recovery (feature-gated) |
| `tests/readonly_library_resolution.rs` | readonly library source resolution (feature-gated) |
| `src/selector.rs` unit tests | `LibraryScope`, `SearchFields`, `exact_selector` |

```bash
cargo test --test selector_search_parity   # also runs in scripts/check.sh
cargo test --test mcp_integration
cargo test --features test-support --test readonly_no_recovery -- --test-threads=1
```

`readonly_*` targets gate compilation on `test-support`; `set_var` in these tests
needs `unsafe` (edition 2024).

---

## Common mistakes

- Adding a field to `searchable_text` without a `SearchFields` flag — it
  becomes searchable everywhere, including in MCP.
- Turning `include_output` on by default "because it's more useful" — that
  silently widens two other surfaces.
- Using `resolve_selector` in an MCP or inspection path — reintroduces recovery
  side effects into a read-only surface.
- Making MCP search folders or `favorite` — those are private/local metadata
  and are never searchable by contract.
- Writing a diagnostic to stdout in the MCP server — corrupts the protocol.
- Assuming `search --library all` should work because `get --library all` does.
  `snp search` deliberately rejects it (the selector operates on one library file).
- Treating an empty query/filter as "match nothing". An empty query is
  **"no filter"** on all three surfaces — it matches every non-deleted
  snippet, including snippets whose searchable text is empty.
- Assuming `snp get --query --library all` and `snp list --library all
  --filter` must produce the same **order** across libraries. They use
  different order families by design (selector family vs merged-set
  family); the two coincide only within a single library.

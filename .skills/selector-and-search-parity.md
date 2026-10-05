# Selector, Search Parity & MCP Skill

## Purpose
Guide agents through `src/selector.rs` (deterministic non-TUI resolution),
`src/mcp/` (read-only stdio MCP server), and the **search-parity contract**
that binds `snp get --query`, `snp list --filter`, and MCP `snippets_search`
to one function.

This is the highest-risk-to-break, lowest-visibility invariant in the repo.
There is **no compiler error** when parity breaks and often **no failing
test** unless you happen to run the right target. Read this before touching
search, filters, or any new consumer of snippet text.

---

## The parity contract (one function, three consumers)

`selector::searchable_text(snippet, fields)` (`src/selector.rs:135`) is the
single source of truth for *which fields participate* in a search. Deleted
filtering, fuzzy scoring, and ranking live with the callers; this helper only
decides field membership, precisely so the three surfaces cannot drift.

| Field | Always searched | Conditional |
|-------|-----------------|-------------|
| `description` | yes | — |
| `command` | yes | — |
| `tags` | — | `include_tags` (default **true**) |
| `output` / notes | — | `include_output` (default **false**) |
| `folders`, `favorite` | **never** | — |
| sync metadata (`id`, `device_id`, timestamps, `deleted`) | **never** | — |
| credentials / keychain data | **never** | — |

### `SearchFields` presets (`src/selector.rs:91-125`)

| Constructor | tags | output | Used by |
|-------------|------|--------|---------|
| `SearchFields::default()` | ✅ | ❌ | generic default |
| `description_and_command()` | ❌ | ❌ | historical minimum only |
| `fuzzy_default()` | ✅ | ❌ | alias of `default()` |
| `with_output()` | ✅ | ✅ | `snp list --search-output`, MCP `search_output: true` |

`output` is bounded for scoring via `OutputPresentation::for_scoring()`
(512 chars) — do not score raw `snippet.output`.

### Adding a searchable field — the checklist

1. Add the field to `SearchFields` (not to `searchable_text` directly).
2. Decide its default. **Changing a default silently changes three surfaces.**
3. Add the opt-in surface: a CLI flag (`snp list --search-output`) and the MCP
   `search_output` parameter must move together.
4. Extend `tests/selector_search_parity.rs` — this is the only thing that
   catches a regression.
5. Confirm the field is not in the never-searchable list above.

---

## Resolver split: mutating vs side-effect-free

`src/selector.rs` exposes two resolvers. **Picking the wrong one is a real
correctness bug**, not a style issue.

| Function | Effect | Use for |
|----------|--------|---------|
| `resolve_selector(&selector)` (`src/selector.rs:681`) | Constructs `LibraryManager`, calls `ensure_library_mode()` — **can migrate, create, and rewrite metadata** | Commands that already own recovery/gating |
| `resolve_selector_readonly(&selector)` (`src/selector.rs:623`) | Uses `readonly_library_sources()` — no migration, no directory creation, no metadata rewrite | Read-only commands, MCP, inspection |
| `resolve_exact_target(...)` (`src/selector.rs:604`) | Builds an exact selector and delegates to the **mutating** resolver | Mutating callers needing ID/description/command matching |

- MCP is read-only, so it must use `resolve_selector_readonly`.
- Guards: `tests/readonly_no_recovery.rs` and
  `tests/readonly_library_resolution.rs` assert the readonly path performs no
  recovery. Both are **serial-ish and feature-gated** — check
  `scripts/check.sh` before assuming CI ran them.
- `ensure_library_mode()` is the mutating step. If you find yourself wanting it
  on a read path, that is the bug, not the fix.

---

## Library scope: two paths, don't mix

`"all"` is defined exactly once — `LibraryScope::from_filter_arg`
(`src/selector.rs:61-68`) → `LibraryScope::AllLibraries`.

- **Selector path** understands `"all"`. Used by `snp get`.
- **`commands::get_library_path`** (`src/commands/mod.rs:91`) does **not**:
  it calls `get_library_by_filename("all")` and returns `library_not_found`.
  Used by `snp list`, `snp new`, `snp edit`.

So `snp get --library all` works and `snp list --library all` errors. That is
current intended behavior, documented in
[`../docs/LIBRARY_SCOPE.md`](../docs/LIBRARY_SCOPE.md). Do not "fix" the error
by adding `"all"` to the single-library path — that forks the rule.

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

1. `snippets_search` uses `searchable_text` — same fields as `get --query`.
2. `snippet_get` matching uses the same case rules as `snp get` flags.
3. MCP must not gain a tool that mutates or executes.

Contract of record: [`../docs/MCP.md`](../docs/MCP.md).
Deep dive: [`../architecture/mcp.md`](../architecture/mcp.md),
[`../architecture/selector.md`](../architecture/selector.md).

---

## Tests that protect this

| Target | Protects |
|--------|----------|
| `tests/selector_search_parity.rs` | field-membership parity across the three surfaces |
| `tests/selector_integration.rs` | selector matching and ambiguity behavior |
| `tests/mcp_integration.rs` | tool dispatch, params, read-only boundary |
| `tests/readonly_no_recovery.rs` | readonly resolver performs no recovery (feature-gated) |
| `tests/readonly_library_resolution.rs` | readonly library source resolution (feature-gated) |
| `src/selector.rs` unit tests | `LibraryScope`, `SearchFields`, `exact_selector` |

```bash
cargo test --test selector_search_parity
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
- Assuming `list --library all` should work because `get --library all` does.

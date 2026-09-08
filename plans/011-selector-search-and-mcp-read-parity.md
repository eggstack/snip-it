# Plan 011: Selector, search, and MCP read parity

Status: ready

Depends on: Plans 009–010

## Objective

Make snippet matching and read-only retrieval semantics consistent across CLI commands, TUI-facing flows, and MCP without expanding MCP into mutation or execution.

This is the primary feature-depth plan from the architecture review. It improves existing snippet organization/search capabilities rather than adding another subsystem.

## Audit findings

The CLI exposes overlapping selector inputs across `run`, `clip`, `edit`, `get`, `search`, `select`, and `list`: exact UUID, exact description, exact command, fuzzy query/filter, library scope, resolution policy, sorting, favorites, and optional output/notes search.

A `selector` module already exists, but command argument and matching policy remain partially repeated.

MCP currently provides read-only list/search/get, but its search path fuzzy-matches only description plus command. It does not have parity with existing tag/output metadata or the richer deterministic selector behavior available to the CLI.

## Scope

Expected files:

```text
src/selector.rs
src/sort.rs
src/commands/get_cmd.rs
src/commands/list_cmd.rs
src/commands/clip_cmd.rs
src/commands/edit_cmd.rs
src/commands/** selection helpers
src/mcp/tools.rs
src/mcp/protocol.rs or schemas if tool arguments change
docs/MCP.md
USER_GUIDE.md
architecture/cli.md
tests/**
```

## Non-goals

Do not:

- add MCP command execution;
- add MCP snippet create/edit/delete;
- bind an MCP network port;
- add a database or search index;
- replace the existing fuzzy matcher;
- add regex/full-text-query languages;
- redesign the TUI;
- introduce a generic query engine;
- make all commands expose every selector flag when the operation does not need it.

## Execution steps

### 1. Define one internal selection/query specification

Create or extend a concrete internal type representing the matching inputs that are genuinely shared, such as:

- library scope;
- exact ID;
- exact description;
- exact command;
- fuzzy query;
- resolution policy.

Keep sorting/filter presentation options separate when they are not part of identity resolution.

The type should be ordinary Rust data passed into existing selector functions, not a trait hierarchy.

### 2. Centralize exact matching semantics

Ensure case-sensitivity, deleted-snippet filtering, ambiguity behavior, `all` library behavior, and deterministic ordering are defined once.

Migrate `get`, exact-mode `run`/`clip`, and edit selection to the shared path where behavior is equivalent. Preserve command-specific handling after a snippet is selected.

### 3. Centralize searchable text construction

Define one helper for which fields participate in fuzzy search under a given option set. At minimum preserve current CLI behavior and support:

- description;
- command;
- tags;
- output/notes when explicitly enabled or when the consuming interface's documented search contract includes it.

Do not silently make sensitive/unexpected metadata searchable if current user-facing behavior excludes it; document the exact field contract.

### 4. Improve MCP search within read-only scope

Extend MCP search arguments only where useful and stable. High-value additions are:

- tag-aware search using the canonical searchable-text helper;
- optional output/notes search if CLI semantics already support it;
- library scope using Plan 009's canonical read-only resolver;
- deterministic limits and ranking consistent with CLI relevance sorting.

If adding explicit tag filters is simpler and more predictable than implicit tag text matching, prefer a small `tags` argument over a broad query language.

### 5. Improve MCP deterministic get parity

Where safe, allow MCP `get` to use the same exact identity fields as CLI `get` (ID, exact description, and exact command) and the same ambiguity semantics. Preserve structured `not_found`/`ambiguous` results or documented protocol errors consistently.

Do not add variable expansion that prompts interactively. Any expansion exposed to MCP must be deterministic and noninteractive; otherwise leave stored command retrieval unchanged.

### 6. Preserve CLI/TUI ergonomics

Do not force users to adopt a new syntax. Existing flags and aliases remain valid. This plan should primarily replace internal matching code and add read-only MCP parity.

### 7. Test parity explicitly

Create fixture-driven tests where the same library produces equivalent identities/order for equivalent CLI-core and MCP queries, including:

- exact ID;
- exact description with duplicate ambiguity;
- exact command;
- fuzzy description/command;
- tag matching/filtering;
- output/notes behavior when enabled;
- deleted snippets excluded;
- cross-library scope;
- stable limit/ranking behavior.

Run:

```bash
cargo check --workspace --all-targets
cargo test --workspace
bash scripts/check.sh
```

## Acceptance criteria

Plan 011 is complete only when:

1. Shared exact selector semantics have one canonical implementation.
2. CLI commands that currently duplicate exact matching reuse that implementation where behavior is equivalent.
3. Searchable-field construction is centralized rather than separately assembled by MCP and CLI search paths.
4. MCP search can use existing organizational metadata such as tags, with output/notes parity where explicitly supported.
5. MCP deterministic get semantics align with CLI exact matching for supported fields.
6. MCP remains read-only, stdio-only, non-executing, and noninteractive.
7. No database/index/query-language subsystem is introduced.
8. Existing CLI syntax and TUI behavior remain compatible.
9. Fixture tests demonstrate CLI-core/MCP result parity for representative cases.
10. `docs/MCP.md` documents the final tool schemas and matching fields.
11. `bash scripts/check.sh` passes.
12. `plans/README.md` is updated in the implementation commit.

## Handoff note

Favor parity by reusing core selector/search helpers, not by copying CLI behavior into MCP. The MCP layer should remain a thin JSON/protocol adapter around safe read-only core operations.
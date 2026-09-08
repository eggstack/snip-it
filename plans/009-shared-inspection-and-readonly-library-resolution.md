# Plan 009: Shared inspection and read-only library resolution

Status: complete

Depends on: Plan 008

## Objective

Create small, side-effect-free core primitives for library discovery and state inspection so `doctor`, `validate`, `repair`, `status`, CLI retrieval, and MCP stop encoding parallel definitions of the same repository state.

This plan consolidates policy. It does not merge the user-facing commands or build a generic diagnostics framework.

## Audit findings

`doctor`, `validate`, `repair`, and `status` overlap conceptually because they inspect many of the same configuration, library, sync, and filesystem conditions but present different user semantics.

Separately, `src/mcp/tools.rs` implements its own read-only library source resolution because `LibraryManager::ensure_library_mode` can migrate/write legacy state. The MCP choice is correct—read-only operations must not mutate—but it leaves legacy single-file handling, primary-library resolution, and path construction duplicated outside the canonical library layer.

## Scope

Expected files:

```text
src/library.rs
src/diagnostics.rs
src/status_snapshot.rs
src/commands/doctor_cmd.rs
src/commands/validate_cmd.rs
src/commands/repair_cmd.rs
src/commands/status_cmd.rs
src/commands/get_cmd.rs
src/mcp/tools.rs
src/error.rs
tests/**
```

Exact command filenames may differ; use the current tree rather than creating parallel modules unnecessarily.

## Non-goals

Do not:

- replace doctor/validate/repair/status with one command;
- invent a trait-based diagnostics engine;
- create a generic rule/plugin registry;
- make read-only operations call migration or repair code;
- change library on-disk formats;
- alter repair policy or automatically repair new classes of finding;
- add new diagnostic categories merely because a shared representation exists;
- expose MCP mutations.

## Execution steps

### 1. Introduce canonical read-only library source resolution

Add a side-effect-free API in the library layer that can answer the questions currently reimplemented by MCP:

- what libraries are visible in legacy single-file mode;
- what the primary library is;
- how `all`, a named library, and default scope resolve;
- canonical filename/name, library ID, and path for each source.

The API must not call migration, create directories/files, rewrite metadata, or otherwise mutate state.

Use a small concrete type such as `LibrarySource`/`ResolvedLibrarySource`; do not add a repository trait.

### 2. Replace MCP-local resolution

Delete the MCP-specific source/path compatibility logic and consume the canonical read-only resolver. Preserve MCP's no-write guarantee with tests.

Where CLI `get` or other deterministic read paths duplicate the same source resolution, migrate them too if the change is direct and behavior-preserving.

### 3. Inventory duplicated inspection logic

Compare `doctor`, `validate`, `repair`, and `status_snapshot` for repeated checks such as:

- library existence/readability/parse validity;
- metadata/config consistency;
- legacy mode state;
- sync status/control artifact consistency;
- recoverable vs non-recoverable findings.

Extract only checks that are actually duplicated or encode the same policy.

### 4. Define a narrow finding model

If needed, introduce a small internal finding representation carrying only data required by more than one consumer, for example severity/category/message/repairability plus structured identity.

Avoid a generalized diagnostics DSL. Consumers should remain free to render differently:

- `doctor`: explanatory human/report output;
- `validate`: pass/fail plus warnings;
- `repair`: actionable repair candidates;
- `status`: compact current-state snapshot.

### 5. Separate inspection from mutation

Any shared inspection primitive must be read-only. Repair code may consume findings and separately perform explicit mutations under existing backup/locking/transaction rules.

A call to `doctor`, `validate`, `status`, CLI `get`, or MCP list/search/get must not cause migration or repair side effects.

### 6. Add regression tests

Add focused tests for:

- legacy single-file resolution with no writes;
- primary library resolution;
- named and `all` resolution;
- missing library errors;
- MCP read operations leaving filesystem state unchanged;
- at least one duplicated diagnostic condition producing consistent classification across doctor/validate or validate/repair.

Run:

```bash
cargo check --workspace --all-targets
cargo test --workspace
bash scripts/check.sh
```

## Acceptance criteria

Plan 009 is complete only when:

1. Read-only library source resolution has one canonical implementation in the core/library layer.
2. MCP no longer owns duplicate legacy/primary/path resolution policy.
3. Deterministic CLI read paths reuse the resolver where applicable.
4. Shared inspection logic is extracted only for genuinely overlapping conditions.
5. `doctor`, `validate`, `repair`, and `status` retain their distinct user semantics.
6. Shared inspection functions are side-effect free.
7. Read-only MCP/CLI/diagnostic calls cannot trigger legacy migration or file creation.
8. No generic plugin/rule/repository framework is introduced.
9. Focused regression tests prove compatibility and no-write behavior.
10. `bash scripts/check.sh` passes.
11. `plans/README.md` is updated in the implementation commit.

## Handoff note

The desired architecture is a few reusable functions and concrete structs, not a subsystem. If extracting a check requires more abstraction code than the duplicated logic it replaces, leave that check local.
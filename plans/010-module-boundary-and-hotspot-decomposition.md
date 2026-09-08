# Plan 010: Module boundary and hotspot decomposition

Status: complete

Depends on: Plans 008–009

## Objective

Reduce maintenance concentration in the largest source files while preserving the current crate/workspace architecture and runtime behavior.

The goal is code locality, not abstraction. Split files only where responsibilities are already distinct and stable.

## Audit findings

Current hotspots include approximately:

```text
src/library.rs            ~97 KB
src/main.rs               ~77 KB
src/commands/doctor_cmd.rs ~54 KB
src/config.rs             ~43 KB
src/commands/backup_cmd.rs ~33 KB
```

Large files are not inherently defects, but these files combine responsibilities that change for different reasons. `library.rs` is especially important because it carries public domain types plus persistence/manager behavior. `main.rs` combines CLI schema, startup/runtime policy, hidden worker commands, and dispatch.

## Scope

Primary targets:

```text
src/library.rs
src/main.rs
src/config.rs
src/commands/doctor_cmd.rs
src/commands/backup_cmd.rs
src/lib.rs
architecture/** when module maps are documented
tests/**
```

Only split a secondary hotspot when the extracted boundary is obvious from existing code.

## Non-goals

Do not:

- create additional workspace crates;
- introduce service/repository/domain-layer traits;
- redesign persistence;
- change public data formats or CLI behavior;
- rewrite modules merely to make files uniformly small;
- chase an arbitrary line-count threshold;
- split every command into many tiny files;
- add dependency-injection architecture;
- refactor sync transport in this plan.

## Execution steps

### 1. Decompose `library.rs` by existing responsibility

Prefer a small module family under `src/library/` with boundaries equivalent to:

- model/schema: `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta` and directly related validation/default helpers;
- persistence: load/save and format-safe filesystem operations;
- manager/resolution: `LibraryManager`, registry/primary-library behavior, and the read-only resolver introduced by Plan 009.

Exact filenames are flexible. Preserve existing root re-exports so downstream Rust API paths do not break unnecessarily.

Do not add interfaces around these modules. Ordinary functions and concrete structs are sufficient.

### 2. Reduce `main.rs` to CLI composition and orchestration

Move command-family `Args`/`Subcommand` definitions beside their handlers where doing so is straightforward after Plan 008. Keep `main.rs` responsible for:

- top-level parser composition;
- runtime/signal/log setup;
- command dispatch;
- final outcome/exit mapping;
- hidden binary-only updater/worker wiring where necessary.

Avoid a dynamic command registry.

### 3. Split `doctor_cmd.rs` only along consumer boundaries

If Plan 009 has produced reusable inspection primitives, remove duplicated implementation from `doctor_cmd.rs`. If the remaining file still has clearly separate concerns, separate report rendering from check orchestration.

Do not split each diagnostic check into its own module.

### 4. Split `config.rs` only where state domains are already independent

Identify whether general client/library configuration, sync configuration, and file identity/atomic configuration helpers form clean independent groups. Extract only boundaries with low cross-coupling.

Preserve current public exports from `snip_it::config` unless a semver-compatible re-export can maintain them.

### 5. Treat `backup_cmd.rs` conservatively

Only extract archive/directory serialization or manifest construction if it is independently testable and clearly separate from CLI orchestration. Do not build a backup framework or backend abstraction.

### 6. Check API and compile-time fallout

Because `snip-it` exposes a supported library surface, verify that documented public imports continue to compile. Add a small public-API smoke test if one does not already exist.

Run:

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo test --workspace
bash scripts/check.sh
```

## Acceptance criteria

Plan 010 is complete only when:

1. `library.rs` no longer combines all model, persistence, and manager/resolution implementation in one file.
2. Existing supported public library imports remain source-compatible through re-exports where needed.
3. `main.rs` is materially reduced and primarily performs top-level CLI/runtime composition rather than owning most command schemas.
4. Any `doctor`, `config`, or backup split follows an existing responsibility boundary rather than an invented abstraction.
5. No additional workspace crate, service layer, repository trait, or dependency-injection mechanism is added.
6. No user-visible CLI, persistence, sync, or output behavior changes intentionally.
7. Tests cover moved public/persistence behavior sufficiently to distinguish refactor regressions.
8. `cargo check --workspace --all-targets` and `bash scripts/check.sh` pass.
9. Architecture documentation is adjusted only if paths/module ownership described there changed.
10. `plans/README.md` is updated in the implementation commit.

## Handoff note

Measure success by fewer unrelated reasons for a file to change, not by file count. Prefer three cohesive 20–35 KB modules over fifteen tiny wrapper modules.
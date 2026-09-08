# Plan 008: CLI surface and outcome consolidation

Status: ready

Depends on: none

## Objective

Remove duplicated CLI schemas and redundant command-outcome translation without changing user-visible behavior.

This is a consolidation pass. It must reduce maintenance surface while preserving the current command names, aliases, exit-code contract, machine-output rules, and lightweight architecture.

## Audit findings

The current `Commands` enum declares top-level `Validate`, `Backup`, `Restore`, `Repair`, and `Status` commands while `DataCommands` independently declares the same command families with nearly identical arguments. This creates two Clap schemas for one semantic operation and makes drift likely.

The result path also contains overlapping enums (`ProcessResult`, `SelectionOutcome`, `CommandOutcome`, `CliOutcome`) with repeated success/cancel/execution-failure concepts. Distinctions between TUI selection and CLI exit semantics are useful, but the current middle-layer mappings should be simplified where they do not carry unique state.

## Scope

Expected files:

```text
src/main.rs
src/commands/mod.rs
src/outcome.rs
src/lib.rs
src/ui/** or selector-facing code only if needed for result conversion
tests/**
architecture/cli.md
USER_GUIDE.md or README.md only if command compatibility needs clarification
```

## Non-goals

Do not:

- remove documented top-level commands;
- introduce a new CLI framework or command registry abstraction;
- create a new crate;
- redesign command behavior;
- renumber established exit codes;
- change machine-readable output formats;
- combine all internal outcomes into one large enum merely to reduce type count;
- add new feature commands in this pass.

## Execution steps

### 1. Establish the compatibility contract

Before editing, capture `snp --help`, `snp data --help`, and the help for the duplicated command families. Identify which spellings are documented and which aliases are relied on by tests.

The target behavior is that existing invocations continue to work. Prefer one canonical argument type/handler per semantic command with aliases or forwarding for secondary spellings.

### 2. Deduplicate maintenance command argument declarations

Create one source of truth for each of:

- validate;
- backup;
- restore;
- repair;
- status.

The simplest acceptable implementation is shared Clap `Args` structs reused by top-level and `data` subcommands. Another acceptable implementation is making `data` a compatibility alias layer that dispatches into the same typed arguments.

Do not maintain two independently declared field lists after this plan.

### 3. Keep dispatch single-path

Ensure both top-level and `snp data ...` spellings enter the same handler and output path. There should be no duplicated validation, JSON formatting, exit-code mapping, or side-effect policy between the two entry points.

### 4. Simplify result translation

Trace the full path from selector/TUI operation to process exit. Preserve a distinct internal selector/execution result if it models UI-only state, and preserve `CliOutcome` as the stable public CLI result.

Remove or narrow any intermediary result enum that only mirrors another type without carrying unique information. `CommandOutcome` is the primary candidate, but implementation should confirm actual call sites before removal.

Keep conversions explicit and local. Do not replace them with generic traits or a broad error-conversion framework.

### 5. Reduce `main.rs` orchestration noise opportunistically

If shared `Args` structs naturally belong beside command handlers, move them there. Do not perform a full `main.rs` split in this plan; Plan 010 handles structural decomposition.

### 6. Verify compatibility

At minimum:

```bash
cargo fmt --check
cargo check --workspace --all-targets
cargo test --workspace
bash scripts/check.sh
```

Add focused CLI integration tests proving that top-level and `data` spellings produce equivalent behavior for at least validate, backup dry-run or safe fixture behavior, repair dry-run, and status JSON where existing fixtures make this practical.

## Acceptance criteria

Plan 008 is complete only when:

1. Each duplicated maintenance command has one canonical argument schema.
2. Top-level `validate`, `backup`, `restore`, `repair`, and `status` remain functional.
3. Existing `snp data ...` compatibility remains functional unless repository documentation explicitly establishes that one spelling is intentionally deprecated; no breaking removal is allowed in this plan.
4. Both spellings dispatch to the same command implementation.
5. Stable exit codes and machine-output behavior are unchanged.
6. Redundant outcome translation is reduced where possible without collapsing genuinely distinct TUI state into CLI state.
7. No new abstraction framework, crate, or feature surface is introduced.
8. `cargo check --workspace --all-targets` and `bash scripts/check.sh` pass.
9. Focused compatibility tests cover the deduplicated paths.
10. `plans/README.md` is updated to mark this plan complete in the same implementation commit.

## Handoff note

Prioritize literal duplication removal over aesthetic refactoring. A successful implementation should leave fewer declarations and fewer conversion branches than it started with while preserving the CLI contract exactly.
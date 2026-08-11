# Phase 14C — Command and Control-Flow Consolidation

Status: IMPLEMENTED

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Required predecessors: Phase 14A and Phase 14B

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

This phase removes duplicated command plumbing that has already caused behavioral drift. It is a consolidation pass, not a redesign.

The target is fewer independent places that encode:

- exact selector construction;
- explicit sync orchestration;
- clipboard copy side effects;
- advanced data-command dispatch;
- startup logging/recovery classification;
- command execution result bookkeeping.

The user-visible CLI, aliases, exit codes, TUI behavior, sync protocol, and persistence format must remain unchanged.

## 2. Allowed files

Primary files:

```text
src/main.rs
src/selector.rs
src/commands/mod.rs
src/commands/clip_cmd.rs
src/commands/run_cmd.rs
src/outcome.rs          # only if an existing outcome type is the cleanest reuse point
```

Tests should prefer existing unit/integration locations for selector, CLI, run, and clip behavior.

Do not touch Cargo dependencies in this phase.

## 3. Small-model rules

1. Complete one workstream at a time.
2. Run the focused tests named for that workstream before starting the next.
3. Introduce a helper only when at least two existing call sites immediately use it.
4. Do not create traits, macros, registries, command tables, dynamic dispatch, or a new command framework.
5. Prefer private functions and small structs local to `main.rs` or the owning module.
6. Preserve compatibility aliases and exact exit codes.
7. If a consolidation changes behavior, treat that as a defect and stop unless Phase 14A/14B explicitly requires the change.

## 4. Workstream A — One exact-selector construction path

### 4.1 Baseline duplication

`src/main.rs` independently constructs the same `SnippetSelector` shape for exact run, exact clip, and exact output edit:

```text
library scope
+ ResolutionPolicy::Unique
+ optional --id
+ optional --description-exact
+ optional --command-exact
+ resolve_selector()
```

This duplication increases the chance that one command forgets a selector field, output mapping, or future compatibility rule.

### 4.2 Preferred implementation

Create one private helper. Two acceptable locations:

- `src/main.rs` if it is only CLI assembly;
- `src/selector.rs` if the helper naturally belongs to selector semantics and can remain implementation-only.

Conceptual shape:

```rust
fn resolve_exact_target(
    library: Option<&str>,
    id: Option<String>,
    description_exact: Option<String>,
    command_exact: Option<String>,
) -> SnipResult<SelectionResult>
```

Do not create a public builder wrapper merely to save a few lines.

### 4.3 Call sites

Migrate at minimum:

- exact `Run`;
- exact `Clip`;
- exact output-edit path.

Keep command-specific handling of `One`, `Ambiguous`, and `NotFound` outside the helper unless the handling is genuinely identical.

### 4.4 Acceptance

- [ ] Selector field mapping exists in one place.
- [ ] Run, clip, and edit use it.
- [ ] Ambiguous output still lists identities exactly as before.
- [ ] Not-found/ambiguous exit codes remain unchanged.

## 5. Workstream B — Finish explicit-sync consolidation

Phase 14A must introduce the canonical explicit-sync helper. Phase 14C must remove remaining duplicate implementations inside `run_snippet_selection()`.

Current duplicate contexts include:

- delete from the TUI;
- selected/processed snippet post-action sync;
- exact run;
- exact clip.

All must use one helper for:

```text
execution lock
-> pending generation observation
-> canonical sync
-> success/failure classification
-> generation-safe pending clear
```

The operation-specific code should decide only *whether* explicit sync is requested and when the local action has reached the point where sync may run.

Do not move the network sync implementation into `commands/mod.rs`; reuse `sync_commands::run_default_sync`.

### Acceptance

- [ ] There is one execution-lock/pending-clear sequence for explicit sync.
- [ ] TUI delete no longer carries its own copy.
- [ ] TUI post-selection no longer carries its own copy.
- [ ] Exact run/clip remain on the same helper from Phase 14A.
- [ ] Fresh pending generations cannot be cleared by an older sync attempt.

## 6. Workstream C — One clipboard operation implementation

### 6.1 Baseline duplication

`clip_cmd::process_snippet()` and `clip_cmd::run_exact()` both perform:

1. variable expansion;
2. clipboard copy;
3. audit logging;
4. usage-index update/save.

They differ mostly in return shape.

### 6.2 Preferred implementation

Extract one private operation function, for example:

```rust
fn copy_snippet(snippet: &Snippet) -> SnipResult<ProcessResult>
```

Use it from both TUI callback and exact command entry point.

The exact wrapper may map `Cancel`/`Continue`/`Done` to its existing CLI outcome, but must not repeat clipboard/audit/usage code.

Do not create a generic "snippet action" framework shared with run/search/select.

### 6.3 Correctness detail

Preserve usage recording only after successful copy. If current exact/TUI paths differ on ignored usage-save errors, normalize to one existing policy and add a focused test/log assertion if practical.

### Acceptance

- [ ] Variable expansion for clip exists in one operation path.
- [ ] Clipboard write/audit/usage update exist in one operation path.
- [ ] Exact and TUI clip cancellation behavior remains correct.
- [ ] Phase 14A `--sync` parity remains intact.

## 7. Workstream D — Reduce duplicated run result bookkeeping

`run_cmd.rs` has separate output-file and normal-execution branches that duplicate successful audit logging, usage recording, result conversion for tracing, and command-execution logging.

After `spawn_and_wait_execution()` returns, move common post-result bookkeeping into one private helper.

Conceptual shape:

```rust
fn record_execution_result(
    snippet: &Snippet,
    final_command: &str,
    result: &ProcessResult,
    working_dir: Option<&Path>,
)
```

Do not merge output-file creation/path validation with ordinary execution. Those branches have distinct safety semantics and should stay separate.

### Acceptance

- [ ] Success audit/usage logic is not duplicated between execution branches.
- [ ] Output-path containment and `create_new` behavior are unchanged.
- [ ] Exit-code mapping is unchanged.

## 8. Workstream E — One dispatcher for canonical and compatibility data commands

The root CLI intentionally exposes both:

```text
snp validate|backup|restore|repair|status
snp data validate|backup|restore|repair|status
```

The aliases are part of compatibility; keep them. The implementation mapping should not be duplicated.

Create one private dispatch helper for `DataCommands` and map legacy top-level commands into the same command invocation/result mapping.

A simple approach is to construct a `DataCommands` value from the legacy variant and call the helper.

Do not change clap command names or deprecate the legacy forms in this phase.

### Acceptance

- [ ] Repair exit mapping exists once.
- [ ] Backup/restore/validate/status data dispatch exists once.
- [ ] Legacy and canonical spellings produce identical output/exit behavior.

## 9. Workstream F — One startup command-behavior classification

### 9.1 Baseline

`classify_command()` determines `StartupRecoveryPolicy` while `startup_services()` independently matches most of the same `Commands` variants to decide logging/audit startup.

This is an overlapping taxonomy that can drift.

### 9.2 Preferred implementation

Replace the two broad matches with one private classification result:

```rust
struct CommandBehavior {
    recovery: StartupRecoveryPolicy,
    services: StartupServices,
}

fn command_behavior(cmd: Option<&Commands>) -> CommandBehavior
```

Keep `StartupServices` and `StartupRecoveryPolicy` as existing enums unless there is a direct simplification available.

One match over the CLI enum should assign both properties.

Do not generalize this into command metadata tables or procedural macros.

### 9.3 Testing

Use table-driven tests by representative command classes rather than one test per trivial enum branch.

Required categories:

- minimal read-only;
- logging-only explicit sync/config command;
- logging+audit mutation;
- read-only `Data` subcommand;
- mutating `Data` subcommand;
- dry-run import/restore/repair;
- internal auto-sync worker;
- default/no-subcommand behavior.

### Acceptance

- [ ] Every command receives logging/audit and recovery policy from one match.
- [ ] Read-only commands still avoid recovery/network side effects.
- [ ] Mutations still allow pending recovery.
- [ ] Explicit sync commands still suppress startup auto-sync recovery.

## 10. Workstream G — Remove obsolete auto-sync command tag API only if unused

Phase 14 review found both:

```text
SubcommandTag + should_attempt_auto_sync_recovery()
StartupRecoveryPolicy + should_attempt_auto_sync_recovery_for_policy()
```

The binary uses the policy API.

Before deletion, search the repository for real production/test callers of `SubcommandTag` and `should_attempt_auto_sync_recovery`.

If they are only legacy unit-test/self-test callers, remove the obsolete pair and replace any meaningful test with the policy equivalent.

If an actual compatibility surface still depends on them, leave them and record why. Do not add another compatibility wrapper.

## 11. Focused verification sequence

After each workstream:

```text
A: selector/CLI exact-target tests
B: explicit-sync focused tests from Phase 14A
C: clip command tests
D: run command tests
E: data command/exit-code tests
F/G: main/auto-sync policy unit tests
```

Then:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

## 12. Non-goals

Do not:

- redesign clap enums;
- convert all commands to traits;
- split `main.rs` merely to achieve a file-size target;
- expose new public Rust APIs;
- alter sync retry policy;
- change persistence formats;
- remove CLI compatibility aliases;
- merge unrelated run/clip/select/search semantics into one generic executor.

## 13. Final acceptance criteria

- [ ] Exact selector construction is canonicalized.
- [ ] Explicit-sync orchestration is canonicalized.
- [ ] Clipboard copy side effects are canonicalized.
- [ ] Run post-execution bookkeeping is reduced without safety changes.
- [ ] Legacy/canonical data command dispatch shares one implementation.
- [ ] Startup recovery and logging/audit classification come from one command match.
- [ ] Obsolete `SubcommandTag` API is removed if truly unused.
- [ ] No new framework, crate, dependency, or public API is introduced.
- [ ] Phase 14A and 14B regression tests remain passing.
- [ ] `bash scripts/check.sh` passes.

## 14. Suggested implementation commit

```text
phase-14c: consolidate command and sync control flow
```

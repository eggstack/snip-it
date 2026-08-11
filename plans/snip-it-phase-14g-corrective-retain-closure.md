# Phase 14G Corrective Pass — Restore Proven Transaction Recovery and Close Phase 14

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Supersedes the implementation decision recorded by commit `29fda50faf3e84538964e73bf18f42c8999e6b05`.

Reviewed repository head: `29fda50faf3e84538964e73bf18f42c8999e6b05`

Known-good pre-14G implementation baseline: `11d677a8ff0d3850b68e1b19d71d06dabcf782c2`

Date: 2026-08-11

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

This is a narrow corrective pass for the Phase 14G transaction-boundary decision.

Phases 14A through 14F landed in the intended direction and must remain intact. The problem is isolated to the Phase 14G `SIMPLIFY` implementation in commit `29fda50`: it introduced a second marker-based recovery model while retaining the old transaction engine for compatibility, but the replacement does not preserve enough information or validation to guarantee coherent rollback after interruption.

The corrective decision is:

```text
RETAIN
```

Restore the proven transaction-journal implementation that existed at `11d677a8`, record Phase 14G as a deliberate RETAIN decision, normalize the stale Phase 14 planning records, and run one final verification pass.

Do not harden the new `InterruptedOperation` marker until it becomes another transaction engine. Do not invent a third persistence/recovery design. Do not reopen Phase 14A through 14F.

## 2. Why the current 14G implementation must be corrected

The current marker-based path has several correctness gaps.

### 2.1 The marker cannot distinguish created files from lost backups

`restore_cmd.rs` knows whether each destination existed before restore and whether the intended action is `Create`, `Replace`, `Delete`, or `NoOp`.

The `InterruptedOperation` marker does not preserve that distinction. It stores parallel `affected_paths` and `backup_paths`, using an empty path when no backup exists.

During recovery this is ambiguous:

```text
no backup + destination exists
```

can mean either:

```text
A. destination did not exist before restore and must be removed during rollback
B. destination existed before restore but its required backup is missing
```

Those cases require opposite behavior. The current recovery code cannot tell them apart.

### 2.2 Recovery may clear evidence while state remains partially restored

The new recovery path logs a warning when a destination exists but no backup is available, then continues cleanup and removes the marker.

Therefore an interrupted restore can leave a newly-created or otherwise unrecovered destination in place while `snp repair` clears the recovery evidence.

This violates the Phase 14G minimum guarantee:

> A crash must not silently leave damaged or partial local state looking resolved.

### 2.3 The new marker recovery does not carry forward the old containment checks

The legacy transaction engine validates transaction IDs, artifact roots, backup paths, durable staged paths, symlinks, lexical traversal, and out-of-root references before recovery operations.

The new marker path directly consumes marker-provided paths for:

```text
backup reads
destination replacement
artifact directory removal
```

without equivalent containment validation.

Do not fix this by duplicating the entire validation layer into the marker implementation. That would defeat the purpose of simplification.

### 2.4 Parallel-vector cardinality is not validated

The marker uses separate vectors for:

```text
affected_paths
backup_paths
original_metadata
```

Recovery iterates with `zip`, so a malformed/truncated marker can silently omit affected paths from recovery.

Again, do not solve this by growing the marker schema. Restore the already-correct transaction model instead.

### 2.5 Marker repair does not use the same transaction-lock authority

Legacy `recover_transaction_by_id()` acquires the transaction lock before loading/classifying the journal and holds it through recovery.

The new `recover_interrupted_operation()` path is called directly by repair and does not provide the same lock-scoped recovery contract.

### 2.6 The intended simplification did not occur

The Phase 14G plan allowed SIMPLIFY only if the old state-machine machinery could be materially removed after callers migrated.

Instead, current `src/transaction.rs` still contains the legacy:

```text
TransactionJournal
TransactionState
recovery classification
restartable commit/rollback/cleanup machinery
legacy journal scanning
artifact validation
large compatibility test surface
```

and the new marker model was added alongside it.

The result is two recovery models rather than one smaller model. That fails the Phase 14G decision rule and increases maintenance burden.

## 3. Corrective decision: RETAIN

Phase 14G must be changed from `SIMPLIFY` to `RETAIN`.

This is not a statement that the existing transaction engine is ideal or minimal in isolation. It is a scope decision based on the actual repository state:

- the old implementation already exists;
- it has dedicated crash/recovery tests;
- it already handles old on-disk journals;
- restore is the only meaningful multi-file caller;
- the replacement cannot become correct without restoring much of the state/validation it attempted to remove;
- retaining one mature recovery model is simpler than maintaining old compatibility plus a second incomplete model.

After this corrective pass, transaction simplification is closed for Phase 14. Do not create a Phase 14H to revisit it.

## 4. Scope guardrails

### 4.1 Must preserve

Do not alter the Phase 14A through 14F implementation work, including:

- explicit native keyring backend features;
- canonical explicit-sync behavior for exact and TUI paths;
- fail-closed malformed library/index TOML behavior;
- deterministic legacy snippet IDs;
- command/control-flow consolidation;
- `arboard` image-feature removal;
- narrowed root Tonic client features;
- narrowed `tracing-subscriber` features;
- synchronous low-volume audit logging;
- reduced macOS/Windows CI duplication;
- Phase 14F test consolidation and release-check placement.

### 4.2 Must restore

Restore the transaction/restore/recovery production behavior that existed at commit:

```text
11d677a8ff0d3850b68e1b19d71d06dabcf782c2
```

for the files changed by Phase 14G, subject only to later unrelated changes if any have landed after this plan was written.

### 4.3 Must not add

Do not add:

- a strengthened or version-2 `InterruptedOperation` marker;
- new marker fields to emulate `StagedFile`;
- another transaction/recovery abstraction;
- SQLite/WAL/database persistence;
- a daemon/background repair service;
- new locks;
- new recovery queues;
- new crash-test infrastructure;
- another CI lane;
- generalized path-validation frameworks;
- new dependencies.

The preferred code change is deletion/reversion, not new architecture.

## 5. Required preflight

Before editing, run:

```text
git status --short
git log --oneline --decorate -12
git show --stat --oneline 29fda50faf3e84538964e73bf18f42c8999e6b05
git diff 11d677a8ff0d3850b68e1b19d71d06dabcf782c2..29fda50faf3e84538964e73bf18f42c8999e6b05 -- \
  src/transaction.rs \
  src/commands/restore_cmd.rs \
  src/commands/repair_cmd.rs \
  src/test_failpoints.rs \
  tests/destination_permissions.rs
```

Expected baseline when this plan was written:

```text
29fda50 phase-14g: simplify multi-file recovery to fail-closed repair
11d677a phase-14f: reduce routine verification and CI duplication
```

If `29fda50` is still the direct implementation head with no later production changes touching the same files, a normal revert of that implementation commit is preferred.

If later commits have landed, do not blindly revert over unrelated work. Revert only the Phase 14G changes by comparing the affected files against `11d677a8` and preserving later unrelated modifications.

## 6. Workstream A — Remove the marker-based production path

### Goal

Return to one transaction/recovery implementation.

### Required removals

Production code must no longer contain the Phase 14G marker model introduced by `29fda50`, including concepts equivalent to:

```text
InterruptedOperation
interrupted-operation.toml
write_interrupted_operation
read_interrupted_operation
remove_interrupted_operation
rollback_interrupted_operation
recover_interrupted_operation
RecoverInterruptedOperation
rollback_from_marker
fixed transaction artifact ID "marker"
```

The global mutation gate must return to the journal-based behavior from the Phase 14F baseline.

### Preferred implementation

When safe relative to current history:

```text
git revert --no-edit 29fda50faf3e84538964e73bf18f42c8999e6b05
```

Then continue with the documentation/status corrections below in a follow-up commit.

If a direct revert conflicts because of later work, manually restore only the Phase 14G production/test/docs changes. Do not revert Phase 14A through 14F commits.

### Acceptance criteria — Workstream A

- [ ] No normal production path creates `interrupted-operation.toml`.
- [ ] `InterruptedOperation` is absent from production code.
- [ ] `RepairAction::RecoverInterruptedOperation` is absent.
- [ ] `restore_cmd.rs` no longer uses `rollback_from_marker`.
- [ ] Recovery does not consume unvalidated arbitrary marker-provided filesystem paths.
- [ ] The transaction gate uses the single journal/recovery model again.
- [ ] No Phase 14A–14F production behavior is reverted.

## 7. Workstream B — Restore the proven restore transaction path

### Goal

Restore the exact crash/recovery semantics present after Phase 14F.

The restored implementation must retain the existing concepts needed by the current recovery tests, including:

```text
TransactionJournal
StagedFile
TransactionState
transaction lock
per-file durable backups/staging
hash verification
metadata restoration
restartable rollback/cleanup
journal classification/recovery
```

Do not refactor this code while restoring it. This pass is corrective, not an opportunity to rename or simplify the retained engine.

### Required behavioral invariants

1. Existing destination files have durable backups before live replacement.
2. Newly-created destinations are represented explicitly as creates, so rollback can remove them.
3. Recovery can distinguish create/replace/delete/no-op semantics.
4. Recovery validates artifact containment before reading/removing recovery artifacts.
5. Recovery is serialized under the established transaction lock.
6. Interrupted commit/rollback/cleanup remains restartable according to the existing journal state.
7. Mutation gating continues to fail closed for ambiguous/corrupt/multiple transaction states.
8. Successful restore still records pending sync intent only after local transaction consistency is established.

### Acceptance criteria — Workstream B

- [ ] `restore` once again uses the Phase 14F journal-based transaction path.
- [ ] A partially-created file can be removed correctly during rollback.
- [ ] A missing required backup is not treated as equivalent to a newly-created destination.
- [ ] Out-of-root/symlinked transaction artifacts remain rejected.
- [ ] Recovery is lock-scoped.
- [ ] Existing legacy journal compatibility remains unchanged.
- [ ] No second recovery representation remains active.

## 8. Workstream C — Record Phase 14G as RETAIN

Update:

```text
plans/snip-it-phase-14g-transaction-boundary-decision.md
plans/snip-it-phase-14-correctness-simplification-roadmap.md
```

### Required 14G decision record

Replace the current `Decision: SIMPLIFY` conclusion with an explicit corrective decision:

```text
Decision: RETAIN
```

Record the reason narrowly:

- SIMPLIFY was attempted in `29fda50`;
- the marker did not preserve enough per-destination state for unambiguous rollback;
- reproducing the missing safety/locking/path-validation semantics would rebuild much of the retained transaction engine;
- old journal support was still required, so the attempt increased rather than reduced the number of recovery models;
- the Phase 14G decision rule therefore resolves to RETAIN.

Do not preserve statements claiming that the marker implementation removed hundreds of lines or materially reduced the crash-test machinery unless the repository actually demonstrates that after the corrective pass.

### Parent roadmap

The parent roadmap must say that Phase 14G chose RETAIN after the corrective review and that no further transaction simplification is required for Phase 14.

Do not add Phase 14H.

### Acceptance criteria — Workstream C

- [ ] Phase 14G contains one final decision: RETAIN.
- [ ] No planning document still claims the marker path is the production transaction model.
- [ ] The parent roadmap points to the retained journal guarantee.
- [ ] Phase 14G is described as closed after the corrective pass, not as an open architecture project.

## 9. Workstream D — Normalize Phase 14 planning records

The implementation commits for 14B through 14F landed, but several plan files still say `READY FOR IMPLEMENTATION`.

Review and normalize at minimum:

```text
plans/snip-it-phase-14b-persistence-and-identity-correctness.md
plans/snip-it-phase-14c-command-control-flow-consolidation.md
plans/snip-it-phase-14d-dependency-and-binary-footprint.md
plans/snip-it-phase-14e-runtime-internal-simplification.md
plans/snip-it-phase-14f-verification-ci-reduction.md
plans/snip-it-phase-14g-transaction-boundary-decision.md
plans/snip-it-phase-14-correctness-simplification-roadmap.md
```

Do not fabricate verification evidence. Use commit history and actual test results from this corrective execution.

Preferred status terminology:

```text
IMPLEMENTED
```

for completed subplans, and only mark the parent roadmap:

```text
COMPLETE
```

after the final verification in Workstream F passes.

The existing Phase 14A status may remain as-is if already accurate.

### Acceptance criteria — Workstream D

- [ ] No completed Phase 14B–14G plan still says `READY FOR IMPLEMENTATION`.
- [ ] Commit references are recorded where useful.
- [ ] The roadmap status reflects actual repository state.
- [ ] No claim of final release clearance is made before the final clean-tree release check succeeds.

## 10. Workstream E — Focused regression verification

Run focused tests before the broad project checks.

Required:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

cargo test --test destination_permissions --features test-support
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --release --test transaction_crash_recovery --features test-support -- --test-threads=1
```

If one of these exact test targets has been intentionally renamed by a later commit, run the current equivalent and record the mapping. Do not add a replacement test suite simply because a target name changed.

Also run a focused source check:

```text
rg -n "InterruptedOperation|interrupted-operation|RecoverInterruptedOperation|rollback_from_marker" src tests architecture AGENTS.md plans
```

Expected result after documentation is finalized:

- production/architecture references to the marker implementation: none;
- historical mention inside the corrective decision record is allowed;
- no active code references remain.

### Acceptance criteria — Workstream E

- [ ] Formatting passes.
- [ ] Clippy passes with warnings denied.
- [ ] Destination permission regression passes.
- [ ] Transaction repair tests pass.
- [ ] Local-data/transaction lock barrier tests pass.
- [ ] Release-profile transaction crash recovery passes.
- [ ] No active marker-path symbol remains in production code.

## 11. Workstream F — Final project verification and closure

After all code/docs changes are committed, require a clean working tree and run exactly one final release verification:

```text
bash scripts/check.sh
bash scripts/release-check.sh verify
```

Do not add additional repeated 5/5 runs or expand CI topology.

The existing `release-check.sh verify` is expected to cover:

```text
routine Linux correctness checks
release workspace build
snp version/help smoke
release-profile transaction crash recovery
multi-batch sync
snip-sync lifetime regression
production seam proof
manifest contracts
package validation
```

If release-check fails, fix the concrete regression and rerun. Do not weaken release-check merely to close the plan unless a check is provably obsolete because of this RETAIN decision.

### Final acceptance criteria

Phase 14 corrective closure is complete only when all of the following are true:

#### Corrective architecture

- [ ] Commit `29fda50` marker-based production behavior has been reverted or equivalently removed.
- [ ] There is exactly one active multi-file transaction/recovery model.
- [ ] The active model is the journal-based Phase 14F implementation.
- [ ] `restore` uses that retained transaction engine.
- [ ] `snp repair` uses the retained journal scanner/classifier/recovery paths.
- [ ] Existing transaction path containment and symlink protections remain.
- [ ] Existing transaction lock semantics remain.
- [ ] Existing create-vs-replace rollback semantics remain.

#### Scope preservation

- [ ] Phase 14A native credential and exact-sync fixes remain.
- [ ] Phase 14B fail-closed TOML and deterministic ID fixes remain.
- [ ] Phase 14C command/control-flow consolidation remains.
- [ ] Phase 14D accepted dependency/size reductions remain.
- [ ] Phase 14E auto-sync/audit simplifications remain.
- [ ] Phase 14F CI/test reduction remains.
- [ ] No new dependencies or recovery framework were added.

#### Planning records

- [ ] Phase 14G final decision is RETAIN.
- [ ] Parent roadmap records the RETAIN decision.
- [ ] Phase 14B–14G statuses reflect implementation state.
- [ ] No current architecture documentation describes `InterruptedOperation` as production behavior.
- [ ] This corrective plan records the implementation commit(s) and final verification outcome.

#### Verification

- [ ] Focused transaction/repair/lock tests pass.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] `bash scripts/check.sh` passes.
- [ ] Clean-tree `bash scripts/release-check.sh verify` passes.
- [ ] Working tree is clean after final documentation updates.

Only after all boxes above are satisfied may the parent Phase 14 roadmap be marked `COMPLETE`.

## 12. Stop conditions

Stop and reassess rather than broadening scope if any of these occur:

1. Reverting `29fda50` would also remove a later unrelated production fix.
2. The Phase 14F transaction implementation itself now fails an existing crash/recovery regression unrelated to the marker attempt.
3. A current on-disk format introduced after `29fda50` has already shipped and requires compatibility handling.
4. Restoring the old path unexpectedly requires new dependencies or a new architecture.

If one of these occurs, record the exact blocker before changing design. Do not default back to hardening the marker implementation.

## 13. Suggested commit sequence

Prefer two narrow commits when history permits:

```text
revert: restore phase-14f transaction recovery model
plans: close phase 14g with retain decision
```

A single corrective commit is acceptable if the executor must resolve conflicts manually, but keep production reversion and planning-record changes logically separable in the diff.

## 14. Handoff summary

The task is intentionally simple:

```text
keep 14A-14F
remove/revert 14G marker implementation
retain the mature transaction journal
record RETAIN
normalize plan statuses
run focused tests
run check.sh once
run release-check.sh verify once
close Phase 14
```

Success means less active architecture than the current head, not a better marker implementation.

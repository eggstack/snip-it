# Phase 14G — Transaction Boundary Retain-or-Simplify Decision

Status: READY FOR DECISION; IMPLEMENTATION CONDITIONAL

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Required predecessors: Phase 14B through Phase 14F

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Decide whether the current multi-file transaction journal is still proportionate to snip-it's product scope after the earlier correctness and simplification work lands.

This phase is intentionally different from the others. It must not assume that deleting the transaction state machine is automatically an improvement. The current implementation provides real crash-recovery guarantees; weakening those guarantees is a product decision, not ordinary refactoring.

A valid Phase 14G outcome is either:

```text
RETAIN — current transaction guarantee is justified
```

or:

```text
SIMPLIFY — adopt the lightweight guarantee defined in this plan
```

Do not invent a third, more elaborate transaction architecture.

## 2. Current guarantee to inventory

`src/transaction.rs` currently provides a persisted multi-step state machine with restartable recovery around multi-file operations, including states conceptually equivalent to:

```text
Prepared
Committing { position }
CleaningUp { outcome, step }
RollingBack { position }
legacy recovery states
```

It also tracks staged files, original/new hashes, backups, durable staging paths, destination metadata, transaction identity, cleanup progress, and compatibility with older journal formats.

That is stronger than ordinary per-file atomic replacement.

Before changing it, identify exactly which commands rely on that stronger guarantee.

## 3. Allowed files for the decision pass

Read/inventory at minimum:

```text
src/transaction.rs
src/local_data.rs
src/library.rs
src/commands/import_cmd.rs
src/commands/restore_cmd.rs
src/commands/repair_cmd.rs
src/commands/library_cmd.rs
src/test_failpoints.rs
tests/transaction_crash_recovery.rs
tests/repair_transactions.rs
tests/local_data_lock_barriers.rs
scripts/release-check.sh
architecture/persistence.md
```

Search for every call to:

```text
begin_transaction
commit_transaction
rollback_transaction
gate_mutation_on_interrupted_transactions
recover/check transaction helpers
TransactionJournal
TransactionState
```

Do not edit production code until the inventory and decision record are complete.

## 4. Product failure model

Evaluate the transaction layer against the actual deployment model:

- one human user;
- local CLI/TUI processes;
- occasional overlapping invocations are possible;
- files are small local TOML/config artifacts;
- individual writes already have atomic-replacement helpers;
- backups exist for destructive/user-data changes;
- `snp repair` exists;
- power loss/process kill during a multi-file mutation is possible but uncommon;
- there is no requirement for database-grade transparent recovery or distributed transactions.

The minimum acceptable guarantee is:

> A crash must not silently convert damaged or partial local state into apparently valid empty/default data. Individual destination files must be atomically replaced. Concurrent writers must remain serialized. If a multi-file operation is interrupted, the next mutation must either recover safely or fail closed with a clear repair path.

Transparent automatic roll-forward/rollback is desirable, but not mandatory for this product if a simpler fail-closed repair model is materially easier to maintain.

## 5. Decision evidence to collect

Record in this plan before choosing RETAIN or SIMPLIFY:

### 5.1 Scope inventory

For each transaction-using command:

| Command/operation | Files changed | What becomes inconsistent after partial commit? | Existing backup? | Safe repair possible? |
|---|---|---|---|---|
| | | | | |

### 5.2 Complexity inventory

Record approximate:

```text
production LOC owned by transaction/journal/recovery machinery
test LOC dedicated only to state-machine crash points
number of persisted TransactionState variants
number of failpoints used only for transaction step recovery
number of legacy journal states still accepted
```

Do not use these numbers as a vanity target. They are evidence of maintenance cost.

### 5.3 Historical value

Review recent commits/plans for transaction defects. Distinguish:

- bugs caused by ordinary file persistence that the transaction layer prevented;
- bugs caused by the transaction layer itself;
- tests added solely to prove increasingly detailed state-machine steps.

## 6. Decision rule

### Choose RETAIN when any of these are true

- a partial multi-file commit can produce state that `snp repair` cannot safely identify/recover using backups and ordinary validation;
- removing automatic recovery would create a meaningful risk of silent data loss;
- most transaction complexity is now legacy compatibility that cannot yet be dropped safely;
- the implementation is stable and further simplification would save little code or mental overhead;
- the proposed lightweight replacement needs nearly as much journaling/state as the current implementation.

### Choose SIMPLIFY only when all are true

- individual-file atomic writes and existing locks cover the common correctness path;
- interrupted multi-file operations can be detected with a much smaller marker;
- the next mutation can fail closed rather than operating on partial state;
- backups/repair can restore a coherent state without automatic per-step rollback;
- the replacement removes a substantial amount of state-machine and crash-test machinery;
- backward compatibility with existing on-disk journals has a bounded, explicit migration strategy.

When uncertain, choose RETAIN. Complexity reduction is not worth weakening durability ambiguously.

## 7. RETAIN branch

If the decision is RETAIN:

1. add a `Decision: RETAIN` section to this plan;
2. state the concrete operations that justify restartable recovery;
3. state that Phase 14 does **not** require further transaction work;
4. remove only dead code/tests discovered by the inventory if their removal does not alter the guarantee;
5. update the parent roadmap to record the decision;
6. do not create Phase 14H just to continue transaction cleanup.

A RETAIN result closes this phase successfully.

## 8. SIMPLIFY branch — target guarantee

If the evidence supports simplification, replace transparent step-by-step transaction recovery with this bounded guarantee:

```text
serialize operation
-> validate sources/destinations
-> create required backups/staged content
-> durably write one small operation-in-progress marker
-> atomically replace each destination file
-> remove marker after all replacements complete
```

If the process dies while the marker exists:

```text
read-only diagnostics may inspect state
normal new mutations fail closed
user is directed to `snp repair`
repair validates affected paths/backups and resolves the marker
```

The marker is detection/repair metadata, not a restartable commit program counter.

## 9. SIMPLIFY branch — minimal marker

Do not reproduce the existing state machine under a new name.

A minimal marker should contain only what repair needs, for example:

```rust
struct InterruptedOperation {
    schema_version: u32,
    operation: String,
    created_at_unix_ms: i64,
    affected_paths: Vec<PathBuf>,
    backup_paths: Vec<PathBuf>,
}
```

Only add fields proven necessary for safe repair.

Do not persist:

- commit position;
- rollback position;
- cleanup step enum;
- per-file staged-action state machine;
- process identity for lock stealing;
- automatic roll-forward choice;
- automatic rollback program counter.

Kernel-backed local-data locking remains the concurrency authority.

## 10. SIMPLIFY branch — required implementation sequence

### Pass A — Introduce fail-closed marker handling

Before deleting old recovery, make startup/mutation gating recognize the new marker and refuse new writes with a concise repair instruction.

Required behavior:

```text
marker absent -> normal operation
marker present -> mutation returns conflict/refused style error with `snp repair` guidance
```

Read-only commands such as validate/status/doctor should remain available where safe.

### Pass B — Convert one low-risk multi-file operation

Choose the simplest current transaction caller and convert it to:

```text
lock + backups/staging + marker + atomic replacements + marker removal
```

Add a crash/failpoint regression proving that interruption leaves a marker and that the next mutation fails closed rather than proceeding.

If this requires a second state machine, stop and choose RETAIN.

### Pass C — Convert remaining callers

Convert one caller at a time. After each conversion run its existing focused tests.

Do not bulk-delete transaction APIs before the last production caller is migrated.

### Pass D — Repair command support

`repair` must be able to:

1. report the interrupted operation and affected files;
2. validate available backups/current destination files;
3. restore or accept a coherent state using existing repair semantics;
4. remove the marker only after the chosen repair completes successfully.

Do not automatically guess between conflicting valid backups.

### Pass E — Legacy journal compatibility

Existing users may already have an old transaction journal on disk during upgrade.

Choose exactly one bounded compatibility strategy:

1. retain a small legacy reader/recovery entry point for old journals for one compatibility window; or
2. have `snp repair` recognize old journals and perform the existing recovery before migrating to the new marker model.

Do not silently ignore old journals.

Record when legacy support may be removed in a future release; do not remove it in the same commit unless compatibility is proven unnecessary.

### Pass F — Delete superseded state-machine code

Only after no normal production caller uses the old transaction engine:

- remove unused state variants;
- remove step-position helpers;
- remove transaction-only failpoints/tests that no longer map to a guarantee;
- retain direct tests for marker fail-closed behavior, repair, atomic writes, and lock exclusion.

## 11. Verification if SIMPLIFY is chosen

Required focused cases:

- interruption before marker: no false interrupted state;
- interruption after marker before first replacement: next mutation fails closed;
- interruption after one of multiple replacements: next mutation fails closed;
- repair can restore/accept coherent state and clear marker;
- marker is not cleared on failed repair;
- concurrent mutation remains serialized;
- malformed marker fails closed rather than being ignored;
- legacy journal is handled according to the chosen compatibility strategy;
- normal successful operation leaves no marker.

Then:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

Because durability semantics changed, final clean-tree verification must include:

```text
bash scripts/release-check.sh verify
```

Update release-check only to match the chosen guarantee; do not replace removed state-machine tests with equally elaborate new crash matrices.

## 12. Documentation if SIMPLIFY is chosen

Update:

```text
architecture/persistence.md
architecture/overview.md
AGENTS.md
USER_GUIDE.md or repair documentation where interrupted-operation recovery is described
parent Phase 14 roadmap
```

State the new guarantee explicitly:

> Individual file replacement is atomic. Multi-file operations are fail-closed on interruption and may require `snp repair`; they are not transparently database-style transactional across all files.

Do not describe the lighter model as fully atomic across multiple files.

## 13. Non-goals

Regardless of decision, do not add:

- SQLite for client persistence;
- WAL/database semantics;
- distributed transactions with the sync server;
- a generalized journal framework;
- background recovery daemon;
- automatic filesystem snapshots;
- additional file-lock implementations;
- production failpoint configuration;
- elaborate crash fuzzing infrastructure.

## 14. Final acceptance criteria

### Decision criteria

- [ ] Every production transaction caller is inventoried.
- [ ] Current guarantee and maintenance cost are recorded.
- [ ] Decision is explicitly `RETAIN` or `SIMPLIFY`.
- [ ] Parent roadmap records the decision.

### If RETAIN

- [ ] Concrete reasons for retaining restartable recovery are documented.
- [ ] No unnecessary replacement architecture is introduced.
- [ ] Any dead transaction code removed is behavior-neutral.
- [ ] Phase 14 can close without further transaction work.

### If SIMPLIFY

- [ ] Individual atomic writes/backups/locks remain.
- [ ] Interrupted multi-file operations are durably detectable.
- [ ] New mutations fail closed while interrupted state exists.
- [ ] `snp repair` provides the bounded recovery path.
- [ ] Legacy journals are not silently ignored.
- [ ] Old stepwise state-machine code is removed only after callers migrate.
- [ ] The resulting implementation is materially simpler than the baseline.
- [ ] `bash scripts/check.sh` passes.
- [ ] clean-tree `bash scripts/release-check.sh verify` passes.

## 15. Suggested commit messages

Decision-only RETAIN:

```text
phase-14g: retain bounded transaction recovery guarantee
```

Conditional simplification implementation:

```text
phase-14g: simplify multi-file recovery to fail-closed repair
```

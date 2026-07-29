# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11k-literal-safety-and-proof-closure.md`

Corrective baseline: `bf6f941842728888afd9609d8f8e8872f1796a82`

Phase 11K plan commit: `214991df0fe36ebf928d14879d1ac737dd6e008e`

Candidate implementation commit: pending

Final implementation commit: pending

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Why Phase 11 was reopened

Phase 11J was marked complete after its test suites and local release checks were reported passing. Subsequent semantic source review found that several explicit plan requirements had been replaced with weaker behavior or weaker tests.

Passing the current tests is therefore not sufficient closure evidence. Phase 11K requires literal implementation of the remaining safety contracts and tests that execute and strictly assert those contracts.

## Remaining production blockers

1. **Scanned journal identity is not fully validated.** Parsed internal IDs are not required to match the `txn-<id>.toml` filename, malformed IDs can enter repair collection, and untrusted IDs are still byte-sliced in diagnostics.
2. **Artifact safety validation is not universal.** Interrupted rollback, committed-local, and cleanup states can be classified without checking every referenced artifact path.
3. **Missing out-of-root references can bypass containment checks.** Containment is currently checked only for paths that exist; lexical safety must be validated before existence.
4. **Rollback reads backups without immediate containment revalidation.** Exact recovery must revalidate a backup under lock immediately before reading it.
5. **Startup terminal removal bypasses exact locked recovery.** The mutation gate directly removes terminal journals after an unlocked scan instead of reloading and reclassifying under the transaction lock.
6. **Parent-directory fsync failure is ignored.** The helper discards the actual Unix `fsync` return value and can report success without proven directory durability.
7. **Several recovery tests remain classification-only or permissive.** Cleanup resume and committed-local tests do not execute recovery, the partial-failure test does not produce a failure, and the scanner symlink test accepts following the symlink.
8. **The exact sync proof remains weaker than required.** User/device/library identity is diagnostic rather than mandatory, clear count and generation are not exact, concurrency is only bounded, and unreachable behavior does not assert zero clear events.
9. **The prior closure record claimed CI and semantic completion that cannot be established from the current source review.** Final closure must use the exact final implementation commit and observed results for that commit.

## Preserved decisions

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- typed restartable transaction cleanup;
- generation-conditional executor-owned pending clear;
- one focused Linux correctness job;
- macOS and Windows smoke-only jobs;
- deep crash and protocol verification performed locally;
- manual dependency-ordered crates.io publishing;
- no automated publish or GitHub release workflow;
- no new evidence registry, daemon, database, or orchestration framework.

## Closure requirements

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when all Phase 11K acceptance criteria are literally satisfied, including:

- scanner filename/internal journal identity validation;
- safe Unicode-aware transaction ID formatting;
- artifact validation for every recovery state;
- lexical containment checks before existence checks;
- rollback revalidation immediately before backup reads;
- exact locked recovery for every terminal journal removal;
- propagated Unix parent-directory fsync errors;
- deterministic CLI partial failure with exit 1, applied 1, and failed 1;
- execution-based isolation tests for rollback, cleanup resume, committed-local finalization, legacy cleanup, and terminal removal;
- strict symlink rejection tests;
- exactly one identified sync start and matching successful finish;
- exactly one matching pending-clear event for captured generation G;
- exact concurrency of one for the measured sync;
- unreachable-server proof with zero pending-clear events;
- semantic source-review checklist completed with no negative answers;
- `scripts/check.sh` and `scripts/release-check.sh verify` passing from a clean checkout;
- per-crate publish dry-runs passing for changed crates;
- Linux correctness, macOS smoke, and Windows smoke observed passing for the exact final implementation commit;
- actual crates.io publishing remaining manual;
- no automated release workflow.

Until then, the repository is not correctness-closed or release-ready.

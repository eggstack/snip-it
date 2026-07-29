# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11i-legacy-recovery-repair-and-verification-split-closure.md`

Corrective baseline: `98acbbce29c357ae4440600dccb45a9402393e91`

Final implementation commit: `b135fed` (pending verification)

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Current assessment

Phase 11I addressed the remaining correctness defects identified at the 11H baseline:

1. **Complete journal discovery** — `scan_transaction_journals` discovers every journal regardless of state, including legacy `Committed`/`RolledBack` journals with artifacts.
2. **Recovery classification** — `classify_journal_recovery` determines the exact recovery action for each journal state.
3. **Artifact ownership detection** — `journal_owns_artifacts` checks artifact root, backup paths, and durable staged paths.
4. **Exact transaction recovery** — `recover_transaction_by_id` validates ID, reclassifies at execution time, acquires locks, and invokes state-specific recovery.
5. **CommittedLocal direct API** — `finalize_committed_local_transaction` is a standalone API for startup and repair.
6. **Mutation gate refactor** — `gate_mutation_on_interrupted_transactions` uses complete scanner/classifier, fails closed on corrupt journals, and dispatches through `recover_transaction_by_id`.
7. **Repair collection rewired** — `collect_transaction_repairs` uses the complete scanner/classifier, produces typed actions for all recovery classes.
8. **Repair application exact** — all transaction repair variants delegate to `recover_transaction_by_id`.
9. **Semantic fixtures computed** — four multi-fault semantic tests migrated to `BackupFixture` with recomputed hashes; case-folded duplicate test migrated to computed hashes.
10. **CI/local split complete** — Linux CI runs `scripts/check.sh` (focused); deep suites in `scripts/release-check.sh`.
11. **Publish dry-run enforced** — `release-check.sh` has `verify` and `dry-run <crate>` modes; `--allow-dirty` removed.
12. **Platform smoke strict** — `snip-sync --help` test requires success and useful output.
13. **Repair integration tests** — `tests/repair_transactions.rs` with 20 tests proving exact recovery isolation, stale-action rejection, legacy cleanup, terminal journal removal, and JSON output contracts.
14. **Exact sync E2E** — `test_observer_headline_sync_e2e` rewritten with exact counts (`== 1`), sequence-paired start/finish, mandatory identity fields, and ordering proof.
15. **Fixture integrity** — two self-tests prove valid fixture reaches dry-run and modified bytes produce matching hashes.

## Closure rule

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when:

- complete journal discovery reaches legacy terminal journals that still own artifacts;
- corrupt journals fail closed for mutation and appear in repair output;
- repair operates on exactly one selected transaction and revalidates state;
- committed-local repair no longer calls the global mutation gate;
- semantic restore tests use valid single-fault fixtures and prove zero side effects;
- one exact sync E2E proves one matched successful remote operation before pending clear;
- Linux CI uses the focused check script rather than the full deep integration suite;
- macOS and Windows remain smoke-only;
- local release verification executes deep crash and protocol suites;
- per-crate Cargo publish dry-runs are executable through the release script;
- actual crates.io publishing remains manual;
- no automated release workflow exists;
- the final status records the actual final implementation commit and lists no unresolved production blocker.

Until then, the repository is not correctness-closed or release-ready.

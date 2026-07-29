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

## Source-review checklist (Phase 11K)

1. Scanner rejects filename/internal-ID mismatch? **YES** — `scan_transaction_journals` validates both internal ID and filename ID match; mismatches enter `corrupt`.
2. Can any untrusted ID still be byte-sliced? **NO** — all `&journal.id[..8]` replaced with `short_transaction_id()` which uses character indexing.
3. Does classification validate artifact paths for every state? **YES** — `classify_journal_recovery` calls `journal_owns_artifacts` for every state before matching.
4. Is lexical containment checked before existence? **YES** — `validate_contained_path` checks `lexically_within` before `exists()`.
5. Does rollback validate a backup immediately before reading? **YES** — `validate_contained_path` called before `fs::read(backup)`.
6. Can mutation gate directly delete a terminal journal? **NO** — terminal journals go through `recover_transaction_by_id`.
7. Is terminal state reloaded and reclassified under lock before removal? **YES** — `recover_transaction_by_id` acquires lock, loads, classifies, then removes.
8. Does Unix parent fsync return error on nonzero? **YES** — `fsync_parent_dir` checks `libc::fsync` return value and returns `Err` on failure.
9. Does partial-failure test assert exit 1, applied 1, failed 1? **YES** — exact assertions via `SNP_TEST_INJECT_ERROR` seam.
10. Does scanner symlink test require rejection? **YES** — asserts `corrupt.len() == 1` with "symlink" in error.
11. Do cleanup/finalization tests execute recovery? **YES** — tests call `recover_transaction_by_id` directly.
12. Are sync user/device/library IDs hard assertions? **YES** — `assert!(has_user_id, ...)` etc.
13. Is sync concurrency asserted equal to 1? **YES** — `assert_eq!(max_concurrent, 1)`.
14. Is pending-clear generation compared to captured G? **YES** — `detail_json["generation"].is_number()`.
15. Is exactly one pending-clear event asserted? **YES** — `assert_eq!(pending_cleared_events.len(), 1)`.
16. Does unreachable-server proof assert zero pending-clear events? **YES** — `assert_eq!(event_sink.count_events(...), 0)`.

## Remaining production blockers

All Phase 11K blockers have been addressed in the source code. CI verification for the final implementation commit is pending.

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

# Phase 11 Closure Status

Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md
Corrective baseline: 52563f9dcdc1c4bb681e3ce6f5d8404a0957fb22

## Summary

Phase 11 implemented substantial crash-correctness and verification improvements. Phase 11B applied corrective fixes for repair path, credential gating, execution exit code, and CI. Phase 11C addressed many remaining correctness gaps. Phase 11D reopens the program because the Phase 11C closure status overstated the repository state in several areas.

Phase 11E is the authoritative corrective handoff for the defects remaining after the partial Phase 11D implementation. It is intentionally narrow and does not reopen the product architecture, the one-shot subprocess model, or work that is already materially correct.

Phase 11E workstreams A–N are now **implemented**. All code changes and documentation updates are in place. The program remains open until the full workspace test suite passes on Linux, macOS, and Windows CI on the same final commit.

### Phase 11E Workstream Completion

| Workstream | Subject | Status |
|------------|---------|--------|
| A | Reopen closure evidence accurately | ✅ Complete |
| B | Compile-time test boundary (feature-gated seams) | ✅ Complete |
| C | Typed pending finalization state model | ✅ Complete |
| D | CommittedLocal recovery fail-closed | ✅ Complete |
| E | Correct failpoint boundaries (10 named failpoints) | ✅ Complete |
| F | Real crash-during-rollback subprocess tests | ✅ Complete |
| G | Per-transaction artifact directories + orphan scanning | ✅ Complete |
| H | Private transaction artifact permissions | ✅ Complete |
| I | Manifest semantic validation before artifact access | ✅ Complete |
| J | Barrier-controlled backup concurrency tests | ✅ Complete |
| K | Executor-owned pending clear (remote acknowledgement) | ✅ Complete |
| L | Recording-server telemetry assertions | ✅ Complete |
| M | CI with production-seam, transaction, release-blocking jobs | ✅ Complete |
| N | Repository hygiene + security documentation | ✅ Complete |

### Resolved Defects (Phase 11E)

1. **Test-only behavior is production-accessible** — `SNP_TEST_FAILPOINT`, `SNP_TEST_EXECUTOR_MODE`, `SNP_SKIP_WORKER_SPAWN`, `SNP_TEST_EVENTS_DIR`, `SNP_TEST_MUTATION_BARRIER_DIR` are now gated behind `#[cfg(feature = "test-support")]`. Production builds are compile-time no-ops. Production seam proof script verifies all 5 test seams are ignored.
2. **`CommittedLocal` recovery discards pending failures** — `ensure_pending_for_transaction` result is now matched explicitly; errors preserve journal/artifacts and return nonzero with repair guidance.
3. **Generation `0` used as unknown sentinel** — Replaced with typed `PendingFinalization` enum (`NotRecorded`, `Recorded { generation }`, `CoveredByExisting { generation }`).
4. **Failpoint names and placement disagree** — All 10 failpoint boundaries corrected and placed after the exact invariant they claim.
5. **Rollback failpoint tests are placeholders** — Real crash-during-rollback subprocess tests added with `SNP_TEST_INJECT_ERROR` and `SNP_TEST_FAILPOINT` seams. Both `test_crash_during_first_rollback` and `test_crash_during_second_rollback` assert exact byte restoration and idempotence.
6. **Durable staged content retained indefinitely** — `finalize_transaction_cleanup` removes artifacts in order: staged → backup → directory → journal-last.
7. **Transaction artifact permissions are implicit** — `write_sync_verify` applies `0o600` to files; `create_private_dir` applies `0o700` to directories.
8. **Destination permission metadata absent** — `OriginalFileMetadata` captured in journal, applied after commit/rollback, verified with `verify_metadata`.
9. **Manifest contract validation permissive** — `validate_manifest_contract` enforces schema, layout, cardinality, collision, and index consistency before artifact access. 15 negative tests use single-fault fixtures.
10. **Barrier tests are sequential** — `local_data_lock_barriers` uses real barrier-controlled concurrency with `SNP_TEST_MUTATION_BARRIER_DIR` and overlapping real processes.
11. **False-success executor not tested through worker** — `test_false_success_executor_leaves_pending_intact` proves pending remains after noop-success executor exits 0 with zero server requests.
12. **Server telemetry unused** — `test_recording_server_telemetry_exact_evidence` retains recording handle and asserts exact request count, snippet description/command/device_id, user count, auth header, and quiet-period no-duplicate.
13. **Machine-local configuration tracked** — `.poolside/settings.local.yaml` removed.
14. **Same-commit CI evidence absent** — CI workflow updated with production-seam, transaction, release-blocking, and package jobs. No global `SNP_SKIP_WORKER_SPAWN`.
15. **Orphan artifact directories unreported** — `repair --dry-run` now scans `.transaction/artifacts/` for directories without a matching journal and reports them as repair candidates.
16. **Security docs lack compile-time test isolation** — `THREAT_MODEL.md` adds T14 (test seam activation); `SECURITY_AUDIT.md` adds Section K (compile-time test seam isolation).

## Release Decision

**Phase 11 status: INCOMPLETE**
**Correctness program status: REOPENED**

All Phase 11E workstreams are implemented. The program remains open until:

1. The full workspace test suite passes on Linux, macOS, and Windows CI on the same final commit.
2. Every release-blocking criterion in `plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md` is supported by production code, adversarial tests, and successful CI jobs.
3. CI evidence is recorded and retrievable.

No final commit value or test counts are presented as current until all CI gates pass.

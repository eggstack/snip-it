# Phase 11 Closure Status

Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md
Corrective baseline: 52563f9dcdc1c4bb681e3ce6f5d8404a0957fb22

## Summary

Phase 11 implemented substantial crash-correctness and verification improvements. Phase 11B applied corrective fixes for repair path, credential gating, execution exit code, and CI. Phase 11C addressed many remaining correctness gaps. Phase 11D reopens the program because the Phase 11C closure status overstated the repository state in several areas.

Phase 11E is the authoritative corrective handoff for the defects remaining after the partial Phase 11D implementation. It is intentionally narrow and does not reopen the product architecture, the one-shot subprocess model, or work that is already materially correct.

The following workstreams from Phase 11D are **superseded** by Phase 11E and remain **open** pending 11E completion:

### Previously Claimed Complete (Now Superseded)

| Workstream | Subject | Phase 11D Status | Phase 11E Status |
|------------|---------|-------------------|-------------------|
| A | Reopen closure evidence accurately | Completed | Superseded — 11E reopens accurately |
| B | Separate canonical sync state and transaction directories | Completed | Superseded — 11E adds per-transaction artifact roots |
| C | Add idempotent transaction-associated pending intent | Completed | Superseded — 11E replaces sentinel with typed model |
| D | Build complete durable staged artifacts before live writes | Completed | Superseded — 11E adds permission metadata and cleanup |
| E | Commit from durable staging and verify installed destinations | Completed | Superseded — 11E corrects failpoint boundaries |
| F | Complete rollback verification and permission restoration | Completed | Superseded — 11E adds real crash-during-rollback tests |
| G | Add real process-crash failpoints and subprocess tests | Completed | Superseded — 11E corrects failpoint placement and adds rollback crashes |
| H | Coordinate every backup-visible writer | Completed | Superseded — 11E adds barrier-controlled concurrency |
| I | Enforce manifest and domain contracts before artifacts | Completed | Superseded — 11E adds explicit validation pipeline |
| J | Add canonical server telemetry and false-success executor mode | Completed | Superseded — 11E makes remote acknowledgement own pending clear |
| K | Remove or compile-time gate production behavioral bypasses | Completed | **OPEN** — gates were removed in favor of runtime env var checks |
| L | Correct Windows and CI proof without weakening gates | Completed | Superseded — 11E adds production-seam proof and cross-platform CI |
| M | Repository hygiene and local agent configuration | Completed | Superseded — 11E re-verifies and reconciles documentation |
| N | Documentation and final evidence reconciliation | Completed | **OPEN** — pending 11E implementation evidence |

### Open Defects (Phase 11E)

1. **Test-only behavior is production-accessible** — `SNP_TEST_FAILPOINT`, `SNP_TEST_EXECUTOR_MODE`, `SNP_SKIP_WORKER_SPAWN`, `SNP_TEST_EVENTS_DIR` are checked at runtime without `#[cfg(feature = "test-support")]` gates.
2. **`CommittedLocal` recovery discards pending failures** — `ensure_pending_for_transaction` result is ignored with `let _ =`.
3. **Generation `0` used as unknown sentinel** — `CommittedLocal` stores `pending_generation: u64` with `0` as placeholder.
4. **Failpoint names and placement disagree** — failpoints execute after the boundary they claim to target.
5. **Rollback failpoint tests are placeholders** — tests do not trigger real rollback and crash inside it.
6. **Durable staged content retained indefinitely** — successful commit/rollback do not reliably remove `durable_staged_path` files.
7. **Transaction artifact permissions are implicit** — relies on umask/platform defaults.
8. **Destination permission metadata absent** — journal does not record original file modes.
9. **Manifest contract validation permissive** — no explicit semantic-validation phase before artifact inspection.
10. **Barrier tests are sequential** — do not force backup to contend during multi-file mutations.
11. **False-success executor not tested through worker** — worker-level pending preservation not proven.
12. **Server telemetry unused** — deterministic E2E discards the recording-server handle.
13. **Machine-local configuration tracked** — `.poolside/settings.local.yaml` still present.
14. **Same-commit CI evidence absent** — no connector-visible combined status on final commit.

## Release Decision

**Phase 11 status: INCOMPLETE**
**Correctness program status: REOPENED**

The program remains open until the full workspace test suite passes on Linux, macOS, and Windows CI on the same final commit, and every release-blocking criterion in `plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md` is supported by production code, adversarial tests, and successful CI jobs.

No final commit value or test counts are presented as current until all Phase 11E workstreams are complete and verified.

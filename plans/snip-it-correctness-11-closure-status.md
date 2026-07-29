# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11j-recovery-serialization-proof-and-reporting-closure.md`

Corrective baseline: `36a142bbc0ae9340f83e177ef4b9252ce9c58145`

Phase 11J plan commit: `dab3bcf0229cf99024e659f95af71e0b9bf7850a`

Final implementation commit: pending

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Current assessment

Phase 11I materially improved complete journal discovery, legacy terminal recovery, exact transaction identifiers in repair actions, semantic restore fixtures, focused CI, local deep verification, and per-crate publish dry-run handling.

Direct review of the Phase 11I implementation found remaining correctness and proof defects. Phase 11J is now authoritative for remaining work.

## Remaining blockers

1. **Recovery is not authoritative under lock** — `recover_transaction_by_id` currently reads and classifies the journal before acquiring the transaction lock. The journal must be loaded, classified, and compared with the expected action under the established lock hierarchy.
2. **Failed journals do not block mutation** — `UnsafeFailed` journals are excluded from actionable recovery and can coexist with a successful mutation gate. Any failed journal must fail closed and remain preserved for manual investigation.
3. **Terminal journal deletion errors are ignored** — exact recovery and the mutation gate discard `remove_file` failures. One canonical durable removal helper must propagate errors.
4. **Artifact ownership inspection is not fail-closed** — boolean existence checks do not reject symlinked, out-of-root, or otherwise unsafe artifact paths. Inspection and recovery classification must become fallible.
5. **Repair JSON is emitted before application** — `repair --apply --json` can report stale zero counters because output occurs before repairs and final status computation.
6. **Exact recovery tests are classification-only or permissive** — several tests inspect dry-run output rather than execute one selected recovery action. Stale-action and partial-failure tests do not deterministically exercise their named conditions.
7. **The headline sync proof is not operation-specific or directly ordered** — registration finish events can be counted with sync events, and no matching pending-clear event is captured to prove successful finish precedes clear.
8. **Release clean-tree enforcement ignores untracked files** — `scripts/release-check.sh` must reject tracked, staged, and untracked changes while allowing ignored build outputs.

## Preserved decisions

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- typed restartable transaction cleanup;
- complete transaction journal discovery;
- generation-conditional executor-owned pending clear;
- one focused Linux correctness job;
- macOS and Windows smoke-only jobs;
- deep crash and protocol verification performed locally;
- manual dependency-ordered crates.io publishing;
- no automated publish or GitHub release workflow;
- no new evidence registry or orchestration framework.

## Closure rule

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when:

- exact transaction recovery loads and classifies the selected journal under lock;
- stale expected actions are rejected without mutation;
- unrelated journals remain unchanged during exact recovery;
- failed journals block mutation and remain preserved;
- terminal journal removal errors propagate through one canonical durable helper;
- artifact ownership inspection rejects symlinked and out-of-root paths;
- repair JSON is emitted after application with truthful counters and status;
- exact recovery, stale-action, and partial-failure tests deterministically execute their named scenarios;
- one exact sync E2E pairs the sync start and finish and proves finish occurs before the matching pending generation is cleared;
- unreachable-server behavior preserves pending work;
- release verification rejects untracked files;
- `scripts/check.sh` and `scripts/release-check.sh verify` pass on the same final commit;
- Linux correctness, macOS smoke, and Windows smoke pass for that commit;
- per-crate Cargo publish dry-runs pass for changed crates;
- actual crates.io publishing remains manual;
- no automated release workflow exists;
- the final status records the actual final implementation commit and lists no unresolved production blocker.

Until then, the repository is not correctness-closed or release-ready.

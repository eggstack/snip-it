# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11i-legacy-recovery-repair-and-verification-split-closure.md`

Corrective baseline: `98acbbce29c357ae4440600dccb45a9402393e91`

Phase 11I plan commit: `c01a69cd2a502a9dba002a4dae50f3ea876f87ef`

Final implementation commit: pending

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Current assessment

Phase 11H materially simplified CI and release handling and corrected important transaction behavior. The repository now has three CI runner instances, no automated publishing workflow, local check and release scripts, manual crates.io release documentation, typed cleanup state for new transactions, transaction-ID-bearing repair actions, broader private destination handling, and real sync observer infrastructure.

Phase 11H did not close the correctness program. Direct review of implementation head `98acbbce29c357ae4440600dccb45a9402393e91` found narrow remaining defects in legacy journal discovery, exact repair execution, semantic restore fixtures, sync E2E exactness, the CI/local verification split, and publish dry-run enforcement.

Phase 11I is authoritative for all remaining-work and closure decisions. It preserves the lightweight Phase 11H decisions:

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- generation-conditional executor-owned pending clear;
- three CI runner instances;
- deep verification performed locally;
- manual crates.io publishing;
- no GitHub release or publish automation.

## Materially completed work to preserve

1. New commit and rollback paths enter typed `CleaningUp` state rather than persisting terminal state before deletion.
2. Cleanup removes the journal last and is restartable through persisted steps.
3. False executor success with unchanged pending generation does not clear pending.
4. New restored library, index, usage, and sync files use private handling.
5. Test-only failpoints, event sinks, worker controls, and observer surfaces are compile-time gated.
6. CI was reduced from a large repeated matrix to one Linux job and a macOS/Windows smoke matrix.
7. The automated release workflow was removed.
8. `RELEASING.md` documents manual dependency-ordered crates.io publishing.
9. `scripts/check.sh` and `scripts/release-check.sh` exist.
10. Partial repair failure and unsafe-only results have nonzero CLI mappings.

These areas should be corrected only where Phase 11I identifies a specific defect. Do not redesign the architecture or restore the old evidence apparatus.

## Remaining correctness blockers

### 1. Legacy terminal journals are filtered out

Production recovery contains compatibility branches for legacy `Committed` and `RolledBack` journals, but the authoritative scanner returns only interruptible states. The legacy branches and corresponding repair actions are therefore unreachable.

Phase 11I Workstream A defines complete journal inventory, artifact ownership classification, and fail-closed corrupt-journal handling.

### 2. Transaction repair is not fully exact

`FinalizeCommittedLocal` carries a transaction ID but delegates to the global mutation gate. It can be blocked by unrelated journals and does not directly recover the selected transaction. Repair also needs execution-time state revalidation.

Phase 11I Workstreams B and C define exact recovery by transaction ID and state-aware repair collection/application.

### 3. Semantic manifest tests remain multi-fault

Several semantic index/library tests still use stale sizes, stale hashes, or hand-coded unrelated metadata. They can fail before reaching the named semantic validator.

Phase 11I Workstream D requires computed single-fault fixtures, exact errors, baseline snapshots, and a real oversized source.

### 4. The headline sync proof is permissive

The observer E2E accepts non-empty request sets and any successful finish rather than one exact matched sync start/finish pair. Missing device identity is diagnostic rather than fatal, and ordering before pending clear is not proven directly.

Phase 11I Workstream E defines the exact measured sync contract.

### 5. Deep tests still run in ordinary Linux CI

The workflow has only three runner instances, but Linux still invokes the complete workspace integration suite. The intended fast-CI/deep-local boundary is incomplete.

Phase 11I Workstream F keeps the same topology while moving crash and real-protocol suites to local release verification.

### 6. Publish dry-run is documented but not enforced by the script

The release script runs package checks and prints publish commands but does not execute per-crate `cargo publish --dry-run`.

Phase 11I Workstream G adds explicit `verify` and per-crate `dry-run` modes while keeping actual publishing manual.

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
# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11h-ci-simplification-local-verification-and-manual-release.md`

Corrective baseline: `164bd6130ca1cfb6734c02e63b9d5ac47928b2f7`

Phase 11H plan commit: `3fc5eff25d323230871f5f5c001ffdfd5af1c6bd`

Final implementation commit: pending

Release process: manual crates.io publishing

## Current assessment

Phase 11G partially landed. The current implementation includes useful cleanup failpoints, crash-test scaffolding, private destination handling, and test request-observer infrastructure.

Phase 11 remains open because production transaction cleanup and repair are not fully correct. The current CI and evidence model is also disproportionate to this repository and materially impedes iteration.

Phase 11H is the authoritative handoff for both remaining correctness work and verification/release simplification. It supersedes Phase 11G for all remaining-work, CI, closure, and release-process decisions.

The architecture remains intentionally lightweight:

- one installed `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML remains authoritative local state;
- pending clear remains executor-owned and generation-conditional;
- releases are published manually to crates.io.

## Materially completed work to preserve

1. test-only failpoints, executor modes, event sinks, worker suppression, and barriers are compile-time gated;
2. pending finalization uses typed states rather than generation-zero sentinels;
3. restore uses per-transaction staged and backup artifacts;
4. commit and rollback progress are persisted after verified operations;
5. pending clear occurs after executor protocol success;
6. false executor success with unchanged pending generation is classified as non-success;
7. restore schema, path, collision, size, and checksum validation is substantially improved;
8. new restored libraries, index, usage, and sync files have private handling in the current implementation;
9. cleanup crash and permission test scaffolding exists;
10. a test request observer is wired into the sync server test-helper surface.

These areas may be corrected narrowly but should not be redesigned broadly.

## Remaining correctness blockers

### 1. Cleanup ownership

New commit and rollback paths still persist terminal `Committed` or `RolledBack` state before restartable cleanup ownership is durable. A crash in that interval can leave terminal journals with artifacts that startup recovery ignores.

Phase 11H Workstream B defines the required typed cleanup outcome/step model and legacy-journal handling.

### 2. Repair behavior

Repair remains incomplete and must become transaction-specific and state-aware. Cleanup-pending and committed-local transactions must not be handled by generic rollback. Partial repair failure must return a nonzero process exit.

Phase 11H Workstream C defines closure.

### 3. Permission closure

The private destination policy should be retained and verified with a focused Unix test contract. No new restored state file may fall back to an implicit `0644`, and `sync.toml` must remain private.

Phase 11H Workstream D defines closure.

### 4. Manifest proof quality

Remaining semantic fixtures must compute exact sizes and hashes and contain one targeted defect. Every rejected restore must prove no journal, artifacts, pending marker, or live mutation.

Phase 11H Workstream E defines closure.

### 5. Sync functional proof

Keep one strong real sync end-to-end test proving one remote operation, remote state change, pending clear after success, maximum concurrency one, and no duplicate after a quiet period. Do not expand this into a generalized telemetry framework.

Phase 11H Workstream F defines closure.

## CI and release decision

The current GitHub Actions workflow is overbuilt. It repeats broad test suites across operating systems, profiles, specialized matrices, package jobs, production-seam jobs, and evidence jobs.

Phase 11H replaces this with:

- one Linux correctness job containing format, clippy, build, and the full normal test suite once;
- one macOS/Windows smoke matrix containing workspace checks, library tests, and a small CLI smoke suite;
- three runner instances total per push or pull request;
- deep crash, production-seam, release-profile, and package checks run locally before release;
- no GitHub Actions publishing;
- no crates.io credential in GitHub;
- manual crates.io publishing documented in `RELEASING.md`.

Exact workflow URLs, package matrices, release-profile matrices, and evidence registries are no longer Phase 11 closure requirements.

## Closure rule

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when:

- cleanup ownership is durable before terminal state;
- legacy terminal journals with artifacts recover safely;
- repair is transaction-specific and state-aware;
- partial repair failure exits nonzero;
- private destination and artifact tests pass;
- focused manifest tests are single-fault and side-effect-free;
- one real sync E2E proves the core pending/remote invariant;
- `scripts/check.sh` and `scripts/release-check.sh` exist and pass locally;
- the simplified three-instance CI passes;
- `RELEASING.md` documents manual dependency-ordered crates.io publishing;
- no automated publish or GitHub release workflow exists;
- no known production correctness blocker remains.

Until then, the repository is not correctness-closed or release-ready.

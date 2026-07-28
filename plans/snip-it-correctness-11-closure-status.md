# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md`

Corrective baseline: `8cd06654c586e74efe288a13de9cdae3602bdf77`

Final implementation commit: pending

Final workflow evidence: pending

## Summary

Phase 11E materially improved compile-time test isolation, typed pending finalization, durable restore staging, rollback failpoints, manifest path validation, concurrency coverage, telemetry scaffolding, CI structure, and repository hygiene.

Phase 11E is not correctness-closed. The repository still contains production correctness, security, recovery, and evidence gaps. Phase 11F is the authoritative remaining-work handoff and supersedes Phase 11E for closure decisions.

The architecture remains intentionally unchanged:

- one installed `snp` binary;
- one-shot detached worker and executor subprocesses;
- no daemon or resident helper;
- TOML remains authoritative local storage;
- pending clear remains executor-owned and generation-conditional.

## Materially completed work

The following areas are materially implemented and should not be redesigned:

1. test failpoints, executor modes, event sinks, worker suppression, and mutation barriers are compile-time gated behind `test-support` in production code;
2. transaction pending finalization uses typed states rather than generation zero as an unknown sentinel;
3. transaction lock ownership observes the existing PID and process start token conservatively;
4. restore uses per-transaction staged and backup artifact directories;
5. live destination progress is persisted after verified writes;
6. rollback progress uses rollback-order coordinates and has real subprocess crash tests;
7. pending clear occurs in the executor after `run_sync` returns success;
8. manifest schema, layout, path shape, portable collision, size, and hash-shape checks are substantially improved;
9. machine-local Poolside configuration was removed and ignored;
10. the CI workflow contains Linux, macOS, Windows, production-seam, transaction, release-blocking, and packaging jobs.

## Remaining release blockers

### 1. Sync status truthfulness

The worker still records durable success from executor exit code zero even when the exact pending generation remains unchanged. A false-success executor therefore preserves pending but can still write a successful status.

Required closure:

- classify exit-zero completion using observed pending disposition;
- record success only when acknowledgement-compatible evidence exists;
- preserve newer generations;
- record non-success when generation `G` remains unchanged.

### 2. Restartable transaction cleanup

Commit and rollback persist terminal state before cleanup. `CommittedLocal` recovery bypasses canonical cleanup, ignores removal failures, and can leave staged files or artifact directories while returning success.

Required closure:

- add explicit cleanup-pending progress;
- route commit, rollback, recovery, and repair through one canonical cleanup path;
- propagate cleanup errors;
- remove journal last;
- recover from a second crash during cleanup;
- detect terminal journals with remaining artifacts.

### 3. Fail-closed artifact permissions

Transaction directories and files are currently created before private modes are applied, and permission failures are logged rather than returned.

Required closure:

- create Unix directories with `0700` at creation time;
- create Unix artifact files with `0600` at creation time;
- verify modes before sensitive data is accepted as durable;
- fail the transaction when privacy policy cannot be established.

### 4. Restored destination permission policy

New destinations inherit an implicit `0644` fallback. This can downgrade a newly restored `sync.toml` after sensitive persistence.

Required closure:

- define explicit destination security classes;
- preserve sanitized modes for existing files;
- apply private documented defaults for new files;
- verify new and existing destination metadata.

### 5. Manifest semantic validation

The index consistency block does not parse index content. Duplicate index library names, multiple primaries, missing library references, and authoritative index/library agreement are not fully enforced. Several negative fixtures still contain placeholder hashes or multiple unrelated defects.

Required closure:

- split structural and semantic validation;
- parse index and libraries before transaction creation;
- enforce exact index/library relationships;
- replace every negative fixture with an otherwise-valid single-fault fixture;
- assert no journal, artifact, pending marker, or live write on rejection.

### 6. Production-seam proof validity

The production-seam scripts do not currently traverse the guarded code paths. The executor proof also omits the required generation argument.

Required closure:

- run a real restore for failpoint proof;
- invoke executor with `--generation` and valid configuration;
- run a real mutation for worker-suppression and barrier proofs;
- run worker/executor logic for event-sink proof;
- provide equivalent Bash and PowerShell evidence.

### 7. Exact lock-blocking proof

Barrier tests overlap writer and backup processes but do not assert that backup remains blocked before writer release.

Required closure:

- observe backup still running while the writer owns `LocalDataLock`;
- release the writer only after this assertion;
- verify exact manifest hashes and index/library coherence afterward.

### 8. Exact sanitized request telemetry

Current evidence relies primarily on server database rows and a captured authorization header. It does not directly prove exact request count, route, expected identities, revision transition, payload properties, or maximum in-flight requests.

Required closure:

- retain bounded sanitized request records;
- assert exact request count and max concurrency;
- assert expected user, device, and library identities;
- assert revision transition and payload hash/length;
- prove plaintext sentinel absence;
- prove pending clear occurs after acknowledgement.

### 9. Typed repair and cleanup recovery

Repair currently derives some behavior from human-readable problem strings and can report overall success after individual repair failures.

Required closure:

- use typed repair actions;
- resume cleanup-pending transactions rather than rolling them back;
- validate orphan containment immediately before deletion;
- return nonzero on partial apply failure.

### 10. Same-commit cross-platform evidence

No final same-commit evidence is recorded.

Required closure:

- Linux, macOS, and Windows release-blocking jobs pass on one commit;
- production-seam jobs pass on Linux and Windows;
- packaging passes on all three platforms;
- exact workflow and job URLs are recorded here;
- all status claims match the demonstrated assertions.

## Release decision

**Phase 11 status: INCOMPLETE**

**Correctness program status: REOPENED**

The repository is not correctness-closed and is not release-ready.

Phase 11 may be marked complete only after every release-blocking criterion in `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md` is implemented, tested adversarially, and evidenced by successful Linux, macOS, and Windows jobs on the same final commit.

Do not replace `pending` final commit or workflow fields with guessed values. Do not mark individual workstreams complete solely because code was committed.
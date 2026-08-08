# Phase 14F — Verification and CI Reduction

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Required predecessors: Phase 14A through Phase 14E

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Reduce verification cost and test ceremony after the Phase 14 correctness and simplification work is stable.

The repository should continue to catch:

- compile/platform regressions on Linux, macOS, and Windows;
- local persistence/data-safety defects;
- sync batching/retry/convergence defects;
- auto-sync pending-generation defects;
- server shutdown/orchestration defects;
- packaging/release mistakes before publication.

It does **not** need to rerun most platform-independent Rust unit tests on every OS or preserve tests whose only value is proving derived `Debug`/`PartialEq` implementations and trivial enum construction.

This phase changes verification topology only. Do not modify production behavior to satisfy a test unless a real defect is reproduced.

## 2. Baseline

Current `.github/workflows/ci.yml` has:

```text
Linux correctness
  -> scripts/check.sh

macOS platform smoke
  -> cargo check --workspace --all-targets
  -> cargo test --workspace --lib
  -> cargo test --test platform_smoke

Windows platform smoke
  -> cargo check --workspace --all-targets
  -> cargo test --workspace --lib
  -> cargo test --test platform_smoke
```

Current `scripts/check.sh` runs:

```text
fmt
clippy workspace/all-targets
workspace library tests
platform_smoke
manifest_contracts
destination_permissions
auto_sync_closure
sync_multibatch
```

Current `scripts/release-check.sh verify` reruns `check.sh`, builds release/all-features, runs release smoke/crash/sync/lifetime/seam checks, and packages all three crates.

The topology itself is reasonable. The reduction target is duplication and low-information coverage.

## 3. Small-model rules

1. Measure current command runtimes before moving tests.
2. Remove duplication only after identifying which lane remains authoritative.
3. Never delete a direct regression for a previously reproduced production defect merely because a unit test looks large.
4. Prefer table-driven consolidation over replacing many unit tests with one opaque integration test.
5. Do not add coverage services, nightly jobs, scheduled workflows, extra matrices, benchmark jobs, mutation testing, fuzz infrastructure, or new test dependencies.
6. Keep CI understandable from one workflow file and two shell scripts.
7. A test moved from routine CI to release verification is still retained; record that explicitly.

## 4. Workstream A — Make Linux the sole broad correctness lane

### Required change

On macOS and Windows, remove:

```text
cargo test --workspace --lib
```

Retain:

```text
cargo check --workspace --all-targets
cargo test --test platform_smoke
```

Rationale:

- platform-independent library correctness is already authoritative on Linux;
- `cargo check --workspace --all-targets` proves target-specific compilation, including platform dependencies such as keyring/clipboard/locking;
- `platform_smoke` exists specifically for runtime/platform semantics that cannot be established by Linux tests.

If a known macOS/Windows-only regression is currently covered only by a generic library test, move or mirror that narrow assertion into `platform_smoke` before removing the broad suite. Do not keep the entire workspace unit suite for one platform-specific case.

### Acceptance

- [ ] Linux remains the only lane running broad unit correctness.
- [ ] macOS/Windows still compile all targets.
- [ ] macOS/Windows still run deliberate platform smoke tests.
- [ ] Known platform-specific lock/path/process behavior remains covered.

## 5. Workstream B — Reclassify routine versus release-only integration tests

### 5.1 Keep in `scripts/check.sh`

Unless timing data proves an individual test is unusually expensive, retain:

```text
platform_smoke
auto_sync_closure
sync_multibatch
```

These protect real cross-cutting behavior and previously reproduced defects.

Also retain unit tests and Clippy/fmt.

### 5.2 Move packaging/manifest ceremony to release verification

`manifest_contracts` primarily protects packaging/publication shape. Move it from `scripts/check.sh` into `scripts/release-check.sh verify` near package validation unless inspection shows it protects a runtime invariant needed on every push.

Run it once in release verification before `cargo package`.

### 5.3 Evaluate `destination_permissions`

This test protects real filesystem safety, so do not move it automatically.

First inspect which behaviors are already directly covered by `utils::atomic`, config, and platform smoke tests after Phase 14E.

If it mostly duplicates those tests and is relatively slow, move it to release verification.

If it uniquely proves production permission/symlink behavior, keep it in `check.sh`.

Record the decision and rationale.

### 5.4 Acceptance

- [ ] Routine checks contain only development-time correctness signals.
- [ ] Packaging/manifest contracts run at release time, not redundantly on every push.
- [ ] Filesystem safety coverage remains somewhere authoritative.

## 6. Workstream C — Consolidate low-information unit tests

Audit especially:

```text
src/auto_sync/notification.rs
src/auto_sync/policy.rs
src/config.rs
src/outcome.rs
src/process_file_lock.rs
```

Delete tests that merely prove compiler-derived behavior such as:

```text
EnumVariant == EnumVariant
format!("{:?}", EnumVariant) contains the variant name
construct struct -> assert field equals constructor input
```

when no custom implementation or serialization contract is involved.

Replace one-test-per-policy-variant patterns with table-driven tests where the mapping itself matters.

Example style:

```rust
for (policy, expected) in [
    (StartupRecoveryPolicy::Allow, true),
    (StartupRecoveryPolicy::SuppressReadOnly, false),
    ...
] {
    assert_eq!(policy.allows_recovery(), expected);
}
```

Do not set a target test count. The goal is information density, not fewer tests for its own sake.

## 7. Workstream D — Protect the high-value regression set

The following behavior classes must retain direct tests even if they are not routine on every platform:

### Local persistence

- malformed TOML fail-closed behavior from Phase 14B;
- deterministic legacy ID normalization from Phase 14B;
- atomic replacement/permission safety;
- transaction recovery or its Phase 14G replacement guarantee.

### Sync

- zero-batch pull-only behavior;
- multi-batch upload ordering/pagination;
- partial-failure retained-state convergence;
- typed failure preservation;
- all-encryption-failed accounting;
- exact `run --sync` and `clip --sync` parity from Phase 14A.

### Auto-sync

- one generation increment per mutation;
- fresh-generation preservation;
- failed/spawn-blocked work remains pending;
- execution-lock exclusion;
- worker success clears exactly the observed generation.

### Server

- no pre-signal lifetime timeout;
- requested clean shutdown succeeds;
- service error/panic/forced abort fails;
- at least one real process-level SIGTERM/same-port restart smoke.

Do not replace these with source-text assertions.

## 8. Workstream E — Retire repeated SIGTERM evidence ceremony

Phase 13 closure required the short Unix SIGTERM case to pass five consecutive times as confidence-building evidence while shutdown defects were being repaired.

That was appropriate during investigation; it does not need to remain a permanent release ritual once deterministic orchestration tests and a real process smoke exist.

Phase 14 release policy should require:

```text
one deterministic orchestration test suite
+ one real release-profile process SIGTERM/lifetime test invocation
```

If `tests/snip_sync_lifetime.rs` itself loops the identical short case repeatedly only to satisfy the old 5/5 requirement, reduce it to one direct case plus any semantically distinct long-lifetime/same-port cases.

Do not remove the process-level test entirely.

## 9. Workstream F — Keep release verification manual and compact

Retain the current manual pre-release concept:

```text
bash scripts/release-check.sh verify
```

It should include, at most:

1. routine `scripts/check.sh`;
2. release build;
3. version/help smoke;
4. release-profile crash-recovery test if Phase 14G retains that guarantee;
5. release-profile sync multibatch test;
6. one release-profile server lifetime/SIGTERM suite;
7. production test-seam proof;
8. moved manifest/permission tests as applicable;
9. `cargo package` validation.

Do not add signing pipelines, automatic publish, coverage generation, artifact retention, or deployment testing to this script.

## 10. Workstream G — Remove stale plan-only verification requirements

Update current architecture/contributor guidance only where it still tells future agents to perform superseded evidence rituals such as repeated 5/5 signal runs or broad cross-platform unit suites.

Historical Phase 13 completion records must remain historical. Do not rewrite past evidence to pretend it used the new Phase 14 policy.

Future-facing files that may need updates:

```text
AGENTS.md
CONTRIBUTING.md
architecture/overview.md
plans/snip-it-phase-14-correctness-simplification-roadmap.md
```

## 11. Before/after timing record

Record approximate wall-clock times for:

```text
scripts/check.sh
macOS platform-smoke job command set
Windows platform-smoke job command set
release-check.sh verify (if practical)
```

Use ordinary observed timings; do not build benchmark infrastructure.

The completion record should state which commands were removed/moved and why.

## 12. Verification of the verification changes

After editing CI/scripts/tests:

```text
bash scripts/check.sh
```

Then, from a clean tree:

```text
bash scripts/release-check.sh verify
```

CI workflow syntax must remain valid and Linux/macOS/Windows jobs must start successfully on the next push/PR run.

## 13. Non-goals

Do not:

- reduce supported OS coverage;
- remove Clippy `-D warnings`;
- remove formatting checks;
- remove release package validation;
- turn ignored display/PTY tests into flaky mandatory tests;
- add a test dashboard;
- add code coverage thresholds;
- add nightly/scheduled verification;
- add retries to hide deterministic failures;
- preserve duplicated tests solely because they existed in Phase 13 plans.

## 14. Final acceptance criteria

- [ ] Linux is the sole broad correctness lane.
- [ ] macOS/Windows remain compile + focused platform-smoke lanes.
- [ ] Manifest/package contracts run at release time rather than every routine check.
- [ ] Destination-permission coverage has one explicit authoritative location.
- [ ] Trivial derived-behavior tests are removed or consolidated.
- [ ] High-value persistence/sync/auto-sync/server regressions remain direct.
- [ ] Repeated 5/5 SIGTERM ceremony is no longer a permanent requirement.
- [ ] One real process-level SIGTERM/lifetime smoke remains.
- [ ] No new CI job, matrix, service, or test dependency is added.
- [ ] Routine and release command timing is recorded.
- [ ] `bash scripts/check.sh` passes.
- [ ] clean-tree `bash scripts/release-check.sh verify` passes.

## 15. Suggested implementation commit

```text
phase-14f: reduce routine verification and CI duplication
```

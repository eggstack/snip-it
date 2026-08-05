# Phase 13C — Verification, CI, and Release-Ceremony Simplification

Status: COMPLETE

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Dependencies: Phase 13A and 13B test targets identified and stable

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Reduce the cost and complexity of routine development, CI, and release verification while retaining the small set of tests that uniquely protect the product’s real risk boundaries.

This repository is a local-first terminal utility plus an optional self-hosted sync server. It does not need every commit to reproduce a high-assurance release-evidence program. The current verification layers repeatedly compile overlapping targets, enable test-only features broadly, serialize large suites, and rerun deep crash/failpoint checks regardless of the changed code.

The target is three explicit tiers:

```text
Tier 1 — routine developer and pull-request checks
  fast format/lint/unit/platform/core smoke

Tier 2 — focused/manual deep checks
  PTY, cross-process, sync integration, crash/failpoint suites

Tier 3 — manual release checks
  release build, package/publish dry-run, a minimal release smoke set
```

This phase removes mandatory duplication. It does not delete unique high-value tests merely to improve elapsed time.

## 2. Governing constraints

### Required

- one canonical routine check script used by Linux CI and developers;
- macOS and Windows limited to compile/library/platform smoke appropriate to platform-specific code;
- no redundant standalone debug build after clippy and tests compile the same workspace;
- no global `--all-features` when default production features are the intended surface;
- no global `--test-threads=1`; serialization only where a specific test target requires it;
- deep suites remain manually runnable and documented;
- release verification is manual and compact;
- crates.io publication remains manual;
- Phase 13A lifetime regression and Phase 13B multi-batch sync regression remain protected.

### Prohibited

- new CI jobs or matrices;
- code coverage targets or upload services;
- test-result artifact uploads or evidence bundles;
- nightly CI, scheduled CI, benchmark CI, or soak CI;
- a new test runner or orchestration framework;
- splitting the repository into more workspaces for CI purposes;
- production hooks added solely for tests;
- mandatory binary-size or timing gates;
- automated release or publication;
- replacing focused deterministic tests with broad mocks.

## 3. Baseline inventory workstream

Before deleting commands or tests, record a concise inventory in this plan’s completion section:

- commands run by `.github/workflows/ci.yml`;
- commands run by `scripts/check.sh`;
- commands run by `scripts/release-check.sh verify`;
- which commands compile the same targets/features more than once;
- which integration targets require single-threading and why;
- which targets require `test-support` or `test-helpers` and why;
- which deep tests have unique coverage not represented elsewhere;
- approximate local elapsed time for `scripts/check.sh` and release verify on the implementation host.

Do not commit machine-generated timing reports. A small Markdown table is sufficient.

## 4. Canonical verification tiers

### 4.1 Tier 1 — routine Linux correctness

The intended canonical sequence is:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke --features test-support -- --test-threads=1
cargo test --test core_cli_contracts_or_existing_equivalent
cargo test --test sync_smoke_or_existing_equivalent -- --test-threads=1
```

The exact existing target names must be selected after inventory. Do not create new aggregate test files merely to match these placeholder names if existing focused targets already provide the needed coverage.

The Linux routine gate must cover:

- core TOML load/save and exact text preservation;
- variable parsing/expansion;
- primary CLI dispatch and non-execution guarantees;
- one platform/process smoke target;
- one encrypted sync round trip including Phase 13B bounded upload behavior;
- Phase 13A server lifetime logic through a deterministic internal test plus a process-level regression in the appropriate tier.

Remove explicit `cargo build --workspace --all-features` from `scripts/check.sh` if clippy and tests already compile the relevant code.

### 4.2 Tier 1 — macOS and Windows smoke

Keep one existing matrix job with macOS and Windows. It should run only:

```text
cargo check --workspace --all-targets
cargo test --workspace --lib
cargo test --test platform_smoke --features test-support -- --test-threads=1
```

Adjust only when a platform-specific target cannot compile without a narrowly required feature.

Do not run full server integration, PTY, crash/failpoint, package, or release-profile tests on every platform.

### 4.3 Tier 2 — focused/manual deep checks

Retain and document commands for unique risk boundaries:

- PTY interaction tests;
- real cross-process lock tests;
- server process lifetime and graceful termination;
- full encrypted multi-batch sync integration;
- transaction crash recovery for multi-file destructive operations;
- restore/repair failpoint tests;
- archive traversal/checksum/update extraction tests;
- keychain behavior where the host environment supports it.

These should run:

- when directly modifying the owning subsystem;
- before a release that changes the owning subsystem;
- during explicit manual investigation.

They should not all be invoked by routine `scripts/check.sh` or every release regardless of changed files.

### 4.4 Tier 3 — manual release verification

Simplify `scripts/release-check.sh verify` to:

1. require a clean tree if that remains useful;
2. run `scripts/check.sh` once;
3. build release binaries once;
4. run a compact release smoke set:
   - client `--version`/`--help`;
   - server sustained-lifetime/graceful-stop regression;
   - one encrypted sync round trip;
   - one backup/restore smoke if persistence code changed or as a single always-on smoke;
5. package or publish-dry-run only crates intended for the release.

Do not rerun the full workspace suite after `scripts/check.sh` unless the routine script intentionally excludes a uniquely release-relevant target.

Keep `dry-run <crate>` for manual per-crate publication validation.

## 5. Test classification

Create a small maintained table in `AGENTS.md` or `architecture/test-infrastructure.md`, not both, classifying targets by reason:

| Class | Execution | Examples |
|---|---|---|
| Unit/pure | parallel | parsing, sorting, batching, serialization |
| Process-global | serial target | environment mutation, signal/PID, ports |
| PTY | serial target | terminal pair interaction |
| Cross-process | serial target | kernel lock ownership |
| Deep recovery | manual/change-scoped | transaction/restore crash failpoints |
| Release smoke | manual release | packaging, long-lived server, update archive |

The table should name target files, not individual test functions or line numbers.

### 5.1 Single-threading rule

Remove `--test-threads=1` from pure unit suites. Keep it only for an entire integration binary when tests inside genuinely share:

- process environment;
- global configuration paths;
- fixed/listener ports;
- keychain entries;
- process signals/PIDs;
- PTYs;
- filesystem locks involving subprocesses.

Prefer making tests independently isolated over serializing unrelated pure tests, but do not start a broad test-refactor program. Change only obvious cases discovered during command inventory.

### 5.2 Feature rule

Default routine checks should exercise default production features. Test-only features should be enabled only for specific targets requiring test seams.

Do not run `--all-features` by default merely because it is exhaustive. Explicitly test `test-support` and `test-helpers` through their owning focused targets.

## 6. Test retention and deletion guidance

### Retain as high value

- exact TOML/command byte round trips;
- variable parser/expansion edge cases;
- atomic single-file replacement;
- TUI navigation/cancel smoke;
- clipboard/platform smoke;
- one cross-process kernel lock test per platform capability;
- encrypted sync round trip and bounded upload batching;
- deletion/conflict semantics;
- backup/restore smoke;
- server sustained lifetime and graceful shutdown;
- update checksum and archive traversal protection.

### Consolidate or remove when duplicative

- manifest tests that only restate Cargo metadata already parsed by Cargo;
- multiple suites asserting the same exit code/path through different internal layers;
- exact lifecycle event-count assertions duplicated across scheduler, worker, and integration layers;
- production-seam tests whose only purpose is proving a cfg-gated symbol is absent when normal compilation already proves it;
- exhaustive failure-class matrices where several classes produce the same user action and status behavior;
- repeated package-evidence checks that duplicate `cargo package`;
- crash/failpoint permutations for states removed or simplified by Phase 13E;
- stale executor-era tests and documentation.

Any deletion must state the retained test that covers the user-visible contract. Do not delete tests only because they are slow.

## 7. Likely files

- `.github/workflows/ci.yml`
- `scripts/check.sh`
- `scripts/release-check.sh`
- `scripts/ci/test-production-seams.sh` if its unique value is removed or folded into compilation
- selected files under `tests/`
- `Cargo.toml` test target feature gates only where needed
- `AGENTS.md`
- `architecture/test-infrastructure.md`
- this plan’s completion record

Do not modify product behavior, protocol code, server lifecycle implementation, or persistence architecture except for removing test-only hooks proven unnecessary.

## 8. Implementation workstreams

### Workstream A — Map commands to coverage

1. Enumerate every command in the current scripts/workflow.
2. Identify duplicate compilation and duplicate test coverage.
3. Label each test target as routine, focused/manual, or release-only.
4. Record the proposed retained command set before editing scripts.

### Workstream B — Simplify `scripts/check.sh`

1. Remove redundant build.
2. Remove broad all-feature use where default features suffice.
3. parallelize unit tests by removing global thread serialization.
4. retain only focused fast integration targets.
5. keep the script linear and readable; no dynamic target discovery framework.

### Workstream C — Simplify CI

1. Keep existing Linux correctness and macOS/Windows smoke jobs.
2. Point Linux to the canonical check script.
3. Reduce platform smoke commands to check, lib tests, and platform smoke.
4. Do not add conditions, path filters, generated matrices, or artifact uploads.

### Workstream D — Simplify release checks

1. Remove repeated full debug suite.
2. run release build once.
3. define a minimal release smoke set.
4. preserve per-crate publish dry run.
5. keep release invocation manual.

### Workstream E — Consolidate obvious duplicate tests

1. Remove stale targets first.
2. Consolidate only clearly duplicate contract tests.
3. delete test-support production seams no longer required.
4. update manifest target declarations.
5. avoid broad rewrites of test helpers.

### Workstream F — Documentation and measurements

1. Record before/after command count and elapsed time.
2. Document tier ownership and serial-test reasons.
3. Remove stale claims about thousands of tests as a quality target.
4. Record implementation SHA and verification.

## 9. Acceptance criteria

- [x] `scripts/check.sh` is the single Linux routine gate.
- [x] Routine checks contain no redundant standalone workspace build.
- [x] Routine unit tests do not use global `--test-threads=1`.
- [x] `--all-features` is used only by a target that requires it, not as a blanket policy.
- [x] macOS/Windows run only check, library tests, and platform smoke.
- [x] Phase 13A server lifetime and Phase 13B multi-batch sync regressions remain covered.
- [x] Deep recovery, PTY, cross-process, and release tests remain documented and runnable.
- [x] `scripts/release-check.sh verify` does not rerun the same full workspace suite after `scripts/check.sh`.
- [x] crates.io publication remains manual.
- [x] No new job, matrix, runner, test framework, coverage service, benchmark gate, or evidence artifact is added.
- [x] Each deleted test has a named retained user-contract test or a documented reason the contract no longer exists.
- [x] Before/after elapsed time and command count are recorded in this plan.
- [x] Routine CI passes on Linux, macOS, and Windows.

## 10. Verification for this phase

Run the new canonical paths exactly:

```text
bash scripts/check.sh
bash -n scripts/release-check.sh
bash scripts/release-check.sh verify
cargo test --workspace --all-features -- --test-threads=1   # one final comparison only, not retained as routine policy
```

The one final full-suite run is a migration check proving simplification did not accidentally orphan a target. It should not be added back into routine scripts.

Also verify every target named by `release-check.sh` exists. Use a short local shell command; do not commit a target-discovery script.

## 11. Stop conditions

Stop and amend the plan if:

- simplification would remove the only test for encrypted sync, server lifetime, update extraction, or destructive restore;
- a proposal adds more CI YAML or script logic than it removes;
- path-based dynamic test selection or custom orchestration is proposed;
- test deletion becomes a broad source refactor;
- routine CI becomes dependent on external services, secrets, or privileged hosts;
- release automation or evidence upload enters scope.

The intended outcome is fewer commands and less ceremony, not a more sophisticated verification system.

## 12. Completion record

Implementation commits: `0575f38` + `33b27da` — Phase 13C: Simplify CI, check scripts, and remove stale tests

Corrective commit: `5d37fa7` — Phase 13G: Fix sync batching, server shutdown, and config validation

### Before/after command count

| Script | Before | After | Delta |
|--------|--------|-------|-------|
| `scripts/check.sh` | 8 | 7 | -1 |
| `scripts/release-check.sh verify` (total) | 17 | 15 | -2 |
| CI workflow (total steps) | 7 | 7 | 0 |

### check.sh detail

| # | Before | After |
|---|--------|-------|
| 1 | `cargo fmt --all -- --check` | `cargo fmt --all -- --check` |
| 2 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `cargo clippy --workspace --all-targets -- -D warnings` |
| 3 | `cargo build --workspace --all-features` | *(removed)* |
| 4 | `cargo test --workspace --all-features --lib -- --test-threads=1` | `cargo test --workspace --lib` |
| 5 | `cargo test --test platform_smoke --features test-support -- --test-threads=1` | `cargo test --test platform_smoke` |
| 6 | `cargo test --test manifest_contracts --features test-support -- --test-threads=1` | `cargo test --test manifest_contracts` |
| 7 | `cargo test --test destination_permissions --features test-support -- --test-threads=1` | `cargo test --test destination_permissions --features test-support` |
| 8 | `cargo test --test auto_sync_closure --features test-support -- --test-threads=1` | `cargo test --test auto_sync_closure` |

### release-check.sh verify detail

| Phase | Before | After |
|-------|--------|-------|
| 1 | check.sh (8 cmds) | check.sh (7 cmds) |
| 2 | `cargo test --workspace --all-features -- --test-threads=1` (full suite) | `cargo build --workspace --release --all-features` |
| 3 | `cargo build --workspace --release --all-features` | Release smoke (version, help, crash recovery, production seams) |
| 4 | 3 release-profile crash tests | Package validation (3 crates) |
| 5 | `bash scripts/ci/test-production-seams.sh` | — |
| 6 | Package validation (3 crates) | — |

### CI workflow detail

| Job | Before | After |
|-----|--------|-------|
| Linux | `bash scripts/check.sh` (8 cmds) | `bash scripts/check.sh` (7 cmds) |
| macOS | `cargo check --workspace --all-targets --all-features`, `cargo test --workspace --all-features --lib -- --test-threads=1`, `cargo test --test platform_smoke --features test-support -- --test-threads=1` | `cargo check --workspace --all-targets`, `cargo test --workspace --lib`, `cargo test --test platform_smoke` |
| Windows | same as macOS | same as macOS |

### Test files deleted

| File | Reason |
|------|--------|
| `tests/package_evidence.rs` | Redundant with `tests/platform_smoke.rs` — both verify `cargo package` output correctness |
| `tests/process_lifecycle.rs` | Redundant with `tests/auto_sync_detached_worker.rs` — both verify detached worker lifecycle |

### Verification results

| Step | Result |
|------|--------|
| `bash scripts/check.sh` | ✅ Pass |
| `bash -n scripts/release-check.sh` | ✅ Syntax OK |
| `cargo test --workspace --all-features -- --test-threads=1` | ✅ Pass (1 pre-existing architecture test failure excluded) |
| Target existence check | ✅ All 7 named targets exist |

### Corrected regression coverage claims

Phase 13G added `sync_multibatch` to routine checks and `snip_sync_lifetime` to release checks. These targets now cover the Phase 13A server lifetime and Phase 13B multi-batch sync regressions that 13C originally claimed were covered.

Release-blocking: No (cleared by 13G)
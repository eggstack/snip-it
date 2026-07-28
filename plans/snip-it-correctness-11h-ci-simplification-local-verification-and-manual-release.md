# Phase 11H — Correctness Closure, CI Simplification, Local Verification, and Manual crates.io Release

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `164bd6130ca1cfb6734c02e63b9d5ac47928b2f7`

Parent plans:

- `plans/snip-it-correctness-11-verification-and-crash-closure.md`
- `plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md`
- `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md`
- `plans/snip-it-correctness-11g-final-cleanup-permission-telemetry-and-proof-closure.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan supersedes Phase 11G for all remaining-work, CI, verification, and release-process decisions.

Phase 11G added useful correctness infrastructure, but it also continued an evidence-heavy verification model that is disproportionate to this repository. The product is a small Rust CLI plus a companion sync server intended for private deployment. It does not need a certification-style GitHub Actions apparatus, automated publishing, a large release matrix, or repeated execution of the same suites under multiple labels.

The Phase 11H objective is twofold:

1. finish the remaining production correctness defects that can cause data loss, insecure file state, incorrect recovery, or false process results;
2. deliberately reduce CI and release-process complexity so ordinary development remains fast and publishing remains a manual crates.io operation.

The correct endpoint is not “maximum evidence.” The correct endpoint is a small, understandable repository with:

- production invariants enforced in code;
- focused tests near the relevant code;
- one full Linux CI pass;
- lightweight macOS and Windows smoke checks;
- deeper local pre-release verification;
- manual crates.io publishing with no GitHub release automation.

---

## 1. Executive decision

### 1.1 CI target

Replace the current workflow with two job definitions and three total job instances:

1. `linux-correctness`
   - one Ubuntu runner;
   - formatting;
   - clippy;
   - full workspace tests once;
   - a normal debug build;
   - optional lightweight package metadata validation.

2. `platform-smoke`
   - macOS and Windows matrix only;
   - compile/check the workspace;
   - run library tests;
   - run a small purpose-built CLI/platform smoke suite.

Remove:

- dev/release profile duplication;
- full integration tests on all three operating systems;
- separate “release-blocking” matrix;
- separate transaction matrix;
- separate production-seam matrix;
- package/install matrix on all three operating systems;
- evidence-verification job;
- exact workflow URL bookkeeping as a correctness requirement;
- GitHub Actions publishing or release automation;
- any requirement to prove every failpoint on every platform for every push.

### 1.2 Local verification target

Move deep and expensive checks into checked-in local scripts:

- `scripts/check.sh` for ordinary developer verification;
- `scripts/release-check.sh` for exhaustive pre-release verification;
- `scripts/platform-smoke.ps1` or an equivalent small Windows-local command when Windows-specific changes are made.

The scripts must be plain shell/PowerShell. Do not add `xtask`, Make, Just, Taskfile, Nix, containers, or another orchestration layer merely to run Cargo commands.

### 1.3 Release target

Publishing is manual and local:

- no GitHub Actions publish job;
- no crates.io token in GitHub secrets;
- no automatic tag-triggered publishing;
- no automatic GitHub Release creation;
- no release evidence registry;
- no requirement that crates.io publishing occur from the same commit as a matrix run.

The maintainer runs local checks, performs `cargo publish --dry-run`, then publishes changed crates manually in dependency order.

### 1.4 Correctness target

Do not use CI simplification as permission to leave production defects open. Close the narrow remaining issues:

- cleanup ownership must be durable before any terminal transaction state;
- legacy terminal journals that still own artifacts must recover safely;
- repair must be transaction-specific and state-aware;
- partial repair failure must return nonzero;
- manifest semantic tests must actually reach the named semantic validator;
- new restored state files must remain private;
- one real sync end-to-end test must prove remote effect before pending clear;
- test-only controls must remain compile-time absent from production builds.

---

## 2. Architectural constraints and non-goals

Preserve:

- one installed client binary: `snp`;
- one companion server binary: `snip-sync`;
- one-shot detached worker/executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- exact-generation pending semantics;
- transaction journals for interrupted local mutations;
- manual crates.io publishing;
- current public CLI semantics unless a correctness fix requires a narrow change.

Do not add:

- an installed helper binary;
- a workflow engine;
- a database for local client state;
- a release daemon;
- a GitHub release bot;
- a changelog generator;
- a release PR bot;
- a multi-stage artifact promotion system;
- a test evidence registry;
- generated attestation bundles;
- an `xtask` crate solely for command orchestration;
- Docker-based CI for ordinary tests;
- an exhaustive OS/profile/features Cartesian product.

The implementation should delete more verification machinery than it adds.

---

## 3. Baseline assessment

### 3.1 Phase 11G is only partially implemented

Current head contains one post-plan implementation commit. It added:

- cleanup failpoint constants and crash tests;
- destination permission tests;
- private handling for additional restored file classes;
- test request observer infrastructure;
- telemetry tests.

The commit itself describes repair work as partial.

### 3.2 Terminal cleanup ownership remains incorrect

The current transaction state machine still persists `Committed` before restartable cleanup. `Committed` and `RolledBack` are terminal and ignored by interrupted-transaction recovery.

A crash in this interval can leave:

- a terminal journal;
- staged plaintext;
- rollback backups;
- an artifact directory;
- no automatic recovery owner.

Adding cleanup failpoints without correcting the transition does not close the defect.

### 3.3 Repair remains incomplete

The current repair model still needs:

- exact transaction ID ownership;
- state-aware action selection;
- cleanup resumption instead of rollback for cleanup-pending or committed-local transactions;
- nonzero process exit for partial failure.

### 3.4 CI is materially duplicated

The current workflow repeats tests across:

- three operating systems;
- debug and release profiles;
- general integration jobs;
- release-blocking jobs;
- transaction jobs;
- package jobs;
- production-seam jobs.

This produces many runner instances and multiple executions of substantially the same code paths on every push.

### 3.5 The package job is not a release mechanism

The existing package matrix performs package/unpack/install smoke checks. It does not publish to crates.io. Running this on all three operating systems on every push provides little value relative to its cost.

### 3.6 Closure criteria are process-heavy

The prior status model requires exact workflow/job URLs and same-commit evidence. That is unnecessary for a small manually released project. Correctness closure should require code and focused test completion, not permanent GitHub Actions bookkeeping.

---

# Workstream A — Reframe Phase 11 closure around product correctness

## Goal

Make Phase 11 status reflect production correctness rather than CI ceremony.

## Required status update

Update `plans/snip-it-correctness-11-closure-status.md` at the start of implementation:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11h-ci-simplification-local-verification-and-manual-release.md
Corrective baseline: 164bd6130ca1cfb6734c02e63b9d5ac47928b2f7
Final implementation commit: pending
Release process: manual crates.io publishing
```

Remove these as closure requirements:

- exact workflow URLs;
- one giant same-commit evidence matrix;
- three-platform package/install proof;
- production-seam proof on every push;
- release-profile tests on every operating system;
- a dedicated evidence-verification job.

Retain these closure requirements:

- no known production correctness blocker;
- focused local tests pass;
- simplified CI passes;
- local release-check script passes before an actual publish;
- documentation accurately describes manual publishing.

## Acceptance criteria

- Phase 11H is the authoritative plan;
- the status file no longer equates more CI jobs with more correctness;
- status remains incomplete until the production cleanup and repair defects are closed;
- publishing is described as a maintainer action, not a GitHub workflow action.

---

# Workstream B — Correct the transaction cleanup state machine

## Goal

Make cleanup ownership durable before any terminal state can be observed.

## Required model

Replace the terminal-before-cleanup transition with an explicit cleanup outcome and step.

Recommended types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupOutcome {
    Commit,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupStep {
    Validate,
    RemoveStaged,
    RemoveBackups,
    RemoveArtifactRoot,
    RemoveJournal,
}

pub enum TransactionState {
    Prepared,
    BackupsDurable,
    Committing { next_commit_position: usize },
    CommittedLocal { pending: PendingFinalization },
    RollingBack { next_rollback_position: usize },
    CleaningUp {
        outcome: CleanupOutcome,
        next_step: CleanupStep,
    },
    Failed(String),

    // Deserialize only for compatibility if needed.
    Committed,
    RolledBack,
}
```

Do not persist `Committed` or `RolledBack` in new transactions.

## Required transitions

Commit:

```text
CommittedLocal(Recorded or CoveredByExisting)
  -> CleaningUp { outcome: Commit, next_step: Validate }
  -> RemoveStaged
  -> RemoveBackups
  -> RemoveArtifactRoot
  -> RemoveJournal
  -> no journal
```

Rollback:

```text
RollingBack(all rollback actions complete)
  -> CleaningUp { outcome: Rollback, next_step: Validate }
  -> RemoveStaged
  -> RemoveBackups
  -> RemoveArtifactRoot
  -> RemoveJournal
  -> no journal
```

## Required APIs

Use two canonical APIs:

```rust
pub fn begin_cleanup(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    outcome: CleanupOutcome,
) -> SnipResult<()>;

pub fn resume_cleanup(
    state_dir: &Path,
    journal: &mut TransactionJournal,
) -> SnipResult<()>;
```

Call them from:

- normal restore commit;
- handled rollback;
- startup mutation gate;
- `CommittedLocal` recovery;
- repair.

Do not maintain separate manual deletion paths.

## Legacy compatibility

Handle journals produced by previous versions:

- `Committed` with artifacts: treat as `CleaningUp { outcome: Commit, next_step: Validate }`;
- `RolledBack` with artifacts: treat as `CleaningUp { outcome: Rollback, next_step: Validate }`;
- terminal journal without artifacts: remove journal safely;
- `CommittedLocal`: complete pending finalization, then begin commit cleanup;
- `CleaningUp`: resume its exact step;
- pre-commit interruptible states: preserve current rollback policy.

A terminal journal with artifacts must never be silently ignored.

## Cleanup step semantics

Each step means “this is the next operation,” not “this operation may or may not have completed.”

Rules:

1. persist the next step before executing its destructive operation;
2. make every deletion idempotent;
3. missing files are success where absence is the intended result;
4. symlink or containment violation is an error;
5. journal removal is last;
6. parent sync after journal removal is best effort only where the platform cannot provide a meaningful directory sync, but must not recreate a removed journal;
7. cleanup errors return nonzero and preserve recoverable authority.

## Focused tests

Retain a compact cleanup crash suite. It does not need every failpoint on every platform in CI.

Required local tests:

- crash after cleanup state is persisted, before first deletion;
- crash after staged removal;
- crash after backup removal;
- crash after artifact-root removal;
- crash during rollback cleanup;
- a second crash during cleanup recovery;
- legacy `Committed` journal with artifacts;
- legacy `RolledBack` journal with artifacts;
- idempotent second recovery.

Run the full suite locally and in the Linux correctness job. macOS/Windows receive smoke coverage only.

## Acceptance criteria

- new code never persists terminal state before cleanup;
- startup discovers all journals that still own artifacts;
- cleanup state uses one documented coordinate model;
- normal commit, rollback, recovery, and repair share the same cleanup code;
- a crash at every significant cleanup boundary recovers on Linux;
- no successful terminal path leaves staged or backup artifacts.

---

# Workstream C — Finish transaction-specific, state-aware repair

## Goal

Make repair act on one exact transaction with the correct recovery action.

## Required typed actions

Use transaction-specific actions:

```rust
pub enum RepairAction {
    RollbackTransaction { transaction_id: String },
    FinalizeCommittedLocal { transaction_id: String },
    ResumeCleanup { transaction_id: String },
    CleanupLegacyCommitted { transaction_id: String },
    CleanupLegacyRolledBack { transaction_id: String },
    RemoveOrphanedArtifact { path: PathBuf },
    PruneOrphanedUsage { snippet_id: String },
    RepairLibraryIndex,
    RepairSnippetIds,
    RepairTimestamps,
}
```

Equivalent naming is acceptable. Every transaction action must carry one transaction ID.

## Required state mapping

- `Prepared`, `BackupsDurable`, `Committing`, `RollingBack` → rollback exact transaction;
- `CommittedLocal` → finish pending finalization, then cleanup exact transaction;
- `CleaningUp` → resume cleanup exact transaction;
- legacy `Committed` → commit cleanup exact transaction;
- legacy `RolledBack` → rollback cleanup exact transaction;
- `Failed` → report unsafe/manual unless a narrowly defined recovery exists.

Never run rollback over every interrupted journal because one repair item was selected.

## Required application behavior

`apply_repair` must:

1. load the journal by exact ID;
2. verify the loaded state still matches the action class;
3. acquire the appropriate lock hierarchy;
4. invoke the canonical recovery API;
5. return a typed per-item result;
6. continue to the next independent repair item when safe;
7. aggregate failures accurately.

## CLI exit mapping

The top-level CLI must map outcomes explicitly:

```text
Clean                  -> exit 0
DryRun                 -> exit 0
Repaired               -> exit 0
UnsafeOnly             -> exit 2
PartialFailure         -> exit 1
Fatal scan/apply error -> exit 1
```

Exact nonzero values may differ, but `PartialFailure` and unsafe-only apply attempts must not exit zero.

Do not call `std::process::exit` deep in domain code. Return a value or error to `main` and set the process exit there.

## Tests

Required:

- two interrupted journals produce two distinct repair items;
- applying one item does not mutate the other;
- cleanup-pending committed data is not rolled back;
- `CommittedLocal` recovery records/reuses pending and then cleans up;
- partial failure exits nonzero;
- all-success apply exits zero;
- dry run performs no writes;
- orphan path containment and symlink rejection remain enforced.

## Acceptance criteria

- no repair action loops over all transactions;
- action and target ID are inseparable;
- committed or cleanup-pending data is never rolled back by generic repair;
- partial failure reaches a nonzero process exit;
- repair output identifies the exact transaction without exposing snippet content or secrets.

---

# Workstream D — Finish destination privacy without expanding the permission subsystem

## Goal

Retain the Phase 11G permission correction and close only demonstrable gaps.

## Required policy

On Unix:

- new libraries: `0600`;
- new `libraries.toml`: `0600`;
- new `usage.toml`: `0600`;
- new `sync.toml`: `0600`;
- existing ordinary files: preserve sanitized original mode;
- existing `sync.toml`: preserve only if already no broader than the sensitive policy; otherwise normalize to `0600`;
- transaction directories: `0700`;
- transaction artifacts and journals: `0600`.

Strip setuid, setgid, and sticky bits from restored ordinary files.

On Windows:

- keep files beneath the user configuration directory;
- preserve readonly semantics where already supported;
- do not claim ACL hardening unless actually implemented.

## Simplification rule

Do not add a generalized cross-platform ACL abstraction. The product needs a clear Unix mode policy and conservative Windows behavior, not a permissions framework.

## Tests

Keep one Unix-focused integration test file covering:

- new state files are `0600`;
- existing `0640` ordinary file remains `0640`;
- new `sync.toml` cannot become `0644`;
- transaction directories/files use `0700`/`0600`;
- setuid/setgid/sticky bits are stripped;
- rollback restores the supported original mode.

Run this file on Linux CI. macOS may compile and run it locally; it does not need a dedicated matrix gate.

## Acceptance criteria

- no new destination uses an implicit `0644` fallback;
- `sync.toml` is never downgraded to a broader mode;
- permission helpers fail closed when the required Unix mode cannot be applied;
- documentation states the exact supported contract without ACL overclaims.

---

# Workstream E — Correct manifest tests, not the whole restore architecture

## Goal

Ensure semantic tests fail for the semantic reason named by the test.

## Shared fixture builder

Add a small test helper that:

1. writes actual artifact content;
2. calculates exact byte length;
3. calculates exact SHA-256;
4. emits a valid manifest;
5. allows one targeted mutation;
6. rewrites dependent size/hash fields when content changes unless size/hash is the targeted fault.

Example API:

```rust
struct BackupFixture {
    root: TempDir,
    manifest: BackupManifest,
}

impl BackupFixture {
    fn valid_replace() -> Self;
    fn rewrite_index(&mut self, content: &str);
    fn add_library(&mut self, name: &str, content: &str);
    fn set_schema(&mut self, schema: u32);
    fn set_layout(&mut self, layout: &str);
    fn write_manifest(&self);
    fn assert_no_restore_side_effects(&self, config_dir: &Path);
}
```

## Required semantic cases

Keep a focused set:

- unsupported schema;
- unsupported layout;
- exact duplicate destination;
- portable case-fold collision;
- duplicate library name in index;
- multiple primary libraries;
- index references missing artifact;
- replace manifest contains unreferenced library;
- duplicate snippet IDs;
- size mismatch;
- checksum mismatch;
- symlink source;
- oversized source.

Do not maintain a sprawling combinatorial path suite unless a real bug requires it.

## Required assertions

Every rejection test must assert:

- nonzero result;
- stable intended error category;
- no transaction journal;
- no transaction artifact root;
- no pending marker;
- no live destination change.

## Acceptance criteria

- no semantic fixture uses stale size or checksum accidentally;
- each test contains one targeted defect;
- tests do not accept multiple unrelated error messages;
- all focused manifest tests run in the normal Linux workspace test command.

---

# Workstream F — Keep one meaningful sync E2E and trim telemetry ambitions

## Goal

Prove the user-visible sync invariant without maintaining a generalized evidence system.

## Required invariant

One local mutation must result in:

1. pending generation creation;
2. one executor cycle;
3. one successful remote sync operation;
4. server state change;
5. pending generation clear only after remote success;
6. no duplicate operation after a bounded quiet period.

## Minimal observer contract

The test-only observer may record:

- sequence number;
- operation name;
- start timestamp;
- finish timestamp;
- success boolean;
- current/max in-flight count;
- sanitized device/library identifiers where already available.

Do not require payload hashes, request-body inspection, revision graphs, or a generalized telemetry schema unless the production protocol already exposes those values cheaply and the test uses them directly.

The observer must remain behind `snip-sync`’s test-helper feature and must not affect production builds.

## Required test

The single headline test must:

- retain the observer handle;
- perform a real CLI mutation;
- assert exactly one relevant sync operation starts and finishes successfully;
- assert maximum relevant concurrency is one;
- assert server state changes from zero to one expected snippet;
- assert pending is absent after the operation finishes;
- wait one debounce window plus a small margin and assert no second relevant operation.

A database row count alone is insufficient for exact request count, but the observer does not need to become a production observability subsystem.

## Failure tests

Keep focused tests for:

- unreachable server preserves pending;
- authentication failure preserves pending;
- executor no-op false success preserves pending and records failure/non-success status.

These can run on Linux. Platform smoke need only prove the binaries start and isolated local commands work.

## Acceptance criteria

- one real E2E proves remote effect before pending clear;
- observer records real handler activity, not manual test calls;
- production builds do not contain active observer behavior;
- no duplicate sync occurs in the bounded quiet period;
- telemetry requirements are limited to assertions the test actually consumes.

---

# Workstream G — Replace the CI workflow with a small three-instance design

## Goal

Reduce ordinary GitHub Actions cost and failure surface while retaining useful regression detection.

## Target workflow shape

`.github/workflows/ci.yml` should be approximately 100–180 lines, with two job definitions.

Recommended workflow:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

permissions:
  contents: read

concurrency:
  group: ci-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: always
  PROTOC_VERSION: "25.1"

jobs:
  linux-correctness:
    name: Linux correctness
    runs-on: ubuntu-latest
    timeout-minutes: 35
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Install protoc
        run: bash scripts/ci/install-protoc.sh
      - uses: Swatinem/rust-cache@v2
      - name: Format
        run: cargo fmt --all -- --check
      - name: Clippy
        run: cargo clippy --workspace --all-targets --all-features -- -D warnings
      - name: Build
        run: cargo build --workspace --all-features
      - name: Test
        run: cargo test --workspace --all-features -- --test-threads=1

  platform-smoke:
    name: Platform smoke (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    timeout-minutes: 25
    strategy:
      fail-fast: false
      matrix:
        os: [macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install protoc (Unix)
        if: runner.os != 'Windows'
        run: bash scripts/ci/install-protoc.sh
      - name: Install protoc (Windows)
        if: runner.os == 'Windows'
        shell: pwsh
        run: pwsh -File scripts/ci/install-protoc.ps1 -Version ${{ env.PROTOC_VERSION }}
      - uses: Swatinem/rust-cache@v2
      - name: Check workspace
        run: cargo check --workspace --all-targets --all-features
      - name: Library tests
        run: cargo test --workspace --all-features --lib -- --test-threads=1
      - name: CLI/platform smoke
        run: cargo test --test platform_smoke --features test-support -- --test-threads=1
```

Exact action versions may follow repository policy. Do not add more jobs unless a demonstrated defect cannot be caught by either job.

## Remove these jobs

Delete:

- `fmt` as a separate runner;
- `clippy` as a separate runner;
- `production-seam` matrix;
- dev/release `test` matrix;
- `release-blocking-tests` matrix;
- `transaction-tests` matrix;
- `verify-evidence`;
- three-platform `package` matrix.

Formatting and clippy belong in the Linux job. Focused tests belong in the normal Linux `cargo test` command.

## Platform smoke suite

Create `tests/platform_smoke.rs` with fast, deterministic cases that use the real binary:

1. `snp --version` succeeds;
2. `snp --help` succeeds;
3. isolated `library create smoke` succeeds;
4. isolated snippet creation/listing succeeds;
5. a simple backup and dry-run restore succeeds;
6. `snip-sync --help` or `--version` succeeds where the binary is part of the workspace test build;
7. no command depends on a system keychain or external network.

Keep total runtime small. Do not put crash failpoint matrices or full server E2E in this smoke file.

## Long-running tests

Prefer normal Rust tests. If a genuinely expensive or timing-sensitive suite materially slows ordinary Linux CI:

- mark only that test or test file ignored with a clear reason;
- run it explicitly in `scripts/release-check.sh`;
- do not create another GitHub Actions job to compensate;
- do not dynamically parse test names or use `eval` to build skip lists.

## CI acceptance criteria

- exactly two job definitions;
- exactly three runner instances per push/PR;
- no debug/release matrix;
- no package matrix;
- no publishing permissions or secrets;
- no evidence verification job;
- no dynamic skip-list generation;
- no `eval`;
- normal Linux job runs all non-ignored tests once;
- macOS and Windows run compile, library, and smoke checks only;
- branch protection is updated to require only the simplified checks.

---

# Workstream H — Add straightforward local verification scripts

## Goal

Make local verification the primary deep-check mechanism without adding orchestration complexity.

## `scripts/check.sh`

Purpose: ordinary development verification before pushing.

Required commands:

```bash
#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features
cargo test --workspace --all-features -- --test-threads=1
```

Allow an optional environment variable such as `SNP_CHECK_JOBS` only if needed for resource control. Do not add a large option parser.

## `scripts/release-check.sh`

Purpose: maintainer-run pre-release verification.

Required sequence:

```bash
#!/usr/bin/env bash
set -euo pipefail

bash scripts/check.sh

cargo build --workspace --release --all-features

cargo test --release --test cleanup_crash_failpoints \
  --features test-support -- --test-threads=1
cargo test --release --test restore_crash_failpoints \
  --features test-support -- --test-threads=1
cargo test --release --test manifest_contracts \
  --features test-support -- --test-threads=1
cargo test --release --test deterministic_e2e \
  --features test-support -- --test-threads=1
cargo test --release --test executor_noop_success \
  --features test-support -- --test-threads=1

bash scripts/ci/test-production-seams.sh

cargo package -p snip-proto --locked
cargo package -p snip-sync --locked
cargo package -p snip-it --locked

cargo publish -p snip-proto --dry-run
cargo publish -p snip-sync --dry-run
cargo publish -p snip-it --dry-run
```

Adjust package/dry-run commands so unchanged already-published versions do not make the script unusable. One acceptable approach:

- accept explicit crate arguments;
- or provide `SNP_RELEASE_CRATES="snip-it"`;
- default to package all but dry-run only selected changed crates;
- never publish automatically.

Keep this script readable. Do not implement release dependency resolution in shell beyond the known three-crate order.

## Production seam script

Retain the script as a local release check, not a per-push CI matrix.

Correct it so each assertion reaches the intended guarded path:

- non-dry-run restore reaches the matching restore failpoint location;
- worker-spawn test enables auto-sync and proves the production binary does not honor suppression;
- executor test establishes a valid pending generation and execution context;
- event test invokes a path that would emit an event in a feature-enabled build;
- barrier test uses the exact production barrier name and reaches it.

The no-feature binary must ignore matching test variables.

## Optional PowerShell smoke script

A small `scripts/platform-smoke.ps1` may run:

```powershell
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --lib -- --test-threads=1
cargo test --test platform_smoke --features test-support -- --test-threads=1
```

Do not duplicate the full release script in PowerShell unless Windows becomes the publishing host.

## Acceptance criteria

- one ordinary local check command exists;
- one deeper release-check command exists;
- scripts contain plain Cargo commands and minimal shell logic;
- exhaustive crash and production-seam tests remain available locally;
- no script performs `cargo publish` without the maintainer issuing an explicit publish command.

---

# Workstream I — Codify manual crates.io publishing

## Goal

Make publishing simple, explicit, and independent of GitHub Actions.

## Required documentation

Add `RELEASING.md` with:

1. prerequisites;
2. version bump rules;
3. dependency order;
4. local checks;
5. dry-run commands;
6. manual publish commands;
7. optional tag creation;
8. post-publish verification;
9. immutable-version warning.

## Crates and dependency order

Known crates:

1. `snip-proto`;
2. `snip-sync`, which depends on `snip-proto`;
3. `snip-it`.

Publish only crates whose version changed.

If `snip-proto` changes:

- bump `snip-proto`;
- update `snip-sync` dependency/version as needed;
- publish `snip-proto` first;
- wait until the version resolves from crates.io;
- publish `snip-sync` if changed;
- publish `snip-it` if changed.

If only `snip-it` changes, publish only `snip-it`.

## Required manual commands

Example:

```bash
bash scripts/release-check.sh

cargo publish -p snip-proto --dry-run
cargo publish -p snip-proto

cargo publish -p snip-sync --dry-run
cargo publish -p snip-sync

cargo publish -p snip-it --dry-run
cargo publish -p snip-it
```

The documentation must say to omit unchanged crates.

## Version immutability

State explicitly:

- crates.io versions are immutable;
- a failed or incomplete published release cannot be overwritten;
- any correction requires a new version bump;
- verify package contents before publishing;
- do not attempt to “retry” a published version after changing files.

## Git tags

Tags are optional and manual:

```bash
git tag -a v1.3.4 -m "snip-it 1.3.4"
git push origin v1.3.4
```

Do not require a GitHub Release. Do not automatically publish from tags.

## Security

- crates.io token remains in the maintainer’s local Cargo credentials;
- no crates.io token is stored in GitHub Actions;
- no workflow has `id-token: write` or package publishing permissions;
- release documentation does not print or inspect credentials.

## Acceptance criteria

- no publish workflow exists;
- no release workflow exists;
- no crates.io secret is referenced by GitHub Actions;
- `RELEASING.md` contains exact dependency-order commands;
- publish steps are manual and explicit;
- package/dry-run validation occurs before publishing.

---

# Workstream J — Simplify tests and documentation around the new verification model

## Goal

Remove process artifacts left by the previous evidence-heavy approach.

## Required cleanup

Remove or revise documentation that requires:

- exact workflow run URLs;
- exact job URLs;
- a same-commit evidence registry;
- all-platform release-profile testing;
- package installation on every platform;
- a production-seam CI gate;
- dedicated “release-blocking” and “transaction” job taxonomies.

Keep plan history, but mark older requirements superseded by Phase 11H.

## Test naming

Tests should describe product contracts rather than release bureaucracy.

Prefer:

- `cleanup_crash_failpoints`;
- `manifest_contracts`;
- `sync_e2e`;
- `platform_smoke`.

Avoid adding more suites named “closure,” “evidence,” or “release-blocking” unless they encode a distinct product behavior.

Existing files do not need to be renamed solely for aesthetics. Do not create churn without value.

## CI comments

Keep workflow comments short. The workflow should be understandable without reference to a planning document.

## Acceptance criteria

- current docs explain the two-tier model: lightweight CI plus deep local release checks;
- old evidence requirements are explicitly superseded;
- no status file claims GitHub Actions publishes releases;
- contributors can determine the standard check command in under one minute.

---

## 4. Recommended implementation sequence

Use small commits. Recommended order:

1. `docs: reopen Phase 11 under simplified verification model`
   - update closure status and identify Phase 11H.

2. `transaction: enter cleanup state before terminal outcome`
   - add `CleanupOutcome` and `CleanupStep`;
   - stop persisting new `Committed`/`RolledBack` states.

3. `transaction: recover legacy terminal journals with artifacts`
   - compatibility classification.

4. `transaction: unify commit rollback recovery cleanup APIs`
   - `begin_cleanup` and `resume_cleanup`.

5. `tests: correct cleanup crash suite for new state model`
   - focused commit, rollback, legacy, and second-crash tests.

6. `repair: make transaction recovery actions target exact journals`
   - typed transaction IDs and state mapping.

7. `repair: return nonzero on partial or unsafe apply outcomes`
   - top-level CLI mapping.

8. `restore: finalize private destination policy`
   - all entry kinds, no `0644` fallback.

9. `tests: consolidate destination permission coverage`
   - focused Unix policy tests.

10. `tests: add valid single-fault backup fixture builder`
    - shared sizes and hashes.

11. `tests: rewrite semantic manifest cases with no-side-effect assertions`
    - remove stale/dummy metadata.

12. `tests: reduce sync telemetry to one real functional E2E contract`
    - real observer, one request, pending ordering.

13. `tests: add fast cross-platform CLI smoke suite`
    - `tests/platform_smoke.rs`.

14. `ci: replace large matrix with Linux correctness and platform smoke`
    - two job definitions, three instances.

15. `scripts: add ordinary local check command`
    - `scripts/check.sh`.

16. `scripts: add local pre-release verification`
    - `scripts/release-check.sh`.

17. `scripts: correct production seam check for local release use`
    - real guarded paths.

18. `docs: add manual crates.io release guide`
    - `RELEASING.md`.

19. `docs: remove obsolete evidence and automated-release expectations`
    - status, contributing, agent guidance.

20. `docs: close Phase 11 after code and simplified CI pass`
    - record final commit and concise test summary;
    - do not add workflow URL tables.

---

## 5. Verification model after Phase 11H

### 5.1 Every push or pull request

GitHub Actions:

```text
Linux correctness
  fmt
  clippy
  debug build
  full non-ignored workspace tests once

macOS smoke
  cargo check
  library tests
  platform smoke

Windows smoke
  cargo check
  library tests
  platform smoke
```

### 5.2 Before a normal push

Developer runs:

```bash
bash scripts/check.sh
```

### 5.3 Before publishing

Maintainer runs:

```bash
bash scripts/release-check.sh
```

Then explicit `cargo publish` commands for changed crates.

### 5.4 When touching platform-specific code

Run the relevant platform locally or rely on the platform smoke CI for compile/basic behavior. Do not expand ordinary CI to full exhaustive testing on every operating system.

### 5.5 When touching transaction recovery

Run locally:

```bash
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
```

These also run under the normal Linux full test command unless explicitly ignored for runtime reasons.

### 5.6 When touching sync orchestration

Run locally:

```bash
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
```

Do not create another permanent CI matrix for this category.

---

## 6. Release-blocking acceptance checklist

### Production correctness

- [ ] New transactions enter cleanup state before any terminal state.
- [ ] Commit cleanup is restartable.
- [ ] Rollback cleanup is restartable.
- [ ] Legacy terminal journals with artifacts are recovered.
- [ ] Journal removal is last.
- [ ] Successful finalization leaves no staged or backup artifacts.
- [ ] Cleanup recovery is idempotent.

### Repair

- [ ] Every transaction repair action contains one transaction ID.
- [ ] Cleanup-pending transactions resume cleanup.
- [ ] `CommittedLocal` transactions finalize pending and clean up.
- [ ] Pre-commit interrupted transactions roll back.
- [ ] Applying one repair item does not affect another transaction.
- [ ] Partial failure exits nonzero.
- [ ] Dry run performs no writes.

### File privacy

- [ ] New libraries are `0600` on Unix.
- [ ] New index and usage files are `0600` on Unix.
- [ ] New and restored `sync.toml` remains private.
- [ ] Transaction directories are `0700`.
- [ ] Transaction files are `0600`.
- [ ] Existing supported ordinary modes are preserved safely.
- [ ] No implicit `0644` fallback remains for new restored state.

### Manifest proof

- [ ] Shared fixture builder computes actual sizes and hashes.
- [ ] Each negative semantic test has one defect.
- [ ] Tests assert the intended error category.
- [ ] Rejected restores create no journal or artifact root.
- [ ] Rejected restores create no pending state.
- [ ] Rejected restores do not mutate live destinations.

### Sync

- [ ] One real mutation produces one relevant remote operation.
- [ ] Server state changes as expected.
- [ ] Pending clears only after remote success.
- [ ] Maximum relevant in-flight count is one.
- [ ] No duplicate operation occurs after the quiet period.
- [ ] False-success, auth failure, and network failure preserve pending.

### CI simplification

- [ ] `.github/workflows/ci.yml` has two job definitions.
- [ ] CI creates three runner instances total.
- [ ] Linux runs fmt, clippy, build, and tests once.
- [ ] macOS and Windows run check, library tests, and smoke only.
- [ ] No dev/release matrix exists.
- [ ] No release-blocking matrix exists.
- [ ] No transaction matrix exists.
- [ ] No production-seam matrix exists.
- [ ] No package matrix exists.
- [ ] No evidence-verification job exists.
- [ ] No publish or release workflow exists.

### Local verification

- [ ] `scripts/check.sh` is documented and executable.
- [ ] `scripts/release-check.sh` is documented and executable.
- [ ] Deep crash tests are included in release check.
- [ ] Correct production-seam tests are included in release check.
- [ ] Package and publish dry-runs are included for selected changed crates.
- [ ] Actual publishing is never automatic.

### Manual release

- [ ] `RELEASING.md` exists.
- [ ] Dependency order is documented.
- [ ] Unchanged crates are not republished.
- [ ] crates.io immutability is documented.
- [ ] No crates.io token is referenced by GitHub Actions.
- [ ] Tags and GitHub Releases are optional and manual.

---

## 7. Stop conditions

Keep Phase 11 open if any of the following remains true:

- `commit_transaction` persists `Committed` before cleanup ownership;
- rollback persists `RolledBack` before cleanup ownership;
- a terminal journal with artifacts is ignored;
- repair applies one item to multiple transactions;
- cleanup-pending committed data can be rolled back;
- partial repair failure exits zero;
- new `sync.toml` can become `0644`;
- semantic manifest tests still contain stale size/hash metadata;
- the sync E2E cannot prove remote operation count and pending-clear order;
- production test controls are active without test features;
- CI still contains duplicated full matrices;
- GitHub Actions contains publishing credentials or commands;
- the release guide suggests overwriting an immutable crates.io version;
- status claims completion based only on workflow configuration.

Do not resolve a stop condition by weakening assertions, skipping the test globally, or adding another CI job.

---

## 8. Handoff instructions

1. Read the current transaction implementation and Phase 11H before changing CI.
2. Update closure status first.
3. Correct cleanup ownership before relying on cleanup crash tests.
4. Keep existing Phase 11G tests where they prove a real contract, but rewrite them for the corrected state model.
5. Finish repair before declaring transaction closure.
6. Preserve the private destination changes already landed.
7. Replace only defective manifest fixtures; do not redesign backup format.
8. Keep one strong sync E2E and remove pressure to build a telemetry framework.
9. Add the small platform smoke suite before deleting broad platform matrices.
10. Replace CI in one reviewable commit.
11. Add local scripts and release documentation before removing package/production-seam CI jobs.
12. Never add crates.io credentials to GitHub.
13. Run `scripts/check.sh` locally.
14. Run focused transaction, repair, manifest, permissions, and sync tests.
15. Push and verify the simplified three-instance CI passes.
16. Leave actual crates.io publishing to the maintainer.
17. Mark Phase 11 complete only after the remaining production defects are closed and the simplified CI passes.

The desired result is a smaller repository process, not a smaller correctness standard. Product invariants belong in code and focused tests. GitHub Actions should provide quick regression feedback, while exhaustive pre-release validation and crates.io publishing remain explicit local maintainer actions.

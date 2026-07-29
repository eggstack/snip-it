# Phase 11I — Legacy Recovery, Exact Repair, Focused CI, and Release-Check Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `98acbbce29c357ae4440600dccb45a9402393e91`

Parent plan:

- `plans/snip-it-correctness-11h-ci-simplification-local-verification-and-manual-release.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan supersedes Phase 11H only for remaining-work and closure decisions. The Phase 11H architectural and process decisions remain in force:

- retain the simplified three-runner GitHub Actions topology;
- retain manual crates.io publishing;
- retain one `snp` client binary and one `snip-sync` server binary;
- retain one-shot worker and executor subprocesses;
- do not introduce a client daemon, workflow engine, release bot, evidence registry, `xtask`, container test layer, or automated publishing.

Phase 11H materially improved the repository. The large CI matrix and automated release workflow were removed, local verification scripts and `RELEASING.md` were added, new transactions now enter typed cleanup before artifact deletion, repair actions gained transaction IDs, private destination coverage improved, and real sync tests gained observer support.

Direct review of the implementation at `98acbbce29c357ae4440600dccb45a9402393e91` found a small set of remaining defects. These are correctness and verification-boundary defects, not grounds for re-expanding CI.

The Phase 11I objective is to close those defects with the smallest coherent implementation:

1. make legacy terminal journals discoverable and recoverable;
2. make every transaction repair operate on exactly one selected journal;
3. migrate remaining semantic restore tests to valid single-fault fixtures;
4. make the one retained sync E2E exact rather than permissive;
5. finish the intended split between fast CI and deeper local release verification;
6. enforce per-crate `cargo publish --dry-run` without automating actual publishing;
7. reconcile closure status against the actual final commit.

---

## 1. Executive closure decision

Phase 11I is a corrective closure pass. It must delete or consolidate redundant proof where possible.

The required endpoint is:

- transaction cleanup authority is durable and discoverable for both current and legacy journals;
- `snp repair` applies one exact state-appropriate transaction action;
- semantic restore rejection tests prove the semantic rule named by each test;
- one real sync E2E proves one identified remote operation and pending-clear ordering;
- normal CI remains small and fast;
- deeper crash and protocol verification runs locally before release;
- crates are published manually from a maintainer machine;
- the status document contains one internally consistent closure verdict.

Do not interpret “closure” as a request for more matrices, more telemetry infrastructure, or more process documentation.

---

## 2. Confirmed baseline defects

### 2.1 Legacy terminal recovery branches are unreachable

The mutation gate contains branches for legacy `TransactionState::Committed` and `TransactionState::RolledBack` journals. However, the gate obtains journals through `check_interrupted_transactions`, and that scanner only returns states for which `is_interruptible()` is true.

`Committed` and `RolledBack` are not interruptible. The scanner discards them before the gate can classify them.

Consequences:

- a legacy `Committed` journal with an artifact directory is ignored;
- a legacy `RolledBack` journal with an artifact directory is ignored;
- the compatibility branches in the mutation gate are dead code;
- repair never emits `CleanupLegacyCommitted` or `CleanupLegacyRolledBack` because it uses the same filtered scanner;
- sensitive staged or backup content can remain indefinitely.

### 2.2 `FinalizeCommittedLocal` repair is not exact

`RepairAction::FinalizeCommittedLocal` contains one transaction ID, but application delegates to `gate_mutation_on_interrupted_transactions`, which scans the whole transaction directory.

Consequences:

- one selected repair can be blocked by an unrelated second journal;
- the action is not guaranteed to apply to the selected transaction;
- state drift between collection and application is not validated;
- the transaction-specific action type overstates the execution semantics.

### 2.3 Remaining semantic manifest tests are multi-fault

The shared fixture builder computes exact sizes and hashes, but several older semantic tests still mutate index or library content without regenerating manifest metadata.

Known examples include:

- duplicate library names;
- an index reference to a missing library;
- multiple primary libraries with hard-coded unrelated size/hash values;
- an unreferenced library with hard-coded unrelated size/hash values.

These tests can fail at checksum or size validation before reaching the semantic validator named by the test.

### 2.4 Observer E2E assertions remain permissive

The observer headline test currently accepts:

- at least one sync or push start;
- at least one successful finish of any observed request;
- missing device identity with a diagnostic note rather than failure.

It does not require one matched start/finish pair for the exact sync operation.

### 2.5 Linux CI still runs the exhaustive integration suite

The workflow topology was reduced to three runner instances, which is correct. However, `linux-correctness` still runs:

```bash
cargo test --workspace --all-features -- --test-threads=1
```

That includes the deep cleanup crash suite, restore crash suite, deterministic sync E2E, and other expensive integration targets. These were intended to move to local release verification.

The runner count is small, but the verification boundary is not yet correctly separated.

### 2.6 Release-check does not execute publish dry-runs

`scripts/release-check.sh` runs `cargo package` and prints publish commands, but does not execute `cargo publish --dry-run`.

The actual publish must remain manual. The dry-run should be locally executable and mandatory for the exact crate about to be published.

### 2.7 Closure status is stale and contradictory

The status document currently:

- correctly says `INCOMPLETE / REOPENED`;
- still lists Phase 11H blockers;
- claims Workstreams A–J are complete;
- records `fa314fd` as the final implementation commit even though later corrective commits exist.

Phase 11I must leave one accurate record.

---

# Workstream A — Replace filtered interruption scanning with complete journal discovery

## Goal

Discover every transaction journal that can block mutation or require repair, including legacy terminal journals that still own artifacts.

## Required design

Separate file discovery from recovery classification.

Do not use one function whose name and behavior imply that only non-terminal journals exist.

Recommended types:

```rust
#[derive(Debug)]
pub struct JournalInventory {
    pub journals: Vec<TransactionJournal>,
    pub corrupt: Vec<CorruptJournal>,
}

#[derive(Debug)]
pub struct CorruptJournal {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryClass {
    Rollback,
    FinalizeCommittedLocal,
    ResumeCleanup,
    CleanupLegacyCommitted,
    CleanupLegacyRolledBack,
    RemoveTerminalJournal,
    UnsafeFailed,
    NoAction,
}
```

Equivalent naming is acceptable.

## Required scanner

Add one complete scanner:

```rust
pub fn scan_transaction_journals(
    transaction_dir: &Path,
) -> SnipResult<JournalInventory>;
```

The scanner must:

1. enumerate every `txn-*.toml` file;
2. parse every valid journal regardless of state;
3. report corrupt journal files rather than silently skipping them;
4. avoid following symlinks;
5. use stable ordering, preferably journal ID or path order, so diagnostics and tests are deterministic;
6. perform no mutation.

Do not hide legacy terminal states in the scanning layer.

## Required classification

Add one pure classification function:

```rust
pub fn classify_journal_recovery(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> RecoveryClass;
```

Required mapping:

- `Prepared` → `Rollback`;
- `BackupsDurable` → `Rollback`;
- `Committing { .. }` → `Rollback`;
- `RollingBack { .. }` → `Rollback`;
- `CommittedLocal { .. }` → `FinalizeCommittedLocal`;
- `CleaningUp { .. }` → `ResumeCleanup`;
- legacy `Committed` with artifacts → `CleanupLegacyCommitted`;
- legacy `RolledBack` with artifacts → `CleanupLegacyRolledBack`;
- legacy `Committed` without artifacts → `RemoveTerminalJournal`;
- legacy `RolledBack` without artifacts → `RemoveTerminalJournal`;
- `Failed(_)` → `UnsafeFailed`;
- any future safe terminal state with no owned artifact → `NoAction` or `RemoveTerminalJournal`, explicitly documented.

## Artifact ownership detection

Replace the current weak helper that only checks whether `artifacts/<id>/` exists.

Required helper:

```rust
pub fn journal_owns_artifacts(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<bool>;
```

It must consider:

- the per-transaction artifact root;
- every `backup_path` in the journal;
- every `durable_staged_path` in the journal;
- any other transaction-owned artifact explicitly represented by the schema.

Before treating a referenced path as owned, validate containment and reject symlinks.

A missing artifact is not an error where absence is a valid idempotent cleanup result.

## Corrupt journal policy

Mutation must fail closed when a corrupt `txn-*.toml` exists.

Required behavior:

- read-only commands remain read-only and do not mutate recovery state;
- a mutating command reports the corrupt journal path and directs the user to `snp repair`;
- `snp repair` lists the corrupt journal as unsafe/manual unless a separately defined quarantine action is implemented;
- do not silently skip a corrupt journal and proceed with mutation.

Do not add automatic deletion of corrupt journals in this phase.

## Compatibility wrapper

If `check_interrupted_transactions` is retained for API compatibility, implement it as a narrow wrapper over the complete scanner and classifier. It must not remain the authoritative scanner for mutation gate or repair.

## Required tests

Add focused tests for:

1. `Committed` with an artifact directory is discovered and classified as `CleanupLegacyCommitted`;
2. `RolledBack` with an artifact directory is discovered and classified as `CleanupLegacyRolledBack`;
3. `Committed` without artifacts is classified as terminal-journal cleanup;
4. `RolledBack` without artifacts is classified as terminal-journal cleanup;
5. `CleaningUp` is classified as resume cleanup;
6. `CommittedLocal` is classified as finalize committed-local;
7. corrupt journal blocks mutation and appears in repair output;
8. symlinked journal path or artifact path is rejected;
9. multiple journals are returned in stable order;
10. scanning itself does not remove or rewrite anything.

## Acceptance criteria

- legacy terminal compatibility branches are reachable through production discovery;
- no journal that still owns artifacts is filtered out because its state is “terminal”;
- corrupt journals are visible and fail closed for mutations;
- scanner and classifier are independently testable;
- no additional CI matrix is added.

---

# Workstream B — Centralize exact transaction recovery

## Goal

Provide one canonical API that recovers exactly one transaction by ID and expected action.

## Required API

Add a transaction-layer API similar to:

```rust
pub fn recover_transaction_by_id(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    transaction_id: &str,
    expected: RecoveryClass,
) -> SnipResult<()>;
```

The implementation must:

1. validate the transaction ID as an expected journal identifier, not an arbitrary path;
2. derive `txn-<id>.toml` internally;
3. reject symlinked journal files;
4. load exactly that journal;
5. classify its current state again at execution time;
6. compare actual classification with `expected`;
7. return a stale-action error if state changed incompatibly;
8. acquire the established lock hierarchy;
9. invoke the canonical state-specific recovery function;
10. return only after that exact journal is recovered or a precise error is produced.

## Locking

Preserve the existing lock order:

```text
LocalDataLock
  -> TransactionLock
  -> pending or destination mutation
```

Do not acquire the transaction lock separately for each cleanup step.

Do not invoke the global mutation gate while already applying one repair action.

## State-specific execution

Required execution mapping:

### Rollback

For `Rollback`:

- invoke `rollback_transaction` on the selected journal only;
- preserve rollback progress coordinates;
- transition to canonical rollback cleanup;
- do not inspect or process unrelated journals.

### Finalize committed-local

Extract or expose a direct API:

```rust
pub fn finalize_committed_local_transaction(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<()>;
```

It must:

1. finish or reuse the transaction-associated pending generation;
2. persist the resulting `PendingFinalization` state;
3. enter `CleaningUp { outcome: Commit, .. }` durably;
4. run canonical cleanup;
5. affect no other journal.

Both startup recovery and repair must call this API.

### Resume cleanup

For `ResumeCleanup`:

- require the current state to be `CleaningUp`;
- call `resume_cleanup` on the selected journal;
- preserve the recorded cleanup outcome and step.

### Legacy terminal cleanup

For legacy commit or rollback cleanup:

- require the matching legacy state;
- enter `CleaningUp` with the matching outcome;
- run canonical cleanup;
- remove a terminal journal without artifacts safely and journal-last where applicable.

### Unsafe failed state

Do not auto-recover `Failed(_)` without a separately proven recovery protocol.

Return a diagnostic that retains the journal and artifacts.

## Mutation gate refactor

The mutation gate should:

1. call the complete scanner;
2. fail closed on corrupt journals;
3. classify all actionable journals;
4. auto-recover only when exactly one unambiguous safe action exists;
5. invoke `recover_transaction_by_id` for that exact journal;
6. refuse and direct to repair when multiple actionable journals exist;
7. avoid duplicate state-specific recovery logic.

The mutation gate must become an orchestration layer, not a second implementation of recovery.

## Required tests

Add integration or unit tests for:

1. exact rollback with two journals present changes only the selected transaction;
2. exact cleanup resume with two journals present changes only the selected transaction;
3. exact committed-local finalization succeeds while an unrelated second journal remains untouched;
4. stale repair action is rejected if the journal state changed after report generation;
5. unknown transaction ID is rejected without touching other journals;
6. malformed transaction ID cannot escape the transaction directory;
7. lock conflict returns a deterministic error;
8. a second invocation after successful cleanup is idempotent where “already complete” is safe;
9. legacy `Committed` exact recovery removes its artifacts and journal;
10. legacy `RolledBack` exact recovery removes its artifacts and journal.

## Acceptance criteria

- no transaction-specific repair calls the global mutation gate;
- one repair item can never process every journal;
- expected state is revalidated immediately before mutation;
- startup and repair share canonical recovery functions;
- unrelated journals are byte-for-byte unchanged by exact recovery.

---

# Workstream C — Finish repair collection and process semantics

## Goal

Make `snp repair` accurately describe and apply every recoverable transaction class.

## Candidate collection

Replace transaction candidate collection based on `check_interrupted_transactions` with the complete inventory and classifier from Workstream A.

Required action mapping:

- `Rollback` → `RepairAction::RollbackTransaction { transaction_id }`;
- `FinalizeCommittedLocal` → `RepairAction::FinalizeCommittedLocal { transaction_id }`;
- `ResumeCleanup` → `RepairAction::ResumeCleanup { transaction_id }`;
- `CleanupLegacyCommitted` → `RepairAction::CleanupLegacyCommitted { transaction_id }`;
- `CleanupLegacyRolledBack` → `RepairAction::CleanupLegacyRolledBack { transaction_id }`;
- `RemoveTerminalJournal` → a typed exact action carrying `transaction_id`;
- `UnsafeFailed` → unsafe/manual item;
- corrupt journal → unsafe/manual item carrying exact path.

Do not keep a transaction action variant that cannot be produced by collection.

## Exact application

All transaction action variants must delegate to `recover_transaction_by_id` with the expected class.

Remove duplicated journal loading and state handling from `repair_cmd.rs` where the transaction module can own it.

## Backup policy

Do not create a full configuration backup merely to remove already-committed transaction artifacts unless current repository policy requires it for every repair invocation.

Preferred behavior:

- create a config backup before repairs that mutate live libraries, index, usage, or sync configuration;
- transaction cleanup that only removes transaction-owned artifacts does not need to duplicate the entire config tree;
- rollback retains and uses its transaction backups as authoritative evidence.

If preserving one backup-before-any-repair rule is simpler, retain it, but do not weaken transaction isolation.

## Exit semantics

Retain:

- `Clean`, `DryRun`, `Repaired` → exit 0;
- `PartialFailure` → exit 1;
- `UnsafeOnly` → exit 2.

Add subprocess-level tests that execute the real `snp repair` binary and assert those process codes.

Do not test only the returned enum.

## Required repair tests

1. dry-run reports each exact transaction ID and changes nothing;
2. applying one selected safe transaction repair leaves unrelated journals unchanged;
3. legacy committed cleanup action is generated and succeeds;
4. legacy rolled-back cleanup action is generated and succeeds;
5. committed-local finalization action is generated and succeeds with another journal present;
6. cleanup resume action starts at the recorded step;
7. stale action state mismatch increments failure count and exits 1;
8. one success plus one failure exits 1;
9. unsafe-only corrupt journal exits 2 when `--apply` has no safe item;
10. JSON output contains typed action/category, transaction ID, applied count, failed count, and exit classification without exposing snippet plaintext.

## Acceptance criteria

- all typed transaction repair variants are reachable;
- every action carries and uses an exact transaction ID;
- partial failure is proven at process level;
- repair does not silently reinterpret a changed journal state;
- transaction cleanup does not mutate unrelated local data.

---

# Workstream D — Convert semantic restore tests to computed single-fault fixtures

## Goal

Ensure each semantic rejection test reaches and proves the named semantic validator.

## Fixture rule

Use one shared `BackupFixture` or equivalent builder for every semantic relationship test.

The builder must provide operations such as:

```rust
let mut fixture = BackupFixture::valid_replace();
fixture.set_index(...);
fixture.add_library("second", valid_library_bytes);
fixture.remove_library("default");
fixture.rebuild_manifest();
```

Every builder mutation must regenerate exact:

- file size;
- SHA-256;
- manifest entry path;
- entry kind;
- library index bytes.

Do not hand-code placeholder hashes in semantic tests.

## Required semantic cases

Migrate at minimum:

1. duplicate library names in `libraries.toml`;
2. zero primary libraries where one is required;
3. multiple primary libraries;
4. index references a missing library artifact;
5. library artifact is not referenced by index in replace mode;
6. case-folded duplicate library destination;
7. duplicate manifest destination;
8. index cardinality violations;
9. wrong entry kind for a canonical path;
10. semantic mismatch between manifest library set and index library set.

## Exact error assertions

Each test must assert a stable semantic error fragment specific to its rule.

Examples:

```rust
assert_stderr_contains(&output, "duplicate library name");
assert_stderr_contains(&output, "multiple primary libraries");
assert_stderr_contains(&output, "references missing library");
assert_stderr_contains(&output, "not referenced by libraries.toml");
```

Do not accept any nonzero exit without checking the intended error.

## Side-effect proof

Replace the current absence-based helper with baseline snapshot comparison.

Recommended helper:

```rust
let before = ConfigSnapshot::capture(&config_dir)?;
let output = run_restore(...);
let after = ConfigSnapshot::capture(&config_dir)?;
assert_eq!(after, before);
```

The snapshot should cover:

- `libraries.toml`;
- `libraries/*.toml`;
- `usage.toml`;
- `sync.toml`;
- `auto-sync-pending.toml`;
- transaction journals;
- transaction artifact directories;
- relevant file hashes and modes on Unix.

This allows tests to begin with existing local content and prove it remains unchanged.

## Oversized source test

The oversized-source test must create an actual source file larger than `MAX_RESTORE_SOURCE_SIZE`.

Use a sparse file where supported or write bounded repeated bytes. The manifest size and hash must match the actual source.

The test must assert the specific maximum-size error, not a checksum mismatch.

## Required tests for fixture integrity

Add one self-test that proves a freshly built valid fixture successfully reaches dry-run validation.

Add one self-test that modifying fixture bytes and rebuilding the manifest produces matching size/hash values.

## Acceptance criteria

- every semantic test contains exactly one intended defect;
- all unrelated hashes and sizes are valid;
- each test asserts the named semantic error;
- rejected restore is byte-for-byte side-effect-free relative to its baseline;
- placeholder hashes are removed from semantic relationship tests.

---

# Workstream E — Consolidate to one exact sync closure E2E

## Goal

Retain one strong real sync E2E and remove permissive or redundant release-gate assertions.

## Test selection

Choose one headline test, preferably the observer-based real server test, as the authoritative sync closure proof.

Other tests may remain for focused regressions, but the status document should cite only the one exact headline contract.

Delete or downgrade redundant tests that merely infer request count from database row count when the observer can prove the request directly.

Do not expand the observer into production telemetry.

## Exact required sequence

The headline test must prove:

```text
one local mutation
  -> one pending generation G
  -> one worker cycle for G
  -> one executor cycle for G
  -> exactly one identified sync request start
  -> exactly one matching successful finish
  -> remote state changes from R0 to R1
  -> pending generation G is cleared
  -> quiet period produces no second sync request
```

## Start/finish pairing

Require exactly one sync-related start for the measured mutation.

Require exactly one finish with the same observer sequence ID.

The finish must report success.

A successful register, health, or library request must not satisfy the sync finish assertion.

## Identity requirements

The measured sync start must include:

- non-empty authenticated user ID;
- authenticated device ID matching the client configuration or registration result;
- target library ID matching the library changed by the mutation;
- operation equal to the exact expected operation (`sync` or `push`, choose one contract and assert it).

Missing identity is a test failure. Remove diagnostic-only acceptance.

## Ordering proof

Record or obtain timestamps for:

- sync request finish;
- remote state observation;
- executor pending-clear event.

Assert that the successful request finish and remote effect occur before the pending-clear event.

If event timestamps are not sufficiently precise, use monotonic sequence evidence emitted by the test event sink rather than wall-clock comparison.

Do not infer ordering only from checking server state after pending is already absent.

## Exact count and concurrency

Assert:

- measured sync starts: exactly 1;
- matching finishes: exactly 1;
- successful matching finishes: exactly 1;
- maximum in-flight requests for the measured sync path: 1;
- server state transition: exactly R0 → R1 for the test object;
- request count remains unchanged through the quiet period.

Registration and setup requests must be excluded from measured sync counts by clearing observer history after setup or filtering by an explicit measurement boundary.

## Negative regression

Retain one false-success or unreachable-server test proving:

- local mutation commits;
- pending generation remains;
- no successful sync finish is recorded;
- remote state does not change;
- status does not claim success.

Do not require both several no-op tests and several unreachable-server tests unless they cover distinct production branches.

## Acceptance criteria

- the headline test uses exact counts, not `>= 1` or non-empty checks;
- start and finish are paired by sequence;
- identity fields are mandatory;
- remote effect and successful finish precede pending clear;
- quiet period proves no duplicate;
- the test remains test-helper-only and does not add production telemetry.

---

# Workstream F — Complete the fast-CI/deep-local verification split

## Goal

Keep the three-runner CI topology while ensuring ordinary CI does not execute every deep crash and protocol suite.

## CI topology

Do not add jobs.

Retain exactly:

1. `linux-correctness` on Ubuntu;
2. `platform-smoke` on macOS;
3. `platform-smoke` on Windows.

## Linux correctness command set

Replace the broad full integration invocation with explicit fast checks.

Recommended workflow:

```bash
bash scripts/check.sh
```

`scripts/check.sh` should run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features
cargo test --workspace --all-features --lib -- --test-threads=1
cargo test --test platform_smoke --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test destination_permissions --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
```

Equivalent focused targets are acceptable if repository test inventory shows a more representative fast set.

The normal CI list must be explicit. Do not restore `cargo test --workspace --all-features` in CI.

## Deep local suites

`scripts/release-check.sh` must run the full deep set locally, including:

- cleanup crash failpoints;
- restore crash failpoints;
- rollback crash recovery;
- deterministic real sync E2E;
- exact repair integration tests;
- manifest semantic contracts;
- production-seam proof;
- release-profile build;
- package validation.

A reasonable implementation is:

```bash
bash scripts/check.sh
cargo test --workspace --all-features -- --test-threads=1
cargo build --workspace --release --all-features
cargo test --release --test cleanup_crash_failpoints --features test-support -- --test-threads=1
cargo test --release --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --release --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --release --test repair_transactions --features test-support -- --test-threads=1
bash scripts/ci/test-production-seams.sh
```

Avoid running the same release-profile suite twice under different labels.

## Script authority

Make `.github/workflows/ci.yml` call the checked-in script rather than duplicating its command list.

This keeps local and Linux CI behavior aligned without adding another build system.

Required structure:

```yaml
- name: Linux checks
  run: bash scripts/check.sh
```

The platform-smoke matrix may retain direct commands or use a small cross-platform-compatible script where practical.

## Platform smoke

Retain:

- workspace `cargo check`;
- workspace library tests;
- `tests/platform_smoke.rs`.

Correct the `snip-sync --help` smoke test so it is a real assertion:

- invoke a known built binary or `cargo run -p snip-sync --bin snip-sync -- --help`;
- require success;
- require useful help output;
- do not treat build failure or exit 101 as acceptable smoke success.

Keep smoke tests deterministic and offline.

## Duration and reliability constraints

The design objective is not a strict timing SLA, but normal CI must avoid:

- 30-second worker debounce waits;
- multiple full real-server E2Es;
- exhaustive crash-boundary subprocess tests;
- release-profile duplication;
- packaging all crates on every push.

Deep local release verification may take longer and should be explicit.

## Acceptance criteria

- CI still has only three runner instances;
- Linux CI uses an explicit focused test list through `scripts/check.sh`;
- deep crash and protocol suites are absent from ordinary CI;
- `scripts/release-check.sh` runs those deep suites locally;
- macOS/Windows remain smoke-only;
- no new orchestration dependency is added;
- the `snip-sync --help` smoke test fails on a missing or broken binary.

---

# Workstream G — Add enforceable per-crate publish dry-run mode

## Goal

Require `cargo publish --dry-run` for the exact crate about to be published while keeping actual publishing manual.

## Release-check interface

Extend `scripts/release-check.sh` with two clear modes.

Recommended interface:

```bash
# Full local correctness and packaging validation.
bash scripts/release-check.sh verify

# Validate one exact crate immediately before manual publication.
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

Equivalent option syntax is acceptable.

## Verify mode

`verify` must:

1. require a clean working tree;
2. run `scripts/check.sh`;
3. run deep local tests;
4. run release build;
5. run production-seam proof;
6. run `cargo package --locked` for the crates selected for release or all crates conservatively;
7. avoid `--allow-dirty`.

## Dry-run mode

`dry-run <crate>` must:

1. accept only `snip-proto`, `snip-sync`, or `snip-it`;
2. require a clean working tree;
3. run `cargo publish -p <crate> --dry-run --locked`;
4. perform no actual publish;
5. return Cargo’s nonzero exit unchanged on failure.

## Dependency-order documentation

Update `RELEASING.md` to use this flow:

```text
1. Run `bash scripts/release-check.sh verify` once.
2. For each changed crate in dependency order:
   a. bump and commit the version;
   b. run `bash scripts/release-check.sh dry-run <crate>`;
   c. run `cargo publish -p <crate>` manually;
   d. wait until crates.io indexes that version before validating/publishing dependents.
```

This sequencing handles the case where `snip-sync` depends on a newly published `snip-proto` version that was not available during an earlier combined dry-run.

## Release policy

Retain:

- no GitHub Actions publishing;
- no crates.io token in GitHub;
- no tag-triggered release;
- no automatic GitHub Release;
- immutable published versions require a new version bump for corrections.

## Acceptance criteria

- the script executes, rather than prints, `cargo publish --dry-run`;
- dry-run is per crate and dependency ordered;
- actual `cargo publish` remains a manual command;
- package verification does not use `--allow-dirty`;
- no release workflow is added.

---

# Workstream H — Reconcile documentation and closure status

## Goal

Leave one accurate final record with no conflicting completion claims.

## Start-of-implementation status

At the first implementation commit, update the status document to:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11i-legacy-recovery-repair-and-verification-split-closure.md
Corrective baseline: 98acbbce29c357ae4440600dccb45a9402393e91
Final implementation commit: pending
Release process: manual crates.io publishing
CI topology: one Linux correctness job plus macOS/Windows smoke
```

Remove the statement that all Phase 11H workstreams are complete.

## Final status

Only after implementation and verification, record:

- exact final implementation commit SHA;
- Phase 11 status: `COMPLETE` or `INCOMPLETE`;
- correctness program status: `CLOSED` or `REOPENED`;
- simplified CI result for the final commit;
- local release-check result recorded as a maintainer assertion, without workflow URLs or evidence bundles;
- remaining known blockers, if any.

Do not call an earlier commit “final” when corrective commits follow it.

## Documentation updates

Update only documents that currently describe the old behavior:

- `AGENTS.md` test command summary;
- `CONTRIBUTING.md` normal versus release verification;
- `RELEASING.md` verify/dry-run/manual publish sequence;
- closure status.

Do not create another evidence document or release registry.

## Acceptance criteria

- status is internally consistent;
- Phase 11I is the authoritative remaining-work plan;
- normal CI and local release commands match actual scripts;
- manual publishing remains explicit;
- no stale “all complete” statement remains while blockers are listed.

---

# Workstream I — Focused verification matrix

## Required local verification

Run from a clean checkout at the final implementation commit.

### Fast verification

```bash
bash scripts/check.sh
```

### Exact transaction recovery

```bash
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
```

If `repair_transactions` does not exist, create one focused integration target rather than scattering process-level repair assertions across unrelated files.

### Restore semantic proof

```bash
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test restore_security --features test-support -- --test-threads=1
```

### Sync closure

```bash
cargo test --test deterministic_e2e --features test-support \
  test_observer_headline_sync_e2e -- --exact --test-threads=1
```

Use the actual final test name if renamed.

### Production seam

```bash
cargo build --release --no-default-features --target-dir target/production-seam
bash scripts/ci/test-production-seams.sh
```

### Full release verification

```bash
bash scripts/release-check.sh verify
```

### Publish dry-run

For each crate selected for release, in dependency order:

```bash
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

Run only the changed crates, and only after newly published dependency versions are indexed where applicable.

## Required CI verification

The final commit should produce only:

- Linux correctness;
- macOS platform smoke;
- Windows platform smoke.

All three must pass.

No exact workflow URL needs to be copied into the repository.

## Clean-tree check

After verification:

```bash
git status --short
```

Expected output: empty.

---

# Workstream J — Implementation sequence for reliable handoff

Use small commits. The following sequence is recommended.

## Commit 1 — Reopen status under Phase 11I

Files:

- `plans/snip-it-correctness-11-closure-status.md`

Changes:

- point to Phase 11I;
- set baseline to `98acbbce29c357ae4440600dccb45a9402393e91`;
- mark final implementation pending;
- remove contradictory completion claims.

## Commit 2 — Complete journal inventory and classification

Files:

- `src/transaction.rs`;
- focused unit tests.

Changes:

- add complete scanner;
- add recovery classifier;
- add complete artifact ownership check;
- fail closed on corrupt journals.

## Commit 3 — Add exact transaction recovery API

Files:

- `src/transaction.rs`;
- transaction tests.

Changes:

- add `recover_transaction_by_id`;
- extract direct committed-local finalization;
- centralize state-specific recovery;
- preserve lock ordering.

## Commit 4 — Rewire mutation gate

Files:

- `src/transaction.rs`;
- startup/mutation gate tests.

Changes:

- use complete scanner/classifier;
- auto-recover one exact action;
- refuse multiple or corrupt cases;
- remove duplicated branches.

## Commit 5 — Rewire repair collection and application

Files:

- `src/commands/repair_cmd.rs`;
- `src/main.rs` only if exit mapping changes;
- new `tests/repair_transactions.rs`.

Changes:

- emit all exact action variants;
- apply exact recovery API;
- test process exit semantics;
- remove dead variants or dead application paths.

## Commit 6 — Replace semantic manifest fixtures

Files:

- `tests/manifest_contracts.rs`;
- shared test support if useful.

Changes:

- migrate semantic cases to computed fixtures;
- add baseline snapshot helper;
- assert exact errors;
- create actual oversized source.

## Commit 7 — Consolidate exact sync E2E

Files:

- `tests/deterministic_e2e.rs`;
- `tests/support/recording_server.rs`;
- `snip-sync/src/test_observer.rs` only where exact pairing/identity requires it.

Changes:

- clear setup observations before measurement;
- assert one exact start/finish pair;
- require identity;
- prove finish/remote effect before pending clear;
- remove permissive diagnostic acceptance;
- delete redundant evidence-only tests if covered.

## Commit 8 — Finish CI/local split

Files:

- `.github/workflows/ci.yml`;
- `scripts/check.sh`;
- `scripts/release-check.sh`;
- `tests/platform_smoke.rs`.

Changes:

- make Linux call focused check script;
- move deep suites to release-check;
- make `snip-sync --help` smoke strict;
- retain exactly three runner instances.

## Commit 9 — Add dry-run mode and release documentation

Files:

- `scripts/release-check.sh`;
- `RELEASING.md`;
- `CONTRIBUTING.md`;
- `AGENTS.md`.

Changes:

- add `verify` and `dry-run <crate>` modes;
- remove `--allow-dirty`;
- document dependency-ordered per-crate validation and manual publish.

## Commit 10 — Final verification and status reconciliation

Files:

- `plans/snip-it-correctness-11-closure-status.md`;
- documentation only if command names changed.

Changes:

- record exact final implementation SHA;
- record truthful complete/incomplete verdict;
- do not claim closure if any acceptance criterion failed.

---

# Global acceptance checklist

## Legacy discovery and cleanup

- [ ] Complete journal scanner returns valid journals in every state.
- [ ] Legacy `Committed` with artifacts is discovered.
- [ ] Legacy `RolledBack` with artifacts is discovered.
- [ ] Terminal journal without artifacts is handled safely.
- [ ] Corrupt journal blocks mutation and appears in repair.
- [ ] Artifact ownership checks journal paths, not only the artifact root.
- [ ] Symlink and containment checks remain fail closed.

## Exact repair

- [ ] Every transaction repair carries an exact transaction ID.
- [ ] Repair revalidates current state before mutation.
- [ ] Committed-local repair does not call the global mutation gate.
- [ ] One repair item never processes unrelated journals.
- [ ] Multiple-journal tests prove isolation.
- [ ] Partial failure exits 1 through the real binary.
- [ ] Unsafe-only exits 2 through the real binary.

## Manifest proof

- [ ] Semantic fixtures compute all hashes and sizes.
- [ ] Each semantic test contains one defect.
- [ ] Each test asserts the intended semantic error.
- [ ] Baseline snapshot proves no local side effects.
- [ ] Oversized test uses an actual oversized source.
- [ ] No placeholder hash remains in semantic relationship tests.

## Sync proof

- [ ] One exact measured sync start.
- [ ] One exact matching successful finish.
- [ ] Start/finish paired by sequence.
- [ ] User, device, and library identities are required.
- [ ] Remote effect is exact.
- [ ] Successful finish and remote effect precede pending clear.
- [ ] Maximum measured concurrency is one.
- [ ] Quiet period adds no second sync request.
- [ ] Negative path preserves pending and produces no remote effect.

## CI and local verification

- [ ] CI has exactly three runner instances.
- [ ] Linux CI calls focused `scripts/check.sh`.
- [ ] Linux CI does not run the full workspace integration suite.
- [ ] macOS and Windows remain smoke-only.
- [ ] Deep crash and protocol tests run in `release-check.sh verify`.
- [ ] `snip-sync --help` smoke requires success.
- [ ] No release workflow exists.
- [ ] No evidence registry or workflow URL bookkeeping is added.

## Manual release

- [ ] `release-check.sh verify` requires a clean tree.
- [ ] Package validation omits `--allow-dirty`.
- [ ] `release-check.sh dry-run <crate>` executes Cargo dry-run.
- [ ] Only known workspace crate names are accepted.
- [ ] Actual publishing remains manual.
- [ ] Dependency-order waits are documented.
- [ ] No crates.io credential exists in GitHub configuration.

## Documentation and closure

- [ ] Phase 11I is the blocking plan during implementation.
- [ ] Final status names the actual final implementation SHA.
- [ ] No stale Phase 11H completion claim remains.
- [ ] Status and listed blockers do not contradict each other.
- [ ] Closure is declared only if no production correctness blocker remains.

---

# Stop conditions

Stop and leave Phase 11 open if any of the following remains true:

1. a legacy terminal journal with artifacts is still filtered out;
2. repair can operate on a journal other than the selected transaction ID;
3. committed-local repair still delegates to the global mutation gate;
4. a semantic test can fail because of an unrelated stale hash or size;
5. the headline sync test accepts multiple requests or an unmatched success;
6. missing device or library identity is accepted diagnostically;
7. Linux CI still runs all deep integration suites;
8. release-check only prints publish dry-runs rather than executing them;
9. an automated publish workflow is reintroduced;
10. closure status names a non-head commit as final or contains conflicting verdicts.

---

# Final closure rule

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when:

- all Phase 11I production acceptance criteria pass;
- the focused Linux check passes;
- macOS and Windows smoke checks pass;
- `scripts/release-check.sh verify` passes locally from a clean checkout;
- per-crate dry-run succeeds for every crate selected for the next manual release;
- no automated publishing workflow exists;
- the closure status records the exact final implementation commit and contains no pending blocker.

The final repository should be simpler than the Phase 11H baseline in verification behavior, not more complex.
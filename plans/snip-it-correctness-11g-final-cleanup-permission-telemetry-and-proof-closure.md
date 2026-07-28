# Phase 11G — Final Cleanup, Permission, Repair, Telemetry, and Proof Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `5f430b0a5fca2b1fce486b50445337826358a3f6`

Parent plans:

- `plans/snip-it-correctness-11-verification-and-crash-closure.md`
- `plans/snip-it-correctness-11b-durability-verification-windows-ci-closure.md`
- `plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md`
- `plans/snip-it-correctness-11d-pending-staging-and-cross-platform-proof-closure.md`
- `plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md`
- `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan supersedes Phase 11F for all remaining-work and release-closure decisions. Phase 11F materially improved the repository, but its final implementation commit does not satisfy every release-blocking criterion. This plan addresses only the verified residual defects. It must not be expanded into a new architecture phase.

Phase 11 and the correctness program remain open until all acceptance criteria in this plan pass on Linux, macOS, and Windows where applicable, and the same final commit has successful GitHub Actions evidence.

---

## 1. Objective

Close the remaining correctness, security, recovery, repair, test-proof, and evidence gaps:

1. eliminate the crash window between terminal transaction state and durable cleanup ownership;
2. make cleanup progress and recovery coordinates internally consistent;
3. recover legacy terminal journals that still own artifacts;
4. apply the explicit private destination policy to every restored state file, not only libraries;
5. remove the implicit `0644` fallback for new destinations;
6. add exact permission and permission-failure tests;
7. replace remaining multi-fault manifest fixtures with valid single-fault fixtures;
8. prove every semantic test reaches the intended semantic validator;
9. make no-side-effect assertions standard for every rejected manifest;
10. make production-seam scripts traverse the exact guarded code paths;
11. prove production worker spawning without relying on disabled auto-sync;
12. connect recording telemetry to the real server request path;
13. record bounded, sanitized identity, revision, payload, ordering, and concurrency evidence;
14. make repair actions transaction-specific and state-aware;
15. resume cleanup instead of rolling back committed or cleanup-pending transactions;
16. return nonzero from the CLI on partial repair failure;
17. add crash tests at every cleanup boundary and on a second cleanup crash;
18. add exact artifact and destination mode tests;
19. remove contradictory closure claims;
20. record successful same-commit Linux, macOS, Windows, production-seam, and package evidence.

---

## 2. Architectural constraints and non-goals

Preserve all of the following:

- one installed binary: `snp`;
- detached auto-sync work remains one-shot;
- no resident client daemon;
- no second installed helper binary;
- TOML remains authoritative local state;
- no database replacing local TOML;
- no transaction service;
- no plugin runtime;
- no workflow engine;
- no CRDT expansion;
- no broad CLI redesign;
- no secrets, snippet commands, or plaintext payloads in process arguments, telemetry, journals, status files, or logs.

Allowed internal changes:

- transaction-state compatibility classification;
- a corrected cleanup state machine;
- additional test-only failpoints behind `test-support`;
- explicit destination security classes for all backup entry kinds;
- a shared manifest fixture builder;
- request-observer hooks in test server infrastructure;
- typed repair targets and outcomes;
- explicit CLI mapping for repair outcomes;
- checked-in CI helper scripts and evidence documentation.

Do not solve these defects by adding another process architecture, a persistent service, or a generalized observability subsystem.

---

## 3. Confirmed residual defects

The implementation agent must treat every item in this section as open.

### 3.1 Terminal transaction state is persisted before cleanup ownership

`commit_transaction` persists `Committed` before `finalize_transaction_cleanup` persists `CleaningUp`. Rollback persists `RolledBack` before the same transition.

A crash in that interval leaves a terminal journal and transaction artifacts. Terminal journals are ignored by interrupted-transaction recovery, so cleanup is no longer automatically owned by any state-machine path.

### 3.2 Cleanup position definitions disagree

The enum documentation, cleanup implementation, architecture documentation, and closure status describe different step counts and coordinates. The implementation folds validation and staged removal into one position, uses five loop positions, and describes six conceptual operations.

The persisted cursor must have one unambiguous meaning everywhere.

### 3.3 Legacy terminal journals with artifacts are not recovered

Older or partially implemented versions can leave `Committed` or `RolledBack` journals with staged or backup artifacts. Current `is_interruptible` classification ignores them even when cleanup remains necessary.

### 3.4 Permission policy is only fully applied to library files

Library installation uses `DestinationClass`. Index, usage, and sync-config installation still calls `apply_original_metadata` directly. When no original metadata exists, that helper falls back to `0644`.

A new `sync.toml` can therefore be created privately and then downgraded to `0644`.

### 3.5 Permission tests and failure injection are absent

The repository does not yet prove exact modes for every transaction directory/file and every restored destination class. It also does not prove that permission setup or verification failure occurs before sensitive bytes are accepted or live destinations change.

### 3.6 Manifest semantic fixtures remain multi-fault

Several tests mutate index content without updating manifest size or checksum. Others retain hard-coded or dummy hashes. Restore validates size and checksum before semantic index validation, so these tests can pass without reaching the named semantic rule.

### 3.7 Manifest rejection side effects are not asserted uniformly

Only a subset of negative tests asserts no journal, artifact root, pending marker, or live destination mutation.

### 3.8 Production-seam restore proof uses dry-run

The matching restore failpoint is configured while `restore --mode dry-run` is executed. Dry-run returns before transaction creation and never reaches the failpoint.

### 3.9 Production worker-suppression proof disables auto-sync

The script sets `auto_sync = false`, so a successful mutation does not prove `SNP_SKIP_WORKER_SPAWN` was ignored.

### 3.10 Production event and executor proofs may fail before the intended path

The executor/event tests do not establish a canonical pending state and fully valid execution context before invoking the internal executor. A nonzero result can therefore come from unrelated setup failure.

### 3.11 Production mutation-barrier proof may use a mismatched point name

The script duplicates a barrier point string rather than deriving or sharing the exact production point. A mismatched value trivially proves nonblocking without proving compile-time isolation.

### 3.12 Recording telemetry is disconnected from real requests

`RecordingServer` owns vectors and manual `record_request` helpers, but the real test service handlers do not append to those vectors. The main end-to-end test still discards the server capture and relies on database row count plus local configuration.

### 3.13 Telemetry fields are incomplete

The current `RecordedRequest` lacks revision, payload length/hash, plaintext-sentinel result, start/finish timestamps, and in-flight concurrency. Exact pending-clear ordering cannot be proven from it.

### 3.14 Repair collapses all interrupted transactions into rollback

Every interrupted journal is represented as `RollbackInterruptedTransaction`. Applying one repair item loops over every interrupted journal and calls rollback, including `CleaningUp` and `CommittedLocal` states.

Committed local data or cleanup-pending transactions must be finalized, not rolled back.

### 3.15 Repair actions are not transaction-specific

A repair item does not identify the exact transaction it owns. Multiple repair items can each reprocess all journals.

### 3.16 Partial repair failure exits zero

`repair_cmd::run` calculates `PartialFailure`, but the CLI dispatcher discards the result and returns a successful process outcome.

### 3.17 Cleanup crash and permission adversarial tests are missing

The implementation added cleanup failpoint constants but did not add release-blocking subprocess tests for every cleanup boundary, second-crash recovery, or permission failures.

### 3.18 Closure status is contradictory

The status file declares Phase 11 complete and closed while the final implementation commit and workflow evidence are explicitly pending.

### 3.19 Same-commit CI evidence is absent

No successful final Linux/macOS/Windows matrix and package evidence is recorded for the implementation commit.

---

# Workstream A — Reopen the closure record accurately

## Goal

Make the repository status truthful before code changes continue.

## Required first commit

Update `plans/snip-it-correctness-11-closure-status.md` to contain:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11g-final-cleanup-permission-telemetry-and-proof-closure.md
Corrective baseline: 5f430b0a5fca2b1fce486b50445337826358a3f6
Final implementation commit: pending
Final workflow evidence: pending
```

Mark Phase 11F evidence as historical and partial. Remove the claim that all workstreams are implemented, tested, and evidenced.

## Acceptance criteria

- no remaining Phase 11G item is marked complete;
- local command output is not presented as cross-platform evidence;
- `COMPLETE`, `CLOSED`, `release-ready`, and equivalent claims are absent until the final evidence commit;
- the status document names this plan as authoritative.

---

# Workstream B — Make cleanup ownership durable before cleanup begins

## Goal

Eliminate the terminal-state crash window and make cleanup restartable from every boundary.

## Required state-machine correction

Do not persist `Committed` or `RolledBack` before cleanup. The transition must be:

```text
CommittedLocal(Recorded | CoveredByExisting)
  -> CleaningUp { outcome: Commit, next_step: Validate }
  -> ... cleanup steps ...
  -> journal removed last

RollingBack(all actions complete)
  -> CleaningUp { outcome: Rollback, next_step: Validate }
  -> ... cleanup steps ...
  -> journal removed last
```

One acceptable typed model is:

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
    // ...
    CleaningUp {
        outcome: CleanupOutcome,
        next_step: CleanupStep,
    },
    // terminal states may remain only for backward compatibility
}
```

An integer cursor is acceptable only if one constant table defines its meaning and every document/test imports or mirrors that table exactly.

## Required ordering

1. persist `CleaningUp { outcome, next_step: Validate }`;
2. validate containment and symlink policy;
3. persist the next step after successful validation;
4. remove staged files idempotently;
5. persist progress;
6. remove backups idempotently;
7. persist progress;
8. remove the transaction artifact root with bounded platform handling;
9. persist `next_step: RemoveJournal`;
10. remove journal last;
11. sync the transaction directory where supported;
12. return success.

Do not persist a terminal state between transaction completion and the first cleanup state.

## Compatibility handling

Classify legacy journals explicitly:

- `Committed` plus any artifact: cleanup as committed;
- `RolledBack` plus any artifact: cleanup as rolled back;
- terminal journal with no artifacts: remove stale terminal journal through a safe typed repair or compatibility cleanup path;
- `CommittedLocal`: finalize pending, then cleanup;
- `CleaningUp`: resume exact cleanup step;
- `Prepared`, `BackupsDurable`, `Committing`, `RollingBack`: continue the existing rollback/recovery policy.

Do not silently ignore a terminal journal that still owns files.

## Canonical APIs

All paths must use one API:

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

`commit_transaction`, `rollback_transaction`, `CommittedLocal` recovery, startup recovery, and repair must call these APIs rather than duplicating deletion logic.

## Acceptance criteria

- no commit/rollback path persists a terminal state before cleanup ownership;
- a crash immediately before the first deletion is recoverable;
- a crash after every deletion step is recoverable;
- cleanup retry is idempotent when a file or directory is already absent;
- legacy terminal journals with artifacts are discovered;
- journal removal remains the last destructive authority-removal step;
- cleanup errors remain visible and recoverable.

---

# Workstream C — Add exact cleanup crash tests

## Goal

Prove cleanup survives a crash at every persisted boundary, including a second crash during recovery.

## Required failpoints

Use exact stable names and semantics:

- `cleanup-after-state-before-validation`;
- `cleanup-after-validation-before-staged`;
- `cleanup-after-staged-before-backups`;
- `cleanup-after-backups-before-artifact-root`;
- `cleanup-after-artifact-root-before-journal`;
- `cleanup-after-journal-removal-before-parent-sync` where the platform can meaningfully exercise it.

Each failpoint must be behind `test-support`; a no-feature production binary must ignore every matching value.

## Required subprocess scenarios

For both commit and rollback outcomes:

1. create a real multi-file transaction with staged and backup artifacts;
2. launch the real feature-enabled `snp` binary;
3. abort at the target cleanup boundary;
4. inspect the exact journal state and remaining files before recovery;
5. run a normal mutating command or `snp repair --apply` to trigger recovery;
6. verify cleanup completes without changing the committed/rolled-back live result;
7. rerun recovery and prove no-op idempotence.

At least one scenario must crash again during cleanup recovery at a later step, then recover successfully on the third process.

## Required assertions

- live committed bytes remain committed for commit cleanup;
- live original bytes and permissions remain restored for rollback cleanup;
- no pending generation is duplicated;
- staged and backup files are removed exactly as progress advances;
- journal remains until its removal step;
- no orphan artifact root remains after success;
- a second recovery run is a no-op.

## Acceptance criteria

- every cleanup step has substantive crash coverage;
- tests inspect pre-recovery state, not only the final state;
- commit and rollback outcomes are both covered;
- a second cleanup crash is covered;
- production seam proof confirms cleanup failpoints are absent without `test-support`.

---

# Workstream D — Apply destination permission policy to every entry kind

## Goal

Ensure every new restored local state file is private and every existing file preserves the supported sanitized metadata contract.

## Required model

Use destination kind plus existence, not a generic “restore” boolean:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationKind {
    Library,
    LibraryIndex,
    UsageIndex,
    SyncConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationPolicy {
    NewPrivate,
    ExistingPreserved,
}

pub fn destination_policy(
    kind: DestinationKind,
    existed_before: bool,
    original: &OriginalFileMetadata,
) -> SnipResult<ExpectedMetadata>;
```

Required Unix policy:

- all new libraries: `0600`;
- new `libraries.toml`: `0600`;
- new `usage.toml`: `0600`;
- new `sync.toml`: `0600`;
- existing destinations: preserve sanitized original mode;
- strip setuid, setgid, and sticky bits;
- preserve readonly semantics where supported;
- verify final metadata after installation.

## Remove ambiguous fallback

`OriginalFileMetadata::default()` must not imply `0644`.

`apply_original_metadata` must either:

- require an existing destination with captured metadata; or
- accept an explicit expected metadata value supplied by destination policy.

It must never choose a new-file mode implicitly.

## Installation requirements

Apply the same shared installation helper to:

- library files;
- index;
- usage;
- sync config.

The helper must:

1. choose durability based on destination kind/policy;
2. atomically install content;
3. apply expected metadata;
4. reopen and verify hash;
5. verify exact supported metadata;
6. return error before progress is persisted if verification fails.

Do not special-case only libraries.

## Windows contract

- preserve readonly where supported;
- do not claim Unix mode guarantees;
- ensure new files remain beneath the user-owned config directory;
- permission/attribute errors must not be silently discarded where they affect the supported contract.

## Acceptance criteria

- no new destination can become `0644` through metadata fallback;
- new `sync.toml` remains private after every helper call;
- all four destination kinds share the explicit policy;
- existing mode preservation is verified;
- rollback restores original supported metadata;
- documentation states only the implemented platform contract.

---

# Workstream E — Add permission and confidentiality adversarial tests

## Goal

Prove private creation, exact destination modes, rollback restoration, and fail-closed permission behavior.

## Required Unix tests

Assert exact modes for:

- `.transaction/`;
- `artifacts/`;
- `artifacts/<txn-id>/`;
- `backups/`;
- `staged/`;
- every `.bak`;
- every `.new`;
- transaction journals;
- transaction locks where the product owns their creation policy;
- new library;
- new `libraries.toml`;
- new `usage.toml`;
- new `sync.toml`.

Expected private directory mode: `0700`.

Expected private file mode: `0600` unless an explicit documented exception exists.

Also test:

- existing `0640` remains `0640` after restore;
- existing readonly remains readonly;
- setuid/setgid/sticky bits are stripped;
- rollback restores original sanitized mode;
- successful cleanup removes all plaintext staged content.

## Permission failure injection

Add a narrow `test-support` seam at these points:

- private directory creation/verification;
- private artifact file creation/verification;
- destination metadata application;
- destination metadata verification.

The seam must return an error before sensitive bytes are accepted as durable at the targeted boundary.

For each injected failure assert:

- nonzero exit;
- no insecure artifact remains;
- no live destination changes, or rollback restores exact original state;
- no pending generation is created for an unsuccessful restore;
- journal remains only when it is required for safe recovery;
- retry after removing the fault succeeds or reports a precise repair action.

## Production proof

Build without `test-support`, set every valid permission-injection variable/value, and prove normal behavior is unaffected.

## Acceptance criteria

- exact mode tests run on Linux and macOS;
- Windows attribute tests run on Windows;
- permission failure is fail-closed;
- no test relies only on umask;
- production cannot activate permission injection.

---

# Workstream F — Replace remaining manifest fixtures with single-fault builders

## Goal

Ensure every manifest test proves the named validation rule and no unrelated rule.

## Shared fixture builder

Create a builder that owns artifact bytes and regenerates manifest metadata automatically:

```rust
struct BackupFixture {
    root: TempDir,
    manifest: BackupManifest,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl BackupFixture {
    fn valid_default() -> Self;
    fn set_index(&mut self, index: LibraryConfig);
    fn add_library(&mut self, name: &str, snippets: Snippets);
    fn mutate_manifest_only(&mut self, f: impl FnOnce(&mut BackupManifest));
    fn write_recomputed(&self) -> PathBuf;
}
```

The exact API may differ. Required behavior:

- real files are written;
- sizes are derived from bytes;
- hashes are derived from bytes;
- unrelated entries remain valid;
- one mutation introduces one targeted fault.

## Required corrections

Correct every existing test that contains:

- `placeholder` content when content validity is unrelated;
- all-zero or dummy hashes;
- hard-coded stale sizes;
- index mutation without manifest regeneration;
- broad “any failure” assertions;
- multiple simultaneous defects.

## Exact assertions

Each negative test must assert:

- nonzero exit;
- stable intended error category or exact diagnostic substring;
- no checksum/size error unless that is the target;
- no transaction journal;
- no transaction artifact root;
- no pending marker;
- no live destination mutation.

Use a shared `assert_rejected_without_side_effects` helper.

## Required semantic cases

At minimum, prove exact rejection for:

- duplicate library name in index;
- case-folded duplicate name in index;
- multiple primary libraries;
- index references missing library artifact;
- unreferenced library artifact in replace mode;
- documented merge-mode policy for unreferenced artifacts;
- duplicate snippet IDs;
- malformed required library fields;
- duplicate exact destination;
- portable path alias collision;
- schema zero;
- future schema;
- unsupported layout;
- unknown kind;
- size mismatch;
- checksum mismatch;
- oversized source;
- symlink source.

## Acceptance criteria

- every semantic test reaches semantic validation;
- no semantic test can pass due to size/checksum mismatch;
- no negative test accepts multiple error classes;
- all rejected inputs prove zero mutation side effects;
- valid control fixtures succeed in dry-run and the appropriate write mode.

---

# Workstream G — Make production-seam proofs traverse real guarded paths

## Goal

Prove a no-feature production binary ignores matching test controls by executing the exact guarded paths.

## General rules

- build with `--release --no-default-features` in an isolated target directory;
- use exact matching seam names from one checked-in source of truth;
- establish valid configuration and state before invocation;
- reject argument-parsing, missing-config, missing-pending, and unrelated setup failures as evidence;
- bound every child process;
- run equivalent Bash and PowerShell proofs.

## Restore failpoint proof

Use a real write-mode restore, not dry-run:

1. create a valid backup with computed hashes/sizes;
2. create isolated config;
3. run `restore --mode replace` with `SNP_TEST_FAILPOINT=restore-after-prepared`;
4. assert exit success;
5. assert restored live files exist with expected bytes;
6. assert no interrupted journal/artifact remains;
7. assert no abort signal or failpoint diagnostic.

## Executor no-op proof

Establish:

- valid sync configuration;
- valid credential delivery through a normal production-supported path;
- valid canonical pending state for generation `G`;
- an intentionally unreachable or rejecting server.

Run `auto-sync-execute --generation G` with `SNP_TEST_EXECUTOR_MODE=noop-success`.

Assert:

- the command reaches the real executor path;
- it exits with the documented network/auth class, not zero;
- stderr/status excludes parser/setup failure;
- pending generation `G` remains.

## Worker suppression proof

Enable auto-sync. Perform a real mutation with `SNP_SKIP_WORKER_SPAWN=1`.

Prove worker execution using one of these production-observable effects:

- a bounded status transition produced by the worker against an intentionally failing server;
- a pending retry/backoff transition that only the worker can produce;
- a controlled test server receiving the request.

Do not disable auto-sync. A created library alone is not proof that a worker spawned.

## Event sink proof

Run a valid worker/executor cycle with `SNP_TEST_EVENTS_DIR` set. Assert no event file is created while normal production status/pending behavior occurs.

## Mutation barrier proof

Use the exact barrier point used by the real mutation. Prefer a shared constant exported only to the script-generation/build helper rather than duplicated strings.

Run the mutation without a release file and prove it exits normally. Assert no `entered` file appears.

## Permission and cleanup seam proof

Set matching valid permission-injection and cleanup failpoint values against the no-feature binary. Traverse their real paths and prove they do not activate.

## Acceptance criteria

- no proof uses `list` for a mutation seam;
- no restore proof uses dry-run for a transaction failpoint;
- worker proof has auto-sync enabled;
- executor proof has valid pending and configuration state;
- barrier point names exactly match production;
- Bash and PowerShell pass on CI.

---

# Workstream H — Connect sanitized telemetry to the real server path

## Goal

Make request evidence automatic, exact, bounded, and derived from actual protocol handling.

## Required instrumentation architecture

Add a test-only observer interface to the server service or transport boundary used by `start_test_server`:

```rust
#[derive(Debug, Clone)]
pub struct RequestStarted {
    pub sequence: u64,
    pub started_at_unix_ms: u64,
    pub method: String,
    pub path: String,
    pub operation: String,
    pub authenticated_user_id: Option<String>,
    pub authenticated_device_id: Option<String>,
    pub target_library_id: Option<String>,
    pub request_revision: Option<u64>,
    pub payload_len: usize,
    pub payload_sha256: String,
    pub payload_contains_plaintext_sentinel: bool,
    pub concurrent_at_start: usize,
}

#[derive(Debug, Clone)]
pub struct RequestFinished {
    pub sequence: u64,
    pub finished_at_unix_ms: u64,
    pub success: bool,
    pub response_revision: Option<u64>,
}

pub trait TestRequestObserver: Send + Sync {
    fn request_started(&self, event: RequestStarted);
    fn request_finished(&self, event: RequestFinished);
}
```

Equivalent design is acceptable. Requirements:

- handlers invoke it automatically;
- tests cannot create evidence by manually calling `record_request`;
- observer is test infrastructure only;
- records are bounded, for example maximum 256 requests;
- no API key, authorization header, raw body, decrypted snippet command, or plaintext payload is retained;
- payload evidence is length, hash, and sentinel boolean only;
- in-flight counter is incremented/decremented around the actual handler future;
- sequence and timestamps permit ordering assertions.

## RecordingServer integration

`RecordingServer::start` must pass its observer into the real service before the server starts. Remove disconnected vectors or make them the actual observer sink.

The public test helper must expose:

- exact records;
- maximum concurrent requests;
- exact operation counts;
- exact success/failure counts;
- expected identity lookup;
- first start and finish times;
- bounded wait for exact request count;
- quiet-period assertion.

## Required headline E2E assertions

For one local mutation:

1. retain the recording handle;
2. record server revision/state `R0`;
3. record pending generation `G`;
4. observe exactly one canonical sync request;
5. assert expected authenticated user/device;
6. assert expected target library;
7. assert nonempty payload length and stable payload hash;
8. assert plaintext command sentinel is absent;
9. assert request revision and response revision obey the protocol contract;
10. assert maximum in-flight requests is `1`;
11. assert request finished successfully;
12. assert pending generation `G` was cleared only after request finish/acknowledgement;
13. assert server revision/state becomes exact `R1`;
14. wait effective debounce plus safety margin and assert total request count remains one.

Use timestamps/events for ordering rather than relying only on sleeps.

## Required negative assertions

For auth failure, unreachable server, timeout, conflict, and false-success executor:

- no successful acknowledged request record;
- pending remains;
- status is non-success and truthful;
- no duplicate request storm;
- maximum concurrency remains within contract;
- no secret appears in telemetry.

## Acceptance criteria

- the main E2E test no longer discards capture handles;
- telemetry is populated by real handlers automatically;
- manual `record_request` calls are not used as primary evidence;
- exact identity, revision, payload, concurrency, and ordering assertions pass;
- telemetry remains sanitized and bounded.

---

# Workstream I — Make repair transaction-specific and state-aware

## Goal

Apply exactly one correct recovery action to exactly one transaction and return truthful process status.

## Required typed targets

Use actions that identify the transaction:

```rust
pub enum RepairAction {
    PruneOrphanedUsage,
    ResumeCleanup { transaction_id: String },
    FinalizeCommittedLocal { transaction_id: String },
    RollbackInterrupted { transaction_id: String },
    RemoveOrphanedArtifact { path: PathBuf },
    RemoveStaleTerminalJournal { transaction_id: String },
    // existing manual-only domain repairs
}
```

Equivalent `RepairKind + RepairTarget` structure is acceptable. Do not use one generic action that loops over all journals.

## State classification

- `CleaningUp`: `ResumeCleanup`;
- `CommittedLocal`: `FinalizeCommittedLocal`;
- legacy `Committed`/`RolledBack` with artifacts: `ResumeCleanup` with inferred outcome;
- `Prepared`, `BackupsDurable`, `Committing`, `RollingBack`: `RollbackInterrupted` under the existing safe policy;
- terminal journal without artifacts: `RemoveStaleTerminalJournal` only after validation;
- orphan artifact without journal: `RemoveOrphanedArtifact` after containment and symlink checks;
- corrupt journal: unsafe/manual unless an explicit safe parser/recovery path exists.

## Apply behavior

For each repair item:

1. re-read the exact target immediately before mutation;
2. verify it still matches the planned action;
3. acquire the correct lock hierarchy;
4. call the canonical transaction API;
5. apply only that target;
6. report exact success/failure;
7. continue to other independent actions while retaining partial-failure status.

Do not roll back a committed or cleanup-pending transaction.

## CLI exit mapping

Map `RepairExitStatus` explicitly:

- `Clean`, `Repaired`, `DryRun`: exit `0`;
- `UnsafeOnly`: documented nonzero attention code or existing validation code;
- `PartialFailure`: nonzero runtime/repair failure code.

The main dispatcher must not discard the status.

One acceptable API:

```rust
impl RepairExitStatus {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Clean | Self::Repaired | Self::DryRun => 0,
            Self::UnsafeOnly => 2,
            Self::PartialFailure => 1,
        }
    }
}
```

Use existing project exit-code policy if different; document it and test it.

## Required tests

- one `CleaningUp` journal resumes cleanup, not rollback;
- one `CommittedLocal` journal finalizes pending and cleanup;
- one interrupted commit rolls back only that transaction;
- two journals produce two transaction-specific items;
- applying one item does not process the other;
- stale state between dry-run and apply is revalidated;
- orphan containment traversal is rejected;
- symlink orphan is rejected;
- one successful and one failing action yields partial failure and nonzero exit;
- second apply is idempotent.

## Acceptance criteria

- no repair action loops over all interrupted journals;
- cleanup-pending and committed-local states are never rolled back;
- partial failure exits nonzero;
- JSON and human reports identify exact targets without secrets;
- repair uses canonical transaction APIs.

---

# Workstream J — Complete CI and same-commit evidence

## Goal

Demonstrate the corrected repository on one exact commit and reconcile documentation only after evidence exists.

## Required focused jobs

### Cleanup and permission

Linux, macOS, Windows where applicable:

- cleanup crash boundary suite;
- second-crash cleanup recovery;
- legacy terminal-journal compatibility;
- exact Unix artifact modes;
- destination mode policy for all entry kinds;
- Windows readonly contract;
- permission failure injection;
- production no-feature cleanup/permission seam proof.

### Manifest

Linux, macOS, Windows:

- all structural tests;
- all semantic tests with valid single-fault fixtures;
- no-side-effect assertions;
- valid write-mode control restore.

### Auto-sync telemetry

Linux, macOS, Windows:

- exact request telemetry headline test;
- false-success executor;
- auth/network/timeout/conflict preservation;
- quiet-period duplicate absence;
- maximum concurrency assertion.

### Repair

Linux, macOS, Windows:

- state-aware repair classification;
- transaction-specific apply;
- partial failure exit code;
- orphan containment/symlink handling.

### Production seam

Linux and Windows minimum, macOS recommended:

- real restore failpoint traversal;
- real executor with valid pending state;
- enabled auto-sync worker suppression proof;
- real event path;
- exact mutation barrier point;
- cleanup and permission seam absence.

### General gates

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- dev and release tests on Linux, macOS, Windows;
- package/install smoke on Linux, macOS, Windows;
- no permissive skips or ignored failures in release-blocking jobs.

## Workflow quality rules

- no `continue-on-error` for release gates;
- no `|| true` masking proof failure;
- no test returns early because a required seam is unavailable;
- bound every worker, executor, server, and subprocess;
- upload sanitized transaction/pending/status/test output on failure;
- pin mutable third-party actions to full commit SHAs or document a narrow reviewed exception;
- do not treat a “verify files are committed” job as execution evidence;
- record actual workflow run and job URLs.

## Same-commit closure record

Only after all jobs pass, update the closure status with:

- final commit SHA;
- workflow run URL/ID;
- each release-blocking job name and conclusion;
- OS/profile matrix;
- production-seam results;
- package results;
- any precisely scoped platform limitation;
- final release decision.

All evidence must refer to one commit.

## Acceptance criteria

- Linux release gates pass;
- macOS release gates pass;
- Windows release gates pass;
- production seam passes on Linux and Windows;
- package smoke passes on all three;
- status evidence uses one exact commit;
- no implementation or evidence item remains pending when closure is declared.

---

## 4. Cross-cutting implementation rules

### 4.1 Cleanup authority must never disappear early

The journal may be removed only after every required artifact cleanup step succeeds. No terminal state may cause recovery to ignore remaining artifacts.

### 4.2 Pending intent must remain conservative

Preserve pending on every uncertain sync result. Cleanup correction must not create another pending generation or clear an existing generation.

### 4.3 New local state is private by policy

Do not infer new-file mode from absent original metadata. Every destination kind must select a policy explicitly.

### 4.4 Test evidence must reach the named boundary

A failure before the target function is not evidence. Every seam and semantic test must prove it traversed the intended path.

### 4.5 Telemetry must be real and sanitized

Evidence must originate at the actual server request boundary. Do not retain raw payloads, credentials, headers, or snippet commands.

### 4.6 Repair must be exact

One item owns one target and one action. Re-read state immediately before mutation. Never infer destructive behavior from human-readable text.

### 4.7 Do not discard critical results

Prohibit ignored results for:

- cleanup state persistence;
- staged/backup/artifact/journal deletion;
- permission application/verification;
- pending finalization;
- repair actions;
- CLI repair outcome mapping;
- telemetry observer failures that invalidate a release-blocking test.

### 4.8 Preserve lightweight architecture

The accepted implementation remains one binary, one-shot workers/executors, short-lived locks, bounded TOML recovery state, and test-only instrumentation.

---

## 5. Recommended implementation sequence

Use small auditable commits:

1. `docs: reopen Phase 11 under Phase 11G`
2. `transaction: define canonical cleanup outcome and step model`
3. `transaction: enter cleanup before terminal state`
4. `transaction: recover legacy terminal journals with artifacts`
5. `transaction: unify commit rollback and recovery cleanup APIs`
6. `tests: add commit cleanup boundary crashes`
7. `tests: add rollback cleanup boundary crashes`
8. `tests: add second-crash cleanup recovery`
9. `restore: define destination kind and expected metadata policy`
10. `restore: route index usage and sync config through shared installer`
11. `transaction: remove implicit new-file metadata fallback`
12. `tests: assert exact artifact and destination modes`
13. `test-support: add permission failure injection`
14. `tests: prove permission failures are fail-closed`
15. `tests: add shared valid backup fixture builder`
16. `tests: convert all manifest negatives to single-fault fixtures`
17. `tests: standardize no-side-effect manifest assertions`
18. `ci: correct real production restore failpoint proof`
19. `ci: correct executor worker event and barrier seam proofs`
20. `sync-test: add real request observer hook`
21. `sync-test: add bounded sanitized request and concurrency records`
22. `tests: rewrite headline E2E around exact telemetry`
23. `tests: add exact negative telemetry assertions`
24. `repair: add transaction-specific state-aware actions`
25. `repair: route cleanup and committed-local recovery correctly`
26. `cli: map repair partial failure to nonzero exit`
27. `tests: add repair state and exit-code matrix`
28. `ci: add Phase 11G release-blocking jobs`
29. `docs: record one-commit CI evidence and close Phase 11`

Do not combine all implementation into one opaque commit. Keep invariants reviewable.

---

## 6. Required verification commands

Run from repository root.

### Static and build

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features
cargo build --workspace --release --no-default-features
```

### Cleanup and transaction

```bash
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test cleanup_crash_recovery --features test-support -- --test-threads=1
cargo test --test transaction_permissions --features test-support -- --test-threads=1
```

Create the last two suites if they do not exist.

### Manifest

```bash
cargo test --test manifest_contracts --features test-support -- --test-threads=1
```

### Auto-sync and telemetry

```bash
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test auto_sync_lifecycle --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
```

### Repair

```bash
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test repair_exit_status --features test-support -- --test-threads=1
```

Create these suites if absent.

### Production seam

```bash
bash scripts/ci/test-production-seams.sh
pwsh -File scripts/ci/test-production-seams.ps1
```

### Full suites

```bash
cargo test --workspace --all-features -- --test-threads=1
cargo test --workspace --all-features --release -- --test-threads=1
```

Run equivalent jobs on Linux, macOS, and Windows.

---

## 7. Mandatory closure matrix

| Contract | Linux | macOS | Windows | Required proof |
|---|---:|---:|---:|---|
| Cleanup ownership persisted before deletion | Yes | Yes | Yes | crash before first deletion recovers |
| Crash after staged cleanup | Yes | Yes | Yes | cursor and remaining artifacts exact |
| Crash after backup cleanup | Yes | Yes | Yes | cursor and remaining artifacts exact |
| Crash before journal removal | Yes | Yes | Yes | journal remains recovery authority |
| Second crash during cleanup recovery | Yes | Yes | Yes | third process completes idempotently |
| Legacy terminal journal with artifacts | Yes | Yes | Yes | cleanup resumes, no rollback of commit |
| Exact transaction directory/file modes | Yes | Yes | Platform contract | `0700`/`0600` assertions |
| New library/index/usage/sync modes | Yes | Yes | Platform contract | explicit policy assertions |
| Existing mode and readonly preservation | Yes | Yes | Yes | supported metadata restored |
| Permission failure before sensitive durability | Yes | Yes | Platform contract | nonzero and no insecure residue |
| Manifest single-fault fixtures | Yes | Yes | Yes | intended diagnostic only |
| Manifest rejection zero side effects | Yes | Yes | Yes | no journal/artifact/pending/live write |
| Production real restore failpoint path | Yes | Recommended | Yes | replace restore completes normally |
| Production executor valid pending path | Yes | Recommended | Yes | real failure class, pending retained |
| Production worker spawn with auto-sync enabled | Yes | Recommended | Yes | worker-only observable effect |
| Production exact barrier path | Yes | Recommended | Yes | no blocking and no entered marker |
| Real request telemetry | Yes | Yes | Yes | automatic handler records |
| Exact request count and max concurrency | Yes | Yes | Yes | one request, max in-flight one |
| Identity/revision/payload evidence | Yes | Yes | Yes | expected sanitized values |
| Pending clear after acknowledgement | Yes | Yes | Yes | timestamp/order proof |
| State-aware transaction repair | Yes | Yes | Yes | cleanup/finalize/rollback selected correctly |
| Partial repair failure exit | Yes | Yes | Yes | nonzero process exit |
| Package/install smoke | Yes | Yes | Yes | installed binary help/version |

“Platform contract” must be documented precisely. Unsupported guarantees must not be marked complete by skipping silently.

---

## 8. Release-blocking checklist

### Status truth

- [ ] Phase 11 is marked incomplete during implementation.
- [ ] Phase 11G is the authoritative blocking plan.
- [ ] No final commit or workflow evidence is claimed while pending.
- [ ] Closure language appears only after all gates pass.

### Cleanup state machine

- [ ] Commit enters `CleaningUp` before any terminal state.
- [ ] Rollback enters `CleaningUp` before any terminal state.
- [ ] One cleanup coordinate model is used everywhere.
- [ ] Every cleanup operation is idempotent.
- [ ] Legacy terminal journals with artifacts are recovered.
- [ ] Journal is removed last.
- [ ] Cleanup errors preserve recoverability.

### Cleanup crash proof

- [ ] Crash before validation is tested.
- [ ] Crash before staged removal is tested.
- [ ] Crash before backup removal is tested.
- [ ] Crash before artifact-root removal is tested.
- [ ] Crash before journal removal is tested.
- [ ] Commit cleanup is tested.
- [ ] Rollback cleanup is tested.
- [ ] Second-crash recovery is tested.
- [ ] Second normal recovery is a no-op.

### Permission policy

- [ ] Library uses explicit new/existing policy.
- [ ] Index uses explicit new/existing policy.
- [ ] Usage uses explicit new/existing policy.
- [ ] Sync config uses explicit new/existing policy.
- [ ] No new-file `0644` fallback exists.
- [ ] Exact artifact modes are tested.
- [ ] Exact destination modes are tested.
- [ ] Existing modes and readonly state are preserved.
- [ ] Permission failure is fail-closed.
- [ ] Production ignores permission injection.

### Manifest proof

- [ ] Shared builder computes all sizes and hashes.
- [ ] Every negative fixture has one fault.
- [ ] Every semantic test reaches semantic validation.
- [ ] Exact intended diagnostics are asserted.
- [ ] Every rejection proves no journal.
- [ ] Every rejection proves no artifacts.
- [ ] Every rejection proves no pending marker.
- [ ] Every rejection proves no live mutation.
- [ ] Valid write-mode controls succeed.

### Production seam

- [ ] Restore failpoint proof uses replace/write mode.
- [ ] Executor proof has valid config, credentials, and pending state.
- [ ] Worker proof has auto-sync enabled.
- [ ] Event proof traverses a valid worker/executor cycle.
- [ ] Barrier proof uses the exact production point.
- [ ] Cleanup and permission seam absence is proven.
- [ ] Bash and PowerShell proofs pass.

### Telemetry

- [ ] Observer is connected to actual handlers.
- [ ] Records are bounded.
- [ ] Records contain no secrets or raw payloads.
- [ ] Exact request count is asserted.
- [ ] Exact identity is asserted.
- [ ] Exact target library is asserted.
- [ ] Revision transition is asserted.
- [ ] Payload length/hash are asserted.
- [ ] Plaintext sentinel absence is asserted.
- [ ] Maximum concurrency is asserted.
- [ ] Pending clear ordering is asserted.
- [ ] Quiet-period duplicate absence is asserted.

### Repair

- [ ] Repair items identify exact transactions.
- [ ] `CleaningUp` resumes cleanup.
- [ ] `CommittedLocal` finalizes pending and cleanup.
- [ ] Interrupted uncommitted work rolls back only its target.
- [ ] Legacy terminal artifacts clean without rollback.
- [ ] Orphan deletion revalidates containment and symlinks.
- [ ] Partial failure is retained in the report.
- [ ] Partial failure exits nonzero.
- [ ] Reapply is idempotent.

### CI and release evidence

- [ ] Static gates pass.
- [ ] Linux dev/release gates pass.
- [ ] macOS dev/release gates pass.
- [ ] Windows dev/release gates pass.
- [ ] Cleanup/permission suites pass on required platforms.
- [ ] Manifest suites pass on all three.
- [ ] Telemetry suites pass on all three.
- [ ] Repair suites pass on all three.
- [ ] Production seam passes on Linux and Windows.
- [ ] Package smoke passes on all three.
- [ ] All evidence is from one commit.
- [ ] Exact workflow/job URLs are recorded.

---

## 9. Stop conditions

Keep Phase 11 open if any of the following remains true:

- commit or rollback persists a terminal state before cleanup ownership;
- a terminal journal with artifacts is ignored;
- cleanup coordinate documentation disagrees with code;
- a new index, usage, library, or sync file can become `0644` through fallback;
- permission failures are logged or discarded instead of returned;
- a semantic manifest test can fail first on size/checksum noise;
- a manifest rejection test omits side-effect assertions;
- production restore seam proof still uses dry-run;
- production worker proof disables auto-sync;
- executor proof lacks a valid pending generation;
- barrier proof uses a nonmatching point;
- telemetry requires manual `record_request` calls;
- headline E2E discards the real capture handle;
- repair rolls back `CleaningUp` or `CommittedLocal` state;
- repair applies one item to every journal;
- partial repair failure exits zero;
- cleanup or permission crash tests are missing;
- same-commit CI evidence is pending;
- closure documentation claims more than the evidence proves.

Do not resolve a stop condition by weakening an assertion, accepting multiple outcomes, adding a skip, or relabeling an implementation-only build as evidence.

---

## 10. Handoff instructions

1. Confirm `main` is based on `5f430b0a5fca2b1fce486b50445337826358a3f6` or document intervening commits.
2. Reopen the closure status in the first commit.
3. Correct cleanup state ownership before adding cleanup tests.
4. Add compatibility handling for existing terminal journals with artifacts.
5. Unify cleanup APIs before modifying repair.
6. Add cleanup crash tests before marking cleanup complete.
7. Apply destination policy to all four entry kinds.
8. Remove the implicit metadata fallback.
9. Add exact mode and failure-injection tests.
10. Build one valid manifest fixture framework and convert all negatives.
11. Correct both production-seam scripts using real traversed paths.
12. Connect telemetry at the actual server handler boundary.
13. Rewrite the headline E2E to use automatic exact telemetry.
14. Make repair actions transaction-specific and state-aware.
15. Map repair partial failure to a nonzero process exit.
16. Run focused suites after each workstream.
17. Run the complete Linux/macOS/Windows matrix on one candidate commit.
18. Record exact workflow evidence only after every required job passes.
19. If any criterion remains unsupported, leave Phase 11 open and document the exact gap.

The intended result is a narrow final corrective pass. It must not expand the architecture. The repository is closed only when cleanup authority is crash-safe, every restored state file follows the explicit private policy, tests prove the exact named boundaries, repair performs the correct state-specific action, real server telemetry proves acknowledgement, and one final commit passes the complete cross-platform evidence matrix.

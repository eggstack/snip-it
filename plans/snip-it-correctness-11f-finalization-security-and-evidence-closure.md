# Phase 11F — Finalization, Security, and Evidence Closure

Status: READY FOR IMPLEMENTATION

Program status: REOPENED

Implementation baseline: `8cd06654c586e74efe288a13de9cdae3602bdf77`

Supersedes as the authoritative remaining-work handoff:

- `plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md`

Related status file:

- `plans/snip-it-correctness-11-closure-status.md`

## 1. Purpose

Phase 11E materially improved the repository, but its completion status is broader than the current evidence supports. This phase closes the remaining defects without changing the product architecture.

The remaining work is concentrated in six correctness boundaries:

1. a child exit code of zero is still recorded as remote sync success even when pending remains unchanged;
2. transaction cleanup is not restartable or uniformly routed through the canonical cleanup path;
3. transaction artifact and restored-destination permissions are not fail-closed;
4. manifest semantic validation remains incomplete and several negative fixtures are still multi-fault;
5. concurrency and recording-server tests do not yet prove the exact claims documented by the closure status;
6. production-seam and cross-platform CI evidence has not been demonstrated on one final commit.

This plan must remain narrow. Do not introduce a daemon, resident helper, second executable, database-backed transaction manager, plugin runtime, workflow engine, or distributed coordination layer.

## 2. Architectural constraints

The implementation must preserve all of the following:

- one installed `snp` binary;
- one-shot detached worker and executor subprocesses only;
- TOML remains the authoritative local storage representation;
- local mutation is durable before asynchronous scheduling;
- pending intent is cleared only after remote acknowledgement;
- only the exact observed generation may be cleared;
- failures preserve pending state;
- at most one sync execution occurs at a time;
- no secrets or snippet payloads in process arguments, lock records, status records, lifecycle events, or CI logs;
- read-only commands do not mutate or recover state;
- transaction recovery remains local and lightweight.

## 3. Current defects to close

### 3.1 False-success status recording

Current behavior:

- the test-only executor `noop-success` mode exits zero without protocol contact and without clearing pending;
- the worker maps any executor exit zero to `status::record_success`;
- the worker event correctly says pending was not cleared, but the durable status file says `last_result = "success"`.

This makes status observability factually incorrect and causes the new false-success regression test to contradict production behavior.

### 3.2 Cleanup is not a restartable state machine

Current behavior:

- normal commit and rollback persist a terminal state before cleanup;
- cleanup failure can leave a terminal journal and transaction artifacts;
- `check_interrupted_transactions` ignores terminal journals;
- orphan scanning ignores an artifact directory when a matching terminal journal exists;
- `CommittedLocal` recovery manually removes only backup files and the journal, ignores errors, and does not remove staged files or the artifact directory.

This can strand plaintext staged content while returning success.

### 3.3 Artifact permissions are best-effort

Current behavior:

- private directories are created and then chmodded;
- staged files are created using ordinary `File::create`, written, and only then chmodded;
- chmod failures are logged and ignored;
- a permissive umask can expose plaintext staged content before chmod;
- security-policy failure does not abort the transaction.

### 3.4 New destination permissions are incorrect

Current behavior:

- a destination that did not previously exist has empty original metadata;
- empty metadata defaults to Unix mode `0644`;
- this default is applied after installation, including for new `sync.toml`;
- metadata verification does nothing when no original mode exists.

New sensitive configuration can therefore be downgraded after initially secure persistence.

### 3.5 Manifest semantic validation is incomplete

Current behavior:

- schema, layout, cardinality, path shape, destination collisions, sizes, and hash syntax are checked;
- the index consistency block explicitly skips parsing index content;
- duplicate index library names, multiple primaries, missing referenced libraries, and unindexed libraries are not proven;
- several negative tests still include placeholder hashes or unrelated malformed content.

### 3.6 Production-seam proof scripts do not traverse the seams

Current behavior:

- the failpoint proof sets a restore failpoint but runs `snp list`;
- the worker-suppression proof sets `SNP_SKIP_WORKER_SPAWN` but runs `snp list`;
- the mutation-barrier proof sets a barrier but runs `snp list`;
- the executor proof omits the required `--generation` argument, so argument parsing—not compile-time isolation—causes the expected nonzero exit.

### 3.7 Barrier tests do not assert blocking

The tests overlap processes, but they do not assert that the backup process remains alive and blocked before releasing the writer barrier.

### 3.8 Telemetry is database evidence, not exact request evidence

The current telemetry test proves a server-side row exists and an authorization header was seen. It does not prove exact request count, route, expected identity, revision transition, payload properties, or maximum request concurrency.

### 3.9 Same-commit CI evidence is absent

The workflow is broader, but closure requires successful Linux, macOS, and Windows jobs on one final commit, with retrievable run and job URLs.

## 4. Workstream A — Make sync status factually authoritative

### 4.1 Required outcome model

Do not treat executor exit code zero alone as remote success.

After an executor exits zero, the worker must classify the local pending state for the observed generation.

Introduce an internal result such as:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutorCompletion {
    AcknowledgedAndCleared,
    AcknowledgedCoveredByNewer { current_generation: u64 },
    ExitZeroWithoutAcknowledgement,
    PendingStateUnreadable,
}
```

The exact type name may differ, but the semantic distinctions are mandatory.

### 4.2 Classification rules

For observed generation `G`, after executor exit zero:

1. pending marker missing:
   - classify as `AcknowledgedAndCleared`;
   - record durable success for `G`.

2. pending marker exists with generation greater than `G`:
   - classify as `AcknowledgedCoveredByNewer`;
   - record success for completed generation `G` while preserving the newer marker;
   - immediately continue or schedule the bounded follow-up cycle.

3. pending marker still exists with generation exactly `G`:
   - classify as `ExitZeroWithoutAcknowledgement`;
   - do not record success;
   - record an internal/protocol-integrity failure;
   - preserve pending `G`;
   - set attention required or an equivalent factual diagnostic.

4. pending marker exists with generation lower than `G`:
   - treat as corrupt/inconsistent state;
   - do not record success;
   - preserve evidence and return failure.

5. pending marker cannot be read:
   - do not record success;
   - record local persistence/internal failure;
   - preserve recoverability.

### 4.3 Status ownership

The executor remains the only component allowed to clear pending.

The worker remains responsible for execution lifecycle and durable attempt status, but it may record success only after observing evidence compatible with acknowledgement.

Do not move pending clearing back into the worker.

### 4.4 Required tests

Add or correct tests for all of the following:

- real remote acknowledgement clears `G` and records success;
- real remote acknowledgement with a concurrent newer generation preserves the newer marker and records completed success for `G`;
- `noop-success` exits zero, leaves `G`, records no success, and produces zero server requests;
- pending read failure after exit zero records failure and does not clear pending;
- pending clear failure in the executor returns nonzero and leaves factual status;
- unknown executor exit code never records success;
- signal termination never records success.

### 4.5 Acceptance criteria

- no code path calls `record_success` based only on `ExecutorExitCode::Success`;
- `test_false_success_executor_leaves_pending_intact` passes and asserts an exact non-success status code;
- status, events, and pending state describe the same outcome;
- a newer pending generation is never cleared by completion of an older generation.

## 5. Workstream B — Make cleanup a restartable protocol

### 5.1 Required state model

Terminal state must not be persisted before cleanup is complete.

Use an explicit cleanup state. A suggested model is:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionOutcome {
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupStep {
    RemoveStaged,
    RemoveBackups,
    RemoveArtifactDirectory,
    RemoveJournal,
}

TransactionState::CleaningUp {
    outcome: TransactionOutcome,
    next_step: CleanupStep,
}
```

Equivalent representations are acceptable if progress is unambiguous and restartable.

### 5.2 Required protocol

Commit path:

1. all live destinations are installed and verified;
2. pending finalization is complete where required;
3. persist `CleaningUp { outcome: Commit, next_step: RemoveStaged }`;
4. remove staged artifacts idempotently;
5. persist progress;
6. remove backups idempotently;
7. persist progress;
8. remove artifact directory idempotently;
9. fsync the parent directory where supported;
10. remove journal last.

Rollback path:

1. all original destinations are restored and verified;
2. persist `CleaningUp { outcome: Rollback, next_step: RemoveStaged }`;
3. execute the same canonical cleanup protocol;
4. remove journal last.

Do not require a persisted `Committed` or `RolledBack` state before journal removal. If legacy terminal variants remain for compatibility, the recovery scanner must treat terminal journals with existing artifacts as cleanup-pending evidence.

### 5.3 Canonical cleanup function

Provide one public/internal cleanup entry point used by:

- normal commit;
- normal rollback;
- `CommittedLocal` recovery;
- cleanup recovery after a crash;
- `repair --apply` for a complete transaction;
- legacy terminal journal cleanup.

Remove manual backup/journal deletion from `gate_mutation_on_interrupted_transactions`.

Every cleanup error must propagate. Never use `let _ = remove_*` in an authoritative transaction finalization path.

### 5.4 Recovery scanner behavior

`check_interrupted_transactions` or a replacement typed scanner must identify:

- ordinary interrupted transactions;
- cleanup-pending transactions;
- legacy terminal journals with remaining artifacts;
- journals whose artifact directory is missing;
- artifact directories with no journal;
- malformed journals.

Required behavior:

- complete and unambiguous cleanup-pending state: retry cleanup automatically;
- missing required rollback material: fail closed and direct to `snp repair`;
- malformed journal: report explicitly; do not silently skip;
- orphan artifact directory: report and remove only through validated containment.

### 5.5 Cleanup crash tests

Add test-only failpoints at exact cleanup boundaries:

- after staged files removed and progress persisted;
- after backups removed and progress persisted;
- after artifact directory removed and before journal removal;
- immediately before journal removal.

For commit and rollback outcomes, each test must:

1. launch the real feature-enabled binary;
2. abort at the boundary;
3. inspect journal state and remaining artifacts;
4. run recovery through the real binary;
5. assert correct final live bytes;
6. assert exact pending behavior;
7. assert no staged files, backups, artifact directory, or journal remain;
8. run recovery a second time and prove idempotence.

### 5.6 Acceptance criteria

- no successful recovery can leave a staged file or artifact directory;
- cleanup failure never produces an ignored terminal journal;
- journal removal is always the final destructive transaction step;
- all cleanup paths use the same containment validation;
- a second crash during cleanup is recoverable;
- repair reports both orphan artifacts and terminal-journal-plus-artifact states.

## 6. Workstream C — Enforce private artifact creation fail-closed

### 6.1 Unix directory creation

Use `std::os::unix::fs::DirBuilderExt::mode(0o700)` when creating transaction artifact directories.

After creation:

- verify the resulting mode is private enough;
- if the mode cannot be established, return an error before writing sensitive content;
- do not merely log and continue.

### 6.2 Unix file creation

Use `OpenOptionsExt::mode(0o600)` at file creation time.

The write sequence must be:

1. create/open the new artifact with private mode;
2. verify it is a regular file and not a symlink;
3. write all bytes;
4. `sync_all`;
5. reopen and hash;
6. verify mode;
7. sync parent directory.

Do not write sensitive content before private mode is established.

### 6.3 Windows behavior

Document the supported Windows privacy contract precisely.

At minimum:

- artifacts must be created beneath the user-owned configuration directory;
- no inherited broad sharing behavior may be deliberately enabled;
- readonly and ordinary file attributes must not weaken cleanup/recovery;
- tests should verify no world-readable Unix claim is made on Windows.

Do not claim Windows ACL hardening unless it is actually implemented and tested.

### 6.4 Permission failure injection

Add a test seam or filesystem fixture that causes permission application/verification to fail before sensitive bytes are written.

The test must prove:

- restore returns nonzero;
- no live destination was changed;
- no insecure staged content remains;
- journal/evidence remains only when safe recovery is possible;
- no pending generation is created.

### 6.5 Unix permission tests

On Unix, assert exact modes for:

- `.transaction/` where created by the product;
- `artifacts/`;
- `artifacts/<txn-id>/`;
- `backups/`;
- `staged/`;
- every `.bak` and `.new` file.

Expected directories: `0700`.

Expected artifact files: `0600`.

### 6.6 Acceptance criteria

- no sensitive transaction bytes are written before private creation succeeds;
- chmod or permission verification failure is fatal;
- security docs describe only proven platform behavior;
- Unix permission tests run in CI.

## 7. Workstream D — Define destination permission policy

### 7.1 Separate existing-file preservation from new-file defaults

Introduce an explicit destination class, for example:

```rust
pub enum DestinationClass {
    Library,
    LibraryIndex,
    UsageIndex,
    SyncConfig,
}
```

Metadata application must receive both:

- whether the destination existed before the transaction;
- the destination class.

### 7.2 Required policy

For an existing destination:

- preserve the sanitized original mode;
- strip setuid, setgid, and sticky bits;
- preserve readonly semantics where supported;
- verify the final mode.

For a new destination:

- do not use `0644` as a generic fallback;
- use an explicit product policy;
- `sync.toml` must be private;
- library files contain commands and should also default to private user data;
- index and usage files should follow the same local-data privacy policy unless an existing documented contract requires otherwise.

Recommended Unix default for all new local state files: `0600`.

If a different mode is selected for a class, document the rationale and test it.

### 7.3 Remove ambiguous metadata defaults

`OriginalFileMetadata::default()` must not silently imply a destination mode.

Represent absence explicitly and require the caller to supply the destination class/default policy.

### 7.4 Required tests

On Unix:

- new `sync.toml` restored with exact private mode;
- new library file restored with exact private mode;
- new index and usage files have documented exact modes;
- existing `0640` file remains `0640` after restore;
- existing readonly file remains readonly;
- setuid/setgid/sticky bits are stripped;
- rollback restores the original sanitized mode;
- metadata mismatch causes transaction failure and rollback.

On Windows:

- readonly preservation is tested where supported;
- no Unix mode assertions are made.

### 7.5 Acceptance criteria

- no new restored file falls back implicitly to `0644`;
- sensitive config cannot be downgraded after `SensitiveConfig` persistence;
- verification covers new and existing destinations.

## 8. Workstream E — Complete manifest semantic validation

### 8.1 Split structural and semantic validation

Use two explicit phases:

1. structural validation without opening artifacts;
2. semantic validation after safe source-file checks but before any lock, transaction, artifact creation, or live write.

Suggested signatures:

```rust
fn validate_manifest_structure(manifest: &BackupManifest) -> SnipResult<ValidatedManifest>;

fn validate_manifest_semantics(
    backup_root: &Path,
    manifest: &ValidatedManifest,
    mode: RestoreMode,
) -> SnipResult<RestorePlan>;
```

The exact API may differ, but the phase ordering must be testable.

### 8.2 Index semantic rules

When an index entry is present:

- parse it as the actual product index type;
- reject duplicate library filenames;
- reject more than one primary library;
- reject an index reference without a matching library artifact;
- reject duplicate normalized/case-folded names;
- reject path aliases that map to the same destination;
- define and enforce whether every library artifact must be referenced by the index.

For replace mode, recommended rule:

- the index and library artifact set must agree exactly.

For merge mode:

- incoming index references must resolve to incoming library artifacts;
- unreferenced incoming library artifacts must be rejected unless an explicit documented merge rule permits them;
- existing local-only libraries may remain.

### 8.3 Library semantic rules

Before transaction creation:

- parse every incoming library;
- reject duplicate snippet IDs within each library;
- reject malformed required fields;
- reject duplicate library identities where the product schema provides an identity separate from filename;
- enforce size limits before unbounded allocation;
- verify actual size and hash.

### 8.4 Exact diagnostics

Each semantic error should include:

- stable category phrase;
- offending manifest/index/library path;
- normalized identity where relevant;
- no snippet command, API key, or full payload.

### 8.5 Replace multi-fault fixtures

Every negative manifest test must use:

- valid schema unless schema is the targeted fault;
- valid layout unless layout is the targeted fault;
- real artifact files;
- exact actual sizes;
- exact SHA-256 values;
- valid unrelated entries;
- exactly one targeted defect.

Remove all `sha256 = "placeholder"` fixtures except tests that fail during deserialization before hash validation, and even there prefer a fully valid surrounding fixture.

### 8.6 Required negative tests

At minimum:

- schema zero;
- future schema;
- unsupported layout;
- unknown kind deserialization;
- duplicate exact destination;
- case-folded destination collision;
- trailing-dot/space alias;
- drive-relative path;
- UNC path;
- reserved Windows name;
- duplicate index entry;
- duplicate usage entry;
- duplicate sync-config entry;
- duplicate library name in index;
- multiple primary libraries;
- index references missing library artifact;
- library artifact absent from authoritative replace index;
- duplicate snippet IDs;
- manifest size mismatch;
- checksum mismatch;
- symlink source;
- oversized source.

Each test must additionally assert:

- no transaction journal;
- no transaction artifact directory;
- no pending marker;
- no live destination mutation.

### 8.7 Acceptance criteria

- the index validation block no longer contains a skip/stub comment;
- semantic validation completes before lock acquisition;
- every negative fixture is single-fault;
- diagnostics identify the targeted contract;
- all tests reject consistently on Linux, macOS, and Windows.

## 9. Workstream F — Repair production-seam proofs

The production-seam scripts must use the actual production binary built without `test-support` and must traverse the code path guarded by each environment variable.

### 9.1 General script requirements

For both Bash and PowerShell:

- build once into an isolated target directory;
- use an isolated config/state directory;
- create valid fixtures;
- set the exact matching test-control value;
- invoke a command that reaches the guarded seam;
- assert the command outcome and side effects that distinguish production behavior from test behavior;
- assert no argument-parsing error was mistaken for proof;
- use bounded polling and explicit timeouts;
- clean up child processes.

### 9.2 Failpoint proof

Create a valid backup and invoke real restore with:

```text
SNP_TEST_FAILPOINT=restore-after-prepared
```

Production expectation:

- process does not abort at that boundary;
- restore follows normal production behavior;
- no abort/signal exit is observed;
- final output is consistent with a valid restore.

### 9.3 Executor no-op proof

Invoke:

```text
snp auto-sync-execute --state-dir <state> --generation 1
```

with a valid enabled configuration pointing to an intentionally unreachable server and:

```text
SNP_TEST_EXECUTOR_MODE=noop-success
```

Production expectation:

- command reaches executor logic;
- stderr does not contain missing-argument/usage diagnostics;
- command does not exit zero through the no-op seam;
- it attempts normal protocol behavior and returns the expected network/error classification;
- pending is not spuriously cleared.

### 9.4 Worker-suppression proof

Configure auto-sync with zero debounce and a bounded timeout. Set:

```text
SNP_SKIP_WORKER_SPAWN=1
```

Perform a real mutation.

Production expectation:

- scheduling is not suppressed;
- a worker attempt is observable through production artifacts such as execution status or lock lifecycle;
- pending remains if the intentionally unreachable server causes failure;
- no test event file is required for this proof.

### 9.5 Event-sink proof

Set `SNP_TEST_EVENTS_DIR` and run a real worker/executor path.

Production expectation:

- no `test-events.jsonl` is created;
- production status/pending behavior remains normal.

### 9.6 Mutation-barrier proof

Set up a real barrier directory for a barrier point reached by a valid mutation, such as library creation.

Do not create a release file.

Production expectation:

- command completes within a short timeout;
- no `entered` file appears;
- mutation result is correct.

### 9.7 Acceptance criteria

- every script scenario traverses its guarded branch location;
- executor scripts include `--generation`;
- no `snp list` command is used as proof for restore, scheduling, or mutation barriers;
- Bash and PowerShell provide equivalent evidence;
- scripts pass on Linux and Windows CI using a production build without `test-support`.

## 10. Workstream G — Prove LocalDataLock blocking, not just overlap

### 10.1 Required assertion before barrier release

For every barrier-controlled writer/backup test:

1. wait until the writer reports `entered`;
2. launch backup;
3. poll backup with `try_wait` for a bounded observation interval;
4. assert backup has not completed while the writer holds the lock;
5. optionally assert the expected lock file/owner metadata exists;
6. release the writer;
7. assert both processes finish successfully or with an explicitly allowed busy result;
8. validate the resulting backup snapshot.

Example:

```rust
std::thread::sleep(Duration::from_millis(250));
assert!(
    backup.try_wait()?.is_none(),
    "backup completed while writer still held LocalDataLock"
);
```

### 10.2 Required writer coverage

Retain or add real-process coverage for:

- library create;
- library delete;
- snippet save;
- sync configuration update;
- restore after first destination installation.

Use production command paths. Do not directly modify the coordinated files during the overlap portion of a test.

### 10.3 Coherence assertions

After completion, verify more than manifest presence:

- every manifest entry exists;
- actual size equals manifest size;
- actual hash equals manifest hash;
- index references correspond to copied library files;
- every copied library parses;
- no temporary or partially written files are included;
- result represents a complete before-state or complete after-state.

### 10.4 Acceptance criteria

- tests fail if backup ignores `LocalDataLock` and finishes during the barrier;
- tests do not rely on a fixed sleep as the sole blocking proof;
- all five writer classes run on supported CI platforms.

## 11. Workstream H — Add exact sanitized recording-server telemetry

### 11.1 Recording model

Enhance the test recording server with a bounded sanitized record such as:

```rust
struct RecordedRequest {
    sequence: u64,
    operation: String,
    route: String,
    authenticated_user_id: String,
    device_id: String,
    library_ids: Vec<String>,
    revision_before: i64,
    revision_after: i64,
    payload_len: usize,
    payload_sha256: String,
    plaintext_sentinel_present: bool,
    started_at_unix_ms: u64,
    completed_at_unix_ms: u64,
}

struct RecordingSummary {
    requests: Vec<RecordedRequest>,
    current_in_flight: usize,
    max_in_flight: usize,
}
```

Do not store:

- raw bearer token;
- API key;
- plaintext snippet command;
- full encrypted payload.

A boolean or hash-based assertion is sufficient for plaintext absence.

### 11.2 Exact headline assertions

For one local mutation, assert:

- exactly one canonical sync request or operation was accepted;
- operation/route is the expected one;
- authenticated user matches the registered test user;
- device ID equals the client’s configured/registered device ID, not merely nonempty;
- target library ID equals the server-created library ID;
- revision transitions from exact `R0` to exact `R1`;
- payload length is nonzero and within the configured limit;
- payload hash is present;
- plaintext sentinel is absent from wire payload;
- maximum in-flight canonical sync requests is exactly one;
- after a bounded quiet period, request count remains exactly one;
- pending clear occurs only after the recorded response/acknowledgement completion time.

### 11.3 Lifecycle ordering

Record or expose timestamps/sequences sufficient to prove:

1. local mutation committed;
2. pending `G` existed;
3. worker observed `G`;
4. executor request began;
5. server acknowledged revision `R1`;
6. executor cleared `G`;
7. worker recorded successful completion.

Avoid timing-only assertions where a sequence/event relationship can be used.

### 11.4 False-success telemetry

In `noop-success` mode, assert:

- request count exactly zero;
- maximum in-flight exactly zero;
- pending remains exactly `G`;
- durable status is non-success;
- no success lifecycle event is emitted.

### 11.5 Acceptance criteria

- row count is no longer used as a substitute for request count;
- the primary headline test retains the recording handle;
- all telemetry is bounded and sanitized;
- exactly-once and max-concurrency claims are proven directly.

## 12. Workstream I — Repair and scanner correctness

### 12.1 Typed repair candidates

Do not encode authoritative repair behavior by parsing human-readable `problem` strings.

Use a typed repair action, for example:

```rust
pub enum RepairAction {
    RollbackTransaction { id: String },
    ResumeCleanup { id: String },
    RemoveOrphanArtifactDir { path: PathBuf },
    QuarantineMalformedJournal { path: PathBuf },
    // existing data repairs...
}
```

Human text must be derived from the typed action, not used as its identity.

### 12.2 Safe orphan deletion

Before deleting an orphan artifact directory:

- verify it is a direct child of the canonical artifact root;
- reject symlinks;
- reject traversal/alias paths;
- verify no matching journal appeared after scanning;
- acquire the relevant transaction/local-data coordination as required;
- remove with bounded Windows retries;
- report failures nonzero when `--apply` was requested.

### 12.3 Repair exit status

If any requested safe repair fails:

- command must return nonzero;
- report applied, failed, and skipped counts accurately;
- preserve remaining evidence;
- never print overall success when a repair failed.

### 12.4 Acceptance criteria

- no repair behavior depends on string prefixes;
- cleanup-pending transactions are resumed, not rolled back;
- malformed journals are not silently skipped;
- `repair --apply` returns nonzero on partial failure.

## 13. Workstream J — Test isolation and deterministic binary selection

### 13.1 Feature-enabled integration binary

Confirm every integration suite requiring test seams runs the feature-enabled Cargo binary associated with that test invocation.

Do not cache a production binary path for a seam-dependent test.

### 13.2 Environment isolation

All helper constructors must remove every test-control variable by default:

- `SNP_TEST_FAILPOINT`;
- `SNP_TEST_INJECT_ERROR`;
- `SNP_TEST_EXECUTOR_MODE`;
- `SNP_SKIP_WORKER_SPAWN`;
- `SNP_TEST_EVENTS_DIR`;
- `SNP_TEST_MUTATION_BARRIER_DIR`;
- any new cleanup or permission failure seam.

A test must opt in explicitly per child command.

Avoid process-global environment mutation when a child-command environment is sufficient. If a process-global value is unavoidable, serialize the affected test or use a scoped guard plus a global mutex.

### 13.3 No permissive assertions

Remove patterns such as:

- success or failure both accepted;
- nonempty value accepted when an exact expected value is available;
- row count used as a concurrency proxy;
- sleep followed by assumption without process-state observation;
- nonmatching test-control values used to prove production isolation.

### 13.4 Acceptance criteria

- seam-dependent tests fail when run without `test-support` rather than silently exercising production no-ops;
- production-seam tests use a separately built production binary;
- full workspace execution does not leak test-control state between tests.

## 14. Workstream K — CI and release evidence

### 14.1 Required final matrix

The same final commit must pass:

#### Static checks

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`

#### Workspace tests

On Linux, macOS, and Windows:

- debug workspace library tests;
- release workspace library tests;
- debug integration tests;
- release integration tests, or an explicitly equivalent documented matrix;
- no global worker suppression.

#### Focused release-blocking tests

On Linux, macOS, and Windows:

- deterministic E2E;
- false-success executor;
- auto-sync closure;
- lifecycle and concurrency;
- transaction crash recovery;
- restore crash failpoints;
- cleanup crash failpoints;
- manifest contracts;
- LocalDataLock barriers;
- repair/orphan cleanup;
- permission policy tests where platform-applicable.

#### Production seam

- Linux;
- Windows;
- actual production binary without `test-support`;
- repaired valid seam scenarios.

#### Packaging

On Linux, macOS, and Windows:

- `cargo package --locked`;
- install unpacked package;
- `snp --version`;
- `snp --help`;
- no missing runtime assets.

### 14.2 Workflow quality requirements

- centralize protoc installation;
- no `|| true`, `continue-on-error`, or permissive PowerShell error handling for release-blocking jobs;
- explicit step timeouts;
- process cleanup on timeout;
- no shell syntax accidentally evaluated by the wrong shell;
- no test-only environment variables set globally;
- upload bounded sanitized diagnostics on failure;
- artifact/log retention must not include credentials or snippet plaintext.

### 14.3 Same-commit evidence record

Only after all required jobs pass, update `plans/snip-it-correctness-11-closure-status.md` with:

- exact final commit SHA;
- workflow run URL;
- job URLs for Linux, macOS, Windows, production seam, and package jobs;
- exact commands represented by each job;
- final pass/fail result;
- any platform-specific exclusions and why they are legitimate;
- confirmation that all jobs used the same commit.

Do not record guessed test counts or pending URLs.

### 14.4 Acceptance criteria

- all release-blocking jobs pass on one commit;
- evidence is retrievable;
- no status claim exceeds the actual jobs and assertions;
- the correctness program remains reopened until this evidence is committed.

## 15. Workstream L — Documentation and status truthfulness

Update documentation only after behavior is correct.

Required files may include:

- `architecture/auto_sync.md`;
- `docs/THREAT_MODEL.md`;
- `docs/SECURITY_AUDIT.md`;
- `AGENTS.md`;
- `plans/snip-it-correctness-11-closure-status.md`.

Document:

- exit zero is not sufficient evidence of remote acknowledgement;
- executor owns pending clear;
- worker verifies pending disposition before recording success;
- cleanup is restartable and journal-last;
- artifact privacy is fail-closed on Unix;
- exact supported Windows permission claims;
- new destination mode policy;
- manifest structural versus semantic validation;
- exact telemetry recorded and intentionally omitted;
- CI evidence requirements.

Remove or correct any statement that currently claims:

- all Phase 11E workstreams are complete;
- cleanup is complete despite the `CommittedLocal` manual deletion path;
- all manifest tests are single-fault;
- exact request/concurrency evidence exists when only database counts are asserted;
- only CI evidence remains.

## 16. Recommended implementation sequence

Use small, reviewable commits. A recommended sequence is:

1. `status: reopen Phase 11F and remove Phase 11E overclaims`
2. `sync: classify executor zero exit using pending disposition`
3. `sync: make false-success durable status non-success`
4. `test: cover acknowledged clear, newer generation, and false-success status`
5. `transaction: add cleanup-pending state and typed progress`
6. `transaction: route commit rollback and recovery through canonical cleanup`
7. `repair: recognize cleanup-pending and legacy terminal artifacts`
8. `test: add commit cleanup crash failpoints`
9. `test: add rollback cleanup crash failpoints`
10. `security: create artifact directories with private mode at creation`
11. `security: create artifact files with private mode at creation`
12. `security: make artifact permission failure fatal`
13. `restore: add explicit destination security classes`
14. `restore: apply and verify new-file permission defaults`
15. `manifest: split structural and semantic validation phases`
16. `manifest: enforce index/library consistency`
17. `test: replace all multi-fault manifest fixtures`
18. `ci: repair Bash production-seam proof`
19. `ci: repair PowerShell production-seam proof`
20. `test: assert backup process blocks before barrier release`
21. `test-server: add sanitized exact request telemetry`
22. `test: assert exact identity revision payload and concurrency evidence`
23. `repair: use typed repair actions and fail on partial apply`
24. `ci: run full same-commit matrix`
25. `docs: record final evidence and close Phase 11`

The implementer may combine adjacent commits, but each commit should leave the repository compiling and should not mix unrelated architecture changes.

## 17. Required verification commands

Run locally where platform-applicable:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --lib -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test auto_sync_lifecycle --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
bash scripts/ci/test-production-seams.sh
cargo package -p snip-it --locked --allow-dirty
```

Add focused suites for cleanup, permissions, and repair if new test files are introduced.

The PowerShell production-seam script must be run on Windows CI and should also be manually validated in a Windows environment before closure.

## 18. Release-blocking closure checklist

Phase 11F is complete only when every item below is true.

### Sync truthfulness

- [ ] worker never records success from exit zero alone;
- [ ] exact pending disposition determines success/failure status;
- [ ] false-success mode leaves pending and records non-success;
- [ ] newer generation is preserved and handled;
- [ ] status and lifecycle events agree.

### Cleanup

- [ ] explicit cleanup-pending state exists;
- [ ] commit, rollback, recovery, and repair use canonical cleanup;
- [ ] cleanup progress is persisted;
- [ ] staged files are removed;
- [ ] backups are removed;
- [ ] artifact directory is removed;
- [ ] journal is removed last;
- [ ] cleanup errors propagate;
- [ ] second crash during cleanup is recoverable;
- [ ] legacy terminal journals with artifacts are handled.

### Permissions

- [ ] Unix artifact directories are created as `0700`;
- [ ] Unix artifact files are created as `0600`;
- [ ] permission failure is fatal before sensitive write;
- [ ] new destination mode policy is explicit;
- [ ] new `sync.toml` remains private;
- [ ] metadata is verified for new and existing files;
- [ ] Windows claims are accurate and tested.

### Manifest

- [ ] index content is parsed before transaction creation;
- [ ] duplicate library names are rejected;
- [ ] multiple primaries are rejected;
- [ ] missing index/library relationships are rejected;
- [ ] duplicate snippet IDs are rejected;
- [ ] every negative fixture is single-fault;
- [ ] no invalid manifest creates journals, pending, artifacts, or live writes.

### Production seam

- [ ] failpoint proof runs real restore;
- [ ] executor proof includes `--generation` and reaches executor logic;
- [ ] worker suppression proof runs a real mutation;
- [ ] event proof runs worker/executor logic;
- [ ] barrier proof reaches a real mutation barrier;
- [ ] Bash and PowerShell pass with production builds.

### Concurrency and telemetry

- [ ] backup is asserted blocked before writer release;
- [ ] coherent snapshot hashes and index relationships are verified;
- [ ] recording server captures exact bounded request records;
- [ ] request count is exactly one;
- [ ] max in-flight is exactly one;
- [ ] expected user, device, and library IDs match exactly;
- [ ] exact revision transition is asserted;
- [ ] wire payload does not contain plaintext sentinel;
- [ ] quiet period produces no duplicate request;
- [ ] pending clear follows acknowledgement.

### Repair and CI

- [ ] repair actions are typed;
- [ ] cleanup-pending is resumed rather than rolled back;
- [ ] partial repair failure returns nonzero;
- [ ] Linux matrix passes;
- [ ] macOS matrix passes;
- [ ] Windows matrix passes;
- [ ] production seam passes on Linux and Windows;
- [ ] packaging passes on all three platforms;
- [ ] all evidence is from the same final commit;
- [ ] workflow and job URLs are recorded;
- [ ] closure status contains no unsupported claims.

## 19. Stop conditions

Stop implementation and keep the program reopened if any of the following occurs:

- a cleanup path can return success while transaction artifacts remain;
- a terminal journal can hide retryable cleanup;
- artifact privacy is best-effort rather than fail-closed;
- new `sync.toml` can become broadly readable;
- executor exit zero can still produce success while generation `G` remains unchanged;
- an invalid manifest can reach lock acquisition or transaction creation;
- a production-seam proof relies on argument parsing or an unrelated command;
- a concurrency test does not observe the backup blocked before release;
- exact request count is inferred from database row count;
- CI results come from different commits;
- Windows release-blocking tests are skipped without an explicit product-supported reason;
- closure documentation claims more than the code and tests demonstrate.

## 20. Final release rule

Do not mark Phase 11 complete merely because all code workstreams have commits.

The release decision is binary:

- `COMPLETE` only when all closure checklist items pass on one evidenced final commit;
- otherwise `INCOMPLETE / REOPENED`.

No remaining defect in this plan is documentation-only. The status file must continue to classify the repository as not correctness-closed and not release-ready until the final CI evidence commit is present.
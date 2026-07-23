# Phase 11B — Durability Integration, Verification Truthfulness, and Windows CI Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `cd206fc2ee65f3a9a9307074a3eb93b82baeffb3`

Parent plan: `plans/snip-it-correctness-11-verification-and-crash-closure.md`

Current status document: `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan is the authoritative corrective handoff for the unresolved Phase 11 defects found by direct review after `cd206fc2`. It does not reopen already-proven auto-sync core work or broaden product scope. It closes the remaining durability, verification, credential-boundary, execution-outcome, and Windows CI gaps.

The correctness program must not be marked complete until every release-blocking criterion in this document is supported by production code, adversarial tests, and successful Linux, macOS, and Windows CI jobs.

---

## 1. Objective

Deliver a bounded corrective pass that proves all of the following:

1. restore uses the durable transaction state machine in production rather than merely defining it;
2. a process crash at any live replacement point is recoverable to a coherent state;
3. rollback is byte-exact, restartable, and removes newly created destinations;
4. interrupted restore journals are detected from the same directory in which restore writes them;
5. lock ownership survives PID reuse and stale-lock recovery cannot race or silently destroy evidence;
6. backup captures one coherent local state while normal snippet and library mutations are possible;
7. manifest schema, kinds, paths, collisions, and snippet identities fail closed for the intended reason;
8. headline sync and read-only tests prove server and lifecycle effects directly rather than through proxies;
9. every execution path, including output-file snippets, maps timeout and spawn failures to public exit code `8`;
10. test-only credential behavior is impossible to activate in production builds;
11. the Windows CI matrix is portable, deterministic, and green without permissive skips or shell accidents;
12. closure documentation records actual workflow URLs and job conclusions.

This is a corrective closure pass. Do not add unrelated commands, services, plugin systems, workflow features, or architectural layers.

---

## 2. Architectural constraints

Preserve the existing product architecture:

- one installed client binary: `snp`;
- detached auto-sync workers remain one-shot subprocesses;
- no resident client daemon;
- no second installed helper binary;
- no workflow engine;
- no plugin runtime;
- no database replacement for TOML state;
- no remote execution feature;
- no broad CLI redesign;
- no platform-specific behavior that changes public command semantics.

Allowed internal additions:

- a short-lived local-data coordination lock;
- richer transaction journal records;
- process-start identity helpers;
- test-only failpoints and executor modes under `test-support`;
- platform-specific CI scripts or checked-in helper scripts;
- typed manifest and outcome models;
- deterministic test server telemetry.

---

## 3. Baseline findings that remain release-blocking

### 3.1 Transaction states exist but restore does not use them

`TransactionState` defines `BackupsDurable`, `Committing`, and `RollingBack`, but restore still writes a `Prepared` journal, enriches only an in-memory clone with backup paths, performs live writes, and then commits. The state-transition helpers are unused.

A crash during replacement can therefore leave a persisted journal without the backup paths and progress required for safe recovery.

### 3.2 Rollback semantics are incomplete

Current rollback:

- copies backups directly over destinations instead of using the atomic persistence primitive;
- does not remove destinations created by the failed transaction;
- records reverse progress using an index convention that can skip remaining files after a second crash;
- deletes journal evidence after completion without a tested retention policy;
- does not verify original hashes after restoration.

### 3.3 Repair inspects the wrong journal directory

Restore writes under `<state-dir>/.transaction`, while the transaction inspection path used when collecting repair candidates checks `<state-dir>` directly. Tests currently create synthetic journals in the wrong location, so they can pass while real restore journals are invisible.

### 3.4 Lock records do not protect against PID reuse

The transaction lock records PID and nonce, but not process-start identity or transaction ID. Malformed locks are silently removed. Stale reclaim removes the lock and recreates it with ordinary `fs::write`, creating a race between reclaimers.

### 3.5 Backup generation does not cover snippet mutations

`libraries.toml.generation` changes for `LibraryManager` mutations but normal snippet create/edit/delete writes can update a library file without changing that counter. A before/after generation check therefore does not serialize backup against all relevant mutations.

The current concurrency suite mostly performs sequential mutations and does not prove a before-state or after-state snapshot under an actual concurrent write.

### 3.6 Manifest validation is still partly free-form

`BackupEntryKind` exists, but `BackupManifestEntry.kind` remains a string and restore continues to branch on string literals. Some tests use invalid hashes or accept either success or failure, so they do not isolate schema, collision, or duplicate-ID validation.

### 3.7 E2E and read-only evidence is incomplete

The headline test now requires one server-side snippet, but it discards recording telemetry and does not prove canonical request count, expected identity, target library, encrypted payload, or server-side concurrency.

The no-op test uses an unreachable server, which proves normal failure—not an executor that falsely reports success without syncing.

Read-only tests infer worker absence from status content instead of asserting lifecycle-event and server-request absence.

### 3.8 Output-file execution still falls through the generic error path

The ordinary execution path maps timeout and shell-spawn failures to `ExecutionFailed`. The output-file branch still propagates spawn and timeout errors as `SnipError`, which exits with code `1` rather than the documented execution-failure code `8`.

### 3.9 Test credential behavior is available in production builds

`SNP_TEST_CREDENTIAL_FILE` changes serialization, deserialization, and migration behavior without a `test-support` compile-time guard. A normal production binary can therefore bypass keychain behavior and read or write credentials through this test seam.

### 3.10 Windows CI remains unproven and contains a definite shell bug

The current `package_smoke` matrix uses Bash expressions such as `$(...)`, `mktemp`, and Unix-style path handling in a Windows job whose default shell is PowerShell.

Recent corrective commits also show recurring Windows failures involving:

- `HANDLE` versus integer comparisons;
- incorrect `OpenProcess` failure semantics;
- PID `1` being assumed dead when it is the Windows System process;
- Unix-only `libc::kill` and signal tests compiling on Windows;
- `protoc` download naming and PATH propagation;
- FIFO tests being used as nominal cross-platform evidence;
- environment and clipboard assumptions.

The connector does not expose a successful workflow run for the baseline. The implementation agent must inspect the actual GitHub Actions failures before changing code and must preserve the run URLs in closure evidence.

---

# Workstream A — Reopen Phase 11 evidence around this corrective plan

## Goal

Prevent the repository from representing Phase 11 as blocked only on CI when production durability and verification defects remain.

## Required first implementation commit

Update `plans/snip-it-correctness-11-closure-status.md` to include:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking corrective plan: plans/snip-it-correctness-11b-durability-verification-windows-ci-closure.md
Corrective baseline: cd206fc2ee65f3a9a9307074a3eb93b82baeffb3
```

Replace the current “only cross-platform CI evidence remains” statement with the exact open workstreams from this plan.

Do not pre-check closure criteria.

## Acceptance criteria

- the repository cannot appear complete while this plan is open;
- historical Phase 11 evidence is retained but labeled accurately;
- claimed behavior is separated from verified behavior;
- final status references the actual final commit, workflow run URL, and per-job conclusions.

---

# Workstream B — Introduce one canonical transaction directory and recovery API

## Goal

Eliminate state-directory drift and make every transaction producer and consumer use the same path contract.

## Required design

Create a canonical helper owned by the transaction module, for example:

```rust
pub fn transaction_state_dir(base_state_dir: &Path) -> PathBuf {
    base_state_dir.join(".transaction")
}
```

All of the following must use that helper:

- restore transaction creation;
- transaction lock acquisition;
- journal reads and writes;
- interrupted-transaction inspection;
- repair dry-run reporting;
- repair apply/recovery;
- status and doctor reporting;
- tests and failpoint subprocesses.

Do not repeat `.join(".transaction")` at call sites.

## Required tests

- a real restore-created interrupted journal is visible to `snp repair --dry-run --json`;
- synthetic journals created outside the canonical directory are ignored;
- status and doctor report the same transaction count as repair;
- no command accidentally creates a second transaction directory.

## Closure criteria

- one helper defines the transaction directory;
- all production call sites use it;
- tests create journals through production helpers or in the canonical location;
- repair can detect a journal produced by a crashed restore subprocess.

---

# Workstream C — Replace restore’s ad hoc writes with a durable transaction executor

## Goal

Make the transaction model executable, not documentary.

## Required model

Introduce a transaction plan that contains complete rollback and commit information before any live destination changes.

Recommended shape:

```rust
pub struct TransactionPlan {
    pub id: TransactionId,
    pub operation: String,
    pub nonce: String,
    pub owner: ProcessIdentity,
    pub files: Vec<PlannedFile>,
}

pub struct PlannedFile {
    pub destination: PathBuf,
    pub action: StagedAction,
    pub existed_before: bool,
    pub original_hash: Option<Sha256Digest>,
    pub intended_hash: Option<Sha256Digest>,
    pub backup_path: Option<PathBuf>,
    pub staged_path: Option<PathBuf>,
    pub durability: Durability,
    pub original_permissions: Option<PortablePermissions>,
}
```

Exact names may differ, but equivalent information is mandatory.

## Required restore preparation sequence

1. validate manifest schema, layout, kinds, paths, source types, sizes, and checksums;
2. parse every incoming TOML document and validate domain constraints;
3. compute the complete merge or replace result in memory;
4. compute the deterministic destination order;
5. acquire the local-data/transaction lock;
6. revalidate any inputs that can change after lock acquisition;
7. create a transaction ID and private transaction workspace;
8. create durable backups for every existing destination;
9. create durable staged replacement files in the destination filesystem where possible;
10. populate every `PlannedFile` field;
11. atomically and durably persist the complete journal as `BackupsDurable`;
12. only then begin live replacement.

The persisted journal must never rely on information present only in an in-memory clone.

## Required commit sequence

Use a single production API, for example:

```rust
execute_transaction(&state_dir, &mut journal, |planned_file| {
    apply_planned_file(planned_file)
})
```

The executor must:

1. persist `Committing { next_position: 0 }`;
2. apply one file atomically;
3. verify the destination hash;
4. persist the next position;
5. repeat in deterministic order;
6. persist `Committed`;
7. record exactly one pending generation if syncable content changed;
8. schedule auto-sync once after commit;
9. clean staged files and backups only after durable completion.

Do not call `commit_transaction` as a marker around writes performed elsewhere.

## Required action semantics

- `Replace`: atomically install staged bytes over an existing destination;
- `Create`: atomically install a new destination and remember that rollback must remove it;
- `Delete`: atomically move or remove an existing destination with a durable backup;
- `NoOp`: perform no live write and do not count as a changed file.

## Required failpoints

Under `test-support`, add failpoints at:

- after complete `BackupsDurable` journal persistence;
- after transition to `Committing`;
- after first destination replacement;
- after the last library replacement but before index replacement;
- after index replacement;
- before `Committed` persistence;
- after `Committed` but before pending notification;
- after pending notification but before cleanup.

Each failpoint must support:

- returning a typed injected error;
- immediate process termination with a distinctive exit code.

Production builds must not recognize failpoint environment variables.

## Closure criteria

- all state-transition helpers are used by production restore;
- no transaction transition helper is `dead_code`;
- a complete `BackupsDurable` journal exists before the first live write;
- commit progress is durable after each destination;
- successful restore records one pending generation only after durable commit;
- crash tests run against the actual `snp restore` subprocess.

---

# Workstream D — Make rollback atomic, byte-exact, and restartable

## Goal

Guarantee recovery to the exact pre-transaction state after handled failure or process death.

## Progress representation

Do not use an ambiguous live-file index for reverse traversal.

Prefer a rollback cursor defined in rollback-order coordinates:

```rust
RollingBack {
    next_rollback_position: usize,
}
```

Build a deterministic vector of commit positions in reverse order. Persist the number of already completed rollback actions, not the original file index plus one.

Example:

```rust
let rollback_order: Vec<usize> = (0..files.len()).rev().collect();
for position in next_rollback_position..rollback_order.len() {
    let file_index = rollback_order[position];
    rollback_one(&files[file_index])?;
    persist(RollingBack {
        next_rollback_position: position + 1,
    })?;
}
```

## Required rollback semantics

For each planned action:

- `Replace`: atomically restore exact backup bytes and permissions;
- `Create`: remove the newly created destination, using safe path/type checks;
- `Delete`: atomically restore the deleted file from backup;
- `NoOp`: do nothing.

After each action:

- fsync according to durability class;
- verify restored hash or confirmed absence;
- persist progress before continuing.

If an artifact is missing or validation fails:

- persist a typed unrecoverable state;
- retain journal, backups, and staged files;
- do not report success;
- provide operator instructions through `repair --json`.

## Required tests

Handled-error tests:

- failure after first live replacement restores exact original bytes;
- failure after all libraries but before index restores exact original state;
- failure after index replacement restores exact original state;
- a destination created by the failed restore is removed;
- a destination deleted by the failed restore is restored;
- no pending marker is created;
- no worker or executor starts.

Crash-subprocess tests:

- kill after `BackupsDurable`;
- kill after first replacement;
- kill after index replacement;
- kill during rollback after one rollback action;
- rerun repair or a mutating command and complete recovery;
- verify byte-for-byte state and permissions where supported;
- verify no stale lock permanently blocks recovery.

## Closure criteria

- rollback uses atomic persistence, not direct final `fs::copy` writes;
- restart after a second crash continues at the correct rollback position;
- newly created files are removed;
- restored hashes match original hashes;
- unrecoverable state retains evidence.

---

# Workstream E — Gate all mutations on interrupted-transaction recovery

## Goal

Never begin a new local mutation while an interrupted transaction is unresolved.

## Required policy

Before any local mutating operation enters its write phase:

1. inspect the canonical transaction directory;
2. if no interrupted journal exists, continue;
3. if one complete and unambiguous journal exists, either perform safe automatic rollback or return a typed recovery-required result;
4. if multiple or incomplete journals exist, refuse mutation and direct the user to `snp repair`;
5. read-only commands may report but must not mutate transaction state.

The policy must cover:

- snippet new/edit/delete;
- library create/delete/set-primary;
- import;
- restore;
- repair writes;
- migrations;
- sync merge writes;
- any future local-data mutation through a shared application boundary.

Avoid sprinkling checks through command handlers. Prefer one application-level mutation gate.

## Repair behavior

`repair --dry-run --json` must report:

- transaction ID;
- operation;
- state;
- affected destinations;
- available/missing backups and staged artifacts;
- recommended action;
- whether automatic recovery is safe.

`repair --apply` must:

- acquire the transaction lock;
- re-read the journal after lock acquisition;
- execute the canonical recovery path;
- report exact outcome;
- leave evidence on failure.

## Closure criteria

- a new mutation cannot silently proceed over an interrupted restore;
- repair inspects and recovers production journals;
- status and doctor remain read-only;
- machine-readable diagnostics are stable and tested.

---

# Workstream F — Correct transaction lock ownership and stale reclaim

## Goal

Prevent concurrent entry, PID-reuse theft, evidence loss, and stale-lock races.

## Required process identity

Add a process identity with PID plus start identity where available:

```rust
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_token: Option<String>,
}
```

Suggested platform sources:

- Linux: `/proc/<pid>/stat` start time, parsed defensively;
- macOS: process start time via a supported system API or a documented conservative fallback;
- Windows: `OpenProcess` plus `GetProcessTimes` creation time;
- unsupported/failure: fail conservatively when a live PID cannot be distinguished safely.

The lock record must include:

- schema version;
- PID;
- start token;
- random nonce;
- transaction ID or operation ID;
- creation timestamp;
- operation.

## Acquisition protocol

1. create the lock with `create_new`;
2. write and sync the complete record through the opened handle;
3. read back and verify PID/start-token/nonce ownership;
4. on existing lock, parse and validate the record;
5. live matching owner means typed contention;
6. dead or start-token-mismatched owner may be reclaimed;
7. malformed lock is renamed atomically to a quarantine filename and reported, not silently deleted;
8. after quarantine/removal, retry `create_new` from the beginning;
9. never recreate a reclaimed lock with plain `fs::write`.

Unlock only when PID, start token, and nonce match the guard.

## Required tests

Use real child processes rather than magic PID assumptions.

- spawn a child that stays alive; its lock blocks acquisition;
- capture its PID and start token;
- terminate and wait for it; stale lock becomes reclaimable;
- simulate same PID with a mismatched start token; refuse or reclaim safely according to policy;
- wrong nonce cannot remove the lock;
- malformed lock is quarantined and visible to repair;
- two concurrent reclaimers result in exactly one owner;
- two concurrent restores cannot both enter commit;
- Windows tests never assume PID `1` is dead.

## Closure criteria

- PID reuse is addressed where platform APIs permit;
- malformed locks are preserved or quarantined;
- reclaim is exclusive and race-safe;
- no magic PID is used to represent a dead process in integration tests;
- Linux, macOS, and Windows lock tests pass.

---

# Workstream G — Serialize backup snapshots with every relevant mutation

## Goal

Guarantee a backup contains a complete before-state or complete after-state, never a mixed index/library set.

## Required synchronization model

Use a short-lived `LocalDataLock` shared by all local TOML writers and backup capture.

A portable exclusive lock is acceptable; backup does not need a shared/read lock. It should hold the lock only while identifying and reading the snapshot.

Required users:

- snippet create/edit/delete;
- library create/delete/set-primary;
- import;
- restore commit/rollback;
- repair writes;
- migration writes;
- sync merge writes;
- backup snapshot capture.

Recommended boundary:

```rust
pub fn with_local_data_mutation<T>(
    operation: &str,
    f: impl FnOnce() -> SnipResult<T>,
) -> SnipResult<T>;

pub fn capture_local_snapshot<T>(
    operation: &str,
    f: impl FnOnce() -> SnipResult<T>,
) -> SnipResult<T>;
```

Both should acquire the same lock implementation.

The existing generation counter may remain as an integrity assertion, but it is not sufficient unless every mutation updates it within the same lock boundary.

## Snapshot protocol

While holding the local-data lock:

1. enumerate the defined source set;
2. reject symlinks, reparse points, and non-regular files;
3. canonicalize and verify containment;
4. enforce size limits;
5. read exact bytes;
6. parse and validate TOML/domain invariants;
7. capture generation and/or a deterministic snapshot fingerprint.

Release the lock before writing the backup output directory.

Continue using sibling staging plus atomic rename, but also:

- reject an already-existing output directory unless an explicit overwrite policy exists;
- fsync staged files and staging directory before rename;
- fsync the parent directory after rename where supported;
- remove incomplete staging on handled failure.

## Required concurrency tests

Use barriers or failpoints to pause backup after reading a known source while another process attempts a real `snp` mutation.

Test at minimum:

- snippet create during snapshot;
- exact edit during snapshot;
- snippet delete during snapshot;
- library create/delete during snapshot;
- set-primary during snapshot;
- sync merge write during snapshot where practical.

For each test:

- mutation blocks until capture releases the lock, or backup retries under a documented seqlock design;
- backup represents either the complete before-state or complete after-state;
- index and library set agree;
- manifest hashes match copied bytes;
- no arbitrary sleep is the primary synchronization mechanism.

## Closure criteria

- every production writer uses the local-data boundary;
- snippet mutations cannot bypass snapshot coordination;
- actual concurrent tests prove before-or-after coherence;
- test names do not overstate sequential checks as concurrency proof.

---

# Workstream H — Make manifest validation typed, early, and independently tested

## Goal

Ensure every accepted manifest has one unambiguous, safe restore interpretation.

## Typed model

Change the manifest entry to use the enum directly:

```rust
pub struct BackupManifestEntry {
    pub path: BackupRelativePath,
    pub kind: BackupEntryKind,
    pub size: u64,
    pub sha256: Sha256Digest,
}
```

A custom deserializer may retain readable diagnostics, but unknown kinds must fail during manifest parsing for supported schemas.

## Required early validation order

Before source reads and checksum verification:

1. reject unsupported schema version;
2. require exact supported layout;
3. reject unknown kind;
4. validate normalized relative path;
5. compute exact source and destination mapping;
6. reject duplicate source paths;
7. reject duplicate destination paths;
8. reject Unicode/case-fold collisions according to the documented portable policy;
9. reject control characters, trailing dots/spaces, drive-relative paths, UNC paths, mixed-separator traversal, reserved names, and aliases;
10. reject duplicate snippet IDs within and across incoming libraries according to the identity contract;
11. parse every incoming TOML file into the expected domain type;
12. only then stat, size-check, and hash source artifacts.

Canonicalize the backup root once. Open source artifacts safely and prove containment beneath the canonical root.

## Test discipline

Each negative test must fail for exactly one intended condition.

For schema tests:

- create otherwise valid files;
- use correct file sizes and hashes;
- assert the error class/message identifies schema, not checksum.

For collision tests:

- use valid source bytes and hashes;
- require nonzero exit;
- assert no transaction artifacts and no live writes;
- never accept either success or failure.

For duplicate-ID tests:

- use a valid manifest and hashes;
- require rejection before transaction creation;
- assert the duplicate ID appears only in a bounded diagnostic, not raw snippet payloads.

## Required tests

- schema `0` rejected for schema reason;
- future schema rejected for schema reason;
- wrong layout rejected;
- unknown kind rejected during parse, including dry-run;
- duplicate normalized source rejected;
- duplicate normalized destination rejected;
- case-fold collision rejected on every host OS using portable normalization logic;
- trailing-dot and trailing-space aliases rejected;
- Windows drive-relative and UNC paths rejected on all host OSes;
- duplicate IDs within one library rejected;
- duplicate IDs across two incoming libraries rejected if globally forbidden by the identity contract;
- malformed TOML rejected before any transaction artifact;
- valid backup round-trips in merge and replace modes.

## Closure criteria

- restore no longer branches on free-form kind strings;
- schema/layout validation is visible in production code;
- tests use valid hashes when testing domain validation;
- no test accepts both success and failure;
- no ambiguous manifest can reach transaction preparation.

---

# Workstream I — Complete server-observable and read-only evidence

## Goal

Make tests fail when network or lifecycle behavior differs from the contract, even if local status files look plausible.

## Headline E2E telemetry

Retain and expose the recording server handle. Assert:

- initial server revision `R0`;
- initial canonical sync request count `0`;
- exactly one worker start;
- exactly one executor start;
- exactly one canonical sync request start and completion;
- expected authenticated device identity;
- expected target library identity;
- expected encrypted payload is present and non-empty;
- post-sync revision `R1` differs from `R0`;
- stored server snippet count is exactly `1`;
- maximum concurrent canonical sync requests is `1`;
- pending generation `G` clears only after server completion;
- request count remains `1` through a bounded quiet period.

Use event and state polling, not arbitrary sleeps, except for the bounded quiet-period duplicate check.

## Real no-op-success seam

Under `test-support`, implement an executor mode that returns local success without invoking canonical sync.

Example:

```rust
#[cfg(feature = "test-support")]
if std::env::var_os("SNP_TEST_EXECUTOR_NOOP_SUCCESS").is_some() {
    return EXECUTOR_SUCCESS;
}
```

Production builds must not compile this branch.

The regression test must prove:

- executor reports success;
- canonical request count stays zero;
- revision stays `R0`;
- pending is not accepted as safely acknowledged;
- the headline assertion would fail.

An unreachable server is a separate failure test and does not satisfy this requirement.

## Read-only evidence

Create a recording server and isolated event sink for each test. For every read-only/dry-run command:

- set valid pending generation `G`;
- write known status bytes `S0`;
- invoke one command;
- assert zero worker events;
- assert zero executor events;
- assert zero server requests;
- assert pending remains exactly `G`;
- assert status remains byte-identical to `S0`, except where explicitly documented;
- assert exit code and machine-output contract.

Remove the status-string proxy used by `assert_no_worker_spawned`.

## Event isolation

The headline test is reported as flaky in full-suite execution. Fix the isolation rather than serializing the entire workspace indefinitely.

Recommended approach:

- unique event directory per test;
- unique test/session ID included in every event;
- subprocess environment carries that ID;
- readers filter by ID;
- clean sink before invocation;
- avoid process-global mutable environment in concurrently running tests;
- use `serial_test` only as a temporary, documented fallback for unavoidable globals.

## Closure criteria

- zero remote effect always fails the headline test;
- false local executor success cannot clear the proof requirement;
- read-only tests observe network and lifecycle absence directly;
- deterministic E2E passes in isolation and within the full workspace suite.

---

# Workstream J — Compile-time gate test credential behavior

## Goal

Ensure production binaries cannot activate test credential storage or retrieval.

## Required design

All behavior associated with `SNP_TEST_CREDENTIAL_FILE` must be under:

```rust
#[cfg(feature = "test-support")]
```

Preferred design: encapsulate credential access behind a provider selected at compile time.

```rust
trait CredentialProvider {
    fn store(&self, key: &str) -> SnipResult<CredentialReference>;
    fn load(&self, reference: &CredentialReference) -> SnipResult<String>;
}
```

Production provider:

- keychain with existing explicit plaintext fallback policy;
- ignores or rejects test-only environment variables.

Test provider under `test-support`:

- isolated file or in-memory representation;
- private permissions;
- no secret in argv;
- inherited safely by worker and executor subprocesses;
- unavailable when the feature is absent.

## Required tests

- build without `test-support`, set `SNP_TEST_CREDENTIAL_FILE`, and prove it does not change serialization or retrieval;
- production build never writes plaintext because of the test variable;
- test-support build uses the isolated provider across parent, worker, and executor;
- credentials never appear in argv, lifecycle events, diagnostics, or logs;
- test credential file permissions are restrictive where supported.

## Closure criteria

- no production code path recognizes the test credential variable;
- deterministic tests remain cross-platform;
- production credential semantics remain unchanged.

---

# Workstream K — Complete execution-outcome mapping for every branch

## Goal

Make the public exit-code contract independent of output redirection and shell-spawn location.

## Required refactor

Convert execution helpers to return a typed outcome rather than mixing `SnipError` and `ProcessResult` for expected execution failures.

Example:

```rust
pub enum ChildExecutionOutcome {
    Exited(ExitStatus),
    TimedOut,
    SpawnFailed { class: String },
    WaitFailed { class: String },
}
```

Both normal and output-file branches must map:

- normal zero exit -> success;
- child nonzero exit with code -> that child code;
- signal/no code -> `8`;
- timeout -> `8`;
- shell/spawn failure -> `8`;
- wait/status failure -> `8` unless a more specific public policy is documented.

Only configuration, parsing, selection, persistence, or internal application failures should use generic exit code `1`.

## Required tests

Run each case for:

- ordinary snippet;
- snippet with `output` path;
- exact `--id` selection;
- interactive/filter selection where practical.

Cases:

- zero exit;
- explicit child exit `7`;
- signal termination on Unix;
- timeout;
- invalid shell/spawn failure;
- output-file creation failure remains generic filesystem error if no child was started;
- raw command and secret values are absent from diagnostics.

On Windows, add a native process-termination case rather than emulating Unix signals.

## Closure criteria

- output-file timeout and spawn failure exit `8`;
- documented outcome matrix matches implementation;
- no expected child execution failure reaches generic exit `1`.

---

# Workstream L — Diagnose and close Windows CI systematically

## Goal

Replace reactive Windows patches with a reproducible platform validation pass.

## Mandatory first step: capture actual failures

Before modifying tests or workflow files, record:

- workflow run URL;
- failing job names;
- failing step names;
- exact error excerpts;
- whether each failure is setup, compile, test assertion, timeout/hang, or shell portability;
- whether the failure reproduces with the same command on a local Windows VM or runner.

Add this table to the implementation status file:

| Job | Step | Failure class | Root cause | Fix commit | Rerun conclusion |
|---|---|---|---|---|---|

Do not infer a green matrix from local Linux tests.

## L1. Fix package-smoke shell portability

The current matrix uses Unix shell syntax on Windows. Split the step by operating system.

Unix example:

```yaml
- name: Package and install (Unix)
  if: runner.os != 'Windows'
  shell: bash
  run: |
    set -euo pipefail
    cargo package -p snip-it --locked --allow-dirty
    crate_file=$(ls target/package/snip-it-*.crate | head -1)
    unpack_dir=$(mktemp -d)
    tar -xzf "$crate_file" -C "$unpack_dir"
    package_dir=$(find "$unpack_dir" -maxdepth 1 -type d -name 'snip-it-*' | head -1)
    install_root="$RUNNER_TEMP/snp-install"
    cargo install --path "$package_dir" --locked --root "$install_root"
    "$install_root/bin/snp" --version
    "$install_root/bin/snp" --help >/dev/null
```

Windows example:

```yaml
- name: Package and install (Windows)
  if: runner.os == 'Windows'
  shell: pwsh
  run: |
    $ErrorActionPreference = 'Stop'
    cargo package -p snip-it --locked --allow-dirty
    $crate = Get-ChildItem 'target/package/snip-it-*.crate' | Select-Object -First 1
    if (-not $crate) { throw 'snip-it crate archive not found' }
    $unpack = Join-Path $env:RUNNER_TEMP ("snip-it-package-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $unpack | Out-Null
    tar -xzf $crate.FullName -C $unpack
    $packageDir = Get-ChildItem $unpack -Directory | Where-Object Name -Like 'snip-it-*' | Select-Object -First 1
    if (-not $packageDir) { throw 'unpacked snip-it package directory not found' }
    $installRoot = Join-Path $env:RUNNER_TEMP 'snp-install'
    cargo install --path $packageDir.FullName --locked --root $installRoot
    $snp = Join-Path $installRoot 'bin/snp.exe'
    & $snp --version
    & $snp --help | Out-Null
```

Remove `snp status ... || true`. A smoke test must assert an expected outcome. Use an isolated config directory and explicitly accept only a documented status code.

## L2. Consolidate `protoc` provisioning

The workflow currently repeats large OS-specific setup blocks across jobs.

Create one checked-in setup script or composite action, for example:

- `.github/actions/setup-protoc/action.yml`, or
- `ci/setup-protoc.ps1` and `ci/setup-protoc.sh`.

Requirements:

- one pinned protobuf version;
- correct Windows asset name;
- unique extraction directory per job;
- remove stale destination before extraction;
- retry bounded network downloads;
- verify SHA-256 if practical;
- update current process PATH and `GITHUB_PATH`;
- execute `protoc --version` and assert the exact expected version;
- avoid GitHub API-dependent setup actions if rate limits caused prior failures.

Use the helper in every job that compiles `snip-proto`.

## L3. Remove magic PID assumptions

Do not use PID `1`, `u32::MAX/2`, or another guessed PID as the primary liveness proof.

For live-owner tests:

- spawn a test child that blocks on a pipe/event;
- capture PID and start identity;
- write or acquire the lock under that identity;
- assert contention.

For dead-owner tests:

- terminate and wait for that same child;
- retain its PID/start identity in the stale lock;
- assert reclaim behavior.

This is reliable on Windows and also tests PID-reuse protection.

## L4. Separate Unix signals from Windows termination semantics

- gate Unix `libc`, `SIGTERM`, and `SIGKILL` tests with `#[cfg(unix)]` at module/import/function level;
- add Windows-native child termination tests using `Child::kill`, a test helper process, or Windows APIs;
- assert public exit semantics, not Unix signal numbers, on Windows;
- keep common behavior tests platform-neutral.

## L5. Replace permissive filesystem skips with platform capabilities

FIFO is a Unix capability test, not Windows evidence.

- keep FIFO rejection under `#[cfg(unix)]`;
- if the runner cannot create a FIFO, report an explicit ignored/capability result rather than returning silently from a passing test;
- add a Windows reparse-point/symlink rejection test when privileges permit;
- when Windows symlink creation is unavailable, test the production metadata classifier directly with a controlled fixture and document the limitation;
- never count a skipped FIFO test as Windows non-regular-file coverage.

## L6. Audit command and path portability

Review Windows-sensitive tests for:

- `/bin/sh`, `true`, `false`, `sleep`, `kill`, and shell quoting;
- forward-slash-only path assertions;
- executable suffix assumptions;
- files remaining open during rename/delete;
- deleting temporary directories while child handles remain open;
- CRLF-sensitive byte assertions;
- environment variables modified globally across concurrent tests;
- clipboard and editor invocation in headless CI.

Create small platform helper functions in test support rather than conditionals scattered through suites.

Examples:

```rust
fn success_command() -> (&'static str, Vec<&'static str>);
fn sleep_command(seconds: u64) -> CommandSpec;
fn terminate_child(child: &mut Child) -> io::Result<()>;
fn executable_name(base: &str) -> String;
```

## L7. Prevent Windows hangs

For every subprocess test:

- set a bounded timeout;
- close inherited stdin unless needed;
- use `-NonInteractive`/`-NoProfile` for PowerShell;
- ensure editor and clipboard tests cannot open UI;
- terminate the full child process tree on timeout;
- print child stdout/stderr and observed events on failure.

## L8. Matrix commands

At minimum, Windows must run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo test --release --workspace --all-features -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test execution_outcomes --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
cargo test --test update_archive_security --features test-support -- --test-threads=1
package/install smoke from the unpacked .crate
```

A temporary `--test-threads=1` policy is acceptable for process-global environment tests, but the final status must identify why it remains necessary.

## Windows closure criteria

- the exact previously failing Windows jobs are rerun successfully;
- no Windows job uses Bash syntax under PowerShell or vice versa;
- `protoc` setup is centralized and version-verified;
- no liveness test relies on a guessed dead PID;
- Unix-only tests do not compile or run on Windows;
- Windows-specific termination, path, archive, lock, and package tests pass;
- no test converts an unsupported capability into a silent pass;
- no job is hidden behind `continue-on-error` or `|| true`;
- workflow URL and every Windows job conclusion appear in closure evidence.

---

# Workstream M — Reconcile CI policy and closure evidence

## Goal

Make CI results the evidence source rather than commit messages or local test counts.

## Workflow requirements

- `fail-fast: false` may remain so all platform evidence is collected;
- no release-blocking job may use `continue-on-error`;
- no command may append `|| true` to suppress an unexpected result;
- action references must follow one documented pinning policy;
- repeated setup logic should be centralized;
- package jobs must build and install from unpacked `.crate` contents;
- artifact names should include OS and commit SHA where useful;
- upload failure diagnostics for deterministic E2E and crash tests.

## Required final status file update

Update `plans/snip-it-correctness-11-closure-status.md` with:

- corrective baseline and final commit SHA;
- changed files grouped by this plan’s workstreams;
- exact local commands and results;
- total test counts and ignored tests with reasons;
- GitHub Actions workflow URL;
- every job name and conclusion;
- Linux, macOS, and Windows evidence tables;
- failpoint matrix results;
- server-observable E2E measurements;
- transaction crash/recovery results;
- residual limitations;
- criterion-by-criterion checklist.

The top-level program status may change to complete only after all required jobs pass on the final commit.

---

## 4. Recommended implementation sequence

Use narrowly scoped commits. Do not combine all work into one large claim commit.

1. `plans: reopen Phase 11 around 11B corrective blockers`
2. `refactor: centralize transaction state directory and recovery inspection`
3. `fix: persist complete restore transaction plans before live writes`
4. `fix: make rollback atomic restartable and create-aware`
5. `fix: add mutation gate and real interrupted transaction recovery`
6. `fix: harden transaction lock identity quarantine and reclaim`
7. `fix: coordinate backup snapshots with all local mutations`
8. `fix: make backup manifests typed and fail closed`
9. `test: require canonical server and lifecycle evidence`
10. `fix: compile-time gate deterministic test credentials`
11. `fix: unify child execution outcomes including output-file runs`
12. `ci: make package smoke and protoc setup portable on Windows`
13. `test: replace Windows PID signal and filesystem assumptions`
14. `docs: record final Phase 11B evidence and closure decision`

A commit may be split further. Do not mark closure in the same commit that introduces the final functional changes unless CI has already run against that exact commit.

---

## 5. Required local verification commands

Run from repository root.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo test --release --workspace --all-features -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test restore_security --features test-support -- --test-threads=1
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test execution_outcomes --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
cargo test --test update_archive_security --features test-support -- --test-threads=1
cargo deny check
cargo package -p snip-proto --locked --allow-dirty
cargo package -p snip-sync --locked --allow-dirty
cargo package -p snip-it --locked --allow-dirty
```

Also build without test support and prove the test credential seam is absent:

```bash
cargo build --release --no-default-features
```

Use an integration test or binary string/symbol inspection only as supplementary evidence; the decisive proof is behavioral: setting the test variable must not affect the production binary.

---

## 6. Required CI evidence matrix

| Evidence | Linux | macOS | Windows |
|---|---:|---:|---:|
| debug workspace tests | required | required | required |
| release workspace tests | required | required | required |
| deterministic E2E with test-support | required | required | required |
| transaction crash/recovery | required | required | required |
| backup concurrency | required | required | required |
| manifest contracts | required | required | required |
| execution outcomes | required | required | required |
| read-only no-recovery | required | required | required |
| update archive security | required | required | required |
| unpacked package install smoke | required | required | required |
| transaction lock live/dead identity | required | required | required |
| platform-specific process termination | Unix | Unix | Windows-native |
| non-regular/reparse source rejection | FIFO/symlink | FIFO/symlink | reparse/symlink or documented direct classifier proof |

No platform column may be marked complete based only on compilation.

---

## 7. Explicit final closure criteria

### Status and evidence

- [ ] Phase 11 status names this plan until completion.
- [ ] Final status references the actual final commit.
- [ ] Final status includes a GitHub Actions workflow URL.
- [ ] Every required job and platform conclusion is recorded.
- [ ] No claim relies only on a commit message or test name.

### Transactions

- [ ] One canonical transaction directory is used everywhere.
- [ ] Restore persists complete rollback metadata before live writes.
- [ ] Production restore uses `BackupsDurable` and `Committing` transitions.
- [ ] Commit progress is persisted after each destination.
- [ ] Rollback progress uses unambiguous rollback-order coordinates.
- [ ] Rollback atomically restores replaced and deleted files.
- [ ] Rollback removes newly created files.
- [ ] Recovery verifies hashes or absence after each action.
- [ ] A second crash during rollback is recoverable.
- [ ] Failed or rolled-back restore creates no pending generation.
- [ ] Successful content-changing restore creates exactly one pending generation.
- [ ] No new mutation begins over an unresolved transaction.

### Locking

- [ ] Lock includes PID, start identity where available, nonce, and operation/transaction identity.
- [ ] Live owner blocks second acquisition.
- [ ] Dead owner is reclaimed through exclusive retry.
- [ ] PID reuse is handled safely.
- [ ] Malformed lock is quarantined, not silently destroyed.
- [ ] Wrong owner cannot remove a lock.
- [ ] Concurrent reclaimers produce exactly one owner.

### Backup

- [ ] All snippet, library, import, restore, repair, migration, and sync writers use local-data coordination.
- [ ] Backup holds coordination during enumeration and byte capture.
- [ ] Concurrent tests prove complete before-state or complete after-state.
- [ ] Index and library files cannot represent mixed generations.
- [ ] Partial backup output cannot appear complete.

### Manifest and restore domain

- [ ] Manifest kind is typed in the serialized model.
- [ ] Unsupported schema and layout fail before source hashing.
- [ ] Unknown kind fails during parsing.
- [ ] Source and destination collisions fail closed.
- [ ] Portable case-fold and Windows alias rules are enforced.
- [ ] Duplicate incoming snippet IDs fail before transaction creation.
- [ ] Negative tests use otherwise valid files and hashes.
- [ ] No manifest test accepts either success or failure.

### Sync and read-only evidence

- [ ] Headline E2E proves exactly one canonical request.
- [ ] Expected device and library identities are asserted.
- [ ] Server revision changes and encrypted payload exists.
- [ ] Maximum canonical request concurrency is one.
- [ ] Pending clears only after server completion.
- [ ] No-op local success cannot satisfy the headline proof.
- [ ] Read-only tests assert zero lifecycle events and zero server requests.
- [ ] E2E passes both alone and in the full workspace run.

### Credentials and execution

- [ ] Production builds do not recognize the test credential variable.
- [ ] Test credentials remain deterministic across subprocesses.
- [ ] Credentials do not appear in argv, logs, events, or diagnostics.
- [ ] Output-file timeout exits `8`.
- [ ] Output-file shell/spawn failure exits `8`.
- [ ] Signal/no-code termination exits `8`.
- [ ] Generic exit `1` is reserved for application/configuration failures.

### Windows CI

- [ ] Actual failing Windows jobs and root causes are recorded.
- [ ] Package smoke uses valid PowerShell or explicit Bash, never mixed syntax.
- [ ] `protoc` setup is centralized and exact-version verified.
- [ ] No Windows test assumes PID `1` or another guessed PID is dead.
- [ ] Unix signal code is fully gated from Windows.
- [ ] Windows-native termination semantics are tested.
- [ ] Filesystem capability tests do not silently pass when unsupported.
- [ ] No release-blocking Windows step uses `|| true` or `continue-on-error`.
- [ ] All required Windows jobs pass on the final commit.

### Architecture

- [ ] One installed `snp` binary remains the client architecture.
- [ ] Auto-sync workers remain one-shot subprocesses.
- [ ] No daemon, second helper binary, plugin runtime, or workflow engine is introduced.

---

## 8. Release decision rule

The final release decision is binary.

Mark Phase 11 and the correctness program complete only when:

1. every checkbox above is satisfied;
2. production code matches the documented transaction and credential contracts;
3. adversarial tests prove the required behavior for the intended reason;
4. Linux, macOS, and Windows CI jobs pass on the same final commit;
5. the closure status contains the workflow URL and exact conclusions.

If any required Windows job is unavailable, flaky, skipped without a release-blocking justification, or hidden behind permissive shell behavior, the program remains open.

If transaction crash recovery is only detectable but not executable, the program remains open.

If backup coherence relies only on a generation counter that snippet mutations can bypass, the program remains open.

If the test credential seam exists in a production build, the program remains open.

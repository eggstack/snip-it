# Phase 11E — Test-Boundary Security, Pending-Recovery Correctness, and Evidence Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `52563f9dcdc1c4bb681e3ce6f5d8404a0957fb22`

Parent plans:

- `plans/snip-it-correctness-11-verification-and-crash-closure.md`
- `plans/snip-it-correctness-11b-durability-verification-windows-ci-closure.md`
- `plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md`
- `plans/snip-it-correctness-11d-pending-staging-and-cross-platform-proof-closure.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan is the authoritative corrective handoff for the defects remaining after the partial Phase 11D implementation. It is intentionally narrow. It does not reopen the product architecture, the one-shot subprocess model, or work that is already materially correct.

Phase 11 and the correctness program must remain open until every release-blocking criterion in this plan is supported by production code, adversarial tests, and successful Linux, macOS, and Windows GitHub Actions jobs on the same final commit.

---

## 1. Objective

Close the remaining high-risk correctness and verification gaps without adding product scope:

1. make all test-only behavior compile-time unavailable in production builds;
2. make the feature-enabled integration-test binary reliable and explicit;
3. make `CommittedLocal` recovery fail closed on every pending-state error;
4. represent an unknown pending generation explicitly rather than with generation `0`;
5. make pending finalization idempotent across every crash boundary;
6. place failpoints at the exact boundaries their names and tests claim;
7. add real crash-during-rollback subprocess tests;
8. remove all durable staged and rollback artifacts after successful finalization;
9. preserve recovery evidence when cleanup fails;
10. protect transaction artifacts with private permissions;
11. preserve or deliberately normalize destination permissions during commit and rollback;
12. enforce manifest schema, layout, destination uniqueness, and domain consistency before inspecting artifacts;
13. replace permissive manifest fixtures with otherwise-valid, single-fault fixtures;
14. replace sequential lock tests with barrier-controlled concurrent tests;
15. make remote acknowledgement—not child exit code—the authority for pending clear;
16. prove that a false-success executor cannot clear pending;
17. inspect recording-server telemetry in the headline end-to-end test;
18. remove machine-local agent configuration from version control;
19. pass release-blocking Linux, macOS, and Windows CI on one final commit;
20. reconcile closure documentation only after the evidence exists.

---

## 2. Architectural constraints and non-goals

Preserve all of the following:

- one installed client binary: `snp`;
- auto-sync workers remain one-shot subprocesses;
- no resident client daemon;
- no second installed helper binary;
- no database replacing TOML state;
- no plugin runtime;
- no workflow engine;
- no distributed transaction service;
- no CRDT expansion;
- no broad CLI redesign;
- no platform-specific public command semantics;
- no secret, snippet command, or snippet body in process arguments;
- no secret or snippet payload in lifecycle events, status files, or logs.

Allowed internal changes:

- meaningful code behind the existing `test-support` feature;
- hidden test-only subcommands compiled only with `test-support`;
- test-only failpoint and barrier modules compiled only with `test-support`;
- an explicit pending-finalization state model;
- per-transaction artifact directories;
- private-permission helpers;
- destination permission metadata where supported;
- a change in pending-clear ownership from worker to executor;
- recording-server request metadata in test helpers;
- checked-in CI helper scripts.

Do not solve these defects by adding another long-running process, an installed helper, a transaction database, or a generalized test-control framework.

---

## 3. Confirmed baseline defects

The implementation agent must treat every item in this section as an open defect.

### 3.1 Test-only behavior is production-accessible

The current production binary honors these environment variables without a feature boundary:

- `SNP_TEST_FAILPOINT` can abort a normal restore at a matching boundary;
- `SNP_TEST_EXECUTOR_MODE=noop-success` can return executor success without synchronization;
- `SNP_SKIP_WORKER_SPAWN` can suppress worker creation while scheduling still reports `SpawnNow`;
- `SNP_TEST_EVENTS_DIR` can cause production processes to emit test lifecycle files.

“Production does not normally set the variable” is not an acceptable boundary. Environment variables are caller-controlled input.

### 3.2 The current status file contradicts the code

The closure status claims the executor seam and worker suppression are feature-gated. They are not. It marks every Phase 11D workstream complete even though several implementation and evidence requirements remain open.

### 3.3 `CommittedLocal` recovery discards pending failures

When `pending_recorded == false`, recovery calls `ensure_pending_for_transaction` and ignores its result. It then marks pending as recorded and deletes transaction evidence.

A lock timeout, corrupt marker, permission error, I/O error, or semantic conflict can therefore leave committed local content without valid pending intent.

### 3.4 Generation `0` is used as an unknown sentinel

`CommittedLocal` stores a plain `u64` and uses `0` before a pending generation exists. The recovery path can compare a marker against this placeholder and reason incorrectly.

### 3.5 Failpoint names and placement disagree

The failpoint named “after pending, before journal update” currently executes after the journal has been updated.

The failpoint named “after journal pending, before cleanup” currently executes after `commit_transaction` has already removed the journal and backups.

### 3.6 Rollback failpoint tests are placeholders

The two rollback crash tests do not trigger a real rollback and then crash inside rollback. One test performs a successful restore and documents the missing mechanism. The second has no effective body.

### 3.7 Durable staged content is retained indefinitely

Successful commit, rollback, and `CommittedLocal` recovery remove backup files and journals but do not reliably remove `durable_staged_path` files or the containing staged directory.

These files can contain plaintext snippet commands and sync configuration.

### 3.8 Transaction artifact permissions are implicit

The staging helper uses ordinary file creation and relies on umask or platform defaults. Journals, backup copies, staged files, and artifact directories are not explicitly created with the repository’s sensitive-state permission policy.

### 3.9 Destination permission metadata is absent

The journal does not record original destination permissions. Atomic replacement can change file modes, and rollback cannot prove it restored relevant permissions.

### 3.10 Manifest contract validation remains permissive

Restore does not demonstrate one explicit semantic-validation phase that rejects unsupported schema/layout and destination/domain inconsistencies before source artifact inspection.

Several negative tests use placeholder hashes or invalid content. The case-fold collision test accepts either success or failure.

### 3.11 “Barrier” tests are sequential

The new local-data lock tests run mutation and backup one after another. They do not force backup to contend while a multi-file mutation is paused between writes.

### 3.12 A false-success executor is not tested through the worker

The executor seam is tested directly. The suite does not prove that a worker observing child exit code `0` preserves pending when no remote acknowledgement occurred.

### 3.13 Server telemetry remains unused

The deterministic E2E test still discards the recording-server handle. It proves a row-count change and lifecycle counts, but not the exact request identity, target, payload properties, concurrency, or duplicate-request absence.

### 3.14 Machine-local configuration remains tracked

`.poolside/settings.local.yaml` is still present despite the closure document claiming it was removed.

### 3.15 Same-commit CI evidence is absent

The current head has no connector-visible combined status or workflow runs. The commit history shows active Windows and timing stabilization rather than a settled release candidate.

---

# Workstream A — Reopen the closure record accurately

## Goal

Make repository status truthful before implementation proceeds.

## Required first commit

Update `plans/snip-it-correctness-11-closure-status.md` to state:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11e-test-boundary-pending-recovery-and-evidence-closure.md
Corrective baseline: 52563f9dcdc1c4bb681e3ce6f5d8404a0957fb22
```

Remove or mark superseded the claims that:

- every Phase 11D workstream is complete;
- test seams are compile-time gated;
- rollback crash tests cover both rollback positions;
- all transaction artifacts are cleaned;
- permission restoration is complete;
- manifest tests isolate the named failure;
- barrier-controlled concurrency is proven;
- recording-server telemetry is complete;
- only CI evidence remains.

## Acceptance criteria

- no open item in this plan is pre-marked complete;
- the status file contains no stale “final commit” value;
- local test counts are labeled with the exact commit on which they were produced;
- historical evidence remains available but is explicitly marked historical or superseded.

---

# Workstream B — Restore a real compile-time test boundary

## Goal

Ensure a production build cannot activate failpoints, false-success execution, worker suppression, test lifecycle sinks, or mutation barriers through environment variables.

## Required implementation

Use the existing feature as an actual boundary:

```toml
[features]
default = []
test-support = []
```

Every test seam must have paired implementations:

```rust
#[cfg(feature = "test-support")]
pub fn maybe_failpoint(name: &str) {
    if std::env::var("SNP_TEST_FAILPOINT").as_deref() == Ok(name) {
        std::process::abort();
    }
}

#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn maybe_failpoint(_name: &str) {}
```

Apply the same pattern to:

- executor test modes;
- worker-spawn suppression;
- lifecycle event emission;
- mutation barriers;
- injected recoverable errors used to enter rollback;
- any hidden test-only CLI command.

Do not leave a normal-build environment check that changes behavior.

## Integration-test binary selection

The prior implementation removed feature gates because tests were apparently invoking a binary that lacked `test-support`. Correct the test harness instead of weakening production.

Required rules:

1. release-blocking test commands include `--features test-support`;
2. integration helpers invoke `env!("CARGO_BIN_EXE_snp")` or an equivalent Cargo-provided path for the current test build;
3. helpers must not hard-code `target/debug/snp` or discover an unrelated previously built binary;
4. child processes inherit only the explicit test variables required by that test;
5. tests clear test-control variables by default before adding a specific seam.

Example helper:

```rust
pub fn snp_cmd(config_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_snp"));
    command.env("SNP_CONFIG_DIR", config_dir);
    command.env_remove("SNP_TEST_FAILPOINT");
    command.env_remove("SNP_TEST_EXECUTOR_MODE");
    command.env_remove("SNP_SKIP_WORKER_SPAWN");
    command.env_remove("SNP_TEST_MUTATION_BARRIER_DIR");
    command
}
```

Exact configuration environment names may differ. Preserve the current supported config isolation mechanism.

## Production-seam proof

Add a dedicated test or CI script that builds `snp` without `test-support` into an isolated target directory and invokes valid matching values:

```bash
cargo build --locked --release --no-default-features --target-dir target/production-seam

SNP_TEST_FAILPOINT=restore-after-prepared \
  target/production-seam/release/snp restore <valid-backup>

SNP_TEST_EXECUTOR_MODE=noop-success \
  target/production-seam/release/snp auto-sync-execute --state-dir <state>
```

The proof must show:

- the matching failpoint does not abort the production binary;
- `noop-success` does not bypass normal executor behavior;
- `SNP_SKIP_WORKER_SPAWN` does not suppress production scheduling;
- `SNP_TEST_EVENTS_DIR` does not create an event file;
- a matching mutation barrier variable does not block production.

Use a valid scenario for each assertion. A nonmatching failpoint string is not evidence.

## Architecture guard

Add source-level or compile-level checks that reject unguarded test variables in production modules. A source scanner is acceptable only as an additional guard, not as the primary boundary.

## Acceptance criteria

- `cargo build --release --no-default-features` contains no behavioral test seam;
- every feature-enabled test invokes the feature-enabled binary;
- no test depends on deleting pending state to compensate for uncontrolled setup behavior when a hidden test-only setup path can avoid the mutation;
- release CI runs the production-seam proof on Linux and Windows;
- the closure status accurately describes the boundary.

---

# Workstream C — Make pending-finalization state explicit

## Goal

Represent the pending finalization protocol without sentinel values or ambiguous booleans.

## Required state model

Replace generation `0` as “unknown.” One acceptable model is:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingFinalization {
    NotRecorded,
    Recorded { generation: u64 },
    CoveredByExisting { generation: u64 },
}

pub enum TransactionState {
    // ...
    CommittedLocal {
        pending: PendingFinalization,
    },
    // ...
}
```

Equivalent typed structure is acceptable. Requirements:

- unknown is not encoded as a valid generation;
- a recorded transaction-associated marker is distinguishable from coverage by unrelated pending work;
- old journals deserialize through an explicit migration or are rejected with actionable repair output;
- serialization remains deterministic;
- the state contains no secrets or snippet content.

## State-transition rules

The production protocol must be:

```text
Committing(all positions complete)
  -> CommittedLocal(NotRecorded)
  -> ensure canonical pending
  -> CommittedLocal(Recorded(g) | CoveredByExisting(g))
  -> cleanup transaction artifacts
  -> remove journal last
  -> schedule existing pending
```

The journal remains the recovery authority until cleanup succeeds.

## Acceptance criteria

- no pending generation uses `0` as a placeholder;
- every state transition is persisted durably;
- terminal cleanup is idempotent;
- scheduling never records another generation;
- a crash at each arrow resumes without duplicate generation creation or evidence loss.

---

# Workstream D — Make `CommittedLocal` recovery fail closed

## Goal

Never delete recovery evidence unless canonical pending state is valid and the journal records the corresponding result.

## Required recovery behavior

For `CommittedLocal(NotRecorded)`:

1. call `ensure_pending_for_transaction`;
2. match every result explicitly;
3. on `Created` or `Reused`, persist `Recorded { generation }`;
4. on a supported latest-state conflict, persist `CoveredByExisting { generation }`;
5. on lock, I/O, corruption, integrity, serialization, or unsupported-conflict error, preserve the journal and all artifacts and return a nonzero error;
6. do not report successful mutation-gate recovery on failure.

Example:

```rust
let pending = ensure_pending_for_transaction(sync_state_dir, &journal.id, snapshot)
    .map_err(|error| {
        SnipError::runtime_error(
            "Committed restore requires pending recovery",
            Some(&format!(
                "Transaction {} is committed locally but pending intent could not be finalized: {error}. Recovery evidence was preserved; run `snp repair`.",
                journal.id
            )),
        )
    })?;

let finalization = match pending {
    TransactionPendingResult::Created(state)
    | TransactionPendingResult::Reused(state) => {
        PendingFinalization::Recorded { generation: state.generation }
    }
    TransactionPendingResult::Conflict(state) => {
        validate_existing_pending_covers_latest_state(&state)?;
        PendingFinalization::CoveredByExisting { generation: state.generation }
    }
};

persist_committed_local(transaction_dir, journal, finalization)?;
finalize_transaction_cleanup(transaction_dir, journal)?;
```

## Conflict policy

Document why unrelated pending work covers the restored state. This is valid only if one pending generation causes a full current-state synchronization rather than a mutation-specific delta.

If the protocol is not full-current-state, `Conflict` must remain an explicit recovery error rather than being silently accepted.

Add tests for both cases.

## Required fault tests

Inject the following failures while recovering `CommittedLocal(NotRecorded)`:

- pending lock busy;
- pending file permission denied where the platform supports it;
- corrupt canonical pending marker;
- integrity mismatch;
- unrelated pending generation;
- journal persistence failure after pending creation;
- cleanup failure after journal records pending.

For every error:

- committed live files remain correct;
- no newer pending generation is overwritten;
- journal and required artifacts remain;
- the command exits nonzero with repair guidance;
- retry after removing the fault is idempotent.

## Acceptance criteria

- no pending result or error is discarded;
- no recovery path uses `let _ = ensure_pending...`;
- cleanup occurs only after durable pending finalization;
- repeated recovery produces at most one transaction-associated generation;
- a failed recovery never claims success.

---

# Workstream E — Correct failpoint boundaries

## Goal

Make failpoint names, production placement, test comments, and expected artifacts agree exactly.

## Required boundaries

Use stable names with these semantics:

1. `restore-after-prepared`
   - journal is `Prepared`;
   - no rollback backup or staged replacement is assumed durable;
   - no live write.

2. `restore-after-backups-durable`
   - all rollback backups and staged replacement files are synced and verified;
   - journal is `BackupsDurable`;
   - no live write.

3. `restore-after-first-install`
   - first live destination is installed, verified, and progress persisted.

4. `restore-after-index-install`
   - index destination is installed, verified, and progress persisted.

5. `restore-after-all-installs`
   - all live destinations are installed and verified;
   - state is still `Committing(all complete)`.

6. `restore-after-committed-local-before-pending`
   - state is `CommittedLocal(NotRecorded)`;
   - no transaction-associated pending marker has been created.

7. `restore-after-pending-before-journal-update`
   - canonical pending marker is durably created or reused;
   - journal still says `CommittedLocal(NotRecorded)`.

8. `restore-after-journal-pending-before-cleanup`
   - journal durably records `Recorded(g)` or `CoveredByExisting(g)`;
   - backups, staged files, and journal still exist.

9. `restore-during-first-rollback`
   - rollback state is durably initialized;
   - first rollback action has not completed.

10. `restore-during-second-rollback`
    - first rollback action completed and progress is persisted;
    - second rollback action has not completed.

Place each failpoint immediately after the exact preceding invariant and before the next operation.

Do not call a “before cleanup” failpoint after a cleanup function.

## Required artifact assertions

Each crash test must inspect:

- journal existence and exact state;
- backup file existence;
- staged file existence;
- live destination hashes;
- canonical pending marker and source transaction association;
- absence of pending files under `.transaction`;
- lock reclamation on restart;
- exact state after recovery;
- state after a second recovery run.

## Acceptance criteria

- failpoint names describe the observed on-disk state exactly;
- tests assert the state before recovery, not only the final result;
- no test comment contradicts production ordering;
- all ten failpoint tests execute substantive assertions.

---

# Workstream F — Add real crash-during-rollback tests

## Goal

Prove rollback can itself crash and resume from the correct rollback-order position.

## Required recoverable-error injection

Add a separate test-only error seam behind `test-support`. Do not use an abort failpoint to initiate rollback because abort bypasses handled rollback.

Example:

```rust
#[cfg(feature = "test-support")]
pub fn maybe_injected_error(name: &str) -> SnipResult<()> {
    if std::env::var("SNP_TEST_INJECT_ERROR").as_deref() == Ok(name) {
        return Err(SnipError::runtime_error(
            "Injected test failure",
            Some(name),
        ));
    }
    Ok(())
}

#[cfg(not(feature = "test-support"))]
pub fn maybe_injected_error(_name: &str) -> SnipResult<()> {
    Ok(())
}
```

Use it after enough live writes to require at least two rollback actions. Then activate an abort failpoint during rollback.

## Required scenarios

### First rollback action crash

1. establish at least two existing destinations with known bytes and modes;
2. start replace restore;
3. inject a handled error after the second live install;
4. enter rollback;
5. abort before the first rollback action completes;
6. restart recovery;
7. verify exact original bytes, permissions, index consistency, and no pending marker;
8. rerun recovery and verify idempotence.

### Second rollback action crash

1. use the same multi-file setup;
2. inject a handled commit error;
3. allow first rollback action to complete and persist progress;
4. abort before the second action;
5. restart recovery;
6. prove the completed first action is not corrupted or skipped incorrectly;
7. prove remaining actions complete exactly once.

## Required production proof

Build without `test-support` and set both valid `SNP_TEST_INJECT_ERROR` and rollback failpoint values. The normal build must ignore them.

## Acceptance criteria

- neither rollback test is empty or comment-only;
- both launch the real `snp` binary;
- both assert pre-crash journal cursor values;
- both assert exact byte and permission restoration;
- both prove a second recovery run is a no-op;
- production builds cannot activate the error seam.

---

# Workstream G — Use per-transaction artifact directories

## Goal

Make transaction cleanup complete, idempotent, and collision-free.

## Required layout

Prefer:

```text
<config>/.transaction/
  transaction.lock
  local-data.lock
  txn-<id>.toml
  artifacts/
    <id>/
      backups/
        0000.bak
      staged/
        0000.new
```

Requirements:

- artifact paths are derived from the journal ID;
- paths are stored explicitly in the journal;
- no transaction reuses another transaction’s numbered files;
- recovery validates that every artifact path remains contained within the expected transaction artifact root;
- symlinked artifact paths are rejected;
- cleanup removes files first, empty directories second, and the journal last;
- cleanup errors are propagated rather than discarded.

## Cleanup state

Do not mark cleanup complete before cleanup succeeds. Either:

- retain `CommittedLocal(Recorded(...))` until artifacts are removed, then remove the journal last; or
- add an explicit `CleanupPending` state.

Avoid terminal `Committed` plus ignored deletion failures.

## Required cleanup helper

```rust
pub fn finalize_transaction_cleanup(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<()> {
    validate_artifact_containment(transaction_dir, journal)?;
    remove_all_staged_files(journal)?;
    remove_all_backup_files(journal)?;
    remove_empty_transaction_artifact_dir(transaction_dir, &journal.id)?;
    fsync_transaction_dir(transaction_dir)?;
    remove_journal_last(transaction_dir, &journal.id)?;
    fsync_transaction_dir(transaction_dir)?;
    Ok(())
}
```

On Windows, account for delete-pending behavior with bounded retries. Do not convert an unresolved deletion into success.

## Required tests

- successful restore leaves no transaction-specific staged or backup artifact;
- handled rollback leaves no artifact after success;
- crash recovery leaves no artifact after successful finalization;
- injected cleanup failure preserves journal and remaining artifacts;
- retry completes cleanup;
- stale artifact directories without journals are reported by `repair` and not silently deleted;
- two sequential transactions cannot consume each other’s stale numbered files.

## Acceptance criteria

- no successful terminal path leaves `durable_staged_path` content;
- cleanup failures remain recoverable and visible;
- journal is removed last;
- no deletion error critical to recovery is ignored;
- `snp repair --dry-run` reports orphan artifact directories.

---

# Workstream H — Protect transaction artifacts and restore permissions

## Goal

Prevent plaintext transaction data from inheriting permissive defaults and preserve relevant destination permissions.

## Artifact permission policy

On Unix:

- `.transaction` and transaction artifact directories: `0700`;
- journals, lock files, backups, staged files: `0600`;
- apply permissions immediately after exclusive creation and before writing sensitive content when possible;
- strip setuid, setgid, and sticky bits from restored ordinary files;
- never create a world-readable staged or backup file even under a permissive umask.

On Windows:

- use the repository’s existing sensitive-file policy where available;
- at minimum avoid creating artifacts in a shared temporary directory;
- document the platform guarantee and test ordinary-user isolation that is feasible in CI.

## Journal metadata

Extend `StagedFile` with explicit destination metadata, for example:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OriginalFileMetadata {
    #[serde(default)]
    pub unix_mode: Option<u32>,
    #[serde(default)]
    pub readonly: Option<bool>,
}
```

Rules:

- capture metadata before live writes;
- include it in the durable journal before `BackupsDurable`;
- on rollback, restore content first, then metadata, then verify both;
- on commit replace, preserve the previous ordinary data-file mode unless policy requires a stricter mode;
- `sync.toml` must remain sensitive even if the source artifact or previous destination was more permissive;
- new snippet/library files use the repository’s normal private data mode.

## Verification

On Unix, reopen metadata and compare `mode & 0o777` to the expected sanitized value.

Do not claim full ACL or ownership preservation unless implemented. State the exact supported metadata contract.

## Required tests

- permissive umask does not produce permissive journal/staged/backup files;
- successful replace preserves expected library-file mode;
- rollback restores original mode;
- `sync.toml` remains private;
- setuid/setgid bits are not propagated;
- cleanup removes sensitive staging content;
- Windows readonly behavior is tested where relevant.

## Acceptance criteria

- artifact permission policy is encoded in helpers, not comments only;
- transaction metadata records the supported permission contract;
- commit and rollback verify metadata after content installation;
- documentation does not overclaim ACL/owner preservation.

---

# Workstream I — Enforce manifest semantics before artifact access

## Goal

Reject invalid backup contracts deterministically before source-file size, checksum, or content errors can mask the intended validation.

## Required validation pipeline

Use an explicit order:

```text
1. parse manifest syntax
2. validate manifest schema
3. validate layout
4. validate entry kinds and required cardinality
5. canonicalize and validate paths
6. detect exact and portable destination collisions
7. validate index/library relationships
8. validate entry size/hash field shape
9. inspect source artifact type and containment
10. verify source size
11. verify source checksum
12. parse domain content and validate duplicate snippet IDs
13. begin dry-run reporting or mutation
```

No transaction, lock, recovery, or live write may start before all validation succeeds.

## Schema and layout

Define constants:

```rust
const SUPPORTED_BACKUP_SCHEMA: u32 = 1;
const SUPPORTED_BACKUP_LAYOUT: &str = "directory";
```

Reject unsupported values with exact error classifications such as:

- `unsupported backup schema: 0`;
- `unsupported backup schema: 999`;
- `unsupported backup layout: archive`.

## Portable destination key

Derive a platform-independent collision key:

```rust
fn portable_destination_key(entry: &BackupManifestEntry) -> Result<String, ManifestError> {
    let normalized = normalize_separators(&entry.path);
    let logical = map_entry_to_destination(&normalized, entry.kind)?;
    let components = logical.components().map(|component| {
        component
            .to_string_lossy()
            .trim_end_matches(['.', ' '])
            .to_lowercase()
    });
    Ok(components.collect::<Vec<_>>().join("/"))
}
```

Use Unicode normalization only if the project already has an explicit dependency and contract. Otherwise document the limited normalization policy and reject ambiguous names conservatively.

Reject:

- exact duplicate destinations;
- ASCII case-fold collisions;
- slash/backslash aliases;
- trailing-dot/trailing-space aliases;
- Windows drive-relative forms such as `C:foo`;
- reserved device names;
- duplicate index, usage, or sync-config entries;
- libraries that map to the same filename.

## Index consistency

When an index entry is present:

- parse it during validation;
- reject duplicate library filenames case-insensitively;
- reject duplicate primary declarations if the domain allows only one;
- reject references to library files absent from the manifest in replace mode;
- reject manifest library files absent from the index when the index is authoritative;
- define merge-mode policy explicitly;
- reject duplicate incoming snippet IDs before checksum-independent mutation work begins.

## Required test fixture rule

Every negative fixture must be valid except for exactly one targeted fault.

Create shared builders that compute real sizes and SHA-256 values. Do not use `sha256 = "placeholder"` in schema, layout, collision, or cardinality tests.

Every test must assert:

- nonzero exit;
- exact error category or stable substring for the intended validation;
- no transaction journal;
- no pending marker;
- no live mutation.

The case-fold collision test must require rejection on every platform. It must not accept success.

## Required tests

At minimum:

- schema zero;
- future schema;
- unsupported layout;
- unknown kind;
- exact duplicate destination;
- case-fold duplicate destination;
- slash/backslash alias;
- trailing-dot alias;
- trailing-space alias;
- drive-relative path;
- UNC path;
- reserved device name;
- duplicate index entry;
- duplicate library filename in index;
- two primary libraries where forbidden;
- index references missing library;
- manifest library absent from authoritative index;
- duplicate snippet IDs;
- valid schema/layout succeeds.

## Acceptance criteria

- semantic validation occurs before artifact access;
- no named contract test can pass due to checksum failure;
- no test accepts either success or failure;
- error output identifies the intended contract;
- validation behavior is identical on Linux, macOS, and Windows.

---

# Workstream J — Add true barrier-controlled backup concurrency tests

## Goal

Prove backup sees a complete before-state or complete after-state while real writers are paused inside multi-file mutations.

## Test-only barrier design

Use a compile-time-gated barrier helper, for example:

```rust
#[cfg(feature = "test-support")]
pub fn mutation_barrier(point: &str) {
    let Ok(root) = std::env::var("SNP_TEST_MUTATION_BARRIER_DIR") else {
        return;
    };
    let root = PathBuf::from(root);
    let expected = root.join("point");
    if fs::read_to_string(&expected).ok().as_deref() != Some(point) {
        return;
    }
    fs::write(root.join("entered"), point).expect("write barrier entered");
    while !root.join("release").exists() {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(not(feature = "test-support"))]
pub fn mutation_barrier(_point: &str) {}
```

Use unique barrier directories per test. Apply a hard timeout. Clean up all child processes.

## Required barrier points

At minimum:

- library create: after library file creation, before index save;
- library delete: after index save, before file deletion;
- migration: after new library copy, before index save;
- snippet save: after temporary content is durable, before destination replacement if the atomic helper exposes such a point;
- sync config update: before replacement while lock is held;
- restore: after first installed destination while local-data lock remains held.

## Required test sequence

For each logical mutation:

1. establish exact before-state;
2. spawn the real feature-enabled `snp` writer with a barrier;
3. wait until `entered` exists;
4. start a real `snp backup` process;
5. prove backup does not complete while the writer holds `LocalDataLock`;
6. release the writer;
7. wait for both processes;
8. validate the backup is either exact before-state or exact after-state according to lock acquisition order;
9. reject any mixed index/library state;
10. repeat enough times to exercise both ordering outcomes where deterministic control permits.

Do not simulate writer behavior with direct `fs::write` outside production mutation paths.

## CI

Run `local_data_lock_barriers` in the transaction job on all three platforms. Keep `backup_snapshot_concurrency` if it proves a distinct contract; do not use it as a substitute.

## Acceptance criteria

- tests overlap two real processes;
- backup is observed waiting or failing busy while the writer owns the lock;
- each multi-file writer is covered;
- no test is merely backup → mutation → backup;
- production builds ignore barrier variables.

---

# Workstream K — Make remote acknowledgement own pending clear

## Goal

Make it impossible for a child process that merely exits `0` without remote acknowledgement to clear pending.

## Preferred ownership model

Move exact-generation pending clear into the executor, after the real sync protocol returns acknowledged success.

Required flow:

```text
worker:
  read pending generation G
  enforce debounce and execution lock
  spawn executor for G
  wait for child
  update lifecycle/backoff/status based on exit
  DO NOT clear pending

executor:
  load and validate pending generation G
  perform protocol sync
  receive remote acknowledgement/revision
  clear pending only with clear_if_generation_matches(G)
  report success only after clear result is handled
```

This is simpler and stronger than treating child exit status as remote acknowledgement.

If the current subprocess interface does not pass generation explicitly, use a non-secret argument such as `--generation <u64>` or let the executor reread pending under the execution lock. Generation is not sensitive. Preserve the exact-generation clear rule.

## Clear-result handling

After remote acknowledgement:

- `Cleared`: executor exits success;
- `GenerationChanged { current }`: do not clear newer work; report acknowledged old generation and exit in a documented coalesced-success state that causes another worker cycle;
- `Missing`: treat conservatively and record truthful status;
- pending I/O/integrity error: remote work may have succeeded, but local intent is unresolved; exit nonzero/attention and preserve recoverability.

Do not clear before remote acknowledgement.

## False-success seam

Under `test-support`, `noop-success` must exit `0` before protocol contact and before pending clear. Because the worker no longer owns clear, pending remains.

## Required end-to-end test

1. start a recording server;
2. configure a valid client and pending generation;
3. run the worker with `SNP_TEST_EXECUTOR_MODE=noop-success` in a feature-enabled binary;
4. assert executor exits `0`;
5. assert server request count is `0`;
6. assert pending generation remains exactly `G`;
7. assert status does not claim remote success;
8. assert no `sync_completed { success: true }` event exists;
9. remove the seam and retry;
10. assert real remote acknowledgement occurs and only then pending clears.

## Acceptance criteria

- worker code contains no pending clear based only on child success;
- executor clears only after protocol acknowledgement;
- false-success leaves pending intact;
- generation changes cannot cause a stale clear;
- network/auth/conflict/timeout/internal failures preserve pending;
- lifecycle and status remain factual.

---

# Workstream L — Use recording-server telemetry as release evidence

## Goal

Prove the exact remote interaction, not only a database row-count change.

## Recording model

Retain the recording handle returned by `start_test_server`. It should expose sanitized records such as:

```rust
pub struct RecordedSyncRequest {
    pub sequence: u64,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: u64,
    pub route: String,
    pub method: String,
    pub authenticated_device_id: Option<String>,
    pub authenticated_user_id: Option<String>,
    pub target_library_id: Option<String>,
    pub request_revision: Option<u64>,
    pub response_revision: Option<u64>,
    pub payload_len: usize,
    pub payload_sha256: String,
    pub payload_contains_plaintext_sentinel: bool,
    pub concurrent_at_start: usize,
}
```

Do not store API keys, decrypted snippet content, raw request bodies, or commands in telemetry.

## Headline assertions

For one local mutation:

- exactly one canonical sync request reaches the server;
- request authentication resolves to the expected device;
- request targets the expected library;
- payload is nonempty;
- payload does not contain a known plaintext command sentinel;
- response contains or causes a monotonic server revision transition;
- maximum server-side concurrency is `1`;
- server state changes from exact `R0` to exact `R1`;
- pending generation clears only after the recorded request finishes successfully;
- after a quiet period of at least the effective debounce plus a safety margin, no duplicate request appears.

Use event and server timestamps to establish ordering. Do not rely solely on sleeps.

## Required negative assertions

For auth failure, unreachable server, timeout, conflict, and false-success executor:

- no successful acknowledged request is recorded;
- pending remains;
- status classification is truthful;
- no duplicate worker/executor storm occurs.

## Acceptance criteria

- `_captured` is no longer discarded in release-blocking tests;
- local `sync.toml` is not used as proof of server-observed identity;
- server telemetry is sanitized;
- exact request and concurrency assertions pass on Linux, macOS, and Windows;
- quiet-period duplicate absence is deterministic.

---

# Workstream M — Correct CI without weakening evidence

## Goal

Produce same-commit cross-platform evidence for the corrected implementation.

## Required jobs

### 1. Static quality

- format;
- clippy with all targets and all features;
- architecture/source guards;
- dependency and package checks already required by the project.

### 2. Production seam

Linux and Windows minimum:

```text
cargo build --release --no-default-features --target-dir target/production-seam
run matching valid test-control variables against the no-feature binary
assert no test behavior activates
```

### 3. Release-blocking auto-sync

Linux, macOS, Windows:

- `deterministic_e2e`;
- `auto_sync_closure`;
- `auto_sync_lifecycle` where not already included;
- `executor_noop_success` through worker-level assertions;
- `sync_contracts`;
- `readonly_no_recovery`.

All commands must use `--features test-support` only when a test seam is required.

### 4. Transaction and backup

Linux, macOS, Windows:

- `restore_transactions`;
- `transaction_crash_recovery`;
- `restore_crash_failpoints`;
- `local_data_lock_barriers`;
- `backup_snapshot_concurrency` if retained;
- `manifest_contracts`;
- transaction artifact permission tests where supported.

### 5. General tests

Linux, macOS, Windows in dev and release profiles, without global worker suppression.

Where a suite does not need auto-sync, configure auto-sync disabled in its fixture rather than setting a production behavioral bypass.

### 6. PTY

Keep PTY tests isolated, bounded, and platform-specific. PTY instability must not be hidden by removing unrelated correctness tests.

## Workflow rules

- no `SNP_SKIP_WORKER_SPAWN` in repository-wide job environment;
- no test returns early because a release-blocking seam is unavailable;
- no `|| true`, `continue-on-error`, or permissive fallback in release gates;
- use explicit shells for platform-specific steps;
- avoid dynamic `eval` where a checked-in script can build test arguments safely;
- pin mutable third-party actions to full commit SHAs or document a precise exception;
- keep timeouts bounded and diagnose hangs rather than repeatedly increasing them;
- upload sanitized diagnostics on failure: test output, journal state, pending/status files, and lifecycle events with secrets excluded.

## Windows-specific checks

- verify feature-enabled `CARGO_BIN_EXE_snp` selection;
- verify no delete-pending race converts a real cleanup failure into success;
- verify transaction artifact cleanup with bounded retry;
- verify worker and executor processes are reaped;
- verify PowerShell package smoke remains portable;
- verify Git Bash assumptions are explicit where `shell: bash` is used;
- verify x86-64 and ARM64 linker stack settings remain valid if both targets are claimed;
- do not use PID values assumed dead; spawn and terminate a real child for stale-owner tests.

## Same-commit evidence

The final closure document must record for one exact commit:

- workflow run URL or run ID;
- every required job name;
- conclusion;
- OS and profile matrix;
- test counts if retained;
- any intentionally unsupported platform capability with a narrowly scoped explanation.

Do not combine evidence from different commits.

## Acceptance criteria

- all required jobs pass on one commit;
- no release-blocking suite is skipped on Windows;
- no global worker suppression is used;
- no test-support seam exists in production binary proof;
- workflow evidence is retrievable and recorded.

---

# Workstream N — Repository hygiene and final documentation

## Goal

Remove machine-local artifacts and make final claims match evidence.

## Required repository changes

- remove `.poolside/settings.local.yaml` from version control;
- add `.poolside/` or the narrow local-settings path to `.gitignore`;
- verify no other local agent permission/config files are tracked;
- verify no test credential, event log, journal, staged file, or backup fixture escaped into the repository;
- update architecture documentation for the final pending-clear owner and transaction cleanup protocol;
- update threat-model text for test seam compile-time isolation and transaction artifact confidentiality.

## Final closure status

Only after all CI gates pass, update `plans/snip-it-correctness-11-closure-status.md` with:

- exact final commit;
- exact CI run evidence;
- concise implementation summary;
- resolved workstream table;
- remaining nonblocking limitations;
- explicit release decision.

Do not state “complete” based only on local tests or workflow configuration.

## Acceptance criteria

- machine-local config is absent from the tree;
- documentation describes executor-owned pending clear if that model is selected;
- test seam claims match compile-time behavior;
- transaction artifact cleanup and permission guarantees are precise;
- the final decision is evidence-based.

---

## 4. Cross-cutting implementation rules

### 4.1 Preserve exact-generation semantics

Never clear pending without comparing the observed generation. A newer generation must survive a stale executor or recovery process.

### 4.2 Preserve pending on uncertainty

Pending must remain on:

- config failure;
- credential failure;
- authentication failure;
- network failure;
- timeout;
- conflict;
- local integrity failure;
- transaction-finalization failure;
- child spawn failure;
- false-success test executor;
- status persistence failure where remote acknowledgement cannot be proven locally.

### 4.3 Do not discard critical results

Prohibit patterns such as:

```rust
let _ = ensure_pending_for_transaction(...);
let _ = remove_recovery_artifact(...);
let _ = persist_required_state(...);
```

Best-effort deletion is acceptable only for nonessential diagnostics after recovery authority has been safely removed. Document each such case.

### 4.4 Keep logs and evidence secret-safe

Never log or persist:

- API keys;
- raw authorization headers;
- snippet commands;
- decrypted payloads;
- raw encrypted request bodies when a hash/length is sufficient;
- credential file contents;
- arbitrary environment dumps.

### 4.5 Use exact test assertions

Avoid:

- success-or-failure acceptance;
- broad error matching that includes unrelated checksum errors;
- sleeps as the sole synchronization mechanism;
- empty tests;
- comment-only “coverage”;
- nonmatching test variables as proof of production isolation;
- direct file writes as substitutes for production mutation paths.

### 4.6 Keep the architecture lightweight

The accepted solution should consist of:

- one installed `snp` binary;
- one-shot worker and executor invocations;
- short-lived lock files;
- bounded transaction artifacts;
- TOML journals and pending state;
- compile-time test instrumentation.

No additional service is required.

---

## 5. Recommended implementation sequence

Use small commits with one invariant per commit. A recommended sequence is:

1. `docs: reopen Phase 11 for 11E corrective closure`
   - update closure status only.

2. `test-support: restore compile-time gating for lifecycle events`
   - gate event sink and add no-feature proof.

3. `test-support: gate failpoints and injected errors`
   - feature-gated abort/error seams.

4. `test-support: gate executor modes and worker suppression`
   - remove all production runtime bypasses.

5. `tests: use the feature-enabled Cargo binary explicitly`
   - repair helpers and CI commands.

6. `transaction: replace pending generation sentinel with typed finalization`
   - journal migration and serialization tests.

7. `transaction: make CommittedLocal recovery fail closed`
   - explicit result handling and conflict policy.

8. `transaction: correct pending finalization failpoint boundaries`
   - exact ordering and artifact-state assertions.

9. `transaction: add recoverable commit error injection`
   - test-support only.

10. `transaction: implement crash-during-rollback subprocess tests`
    - first and second rollback positions.

11. `transaction: move artifacts under per-transaction roots`
    - containment validation.

12. `transaction: make cleanup complete and retryable`
    - staged, backup, directories, journal-last removal.

13. `security: apply private transaction artifact permissions`
    - directory and file policy.

14. `transaction: preserve and verify destination metadata`
    - Unix modes and supported Windows attributes.

15. `restore: add pre-artifact manifest semantic validation`
    - schema, layout, cardinality, collisions, index relationships.

16. `tests: replace manifest fixtures with valid single-fault builders`
    - exact classifications.

17. `tests: add real local-data lock barriers`
    - overlapping writer and backup processes.

18. `auto-sync: move pending clear behind remote acknowledgement`
    - executor ownership and worker refactor.

19. `tests: prove false-success cannot clear pending`
    - worker-level test.

20. `tests: assert recording-server telemetry and quiet period`
    - exact request evidence.

21. `ci: run 11E release gates on Linux macOS and Windows`
    - production seam, auto-sync, transaction, barriers, package.

22. `chore: remove local agent settings and reconcile documentation`
    - hygiene and architecture docs.

23. `docs: record same-commit Phase 11 closure evidence`
    - only after all jobs pass.

Do not squash implementation into one opaque commit before review. The final integration may be squashed later according to repository policy, but the handoff execution should preserve auditable boundaries.

---

## 6. Required verification commands

Run from repository root.

### Formatting and static analysis

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-features
cargo build --workspace --release --no-default-features
```

### Focused transaction and restore

```bash
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
```

### Auto-sync and remote acknowledgement

```bash
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test auto_sync_lifecycle --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
```

### Production seam

Provide a checked-in script or equivalent commands:

```bash
scripts/ci/test-production-seams.sh
```

Windows equivalent:

```powershell
pwsh -File scripts/ci/test-production-seams.ps1
```

Both must use a binary built without `test-support` and matching valid seam values.

### Full suites

```bash
cargo test --workspace --all-features -- --test-threads=1
cargo test --workspace --all-features --release -- --test-threads=1
```

Run equivalent jobs on Linux, macOS, and Windows.

---

## 7. Mandatory test matrix

| Contract | Linux | macOS | Windows | Required evidence |
|---|---:|---:|---:|---|
| Production ignores matching failpoint | Yes | Optional | Yes | no abort, normal behavior |
| Production ignores noop-success | Yes | Optional | Yes | real executor path used |
| Production ignores worker suppression | Yes | Optional | Yes | worker starts when policy requires |
| Pending finalization retry | Yes | Yes | Yes | one generation, journal preserved on error |
| Pending lock failure during recovery | Yes | Yes | Yes | nonzero, evidence retained |
| Crash after pending before journal update | Yes | Yes | Yes | exact pre-recovery journal state |
| Crash after journal update before cleanup | Yes | Yes | Yes | artifacts remain until recovery |
| Crash during first rollback action | Yes | Yes | Yes | exact rollback cursor and final bytes |
| Crash during second rollback action | Yes | Yes | Yes | no skipped remaining action |
| Staged/backup artifact cleanup | Yes | Yes | Yes | no sensitive residue |
| Artifact private permissions | Yes | Yes | Platform contract | metadata assertions |
| Destination permission restoration | Yes | Yes | Platform contract | content and metadata |
| Unsupported schema/layout | Yes | Yes | Yes | exact semantic error before checksum |
| Portable destination collision | Yes | Yes | Yes | unconditional rejection |
| Index/library consistency | Yes | Yes | Yes | exact domain error |
| Real backup/mutation barrier | Yes | Yes | Yes | concurrent processes and coherent snapshot |
| False-success executor preserves pending | Yes | Yes | Yes | exit 0, zero requests, pending retained |
| Recording-server exact request | Yes | Yes | Yes | identity, target, payload, revision |
| Quiet-period no duplicate | Yes | Yes | Yes | request count remains one |
| Package install smoke | Yes | Yes | Yes | packaged binary version/help |

“Platform contract” must be documented precisely. Do not mark an unsupported guarantee complete by silently skipping it.

---

## 8. Release-blocking acceptance checklist

### Status truth

- [ ] Phase 11 is marked incomplete during implementation.
- [ ] Phase 11E is named as the blocking plan.
- [ ] No stale test count or final commit is presented as current.
- [ ] Closure is not declared before same-commit CI evidence.

### Test-support boundary

- [ ] Matching failpoint environment values cannot affect a no-feature binary.
- [ ] Matching executor test mode cannot affect a no-feature binary.
- [ ] Worker suppression cannot affect a no-feature binary.
- [ ] Test event paths cannot affect a no-feature binary.
- [ ] Mutation barriers and injected errors cannot affect a no-feature binary.
- [ ] Feature-enabled integration tests invoke the correct binary.

### Pending finalization

- [ ] Unknown pending generation is represented explicitly.
- [ ] `CommittedLocal` recovery handles every pending result and error.
- [ ] Pending failure preserves journal and artifacts.
- [ ] One transaction creates at most one associated generation.
- [ ] Unrelated pending work is never overwritten.
- [ ] Scheduling existing pending does not increment generation.

### Failpoint correctness

- [ ] All ten failpoints occur at the documented boundary.
- [ ] Tests inspect exact pre-recovery artifacts and state.
- [ ] First rollback crash is substantive.
- [ ] Second rollback crash is substantive.
- [ ] Second recovery pass is idempotent.

### Artifact cleanup and confidentiality

- [ ] Artifacts use per-transaction directories.
- [ ] Staged and backup files are private.
- [ ] Successful commit removes all staged/backup content.
- [ ] Successful rollback removes all staged/backup content.
- [ ] Recovery cleanup removes all staged/backup content.
- [ ] Cleanup failure preserves the journal.
- [ ] Journal is removed last.
- [ ] `repair --dry-run` reports orphan artifact roots.

### Permission correctness

- [ ] Supported original metadata is recorded before live writes.
- [ ] Commit applies the documented mode policy.
- [ ] Rollback restores original supported metadata.
- [ ] `sync.toml` remains sensitive.
- [ ] Tests do not claim unsupported ACL/ownership preservation.

### Manifest correctness

- [ ] Schema and layout validate before artifact access.
- [ ] Exact and portable destination collisions are rejected.
- [ ] Entry cardinality is enforced.
- [ ] Index/library relationships are enforced.
- [ ] Duplicate snippet IDs are rejected.
- [ ] Every negative fixture has correct hashes and one fault.
- [ ] No case-fold test accepts success.

### Backup coherence

- [ ] Every backup-visible writer holds `LocalDataLock` for its entire logical mutation.
- [ ] Barrier tests overlap real processes.
- [ ] Backup cannot observe a mixed index/library state.
- [ ] Restore and backup coordinate through the same lock hierarchy.
- [ ] CI executes the actual barrier suite on all platforms.

### Remote acknowledgement

- [ ] Worker does not clear pending based only on child exit status.
- [ ] Executor clears exact generation only after protocol acknowledgement.
- [ ] False-success executor leaves pending intact.
- [ ] Network/auth/conflict/timeout failures preserve pending.
- [ ] Server telemetry proves exact request identity and target.
- [ ] Payload evidence is sanitized and proves no plaintext sentinel.
- [ ] Server-side concurrency is at most one.
- [ ] Quiet-period request count remains one.

### CI and release evidence

- [ ] Linux dev and release suites pass.
- [ ] macOS dev and release suites pass.
- [ ] Windows dev and release suites pass.
- [ ] Release-blocking auto-sync suites pass on all three platforms.
- [ ] Transaction/failpoint/barrier suites pass on all three platforms.
- [ ] Production-seam proof passes without `test-support`.
- [ ] Package smoke passes on all three platforms.
- [ ] No permissive workflow step exists.
- [ ] All evidence comes from one final commit.

### Hygiene and documentation

- [ ] `.poolside/settings.local.yaml` is removed.
- [ ] Local agent settings are ignored.
- [ ] Architecture docs describe final pending-clear ownership.
- [ ] Security docs describe compile-time test isolation.
- [ ] Closure status matches actual code and CI.

---

## 9. Stop conditions

Stop implementation and keep the program open if any of the following occurs:

- production behavior still changes when a test-only environment variable is set;
- integration tests cannot reliably select a feature-enabled binary;
- `CommittedLocal` recovery can delete evidence after a pending error;
- any failpoint test passes without reaching the named boundary;
- rollback crash tests remain placeholders;
- staged plaintext remains after successful finalization;
- artifact permissions depend only on umask;
- a manifest contract test can fail first on checksum or parse noise;
- a collision test accepts success;
- concurrency tests remain sequential;
- pending clear still depends solely on child exit status;
- recording-server telemetry is discarded;
- a release-blocking Windows suite is skipped;
- CI evidence is assembled from different commits;
- closure documentation claims more than executable evidence proves.

Do not work around a stop condition by weakening an assertion, increasing a timeout without diagnosis, adding a permissive skip, or moving test-only behavior into production.

---

## 10. Handoff instructions for the implementation agent

1. Read this plan and the current closure status before editing code.
2. Confirm the implementation baseline is `52563f9dcdc1c4bb681e3ce6f5d8404a0957fb22` or document intervening commits.
3. Make the status-truth commit first.
4. Restore compile-time test isolation before relying on any new failpoint or barrier result.
5. Repair test binary selection rather than removing feature gates.
6. Correct `CommittedLocal` recovery before changing cleanup behavior.
7. Correct failpoint placement before interpreting crash-test results.
8. Implement real rollback entry and rollback crashes before marking crash closure complete.
9. Make cleanup and permission guarantees production code, not test cleanup.
10. Validate manifest semantics before artifact access.
11. Use production mutation paths in barrier tests.
12. Move pending-clear authority behind real remote acknowledgement.
13. Retain and assert recording-server telemetry.
14. Run focused suites after each workstream.
15. Run full Linux, macOS, and Windows CI on one candidate commit.
16. Update closure documentation only after all jobs pass.
17. If any release-blocking criterion remains unsupported, leave Phase 11 open and list the exact gap.

The correct outcome is not the largest change set. The correct outcome is a small, auditable closure pass in which production behavior is free of test controls, committed local data cannot lose pending intent, transaction artifacts remain confidential and recoverable, and every release claim is backed by exact cross-platform evidence.

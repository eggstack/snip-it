# Phase 11D — Pending Finalization, Durable Staging, Backup Coherence, and Cross-Platform Proof Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `9982b955830b6b79dce54a06a2c43bd93fd037be`

Parent plans:

- `plans/snip-it-correctness-11-verification-and-crash-closure.md`
- `plans/snip-it-correctness-11b-durability-verification-windows-ci-closure.md`
- `plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This is the authoritative follow-up for the defects that remain after the partial Phase 11C implementation. It is intentionally narrow. It does not reopen work that is already materially correct, including process-start identity observation, live-owner lock refusal, stale-lock reclaim through exclusive creation, restartable rollback-order cursors, typed manifest entry kinds, native ZIP extraction, operation-aware read-only recovery classification, and unified ordinary/output-file execution result mapping.

Phase 11 and the correctness program must remain open until every release-blocking criterion in this plan is demonstrated by production code, adversarial tests, and successful Linux, macOS, and Windows GitHub Actions jobs on the same final commit.

---

## 1. Objective

Close the remaining correctness and proof gaps without expanding product scope:

1. write restore pending intent to the canonical state directory, never `.transaction/`;
2. record exactly one pending generation for one successful restore;
3. schedule an already-recorded generation without incrementing it again;
4. make transaction finalization idempotent across crashes at every boundary;
5. build and verify complete durable staged replacement files before live writes;
6. synchronize backup and staging files before claiming `BackupsDurable`;
7. verify hashes from installed destinations, not only source buffers;
8. preserve and restore relevant file permissions where supported;
9. add real process-kill failpoint tests around production restore;
10. coordinate backup with every writer of backup-visible state;
11. replace sequential “concurrency” tests with barrier-controlled concurrent tests;
12. enforce manifest schema, layout, portable destination collisions, and index/library consistency before transaction creation;
13. replace permissive or invalid negative fixtures with otherwise-valid targeted fixtures;
14. use recording-server telemetry for exact request, identity, payload, revision, concurrency, and quiet-period evidence;
15. add a true false-success executor seam that exits successfully without server work;
16. compile-time gate or remove all CI-only behavioral bypasses from production builds;
17. run release-blocking auto-sync evidence on Windows instead of excluding it;
18. separate slow PTY infrastructure from correctness jobs without weakening release gates;
19. remove machine-local agent configuration from the repository;
20. reconcile closure documentation only after same-commit CI evidence exists.

---

## 2. Non-goals and architectural constraints

Preserve all of the following:

- one installed client binary: `snp`;
- auto-sync workers remain one-shot subprocesses;
- no resident client daemon;
- no second installed helper binary;
- no database replacing TOML state;
- no plugin runtime;
- no workflow engine;
- no distributed transaction protocol;
- no CRDT expansion;
- no broad CLI redesign;
- no platform-specific public command semantics.

Allowed internal additions:

- an idempotent transaction-associated pending-intent API;
- richer pending on-disk metadata with backward-compatible parsing;
- durable staged files under the transaction directory;
- file permission metadata where supported;
- test-only restore failpoints behind `test-support`;
- test-only executor modes behind `test-support`;
- recording-server request telemetry;
- checked-in CI helper scripts;
- guard-required internal mutation APIs.

Prefer one coherent implementation path over compatibility wrappers that retain incorrect behavior.

---

## 3. Confirmed baseline defects

The implementation agent must treat the following as defects, not optional cleanup.

### 3.1 Restore records pending under `.transaction/`

The restore command derives:

```text
transaction_dir = <config>/.transaction
```

and passes `transaction_dir` to `pending::record_pending_mutation`. That writes:

```text
<config>/.transaction/auto-sync-pending.toml
```

instead of the canonical:

```text
<config>/auto-sync-pending.toml
```

The restore path then calls `notify_mutation`, which records another pending mutation in the canonical state directory. One successful restore can therefore create an orphan marker plus a second canonical generation.

### 3.2 `CommittedLocal` recovery checks the wrong pending marker

Transaction recovery receives the transaction directory and calls pending helpers relative to that directory. It can clean up a transaction even when canonical pending intent was never durably recorded.

### 3.3 Pending finalization is not idempotent

The current flow records pending before persisting `CommittedLocal`, then calls a notification API that records pending again. It does not provide a transaction-associated idempotency key that allows recovery to answer:

- was this restore already represented by a pending generation?
- which generation belongs to this transaction?
- should recovery create the marker, reuse it, or refuse due to conflicting state?

### 3.4 Durable staging is declared but unused

`StagedFile::durable_staged_path` exists, but restore does not populate or consume it. `new_hash` and staged hashes are not complete before the live-write phase. Incoming or merged bytes are read and written directly to live destinations.

### 3.5 `BackupsDurable` is stronger than the actual protocol

Restore uses `fs::copy` for rollback backups and then persists `BackupsDurable`. It does not explicitly sync each backup file, verify its hash from disk, or sync the containing backup directory before the state transition.

### 3.6 Commit verification hashes the wrong object

Commit progress is advanced after an atomic write, but the code does not consistently reopen and hash the installed destination. Rollback similarly hashes the backup buffer rather than the destination after replacement.

### 3.7 Real crash behavior is unproven

The transaction crash suite mostly writes synthetic journals. It does not kill a real restore subprocess at production failpoints and verify exact recovery.

### 3.8 Backup-visible writers remain outside one lock protocol

Normal snippet saves and sync settings participate in `LocalDataLock`; library create, delete, migration, index updates, and some other writers do not hold the lock across their complete logical mutation.

### 3.9 Backup concurrency tests are sequential

The current suite performs backup, direct file mutation, then another backup. It does not force a writer to pause inside a multi-file mutation while backup attempts capture.

### 3.10 Manifest validation tests can pass for the wrong reason

Several schema and duplicate-destination fixtures use invalid placeholder hashes. Case-fold collision tests accept success. These tests do not prove the named validation rule.

### 3.11 Recording-server telemetry is discarded

The headline tests discard the server recording handle and use database row count plus local config assertions. They do not prove exact request count, server-observed identity, target library, encrypted payload presence, revision transition, maximum concurrency, or duplicate-request absence.

### 3.12 The false-success executor test is not a false-success executor test

The current “no-op” regression points the client at an unreachable server. That proves normal network failure preserves pending, not that an executor which exits `0` without syncing cannot clear pending.

### 3.13 `SNP_SKIP_WORKER_SPAWN` changes production behavior

The variable is checked in normal production code. When set, scheduling can report `SpawnNow` while no worker is spawned. This is an untruthful production-accessible behavioral bypass.

### 3.14 Windows release evidence excludes important paths

The CI workflow excludes deterministic E2E and sync contract suites on Windows. General workspace jobs suppress worker spawn. The workflow therefore does not prove the worker/executor lifecycle on Windows.

### 3.15 Closure documentation overstates the repository state

The status document marks all Phase 11C workstreams complete, references stale commits and counts, and claims only Windows evidence remains even though production correctness defects are still open.

---

# Workstream A — Reopen closure evidence accurately

## Goal

Make the repository status truthful before additional implementation begins.

## Required first commit

Update `plans/snip-it-correctness-11-closure-status.md` to state:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11d-pending-staging-and-cross-platform-proof-closure.md
Corrective baseline: 9982b955830b6b79dce54a06a2c43bd93fd037be
```

List the open workstreams from this plan. Remove or mark superseded claims that:

- pending finalization is atomic and complete;
- durable staging is implemented;
- all backup-visible writers coordinate;
- manifest tests are strict;
- server telemetry is complete;
- a true false-success executor seam exists;
- all Phase 11C criteria are complete;
- only Windows CI remains.

## Closure criteria

- no status claim says only CI remains;
- no Phase 11D item is pre-marked complete;
- prior evidence remains available but is labeled historical or superseded;
- test counts are not presented as current unless regenerated on the final commit.

---

# Workstream B — Separate canonical state and transaction directories

## Goal

Eliminate path confusion between auto-sync state and transaction artifacts.

## Required model

Use distinct typed or named paths throughout restore and recovery:

```rust
pub struct RestorePaths {
    pub config_dir: PathBuf,
    pub sync_state_dir: PathBuf,
    pub transaction_dir: PathBuf,
}

impl RestorePaths {
    pub fn discover() -> Self {
        let config_dir = get_config_dir();
        Self {
            sync_state_dir: config_dir.clone(),
            transaction_dir: config_dir.join(".transaction"),
            config_dir,
        }
    }
}
```

Exact names may differ. Equivalent separation is mandatory.

Rules:

- pending marker, status file, execution lock, worker events, and sync config live relative to `sync_state_dir`;
- transaction journals, transaction lock, local-data lock, durable backups, and durable staged files live relative to `transaction_dir`;
- no generic parameter named only `state_dir` may be passed across both domains without a wrapper type or explicit variable name;
- transaction recovery APIs that need pending state must receive both directories explicitly.

## Required refactor targets

At minimum review and update:

- `src/commands/restore_cmd.rs`;
- `src/transaction.rs`;
- `src/auto_sync/pending.rs`;
- `src/auto_sync/notification.rs`;
- repair/recovery integration;
- tests that construct `.transaction` paths manually.

## Required tests

1. successful restore creates no `auto-sync-pending.toml` under `.transaction`;
2. successful restore creates exactly one canonical pending marker when sync is configured;
3. failed or rolled-back restore creates no canonical pending marker;
4. no-op restore creates no pending marker;
5. repair and transaction recovery inspect `.transaction` while pending recovery inspects the config root;
6. a pre-existing orphan marker under `.transaction` is ignored or reported as repairable legacy debris, never treated as canonical work.

## Closure criteria

- pending APIs never receive `transaction_dir`;
- transaction APIs never infer canonical sync state by calling `parent()` implicitly;
- directory roles are explicit in production signatures;
- no successful path leaves a pending file inside `.transaction`.

---

# Workstream C — Add idempotent transaction-associated pending intent

## Goal

Guarantee that committed restore content is represented by exactly one durable pending generation across crashes and retries.

## Required pending metadata

Extend the pending on-disk model in a backward-compatible way, for example:

```rust
#[derive(Serialize, Deserialize)]
struct PendingOnDisk {
    schema: u32,
    generation: u64,
    created_at_unix_ms: u64,
    snapshot: PendingSnapshot,
    #[serde(default)]
    source_transaction_id: Option<String>,
    integrity: String,
}
```

The exact field name may differ. Requirements:

- old pending records without the field still parse;
- integrity covers the transaction association;
- no secret or snippet content is stored;
- one transaction ID can resolve to at most one generation;
- a different existing transaction ID is not silently overwritten.

## Required API

Add an idempotent API, for example:

```rust
pub enum TransactionPendingResult {
    Created(PendingState),
    Reused(PendingState),
    Conflict(PendingState),
}

pub fn ensure_pending_for_transaction(
    sync_state_dir: &Path,
    transaction_id: &str,
    snapshot: PendingSnapshot,
) -> Result<TransactionPendingResult, PendingError>;
```

Semantics:

1. acquire the pending transaction guard;
2. read the current canonical marker;
3. if it already names the same transaction, return its generation without incrementing;
4. if no marker exists, create one generation and associate it with the transaction;
5. if a marker exists for unrelated newer work, preserve it and return a conflict that recovery can handle conservatively;
6. never clear or replace a newer generation.

## Required transaction state

Represent finalization without guessing. One acceptable model:

```rust
CommittedLocal {
    pending_generation: Option<u64>,
    pending_recorded: bool,
}
```

Protocol:

1. after all destinations are installed and verified, persist `CommittedLocal { pending_generation: None, pending_recorded: false }`;
2. call `ensure_pending_for_transaction(sync_state_dir, journal.id, snapshot)`;
3. persist `CommittedLocal { pending_generation: Some(g), pending_recorded: true }`;
4. commit/clean transaction artifacts;
5. schedule the existing pending generation without recording another mutation.

A crash at step 1 is recovered by idempotently creating/reusing the marker. A crash after step 2 but before step 3 reuses the same generation. A crash after step 3 but before cleanup finalizes cleanup without another increment.

## Scheduling API

Do not call `notify_mutation` after pending has already been recorded. Add or use an API whose only responsibility is scheduling existing work:

```rust
pub fn schedule_existing_pending(
    sync_state_dir: &Path,
    policy: &AutoSyncPolicy,
    caller: Caller,
) -> ScheduleDecision;
```

It must not mutate pending state.

## Conflict policy

When an unrelated newer pending generation exists during transaction recovery:

- do not overwrite or clear it;
- if the existing marker already encompasses the restored state under the product’s latest-state sync semantics, record that relationship explicitly and finalize safely;
- otherwise preserve both facts through repair metadata or return a clear nonzero recovery error;
- never create an unbounded queue or workflow engine.

Document the chosen policy and test it.

## Required tests

- one successful restore produces exactly one generation increment;
- restore never calls the ordinary mutation notification recorder after transaction pending is established;
- crash before pending creation creates one generation on recovery;
- crash after marker creation but before journal update reuses the same generation;
- crash after journal update but before cleanup does not increment;
- two recovery attempts remain idempotent;
- unrelated newer pending generation is preserved;
- failed/rolled-back/no-op restore creates no generation;
- orphan `.transaction/auto-sync-pending.toml` has no effect.

## Closure criteria

- one restore equals one pending generation;
- recovery can always identify whether its generation already exists;
- no code path records pending twice for restore;
- scheduling reports truthfully and never increments pending.

---

# Workstream D — Build complete durable staged artifacts before live writes

## Goal

Make `BackupsDurable` a factual statement: every rollback and commit artifact required for recovery exists, is synchronized, and is verified before the first destination changes.

## Required transaction entry

Each file entry must contain enough information to execute and recover independently:

```rust
pub struct StagedFile {
    pub original_path: PathBuf,
    pub action: StagedAction,
    pub existed_before: bool,
    pub original_hash: Option<Sha256Digest>,
    pub intended_hash: Option<Sha256Digest>,
    pub backup_path: Option<PathBuf>,
    pub durable_staged_path: Option<PathBuf>,
    pub original_permissions: Option<PortablePermissions>,
    pub durability: FileDurability,
}
```

Exact types may differ. Avoid empty-string sentinel hashes in new journals.

## Preparation order

All manifest and domain validation must complete before creating transaction artifacts. Then:

1. acquire `LocalDataLock`;
2. re-read and revalidate any local inputs used to compute merge results;
3. acquire `TransactionLock` in the documented order;
4. compute the complete intended bytes for every destination;
5. create a transaction-private directory such as `.transaction/txn-<id>/`;
6. write rollback backups for existing destinations;
7. write intended replacement bytes to durable staged files;
8. call `sync_all` on each staged/backup file;
9. reopen each artifact and verify its expected hash;
10. sync transaction artifact directories where supported;
11. persist the fully populated journal;
12. persist `BackupsDurable`;
13. begin live replacement.

No destination may be modified before step 12 completes.

## Merge restore requirement

Merge mode must not compute and write merged bytes inside the live commit loop. Compute the merged library once during preparation, serialize it deterministically, validate it, hash it, and place it in the durable staging area.

Example:

```rust
let merged = merge_libraries(&existing, &incoming)?;
validate_library(&merged)?;
let bytes = toml::to_string_pretty(&merged)?.into_bytes();
let stage = txn_dir.join(format!("{position}.new"));
write_sync_verify(&stage, &bytes)?;
entry.intended_hash = Some(sha256(&bytes));
entry.durable_staged_path = Some(stage);
```

The live commit loop should not need to parse or merge TOML.

## Durability helpers

Provide focused helpers with explicit semantics:

```rust
fn write_sync_verify(path: &Path, bytes: &[u8]) -> SnipResult<Sha256Digest>;
fn copy_sync_verify(src: &Path, dst: &Path) -> SnipResult<Sha256Digest>;
fn sync_parent_dir(path: &Path) -> SnipResult<()>;
```

Platform differences may be encapsulated, but failures must not be silently ignored for release-critical user data.

## Required tests

- journal contains nonempty typed hashes for every replace/create action;
- every replacement has a distinct durable staged path before commit;
- every existing destination has a verified backup path;
- `BackupsDurable` is not persisted if any sync or verification fails;
- merge output is staged before the first live write;
- corrupt staged content is detected before commit;
- corrupt backup content blocks live writes;
- no staged path aliases a live destination;
- transaction artifacts contain no API keys or raw command-line arguments beyond necessary restored file bytes.

## Closure criteria

- `durable_staged_path` is populated and consumed by production restore;
- intended hashes are complete before live writes;
- backup and stage files are verified from disk;
- `BackupsDurable` is persisted only after all artifacts are ready.

---

# Workstream E — Commit from durable staging and verify installed destinations

## Goal

Make commit progress represent completed, verified destination state.

## Required commit loop

For each planned entry at `next_commit_position`:

1. read or move from the durable staged artifact;
2. install via the appropriate atomic replacement primitive;
3. restore intended permissions if applicable;
4. reopen the live destination;
5. hash the live destination and compare with `intended_hash`;
6. sync the destination and parent directory according to durability policy;
7. persist `next_commit_position = position + 1`.

Example:

```rust
for position in journal.next_commit_position()..journal.files.len() {
    install_one(&journal.files[position])?;
    verify_installed_destination(&journal.files[position])?;
    journal.state = TransactionState::Committing {
        next_commit_position: position + 1,
    };
    persist_journal(&journal)?;
}
```

Do not hash only the source buffer. Do not advance progress before verification.

## Replay behavior

Recovery from `Committing { next_commit_position: N }` must be deterministic:

- positions `< N` must already match intended state or recovery fails closed;
- position `N` and later may be installed from durable staging;
- replaying an already-installed matching destination is idempotent;
- an unexpected destination hash preserves evidence and requires repair rather than guessing.

The project may choose rollback instead of roll-forward for interrupted commit, but the choice must be consistent, documented, and supported by complete artifacts. Do not automatically roll back a `CommittedLocal` transaction.

## Required tests

- crash after first verified install resumes without duplicating pending;
- a destination mismatch at a supposedly completed position fails closed;
- commit progress never skips an unverified destination;
- replay is idempotent;
- staged files remain available until pending finalization completes;
- destination hash is measured from the installed file.

## Closure criteria

- commit consumes durable staging;
- completed positions are verified from live destinations;
- progress is persisted after verification;
- crash replay cannot silently accept divergent destination bytes.

---

# Workstream F — Complete rollback verification and permission restoration

## Goal

Restore exact pre-transaction state and prove it from live destinations.

## Required behavior

For each rollback-order position:

- `Replace`: atomically restore verified backup bytes;
- `Create`: remove the created destination;
- `Delete`: recreate from verified backup bytes;
- `NoOp`: verify the destination still matches expected pre-state or skip only when explicitly safe.

After each action:

- reopen and hash the live destination, or verify absence;
- restore recorded permissions where supported;
- sync according to durability policy;
- persist `next_rollback_position = position + 1`.

Do not compute the verification hash from the backup buffer.

## Permissions

Record portable permission information that matters to product correctness:

- Unix mode bits needed to avoid unexpectedly executable or world-readable files;
- Windows read-only state if meaningful;
- sensitive config should return to private permissions.

Do not attempt to replicate full ACLs unless the repository already supports them.

## Failure policy

Missing or corrupt backup/staged artifacts must:

- return nonzero;
- preserve the journal and remaining artifacts;
- avoid deleting evidence;
- direct the user to `snp repair`;
- never mark the transaction rolled back.

## Required tests

- replace rollback restores exact original destination bytes;
- create rollback removes the destination;
- delete rollback restores the destination;
- destination hash is measured after rollback;
- Unix permissions are restored;
- sensitive config permissions remain private;
- crash during rollback resumes at the correct rollback-order position;
- second crash during rollback remains recoverable;
- corrupt backup preserves artifacts and returns nonzero;
- rollback creates no pending generation.

## Closure criteria

- verification reads installed destinations;
- permissions are restored where supported;
- artifact failures preserve evidence;
- rollback remains restartable after repeated interruption.

---

# Workstream G — Add real process-crash failpoints and subprocess tests

## Goal

Prove the production transaction protocol rather than synthetic journal detection alone.

## Test-only boundary

Failpoints must compile only with `test-support`:

```rust
#[cfg(feature = "test-support")]
fn maybe_failpoint(name: &str) {
    if std::env::var("SNP_TEST_FAILPOINT").as_deref() == Ok(name) {
        std::process::abort();
    }
}

#[cfg(not(feature = "test-support"))]
fn maybe_failpoint(_name: &str) {}
```

A production build must ignore `SNP_TEST_FAILPOINT` entirely.

## Required failpoints

At minimum:

- `restore-after-prepared`;
- `restore-after-backups-durable`;
- `restore-after-first-install`;
- `restore-after-index-install`;
- `restore-after-all-installs`;
- `restore-after-committed-local-before-pending`;
- `restore-after-pending-before-journal-update`;
- `restore-after-journal-pending-before-cleanup`;
- `restore-during-first-rollback`;
- `restore-during-second-rollback`.

Use names that are stable and documented in tests.

## Test structure

Each crash test must:

1. create a valid backup with exact hashes;
2. establish known pre-state bytes and permissions;
3. launch the real `snp restore` binary with `test-support` and one failpoint;
4. confirm the process terminated at the expected boundary;
5. inspect journal, stage, backup, live files, and canonical pending marker;
6. launch a second real mutating command or explicit repair/recovery path;
7. verify exact final state;
8. verify pending generation count and transaction cleanup;
9. repeat recovery to prove idempotence.

Do not satisfy these tests by manually writing journals.

## Required matrix

| Failpoint | Required recovery result |
|---|---|
| after prepared | no live change; rollback/cleanup safe; no pending |
| after backups durable | no live change; rollback safe; no pending |
| after first install | exact rollback or deterministic roll-forward; one final state only |
| after index install | library/index consistency restored |
| after all installs | finalization creates/reuses exactly one pending generation |
| before pending | recovery creates one canonical generation |
| after pending before journal update | recovery reuses same generation |
| after journal pending before cleanup | cleanup only; no increment |
| during rollback | second recovery resumes correctly |

## Closure criteria

- real subprocesses are killed at production boundaries;
- tests inspect actual artifacts;
- pending behavior is exact;
- all failpoints are unavailable in production builds.

---

# Workstream H — Coordinate every backup-visible writer

## Goal

Ensure backup sees a complete before-state or complete after-state for every included logical mutation.

## Required writer inventory

Create and maintain a table in `architecture/persistence.md` listing at least:

| State | Writers | Backup inclusion | Required guard |
|---|---|---|---|
| library TOML | new/edit/delete/import/restore/sync pull/migration | always | LocalDataGuard |
| `libraries.toml` | library create/delete/primary/link/sync/migration/restore | always | LocalDataGuard |
| `usage.toml` | run/use accounting/restore | optional | LocalDataGuard when included |
| `sync.toml` | register/config/restore/migration | optional | LocalDataGuard when included |
| legacy snippets file | migration/legacy commands | when applicable | LocalDataGuard |

Include any other file currently included in backup.

## Guard-required APIs

Refactor multi-file library operations so the lock spans the complete logical mutation:

```rust
pub fn create_library(&mut self, name: &str) -> SnipResult<PathBuf> {
    let state = derive_local_data_state_dir();
    gate_mutation_on_interrupted_transactions(&state)?;
    let guard = acquire_local_data_lock(&state)?;
    self.create_library_guarded(name, &guard)
}

fn create_library_guarded(
    &mut self,
    name: &str,
    guard: &LocalDataLock,
) -> SnipResult<PathBuf> {
    // write library and index while the same guard remains alive
}
```

Apply equivalent treatment to:

- create library;
- delete library;
- legacy migration;
- set primary;
- server link/unlink metadata;
- add server library;
- restore;
- sync pull or merge writes;
- usage writes when included;
- sync settings writes when included;
- repair actions touching included state.

Avoid nested acquisition by providing internal guarded variants. Do not add public `skip_lock: bool` parameters.

## Backup behavior

Backup must hold `LocalDataLock` from before file enumeration until all source bytes and metadata needed for the snapshot are captured and verified. It may release the lock before writing the external backup output, provided no later reads from live state occur.

## Barrier-controlled tests

Add deterministic barriers under `test-support`. Example:

1. writer acquires the lock and writes the first half of a logical mutation;
2. writer signals `first_write_complete` and waits;
3. backup process starts and must block on the lock;
4. test confirms no complete backup output appears;
5. release writer to finish the second half and release lock;
6. backup completes;
7. assert snapshot is exactly before-state or after-state, never mixed.

Required scenarios:

- library create: file plus index;
- library delete: index plus file removal;
- primary switch involving index metadata;
- sync pull updating library plus index metadata;
- restore replacing multiple included files;
- sync settings update while `--include-sync-state` is used;
- usage update while `--include-usage` is used;
- owner process dies while holding local-data lock, followed by safe reclaim.

## CI correction

The CI step named “Backup snapshot concurrency” must run:

```bash
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
```

It must not run `backup_contracts` under a misleading name.

## Closure criteria

- every included-state writer appears in the inventory;
- every listed writer participates in the same lock protocol;
- multi-file mutations hold one guard across all writes;
- barrier tests prove no mixed snapshot;
- local-data lock recovery is tested with a dead real owner process.

---

# Workstream I — Enforce manifest and domain contracts before artifacts

## Goal

Reject invalid backups for the intended reason before locks, journals, backups, stages, or live writes are created.

## Required validation order

Immediately after deserialization:

1. schema must equal the supported schema exactly;
2. layout must equal the supported layout exactly;
3. manifest file count and total declared size must be bounded;
4. entry paths and kinds must be valid;
5. canonical destination mapping must be computed;
6. exact destination duplicates must fail;
7. portable case-fold/Windows-alias collisions must fail on every host;
8. source artifacts must be regular files and within size limits;
9. declared sizes must match;
10. domain content must parse and satisfy duplicate-ID and index/library consistency rules;
11. checksums must match;
12. only then may locks or transaction artifacts be created.

## Schema and layout

Use explicit constants:

```rust
const SUPPORTED_BACKUP_SCHEMA: u32 = 1;
const SUPPORTED_BACKUP_LAYOUT: &str = "directory";
```

Errors must identify the unsupported value.

## Portable destination key

Define one platform-independent collision key for safety:

```rust
fn portable_destination_key(path: &Path) -> SnipResult<String> {
    // normalize separators, reject aliases, trim forbidden suffixes,
    // Unicode normalize according to documented policy, and case-fold.
}
```

At minimum reject:

- `Default.toml` vs `default.toml`;
- slash vs backslash aliases;
- Windows drive-relative paths such as `C:foo.toml`;
- UNC paths;
- reserved device stems with extensions;
- trailing dots/spaces;
- duplicate fixed destinations such as two index entries.

## Index/library consistency

Before transaction creation, verify:

- every index library entry has exactly one corresponding library file when required;
- no library file maps to duplicate index names under portable comparison;
- primary designation is valid;
- duplicate snippet IDs fail;
- malformed library TOML fails rather than being silently deduplicated during restore validation.

## Fixture builder

Create a helper that produces a valid backup and recomputes sizes/hashes after each mutation:

```rust
let mut fixture = ValidBackupFixture::new();
fixture.add_library("default", valid_snippets());
fixture.write();
fixture.set_schema(0);
fixture.rewrite_manifest_with_valid_hashes();
```

Negative tests must change one property only.

## Required exact tests

- schema `0` fails with unsupported-schema error;
- future schema fails with unsupported-schema error;
- unsupported layout fails with unsupported-layout error;
- exact duplicate destination fails before checksum phase;
- case-fold duplicate fails on Linux, macOS, and Windows;
- Windows alias fails on every host;
- duplicate snippet ID fails before transaction creation;
- index/library mismatch fails before transaction creation;
- each failure leaves no `.transaction/txn-*`, backup artifact, stage artifact, pending generation, or live write;
- no test accepts either success or failure;
- no targeted fixture contains placeholder or intentionally incorrect hashes unless checksum mismatch is the rule being tested.

## Closure criteria

- validation order is explicit and tested;
- all negative fixtures are otherwise valid;
- portable collisions fail closed;
- validation failures produce zero transaction artifacts.

---

# Workstream J — Add canonical server telemetry and false-success executor mode

## Goal

Prove exactly what the worker and executor did at the server boundary.

## Recording model

Extend the existing recording server to capture sanitized metadata for each canonical sync request:

```rust
pub struct RecordedSyncRequest {
    pub sequence: u64,
    pub started_at_unix_ms: u64,
    pub completed_at_unix_ms: u64,
    pub authenticated_device_id: String,
    pub library_id: String,
    pub operation: SyncOperation,
    pub encrypted_payload_present: bool,
    pub revision_before: Option<u64>,
    pub revision_after: Option<u64>,
}

pub struct RecordingSummary {
    pub requests: Vec<RecordedSyncRequest>,
    pub max_concurrency: usize,
}
```

Do not record API keys, raw snippet commands, or decrypted payload content.

## Headline test requirements

The headline test must retain and inspect the recording handle. It must assert:

- server starts at revision/state `R0`;
- exactly one canonical mutation sync request occurs;
- server observed the expected authenticated device ID;
- server observed the expected target library ID;
- encrypted payload is present;
- revision/state transitions to `R1`;
- request completed before pending was cleared;
- maximum server concurrency is exactly one;
- after a quiet period longer than debounce, no duplicate request appears;
- lifecycle contains exactly one worker and one executor start/finish pair;
- pending generation is cleared only after acknowledgement.

A local `sync.toml` assertion is not server identity evidence.

## True false-success executor mode

Add a test-only executor mode behind `test-support`, for example:

```rust
#[cfg(feature = "test-support")]
match std::env::var("SNP_TEST_EXECUTOR_MODE").as_deref() {
    Ok("noop-success") => return Ok(ExecutionSummary::SuccessWithoutRemoteWorkForTest),
    _ => {}
}
```

Production builds must ignore the variable.

The regression test must:

1. start a real recording server;
2. configure the client to point at that server;
3. enable `noop-success` executor mode;
4. perform a real local mutation;
5. observe executor exit success;
6. assert zero server requests and no server revision change;
7. assert pending remains;
8. assert status does not claim acknowledged success;
9. prove the normal headline test would fail under this mode.

An unreachable server is not an acceptable substitute.

## Read-only evidence

For read-only commands with an existing pending generation, assert:

- zero lifecycle events;
- zero server requests;
- pending bytes unchanged;
- status bytes unchanged;
- no lock or transaction artifacts created.

## Closure criteria

- recording handle is used, not discarded;
- exact request and identity assertions exist;
- no-op-success mode preserves pending;
- telemetry contains no secrets or raw snippet payload.

---

# Workstream K — Remove or compile-time gate production behavioral bypasses

## Goal

Ensure CI controls cannot silently alter production behavior.

## `SNP_SKIP_WORKER_SPAWN`

Preferred action: remove the environment-variable branch from production scheduling and replace broad CI suppression with explicit test selection.

If retained for test infrastructure, it must be fully feature-gated:

```rust
#[cfg(feature = "test-support")]
fn test_worker_spawn_suppressed() -> bool {
    std::env::var_os("SNP_SKIP_WORKER_SPAWN").is_some()
}

#[cfg(not(feature = "test-support"))]
fn test_worker_spawn_suppressed() -> bool {
    false
}
```

Scheduling must return a truthful test-only decision rather than `SpawnNow`, for example:

```rust
#[cfg(feature = "test-support")]
ScheduleDecision::SuppressedForTest
```

Do not report `Scheduled` when no process was spawned.

## Production seam tests

Build a production binary without `test-support` and set:

- `SNP_SKIP_WORKER_SPAWN`;
- `SNP_TEST_EXECUTOR_MODE`;
- `SNP_TEST_FAILPOINT`;
- `SNP_TEST_CREDENTIAL_FILE`.

The variables must not alter behavior or expose test-specific diagnostics.

## CI policy

Do not set worker suppression globally for release-blocking tests. Unit-only jobs may use internal test configuration, but all lifecycle, acknowledgement, pending-clear, and deterministic E2E suites must run with real worker spawn.

## Closure criteria

- production scheduling cannot be disabled by CI-only variables;
- no code reports scheduled work without a spawn attempt;
- release-blocking jobs run real workers;
- production-seam tests are present.

---

# Workstream L — Correct Windows and CI proof without weakening gates

## Goal

Obtain actual same-commit cross-platform evidence, including worker/executor lifecycle on Windows.

## 1. Preserve centralized setup

Keep the centralized exact-version `protoc` scripts, but verify:

- architecture mapping for x86-64 and ARM64;
- checksum verification before extraction;
- immediate PATH update in the current step;
- `protoc --version` output equals the requested version;
- no writes to `C:\` root;
- temporary files are cleaned.

## 2. Keep Windows stack configuration explicit

The checked-in `.cargo/config.toml` must remain present and documented. Add a focused smoke test or build-log assertion showing the expected target configuration is used. Do not claim the stack fix from an untracked local file.

## 3. Split test classes by behavior, not by hiding workers

Recommended jobs:

### Fast workspace job

Run unit and ordinary integration suites that do not spawn long-lived workers. Do not globally set `SNP_SKIP_WORKER_SPAWN` if doing so causes suites to return early while appearing green.

### Release-blocking lifecycle job

On Linux, macOS, and Windows, run without spawn suppression:

```bash
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
```

If a suite is currently Unix-only because of test harness assumptions, fix the harness rather than excluding Windows unless the product behavior itself is genuinely unsupported. Public auto-sync behavior is not Unix-only.

### Transaction job

Run on all three operating systems:

```bash
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
```

### PTY job

Keep PTY tests isolated from workspace correctness tests. Use explicit per-test timeouts and collect logs/artifacts on timeout. A PTY infrastructure timeout must not cause the repository to skip unrelated lifecycle evidence.

Run PTY only on platforms where the product supports the PTY path, but document the platform scope.

## 4. Eliminate shell ambiguity

Use explicit shells per step:

- Bash only for Unix-specific scripts;
- PowerShell for Windows package/setup scripts;
- avoid Bash conditionals in default Windows shells;
- avoid PowerShell syntax in Bash steps;
- prefer separate matrix include entries where commands materially differ.

## 5. Windows process cleanup

Tests that spawn workers/executors must:

- retain child handles where possible;
- wait for completion with bounded deadlines;
- terminate process trees on failure;
- close server tasks and database pools;
- remove temporary lock files after confirmed process exit;
- avoid PID guesses;
- use Windows-native liveness and creation-time observation.

## 6. Diagnose hangs rather than raising global timeouts indefinitely

For any suite exceeding its expected budget:

- run suites separately to identify the exact test;
- emit start/finish markers per test subprocess;
- capture lifecycle events and child PIDs;
- upload logs on timeout;
- fix leaked child/server/PTY resources;
- set a justified bounded timeout after root-cause correction.

A 90-minute blanket timeout is not closure evidence.

## 7. Package proof

On all three platforms:

1. `cargo package -p snip-it --locked`;
2. unpack the `.crate` as tar/gzip;
3. install from the unpacked package directory;
4. run `snp --version`;
5. run `snp --help`;
6. run a minimal isolated config smoke test;
7. confirm packaged source excludes local agent settings and runtime debris.

## 8. Same-commit evidence

The final status document must record:

- final commit SHA;
- workflow run URL;
- workflow run ID;
- each required job name and conclusion;
- Windows job IDs for lifecycle, transaction, package, clippy/build, and release profile;
- any ignored tests with justification;
- exact retry attempts if GitHub infrastructure failed.

Do not mark CI complete from YAML inspection alone.

## Closure criteria

- deterministic E2E runs on Windows;
- release-blocking jobs do not suppress worker spawn;
- backup concurrency runs the correct suite;
- PTY isolation does not weaken correctness proof;
- all required jobs pass on one final commit;
- workflow evidence is recorded in the repository.

---

# Workstream M — Repository hygiene and local agent configuration

## Goal

Remove machine-local or agent-local configuration from the public source tree.

## Required changes

- remove `.poolside/settings.local.yaml` from version control unless the project explicitly intends to publish it as a shared policy;
- add `.poolside/settings.local.yaml` or `.poolside/` to `.gitignore` according to intended policy;
- inspect recent commits for other local settings, temporary CI downloads, credentials, or environment-specific files;
- ensure package contents exclude local agent configuration.

If a shared Poolside configuration is desired, replace the local file with a documented non-local template whose name and contents clearly indicate repository policy.

## Closure criteria

- no machine-local settings are tracked;
- no secrets are present in repository history introduced by this work;
- `cargo package --list` contains only intended project files.

---

# Workstream N — Documentation and final evidence reconciliation

## Goal

Make all repository claims match executable behavior and current evidence.

## Required documents

Update at least:

- `plans/snip-it-correctness-11-closure-status.md`;
- `architecture/persistence.md`;
- `architecture/auto_sync.md`;
- `docs/EXIT_CODES.md` if execution behavior changes;
- `docs/COMMAND_CONTRACTS.md` if recovery/finalization semantics change;
- `AGENTS.md`;
- any threat model or persistence inventory that describes transaction recovery;
- CI documentation if test jobs are reorganized.

## Required final status contents

- exact final commit SHA;
- exact workflow URL and run ID;
- per-job Linux/macOS/Windows conclusions;
- exact commands run locally;
- release-blocking suite counts from the final commit;
- crash failpoint matrix and results;
- pending-finalization crash matrix and generation results;
- ignored tests with justification;
- production-seam results;
- confirmation that no secrets or raw snippet commands appear in telemetry, journals, logs, or argv;
- confirmation that `.poolside/settings.local.yaml` is removed or intentionally replaced.

Do not retain stale commit references or test counts.

## Closure criteria

- no document claims durable staging unless production restore uses it;
- no document claims server telemetry unless exact telemetry assertions exist;
- no document claims Windows closure without same-commit successful jobs;
- the correctness program is marked complete only after every checklist below passes.

---

## 4. Recommended implementation sequence

Use small, reviewable commits. Recommended order:

1. `docs: reopen Phase 11D closure blockers`
2. `refactor: separate canonical sync state and transaction directories`
3. `refactor: add transaction-associated pending metadata and parser compatibility`
4. `fix: make restore pending finalization idempotent`
5. `fix: schedule existing pending generation without re-recording`
6. `refactor: build complete durable restore staging artifacts`
7. `fix: sync and verify backups before BackupsDurable`
8. `fix: commit from durable staging and verify installed destinations`
9. `fix: verify rollback destinations and restore permissions`
10. `test: add production restore failpoints behind test-support`
11. `test: add real subprocess crash and second-crash recovery matrix`
12. `refactor: route all backup-visible writers through guarded APIs`
13. `test: add barrier-controlled backup writer concurrency`
14. `fix: enforce schema layout portable collisions and index contracts`
15. `test: replace permissive manifest fixtures with valid targeted fixtures`
16. `test: add recording-server request telemetry`
17. `test: add false-success executor mode and regression proof`
18. `fix: remove or feature-gate worker-spawn suppression`
19. `ci: run lifecycle and crash proof on Windows`
20. `ci: isolate and diagnose PTY timeouts`
21. `chore: remove local Poolside settings from source and package`
22. `docs: record final same-commit evidence and close Phase 11D`

Do not combine transaction protocol changes, telemetry changes, and CI rewrites into one opaque commit.

---

## 5. Required verification commands

Run locally where supported:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
cargo test --release --workspace --all-features -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
cargo test --test execution_outcomes --features test-support -- --test-threads=1
cargo test --test update_archive_security --features test-support -- --test-threads=1
cargo package -p snip-it --locked
cargo package -p snip-it --locked --list
```

Production-seam checks:

```bash
cargo build --release --no-default-features
SNP_TEST_CREDENTIAL_FILE=/path/that/exists target/release/snp status --json
SNP_TEST_FAILPOINT=restore-after-first-install target/release/snp status --json
SNP_TEST_EXECUTOR_MODE=noop-success target/release/snp status --json
SNP_SKIP_WORKER_SPAWN=1 target/release/snp status --json
```

The production binary must not expose test-only behavior because of these variables.

Targeted pending checks should inspect both directories:

```bash
find "$CONFIG_DIR" -maxdepth 2 -name 'auto-sync-pending.toml' -print
```

Exactly one canonical path is permitted after a successful mutating restore.

GitHub Actions must run the required final matrix on the exact final commit.

---

## 6. Explicit final closure checklist

### Status

- [ ] Phase 11 status names Phase 11D until completion.
- [ ] Final status references the actual final commit.
- [ ] Final status includes workflow URL and run ID.
- [ ] Linux, macOS, and Windows job conclusions are recorded.
- [ ] No release claim relies only on commit messages, YAML, or test names.

### Directory separation

- [ ] Canonical sync state directory is explicit.
- [ ] Transaction directory is explicit.
- [ ] Pending APIs never receive `.transaction` as their state root.
- [ ] Transaction recovery receives both paths when needed.
- [ ] No pending marker is left inside `.transaction`.

### Pending finalization

- [ ] Pending records support transaction association.
- [ ] One successful restore creates exactly one generation.
- [ ] Restore schedules existing pending work without incrementing.
- [ ] Crash before pending creates one generation on recovery.
- [ ] Crash after pending creation reuses the same generation.
- [ ] Crash before cleanup does not increment.
- [ ] Recovery is idempotent across repeated attempts.
- [ ] Unrelated newer pending work is preserved.
- [ ] Failed, rolled-back, and no-op restores create no generation.

### Restore preparation

- [ ] Validation completes before transaction artifacts.
- [ ] Complete intended output bytes are computed before commit.
- [ ] Durable backup files exist and are synced.
- [ ] Durable staged replacement files exist and are synced.
- [ ] Artifact hashes are verified from disk.
- [ ] Journal contains typed actions, hashes, paths, permissions, and durability.
- [ ] `BackupsDurable` is persisted only after all artifact verification.

### Commit

- [ ] Commit consumes durable staged files.
- [ ] Commit cursor represents completed verified positions.
- [ ] Destination intended hash is verified from the live file.
- [ ] Progress is persisted after verification.
- [ ] Replay after crash is idempotent.
- [ ] Divergent completed destinations fail closed.
- [ ] Staged artifacts remain until pending finalization is complete.

### Rollback

- [ ] Replace restores exact original bytes.
- [ ] Create removes the new destination.
- [ ] Delete restores the deleted destination.
- [ ] Verification reads the live destination.
- [ ] Permissions are restored where supported.
- [ ] Second crash during rollback resumes correctly.
- [ ] Missing/corrupt artifacts preserve evidence and return nonzero.
- [ ] Rollback creates no pending generation.

### Crash proof

- [ ] Real restore subprocesses are killed at every required failpoint.
- [ ] Tests inspect real journal, backup, stage, live, and pending artifacts.
- [ ] Commit-to-pending crash matrix passes.
- [ ] Rollback second-crash matrix passes.
- [ ] Production builds ignore failpoint variables.

### Backup coherence

- [ ] Writer inventory is complete.
- [ ] Every included-state writer uses local-data coordination.
- [ ] Library create holds the lock across file and index writes.
- [ ] Library delete holds the lock across index and file removal.
- [ ] Migration holds the lock across file and index changes.
- [ ] Restore and sync pull participate.
- [ ] Usage and sync settings participate when included.
- [ ] Barrier tests prove complete before-state or after-state.
- [ ] CI runs `backup_snapshot_concurrency`, not a different suite.

### Manifest/domain

- [ ] Unsupported schema fails explicitly.
- [ ] Unsupported layout fails explicitly.
- [ ] Exact destination collisions fail.
- [ ] Portable case-fold and Windows aliases fail on every host.
- [ ] Duplicate snippet IDs fail before artifacts.
- [ ] Index/library inconsistency fails before artifacts.
- [ ] Negative fixtures have valid sizes and hashes.
- [ ] No test accepts either success or failure.
- [ ] Validation failures create no transaction or pending artifacts.

### Server and lifecycle evidence

- [ ] Headline test retains recording-server telemetry.
- [ ] Exactly one canonical request is asserted.
- [ ] Server-observed device identity is asserted.
- [ ] Server-observed library identity is asserted.
- [ ] Encrypted payload presence is asserted.
- [ ] Revision transition is asserted.
- [ ] Maximum server concurrency is one.
- [ ] Server acknowledgement precedes pending clear.
- [ ] Quiet period shows no duplicate request.
- [ ] False-success executor exits `0` but preserves pending.
- [ ] Read-only commands produce zero lifecycle events and server requests.

### Test-only boundaries

- [ ] Production ignores test credential variables.
- [ ] Production ignores restore failpoints.
- [ ] Production ignores executor-mode variables.
- [ ] Production behavior cannot be disabled by worker-suppression variables.
- [ ] Scheduling never reports success without a spawn attempt.
- [ ] Secrets and raw snippet commands do not appear in test telemetry.

### Windows and CI

- [ ] Protoc setup is exact-version, architecture-aware, and verified.
- [ ] Windows stack configuration is present and documented.
- [ ] Workspace commands are shell-neutral.
- [ ] Deterministic E2E runs on Windows.
- [ ] Sync contracts run on Windows.
- [ ] Restore crash failpoints run on Windows.
- [ ] Backup concurrency runs on Windows.
- [ ] Release-blocking jobs do not suppress worker spawn.
- [ ] PTY tests are isolated and bounded.
- [ ] Child processes and handles are cleaned up.
- [ ] Package/install smoke succeeds from unpacked `.crate` on all platforms.
- [ ] All required jobs pass on the same final commit.

### Hygiene and documentation

- [ ] `.poolside/settings.local.yaml` is removed or intentionally replaced.
- [ ] Local agent settings are excluded from package output.
- [ ] Closure status uses current commit and test counts.
- [ ] Workflow URL and job conclusions are recorded.
- [ ] Persistence and auto-sync architecture docs match code.

### Architecture

- [ ] One installed `snp` binary remains the client architecture.
- [ ] Auto-sync workers remain one-shot subprocesses.
- [ ] No daemon, helper binary, plugin runtime, workflow engine, or database expansion was introduced.

---

## 7. Release decision rule

The release decision is binary.

Mark Phase 11 and the correctness program complete only when:

1. every applicable checkbox above is satisfied;
2. production code matches the documented pending, transaction, backup, credential, and execution contracts;
3. adversarial tests prove the intended crash, failure, or recovery condition directly;
4. Linux, macOS, and Windows jobs pass on the same final commit;
5. the closure status includes the workflow URL, run ID, and exact job conclusions.

The program remains open if any of the following is true:

- restore writes pending under `.transaction`;
- one restore can increment pending more than once;
- transaction recovery cannot idempotently reuse its pending generation;
- `durable_staged_path` remains unused in production restore;
- `BackupsDurable` is persisted before backup/stage sync and verification;
- commit or rollback verifies source buffers instead of live destinations;
- crash tests manually synthesize journals instead of killing production restore;
- backup coordination excludes any writer of included state;
- backup concurrency tests are sequential;
- manifest tests use invalid hashes or accept either outcome;
- server recording telemetry is discarded;
- the no-op regression is only an unreachable-server test;
- production scheduling can be changed by `SNP_SKIP_WORKER_SPAWN`;
- release-blocking Windows jobs skip deterministic E2E or sync contracts;
- CI evidence is missing, flaky, permissively skipped, or from another commit;
- closure documentation claims configuration or evidence absent from the repository.

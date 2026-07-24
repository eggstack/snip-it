# Phase 11C — Final Durability, Locking, Evidence, and Windows Closure

Status: READY FOR IMPLEMENTATION

Authoritative implementation baseline: `20b6c52c8d01dea66b7f445ac756af2e71282406`

Parent plans:

- `plans/snip-it-correctness-11-verification-and-crash-closure.md`
- `plans/snip-it-correctness-11b-durability-verification-windows-ci-closure.md`

Current status document:

- `plans/snip-it-correctness-11-closure-status.md`

Program status: REOPENED

This plan is the authoritative follow-up for the defects that remained after the partial Phase 11B implementation. It is intentionally narrower than Phase 11B. It does not reopen already-correct work such as path traversal rejection, typed manifest-kind deserialization, native ZIP validation, operation-aware dry-run classification, or the single-binary auto-sync architecture.

Phase 11 and the correctness program must remain open until every release-blocking criterion in this plan is supported by production code, adversarial tests, and successful Linux, macOS, and Windows GitHub Actions jobs on the same final commit.

---

## 1. Objective

Close the remaining correctness gaps without expanding product scope:

1. ensure a live transaction lock can never be stolen because the contender compares the wrong process identity;
2. make stale-lock reclamation atomic under concurrent contenders;
3. prevent restore from detecting and rolling back its own active transaction;
4. make restore journals complete and durable before live writes begin;
5. persist commit progress only after a destination has been atomically installed and verified;
6. eliminate the crash window between durable restore commit and pending-sync intent;
7. make rollback restartable using rollback-order coordinates rather than ambiguous file indices;
8. verify exact pre-transaction bytes or expected absence after every rollback action;
9. coordinate backup with every writer that can affect included state;
10. make local-data locking crash-recoverable rather than a permanent bare-file mutex;
11. enforce manifest schema, layout, destination, case-fold, and duplicate-ID rules before transaction creation;
12. make negative tests fail for the intended condition with otherwise valid artifacts;
13. prove canonical server requests and lifecycle behavior directly;
14. provide a true false-success/no-op executor regression seam;
15. map output-file spawn failure to public execution exit code `8`;
16. obtain and record actual successful Windows CI evidence rather than inferring portability from YAML.

---

## 2. Non-goals and architectural constraints

Preserve all of the following:

- one installed client binary: `snp`;
- detached auto-sync workers remain one-shot subprocesses;
- no resident client daemon;
- no second installed helper binary;
- no database replacing TOML state;
- no plugin runtime;
- no workflow engine;
- no distributed transaction protocol;
- no CRDT expansion;
- no broad CLI redesign;
- no public command semantics that vary by platform.

Allowed internal additions:

- a reusable owned-file-lock primitive;
- richer transaction states and journal metadata;
- an internal transaction context passed through mutation APIs;
- test-only failpoints under `test-support`;
- deterministic server telemetry;
- checked-in platform CI scripts;
- a small internal mutation coordinator.

Prefer deleting misleading or unused scaffolding over retaining parallel code paths.

---

## 3. Confirmed baseline defects

The implementation agent must treat these as defects to correct, not as optional improvements.

### 3.1 Live lock ownership comparison uses the contender’s start token

The current transaction-lock acquisition path compares the persisted owner start token with the new contender’s own start token. Distinct live processes normally have distinct start tokens, so a contender can classify a valid live owner as PID reuse and quarantine the active lock.

Correct ownership verification must observe the process identified by the existing lock record:

```rust
let observed_owner = ProcessIdentity::observe(existing.pid)?;
if observed_owner.start_token == existing.start_token {
    return Err(lock_held(existing));
}
```

Never compare `existing.start_token` with `current_process_identity().start_token` unless `existing.pid == current_process_id` and that equality is itself part of an explicit self-ownership check.

### 3.2 Stale-lock reclaim loses exclusivity

After quarantine, the current code recreates the lock with ordinary `fs::write`. Two reclaimers can both pass stale detection and race to become the owner.

Reclaim must always return to the same `create_new(true)` acquisition loop. Ordinary overwrite is prohibited.

### 3.3 Restore can roll back its own active transaction

Merge restore calls `save_library`, and `save_library` invokes the global interrupted-transaction gate. Once restore has persisted `Committing`, the gate can see the caller’s own transaction as interrupted and roll it back while restore continues.

The mutation gate needs transaction context, or restore needs an internal save path that is valid only while the caller holds the correct transaction/local-data guards.

### 3.4 Restore progress is recorded before the write

The current restore loop persists `Committing { next_index: file_idx }` before installing the corresponding destination. That state does not distinguish “about to write file N” from “file N was installed and verified.”

Persisted progress must mean completed work. A crash must never cause recovery to skip a destination that may not have been written.

### 3.5 Rollback cursor is not restartable

Rollback iterates in reverse using original file indices and persists `i + 1`. On restart, the skip predicate can skip all remaining rollback actions.

Use rollback-order positions:

```rust
let rollback_order: Vec<usize> = (0..journal.files.len()).rev().collect();
for position in journal.next_rollback_position..rollback_order.len() {
    let file_index = rollback_order[position];
    rollback_one(&journal.files[file_index])?;
    journal.next_rollback_position = position + 1;
    persist(&journal)?;
}
```

### 3.6 Restore journal content remains incomplete

The current journal does not durably contain complete staged replacement information and final hashes before live writes. Backup files are copied without a complete durability protocol, and `staged_path` can still refer to the live destination.

### 3.7 Commit-to-pending crash window remains

Restore removes its committed journal and backups, then records pending sync intent. A crash between those operations can leave committed local content with no pending generation.

### 3.8 Backup and writer locks are not one coherent protocol

Backup acquires `LocalDataLock`; normal `save_library` also acquires it. Other writers—including library-index changes, library create/delete, restore, migration, usage, sync settings, and repair—do not consistently participate.

The current local-data lock is a bare create/delete file with no ownership record or stale recovery. A crash can block future operations indefinitely.

### 3.9 Manifest validation remains incomplete and tests are permissive

Typed kinds are present, but schema/layout and collision rules are not reliably enforced before hashing and transaction creation. Several tests use invalid checksums or accept either success or failure.

### 3.10 Headline E2E does not use server telemetry

The test discards the recording handle and does not assert canonical request count, request identity, target library, encrypted payload, revision transition, or maximum concurrency.

The no-op regression test points at an unreachable server. It does not simulate an executor returning local success without remote work.

### 3.11 Output-file spawn failure still reaches generic exit code `1`

The output-file branch maps wait/timeout errors into `ProcessResult::Failed`, but shell spawning still returns `SnipError` through `?`.

### 3.12 Windows workflow is improved but unproven

The YAML is more portable, but no successful same-commit Windows evidence is recorded. The follow-up commit also claims a Windows stack configuration that is not present in the repository.

---

# Workstream A — Reopen closure evidence accurately

## Goal

Prevent the status document from representing CI as the only blocker while production correctness defects remain.

## Required first commit

Update `plans/snip-it-correctness-11-closure-status.md` before production changes.

It must state:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md
Corrective baseline: 20b6c52c8d01dea66b7f445ac756af2e71282406
```

List the open workstreams from this plan. Remove claims that the durable executor, restartable rollback, complete backup coordination, server telemetry, and all execution outcomes are already closed.

## Closure criteria

- status does not claim only CI remains;
- no Phase 11C checkbox is pre-checked;
- historical evidence remains available but is labeled as superseded where appropriate;
- final status identifies the exact implementation commit and workflow URL.

---

# Workstream B — Build one reusable owned-file-lock primitive

## Goal

Use one ownership and reclaim protocol for transaction and local-data locks.

## Required model

Introduce a private or crate-visible primitive, for example:

```rust
pub struct OwnedFileLock {
    path: PathBuf,
    owner: LockOwner,
}

#[derive(Serialize, Deserialize)]
pub struct LockOwner {
    schema: u32,
    pid: u32,
    start_token: Option<String>,
    nonce: String,
    purpose: String,
    transaction_id: Option<String>,
    created_at_unix_ms: i64,
}
```

Exact names may differ. Equivalent semantics are mandatory.

## Owner observation

Provide separate functions for:

```rust
ProcessIdentity::current()
ProcessIdentity::observe(pid)
```

`observe(pid)` must query the specified PID, not the current process.

Platform guidance:

- Linux: `/proc/<pid>/stat` start time field;
- macOS: use a stable process-start query where available, otherwise conservative PID-liveness policy;
- Windows: use `OpenProcess` plus creation time from `GetProcessTimes` when permissions allow;
- when a live PID’s start identity cannot be observed, treat the lock as live and refuse acquisition rather than reclaiming it.

Do not interpret “identity unavailable” as “stale.”

## Atomic acquisition and reclaim

Use one acquisition loop:

```rust
loop {
    match create_new_lock() {
        Ok(lock) => return Ok(lock),
        Err(AlreadyExists) => {
            let existing = read_and_validate_lock()?;
            match classify_owner(existing)? {
                Live => return Err(lock_held(existing)),
                DeadOrReused => quarantine_with_atomic_rename(existing)?,
                Unknown => return Err(lock_state_unknown(existing)),
            }
        }
        Err(e) => return Err(e.into()),
    }
}
```

After quarantine, loop back to `create_new(true)`. Never call ordinary `fs::write` to claim ownership.

## Release semantics

Drop/removal must verify at least:

- nonce;
- PID;
- start token when present;
- transaction ID or lock purpose when present.

If the record is malformed or mismatched, preserve it and log a bounded warning. Do not remove it.

## Required tests

1. live child owner blocks a second process;
2. same PID with matching observed start token blocks;
3. dead child owner is reclaimed;
4. simulated PID reuse with a mismatched observed token is reclaimed;
5. identity-unavailable live owner is not reclaimed;
6. malformed lock is quarantined only by an explicit acquisition attempt, not by read-only inspection;
7. two concurrent reclaimers produce exactly one owner and one lock record;
8. wrong nonce cannot remove the lock;
9. wrong transaction ID cannot remove the lock;
10. transaction and local-data locks use the same tested primitive.

## Closure criteria

- no code compares an existing owner token with the contender’s token;
- no reclaim path uses ordinary overwrite;
- a live owner always blocks;
- concurrent reclaim has one winner;
- local-data lock cannot remain permanently stale after owner death.

---

# Workstream C — Define lock hierarchy and transaction context

## Goal

Prevent self-recovery, nested lock deadlocks, and inconsistent writer coordination.

## Required lock order

Document and enforce one order:

```text
LocalDataLock -> TransactionLock -> destination writes
```

A transaction may hold both. No path may acquire `LocalDataLock` while already holding `TransactionLock` if another path acquires them in the opposite order.

A simpler acceptable order is:

1. acquire `LocalDataLock` for any operation touching backup-visible files;
2. acquire `TransactionLock` for multi-file transactional work;
3. perform validation/staging/writes;
4. release transaction lock;
5. release local-data lock.

## Transaction context

Introduce an internal context:

```rust
pub struct MutationContext<'a> {
    pub transaction_id: Option<&'a str>,
    pub owns_local_data_lock: bool,
    pub owns_transaction_lock: bool,
}
```

Equivalent typed tokens are preferable to booleans if practical:

```rust
fn save_library_in_transaction(
    guard: &LocalDataGuard,
    txn: &TransactionGuard,
    path: &Path,
    snippets: &Snippets,
) -> SnipResult<()>;
```

Public/top-level mutation APIs should acquire gates and locks. Internal transaction APIs should require guards and must not invoke the global gate against their own active journal.

Do not add a general `skip_recovery: bool` flag that any caller can misuse.

## Mutation gate behavior

The gate must distinguish:

- no interrupted transaction: proceed;
- the caller’s active transaction: proceed only through guard-validated internal APIs;
- one foreign recoverable transaction: recover or refuse according to policy;
- malformed/incomplete journal: refuse and direct to repair;
- multiple transactions: refuse.

## Required tests

- merge restore does not roll back its own journal;
- a foreign interrupted transaction blocks normal snippet save;
- an internal transaction write succeeds only with the owning transaction guard;
- lock ordering does not deadlock under concurrent backup and restore;
- lock ordering does not deadlock under concurrent backup and snippet edit;
- read-only commands acquire neither lock.

## Closure criteria

- merge restore cannot call the ordinary gated `save_library` path;
- no Boolean bypass is exposed broadly;
- all multi-file operations follow the documented lock hierarchy;
- self-transaction detection is explicit and tested.

---

# Workstream D — Prepare a complete durable restore plan

## Goal

Persist all data needed for commit and rollback before the first live destination changes.

## Required transaction entry

Each planned file must contain:

```rust
pub struct PlannedFile {
    pub destination: PathBuf,
    pub action: StagedAction,
    pub existed_before: bool,
    pub original_hash: Option<Sha256Digest>,
    pub intended_hash: Option<Sha256Digest>,
    pub durable_backup_path: Option<PathBuf>,
    pub durable_staged_path: Option<PathBuf>,
    pub original_permissions: Option<PortablePermissions>,
    pub durability: Durability,
}
```

Use typed digests or validated fixed-length strings. Empty strings must not encode absence.

## Required preparation sequence

1. load and parse manifest;
2. validate manifest schema and layout;
3. validate all paths and portable aliases;
4. validate all source artifact types, sizes, and hashes;
5. parse incoming TOML into domain types;
6. validate duplicate snippet IDs and index/library consistency;
7. compute the complete merge/replace output bytes in memory;
8. compute deterministic destination order;
9. acquire local-data lock;
10. acquire transaction lock;
11. revalidate mutable local inputs after locks are held;
12. create a private transaction workspace;
13. create durable backups for every existing destination;
14. create durable staged files containing exact intended bytes;
15. fsync files and required parent directories according to durability class;
16. populate all journal fields, including hashes and action;
17. atomically persist `BackupsDurable`;
18. only then begin live replacement.

## Staging rules

- staged files must not be the live destinations;
- stage on the destination filesystem when required for atomic replacement;
- reject staged symlinks and non-regular files;
- use private permissions for sensitive config;
- verify staged hash before journaling `BackupsDurable`.

## Required failpoints under `test-support`

Provide typed failpoints at:

- after all backups are durable;
- after all staged files are durable;
- after `BackupsDurable` journal persistence;
- before first live replacement.

Failpoints must support both returned errors and immediate process termination.

Production builds must not recognize failpoint variables.

## Closure criteria

- persisted `BackupsDurable` journal contains every backup and staged path;
- all recorded artifacts exist, are regular files, and match recorded hashes;
- no live destination changes before `BackupsDurable` is durable;
- no `staged_path` aliases its live destination;
- crash immediately after `BackupsDurable` is recoverable.

---

# Workstream E — Commit with after-write progress and atomic pending intent

## Goal

Make every persisted commit cursor represent completed and verified work.

## State model

Use explicit completed-position semantics, for example:

```rust
TransactionState::Committing {
    next_commit_position: usize,
}
```

`next_commit_position == N` means positions `0..N` have already been installed and verified; position `N` is next.

## Commit loop

```rust
while journal.next_commit_position < order.len() {
    let position = journal.next_commit_position;
    let file = &journal.files[order[position]];

    apply_atomically(file)?;
    verify_intended_state(file)?;

    journal.next_commit_position = position + 1;
    persist_journal(&journal)?;
}
```

Persist the initial `Committing { next_commit_position: 0 }` before applying the first file. Persist progress only after install and verification.

## Commit-to-pending atomicity

Eliminate the current crash window. Acceptable designs include:

### Preferred: transaction finalization state

```rust
CommittedLocal {
    pending_generation: u64,
    pending_recorded: bool,
}
```

Sequence:

1. all destinations installed and verified;
2. allocate or determine exactly one pending generation;
3. persist `CommittedLocal { pending_generation, pending_recorded: false }`;
4. durably write pending marker for that generation;
5. persist `CommittedLocal { pending_generation, pending_recorded: true }`;
6. mark final/clean up;
7. schedule worker after durable pending intent.

Recovery from `pending_recorded: false` must idempotently write the same generation, not allocate another.

### Acceptable alternative: durable local outbox record

A transaction-owned outbox may be used if it provides equivalent idempotence and exact-generation semantics.

Do not clear the journal before pending intent is durably represented.

## Required tests

- crash after first destination: recovery resumes from the next uncommitted position;
- crash after destination write but before progress persistence: replay is idempotent and verifies content;
- crash after all destination writes but before `CommittedLocal`: recovery completes safely;
- crash after `CommittedLocal` but before pending marker: recovery writes exactly the recorded generation;
- crash after pending marker but before final cleanup: recovery does not allocate a second generation;
- successful restore creates exactly one pending generation;
- no-op restore creates no pending generation.

## Closure criteria

- progress is persisted after verified writes;
- commit replay is idempotent;
- no committed content can lose pending intent;
- no recovery path creates duplicate pending generations;
- scheduling occurs only after durable pending representation.

---

# Workstream F — Correct restartable rollback

## Goal

Restore exact pre-transaction state after handled failure or process death, including a second crash during rollback.

## Rollback cursor

Use rollback-order coordinates:

```rust
RollingBack {
    next_rollback_position: usize,
}
```

Do not reuse original file index semantics.

## Action semantics

- `Replace`: atomically restore exact backup bytes and original permissions;
- `Create`: remove the destination if it exists and verify absence;
- `Delete`: atomically restore backup bytes and original permissions;
- `NoOp`: verify expected unchanged state or do nothing according to the plan.

## Verification

After each action:

- verify SHA-256 equals `original_hash`, or destination is absent when `existed_before == false`;
- fsync according to durability class;
- persist `next_rollback_position = position + 1`.

If verification fails:

- persist `RecoveryFailed { position, reason }` or equivalent;
- retain journal, backups, staged files, and lock evidence;
- return nonzero;
- report exact operator action through `repair --json`.

## Cleanup

Delete artifacts only after:

- every rollback action is verified;
- `RolledBack` is durable;
- pending intent is confirmed absent or unchanged.

## Required failpoint tests

- handled failure after first live replacement;
- process death after first live replacement;
- process death after index replacement;
- process death during first rollback action;
- process death after rollback action but before cursor persistence;
- created destination is removed;
- deleted destination is restored;
- missing backup retains evidence and reports unrecoverable state;
- corrupted backup retains evidence and reports hash failure;
- second invocation resumes and completes rollback.

## Closure criteria

- rollback cursor is unambiguous;
- exact original bytes or absence are verified;
- second crash during rollback is recoverable;
- no failed/rolled-back restore creates pending intent;
- no artifact is deleted before verified terminal state.

---

# Workstream G — Coordinate backup with every included-state writer

## Goal

Guarantee a backup represents one complete before-state or one complete after-state.

## Required writer inventory

Audit and classify every path that writes backup-visible files:

- snippet create/edit/delete;
- library create/delete/rename/set-primary/link/unlink;
- library index saves;
- import;
- restore;
- repair;
- migration;
- usage updates and pruning when `--include-usage` is relevant;
- sync settings when `--include-sync-state` is relevant;
- sync pull/bidirectional application;
- any self-healing or corruption-recovery rewrite.

Record the inventory in `architecture/persistence.md` or a dedicated status appendix.

## Required coordination

Every listed writer must acquire the same owned local-data lock before its first relevant read/write and hold it through the complete logical mutation.

Examples:

- library create must hold the lock across file creation and index update;
- library delete must hold it across index update and file deletion;
- restore must hold it across preparation revalidation, commit, and pending finalization;
- backup must hold it across enumeration and byte capture, but may release it before writing the external backup output from captured bytes;
- sync pull must hold it across all local destination writes.

## Avoid nested reacquisition

Provide guarded internal helpers:

```rust
fn save_library_guarded(
    guard: &LocalDataGuard,
    path: &Path,
    snippets: &Snippets,
) -> SnipResult<()>;
```

Top-level `save_library` may acquire the guard, but transactional callers must use the guarded variant.

## Local-data lock recovery

Use the reusable owned-lock primitive from Workstream B. A dead owner must be reclaimable; malformed records must be quarantined; a live or unknown owner must block.

## Required concurrent tests

Use barriers rather than sleeps:

1. pause backup after first library byte capture;
2. start snippet edit and prove it blocks;
3. release backup and prove edit completes afterward;
4. inspect backup and live state: backup is complete before-state, live is complete after-state;
5. repeat with library creation affecting both file and index;
6. repeat with library deletion;
7. repeat with restore;
8. kill a backup process while holding the lock and prove the next writer reclaims it safely.

No test may modify files directly with `fs::write` and call that proof of mutation coordination. Exercise production mutation paths.

## Closure criteria

- all inventory writers participate;
- no bare local-data lock remains;
- actual concurrent tests prove before-or-after snapshots;
- index/library pairs cannot be mixed;
- crashed owner does not permanently block backup or mutations.

---

# Workstream H — Enforce manifest and restore domain contracts before hashing

## Goal

Fail closed for the intended reason before transaction creation.

## Validation order

1. deserialize manifest with typed kind;
2. reject unsupported schema;
3. reject unsupported layout;
4. validate entry count and aggregate size limits;
5. normalize portable paths;
6. reject duplicate source paths;
7. resolve typed destinations;
8. reject duplicate destinations;
9. reject Unicode/case-fold destination collisions according to the documented portable policy;
10. reject Windows aliases, drive-relative paths, UNC, reserved names, trailing-dot/space aliases, and alternate separators on every platform;
11. inspect source artifact type and containment;
12. verify source size and hash;
13. parse TOML into the correct domain type;
14. reject duplicate incoming snippet IDs;
15. validate index/library consistency;
16. only then create locks, transaction directories, journals, or backups.

## Portable collision key

Define one deterministic key rather than relying on host filesystem behavior. For example:

```rust
fn portable_destination_key(path: &ValidatedDestination) -> String {
    unicode_normalize(path.as_str())
        .replace('\\', "/")
        .trim_end_matches(['.', ' '])
        .to_lowercase()
}
```

The exact Unicode policy must be documented and tested. Avoid locale-sensitive lowercasing.

## Duplicate snippet IDs

Parse each incoming library into `Snippets` and require unique, nonempty IDs before merge or replacement. Do not rely on `load_library`, which may repair/deduplicate IDs and hide malformed backup input.

## Test fixture rule

Every negative test must use otherwise valid:

- source file;
- size;
- SHA-256;
- manifest syntax;
- unrelated entries.

Then assert a stable error category or structured JSON code for the intended validation failure.

Forbidden patterns:

```rust
assert!(!success); // without checking why
if success { /* acceptable */ }
assert!(error.contains("Checksum") || error.contains("duplicate"));
```

## Required tests

- schema `0` fails with `unsupported_schema` before any source read where practical;
- future schema fails with `unsupported_schema`;
- non-directory layout fails with `unsupported_layout`;
- duplicate exact destination fails;
- portable case-fold collision fails on Linux, macOS, and Windows identically;
- Windows drive-relative and UNC forms fail on all hosts;
- trailing-dot/space aliases fail on all hosts;
- duplicate snippet ID fails before transaction directory creation;
- malformed library TOML fails before transaction creation;
- mismatched index/library set fails before transaction creation;
- validation failure creates no journal, lock, pre-restore backup, pending marker, or live write.

## Closure criteria

- schema/layout are validated explicitly;
- collision policy is host-independent;
- duplicate IDs are rejected, never silently repaired;
- tests prove the intended failure with valid hashes;
- no permissive either-outcome assertions remain.

---

# Workstream I — Complete deterministic server and lifecycle evidence

## Goal

Prove one local mutation causes exactly one canonical remote effect before pending clears.

## Server telemetry

Retain and query the recording handle returned by the test server. Record at least:

```rust
pub struct RecordedCanonicalRequest {
    pub request_id: String,
    pub authenticated_device_id: String,
    pub library_id: String,
    pub operation: String,
    pub encrypted_payload_present: bool,
    pub revision_before: i64,
    pub revision_after: i64,
    pub started_at: Instant,
    pub completed_at: Instant,
}
```

Also record active request count and maximum observed concurrency.

## Headline assertions

The test must require:

- exactly one canonical sync mutation request;
- expected authenticated device ID;
- expected target library ID;
- encrypted payload present and nonempty;
- server revision `R0 -> R1` or equivalent monotonic transition;
- server content contains the expected snippet;
- maximum canonical-request concurrency equals `1`;
- exactly one worker start and one executor start;
- server completion timestamp precedes pending-clear observation;
- no duplicate request during a bounded quiet period after completion.

Status-file success is supplemental evidence only.

## True no-op-success seam

Under `test-support`, add an executor mode that:

- starts normally;
- emits lifecycle events;
- returns local success/exit `0`;
- performs no server request;
- does not provide a remote acknowledgement.

The production worker must preserve pending intent because acknowledgement is absent. The headline proof or a dedicated regression must fail if pending clears.

Do not substitute an unreachable-server failure for this test.

## Read-only evidence

Read-only command tests must assert:

- zero worker lifecycle events;
- zero executor lifecycle events;
- zero server requests;
- pending generation unchanged;
- status bytes unchanged unless the command’s documented purpose is status reporting without mutation;
- no transaction/local-data lock acquisition.

## Test isolation

Lifecycle recording must be namespaced per test environment. Remove global shared event directories or static mutable state that can mix parallel tests.

The headline test must pass:

- alone;
- in its suite;
- in full workspace tests;
- debug and release;
- Linux, macOS, Windows.

## Closure criteria

- recording handle is used, not discarded;
- exactly one canonical request is proven;
- false local success cannot clear pending;
- read-only absence is direct evidence;
- no known full-suite flakiness remains.

---

# Workstream J — Finish execution outcome mapping

## Goal

Apply one outcome mapping to both output-file and ordinary execution branches.

## Required refactor

Move shell spawn and wait mapping into one helper:

```rust
fn spawn_and_wait_execution(...) -> ProcessResult
```

It must return:

- `Done { code: 0 }` for success;
- `Failed { exit_code: Some(code) }` for normal child nonzero;
- `Failed { exit_code: None }` for spawn failure, timeout, or signal/no-code termination.

Both output-file and ordinary branches must call the helper. No shell-spawn `?` may bypass outcome mapping.

## Output-file cleanup policy

Document behavior for failed execution:

- either retain partial output with an explicit diagnostic;
- or atomically stage output and publish only on success.

Choose one and test it. Do not leave undocumented partial-file behavior.

## Required tests

Create snippets with a nonempty output path and assert:

- successful output writes content and exits `0`;
- child exit `7` returns `7`;
- timeout returns `8`;
- invalid shell/spawn returns `8`;
- signal/no-code termination returns `8` where applicable;
- application/config error still returns generic `1`;
- raw command and credentials do not appear in diagnostics;
- output-file cleanup policy is respected.

Windows must have a native termination test rather than a Unix signal test.

## Closure criteria

- no output-file spawn error reaches generic `SnipError` mapping;
- exit `8` covers timeout/spawn/no-code consistently;
- child nonzero remains the child code;
- tests exercise a genuinely nonempty output field.

---

# Workstream K — Correct and prove Windows CI

## Goal

Make Windows a first-class verified platform rather than a sequence of reactive patches.

## Step 1: inspect actual run data

Before changing CI again, capture:

- workflow run URL;
- failing Windows job names;
- failing step names;
- exact relevant error excerpts;
- whether failure is compile, link, stack, test timeout, process cleanup, path, or dependency setup.

Record this in the closure status or a dedicated `plans/status/` evidence file.

Do not infer the current failing cause solely from commit messages.

## Step 2: remove shell ambiguity

Avoid `shell: bash` for full Windows workspace testing unless a test explicitly requires Git Bash semantics.

Prefer separate conditional steps using each platform’s default shell:

```yaml
- name: Workspace tests (debug)
  if: matrix.profile == 'dev'
  run: cargo test --workspace --all-features -- --test-threads=1

- name: Workspace tests (release)
  if: matrix.profile == 'release'
  run: cargo test --release --workspace --all-features -- --test-threads=1
```

These commands are shell-neutral.

Use PowerShell only for Windows-specific file manipulation. Use Bash only for Unix-specific scripts.

## Step 3: centralize `protoc`

Reduce duplicated setup by using a checked-in script pair or a tightly scoped action step:

- `scripts/ci/install-protoc.sh`;
- `scripts/ci/install-protoc.ps1`.

Requirements:

- exact version;
- architecture-aware artifact selection;
- download failure is fatal;
- checksum verification if published checksums are available;
- install under `RUNNER_TEMP`, not `C:\` root;
- add path via `GITHUB_PATH`;
- verify `protoc --version` in a subsequent step;
- macOS selects x86_64 or arm64 based on runner architecture.

## Step 4: resolve Windows stack failure correctly

The previous commit claims `.cargo/config.toml` increases Windows stack size, but the file is absent.

Inspect the actual stack-overflowing test and backtrace first.

Preferred order:

1. remove accidental recursion or oversized by-value stack structures;
2. box unusually large parser/command values if that is the actual cause;
3. run the affected test on an explicitly sized test thread if the issue is test-only;
4. only if the production binary legitimately needs a larger Windows stack, add checked-in `.cargo/config.toml`:

```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "link-arg=/STACK:8388608"]
```

Add the corresponding aarch64 target if supported. Verify the linker argument appears in verbose build output and document why it is necessary.

Do not claim the stack fix exists unless the file is present and CI proves it.

## Step 5: Windows-native process tests

Required tests:

- spawn child, verify live owner blocks;
- wait for child exit, verify stale owner reclaims;
- terminate a command with Windows-native APIs and verify public exit `8`;
- ensure handles are closed;
- ensure no child or grandchild survives timeout tests;
- use actual spawned PIDs, never PID `1` or guessed large PIDs.

## Step 6: filesystem capability tests

For symlink/reparse behavior:

- attempt capability setup explicitly;
- if the runner lacks privilege, run a direct classifier/unit test against the production validation function;
- mark capability absence in test output;
- do not silently return success without alternate proof.

FIFO tests remain Unix-only and do not count as Windows evidence.

## Step 7: package smoke

Keep separate Unix and PowerShell package steps. On Windows, require:

- `.crate` archive found;
- archive extraction succeeds;
- package directory uniquely identified;
- `cargo install --path` succeeds;
- installed `snp.exe --version` succeeds;
- installed `snp.exe --help` succeeds;
- no workspace path dependency is accidentally used during install proof.

## CI job matrix required on final commit

- format;
- Clippy all targets/all features;
- Linux debug workspace;
- Linux release workspace;
- macOS debug workspace;
- macOS release workspace;
- Windows debug workspace;
- Windows release workspace;
- deterministic E2E on all three platforms;
- transaction crash/recovery on all three platforms;
- backup concurrency on all three platforms;
- manifest contracts on all three platforms;
- execution outcomes on all three platforms;
- read-only no-recovery on all three platforms;
- update archive security on all three platforms;
- package/install smoke on all three platforms.

The matrix may be split into focused jobs to control wall-clock time. Do not remove release-blocking suites merely to make a broad workspace job finish.

## Timeouts and diagnostics

Timeouts are safeguards, not fixes. On timeout:

- upload test logs;
- list surviving `snp` processes where platform permits;
- preserve lifecycle-event directories;
- print the last relevant status/journal records with secrets redacted.

## Closure criteria

- all required Windows jobs pass on the same final commit;
- workflow URL and job conclusions are recorded;
- no release-blocking step uses `continue-on-error`, `|| true`, or permissive early return;
- no claimed file/config is absent from the repository;
- no Windows test depends on Unix signal semantics.

---

# Workstream L — Final documentation and evidence reconciliation

## Goal

Make repository claims match executable behavior.

## Required documents

Update at least:

- `plans/snip-it-correctness-11-closure-status.md`;
- `architecture/persistence.md`;
- `architecture/auto_sync.md` if acknowledgement/finalization changes;
- `docs/EXIT_CODES.md`;
- `AGENTS.md`;
- any persistence inventory or threat model that describes transaction recovery.

## Required closure evidence

The final status must include:

- final commit SHA;
- workflow run URL;
- per-job conclusion for Linux/macOS/Windows;
- exact commands run locally;
- test counts by release-blocking suite;
- failpoint matrix and result;
- known ignored tests with justification;
- confirmation that production build ignores all `test-support` failpoints and credential seams;
- confirmation that no secrets or snippet payloads appear in logs/events/arguments.

Do not write “all tests pass” without linking the workflow and naming the required jobs.

## Closure criteria

- no status claim contradicts code or tests;
- no historical stale count is presented as current evidence;
- no release gate is satisfied by test name alone;
- the correctness program is marked complete only after all checklists below pass.

---

## 4. Recommended implementation sequence

Use small, reviewable commits. Recommended order:

1. `docs: reopen Phase 11C closure blockers`
2. `refactor: add owned file lock and process identity observation`
3. `fix: make stale lock reclaim exclusive and live-owner safe`
4. `refactor: define local-data and transaction lock hierarchy`
5. `fix: prevent restore from recovering its own transaction`
6. `refactor: build complete durable restore plans and staged artifacts`
7. `fix: persist commit progress after verified writes`
8. `fix: make pending intent transaction-finalization safe`
9. `fix: make rollback cursor restartable and hash verified`
10. `refactor: route all backup-visible writers through local-data guards`
11. `test: add barrier-based backup concurrency and crash-owner recovery`
12. `fix: enforce schema layout collisions and duplicate IDs before transaction`
13. `test: replace permissive manifest fixtures with valid targeted failures`
14. `test: add canonical server telemetry and false-success executor mode`
15. `fix: unify output-file and ordinary execution outcome mapping`
16. `ci: make Windows setup shell-neutral and architecture-aware`
17. `test: add Windows-native process and filesystem evidence`
18. `docs: record same-commit CI evidence and close Phase 11C`

Do not combine transaction protocol changes, Windows CI rewrites, and documentation closure into one opaque commit.

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
cargo test --test backup_snapshot_concurrency --features test-support -- --test-threads=1
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test readonly_no_recovery --features test-support -- --test-threads=1
cargo test --test execution_outcomes --features test-support -- --test-threads=1
cargo test --test update_archive_security --features test-support -- --test-threads=1
cargo package -p snip-it --locked --allow-dirty
```

Production-seam checks:

```bash
cargo build --release --no-default-features
SNP_TEST_CREDENTIAL_FILE=/path/that/exists target/release/snp status --json
SNP_TEST_FAILPOINT=restore-after-first-write target/release/snp status --json
```

The production binary must not change behavior because of test-only variables.

GitHub Actions must execute the required final matrix on the exact final commit.

---

## 6. Explicit final closure checklist

### Status

- [ ] Phase 11 status names Phase 11C until completion.
- [ ] Final status references the actual final commit.
- [ ] Final status includes the GitHub Actions workflow URL.
- [ ] Linux, macOS, and Windows job conclusions are recorded.
- [ ] No release claim relies only on commit messages or test names.

### Lock ownership

- [ ] Existing owner identity is observed using `existing.pid`.
- [ ] Live owner cannot be stolen.
- [ ] Identity-unavailable live owner is treated conservatively.
- [ ] Dead/reused owner reclaim loops through exclusive create.
- [ ] Concurrent reclaimers produce one owner.
- [ ] Malformed records are quarantined, not destroyed.
- [ ] Wrong nonce/PID/start-token/transaction ID cannot release a lock.
- [ ] Local-data lock uses the same crash-recoverable primitive.

### Transaction context

- [ ] Lock hierarchy is documented and enforced.
- [ ] Restore cannot recover its own active journal.
- [ ] Internal guarded writes cannot be called without guards.
- [ ] Foreign interrupted transactions block new mutations.
- [ ] Backup/restore and backup/edit tests complete without deadlock.

### Restore preparation

- [ ] Schema/layout/domain validation completes before locks and transaction artifacts.
- [ ] Complete intended output bytes are computed before commit.
- [ ] Durable backups exist before live writes.
- [ ] Durable staged files exist before live writes.
- [ ] Journal contains typed actions, hashes, paths, permissions, and durability.
- [ ] `BackupsDurable` is persisted only after artifact verification.

### Commit and pending

- [ ] Commit cursor represents completed verified positions.
- [ ] Progress is persisted after each destination.
- [ ] Destination intended hash is verified.
- [ ] Replay after crash is idempotent.
- [ ] Committed content cannot lose pending intent.
- [ ] Recovery reuses the recorded pending generation.
- [ ] Successful restore produces exactly one pending generation.
- [ ] No-op restore produces none.

### Rollback

- [ ] Rollback uses rollback-order positions.
- [ ] Replace restores exact original bytes.
- [ ] Create removes the new destination.
- [ ] Delete restores the deleted destination.
- [ ] Original permissions are restored where supported.
- [ ] Hash or absence is verified after each action.
- [ ] Second crash during rollback resumes correctly.
- [ ] Missing/corrupt artifacts preserve evidence and return nonzero.
- [ ] Failed/rolled-back restore creates no pending generation.

### Backup coherence

- [ ] Writer inventory is complete.
- [ ] Every included-state writer uses local-data coordination.
- [ ] Library create/delete hold the lock across file and index changes.
- [ ] Restore and sync pull participate.
- [ ] Usage/sync settings participate when included.
- [ ] Barrier-based tests prove complete before- or after-state.
- [ ] Crashed lock owner is safely reclaimed.
- [ ] Partial backup output cannot appear complete.

### Manifest/domain

- [ ] Unsupported schema fails explicitly.
- [ ] Unsupported layout fails explicitly.
- [ ] Exact and portable collisions fail closed.
- [ ] Windows aliases fail on every host.
- [ ] Duplicate snippet IDs fail before transaction creation.
- [ ] Index/library inconsistency fails before transaction creation.
- [ ] Negative fixtures have valid sizes and hashes.
- [ ] No test accepts either success or failure.
- [ ] Validation failures create no artifacts or writes.

### Sync/read-only evidence

- [ ] Headline test uses server recording telemetry.
- [ ] Exactly one canonical request is asserted.
- [ ] Expected device and library identities are asserted.
- [ ] Encrypted payload and revision transition are asserted.
- [ ] Maximum server concurrency is one.
- [ ] Server completion precedes pending clear.
- [ ] Quiet period shows no duplicate request.
- [ ] False-success/no-op executor preserves pending.
- [ ] Read-only commands produce zero lifecycle events and server requests.
- [ ] E2E passes alone and in full workspace runs.

### Execution

- [ ] Output-file success exits `0`.
- [ ] Output-file child nonzero returns child code.
- [ ] Output-file timeout exits `8`.
- [ ] Output-file spawn failure exits `8`.
- [ ] No-code/signal termination exits `8`.
- [ ] Generic application/config failures remain exit `1`.
- [ ] Output-file partial-output policy is documented and tested.

### Test-only boundaries

- [ ] Production build ignores test credential variable.
- [ ] Production build ignores restore failpoints.
- [ ] Test credentials remain deterministic across subprocesses.
- [ ] Secrets do not appear in argv, logs, events, journals, or diagnostics.

### Windows CI

- [ ] Actual failing job data was recorded before correction.
- [ ] Workspace cargo commands are shell-neutral.
- [ ] `protoc` setup is centralized, exact-version, architecture-aware, and verified.
- [ ] Windows stack issue is fixed at root cause or with a present documented linker config.
- [ ] No PID guess is used.
- [ ] Windows-native termination is tested.
- [ ] Handles and child processes are cleaned up.
- [ ] Filesystem capability absence has alternate production-classifier proof.
- [ ] Package/install smoke succeeds from unpacked `.crate`.
- [ ] No release-blocking step is permissive.
- [ ] All required Windows jobs pass on the final commit.

### Architecture

- [ ] One installed `snp` binary remains the client architecture.
- [ ] Auto-sync workers remain one-shot subprocesses.
- [ ] No daemon, helper binary, plugin runtime, workflow engine, or database expansion was introduced.

---

## 7. Release decision rule

The release decision is binary.

Mark Phase 11 and the correctness program complete only when:

1. every applicable checkbox above is satisfied;
2. production code matches the documented lock, transaction, backup, credential, and execution contracts;
3. adversarial tests prove the intended failure or recovery condition directly;
4. Linux, macOS, and Windows jobs pass on the same final commit;
5. the closure status includes the workflow URL and exact job conclusions.

The program remains open if any of the following is true:

- a live lock can be reclaimed using the contender’s identity;
- restore can encounter its own transaction through the global mutation gate;
- commit progress is persisted before a write;
- rollback uses original reverse indices rather than rollback-order positions;
- committed restore content can exist without durable pending intent;
- backup coordination excludes any writer of included state;
- manifest tests use invalid hashes or accept either outcome;
- the no-op regression is only an unreachable-server test;
- output-file spawn failure exits `1`;
- Windows evidence is missing, flaky, permissively skipped, or from a different commit;
- closure documentation claims a configuration or test that is absent from the repository.

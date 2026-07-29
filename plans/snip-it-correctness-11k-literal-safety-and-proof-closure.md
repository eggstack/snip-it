# Phase 11K — Literal Safety and Proof Closure

Status: READY FOR IMPLEMENTATION

Authoritative predecessor: `plans/snip-it-correctness-11j-recovery-serialization-proof-and-reporting-closure.md`

Corrective baseline: `bf6f941842728888afd9609d8f8e8872f1796a82`

This plan is the authoritative remaining-work plan for Phase 11 correctness closure.

---

## 1. Why another corrective pass is required

Phase 11J was implemented broadly, but several requirements were interpreted as optional, diagnostic, or approximately equivalent. That is the communication failure this plan is designed to eliminate.

Examples of incorrect substitutions that occurred:

- A requirement to reject symlinked journals became a test saying rejection **or following the symlink** was acceptable.
- A requirement for mandatory sync identity became a diagnostic `eprintln!` when identity was absent.
- A requirement for one success plus one deterministic failure became a test asserting zero failures and accepting exit code 0 or 1.
- A requirement for exact execution became a dry-run classification check.
- A requirement to validate every artifact reference became validation only when the referenced file currently existed.
- A requirement for locked terminal removal became an unlocked direct call from the startup mutation gate.
- A requirement to propagate directory durability errors became a helper that ignored the actual `fsync` return value.

Phase 11K closes these literal gaps. The implementation agent must implement the specified behavior, not a nearby behavior that happens to make existing tests pass.

---

## 2. Non-negotiable interpretation rules

These rules override ambiguous comments, existing permissive tests, and earlier completion claims.

1. **“Must” means a hard assertion or propagated error.** A diagnostic log is not compliance.
2. **“Exactly one” means `assert_eq!(count, 1)`.** It does not mean nonempty, `>= 1`, or “at most one.”
3. **“Reject” means return an error and preserve state.** It does not mean follow, ignore, warn, or accept either result.
4. **“Execute” means invoke the mutation/recovery path.** A dry-run or classification report is not execution proof.
5. **“Deterministic failure” means a controlled failure seam that always triggers.** A race, permission assumption, or fresh rescan is not deterministic.
6. **“Under lock” means load, validate, classify, and mutate while the authoritative lock is held.** A scan before the lock is advisory only.
7. **“Every path” includes missing paths.** Lexical containment must be checked before `exists()` is used to classify absence.
8. **“Propagate” means the caller receives `Err`.** `let _ = ...`, warning-only behavior, or unconditional `Ok(())` is not propagation.
9. **A test name is not evidence.** The test body must perform and assert the named scenario.
10. **A passing suite does not override a failed source review.** Closure requires both semantic source review and passing tests.

### Prohibited assertion patterns in Phase 11K tests

Do not add or retain these patterns for required behavior:

```rust
assert!(code == 0 || code == 1);
assert!(count >= 1);
assert!(max_concurrent <= 1);
if missing_required_field {
    eprintln!("NOTE: ...");
}
if let Ok(value) = parse_result {
    // assertions only here
}
```

Use exact forms instead:

```rust
assert_eq!(code, 1);
assert_eq!(count, 1);
assert_eq!(max_concurrent, 1);
assert_eq!(observed_generation, expected_generation);
let value = parse_result.expect("required JSON must parse");
assert!(required_field.is_some(), "required field must be populated");
```

---

## 3. Preserved architecture and scope boundaries

Preserve all of the following:

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- typed restartable transaction cleanup;
- generation-conditional executor-owned pending clear;
- one Linux correctness CI job;
- macOS and Windows smoke-only jobs;
- deep crash/protocol verification performed locally;
- manual dependency-ordered crates.io publishing;
- no automated publish workflow;
- no GitHub Release automation;
- no new evidence registry;
- no new database, daemon, task queue, or orchestration layer.

This is a narrow correctness and proof pass. Do not refactor unrelated command, sync, storage, or release code.

---

## 4. Execution protocol for a smaller model

Complete workstreams in order. Do not combine them into one large speculative edit.

For every workstream:

1. read the named current functions and tests;
2. write or tighten the failing test first where practical;
3. run only the focused test command;
4. implement the smallest production change;
5. rerun the focused test;
6. run `cargo fmt --all -- --check` and focused clippy for touched targets;
7. commit before starting the next workstream.

Do not update the closure status to `COMPLETE` during implementation. The status remains `INCOMPLETE / REOPENED` until the final verification workstream.

---

# Workstream A — Validate journal filename identity during scanning

## Goal

Make complete journal scanning reject malformed IDs and filename/internal-ID mismatches before repair formatting or action generation can use them.

## Current problem

`scan_transaction_journals` parses `txn-*.toml` and appends the parsed `TransactionJournal` without checking:

- whether the filename-derived ID is valid;
- whether the internal `journal.id` is valid;
- whether the internal ID matches the filename-derived ID;
- whether the ID is safe to display or slice.

Later code uses byte slicing such as `&journal.id[..8]`. A short or non-ASCII ID can panic. A mismatched internal ID can generate a repair action for a journal path that does not exist.

## Required implementation

### A1. Parse the ID from the filename

For each file named `txn-<id>.toml`:

```rust
fn journal_id_from_path(path: &Path) -> SnipResult<String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid_journal_name(path))?;

    let id = stem
        .strip_prefix("txn-")
        .ok_or_else(|| invalid_journal_name(path))?;

    validate_transaction_id(id)?;
    Ok(id.to_owned())
}
```

The actual helper names may differ. The behavior may not.

### A2. Validate parsed identity

After TOML parsing:

```rust
let filename_id = journal_id_from_path(&path)?;
let journal: TransactionJournal = toml::from_str(&content)?;
validate_transaction_id(&journal.id)?;

if journal.id != filename_id {
    return_or_record_corrupt(
        path,
        format!(
            "journal ID mismatch: filename contains {filename_id}, body contains {}",
            journal.id
        ),
    );
}
```

The scanner already collects corrupt journals. Identity failures should be added to `inventory.corrupt`; they must not be added to `inventory.journals`.

### A3. Use safe ID display

Replace all direct byte slicing of untrusted journal IDs.

Bad:

```rust
&journal.id[..8]
&journal.id[..8.min(journal.id.len())]
```

Required helper:

```rust
fn short_transaction_id(id: &str) -> String {
    id.chars().take(8).collect()
}
```

Use the full validated ID for machine-readable JSON. Use the safe short form only for human diagnostics.

## Required tests

Add scanner tests with exact outcomes:

1. valid filename and matching internal ID enters `journals`;
2. valid filename with mismatched internal ID enters `corrupt`, not `journals`;
3. empty internal ID enters `corrupt`;
4. internal ID containing `/`, `\\`, or `..` enters `corrupt`;
5. short internal ID does not panic;
6. non-ASCII internal ID does not panic;
7. `snp repair --dry-run --json` against a malformed journal emits one valid JSON document and reports an unsafe/corrupt item;
8. malformed journal ID never becomes a safe repair action.

## Anti-example

This is not acceptable:

```rust
if journal.id != filename_id {
    tracing::warn!("mismatch");
}
journals.push(journal);
```

The mismatched journal must not enter the valid journal collection.

## Acceptance criteria

- every valid scanned journal has a valid ID matching its filename;
- malformed and mismatched journals are reported as corrupt;
- no untrusted ID is byte-sliced;
- repair JSON remains parseable for malformed journal fixtures;
- malformed identity cannot produce an automatically safe repair item;
- no panic occurs for short or Unicode IDs.

---

# Workstream B — Validate every artifact reference before classifying any recovery state

## Goal

Make artifact path safety validation universal across all transaction states and independent of whether the referenced file currently exists.

## Current problem

Artifact inspection is currently used to distinguish legacy terminal cleanup states, but interrupted rollback states can be classified without validating their backup and staged paths. Rollback then reads `backup_path` directly.

The current inspection also checks containment only inside `if path.exists()`. A missing path outside the transaction artifact root can therefore bypass containment validation.

## Required design

Separate **path safety** from **artifact presence**.

Recommended types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactPresence {
    None,
    Present,
}

pub fn inspect_journal_artifacts(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<ArtifactPresence>;
```

A `SnipResult<bool>` is acceptable, but the implementation must perform all validation described below.

## Required validation order

For the transaction artifact root, every `backup_path`, and every `durable_staged_path`:

1. derive the exact per-transaction artifact root;
2. reject lexical traversal or an out-of-root reference **before checking existence**;
3. reject a symlinked artifact root;
4. reject a symlinked existing child path;
5. canonicalize existing root and existing child where possible and re-check containment;
6. classify existing safe paths as `Present`;
7. classify missing safe in-root paths as absent;
8. return `Err` for any unsafe reference;
9. preserve the journal and all artifacts on error.

## Lexical containment example

Do not rely only on `canonicalize`, because missing paths cannot be canonicalized.

A small lexical helper is required. One acceptable shape:

```rust
fn lexically_within(root: &Path, child: &Path) -> bool {
    if !root.is_absolute() || !child.is_absolute() {
        return false;
    }

    let normalized_root = normalize_without_parent(root)?;
    let normalized_child = normalize_without_parent(child)?;
    normalized_child.starts_with(&normalized_root)
}
```

Where normalization rejects `Component::ParentDir` rather than resolving it.

An alternative is acceptable if these cases are exact:

| Reference | Required result |
|---|---|
| existing file inside root | safe, present |
| missing file inside root | safe, absent |
| existing file outside root | error |
| missing file outside root | error |
| path containing `..` | error |
| symlink inside root | error |
| symlinked root | error |

## Universal classification rule

`classify_journal_recovery` must call artifact inspection for **every** parsed journal before matching on transaction state.

Required shape:

```rust
pub fn classify_journal_recovery(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<RecoveryClass> {
    let artifacts = inspect_journal_artifacts(transaction_dir, journal)?;

    match &journal.state {
        TransactionState::Prepared
        | TransactionState::BackupsDurable
        | TransactionState::Committing { .. }
        | TransactionState::RollingBack { .. } => Ok(RecoveryClass::Rollback),
        TransactionState::CommittedLocal { .. } => Ok(RecoveryClass::FinalizeCommittedLocal),
        TransactionState::CleaningUp { .. } => Ok(RecoveryClass::ResumeCleanup),
        TransactionState::Committed => terminal_commit_class(artifacts),
        TransactionState::RolledBack => terminal_rollback_class(artifacts),
        TransactionState::Failed(_) => Ok(RecoveryClass::UnsafeFailed),
    }
}
```

The artifact result may be unused for nonterminal class selection, but the validation must still run.

## Defense in depth before destructive reads

Before rollback reads a backup, call the same path validation helper again while recovery is under lock.

Required principle:

```rust
validate_artifact_reference(&artifact_root, backup, "backup_path")?;
let bytes = fs::read(backup)?;
```

Do not assume classification performed earlier is sufficient for a later destructive operation.

## Required tests

Add exact tests for each state family:

1. `Prepared` with existing out-of-root backup is rejected;
2. `Prepared` with missing out-of-root backup is rejected;
3. `RollingBack` with symlinked backup is rejected on Unix;
4. `CommittedLocal` with out-of-root durable staged path is rejected;
5. `CleaningUp` with symlinked artifact root is rejected;
6. legacy `Committed` with safe missing in-root backup remains classifiable;
7. missing in-root path is safe absence;
8. path containing `..` is rejected even when missing;
9. mutation gate blocks when any journal has unsafe artifact references;
10. `snp repair --dry-run --json` reports unsafe/manual and does not mark the action safe;
11. exact rollback refuses an unsafe backup and leaves the destination unchanged;
12. exact rollback accepts a valid in-root backup and succeeds.

## Anti-examples

Not acceptable:

```rust
if backup.exists() {
    validate_contained_path(root, backup)?;
}
```

Required:

```rust
validate_artifact_reference(root, backup)?;
if backup.exists() {
    // safe to read
}
```

Not acceptable:

```rust
match journal.state {
    Prepared => Rollback,
    Committed => inspect_artifacts(...),
}
```

Inspection must run before the state match.

## Acceptance criteria

- every recovery class is preceded by complete artifact path validation;
- missing out-of-root references fail closed;
- rollback validates a backup immediately before reading it;
- unsafe paths block startup mutation;
- unsafe paths are unsafe/manual in repair output;
- journal and live destination remain unchanged on validation error;
- tests cover existing, missing, symlinked, traversing, in-root, and out-of-root paths.

---

# Workstream C — Route all terminal-journal removal through locked exact recovery

## Goal

Eliminate the unlocked startup deletion path for `RemoveTerminalJournal`.

## Current problem

`recover_transaction_by_id` loads and reclassifies under the transaction lock. The mutation gate bypasses it for terminal journals by calling `remove_terminal_journal` directly after an unlocked scan.

A journal can change or acquire artifacts between scan/classification and deletion.

## Required implementation

### C1. Create a locked internal removal helper

Required shape:

```rust
fn remove_terminal_journal_locked(
    transaction_dir: &Path,
    transaction_id: &str,
    _lock: &TransactionLock,
) -> SnipResult<()> {
    // path validation, symlink rejection, removal, parent sync
}
```

Only `recover_transaction_by_id` may call this helper after:

1. acquiring the transaction lock;
2. loading the exact journal under lock;
3. validating filename/internal identity;
4. validating all artifact references;
5. classifying as `RemoveTerminalJournal` under lock;
6. confirming the caller's expected class is still `RemoveTerminalJournal`.

### C2. Mutation gate must use exact recovery

Bad:

```rust
remove_terminal_journal(transaction_dir, &journal.id)?;
```

Required:

```rust
recover_transaction_by_id(
    sync_state_dir,
    transaction_dir,
    &journal.id,
    RecoveryClass::RemoveTerminalJournal,
)?;
```

### C3. Multiple terminal journals

When the inventory contains only terminal journals with no artifacts:

- process them one at a time through exact recovery;
- acquire/release the existing transaction lock per item;
- revalidate each item under lock;
- stop and return `Err` on the first stale or unsafe item;
- do not directly delete remaining journals after an error.

When one recoverable nonterminal journal and removable terminal journals coexist:

1. fail first on corrupt or unsafe journals;
2. recover the single nonterminal journal through exact recovery;
3. rescan;
4. remove remaining terminal journals through exact recovery;
5. return success only after the rescan is clean.

A rescan is required because the first recovery may change transaction state.

## Required deterministic race test

Add a test-support barrier before authoritative under-lock load.

One acceptable mechanism:

- recovery thread/process attempts terminal removal and blocks before lock acquisition or immediately after lock acquisition but before load;
- test changes the journal from terminal/no-artifacts to a class owning artifacts before the authoritative load;
- release the barrier;
- exact recovery must return stale-action or new-class error;
- journal and newly created artifacts must remain.

Do not use sleeps as the synchronization mechanism.

## Other required tests

1. one terminal journal is removed through exact recovery;
2. two terminal journals are each removed through exact recovery;
3. terminal journal changed to `Prepared` before authoritative load is not deleted;
4. artifacts created before authoritative load prevent terminal deletion;
5. mutation gate does not call direct terminal deletion;
6. a symlinked terminal journal is rejected;
7. a deletion failure returns error and preserves evidence;
8. one failed terminal removal stops further removals.

## Acceptance criteria

- startup and repair use the same exact recovery entry point;
- no mutation-gate branch calls terminal deletion directly;
- terminal state is loaded and classified under lock immediately before deletion;
- stale state or new artifacts prevent deletion;
- multiple terminal journals are handled sequentially and safely;
- deterministic race tests prove revalidation.

---

# Workstream D — Propagate parent-directory durability failures

## Goal

Make terminal journal cleanup return an error when parent directory synchronization fails.

## Current problem

The current helper opens the parent directory but discards the `libc::fsync` return value.

Bad:

```rust
unsafe {
    let _ = libc::fsync(dir.as_raw_fd());
}
Ok(())
```

## Required Unix implementation

```rust
#[cfg(unix)]
fn fsync_parent_dir(path: &Path) -> SnipResult<()> {
    use std::os::fd::AsRawFd;

    let parent = path.parent().unwrap_or(path);
    let dir = std::fs::File::open(parent)
        .map_err(|e| SnipError::io_error("open parent dir for fsync", parent, e))?;

    let rc = unsafe { libc::fsync(dir.as_raw_fd()) };
    if rc != 0 {
        return Err(SnipError::io_error(
            "fsync parent directory",
            parent,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}
```

Using `File::sync_all()` is also acceptable if it reliably propagates failure on supported Unix targets.

Windows may remain a documented no-op if that is the established platform policy.

## Required deterministic failure seam

Add a compile-time-gated test-only I/O error injection at the parent-sync operation.

Preferred reuse:

- extend the existing test error-injection mechanism;
- do not add a production environment-variable check;
- production builds must compile without the seam.

Example test-only behavior:

```rust
#[cfg(feature = "test-support")]
crate::test_failpoints::maybe_io_error("terminal-journal-parent-sync")?;
```

The exact helper may differ.

## Required tests

1. successful removal plus successful parent sync returns `Ok`;
2. injected parent-sync failure returns `Err`;
3. the error message identifies parent-directory sync;
4. repair reports the item as failed and exits 1 when another item succeeds;
5. production seam test proves the injection control is absent without `test-support`;
6. no `let _ = libc::fsync` remains in transaction durability code.

## Important state rule

If the journal file was removed but parent sync failed, return `Err` and report uncertainty. Do not recreate the journal from stale memory. The operation is not proven durable and must not be reported as successful.

## Acceptance criteria

- actual Unix fsync failures propagate;
- deterministic tests do not rely on filesystem permissions;
- repair counters include the failure;
- process exit code is nonzero;
- production binaries contain no test-only control.

---

# Workstream E — Replace weak recovery tests with literal execution proofs

## Goal

Make every Phase 11K recovery test execute the named operation and assert one exact result.

## E1. Exact isolation matrix

Create a table-driven or individually named test for these classes:

| Selected transaction A | Unrelated transaction B | Required result |
|---|---|---|
| `Prepared` / rollback | `Prepared` | A removed after rollback; B byte-for-byte unchanged |
| `CleaningUp` / resume | `Prepared` | A cleanup completes; B unchanged |
| `CommittedLocal` / finalize | `Prepared` | A pending finalization and cleanup complete; B unchanged |
| legacy `Committed` with artifacts | `Prepared` | A artifacts and journal removed; B unchanged |
| legacy `RolledBack` with artifacts | `Prepared` | A artifacts and journal removed; B unchanged |
| terminal no-artifacts | `Prepared` | A journal removed; B unchanged |

For B, snapshot:

- journal bytes;
- referenced artifact bytes;
- live destination bytes if present.

After recovering A, compare all snapshots exactly.

A dry-run report is not sufficient.

## E2. Deterministic CLI partial failure

Add one test-only failure selector for repair application.

Recommended environment contract, compiled only with `test-support`:

```text
SNP_TEST_REPAIR_FAIL_TRANSACTION_ID=<exact transaction id>
```

In `apply_repair`, immediately before calling exact recovery for a transaction action:

```rust
#[cfg(feature = "test-support")]
if injected_failure_id() == Some(transaction_id.as_str()) {
    return Err(SnipError::runtime_error(
        "injected repair failure",
        Some(transaction_id),
    ));
}
```

Create two safe repair items:

- transaction A succeeds;
- transaction B is selected by the deterministic failure seam.

Run:

```text
snp repair --apply --json
```

Required exact assertions:

```rust
assert_eq!(exit_code, 1);
assert_eq!(json["applied"], 1);
assert_eq!(json["failed"], 1);
assert_eq!(json["skipped"], 0);
assert_eq!(json["exit_status"], "partial_failure");
assert!(!journal_a.exists());
assert!(journal_b.exists());
```

Do not accept exit code 0. Do not assert `applied >= 1`. Do not rescan into a different successful action.

## E3. Strict symlink scanner test

Replace the permissive scanner test.

Required Unix assertion:

```rust
assert_eq!(inventory.journals.len(), expected_real_journals);
assert_eq!(inventory.corrupt.len(), 1);
assert!(inventory.corrupt[0].error.contains("symlink"));
```

Delete comments saying following the symlink is acceptable.

## E4. Unknown and malformed exact API tests

Call `recover_transaction_by_id` directly for:

- unknown valid ID;
- empty ID;
- forward-slash traversal;
- backslash traversal;
- `..` traversal;
- filename/internal-ID mismatch;
- symlinked journal.

For every case:

- assert `Err`;
- assert unrelated journal bytes unchanged;
- assert unrelated artifacts unchanged;
- assert no live destination mutation.

## E5. JSON strictness

For every JSON process test:

```rust
let json: serde_json::Value = serde_json::from_slice(&output.stdout)
    .expect("repair command must emit exactly one valid JSON document");
```

Do not use `unwrap_or_default`, `if let Ok`, or optional assertions for required fields.

## Required grep gate

Before closing this workstream, run:

```bash
rg -n \
  'Both are acceptable|NOTE:|code == 0 \\|\\| code == 1|applied >=|if let Ok\\(json|unwrap_or_default\\(\\)' \
  tests/repair_transactions.rs src/transaction.rs
```

Expected result: no targeted permissive pattern remains. Legitimate unrelated uses must be reviewed manually rather than blindly deleted.

## Acceptance criteria

- every exact recovery class has an execution isolation test;
- unrelated transaction state is byte-for-byte unchanged;
- partial failure always produces exactly one success and one failure;
- CLI exit code is exactly 1 for partial failure;
- symlink scanner test requires rejection;
- malformed-ID tests call the exact API;
- required JSON always parses or the test fails;
- no permissive fallback remains in the targeted tests.

---

# Workstream F — Complete the exact sync identity and pending-clear proof

## Goal

Prove one mutation produces exactly one identified sync operation and exactly one matching pending clear after successful remote completion.

## Current problem

The observer test pairs start and finish by sequence, but mandatory identity fields are diagnostics. Pending-clear events are only required to be nonempty, generation is only checked as numeric, concurrency is `<= 1`, and the unreachable test does not assert zero pending-clear events.

## F1. Wire identity into the observer

Locate the sync/push server handler where authentication and library resolution are complete.

After resolving:

- authenticated user ID;
- authenticated device ID;
- target library ID;

call the existing observer update hook for the request sequence.

Expected shape:

```rust
observer.update_request_ids(
    request_sequence,
    Some(authenticated_user_id.clone()),
    Some(authenticated_device_id.clone()),
    Some(target_library_id.clone()),
);
```

If the hook currently updates only a separate map, ensure `observer.starts()` returns the updated record. Do not merely log the IDs.

The no-op production observer must remain cheap and behaviorally inert.

## F2. Reset observer state after registration

Add or use an `InMemoryObserver::reset()` method after registration and before the measured mutation.

Reset must clear:

- starts;
- finishes;
- in-flight count;
- maximum concurrency;
- sequence-visible request records used by the test.

The request sequence generator may remain monotonic. The test must not depend on sequence starting at 1.

Required setup order:

1. start server;
2. register client;
3. enable auto-sync;
4. reset observer;
5. clear test event sink;
6. record remote state R0;
7. perform exactly one mutation.

## F3. Capture the exact pending generation

Use the worker `cycle_started` event or read the pending marker before clear to capture generation G.

Hard assertion:

```rust
let generation = cycle.generation.expect("cycle_started must include generation");
```

Do not infer G from “any numeric generation” in the clear event.

## F4. Exact observer assertions

After sync settles:

```rust
let sync_starts: Vec<_> = observer
    .starts()
    .into_iter()
    .filter(|s| matches!(s.operation.as_str(), "sync" | "push"))
    .collect();
assert_eq!(sync_starts.len(), 1);

let start = &sync_starts[0];
assert!(start.authenticated_user_id.as_deref().is_some_and(|v| !v.is_empty()));
assert!(start.authenticated_device_id.as_deref().is_some_and(|v| !v.is_empty()));
assert!(start.target_library_id.as_deref().is_some_and(|v| !v.is_empty()));

let finishes: Vec<_> = observer
    .finishes()
    .into_iter()
    .filter(|f| f.sequence == start.sequence)
    .collect();
assert_eq!(finishes.len(), 1);
assert!(finishes[0].success);
assert_eq!(observer.max_concurrent(), 1);
```

These are hard assertions. Delete diagnostic fallback branches.

Where possible, compare observed IDs to values known from the test environment or server database rather than only checking nonempty.

## F5. Exact pending-clear event assertions

Filter by component, event, and generation G:

```rust
let clears: Vec<_> = events
    .iter()
    .filter(|e| {
        e.component == "executor"
            && e.event == "pending_cleared"
            && e.generation == Some(generation)
    })
    .collect();

assert_eq!(clears.len(), 1);
let clear = clears[0];
assert!(finish.finished_at_unix_ms <= clear.at_unix_ms);
assert!(!pending_marker.exists());
```

Also parse detail strictly if detail remains part of the event contract:

```rust
let detail = clear.detail.as_ref().expect("pending clear detail required");
let detail: serde_json::Value = serde_json::from_str(detail).expect("valid detail JSON");
assert_eq!(detail["generation"].as_u64(), Some(generation));
```

## F6. Quiet-period assertion

After the quiet period:

```rust
assert_eq!(sync_start_count_after_quiet, 1);
assert_eq!(matching_clear_count_after_quiet, 1);
```

Do not compare two previously broad counts that could both include setup traffic.

## F7. Unreachable server assertion

The unreachable-server test must set up and clear the event sink before mutation.

After the worker cycle and retry window:

```rust
assert!(pending_marker.exists());
assert_eq!(
    sink.count_events("executor", "pending_cleared"),
    0,
    "unreachable sync must never clear pending"
);
```

Also assert remote server state remains unchanged when a server fixture is used.

## F8. Failed unrelated request test

Add one focused observer unit/integration test:

- record a failed registration or unrelated request finish;
- record one successful sync start/finish pair;
- verify pairing by sequence selects only the sync finish;
- verify the failed unrelated finish cannot satisfy the sync assertion.

## Required grep gate

```bash
rg -n \
  'IDs not populated|diagnostic, not a hard failure|NOTE:|max_concurrent <=|pending_cleared_events.is_empty|generation.*is_number' \
  tests/deterministic_e2e.rs snip-sync/src
```

Expected result: no weak substitute remains in the headline proof.

## Acceptance criteria

- observer identity is populated by real handler data;
- observer state is reset after registration;
- exactly one sync/push start is observed;
- exactly one matching finish is observed;
- the matching finish is successful;
- user, device, and library identity are mandatory hard assertions;
- maximum measured sync concurrency equals exactly 1;
- exact generation G is captured before clear;
- exactly one `pending_cleared` event exists for G;
- successful finish precedes or equals clear timestamp;
- pending marker is absent after the matching clear;
- quiet period leaves one start and one clear;
- unreachable server leaves pending and emits zero clear events;
- all test-only observation controls remain compile-time gated.

---

# Workstream G — Make repair process semantics provable, not inferred

## Goal

Ensure repair output, exit codes, and action application remain consistent after the new failure and safety checks.

## Required process matrix

Implement or retain process tests with exact expected results:

| Scenario | Exit | `exit_status` | applied | failed | skipped |
|---|---:|---|---:|---:|---:|
| no issues, `--apply --json` | 0 | `clean` | 0 | 0 | 0 |
| one successful safe item | 0 | `repaired` | 1 | 0 | 0 |
| one deterministic failed safe item | 1 | `partial_failure` | 0 | 1 | 0 |
| one success + one deterministic failure | 1 | `partial_failure` | 1 | 1 | 0 |
| unsafe item only | 2 | `unsafe_only` | 0 | 0 | 1 |
| dry run with issue | 0 | `dry_run` | 0 | 0 | exact issue count or defined skipped contract |

If current skipped semantics differ, document and assert one stable contract. Do not leave it implicit.

## Exact stdout rule

For every JSON mode:

- stdout contains exactly one JSON document;
- progress and failure details may go to stderr;
- no pre-application JSON is emitted;
- JSON status agrees with process exit code.

## Unsafe inspection behavior

An unsafe journal caused by identity, symlink, or containment validation must:

- appear in `items`;
- have `safe=false`;
- never be passed to `apply_repair`;
- remain on disk;
- contribute to `skipped` in apply mode;
- produce `unsafe_only` when no safe item exists;
- coexist with `repaired` or `partial_failure` according to the explicitly documented mixed safe/unsafe contract.

For a mixed safe success plus unsafe skipped item, choose and document one stable status. Recommended: `repaired` with `skipped=1` and exit 0, because all selected safe work succeeded and unsafe work was intentionally not selected. Do not conflate skipped unsafe work with failed safe work.

## Acceptance criteria

- the process matrix is covered exactly;
- failure injection produces real CLI partial failure;
- JSON and exit code agree;
- unsafe evidence is never mutated;
- no test accepts multiple exit codes;
- no required JSON field is optional in tests.

---

# Workstream H — Semantic review gates and truthful closure status

## Goal

Prevent another premature closure caused by equating passing tests with complete requirements.

## H1. Reopen status before implementation

Update `plans/snip-it-correctness-11-closure-status.md` to:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11k-literal-safety-and-proof-closure.md
Corrective baseline: bf6f941842728888afd9609d8f8e8872f1796a82
Final implementation commit: pending
```

Retain the simplified CI and manual crates.io decisions.

## H2. Source-review checklist

Before final closure, inspect the final source and answer each question with a literal yes/no:

1. Does scanner validation reject filename/internal-ID mismatch?
2. Can any untrusted ID still be byte-sliced?
3. Does classification validate artifact paths for every state?
4. Is lexical containment checked before existence?
5. Does rollback validate a backup immediately before reading?
6. Can mutation gate directly delete a terminal journal without exact recovery?
7. Is terminal state reloaded and reclassified under lock immediately before removal?
8. Does Unix parent fsync return an error on nonzero return code?
9. Does the partial-failure process test assert exit 1, applied 1, failed 1?
10. Does the scanner symlink test require rejection?
11. Do exact cleanup/finalization tests execute recovery rather than dry-run?
12. Are sync user/device/library IDs hard assertions?
13. Is sync concurrency asserted equal to 1?
14. Is pending-clear generation compared to captured G?
15. Is exactly one pending-clear event asserted?
16. Does unreachable-server proof assert zero pending-clear events?

Any “no” keeps Phase 11 open even if all tests pass.

## H3. Prohibited-pattern grep

Run and manually inspect:

```bash
rg -n \
  'Both are acceptable|NOTE:|diagnostic, not a hard failure|code == 0 \\|\\| code == 1|applied >=|max_concurrent <=|let _ = libc::fsync|\\[..8\\]' \
  src tests snip-sync/src
```

Expected: no Phase 11K violation remains. Do not blindly remove unrelated legitimate matches; inspect each result.

## H4. Closure status rule

The final status may say `COMPLETE / CLOSED` only when:

- all source-review answers are yes;
- all focused tests pass;
- `scripts/check.sh` passes;
- `scripts/release-check.sh verify` passes from a clean checkout;
- changed crates pass publish dry-run in dependency order;
- Linux correctness, macOS smoke, and Windows smoke are actually observed passing for the exact final implementation commit;
- the final implementation SHA is the last production/test change, not an earlier commit followed by corrections.

A later status-only commit may reference the final implementation SHA, but must not claim CI evidence unavailable to the maintainer.

If CI evidence is unavailable, record:

```text
Candidate implementation commit: <sha>
CI verification: pending or unavailable
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
```

Do not infer CI success from local tests or an empty connector response.

## Acceptance criteria

- status is reopened before code changes;
- Phase 11K is authoritative;
- completion is based on source semantics plus tests;
- no unavailable CI result is claimed;
- final SHA is accurate;
- no release automation or heavy CI is reintroduced.

---

# 5. Required implementation sequence

Use this sequence so a smaller model can complete one bounded task at a time.

## Commit 1 — Reopen Phase 11 under Phase 11K

Files:

- `plans/snip-it-correctness-11-closure-status.md`

Only status and blocker truthfulness. No production code.

## Commit 2 — Validate scanned journal identity

Files:

- `src/transaction.rs`;
- transaction scanner tests;
- repair malformed-journal process test if needed.

Required outcome: malformed IDs and filename/body mismatch are corrupt, never safe.

## Commit 3 — Validate artifact paths for all states

Files:

- `src/transaction.rs`;
- focused transaction tests;
- repair unsafe-item tests.

Required outcome: existing and missing out-of-root references fail closed.

## Commit 4 — Route terminal removal through exact recovery

Files:

- `src/transaction.rs`;
- deterministic barrier/failpoint support;
- focused recovery tests.

Required outcome: no direct mutation-gate deletion.

## Commit 5 — Propagate parent sync failures

Files:

- `src/transaction.rs`;
- existing test-only error-injection module;
- focused durability and repair tests.

Required outcome: injected parent sync error reaches repair and process exit.

## Commit 6 — Replace weak repair tests

Files:

- `tests/repair_transactions.rs`;
- `src/commands/repair_cmd.rs` only for test-support injection or semantic correction;
- `src/lib.rs` only for existing test-support exports.

Required outcome: exact execution matrix and deterministic partial failure.

## Commit 7 — Complete sync observer identity and ordering proof

Files:

- `snip-sync/src/lib.rs` or exact handler file;
- `snip-sync/src/test_observer.rs`;
- `tests/support/recording_server.rs` only if necessary;
- `tests/deterministic_e2e.rs`;
- existing test event support.

Required outcome: one identified request, one matched finish, one matching clear.

## Commit 8 — Focused verification cleanup

Files:

- tests only where a legitimate exact assertion remains missing;
- `AGENTS.md` only if test commands changed.

Run source grep and focused matrix. Do not update closure yet.

## Commit 9 — Final verification and status

Files:

- `plans/snip-it-correctness-11-closure-status.md`.

Record actual results. Close only when all gates pass.

---

# 6. Focused verification commands

Run these after their respective workstreams and again on the final candidate.

## Formatting and static checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Transaction scanner, recovery, and safety

```bash
cargo test --lib transaction --all-features -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
```

## Sync exactness

```bash
cargo test --test deterministic_e2e --features test-support \
  test_observer_headline_sync_e2e -- --exact --test-threads=1

cargo test --test deterministic_e2e --features test-support \
  test_unreachable_server_preserves_pending -- --exact --test-threads=1
```

Use final test names if renamed, but retain one exact command for the positive proof and one for unreachable behavior.

## General focused CI check

```bash
bash scripts/check.sh
```

## Production seam

```bash
cargo build --release --no-default-features --target-dir target/production-seam
bash scripts/ci/test-production-seams.sh
```

The test-only failure selector, observer controls, and pending-clear event controls must be absent in this build.

## Full local release verification

```bash
bash scripts/release-check.sh verify
```

## Publish dry-runs for changed crates only

```bash
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

Run in dependency order and only when that crate changed or its version must be published.

## Clean checkout

```bash
git status --short
```

Expected output: empty.

---

# 7. Final binary acceptance checklist

Phase 11K is complete only when every box can be checked without qualification.

## Journal identity

- [ ] Scanner derives and validates ID from filename.
- [ ] Scanner validates internal ID.
- [ ] Filename and internal ID must match.
- [ ] Mismatch is corrupt, not safe.
- [ ] Short and Unicode IDs cannot panic diagnostics.
- [ ] No untrusted byte slicing remains.

## Artifact safety

- [ ] Artifact validation runs for every transaction state.
- [ ] Lexical containment runs before existence checks.
- [ ] Existing out-of-root reference is rejected.
- [ ] Missing out-of-root reference is rejected.
- [ ] Missing in-root reference is accepted as absent.
- [ ] Symlinked root is rejected.
- [ ] Symlinked child is rejected.
- [ ] Rollback revalidates before reading backup.
- [ ] Unsafe state blocks mutation and automatic repair.

## Terminal removal

- [ ] Mutation gate never directly deletes terminal journals.
- [ ] Exact recovery reloads and classifies under lock.
- [ ] Terminal deletion occurs only for under-lock `RemoveTerminalJournal`.
- [ ] Stale state prevents deletion.
- [ ] New artifacts prevent deletion.
- [ ] Multiple terminal journals are processed sequentially.
- [ ] First failure stops further deletion.

## Durability

- [ ] File removal errors propagate.
- [ ] Parent directory open errors propagate.
- [ ] Parent directory fsync errors propagate on Unix.
- [ ] Deterministic injected sync failure returns nonzero.
- [ ] Test injection is absent from production build.

## Repair proof

- [ ] Rollback isolation test executes rollback.
- [ ] Cleanup-resume isolation test executes cleanup.
- [ ] Committed-local isolation test executes finalization.
- [ ] Legacy commit and rollback isolation tests execute cleanup.
- [ ] Unrelated journal and artifacts are byte-for-byte unchanged.
- [ ] One-success/one-failure test asserts exit 1.
- [ ] It asserts applied 1 and failed 1.
- [ ] Symlink scanner test requires rejection.
- [ ] JSON tests parse exactly one required document.
- [ ] No multiple-outcome assertion remains.

## Sync proof

- [ ] Observer reset occurs after registration.
- [ ] Exactly one sync/push start is recorded.
- [ ] Exactly one matching finish is recorded.
- [ ] Matching finish is successful.
- [ ] Authenticated user ID is populated and asserted.
- [ ] Authenticated device ID is populated and asserted.
- [ ] Target library ID is populated and asserted.
- [ ] Maximum sync concurrency equals 1.
- [ ] Pending generation G is captured before clear.
- [ ] Exactly one pending-clear event matches G.
- [ ] Finish timestamp is not later than clear timestamp.
- [ ] Pending marker is absent after matching clear.
- [ ] Quiet period leaves exactly one start and one clear.
- [ ] Unreachable server leaves pending present.
- [ ] Unreachable server emits zero pending-clear events.

## Process and release

- [ ] Repair exit code and JSON status agree for every matrix row.
- [ ] `scripts/check.sh` passes.
- [ ] `scripts/release-check.sh verify` passes from a clean checkout.
- [ ] Publish dry-runs pass for changed crates.
- [ ] CI remains exactly Linux correctness plus macOS/Windows smoke.
- [ ] Actual publishing remains manual.
- [ ] No automated release workflow exists.
- [ ] Final closure status references the actual final implementation commit.
- [ ] CI claims are based on observed results for that exact commit.

Until every box is checked, Phase 11 remains `INCOMPLETE`, the correctness program remains `REOPENED`, and the repository must not be described as correctness-closed or release-ready.

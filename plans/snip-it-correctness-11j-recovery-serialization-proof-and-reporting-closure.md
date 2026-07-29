# Phase 11J — Recovery Serialization, Exact Proof, and Reporting Closure

Status: READY FOR IMPLEMENTATION

Authoritative predecessor: `plans/snip-it-correctness-11i-legacy-recovery-repair-and-verification-split-closure.md`

Corrective baseline: `36a142bbc0ae9340f83e177ef4b9252ce9c58145`

This plan is the authoritative remaining-work plan for Phase 11 correctness closure.

---

## 1. Purpose

Phase 11I materially improved transaction discovery, recovery classification, repair action typing, CI scope, local release verification, and publish dry-run handling. Direct review of the implementation after Phase 11I found a small set of remaining defects that prevent correctness closure:

1. transaction state is read and classified before the transaction lock is acquired;
2. `Failed` transaction journals do not block new mutations;
3. terminal journal deletion errors are ignored;
4. artifact ownership classification is infallible and does not fail closed on unsafe paths;
5. `repair --apply --json` emits the report before application and therefore reports stale counters;
6. several new repair tests inspect classification but do not execute the exact selected recovery path;
7. stale-action and partial-failure tests are permissive and do not deterministically trigger the named condition;
8. the sync observer E2E counts unrelated registration finishes and does not directly prove pending clear occurs after the matched successful sync finish;
9. release clean-tree enforcement does not reject untracked files.

Phase 11J closes only these defects. It must not redesign the transaction system, add a daemon, add another persistence layer, restore heavy CI, or reintroduce automated publishing.

---

## 2. Preserved architecture and non-goals

The implementation agent must preserve all of the following:

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- generation-conditional executor-owned pending clear;
- typed transaction cleanup state;
- complete transaction journal discovery;
- manual crates.io publishing;
- three CI runner instances only: Linux correctness, macOS smoke, Windows smoke;
- deep crash/protocol verification in local `scripts/release-check.sh verify` only;
- no GitHub release workflow;
- no crates.io token in GitHub Actions;
- no evidence bundle framework;
- no new orchestration dependency.

Do not broaden this pass into general refactoring. Touch only the files needed to satisfy the acceptance criteria below.

---

## 3. Execution rules for a smaller model

Follow these rules exactly:

1. Complete one workstream at a time.
2. Run the focused tests listed for that workstream before starting the next.
3. Do not weaken an assertion to make a test pass.
4. Do not accept multiple outcomes in tests for behavior that has one required outcome.
5. Do not replace an exact assertion with `>=`, `is_some`, `contains`, or a diagnostic `eprintln!` when exact state is available.
6. Do not expose production-only debug APIs. Any new test event or helper must remain behind the existing test-support/test-helper feature boundary.
7. Reuse existing lock types, transaction helpers, observer infrastructure, and failpoint infrastructure. Do not create a parallel transaction engine.
8. Preserve error causes. Do not replace filesystem or stale-action errors with generic success.
9. Keep commits small and aligned with the implementation sequence in Section 13.
10. Keep Phase 11 status `INCOMPLETE` until every closure command in Section 12 passes on one final commit.

---

# Workstream A — Make exact recovery authoritative under lock

## Goal

Eliminate the time-of-check/time-of-use window in `recover_transaction_by_id` and ensure the selected journal is loaded, classified, and executed under the established lock hierarchy.

## Current defect

The current implementation:

1. derives the journal path;
2. reads and parses the journal;
3. classifies the journal;
4. compares the actual class to the expected class;
5. only then acquires the transaction lock inside the selected match arm.

Another process can change or replace the journal between classification and lock acquisition. Recovery can then execute using stale state.

## Required implementation

Refactor recovery into one public orchestration function and locked internal helpers.

Recommended shape:

```rust
pub fn recover_transaction_by_id(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    transaction_id: &str,
    expected: RecoveryClass,
) -> SnipResult<()> {
    validate_transaction_id(transaction_id)?;

    // Acquire the existing outer local-data lock first if this code path
    // does not already hold it, then acquire TransactionLock.
    // Preserve the repository's established lock order.

    let _transaction_lock =
        acquire_transaction_lock(transaction_dir, recovery_operation_name(expected))?;

    let journal = load_exact_journal_under_lock(transaction_dir, transaction_id)?;
    let actual = classify_journal_recovery(transaction_dir, &journal)?;

    if actual != expected {
        return Err(stale_action_error(transaction_id, expected, actual));
    }

    recover_loaded_journal_locked(sync_state_dir, transaction_dir, journal, actual)
}
```

The exact names may differ, but the behavior must match.

### Lock rules

- Use the existing local mutation/data lock if the transaction recovery path requires it.
- Preserve the established order: outer local-data/mutation lock first, transaction lock second.
- Do not acquire the transaction lock again inside `finalize_committed_local_transaction`, rollback, cleanup resume, or terminal-journal removal when the caller already holds it.
- Split functions into orchestration and `_locked` helpers where necessary.
- Do not use recursive lock acquisition.
- Do not classify from a journal loaded before the authoritative lock.

### Exact journal loading

Under the lock:

- derive `txn-<id>.toml` internally;
- reject path separators, traversal components, empty IDs, and malformed identifiers;
- reject a symlinked journal with a hard error;
- read exactly one journal;
- require the parsed journal's internal `id` to equal the requested ID;
- report `NotFound` precisely;
- do not fall back to scanning all journals.

### Execution-time revalidation

The expected action supplied by repair/startup is advisory until checked under lock.

- Reclassify after lock acquisition.
- Return a stale-action error if the class differs.
- Do not silently execute the new action.
- Do not rescan and replace the caller's expected action inside the same repair item.

## Focused tests

Add direct tests that exercise the recovery API, not only the CLI scanner.

Required cases:

1. `Prepared` expected as `Rollback` succeeds.
2. A journal changed to `Committed` before the authoritative under-lock load rejects expected `Rollback` as stale.
3. Requested transaction ID does not match the journal's internal ID and is rejected.
4. A symlinked journal is rejected on Unix.
5. With two journals present, recovering A does not alter B.
6. A concurrent or test-controlled state change before lock acquisition cannot cause stale state execution.

Use existing test-support facilities. A deterministic barrier/failpoint is acceptable if needed. Do not use timing sleeps to manufacture the race.

## Acceptance criteria

- no authoritative classification occurs before the recovery lock is held;
- the selected journal is reloaded under lock;
- expected versus actual recovery class is compared under lock;
- stale actions return an error and mutate nothing;
- the selected recovery path acquires each required lock exactly once;
- unrelated journals remain byte-for-byte unchanged;
- direct tests fail against the Phase 11I implementation and pass after this workstream.

---

# Workstream B — Fail closed on unsafe journals and propagate terminal cleanup failures

## Goal

Ensure mutations cannot proceed while unsafe transaction state exists and ensure terminal journal cleanup cannot claim success when deletion fails.

## Current defects

- `UnsafeFailed` journals are excluded from actionable journals and therefore do not block new mutations when no other actionable journal exists.
- `RemoveTerminalJournal` ignores `fs::remove_file` errors in both exact recovery and the mutation gate.

## Required mutation-gate behavior

After complete scanning and classification:

1. corrupt journals: return an error;
2. any `UnsafeFailed` journal: return an error;
3. more than one recoverable journal: return an error and direct the user to `snp repair`;
4. exactly one recoverable journal: recover that exact journal;
5. only removable terminal journals: remove each safely, propagating failures;
6. no journals: return success.

A failed journal must remain preserved for manual investigation. Never auto-delete or auto-rollback it.

Suggested diagnostic requirements:

- include the transaction ID;
- include the operation;
- include the stored failure message where safe;
- state that mutation is refused;
- direct the user to `snp repair`.

## Terminal journal removal helper

Create one canonical helper, for example:

```rust
fn remove_terminal_journal_locked(
    transaction_dir: &Path,
    transaction_id: &str,
) -> SnipResult<()>;
```

Required semantics:

- validate/derive the journal path internally;
- reject symlinks;
- `NotFound` is idempotent success only when the journal disappeared after the authoritative action was selected;
- all other removal errors propagate;
- fsync the parent directory using the existing durability helper after successful deletion;
- return success only when the journal is absent and directory removal is durably recorded where supported.

Use this helper in both exact repair recovery and the mutation gate. Do not duplicate deletion logic.

## Focused tests

Required cases:

1. one `Failed` journal blocks a new mutation;
2. one `Failed` journal plus one removable terminal journal still blocks mutation;
3. repair reports `Failed` as unsafe and leaves it unchanged;
4. terminal journal removal succeeds and removes the file;
5. terminal journal removal permission/failpoint failure returns nonzero and leaves evidence;
6. `NotFound` after an already-completed idempotent deletion is accepted;
7. no terminal deletion path uses `let _ = fs::remove_file(...)`.

Use a deterministic failpoint or a platform-independent injected filesystem seam for the removal failure test. Do not rely solely on Unix permission behavior because privileged CI environments can bypass it.

## Acceptance criteria

- `UnsafeFailed` always blocks mutation;
- failed journals remain untouched;
- terminal deletion failures are visible to callers;
- parent directory durability is attempted through the existing helper;
- startup and repair share the same terminal removal implementation;
- no ignored terminal-journal deletion result remains.

---

# Workstream C — Make artifact ownership inspection fallible and fail closed

## Goal

Replace boolean artifact ownership detection with a checked inspection that distinguishes absence from unsafe or unreadable artifact state.

## Current defect

`journal_owns_artifacts` returns `bool` and relies on existence checks. It does not produce an error for:

- a symlinked artifact root;
- a symlinked backup or durable staged file;
- a referenced path outside the transaction artifact root;
- unreadable or malformed artifact state.

This allows unsafe state to be reduced to `true` or `false` rather than blocking recovery.

## Required implementation

Change ownership/classification to be fallible.

Recommended shape:

```rust
pub fn inspect_journal_artifacts(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<ArtifactOwnership>;

pub enum ArtifactOwnership {
    None,
    Present,
}

pub fn classify_journal_recovery(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<RecoveryClass>;
```

A `SnipResult<bool>` is acceptable if the implementation remains clear, but a typed enum is preferred.

### Validation rules

For the artifact root, every `backup_path`, and every `durable_staged_path`:

- missing paths are valid absence;
- existing paths must not be symlinks;
- referenced paths must be contained within the exact per-transaction artifact root;
- reject lexical traversal before canonicalization;
- canonicalize existing root and existing child where possible;
- do not canonicalize a missing child into an unrelated path;
- reject a root that is itself a symlink;
- return an error on unsafe containment rather than classifying the journal as removable;
- preserve the journal and artifacts when inspection fails.

### Caller propagation

Update all callers:

- mutation gate;
- repair collection;
- exact recovery revalidation;
- unit tests;
- any compatibility wrapper still using classification.

Repair collection should represent an unsafe artifact-inspection failure as an unsafe/manual item, not as an automatically applicable action.

## Focused tests

Required cases:

1. no artifact paths returns `None`;
2. artifact root exists returns `Present`;
3. backup exists inside root returns `Present`;
4. durable staged path exists inside root returns `Present`;
5. symlinked journal artifact root is rejected on Unix;
6. symlinked backup is rejected on Unix;
7. path outside the artifact root is rejected even when it exists;
8. missing referenced path inside the root is treated as absent, not unsafe;
9. unsafe inspection blocks mutation and produces an unsafe repair item;
10. symlink tests require rejection; following the symlink is not an acceptable alternate outcome.

## Acceptance criteria

- ownership inspection is fallible;
- classification is fallible or otherwise cannot suppress inspection errors;
- unsafe paths block startup mutation and automatic repair;
- repair preserves evidence and reports manual intervention;
- tests contain no comments accepting both symlink rejection and symlink following.

---

# Workstream D — Emit truthful repair reports after application

## Goal

Make human and JSON repair output describe the final result of the requested operation.

## Current defect

`repair_cmd::run` emits JSON/human output before applying repairs. It then changes `applied`, `failed`, `skipped`, and `exit_status` without emitting the completed report again.

`repair --apply --json` can therefore return a JSON body with zero counters even when repairs were applied or failed.

## Required control flow

Refactor `run` to follow this order:

1. collect candidates;
2. if `apply`, select safe items and apply them;
3. compute `applied`, `failed`, `skipped`, and final `RepairExitStatus`;
4. emit exactly one final report in the requested format;
5. return the same final `RepairExitStatus` used by `main` for process exit mapping.

Dry-run must:

- apply nothing;
- set `DryRun` when issues exist;
- set `Clean` when no issues exist, if that is the existing contract;
- emit exactly one report.

Apply mode must:

- return `UnsafeOnly` when issues exist but no safe item can be applied;
- return `PartialFailure` when at least one safe item fails;
- return `Repaired` only when all selected safe items succeed;
- return `Clean` when no issues exist.

## JSON contract

The final JSON object must contain stable fields sufficient for automation:

- `items`;
- `applied`;
- `failed`;
- `skipped`;
- `exit_status`;
- transaction action type;
- transaction ID where applicable;
- `safe`;
- problem/fix text.

`exit_status` should be a stable string such as:

- `clean`;
- `repaired`;
- `partial_failure`;
- `unsafe_only`;
- `dry_run`.

Do not emit preliminary JSON followed by final JSON. One command invocation must emit one valid JSON document to stdout.

Human progress lines may remain on stderr, but the final human summary must agree with returned status.

## Focused tests

Required cases:

1. clean apply returns `Clean`, exit 0, JSON `applied=0`, `failed=0`;
2. one successful repair returns `Repaired`, exit 0, JSON `applied=1`, `failed=0`;
3. one deterministic failed safe repair returns `PartialFailure`, exit 1, JSON `failed=1`;
4. one success plus one deterministic failure returns `PartialFailure`, exit 1, with exact counters;
5. unsafe-only returns the established nonzero unsafe exit and JSON `unsafe_only`;
6. dry-run changes no files and emits `dry_run`;
7. stdout parses as exactly one JSON document;
8. no test accepts either success or failure;
9. no partial-failure test prints a note and passes when the named condition did not occur.

## Acceptance criteria

- reports are emitted after final status computation;
- JSON counters reflect actual work;
- process exit code and JSON `exit_status` agree;
- partial failure is deterministic and strictly asserted;
- no preliminary report is emitted in apply mode.

---

# Workstream E — Replace classification-only repair tests with exact execution tests

## Goal

Make `tests/repair_transactions.rs` prove the behavior named by each test.

## Required corrections

### Exact isolation tests

For rollback, cleanup resume, committed-local finalization, and legacy cleanup:

- create two distinct journals A and B;
- snapshot B's journal bytes and artifacts;
- execute recovery for A through the exact API or a test-support wrapper that invokes it;
- assert A reaches the required terminal result;
- assert B's journal bytes, state, and artifacts are unchanged.

Do not treat a dry-run report containing two IDs as proof of exact execution.

### Stale-action tests

A CLI invocation that rescans after the journal changes does not test stale action rejection.

Use one of these approaches:

1. call `recover_transaction_by_id(..., expected_old_class)` directly after changing the journal; or
2. add a test-support-only apply function that accepts a previously collected `RepairItem` without recollecting.

Required assertion:

- exact stale-action error;
- nonzero result;
- no mutation of the selected journal or live destinations.

### Unknown and malformed ID tests

Call the exact recovery API with:

- an unknown valid ID;
- `../` traversal;
- backslash traversal;
- empty ID;
- mismatched internal journal ID.

Assert exact rejection and unchanged unrelated journals.

### Deterministic partial failure

Use a failpoint or explicit injected failure for one selected safe action. Do not depend on a state change followed by a fresh CLI rescan.

Assert exact counters and exit code.

### Strictness cleanup

Remove permissive patterns such as:

```rust
if condition {
    assert_eq!(...);
} else {
    eprintln!("NOTE: scenario not triggered");
}
```

Replace them with unconditional assertions that the scenario was triggered.

## Test organization

Keep process-level contracts in `tests/repair_transactions.rs`.

Put pure transaction API race/revalidation tests in `src/transaction.rs` unit tests or a focused integration target with test-support exports.

Do not scatter related assertions across unrelated test files.

## Acceptance criteria

- every test named `exact_*` executes a mutation/recovery path;
- every isolation test verifies unrelated state byte-for-byte;
- stale-action tests apply a captured old expectation without rescanning;
- partial-failure tests always trigger partial failure;
- no `NOTE:` fallback allows a missing scenario to pass;
- the tests fail on the Phase 11I baseline for the intended reason.

---

# Workstream F — Make the sync observer proof operation-specific and ordered

## Goal

Prove exactly one matched successful remote sync operation occurs before the matching pending generation is cleared, without counting registration traffic.

## Current defects

- observer recording begins before registration;
- registration emits a successful finish event;
- the test filters starts to sync/push but filters finishes only by `success`;
- the test requires exactly one successful finish across all operations;
- the test only verifies that the finish timestamp is nonzero;
- no actual pending-clear event or timestamp is compared.

## Required observer/test behavior

### Isolate the sync operation

Use either of these approaches:

1. clear/reset the observer after registration and before the mutation; or
2. pair finishes strictly by the sync start sequence and ignore unrelated sequences.

Pairing by sequence is required even if the observer is reset.

Required assertions:

- exactly one sync/push start after the mutation;
- exactly one finish with the same sequence;
- that finish has `success=true`;
- no second sync/push start during the quiet period;
- mandatory authenticated user ID;
- mandatory authenticated device ID;
- mandatory target library ID;
- maximum concurrent sync operations equals exactly 1 unless zero is impossible by construction;
- server state changes from exact R0 to exact R1.

### Prove pending-clear ordering

Add a compile-time-gated test event emitted by the executor immediately after the generation-conditional pending clear succeeds.

Recommended event fields:

```rust
PendingCleared {
    generation: u64,
    cleared_at_unix_ms: i64,
}
```

Use the existing test event sink or observer infrastructure. Do not add a production logging protocol or public runtime API.

The E2E must capture:

- pending generation G created by the mutation;
- successful sync finish for the matched sequence at time T1;
- pending-clear event for generation G at time T2.

Assert:

- finish sequence matches the one sync start;
- pending-clear generation equals G;
- T1 <= T2;
- pending marker is absent after the clear event;
- no duplicate start occurs during the quiet period.

If the existing event system supports monotonic sequence numbers, prefer sequence ordering over wall-clock ordering. Timestamp comparison may be retained as an additional diagnostic.

### Payload/revision assertions

Preserve any existing exact payload hash, payload length, plaintext sentinel, and revision assertions that are already available through observer infrastructure. Do not remove evidence fields merely to simplify the test.

## Focused tests

Required cases:

1. registration plus sync does not cause finish overcounting;
2. sync start and finish are paired by exact sequence;
3. a failed unrelated request cannot satisfy the sync finish assertion;
4. pending clear event references the expected generation;
5. pending clear occurs after successful finish;
6. unreachable server preserves pending and emits no pending-clear event;
7. quiet period produces no duplicate sync start.

## Acceptance criteria

- the headline E2E does not count all successful finishes;
- registration traffic cannot satisfy or break the exact sync finish assertion;
- pending-clear ordering is directly observed;
- one mutation produces one pending generation, one matched successful sync, one clear, and no duplicate;
- all new test-only events are compile-time gated.

---

# Workstream G — Reject untracked files in release clean-tree checks

## Goal

Make `scripts/release-check.sh verify` and `dry-run <crate>` require a genuinely clean checkout, including untracked files.

## Required implementation

Replace or supplement the current tracked/staged checks with a porcelain status check such as:

```bash
status="$(git status --porcelain=v1 --untracked-files=all)"
if [[ -n "$status" ]]; then
    echo "ERROR: Working tree is not clean."
    printf '%s\n' "$status"
    exit 1
fi
```

Requirements:

- tracked modifications fail;
- staged modifications fail;
- untracked files fail;
- ignored build outputs such as `target/` do not fail;
- return nonzero before Cargo package/publish commands execute;
- apply to both `verify` and `dry-run` modes through one shared helper.

## Focused verification

At minimum, manually or through a small shell test fixture prove:

1. clean tree passes the precondition;
2. tracked modification fails;
3. staged modification fails;
4. untracked file fails;
5. ignored `target/` content does not fail.

Do not add a shell test framework dependency solely for this check.

## Acceptance criteria

- untracked files are rejected;
- the check remains simple and readable;
- verify and dry-run share one implementation;
- no automated publishing is added.

---

# Workstream H — Reconcile status and documentation

## Goal

Leave an accurate closure record with no premature completion claim.

## Start-of-implementation status

Before changing production code, update `plans/snip-it-correctness-11-closure-status.md` to record:

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11j-recovery-serialization-proof-and-reporting-closure.md
Corrective baseline: 36a142bbc0ae9340f83e177ef4b9252ce9c58145
Final implementation commit: pending
```

Retain:

- manual crates.io publishing;
- one Linux correctness job;
- macOS and Windows smoke-only jobs.

List the remaining blockers summarized by this plan. Do not state that Phase 11I is fully closed.

## Final status rule

Only after all verification in Section 12 passes on one final commit may the status record:

- exact final implementation commit SHA;
- Phase 11 status `COMPLETE`;
- correctness program status `CLOSED`;
- Linux/macOS/Windows CI result for that commit;
- maintainer assertion that `scripts/release-check.sh verify` passed from a clean checkout;
- no remaining production blocker.

If any required command fails or cannot be executed, keep:

- Phase 11 `INCOMPLETE`;
- correctness program `REOPENED`;
- final implementation `pending verification` or record the candidate commit explicitly as unverified.

Do not create another evidence registry or workflow URL ledger.

## Documentation scope

Update only documents made inaccurate by this pass:

- closure status;
- release clean-tree wording if needed;
- test command wording if a focused target is renamed.

Do not rewrite unrelated README sections.

## Acceptance criteria

- Phase 11J is the authoritative remaining-work plan;
- status remains internally consistent;
- no document calls the program closed while verification is pending;
- final SHA is not an earlier commit followed by corrective commits;
- no new release automation or evidence apparatus appears.

---

# 12. Required verification matrix

Run from a clean checkout at the final candidate implementation commit.

## Fast developer/CI verification

```bash
bash scripts/check.sh
```

## Exact transaction API and repair

```bash
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
```

Run transaction unit tests explicitly if the new under-lock tests are unit tests:

```bash
cargo test --lib transaction --all-features -- --test-threads=1
```

## Restore/security regression

```bash
cargo test --test manifest_contracts --features test-support -- --test-threads=1
cargo test --test restore_security --features test-support -- --test-threads=1
cargo test --test destination_permissions --features test-support -- --test-threads=1
```

## Exact sync closure

```bash
cargo test --test deterministic_e2e --features test-support \
  test_observer_headline_sync_e2e -- --exact --test-threads=1

cargo test --test deterministic_e2e --features test-support \
  test_unreachable_server_preserves_pending -- --exact --test-threads=1
```

Use the actual final test names if renamed, but retain one focused exact command for each contract.

## Production seam

```bash
cargo build --release --no-default-features --target-dir target/production-seam
bash scripts/ci/test-production-seams.sh
```

## Full local release verification

```bash
bash scripts/release-check.sh verify
```

## Per-crate publish dry-run

For changed crates only, in dependency order:

```bash
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

## CI

The final commit must produce only these runner instances:

- Linux correctness;
- macOS platform smoke;
- Windows platform smoke.

All three must pass.

## Clean checkout

```bash
git status --short
```

Expected output: empty.

---

# 13. Implementation sequence for reliable handoff

Use the following commit sequence. One workstream may require more than one commit, but do not combine unrelated corrections.

## Commit 1 — Reopen status under Phase 11J

Files:

- `plans/snip-it-correctness-11-closure-status.md`

Changes:

- point blocking plan to Phase 11J;
- set corrective baseline to `36a142bbc0ae9340f83e177ef4b9252ce9c58145`;
- keep final implementation pending;
- list remaining blockers without completion claims.

## Commit 2 — Serialize exact recovery under lock

Files:

- `src/transaction.rs`;
- focused unit/integration tests.

Changes:

- acquire authoritative locks before journal load/classification;
- split locked helpers to avoid recursive acquisition;
- validate requested ID against internal journal ID;
- implement deterministic stale-action tests.

## Commit 3 — Fail closed on failed journals and terminal deletion errors

Files:

- `src/transaction.rs`;
- transaction/repair tests;
- failpoint definitions only if required.

Changes:

- block mutation on `UnsafeFailed`;
- add canonical terminal journal removal helper;
- propagate deletion and durability errors;
- add deterministic removal failure test.

## Commit 4 — Make artifact inspection fallible

Files:

- `src/transaction.rs`;
- `src/commands/repair_cmd.rs`;
- focused tests.

Changes:

- replace boolean ownership check;
- reject symlinks/out-of-root paths;
- propagate unsafe inspection to mutation gate and repair;
- tighten symlink tests to one required outcome.

## Commit 5 — Correct repair output ordering and strict tests

Files:

- `src/commands/repair_cmd.rs`;
- `src/main.rs` only if exit mapping needs correction;
- `tests/repair_transactions.rs`.

Changes:

- emit final report after application;
- expose stable JSON status/counters;
- replace classification-only and permissive tests;
- make partial failure deterministic.

## Commit 6 — Correct exact sync observer proof

Files:

- executor/pending test-event source;
- existing test observer/event sink support;
- `tests/deterministic_e2e.rs`;
- server test-helper code only where needed.

Changes:

- isolate registration events;
- pair start/finish by exact sequence;
- emit and capture generation-specific pending-clear event;
- prove finish precedes clear;
- preserve quiet-period and unreachable-server contracts.

## Commit 7 — Enforce true clean checkout

Files:

- `scripts/release-check.sh`;
- `RELEASING.md` only if wording changes.

Changes:

- reject untracked files;
- preserve ignored build output behavior;
- retain manual publishing.

## Commit 8 — Verification and final status

Files:

- `plans/snip-it-correctness-11-closure-status.md`.

Changes:

- run the full Section 12 matrix;
- record the exact final implementation commit;
- mark complete/closed only when every gate passes;
- otherwise record remaining failures and keep incomplete/reopened.

---

# 14. Global acceptance criteria

Phase 11J is complete only when all statements below are true:

1. exact transaction recovery loads and classifies the selected journal under lock;
2. stale expected actions are rejected under lock without mutation;
3. unrelated journals remain unchanged during exact recovery;
4. any failed journal blocks new mutation;
5. failed journals are preserved for manual investigation;
6. terminal journal deletion errors propagate;
7. terminal deletion uses one canonical durable helper;
8. artifact ownership inspection is fallible;
9. symlinked or out-of-root artifacts fail closed;
10. repair JSON is emitted after application and contains truthful counters;
11. repair JSON status and process exit code agree;
12. exact recovery tests execute the selected action rather than only inspect dry-run output;
13. stale-action tests do not rescan into a fresh action;
14. partial-failure tests deterministically trigger partial failure;
15. the sync E2E pairs one exact sync start and finish by sequence;
16. registration traffic cannot satisfy or invalidate the sync assertion;
17. the matching successful finish occurs before the matching pending generation is cleared;
18. unreachable server preserves pending and emits no clear event;
19. release verification rejects untracked files;
20. focused CI remains lightweight;
21. deep tests remain local release verification;
22. crates.io publishing remains manual;
23. no new daemon, persistence layer, workflow matrix, or evidence system is added;
24. the final closure status is accurate and references the actual final implementation commit;
25. all commands in Section 12 pass on the same final commit.

Until every criterion passes, Phase 11 remains `INCOMPLETE`, the correctness program remains `REOPENED`, and the repository must not be described as release-ready.

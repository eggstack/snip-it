# Phase 12F — Corrective Closure Pass for Auto-Sync and Recovery

Status: READY FOR IMPLEMENTATION

Baseline: `dc1fe1babd08fcc4c8fc977b9d4fe444fd2145fe`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

Depends on:

- Phase 12C single-helper auto-sync implementation.
- Phase 12D measured footprint work.
- Phase 12E deterministic ordering and recovery-marker implementation.

This is a narrow corrective pass. It closes four concrete mismatches found during review of the implementation that followed the Phase 12 plans:

1. the configured auto-sync timeout is retained in configuration but is not enforced by the direct single-helper execution path;
2. a failed helper attempt may immediately loop into a newer pending generation and bypass the durable retry/backoff decision;
3. recovery markers are durable but startup only logs them, and final marker removal can occur before the final sync cursor is durably persisted;
4. release verification and contributor guidance still reference executor-era or deleted test targets.

The implementation must preserve the Phase 12 architectural result: one detached helper, one execution lock, one durable pending marker, no executor subprocess, no resident daemon, and no new protocol or persistence framework.

---

## 1. Scope and completion boundary

### Required outcomes

Phase 12F must leave the repository with:

- truthful, enforced bounds for automatic sync network execution;
- no immediate retry after an automatic sync failure merely because a newer generation appeared;
- durable backoff remaining authoritative after a failed helper attempt;
- recovery markers that are resumed according to their recorded phase rather than only logged;
- recovery-marker identity checked before a stored remote library ID is trusted;
- non-`NotFound` marker metadata errors failing closed;
- recovery markers removed only after merged content, linkage, and the final `last_sync` cursor are durably persisted;
- stale `Linked` markers able to complete without creating another remote library;
- current release verification containing no command for a deleted test target;
- current contributor documentation containing no normative references to removed executor-era tests;
- a small focused regression set that proves these exact behaviors.

### Explicit non-goals

Do not introduce:

- an executor subprocess or hidden replacement executor command;
- process supervision, process groups, kill/reap logic, or a watchdog;
- a resident client daemon;
- a queue, journal database, lease, heartbeat, or new lock file;
- a new async runtime per library or request;
- a public sync-backend abstraction;
- a generalized cancellation framework;
- a new dependency;
- a new gRPC field, RPC, protocol version, or server schema migration;
- idempotency keys requiring server changes;
- CRDTs, logical clocks, vector clocks, or conflict-history storage;
- a generalized recovery engine beyond the existing sync-recovery marker;
- a new CI job, test matrix, release workflow, coverage target, or artifact upload;
- a benchmark suite or performance gate;
- broad refactoring of `run_sync`, `LibraryManager`, auto-sync policy, or command dispatch unrelated to these defects.

### Stop condition

When the acceptance criteria in this plan pass, mark Phase 12F COMPLETE and close Phase 12. Do not open a Phase 12G hardening pass unless a reproducible defect remains.

---

## 2. Current defects and intended corrections

### 2.1 Configured auto-sync timeout is not applied

Current state:

- `SyncSettings::auto_sync_timeout_seconds` resolves into `AutoSyncPolicy::sync_timeout`.
- The detached helper creates a Tokio runtime and calls `sync_commands::run_sync` directly.
- The helper does not pass or inspect `policy.sync_timeout` during that operation.
- The sync client has connect and request timeouts, but they are separately resolved from environment variables and do not represent the configured automatic-sync budget.
- `worker_lifetime` is checked between helper cycles, not while a multi-library sync is executing.

Required correction:

- Retain direct canonical sync execution.
- Add a small internal execution-limit path that allows the auto-sync helper to provide a deadline/request budget to the existing sync client and orchestration path.
- Manual sync and cron must retain their current default behavior unless they explicitly opt into the same internal limit.
- Do not claim that arbitrary local filesystem operations can be force-cancelled.

Preferred bounded design:

```rust
#[derive(Clone, Copy)]
pub(crate) struct SyncRunLimits {
    pub deadline: std::time::Instant,
    pub request_timeout: std::time::Duration,
}
```

The exact type name is not important. Keep it private or `pub(crate)` and place it in an existing sync module. Avoid a trait or generalized options hierarchy.

Expected call shape:

```text
auto_sync::worker
  -> compute deadline from policy.sync_timeout
  -> run_sync_with_limits(settings, ..., limits, runtime)
      -> create SyncClient with request timeout <= remaining deadline
      -> check remaining deadline before each library/recovery network operation
      -> stop retry loops when the deadline is exhausted
      -> return typed timeout failure
```

A minimal alternative is acceptable if it is equally truthful:

- allow `SyncClient::create_with_request_timeout` or equivalent;
- check an overall `Instant` deadline before each library and each recovery retry;
- cap every request timeout and retry sleep to the remaining budget.

Do not mutate process-wide environment variables to communicate the timeout. The helper is a separate process today, but a hidden environment side channel would be harder to reason about and would make direct unit testing less reliable.

#### Required semantics

- `auto_sync_timeout_seconds` bounds the automatic sync attempt's network/retry window.
- If the deadline expires before another library or retry begins, return a timeout-classified `SnipError`.
- A request already in flight must use a timeout no greater than the remaining helper budget.
- Retry sleep must not extend beyond the remaining budget.
- Successful work already durably written before a later timeout remains committed; pending intent remains so the next attempt can reconcile the complete state.
- The helper records `FailureClass::TransientTimeout` for deadline exhaustion.
- The pending marker is not cleared on timeout.

#### Files to inspect

```text
src/auto_sync/worker.rs
src/auto_sync/policy.rs
src/config.rs
src/sync.rs
src/sync_commands.rs
src/error.rs
architecture/auto_sync.md
architecture/sync.md
architecture/config.md
README.md
```

Do not change the public CLI option or existing configuration field name.

#### Acceptance criteria

- [ ] `policy.sync_timeout` is consumed by the production auto-sync path.
- [ ] Automatic sync request/retry work cannot knowingly begin after its configured deadline.
- [ ] Individual automatic-sync requests use a timeout no greater than the remaining deadline.
- [ ] Deadline exhaustion maps to `FailureClass::TransientTimeout`.
- [ ] Pending intent remains after timeout.
- [ ] Manual sync behavior is unchanged unless the existing command already supplies a limit.
- [ ] No executor subprocess, watchdog, or cancellation thread is introduced.
- [ ] No environment-variable mutation is used as the internal timeout API.
- [ ] Documentation describes the actual bounded behavior and does not promise force-cancellation of local I/O.

---

## 3. Workstream A — Make failure terminate the helper cycle

### Defect

The current helper runs `execute_sync`, then rereads pending state. If the pending generation is newer than the observed generation, it continues the loop without first checking whether `execute_sync` succeeded.

That permits this sequence:

```text
attempt generation 10
  -> network failure
  -> status records future backoff
mutation records generation 11 while attempt is active
helper sees 11 > 10
  -> immediately loops
  -> attempts sync again inside the same helper
```

This bypasses the scheduler's durable backoff decision and can cause repeated requests during an outage and active mutation burst.

### Required correction

Use the direct sync outcome as the first branch:

```rust
match execute_sync(state_dir, policy, observed.generation) {
    WorkerOutcome::Failed => return WorkerOutcome::Failed,
    WorkerOutcome::NothingToDo => return WorkerOutcome::NothingToDo,
    WorkerOutcome::Success => {
        // Only successful acknowledgement may inspect a newer generation
        // and continue into a bounded follow-up cycle.
    }
}
```

Equivalent structure is acceptable. The behavioral rule is mandatory:

- failure exits the helper;
- pending state remains;
- status/backoff remains authoritative;
- a later mutation may attempt scheduling, but `schedule_sync` must return `DeferredUntil` or `RequiresAttention` according to the status file;
- success may continue to a newer generation while the helper lifetime permits.

Do not add a second backoff sleep loop inside the helper. The existing scheduler is the retry authority.

### Failure classes

Preserve existing policy:

- transient network/timeout failure records backoff and exits;
- authentication/configuration/credential failures record attention state and exit;
- local persistence/internal failure preserves evidence and exits;
- only successful canonical sync can clear the exact observed generation.

### Files to inspect

```text
src/auto_sync/worker.rs
src/auto_sync/schedule.rs
src/auto_sync/status.rs
src/auto_sync/policy.rs
tests/auto_sync_closure.rs
```

### Focused tests

Add a small test seam rather than a mock framework. Preferred choices, in order:

1. a private `run_locked_with_sync` function accepting a closure/function pointer under normal compilation;
2. a `#[cfg(test)]` wrapper around the direct sync call;
3. an internal generic helper used only inside `worker.rs` tests.

Do not expose a public `SyncOperation` trait.

Required focused cases:

#### Case A — failed generation with newer pending work

```text
initial pending generation = 1
injected sync operation records generation 2, then returns transient failure
expected:
  sync operation called exactly once
  helper returns Failed
  pending generation 2 remains
  status has future next_attempt_at
  schedule_sync(Mutation) returns DeferredUntil
```

#### Case B — attention failure with newer pending work

```text
initial generation = 1
injected operation records generation 2, then returns Authentication failure
expected:
  one attempt
  helper returns Failed
  generation 2 remains
  scheduling returns RequiresAttention(Authentication)
```

#### Case C — successful generation with newer pending work

```text
attempt generation 1 succeeds while generation 2 appears
expected:
  exact-generation clear preserves generation 2
  bounded follow-up remains permitted
```

Existing lower-generation/corruption tests must remain valid.

### Acceptance criteria

- [ ] Failed direct sync never immediately loops to a newer generation.
- [ ] Successful direct sync may process newer work while lifetime remains.
- [ ] The scheduler, not the helper loop, is authoritative for retry delay and attention state.
- [ ] Transient failure plus a newer generation produces one network attempt in the helper test.
- [ ] Attention failure plus a newer generation produces one network attempt in the helper test.
- [ ] Exact-generation clear behavior is unchanged.
- [ ] No new persistence field or retry loop is introduced.

---

## 4. Workstream B — Complete recovery-marker state transitions

### Current state

The existing recovery marker has useful bounded state:

```text
Creating
RemoteCreated
Linked
```

The recovery path writes the marker atomically, preserves corrupt marker content, records the remote server ID, and relinks the local library. However:

- startup marker scanning only logs markers and does not resume them;
- any `symlink_metadata` error is currently treated like absence instead of distinguishing `NotFound` from permission/I/O failure;
- a stored server library ID may be trusted without validating that the marker identifies the current local library entry;
- failure to persist the final `last_sync` cursor is logged, but the marker may still be removed and success reported;
- a `Linked` marker left by a crash is not actively completed on the next sync;
- the no-parent fallback can create a remote library without durable recovery state, despite the normal library path always being expected to have a parent.

### Required design

Keep the existing marker and phases. Add one narrow internal resume function used by both startup scanning and the `LibraryNotFound` error path.

Suggested shape:

```rust
fn resume_library_recovery(
    marker_path: &Path,
    marker: SyncRecoveryMarker,
    local_library_name: &str,
    local_path: &Path,
    snippets: &Snippets,
    client: &mut SyncClient,
    manager: &mut LibraryManager,
    runtime: &Runtime,
) -> SnipResult<RecoveryOutcome>
```

The exact signature may be smaller. Do not create a public recovery service or state-machine framework.

### Marker lookup semantics

Use explicit filesystem handling:

```rust
match fs::symlink_metadata(&marker_path) {
    Ok(_) => read and validate marker,
    Err(error) if error.kind() == ErrorKind::NotFound => create initial marker,
    Err(error) => fail closed and preserve all state,
}
```

Do not convert permission, malformed path, or transient I/O failures into "marker missing."

### Marker identity validation

Before using `server_library_id`, validate:

- `schema` is supported;
- marker `local_library_name` matches the library being recovered after the same normalization rules used when the marker was created;
- marker `local_library_id` matches the current local library metadata when both values are nonempty;
- the marker path stem corresponds to the expected local library filename;
- the current library still exists in `LibraryManager` and the local TOML file exists;
- if the local config is already linked, the configured server ID must either match the marker or produce an explicit ambiguity/error.

Do not silently rewrite a mismatched marker. Preserve it for diagnosis and return a concise failure.

### Phase behavior

#### `Creating`

1. List remote libraries.
2. Match by the existing normalized-name rule.
3. Exactly one match: reuse it.
4. Zero matches: create one remote library.
5. More than one match: fail as ambiguous and preserve the marker.
6. Persist `server_library_id` and phase `RemoteCreated` before local relinking.

Do not add a protocol idempotency field.

#### `RemoteCreated`

1. Require a nonempty stored `server_library_id`.
2. Validate local identity.
3. Persist local `library_id`, `server_id`, and `last_sync = 0` in one `LibraryManager` save using the existing relink helper.
4. Persist phase `Linked` only after the local config save succeeds.

#### `Linked`

1. Verify local linkage equals the marker's remote ID.
2. Retry canonical encrypted sync against that ID.
3. Merge and durably save returned snippets.
4. Persist the final server timestamp through `LibraryManager::update_last_sync`.
5. If cursor persistence fails, return failure and preserve the marker.
6. Remove the marker only after merged data and cursor persistence both succeed.

A marker removal error should be visible but must not roll back an otherwise durable completed sync. On the next run, the `Linked` phase should verify that linkage/cursor are already complete and safely remove the stale marker without creating or relinking a remote library again.

### Startup recovery

Replace log-only scanning with bounded completion:

- scan only `*.sync_recovery` in the existing libraries directory;
- process markers sequentially;
- preserve corrupt or mismatched markers;
- do not abort all ordinary sync work because one unrelated library marker is corrupt, but record that library as failed so `run_sync` returns partial failure;
- ensure a resumed library is not recreated by the later normal library loop;
- avoid a second immediate network sync for a library already completed during marker recovery in the same invocation.

A small `HashSet<String>` of completed/recovered library filenames is acceptable if needed. Do not add persistent bookkeeping.

### Remove unsafe fallback

If `lib_path.parent()` is unexpectedly absent, fail the recovery operation. Do not create a remote library without a marker and report "Re-linked" without durable local linkage.

### Files to inspect

```text
src/sync_commands.rs
src/library.rs
src/sync.rs
src/error.rs
architecture/sync.md
architecture/persistence.md
README.md
AGENTS.md
```

Server and protobuf files should not change.

### Focused tests

Prefer unit tests beside the private recovery helpers. Reuse the existing fake client/server test support where a real RPC boundary is required; do not add another helper binary.

Required cases:

1. **Non-NotFound metadata error fails closed**
   - marker lookup error is returned;
   - no remote create call occurs.

2. **Marker identity mismatch**
   - mismatched local ID/name is rejected;
   - marker remains unchanged;
   - stored remote ID is not trusted.

3. **`RemoteCreated` resumes relink without creating**
   - existing server ID is reused;
   - local linkage and `last_sync = 0` persist together;
   - create RPC count remains zero.

4. **`Linked` resumes final sync**
   - no create or relink occurs when linkage already matches;
   - final sync succeeds;
   - cursor persists;
   - marker is removed.

5. **Cursor persistence failure preserves marker**
   - merged data may already be durable;
   - `update_last_sync` failure returns failure;
   - marker remains in `Linked` phase.

6. **Stale completed `Linked` marker**
   - durable linkage/cursor are recognized;
   - marker is removed or final sync is safely repeated according to current cursor semantics;
   - no duplicate remote library is created.

7. **Ambiguous normalized remote names**
   - existing behavior remains fail-closed;
   - marker remains.

Do not add crash-failpoint infrastructure for this pass.

### Acceptance criteria

- [ ] Startup scanning performs bounded marker completion rather than logging only.
- [ ] Only `ErrorKind::NotFound` means the marker is absent.
- [ ] Marker schema and local identity are validated before a stored server ID is used.
- [ ] `Creating`, `RemoteCreated`, and `Linked` each have explicit resumable behavior.
- [ ] `Linked` recovery never calls create-library.
- [ ] Final `last_sync` persistence failure preserves the marker and reports failure.
- [ ] Marker removal occurs only after durable merged data and cursor state.
- [ ] A stale linked marker cannot create a duplicate remote library.
- [ ] Corrupt, mismatched, or ambiguous markers remain available for diagnosis.
- [ ] No server/protobuf/schema change is introduced.
- [ ] No new persistence file is introduced.

---

## 5. Workstream C — Repair verification and contributor contracts

### Current stale references

At minimum, inspect and correct:

```text
scripts/release-check.sh
AGENTS.md
architecture/test-infrastructure.md
docs/CANONICAL_OPERATIONS.md
docs/FUZZING_AND_PROPERTY_TESTS.md
.skills/sync-module.md
plans/snip-it-phase-12c-single-helper-auto-sync-simplification.md
plans/snip-it-phase-12e-sync-ordering-recovery-semantics.md
```

The current manual release script references `tests/deterministic_e2e.rs`, which was deleted during the single-helper simplification. `AGENTS.md` also uses that deleted test as normative contributor guidance.

### Required cleanup

- Remove commands for deleted test targets.
- Replace normative references with the current focused test location only when a real replacement exists.
- Do not add a redundant release-mode invocation merely to keep the same number of commands.
- The release script already runs the full test suite in its earlier phase; retain only release-specific crash/packaging checks in the release-only phase.
- Search current non-historical docs and scripts for:

```text
deterministic_e2e
auto-sync-execute
spawn_executor
executor subprocess
worker/executor
```

- Correct current operational documentation.
- Historical completed plans may retain old architecture descriptions where clearly historical. Do not mass-rewrite every old plan.
- Phase 12C and 12E closure records may receive a concise note that Phase 12F owns the identified post-implementation corrections; do not mark those phases incomplete again.

### Test placement

Keep the check surface small:

- add the helper failure/backoff cases to `tests/auto_sync_closure.rs` or `worker.rs` unit tests;
- add recovery state tests to `sync_commands.rs` unit tests unless an existing integration test is necessary;
- keep `scripts/check.sh` structure unchanged unless a new standalone test file is genuinely required;
- do not restore the deleted broad deterministic E2E suite;
- do not add sleep-heavy or probabilistic concurrency tests.

### Release-script validation

Implementation closure must at least prove:

```text
bash -n scripts/release-check.sh
```

and verify that every explicit `cargo test --test <target>` in the script corresponds to a current `tests/<target>.rs` target.

Do not require a new script to perform this validation. A short shell loop run manually and recorded in the plan closure is sufficient.

The full `bash scripts/release-check.sh verify` remains a manual pre-release command. It is not a mandatory Phase 12F implementation gate unless the environment is already configured and the executor chooses to run it once.

### Acceptance criteria

- [ ] `scripts/release-check.sh` contains no deleted test target.
- [ ] Every explicit test target named by the release script exists.
- [ ] `AGENTS.md` contains no normative reference to `tests/deterministic_e2e.rs`.
- [ ] Current architecture documentation contains no executor-era production claim.
- [ ] No deleted broad test suite is restored.
- [ ] `scripts/check.sh` remains focused and does not gain a new matrix or deep-suite phase.
- [ ] Focused tests cover the new corrective behavior without a mocking dependency.

---

## 6. Workstream D — Optional release-profile sanity check

Phase 12D selected `opt-level = "z"` from controlled size measurements. This corrective pass must not reopen general footprint optimization.

A small non-blocking sanity check is permitted after correctness work:

```text
build current opt-level z
measure 5 warm invocations of `snp version`
measure 5 warm invocations of one representative read-only command against an existing moderate library
repeat with opt-level s or 3 in a temporary local edit
record median wall-clock observations and restore the tree
```

Rules:

- no benchmark crate;
- no Criterion dependency;
- no generated benchmark corpus committed to the repository;
- no CI performance threshold;
- no profile matrix;
- do not change `opt-level = "z"` unless the local result shows a clear user-visible regression and the alternative remains materially smaller than the original profile;
- failure to obtain stable timing data does not block Phase 12F closure.

Record the result in the Phase 12D or Phase 12F closure notes only if the experiment is run.

### Acceptance criteria

- [ ] No benchmark infrastructure is added.
- [ ] The release profile is unchanged unless a measured, material regression justifies a bounded correction.
- [ ] This optional check does not delay the correctness work or expand scope.

---

## 7. Recommended implementation sequence

Use one primary implementation commit and one optional closure/documentation commit.

### Step 1 — Add direct-sync limit plumbing

- add the smallest internal limits/deadline representation;
- route it from the helper to sync orchestration and client creation;
- cap request timeout and retry sleep by remaining time;
- add timeout classification tests;
- update timeout documentation.

Recommended commit portion:

```text
phase-12f: enforce bounded direct auto-sync attempts
```

### Step 2 — Correct helper failure transition

- branch on direct sync outcome before newer-generation continuation;
- add deterministic injected-operation tests;
- prove scheduler backoff/attention behavior after the helper exits.

This should normally remain in the same implementation commit as Step 1 because both touch the direct helper contract.

### Step 3 — Complete marker resumption

- centralize marker lookup/validation;
- implement phase-aware resumption;
- make cursor persistence a prerequisite for marker removal;
- make startup scanning perform recovery;
- add focused phase tests.

Recommended commit portion:

```text
phase-12f: complete durable sync recovery transitions
```

A separate commit is acceptable if the auto-sync and recovery diffs are independently reviewable.

### Step 4 — Correct scripts and current docs

- remove stale release target;
- fix contributor and architecture references;
- cross-link closure records to Phase 12F;
- record exact verification commands and implementation SHAs.

Recommended commit:

```text
phase-12f: align closure verification and documentation
```

Do not split mechanical documentation edits into many commits.

---

## 8. Lightweight verification plan

Run formatting and focused tests during implementation:

```text
cargo fmt --all -- --check
cargo test -p snip-it auto_sync::worker --all-features -- --test-threads=1
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test -p snip-it sync_commands --all-features -- --test-threads=1
```

Run an existing sync integration target only for cases that cannot be proven with unit seams:

```text
cargo test --test sync_integration --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
```

At closure:

```text
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash scripts/check.sh
bash -n scripts/release-check.sh
```

Manually verify release-script targets:

```text
for target in $(sed -n 's/.*--test \([^ ]*\).*/\1/p' scripts/release-check.sh | sort -u); do
  test -f "tests/${target}.rs" || { echo "missing test target: ${target}"; exit 1; }
done
```

This command is closure evidence, not a new committed script.

Do not add mandatory soak, fuzz, model-checking, cross-compilation, or timing gates.

Platform CI should continue providing ordinary macOS and Windows compile/library smoke coverage. No workflow change is expected.

---

## 9. Handoff notes for smaller-model execution

Execute the plan in the stated order and avoid opportunistic cleanup.

### Do

- follow the concrete branches and phase transitions specified above;
- preserve direct `run_sync` use;
- use existing `SnipError`, `FailureClass`, status, atomic-write, and `LibraryManager` facilities;
- add assertions for exact RPC/sync-call counts where a seam exists;
- preserve markers on every ambiguous or failed recovery transition;
- report implementation SHA and exact commands in this plan.

### Do not

- reintroduce `src/auto_sync/executor.rs`;
- add a thread solely to kill a timed-out sync;
- convert the entire CLI or `run_sync` to async;
- redesign all sync functions around a general context object;
- add a mock library or dependency;
- change server protocol to solve local recovery;
- add more recovery phases unless one existing transition cannot be represented;
- add a new release or CI script;
- rewrite unrelated historical plans;
- optimize dependencies or binary size during the correctness pass.

### Review checkpoints

Before committing, answer these questions from the diff:

1. Does auto-sync actually consume `policy.sync_timeout`?
2. Can any failed helper path reach `continue`?
3. Can any non-`NotFound` marker lookup error lead to remote creation?
4. Can a marker be removed when `update_last_sync` failed?
5. Can `Linked` recovery call `create_library`?
6. Can a stored remote ID be trusted after local marker identity changed?
7. Does any current script name a deleted test target?
8. Did the change add a dependency, process, lock, persistence file, CI job, or protocol field?

Any answer inconsistent with this plan blocks closure.

---

## 10. Closure checklist

- [ ] Direct automatic sync consumes the configured timeout.
- [ ] Request/retry work is bounded by the remaining automatic-sync deadline.
- [ ] Timeout preserves pending state and records `TransientTimeout`.
- [ ] Failed helper attempts exit without processing a newer generation.
- [ ] Scheduler backoff and attention state remain authoritative.
- [ ] Successful helpers may still process a newer generation within lifetime.
- [ ] Recovery marker lookup distinguishes absence from I/O failure.
- [ ] Marker schema and local identity are validated.
- [ ] `Creating`, `RemoteCreated`, and `Linked` resume correctly.
- [ ] `Linked` recovery performs no remote create.
- [ ] Final cursor persistence is required before marker removal.
- [ ] Stale linked markers complete without duplicate remote libraries.
- [ ] Corrupt, mismatched, and ambiguous markers remain on disk.
- [ ] Release verification names only existing test targets.
- [ ] Current contributor docs contain no deleted executor-era test reference.
- [ ] Focused regression tests pass.
- [ ] `cargo check --workspace --all-targets --all-features` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- [ ] `bash scripts/check.sh` passes.
- [ ] `bash -n scripts/release-check.sh` passes.
- [ ] No new dependency, process, daemon, lock, persistence file, protocol field, CI job, or benchmark framework was added.
- [ ] Plan records implementation SHA and exact verification commands.

When all items are checked, mark this plan COMPLETE, update the Phase 12 roadmap to COMPLETE, and close this line of work.
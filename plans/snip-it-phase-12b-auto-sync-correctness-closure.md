# Phase 12B — Auto-Sync State and Child-Lifecycle Correctness

Status: READY FOR IMPLEMENTATION

Baseline: `8ca5472a9a6481689ab155d79e9a43765b658172`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

Prerequisite: Phase 12A complete.

This phase corrects the current two-process auto-sync implementation before Phase 12C removes the second process. It exists to separate correctness repairs from architectural deletion, making review and rollback straightforward.

The pass must remain narrow. Do not redesign policy, add new persistence artifacts, or implement the one-helper target early.

---

## 1. Required outcomes

Implement all of the following:

1. Unreadable or corrupt pending state is never reported as `NoPending`.
2. Unexpected execution-lock inspection/acquisition failures never fall through to `SpawnNow`.
3. Worker spawn failure is represented truthfully to callers.
4. Startup recovery determines active execution from the kernel lock, not persistent PID metadata.
5. Pending generation rollback is classified as corrupt/inconsistent state, not accepted as new work.
6. Executor wait errors terminate and reap the child before the worker releases the execution lock.
7. Focused tests prove each behavior without expanding the test architecture.

This phase may remove dead metadata-only checks that contradict the kernel-lock design. It must not remove the executor process; that is Phase 12C.

---

## 2. Complexity budget

Expected production files:

```text
src/auto_sync/schedule.rs
src/auto_sync/worker.rs
src/auto_sync/spawn.rs
src/auto_sync/execution_lock.rs
src/auto_sync/pending.rs
src/auto_sync/mod.rs
src/main.rs
```

Tests may touch existing auto-sync integration files. Prefer extending the nearest focused test rather than creating a new broad suite.

Expected production change should normally remain below 500 lines. Most work is error classification and branch correction, not new machinery.

---

## 3. Explicit non-goals

Do not:

- remove `auto-sync-execute` yet;
- make the worker call sync directly yet;
- change debounce, max-delay, timeout, or backoff defaults;
- add a queue, database, event log, heartbeat, lease, or watchdog;
- add another lock file;
- replace CRC32 integrity with cryptographic authentication;
- redesign transaction recovery;
- add a general error-reporting framework;
- alter manual `snp sync` behavior except where it shares corrected lock semantics;
- add randomized stress loops;
- expand CI or release checks;
- add production failpoints.

---

# Workstream A — Make scheduling errors explicit and fail closed

## Current defect

`schedule_sync()` currently maps any pending read error other than `NotFound` to `NoPending`. It also treats execution-lock errors other than `AlreadyHeld` as if the lock were available and may return `SpawnNow`.

These branches hide local state failures and can either suppress work or schedule concurrent work.

## Required API shape

Preferred bounded design:

```rust
pub enum ScheduleDecision {
    SpawnNow,
    AlreadyActive,
    DeferredUntil(u64),
    Disabled,
    RequiresAttention(FailureClass),
    NoPending,
    NotConfigured,
}

pub enum ScheduleError {
    Pending(PendingError),
    ExecutionLock(ExecutionLockError),
    Spawn(SpawnError),
}

pub fn schedule_sync(...) -> Result<ScheduleDecision, ScheduleError>
```

A smaller equivalent is acceptable if it preserves the distinction. Do not encode I/O failures as a new `FailureClass` merely to avoid `Result`; scheduling failure is local control-plane failure, not a remote sync failure.

Required mappings:

- `PendingError::NotFound` -> `Ok(NoPending)`;
- pending deserialize, integrity, I/O, lock, or corruption error -> `Err(ScheduleError::Pending(...))`;
- execution lock `AlreadyHeld` -> `Ok(AlreadyActive)`;
- execution lock I/O/unsupported/other error -> `Err(ScheduleError::ExecutionLock(...))`;
- policy/backoff decisions -> existing successful decision variants.

## Spawn outcome

`schedule_and_spawn()` and `schedule_existing_pending()` must not return `SpawnNow` after `spawn_worker()` fails.

Preferred behavior:

```rust
match schedule_sync(...)? {
    ScheduleDecision::SpawnNow => {
        spawn_worker(state_dir).map_err(ScheduleError::Spawn)?;
        Ok(ScheduleDecision::SpawnNow)
    }
    other => Ok(other),
}
```

If callers need to distinguish “decision allowed spawn” from “child actually created,” introduce a small `ScheduleOutcome::Spawned { pid }` variant. Do not add a separate durable spawn record.

## Caller behavior

For post-mutation notification:

- local mutation remains committed;
- pending marker remains intact;
- scheduling error is logged and handled according to existing auto-sync failure mode;
- no claim is made that a worker was started.

For startup recovery:

- error is visible in status/logging;
- pending marker remains intact;
- ordinary read-only commands remain governed by existing recovery classification.

## Acceptance criteria

- [ ] Corrupt pending TOML cannot return `NoPending`.
- [ ] Integrity mismatch cannot return `NoPending`.
- [ ] Lock I/O failure cannot return `SpawnNow`.
- [ ] Spawn failure cannot be reported as successful scheduling.
- [ ] Pending data is preserved on every local scheduling error.
- [ ] No new persistence file is added.

---

# Workstream B — Use the kernel lock as startup authority

## Current defect

`startup_recover()` reads persistent execution-lock metadata, checks whether the recorded PID appears alive, and suppresses scheduling. The execution-lock module itself documents that metadata as diagnostic-only.

A stale file whose PID has been reused can therefore suppress recovery even when no kernel lock is held.

## Required implementation

Remove metadata-liveness authorization from startup recovery.

Use one of these bounded approaches:

### Preferred: delegate entirely to the scheduler

```rust
let decision = schedule::schedule_and_spawn(
    state_dir,
    &policy,
    Caller::StartupRecovery,
)?;
```

The scheduler already attempts the kernel lock nonblocking.

### Acceptable: explicit availability probe

```rust
match execution_lock::try_acquire(state_dir) {
    Ok(guard) => drop(guard),
    Err(AlreadyHeld { .. }) => return Ok(Some(current)),
    Err(other) => return Err(...),
}
```

Do not call `inspect()` plus `process_alive()` to decide ownership.

Persistent identity remains useful for error messages only.

## Tests

Add a focused regression using a persistent lock file with metadata naming the current test PID but no held kernel lock. Startup recovery must still proceed to a scheduling decision rather than treating the metadata as active ownership.

Suppress actual detached spawning using the existing `test-support` seam where appropriate. Do not create a fake process or PID-reuse stress test.

Also retain one test where the kernel lock is actually held and recovery reports/returns `AlreadyActive` without spawning.

## Acceptance criteria

- [ ] Persistent metadata alone never suppresses startup recovery.
- [ ] A genuinely held kernel lock suppresses duplicate work.
- [ ] Malformed metadata does not weaken kernel exclusion.
- [ ] Metadata remains diagnostic-only in docs and code comments.

---

# Workstream C — Reject generation rollback

## Current defect

During debounce, if the reloaded pending generation is lower than the currently observed generation, the worker assigns the lower state to `current` and continues. Elsewhere, lower generation is treated as corrupt or inconsistent.

Pending generation is monotonic. Rollback must not be consumed as normal work.

## Required behavior

In every worker path that reloads pending state:

- `latest.generation > current.generation`: update to the newer generation and recompute debounce as today;
- `latest.generation == current.generation`: continue normally;
- `latest.generation < current.generation`: stop the cycle, preserve the pending file, record an internal/local-state failure, and return `WorkerOutcome::Failed`.

Use one helper such as:

```rust
enum GenerationRelation {
    Same,
    Newer,
    RolledBack,
}
```

only if it reduces duplicated branch logic. A general generation framework is unnecessary.

The failure message should include observed and current generation numbers but no snippet content.

## Tests

Use a deterministic mock clock or direct state rewrite in the existing worker unit tests:

1. record generation `G`;
2. begin debounce with `G`;
3. replace the marker with valid generation `G-1` using existing test helpers;
4. assert `DebounceResult::Failed` or the chosen explicit rollback result;
5. assert the lower marker remains for repair/diagnosis;
6. assert no sync executor is spawned.

Do not add probabilistic concurrent tests.

## Acceptance criteria

- [ ] Lower generation never becomes the next sync target.
- [ ] The on-disk marker is preserved.
- [ ] Status/logging identifies generation rollback as internal corruption/inconsistency.
- [ ] A newer generation remains supported.
- [ ] Equal generation behavior is unchanged.

---

# Workstream D — Terminate and reap executor on wait failure

## Current defect

The timeout path terminates and reaps the executor. The `try_wait()` error path records failure and returns without ensuring the child exits. Dropping `Child` does not terminate it, so the worker can release the execution lock while the executor remains active.

## Required implementation

Create one bounded cleanup helper used by timeout and wait-error branches:

```rust
fn terminate_and_reap(child: &mut Child, grace: Duration) {
    terminate_child(child);
    wait for grace using bounded polling;
    if still running, force_kill_child(child);
    let _ = child.wait();
}
```

Requirements:

- cleanup is best-effort but always attempted;
- the worker does not return from a wait error until cleanup/reap has been attempted;
- no unbounded wait;
- Unix signal and Windows kill behavior remain as currently implemented;
- status records the original wait error, optionally with cleanup failure detail;
- the execution lock guard remains alive through cleanup.

Do not introduce an async process supervisor or process-group management in this phase.

## Tests

A true `try_wait()` OS error is difficult to induce portably. Extract and unit-test the cleanup decision around a small child-process test only if deterministic on the current platform.

Minimum acceptable proof:

- code path visibly calls the same cleanup helper on timeout and wait error;
- existing timeout test still passes;
- one Unix-focused test starts a long-lived child, invokes cleanup, and confirms it exits within the bounded period.

Do not add sleeps longer than the existing grace interval; use polling and short test-specific grace values.

## Acceptance criteria

- [ ] Every post-spawn error path attempts to reap the executor.
- [ ] The execution lock is held until cleanup finishes.
- [ ] No child can intentionally continue after the worker reports a wait failure.
- [ ] Timeout behavior remains unchanged from the user perspective.

---

# Workstream E — Make corrupt state visible in status

## Goal

The corrected scheduling/worker errors must be visible through existing status and logging paths without adding a new status subsystem.

## Required implementation

Use the existing `auto-sync-status.toml` and `FailureClass::Internal` where a durable failure record is appropriate.

For failures before an observed generation can be trusted, do not invent generation zero as acknowledged work. Record a concise local-state message or expose the error through `snp status` diagnostics using the existing status snapshot machinery.

Avoid storing raw TOML, command text, descriptions, API keys, or file contents in status.

## Acceptance criteria

- [ ] `snp status` or existing logs distinguish corrupt pending state from no pending state.
- [ ] Lock acquisition failure is not reported as active sync unless the lock is genuinely held.
- [ ] Messages are concise and secret-free.
- [ ] No new status file or diagnostic registry is created.

---

## 4. Recommended implementation order

1. Change `schedule_sync` to return explicit errors.
2. Update scheduling wrappers and all call sites.
3. Replace startup metadata liveness with kernel-lock authority.
4. Add generation rollback failure behavior.
5. Consolidate executor termination/reaping.
6. Ensure existing status output conveys corrected failures.
7. Run focused unit/integration tests.
8. Update architecture comments only where current behavior is described incorrectly.
9. Mark this plan COMPLETE before beginning 12C.

---

## 5. Focused verification

Run focused tests by exact target names where available. At minimum:

```text
cargo fmt --all -- --check
cargo test -p snip-it auto_sync::schedule --all-features -- --test-threads=1
cargo test -p snip-it auto_sync::worker --all-features -- --test-threads=1
cargo test --test auto_sync_lifecycle --features test-support -- --test-threads=1
cargo test --test executor_noop_success --features test-support -- --test-threads=1
cargo check --workspace --all-targets --all-features
bash scripts/check.sh
```

If an existing test target has a different exact name, use the nearest current target and record it in the verification section.

Do not require the full release-check script or repeated stress runs.

---

## 6. Prohibited outcomes

The phase fails if it:

- treats any corrupt/unreadable pending state as `NoPending`;
- treats a lock I/O error as lock availability;
- returns `SpawnNow` after spawn failure;
- uses PID metadata as lock authority;
- accepts a lower generation as current work;
- releases the execution lock while a possibly live executor remains unhandled;
- adds new durable state or a new process layer;
- begins the one-helper refactor before these tests pass;
- expands CI, release automation, or test infrastructure.

---

## 7. Closure checklist

- [ ] Scheduling errors are explicit.
- [ ] Spawn result is truthful.
- [ ] Startup recovery uses kernel lock authority.
- [ ] Generation rollback fails closed.
- [ ] Executor cleanup is complete on wait error.
- [ ] Existing status paths expose local state failures.
- [ ] Focused tests pass.
- [ ] `cargo check --workspace --all-targets --all-features` passes.
- [ ] `bash scripts/check.sh` passes.
- [ ] Plan records implementation SHA and verification commands.
- [ ] No architectural simplification work was mixed into this phase.

When complete, proceed to Phase 12C. Do not add a follow-up correctness framework; Phase 12C should delete complexity rather than add more guards around it.
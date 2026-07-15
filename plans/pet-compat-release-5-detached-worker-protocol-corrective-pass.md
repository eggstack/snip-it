# Release 5 Corrective Pass: Detached Worker Protocol, Debounce, and Generation Safety

## Purpose

Close the remaining correctness gaps in the detached one-shot auto-sync worker introduced for Release 5.

The architectural direction is now correct: mutation commands commit local state, record durable pending intent, spawn the current `snp` binary as a detached hidden worker, and return without waiting on remote synchronization. The old monolithic in-process coordinator has also been decomposed into focused modules.

However, the current production protocol still has several blocking defects:

- one logical mutation can increment the pending generation twice;
- the parent acquires and leaks the worker lock before spawn, while the child independently attempts to acquire the same lock;
- the worker performs synchronization immediately rather than honoring the configured quiet-period debounce;
- newer generations created during debounce or sync are not processed through a bounded follow-up cycle;
- timeout enforcement abandons a still-running sync thread and may release the worker lock while sync work continues;
- startup recovery and scheduling can mutate pending state merely by attempting to spawn a worker;
- end-to-end subprocess tests do not yet prove actual sync-attempt counts, timing, or generation ownership.

This pass must correct the worker protocol without adding a daemon, IPC service, second executable, shell wrapper, or service-manager dependency.

## Required Outcomes

After this pass:

1. Each successful logical mutation increments the durable pending generation exactly once.
2. Scheduling an existing pending generation never rewrites, increments, or replaces its snapshot.
3. The parent process never owns the worker execution lock.
4. Every spawned worker races for the worker lock; exactly one worker performs debounce and sync work.
5. The worker honors `auto_sync_debounce_seconds` as a real quiet-period debounce across separate CLI processes.
6. Rapid mutations coalesce into one sync attempt after the final mutation in the quiet window.
7. A mutation arriving during sync is preserved and results in at most one coalesced follow-up cycle before the worker exits.
8. Older workers cannot clear, overwrite, or mark success for newer generations.
9. A sync timeout means the underlying async operation is actually cancelled or bounded before the worker releases its lock.
10. Startup recovery can reschedule existing pending work without incrementing generation.
11. Explicit/manual sync clears pending intent safely without racing an older worker into deleting newer state.
12. Detached auto-sync failures never alter the already-completed parent mutation exit status.
13. Documentation, doctor output, status semantics, and tests describe the final detached-worker contract accurately.
14. Release 5 receives closure evidence from real multi-process tests, not only unit tests of isolated storage and lock primitives.

## Product Invariants

1. Local state commits before any pending marker or worker spawn attempt.
2. Remote failure never rolls back a successful local mutation.
3. Auto-sync is disabled by default.
4. Manual `snp sync`, scheduled sync, cron workflows, and explicit `--sync` behavior remain independently callable.
5. The existing encrypted protocol and conflict model remain unchanged.
6. Mutation commands do not sleep for debounce or wait for network activity.
7. Worker argv, environment, pending markers, lock files, status files, and logs contain no snippet command bodies, descriptions, output metadata, credentials, API keys, encryption material, or credential-bearing URLs.
8. Pending and lock artifacts remain bounded, versioned where applicable, atomically written, and restrictive on supported platforms.
9. Sync-origin writes never trigger auto-sync.
10. Output-only edits remain excluded because `output` is local-only.
11. A worker may perform redundant synchronization after a crash, but must never lose a committed mutation.
12. Failure handling prefers duplicate-safe recovery over clearing uncertain pending state.

## Current Defects to Remove

### Double generation increment

The current parent path performs:

```text
notify_local_mutation
  -> mark_pending(actual mutation)
  -> try_schedule
       -> mark_pending(default snapshot) again
```

This can increment generation twice and replace the actual mutation snapshot with a default snapshot.

### Invalid parent-to-child lock handoff

The parent currently acquires the worker lock, spawns the child, and leaks the RAII guard. The child then attempts to acquire the same lock. There is no real ownership transfer, so the child can observe a valid lock owned by the still-running parent and exit without processing pending work.

### Missing debounce loop

The worker currently reads pending state and immediately invokes sync. It does not wait for the configured quiet period, reload the marker, or observe whether the generation changed.

### Incomplete follow-up handling

A newer generation created while the worker is synchronizing may remain pending until a future CLI invocation. The active worker must detect this and perform one bounded follow-up cycle or explicitly reschedule another worker before exit.

### Unsafe timeout wrapper

The current timeout helper starts a sync thread and returns on channel timeout without cancelling or joining the underlying operation. The worker may release its execution lock while the timed-out sync thread continues.

## Workstream A: Separate Pending Mutation from Worker Scheduling

### A1. Define two explicit APIs

Use distinct operations:

```rust
pub fn record_pending_mutation(
    state_dir: &Path,
    snapshot: PendingSnapshot,
) -> Result<MarkedPending, PendingError>;

pub fn schedule_existing_pending(
    state_dir: &Path,
) -> SpawnResult;
```

`record_pending_mutation` is the only normal mutation path allowed to increment generation.

`schedule_existing_pending` must never call `mark_pending`, change generation, replace snapshot, or alter timestamps.

### A2. Parent notification transaction

The production flow must be:

```text
local mutation commit
  -> record_pending_mutation exactly once
  -> schedule_existing_pending
  -> return parent result
```

Return the generation produced by the single marker write.

### A3. Recovery scheduling

Startup recovery must:

- load an existing valid marker;
- check age and policy;
- schedule that exact existing generation;
- never increment generation merely because recovery ran;
- never replace the snapshot with `None`.

### A4. Tests

Add tests proving:

- one notification changes generation from N to N+1, not N+2;
- scheduling existing pending state leaves serialized bytes unchanged except optional worker/status metadata stored elsewhere;
- startup recovery leaves generation and snapshot unchanged;
- failed spawn preserves the pending generation for later recovery;
- repeated spawn attempts do not grow or rewrite the marker.

## Workstream B: Worker-Only Lock Ownership

### B1. Remove parent lock acquisition

The parent must not acquire, leak, transfer, or pre-create the worker execution lock.

Every mutation may spawn a detached worker. This is acceptable because worker startup is cheap and lock acquisition is the arbiter.

### B2. Child arbitration

Each worker performs:

```text
start
  -> attempt atomic worker-lock acquisition
  -> winner continues
  -> loser exits successfully with NothingToDo
```

The lock contents must identify the worker process, not the parent process.

### B3. Spawn failure behavior

If spawning fails:

- leave pending intent intact;
- do not clear the generation;
- apply bounded parent-side scheduling diagnostics according to policy;
- rely on startup recovery, another mutation, manual sync, or cron as recovery.

### B4. Stale lock policy

Retain PID, start timestamp, and nonce metadata, but document advisory limits including PID reuse.

Prefer stale recovery rules that require both:

- process not alive; or platform-equivalent probe failure;
- lock older than a small safety threshold when process identity is uncertain.

Do not delete a lock merely because it is old while the worker PID is verifiably alive.

### B5. Tests

Use real subprocesses to prove:

- parent remains alive while worker successfully acquires the lock;
- ten spawned workers produce one lock owner;
- losers exit promptly and do not alter pending state;
- stale dead-worker lock is recovered;
- a live worker lock is not stolen;
- the lock file records the child PID.

## Workstream C: Real Quiet-Period Debounce

### C1. Worker debounce loop

After acquiring the worker lock, the worker must repeatedly:

1. read the current pending state;
2. compute `deadline = last_mutation_at + policy.debounce`;
3. sleep only for the remaining duration;
4. reload pending state after waking;
5. if generation or timestamp changed, compute a new deadline;
6. if unchanged and deadline reached, begin sync.

Do not sleep for the full debounce interval after every reload.

### C2. Marker timestamps

The pending schema must contain a timestamp representing the most recent mutation that contributed to the current generation.

Use millisecond or finer resolution. Do not rely on second-resolution timestamps for rapid mutation tests.

### C3. Zero debounce

`debounce = 0` should proceed immediately after lock acquisition and marker validation, while still preserving generation checks.

### C4. Maximum wait and starvation

Decide whether the worker supports a maximum coalescing age separate from the configured quiet period.

If included:

- it must be explicit and documented;
- it must not silently shorten the configured debounce;
- it should prevent an indefinitely active mutation stream from keeping one worker alive forever.

A reasonable contract is:

```text
quiet-period deadline: reset by every mutation
maximum worker cycle age: bounded, after which sync current generation once
```

### C5. Tests

Add deterministic and subprocess tests proving:

- sync does not begin before the quiet period;
- ten mutations inside the window produce one attempt;
- a later mutation extends the deadline;
- zero debounce starts promptly;
- scheduling attempts do not reset the mutation timestamp;
- startup recovery respects the remaining quiet period rather than restarting a full interval unnecessarily.

Use a fake clock for unit tests and real elapsed-time bounds only for a small number of integration tests.

## Workstream D: Generation-Safe Sync Completion

### D1. Observe generation before sync

Immediately before sync, reload pending state and record:

```text
observed_generation
observed_snapshot
observed_mutation_timestamp
```

### D2. Conditional success

After successful sync:

- reload pending state;
- if generation still equals `observed_generation`, clear or record success conditionally;
- if generation is newer, never clear it;
- retain the newer snapshot and timestamp.

### D3. Mutation during sync

If a newer generation exists after sync:

- perform one follow-up debounce/sync cycle in the same worker; or
- explicitly spawn/schedule another worker before releasing the lock.

Preferred contract: the current worker performs one or more bounded follow-up cycles while it owns the lock, with a maximum cycle count or maximum worker lifetime to prevent indefinite residence.

The simplest safe loop is:

```text
loop:
  debounce current generation
  sync observed generation
  conditionally clear observed generation
  reload marker
  if no newer generation: exit
  if newer generation: continue
```

Bound with a maximum worker lifetime rather than dropping work.

### D4. Failure behavior

On sync failure:

- do not clear pending intent;
- record bounded failure status separately or in generation-safe fields;
- leave generation available for recovery/manual sync;
- avoid a tight retry loop.

Retry/backoff should remain bounded and should not cause the worker to outlive the documented maximum unexpectedly.

### D5. Explicit sync race

Manual or explicit sync must clear pending intent only after its successful remote operation.

Use generation-aware clearing where possible:

- capture pending generation before explicit sync;
- after success, clear only if it still matches;
- preserve mutations that occurred while explicit sync was running.

### D6. Tests

Prove:

- old worker completion cannot clear a newer generation;
- mutation during sync produces exactly one later attempt for the newer state;
- failed sync leaves pending intent;
- explicit sync does not clear a mutation created during explicit sync;
- redundant recovery sync is allowed after crash, but lost generation is not.

## Workstream E: Cancellable and Bounded Sync Execution

### E1. Remove abandoned-thread timeout

Do not implement timeout by spawning an unjoined thread and returning while it continues.

### E2. Enforce timeout inside async execution

Preferred implementation:

```rust
runtime.block_on(async {
    tokio::time::timeout(policy.sync_timeout, run_default_sync_async()).await
})
```

If `run_default_sync` is currently synchronous over an async runtime, refactor to expose an async internal function used by:

- manual sync;
- scheduled sync;
- detached worker.

Keep existing public wrappers for compatibility.

### E3. Cancellation semantics

When timeout fires:

- the future is dropped;
- all owned requests/tasks must be cancelled or bounded;
- the worker must not release its lock while hidden sync work continues;
- local sync-merge writes must not be left halfway through an unprotected transaction.

Audit spawned background tasks inside the sync implementation. A timeout around the top-level future is insufficient if detached tasks continue independently.

### E4. Network-level bounds

Ensure DNS, connection, request, and response operations use bounded timeouts compatible with the worker-level timeout.

### E5. Tests

Use a fake sync executor or test server to prove:

- timeout ends the underlying operation;
- no sync-attempt marker is written after timeout completion;
- another worker/manual sync can safely acquire the lock afterward;
- no detached thread survives beyond worker completion in tests;
- retry count and backoff remain bounded.

## Workstream F: Worker Spawn and Detachment Validation

### F1. Hidden command contract

Keep the worker hidden from normal help. It must accept only internal non-secret arguments such as:

```text
--state-dir
--nonce
```

Prefer deriving state directory from configuration when possible; explicit state-dir support may remain for tests.

### F2. Direct re-exec

Use `std::env::current_exe()` and direct argv construction. Never invoke a shell, `nohup`, `setsid` command strings, or PATH-resolved `snp`.

### F3. Platform behavior

Unix:

- start a new session using `setsid()` in the child setup path;
- null stdin/stdout/stderr;
- verify terminal closure does not terminate the worker.

Windows:

- use appropriate detached/no-window creation flags;
- null standard handles;
- verify the parent console can close without terminating the worker.

### F4. Spawn observability

Do not require parent/child IPC. Durable marker plus lock state remains the coordination protocol.

A short-lived nonce sentinel may be retained only if it has a clear purpose. Avoid unbounded `.done` file accumulation.

If nonce replay protection is unnecessary once lock ownership is worker-only, remove it.

### F5. Tests

Prove:

- parent exits before unreachable network timeout;
- no delayed output appears in the parent terminal;
- worker survives parent exit;
- worker argv and environment contain no sentinel secrets;
- repeated operation does not accumulate unbounded nonce artifacts.

## Workstream G: Startup Recovery and Scheduling

### G1. Fast startup probe

For non-worker commands:

- check for a valid pending marker;
- check whether a worker lock is currently live;
- spawn a worker only when pending exists and no active worker owns it;
- never block the command on sync.

### G2. Stale pending state

Do not automatically discard pending work merely because it is older than five minutes unless product semantics explicitly define it as obsolete.

A stale marker generally means unsynchronized local work and should be retried, not cleared.

Prefer:

- mark status as stale/recovery-needed;
- schedule a worker or leave it for manual sync;
- clear only corrupt, explicitly superseded, or successfully synchronized state.

### G3. Corrupt marker

Corrupt markers should:

- fail closed for auto execution;
- emit bounded diagnostics;
- never modify snippet libraries;
- be preserved or quarantined for inspection according to existing file-recovery policy.

### G4. Tests

Prove:

- a pending marker left by a crashed parent is rescheduled on next invocation;
- startup recovery does not increment generation;
- startup recovery does not spawn duplicate workers when one is live;
- old but valid pending work is not silently discarded;
- corrupt pending state does not block ordinary local commands.

## Workstream H: Failure and Status Semantics

### H1. Detached failure policy

Define final meanings:

```text
ignore:
  record bounded status only

warn:
  record status and expose warning through doctor/status surfaces

error:
  record failed state requiring attention, but do not change the already-completed parent mutation exit code
```

### H2. Parent scheduling failure

A failure to write pending state or spawn the worker is immediate and may produce parent stderr diagnostics. Even then, the local mutation remains committed.

Do not return a nonzero mutation exit code unless the CLI contract explicitly distinguishes “local mutation committed, scheduling failed” and scripts are documented against retry hazards. The preferred contract is successful local exit plus warning/status.

### H3. Durable status

Store only bounded non-secret fields:

```text
last_attempt_at
last_success_at
last_failure_at
failure_class
pending_generation
worker_pid if active
```

Do not store raw error bodies or credential-bearing endpoints.

### H4. Doctor and sync status

Expose:

- auto-sync enabled/disabled;
- pending generation and age;
- worker active/stale;
- last result classification;
- recovery recommendation.

Keep JSON stdout machine-clean and human diagnostics on stderr according to existing stream policy.

## Workstream I: Simplify the Worker Protocol

### I1. Remove disconnected abstractions

Delete or reduce APIs that no longer serve production flow, including:

- scheduling functions that also mutate pending state;
- nonce replay state if worker-only locking makes it redundant;
- legacy coordinator terminology;
- dead status variants tied to synchronous parent execution.

### I2. Preserve module boundaries

Retain the current focused layout:

```text
src/auto_sync/
  mod.rs
  policy.rs
  pending.rs
  lock.rs
  spawn.rs
  worker.rs
  notification.rs
```

Add `status.rs` or `executor.rs` only if it materially improves separation.

### I3. Keep public API narrow

Mutation commands should need only:

```rust
notify_mutation(kind, origin)
clear_pending_after_explicit_sync(...)
startup_recover_pending()
```

Do not expose lock or marker internals broadly.

## Workstream J: End-to-End Test Harness

### J1. Observable sync-attempt counter

Introduce a test-only sync executor or local test server capable of recording:

- attempt start time;
- attempt completion time;
- attempt count;
- concurrency count;
- cancellation;
- injected failures and delays.

Do not infer debounce merely from marker presence.

### J2. Required multi-process scenarios

Add subprocess tests for:

1. one mutation increments generation once;
2. one mutation returns before worker network completion;
3. ten mutations inside two seconds cause one attempt;
4. a later mutation extends the quiet deadline;
5. zero debounce causes one prompt detached attempt;
6. multiple workers race and exactly one owns execution;
7. mutation during sync causes one follow-up attempt;
8. older generation completion cannot clear newer pending state;
9. worker crash leaves recoverable pending state;
10. startup recovery schedules without incrementing generation;
11. explicit sync clears only the observed generation;
12. timed-out sync is cancelled before lock release;
13. no parent stdout/stderr contamination;
14. no secret sentinel in argv, env, files, or logs;
15. no unbounded lock, temp, nonce, or status artifact accumulation.

### J3. PTY coverage

Verify terminal lifecycle for interactive edit/delete flows that trigger auto-sync:

- terminal restored before worker activity;
- no delayed warning appears on the controlling terminal;
- closing the terminal does not kill a correctly detached worker;
- machine-facing select/list output remains unchanged.

### J4. Platform matrix

Run supported CI on:

- Linux;
- macOS;
- Windows.

Gate platform-specific detachment tests appropriately, but do not claim support without an exercised path.

## Workstream K: Documentation Reconciliation

Update:

- `README.md`;
- `USER_GUIDE.md`;
- `AGENTS.md`;
- `CHANGELOG.md`;
- `architecture/auto_sync.md`;
- `architecture/sync.md`;
- `architecture/overview.md`;
- `docs/ARCHITECTURE_INVENTORY.md`;
- `docs/CLI_EXITCODE_STREAM_POLICY.md`;
- `docs/PET_COMPATIBILITY.md`.

Remove obsolete claims that:

- the parent owns debounce execution;
- scheduling increments pending state;
- `error` mode changes the completed mutation command exit status;
- old pending work is automatically disposable;
- thread timeout cancels underlying sync work;
- lock ownership transfers from parent to worker.

Document the final sequence precisely:

```text
commit local mutation
  -> increment generation once
  -> spawn detached worker
  -> parent exits
  -> worker-only lock
  -> quiet-period debounce
  -> generation-safe sync
  -> conditional clear or follow-up
  -> worker exits
```

## Suggested Implementation Sequence

### Commit 1: Generation and scheduling separation

- split marker mutation from spawn scheduling;
- remove double `mark_pending`;
- fix startup recovery scheduling;
- add exact-generation tests.

### Commit 2: Worker-only lock ownership

- remove parent lock acquisition and `mem::forget` handoff;
- child acquires lock directly;
- add parent-alive and multi-worker tests.

### Commit 3: Quiet-period debounce and follow-up loop

- implement reload/sleep/reload generation loop;
- support zero debounce and bounded worker lifetime;
- add attempt-count and timing tests.

### Commit 4: Cancellable async timeout

- expose async sync entry point;
- remove abandoned-thread timeout;
- add cancellation and lock-release tests.

### Commit 5: Recovery, explicit-sync races, and status

- generation-safe explicit clearing;
- preserve old valid pending work;
- reconcile failure/status semantics;
- update doctor/status behavior.

### Commit 6: Documentation and closure

- remove stale architecture claims;
- run full workspace, PTY, concurrency, security, and platform matrix;
- record Release 5 closure evidence.

## Validation Commands

At minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --test auto_sync_config
cargo test --test auto_sync_coordinator
cargo test --test auto_sync_concurrency
cargo test --test auto_sync_mutations
cargo test --test auto_sync_regression
cargo test --test auto_sync_security
cargo test --test pty_integration -- --test-threads=1
```

Add the new detached-worker multi-process test target explicitly once created.

## Exit Criteria

Release 5 may close only when all of the following are true:

1. Production mutation notification performs one generation increment.
2. Scheduling and startup recovery never mutate existing pending generation.
3. Parent processes never acquire the worker execution lock.
4. A worker can acquire the lock while the parent is still alive.
5. Configured debounce is observable in real subprocess behavior.
6. Rapid mutations coalesce into the expected attempt count.
7. Newer generations survive older worker completion.
8. Mutation during sync is processed through a bounded follow-up cycle.
9. Timeout cancels or fully bounds the underlying sync operation before lock release.
10. Explicit sync uses generation-safe pending clearing.
11. Old valid pending work is recoverable rather than silently discarded.
12. Parent stdout, stderr, exit code, and terminal lifecycle remain compatible.
13. No secret-bearing data enters worker coordination artifacts.
14. No unbounded artifact accumulation occurs.
15. Full workspace, serialized PTY, security, concurrency, and supported-platform CI pass.
16. Documentation describes the shipped detached-worker protocol exactly.

## Non-Goals

- Persistent daemon or resident background service.
- Unix socket, named pipe, HTTP, or custom IPC protocol.
- A second worker binary.
- Service-manager installation.
- Shell-based detachment.
- Per-library sync targeting before protocol support exists.
- Synchronizing local-only output or usage metadata.
- Changing encryption, conflict resolution, or remote sync backends.
- Making detached worker failures retroactively fail a completed local mutation.

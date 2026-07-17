# Phase 01: Auto-Sync Correctness Closure

## Purpose

Correct the release-blocking detached auto-sync defect and establish one truthful synchronization execution model shared by all entry points.

The current `auto-sync-execute` implementation validates configuration, performs no synchronization, logs a placeholder success, and exits with code zero. The worker interprets that code as success and may clear pending state. This phase must remove that behavior before any further auto-sync polish is attempted.

## Priority

Critical. Do not publish or describe the current detached auto-sync path as reliable until this phase is complete.

## Entry state

Relevant current modules include:

- `src/auto_sync/executor.rs`
- `src/auto_sync/worker.rs`
- `src/auto_sync/execution_lock.rs`
- `src/auto_sync/spawn.rs`
- `src/auto_sync/notification.rs`
- `src/sync_commands.rs`
- foreground sync command wrappers in `src/commands/`
- hidden subcommands in `src/main.rs`

The implementation already has:

- pending generations;
- conditional clear;
- a detached worker;
- a non-detached executor child;
- timeout supervision;
- a shared synchronization execution lock;
- internal executor exit codes.

The implementation does not yet have truthful executor execution or a consistent lock-ownership contract.

## Required invariants

1. Exit code zero from `auto-sync-execute` means the canonical sync operation completed successfully or proved the target was already current.
2. The executor cannot return success without invoking the canonical sync path.
3. Pending state is cleared only after true success and only when the observed generation still matches.
4. Every non-success outcome preserves pending state.
5. Exactly one component owns the execution lock during a detached cycle.
6. The worker can terminate and reap the executor before releasing the execution lock.
7. Manual sync, cron, explicit `--sync`, and detached sync use identical direction and merge semantics.
8. No synchronization implementation is duplicated solely for the subprocess path.

## Architecture decision

Use worker-owned execution locking.

Final detached topology:

```text
parent mutation command
  -> record pending generation
  -> spawn detached worker
  -> return

worker
  -> acquire SyncExecutionLock
  -> debounce current pending state
  -> spawn non-detached executor
  -> supervise timeout and termination
  -> map executor status
  -> conditionally clear pending on true success
  -> release SyncExecutionLock

executor
  -> load validated sync configuration
  -> invoke canonical sync function
  -> return typed internal exit code
  -> never reacquire SyncExecutionLock
```

Do not implement the existing executor TODO literally if it says to acquire the same lock. That would make the child contend with the parent that is waiting for it.

## Workstream A: Extract the canonical sync operation

Identify the current implementation used by manual `snp sync`, cron, and explicit `--sync`. Refactor it into one reusable internal function with no CLI rendering and no lock acquisition.

Recommended shape:

```rust
#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub direction: SyncDirection,
    pub origin: SyncOrigin,
}

#[derive(Debug)]
pub struct SyncReport {
    pub libraries_examined: usize,
    pub libraries_uploaded: usize,
    pub libraries_downloaded: usize,
    pub libraries_unchanged: usize,
    pub conflicts: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncExecutionError {
    // Initial variants may map existing errors without completing Phase 03.
}

pub fn execute_sync_request(
    settings: &SyncSettings,
    request: &SyncRequest,
) -> Result<SyncReport, SyncExecutionError>;
```

The exact names may differ. The required properties are:

- no lock acquisition;
- no process spawning;
- no direct `process::exit`;
- no unconditional printing;
- no fallback success;
- direction is explicit;
- result is typed enough for the executor adapter;
- foreground and detached callers delegate to it.

Do not create a second sync client or copy the manual sync body into `executor.rs`.

## Workstream B: Implement the executor

Replace the placeholder path in `run_executor`.

Required flow:

1. Load sync settings through a fallible API.
2. Distinguish missing/disabled/unconfigured state from malformed or inaccessible configuration.
3. Resolve configured direction for detached operation.
4. Invoke the canonical sync function.
5. Map the real result into `ExecutorExitCode`.
6. Flush any required local persistence before reporting success.
7. Exit nonzero for all errors and partial results.

The executor must not acquire the execution lock because the worker owns it.

The executor should not detach itself. It must remain a direct child that the worker can wait on and terminate.

## Workstream C: Define truthful success and no-op semantics

Distinguish:

- successful changes applied;
- successful comparison with no changes required;
- disabled or unconfigured;
- authentication failure;
- network/timeout failure;
- conflict or partial result;
- local persistence failure;
- internal error.

Both “changes applied” and “already current” may map to process exit code zero because a real comparison completed.

Disabled or unconfigured must not map to zero merely to make the worker quiet. It must preserve pending state.

## Workstream D: Reconcile foreground paths

Audit every actual sync entry point:

- `snp sync`;
- cron invocation;
- `run --sync`;
- `clip --sync` if supported;
- search/select/delete flows that explicitly request sync;
- startup recovery scheduling;
- detached executor.

For each entry point:

- acquire `SyncExecutionLock` exactly once where required;
- call the canonical sync function;
- use the same effective direction resolver;
- use the same error mapping;
- preserve existing user-facing output where compatible;
- keep foreground failures nonzero;
- keep detached failures durable but asynchronous.

Remove wrappers that produce false timeout semantics or invoke a second implementation.

## Workstream E: Correct worker result handling

Update the worker so that:

- `ExecutorExitCode::Success` is the only result that permits conditional clear;
- `NotConfigured` preserves pending;
- authentication, network, conflict, persistence, and internal failures preserve pending;
- executor spawn failure preserves pending;
- executor signal termination preserves pending;
- timeout preserves pending;
- unknown or platform-specific exit status preserves pending;
- `NothingToDo` is reserved for genuinely absent work or lock contention before ownership, not policy-disabled pending intent.

Do not clear pending on a generic `NothingToDo` branch unless the marker is already absent or a real successful comparison proved no changes were needed.

## Workstream F: Timeout and process lifecycle

Retain the process-supervision model, but verify:

1. Worker holds the execution lock before executor spawn.
2. Worker waits for normal exit until the configured timeout.
3. On timeout, graceful termination is attempted where supported.
4. A bounded grace interval follows.
5. Force termination is used if still alive.
6. The child is waited/reaped after termination.
7. The execution lock is released only after child death/reap.
8. Pending state remains.

On Unix, consider whether direct-child signaling is sufficient. If the canonical sync operation can spawn descendants, document that process-group termination is deferred to Phase 09 or implement it now if necessary for truthful timeout.

## Workstream G: Regression tests

### Mandatory real-server test

Add a test that exercises the actual binary path:

1. Create isolated temporary config and data roots.
2. Start a real ephemeral `snip-sync` server on a dynamic local port.
3. Configure sync credentials safely.
4. Create a snippet using the real `snp` binary.
5. Wait through a bounded condition-based loop for the detached cycle.
6. Verify server-side encrypted state or a server-observable library revision changed.
7. Verify pending state clears only after the server-observable change.

This test must fail against the current placeholder executor.

### Mandatory negative tests

For each case, verify local mutation succeeds and pending remains:

- server unavailable;
- authentication rejected;
- executor spawn failure via test hook;
- executor nonzero exit;
- executor timeout;
- executor killed by signal;
- local persistence failure;
- malformed configuration;
- partial/conflict result;
- unknown exit code.

### Lock ownership test

Use a barrier-controlled executor test mode or recording server to prove:

- worker owns the lock while child runs;
- executor does not attempt a second acquisition;
- a manual sync cannot enter concurrently;
- lock is released after child completion or confirmed death.

### Direction parity tests

Prove worker and foreground paths resolve Push, Pull, and Bidirectional identically.

## Workstream H: Documentation cleanup

Update:

- architecture overview;
- auto-sync deep dive;
- executor module docs;
- worker module docs;
- `AGENTS.md` references;
- README auto-sync policy text;
- `USER_GUIDE.md` sync behavior;
- changelog.

Remove:

- claims that the executor currently performs real sync if it did not;
- TODO text describing future behavior after implementation;
- contradictory statements that both worker and executor acquire the execution lock;
- false cancellation language from earlier in-process approaches.

## Recommended commit sequence

1. Add regression test demonstrating placeholder success defect.
2. Extract canonical sync operation and typed report/error boundary.
3. Route foreground sync paths through the canonical operation.
4. Implement real executor and exit-code mapping.
5. Correct worker clearing and failure behavior.
6. Add lock-ownership and timeout lifecycle tests.
7. Add real-server positive and negative integration matrix.
8. Reconcile documentation and remove obsolete wrappers/comments.

Keep commits bisectable. Do not combine unrelated CLI or storage refactors with this phase.

## Verification commands

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
```

Also run the dedicated real-server and detached-worker tests on Linux, macOS, and Windows.

## Exit criteria

Phase 01 is complete only when:

- `run_executor` contains no placeholder success path;
- all actual sync entry points delegate to one canonical operation;
- worker-owned lock semantics are documented and tested;
- executor never reacquires that lock;
- pending clears only on real success/already-current comparison;
- every failure preserves pending;
- timeout kills and reaps before unlock;
- a real-server regression test proves remote state changes before clear;
- the same test fails when canonical sync invocation is replaced with a no-op;
- all platform CI jobs pass;
- documentation describes the implemented behavior exactly.

## Handoff warning

Do not mark this phase complete based only on unit tests for exit-code conversion or lock-file handling. The defining evidence is a real subprocess-to-server effect followed by generation-safe pending clear.

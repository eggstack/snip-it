# Release 5F: Unified Sync Execution and Timeout Correctness

## Purpose

Make every sync entry point mutually exclusive and ensure timeout semantics are truthful.

The detached worker, manual `snp sync`, explicit `--sync`, and scheduled/cron sync currently do not share one execution lock. The worker timeout also wraps a `spawn_blocking` task, which is not cancelled when the timeout future is dropped. These defects can permit overlapping syncs and release the lock while synchronous work is still active.

This phase must preserve the lightweight one-shot helper model while making sync execution singular, bounded, and observable.

## Required Outcomes

1. Exactly one sync operation may execute against local libraries and remote state at a time.
2. Detached worker, manual sync, explicit `--sync`, cron, and recovery paths all use the same execution guard.
3. Startup recovery never spawns a worker immediately before a foreground sync command.
4. A timeout means the underlying sync operation has stopped before the execution lock is released.
5. No `spawn_blocking` task survives a timed-out worker cycle.
6. Configured push, pull, or bidirectional direction is respected by every sync entry point.
7. Explicit/manual sync and worker pending-state clearing remain generation safe.
8. Foreground commands report lock contention and timeout outcomes clearly.
9. Detached workers preserve pending work on any execution failure.
10. Tests prove mutual exclusion and actual process termination, not merely future timeout.

## Architecture Decision

Use one shared `SyncExecutionLock` for all real sync operations.

Because the current sync engine is predominantly synchronous and internally creates/block-on runtimes, do not claim in-process future cancellation unless the entire sync stack is converted to native async. The lowest-complexity correct design is:

- parent mutation command spawns the existing one-shot debounce worker;
- debounce worker acquires `SyncExecutionLock` before actual sync;
- worker launches a hidden one-shot sync executor subprocess using the same binary;
- executor performs one foreground sync and exits;
- worker waits with a timeout;
- on timeout, worker terminates the executor process and waits for confirmed exit;
- only then may the worker release `SyncExecutionLock`.

This retains one binary and no daemon while providing a kill boundary around synchronous sync code.

Suggested hidden subcommand:

```text
snp auto-sync-execute --state-dir <dir>
```

It should be hidden, accept no snippet payload or credentials in argv, load configuration normally, run exactly one sync, and return a meaningful internal exit code.

## Workstream A: Shared SyncExecutionLock

Create a dedicated execution-lock module or generalize the corrected lock primitive from Release 5E.

All of these paths must acquire it before calling sync orchestration:

- detached worker executor;
- `snp sync`;
- `snp run --sync`;
- `snp clip --sync`;
- `snp search --sync`;
- TUI delete with explicit sync;
- cron/scheduled sync;
- any registration or recovery path that mutates remote sync state, if applicable.

Lock behavior:

- foreground manual sync may wait for a bounded period or fail with a clear message;
- detached worker should preserve pending work and exit/retry later when the lock is busy;
- ownership release must be nonce checked;
- no live-owner age stealing;
- status/doctor output should identify owner PID and start time without exposing secrets.

## Workstream B: Startup Recovery Command Classification

Move startup recovery behind explicit command classification.

Do not schedule a detached worker before commands that are themselves about to sync or modify sync policy:

- `auto-sync-worker`;
- `auto-sync-execute`;
- `sync` and all sync subcommands;
- cron execution paths;
- register/account setup;
- doctor repair operations that inspect or mutate worker state.

Read-only commands may perform a cheap pending check and schedule recovery only when no execution lock is held.

Prefer a function such as:

```rust
fn should_attempt_auto_sync_recovery(command: &Option<Commands>) -> bool;
```

with exhaustive tests for every command variant.

## Workstream C: Killable Sync Executor

Implement the hidden executor subprocess.

Parent worker flow:

1. Complete debounce and capture latest pending state.
2. Acquire `SyncExecutionLock`.
3. Spawn `current_exe auto-sync-execute --state-dir ...` with null standard streams or bounded private log output.
4. Wait for completion with configured sync timeout.
5. If timeout expires, request graceful termination where supported.
6. Escalate to forceful termination after a short grace period.
7. Wait/reap the child.
8. Only after confirmed process exit, update pending status and release the execution lock.

Platform requirements:

- Unix: SIGTERM, bounded grace period, then SIGKILL, followed by `wait`.
- Windows: use child termination APIs and wait for process completion.
- Never leave zombies.
- Never release the execution lock while the executor remains alive.

The executor must not spawn another detached worker or run startup recovery.

## Workstream D: Remove False Async Cancellation

Remove or stop using the current `run_default_sync_async -> spawn_blocking` timeout path.

Possible future work may convert sync to true async, but this phase should not mix partial async wrappers with synchronous orchestration.

Update comments and architecture docs so they do not claim dropping a `spawn_blocking` join future cancels the task.

## Workstream E: Direction Correctness

Create one canonical effective-direction resolver used by manual and automatic sync:

```rust
pub fn effective_sync_direction(
    settings: &SyncSettings,
    cli_push_only: bool,
    cli_pull_only: bool,
) -> SyncDirection;
```

Rules:

- explicit CLI flags override configuration;
- no CLI override means use `settings.sync_direction`;
- worker/executor with no flags uses configured direction;
- invalid simultaneous overrides remain rejected by Clap;
- direction appears in bounded diagnostic/status output.

Do not call `run_sync(settings, ..., false, false)` when that silently means bidirectional regardless of configuration.

## Workstream F: Foreground/Detached Contention Semantics

Define precise behavior:

### Detached worker sees lock busy

- do not clear pending;
- record `deferred_busy` status;
- exit successfully as a scheduler outcome, not a sync success;
- later startup recovery or mutation reschedules work.

### Foreground manual sync sees lock busy

Choose and document one behavior:

- default: wait up to a small bounded interval, then return nonzero with owner details and retry guidance;
- optional future `--wait` flag may extend the bound.

Do not run concurrently.

### Manual sync succeeds

- clear only the observed pending generation through Release 5E transactional conditional clear;
- if a newer generation exists, leave it pending and schedule a worker after releasing the execution lock.

## Workstream G: Executor Exit and Failure Classification

Define internal exit codes for the hidden executor:

- 0: sync completed successfully;
- 2: sync not configured/disabled;
- 3: authentication failure;
- 4: network/timeout failure;
- 5: conflict or partial sync failure;
- 6: local persistence failure;
- 7: internal error.

The worker translates these into durable bounded failure status. The parent mutation command never waits for or inherits these codes.

## Tests

### Mutual exclusion tests

1. Start a recording sync executor that blocks at a controlled barrier.
2. Launch detached worker sync.
3. Launch manual sync while worker owns execution lock.
4. Assert manual sync does not enter the server or local merge section concurrently.
5. Reverse the order and assert worker defers while manual sync owns the lock.
6. Run cron and explicit `--sync` variants against the same barrier.

### Timeout tests

- executor intentionally hangs;
- worker timeout expires;
- executor PID is confirmed dead before lock removal;
- a second sync starts only after the first child is reaped;
- no orphan or zombie remains;
- pending state remains intact after timeout.

### Direction tests

- configured Push causes worker to push only;
- configured Pull causes worker to pull only;
- configured Bidirectional does both;
- manual CLI override wins over configuration;
- recording server verifies actual RPC/operation pattern where feasible.

### Recovery classification tests

- `snp sync` does not pre-spawn worker;
- `snp sync config` does not pre-spawn worker;
- worker/executor subcommands do not recurse;
- read-only command can reschedule pending work only when execution lock is free.

### Platform tests

- Unix executor termination and reaping;
- Windows child termination and handle cleanup;
- detached worker remains console-free on Windows;
- macOS session detachment remains valid.

## Documentation

Update architecture docs to show two one-shot subprocess roles:

- debounce worker;
- killable sync executor.

Clarify that this is not a daemon and adds no persistent process or IPC protocol.

## Recommended Commit Sequence

1. Add shared `SyncExecutionLock` and command classification.
2. Route all foreground sync paths through the lock.
3. Add hidden executor subprocess.
4. Replace `spawn_blocking` timeout with process supervision.
5. Centralize effective direction resolution.
6. Add deterministic contention and timeout tests.
7. Reconcile docs and remove obsolete async-cancellation code.

## Exit Criteria

- no two sync operations can overlap;
- timed-out work is confirmed terminated before unlock;
- no `spawn_blocking` cancellation claim remains;
- all sync paths respect configured direction;
- startup recovery cannot race a foreground sync command;
- deterministic Linux, macOS, and Windows tests pass;
- Clippy and formatting are clean.

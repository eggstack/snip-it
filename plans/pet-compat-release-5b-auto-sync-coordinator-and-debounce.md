# Release 5B Plan: Auto-Sync Coordinator, Debounce, and Process-Lifecycle Safety

## Purpose

Implement the reusable coordinator that receives successful local mutation events, coalesces rapid changes, and invokes the existing encrypted synchronization path without blocking interactive commands indefinitely.

This phase provides infrastructure only. Release 5C wires concrete mutation commands into it.

## Product Invariants

1. Local writes are complete before an event enters the coordinator.
2. The coordinator never mutates snippet libraries directly.
3. All remote work uses the existing sync implementation and configuration.
4. Rapid mutations coalesce into bounded sync attempts.
5. Auto-sync is disabled unless the effective Release 5A policy enables it.
6. Process shutdown must not silently promise work that cannot complete.
7. No shell invocation or plaintext helper arguments are used.
8. Manual and scheduled sync remain independent and callable exactly as before.
9. A failed sync cannot roll back local content.
10. Command bodies, output metadata, credentials, and encryption material never appear in coordinator state or logs.

## Architectural Decision: In-Process, Helper Process, or Persistent Worker

Evaluate the current CLI lifecycle and choose the smallest correct design.

### Option A: In-process debounce

Suitable only when the originating process remains alive long enough to execute the delayed sync. This is likely insufficient for short-lived CLI mutation commands unless they wait for the debounce window.

### Option B: Detached helper process

The mutation command spawns the same binary or a dedicated internal mode that owns the debounce and sync attempt.

Requirements:

- no secrets in argv;
- single-instance/coalescing coordination;
- bounded lifetime;
- platform-compatible process detachment;
- clear observability and cleanup.

### Option C: Persistent daemon/service

Use only if the repository already has a suitable long-running process. Do not introduce a new daemon solely for Release 5 unless simpler designs cannot satisfy correctness.

### Preferred decision process

Document the chosen design and rejected alternatives. The implementation must honestly describe delivery guarantees. Do not label a best-effort detached task as durable unless state is persisted.

## Workstream A: Auto-Sync Request Model

Define a minimal request that contains no snippet content:

```rust
pub struct AutoSyncRequest {
    pub library_id: Option<String>,
    pub mutation_kind: MutationKind,
    pub requested_at: i64,
}
```

Only include fields needed to choose the existing sync target. Prefer stable library identifiers or configured mapping keys over filesystem paths. If paths are necessary, validate that they remain under the configured library root and do not log them at normal levels.

## Workstream B: Debounce Semantics

Define exact behavior:

- first mutation starts a debounce window;
- later mutations within the window extend or merge the pending request;
- one sync runs after the quiet period;
- mutations arriving during an active sync schedule at most one follow-up attempt;
- different libraries either share one global sync attempt or maintain separate keys, based on current sync semantics;
- maximum delay is bounded so continuous mutation cannot postpone sync forever.

Suggested state machine:

```text
Idle
  -> Pending(deadline)
Pending + mutation
  -> Pending(updated deadline, bounded by max delay)
Pending deadline
  -> Running
Running + mutation
  -> Running(follow_up = true)
Running complete + follow_up
  -> Pending(new short deadline)
Running complete
  -> Idle
```

Specify:

- debounce duration;
- maximum coalescing window;
- overlap prevention;
- retry behavior;
- treatment of process restart.

## Workstream C: Cross-Process Coordination

Multiple `snp` processes may mutate concurrently. Prevent each process from independently launching redundant sync attempts.

Evaluate:

- advisory lock file;
- atomic create-new lease file;
- existing repository lock primitives;
- OS-specific file locks;
- persisted request marker plus lock owner metadata.

Requirements:

- stale lock recovery;
- no permanent deadlock after crash;
- restrictive permissions;
- atomic updates;
- bounded metadata;
- no secrets;
- no command bodies;
- deterministic behavior on filesystems without reliable advisory locks.

A last-writer-wins marker may be acceptable only if it cannot lose the fact that a sync is required.

## Workstream D: Durable Pending State

Decide whether pending sync intent survives process exit or crash.

### Best-effort model

If pending intent is not durable, document that auto-sync is convenience only and manual/scheduled sync remains the recovery path.

### Durable marker model

Persist a small local marker such as:

```toml
version = 1
pending = true
requested_at = 0
last_attempt_at = 0
last_result = "pending"
```

Do not persist snippet content or credentials.

On startup or the next mutation, stale pending intent should trigger or reschedule sync.

Prefer a durable marker if it materially simplifies truthful delivery guarantees and cross-process coalescing.

## Workstream E: Existing Sync Invocation

Extract or expose a reusable sync entry point that can be called without CLI output contamination.

Requirements:

- accepts resolved configuration/policy;
- uses existing encryption and conflict handling;
- has bounded timeout;
- supports cancellation/shutdown where appropriate;
- returns a structured result;
- separates user-facing rendering from core execution;
- does not recursively schedule auto-sync from local writes performed by the sync merge itself.

Introduce an origin/context flag if needed:

```rust
pub enum MutationOrigin {
    User,
    Import,
    SyncMerge,
    Recovery,
}
```

`SyncMerge` mutations must not trigger another automatic sync loop.

## Workstream F: Timeouts, Retries, and Backoff

Interactive commands must not block indefinitely.

Define:

- sync attempt timeout;
- whether auto-sync retries;
- maximum retry count;
- exponential backoff bounds;
- retryable versus permanent failures;
- authentication/configuration failures;
- conflict handling;
- offline behavior.

Avoid aggressive retry loops. Manual and scheduled sync must remain available for recovery.

Recommended initial policy:

- one attempt after debounce;
- bounded timeout;
- persist/report failure according to policy;
- no immediate retry except one follow-up if a newer mutation arrived;
- leave repeated recovery to later mutation, startup recovery, manual sync, or cron.

## Workstream G: Result and Status Model

Create structured status suitable for diagnostics:

```rust
pub enum AutoSyncStatus {
    Disabled,
    Pending,
    Running,
    Succeeded { completed_at: i64 },
    Failed { completed_at: i64, class: FailureClass },
}
```

Expose status through a safe command or doctor report without secrets.

Possible surface:

```text
snp sync status --json
```

Do not add a public surface unless useful. At minimum, `snp doctor --compatibility` should inspect stale pending/failed state.

## Workstream H: Failure Policy Rendering

Implement Release 5A policy:

- `ignore`: no user-facing warning from the originating mutation;
- `warn`: concise stderr warning when a synchronous result is available, otherwise status/log entry plus next-command diagnostic strategy;
- `error`: only supported when the caller waits for a definitive result; otherwise reject or redefine this policy.

Do not pretend a detached future failure can change an already-exited command's status.

If the design is asynchronous, consider limiting supported modes to `ignore` and `warn`, or define `error` as “fail to schedule” rather than “remote sync failed.” Document precisely.

## Workstream I: Shutdown and Signal Behavior

Test:

- normal command exit;
- SIGINT/SIGTERM during debounce;
- crash/stale lock;
- helper process termination;
- system shutdown;
- Windows process lifecycle where supported.

No terminal state is owned by the coordinator. It must not inherit or hold controlling-terminal resources unnecessarily.

## Workstream J: Security and Privacy

Requirements:

- no secrets in argv, environment dumps, lock files, marker files, logs, or status JSON;
- restrictive permissions for coordinator files;
- server URL redaction;
- no shell execution;
- validate helper mode cannot be invoked to bypass normal sync configuration;
- reject arbitrary library paths or forged request payloads;
- use versioned local state formats;
- bound file sizes and parse complexity.

## Workstream K: Tests

### Unit tests

Cover:

1. debounce state transitions;
2. rapid mutation coalescing;
3. maximum delay bound;
4. mutation during running sync;
5. disabled policy;
6. stale lock recovery;
7. durable marker round-trip;
8. retry classification;
9. sync-origin suppression;
10. no secret-bearing fields in request/status serialization;
11. timeout behavior;
12. failure-policy mapping.

Use a fake clock and fake sync executor. Do not make tests sleep for real debounce intervals.

### Integration tests

Cover:

1. multiple rapid request submissions produce one fake sync call;
2. concurrent processes do not launch duplicate syncs;
3. process crash leaves recoverable state;
4. corrupt coordinator state fails safe;
5. manual sync still works;
6. scheduled sync path remains unchanged;
7. stdout remains clean;
8. no recursive sync after remote merge writes;
9. helper process receives no secrets in argv;
10. disabled config creates no coordinator files where feasible.

### Platform tests

Validate Unix and Windows behavior for locking/process launch if the chosen design is cross-platform. If auto-sync is platform-limited, document and gate it explicitly without breaking compilation.

## Acceptance Criteria

Release 5B is complete when:

- one coordinator owns debounce and overlap prevention;
- rapid mutations coalesce deterministically;
- process-lifecycle guarantees are truthful and documented;
- cross-process behavior cannot create sync storms or permanent locks;
- existing sync core is reused without recursive triggers;
- secrets and snippet content never enter coordinator state;
- tests use fake time/executors and cover crash/recovery;
- no mutation command is wired until the coordinator is ready for Release 5C.

## Non-Goals

- New sync protocol or provider.
- Synchronizing usage or output metadata.
- Full offline queue of historical mutations.
- Guaranteed exactly-once remote delivery.
- A general-purpose background job framework.
- Triggering sync before local commit.

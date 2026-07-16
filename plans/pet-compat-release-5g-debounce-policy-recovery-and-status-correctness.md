# Release 5G: Debounce, Policy, Recovery, and Status Correctness

## Purpose

Correct the remaining behavioral mismatches around debounce, policy loading, stale pending recovery, failure modes, retries, and durable operator visibility.

After Release 5E and 5F establish safe state transactions and singular sync execution, this phase ensures the one-shot worker acts on the latest generation, respects independent policy settings, never clears work because configuration loading failed, and provides truthful durable status.

## Required Outcomes

1. A burst of mutations produces one sync after the final quiet period.
2. The worker synchronizes the latest observed pending generation, not the generation captured before debounce.
3. Marker removal during debounce cancels the pending worker cycle rather than triggering a stale sync.
4. Sync timeout is independent from debounce duration.
5. Configuration parse, integrity, keychain, or I/O failures preserve pending work.
6. Explicitly disabled auto-sync is distinguishable from policy-load failure.
7. Old pending work remains recoverable and is rescheduled rather than becoming permanently dormant.
8. Failure modes have a truthful detached-worker contract.
9. Retry policy is either implemented and bounded or removed from the effective policy surface.
10. Durable status reports last attempt, success, failure class, deferral, and pending generation without secrets.
11. Doctor and sync status output agree with on-disk state.
12. Documentation no longer promises nonzero mutation exit codes for asynchronous failures.

## Workstream A: Latest-State Debounce Result

Replace the current `wait_for_quiet` return type with a state-bearing result.

Recommended model:

```rust
pub enum DebounceResult {
    Ready(PendingState),
    CancelledMarkerRemoved,
    LifetimeExceeded(PendingState),
}
```

Behavior:

1. Read initial pending state.
2. Compute deadline from its request timestamp.
3. Sleep in bounded intervals.
4. Reload marker after every interval.
5. If marker is removed, return `CancelledMarkerRemoved` and do not sync.
6. If generation or timestamp changes, replace the observed state and recompute the deadline.
7. On quiet-period expiry, return the latest complete `PendingState`.
8. The worker syncs that returned generation and snapshot.

A generation arriving during debounce must not cause an unnecessary first sync for the stale generation.

## Workstream B: Worker Lifetime Budget

Separate these durations:

- debounce window;
- sync timeout;
- termination grace period;
- retry delay;
- overall worker lifetime.

Define worker lifetime from the worst valid configuration, for example:

```text
max_worker_lifetime = max_debounce
                    + sync_timeout
                    + termination_grace
                    + optional_retry_budget
                    + safety_margin
```

Do not set maximum worker lifetime equal to maximum debounce.

If the lifetime budget is reached before sync starts:

- preserve pending;
- record `deferred_lifetime_budget`;
- exit without syncing;
- allow later recovery to reschedule.

## Workstream C: Independent Sync Timeout Configuration

Stop deriving `sync_timeout` from `auto_sync_debounce_seconds`.

Choose one of these compatible approaches:

1. Add a persisted `auto_sync_timeout_seconds` setting with default 30 and bounded range 5–120.
2. Keep timeout internal and fixed at 30 seconds for Release 5, exposing configuration later.

Prefer option 1 only if configuration surface expansion is justified. Otherwise use the constant and remove misleading fields.

Required tests:

- debounce 0 still uses default sync timeout;
- debounce 2 still uses default sync timeout;
- debounce 300 does not change sync timeout;
- timeout clamps independently if made configurable.

## Workstream D: Typed Policy Loading

Replace `get_sync_settings() -> SyncSettings` fallback-to-default behavior in the worker path with a typed result:

```rust
pub enum PolicyLoadOutcome {
    Loaded(AutoSyncPolicy),
    ExplicitlyDisabled,
    Failed(PolicyLoadError),
}
```

Rules:

- missing config may resolve to explicitly disabled;
- valid config with `auto_sync = false` is explicitly disabled;
- malformed TOML is failure;
- integrity mismatch is failure;
- keychain retrieval failure is failure unless an explicitly supported credential mode succeeds;
- filesystem read failure is failure.

On `Failed`:

- preserve pending;
- record durable failure class;
- do not clear the marker;
- do not silently replace policy with defaults.

On `ExplicitlyDisabled`:

- do not perform sync;
- decide explicitly whether to retain or clear pending created under a previously enabled configuration;
- recommended behavior: retain with status `disabled_with_pending` until user runs manual sync, reenables auto-sync, or explicitly clears it.

## Workstream E: Stale Pending Recovery

Remove the current behavior where pending work older than five minutes is only logged and never scheduled.

Recovery rules:

- marker age does not make work invalid;
- valid old pending state should be rescheduled when no execution worker is active;
- malformed markers should be preserved/renamed for diagnostics, not deleted silently;
- repeated recovery attempts must not increment generation;
- use durable next-attempt/backoff metadata to avoid spawning a worker on every read-only CLI invocation during a persistent outage.

Suggested status fields:

```toml
last_attempt_at_unix_ms = 0
next_attempt_at_unix_ms = 0
attempt_count = 0
last_result = "pending"
last_failure_class = ""
```

Keep status in a separate bounded file if modifying the pending schema complicates transaction semantics.

## Workstream F: Detached Failure-Mode Contract

Redefine `AutoSyncFailureMode` for asynchronous operation.

Recommended semantics:

### ignore

- persist bounded status;
- no parent stderr message for remote failure;
- scheduling failures may remain silent unless diagnostics are requested.

### warn

- persist bounded status;
- parent may warn only for immediate scheduling/state-recording failure;
- later remote failure appears in `snp sync status` and doctor output;
- no retroactive parent exit change.

### error

- persist an attention-required state;
- immediate scheduling/state-recording failure emits a clear post-commit error message;
- later remote failure is surfaced prominently by status/doctor and possibly the next interactive invocation;
- original mutation still exits according to local commit success.

Remove documentation claiming a detached remote failure causes the already-completed mutation command to return nonzero.

Foreground `snp sync` retains normal nonzero exit behavior.

## Workstream G: Retry and Backoff

`max_retries` is currently populated but unused.

Choose one policy:

### Minimal recommended policy

- one sync attempt per worker lifecycle;
- preserve pending on failure;
- durable exponential backoff determines next recovery spawn;
- no in-process retry loop.

This keeps the helper lightweight and avoids a long-lived process.

Suggested bounds:

- initial retry: 5 seconds;
- exponential growth with jitter;
- maximum: 15 minutes;
- authentication/configuration failures do not retry rapidly;
- conflict/local-persistence failures require user attention.

Remove `max_retries` if this model is selected.

## Workstream H: Durable Status

Add a private bounded status artifact, for example:

```text
auto-sync-status.toml
```

Fields may include:

- schema;
- pending generation;
- last attempt timestamp;
- last success timestamp;
- attempt count;
- next attempt timestamp;
- result code;
- failure class;
- executor exit code;
- worker PID only while useful;
- integrity checksum.

Do not include:

- snippet command or description;
- tags/output;
- API key;
- server credential;
- raw upstream response bodies;
- full URLs containing secrets.

Use atomic private writes and bounded field lengths.

## Workstream I: Status and Doctor UX

Add or complete:

```text
snp sync status
snp doctor --compatibility
```

Output should distinguish:

- auto-sync disabled;
- enabled and idle;
- pending debounce;
- deferred because execution lock busy;
- retry backoff active;
- last sync succeeded;
- last sync failed: network/auth/conflict/config/local;
- malformed marker/status artifact;
- pending work older than expected;
- worker/executor currently active.

JSON output should use stable field names if supported.

## Tests

### Debounce tests

- 20 rapid mutations result in one actual recording-server sync attempt;
- latest generation is the generation cleared on success;
- mutation during debounce resets the quiet deadline;
- marker removal during debounce causes zero sync attempts;
- mutation during active sync creates one follow-up cycle after quiet period.

### Policy tests

- malformed config preserves pending;
- integrity mismatch preserves pending;
- keychain failure preserves pending;
- explicitly disabled policy does not masquerade as load failure;
- debounce and timeout remain independent.

### Recovery tests

- pending marker older than five minutes is rescheduled;
- recovery does not increment generation;
- backoff prevents repeated spawn storms;
- successful later sync clears matching pending state and resets backoff;
- newer generation resets or updates retry scheduling appropriately.

### Failure-mode tests

- ignore/warn/error produce distinct durable status and immediate scheduling UX;
- none claim retroactive mutation exit failure;
- foreground sync still returns nonzero on real failure.

### Security tests

- status contains no sentinel secrets or snippet payload;
- status size remains bounded under repeated failures;
- permissions are restrictive;
- corruption is detected and reported.

## Documentation

Update configuration reference, architecture docs, doctor docs, and examples. Include a clear operator recovery section:

- inspect status;
- run manual sync;
- reenable auto-sync;
- clear/repair malformed state through an explicit command;
- understand backoff timing.

## Recommended Commit Sequence

1. Refactor debounce to return latest pending state/cancellation.
2. Separate all duration budgets and fix timeout resolution.
3. Introduce typed policy loading.
4. Add durable status and backoff.
5. Correct stale pending recovery.
6. Reconcile failure-mode behavior and CLI status UX.
7. Add deterministic recording-server tests.
8. Update documentation.

## Exit Criteria

- one burst produces exactly one sync attempt;
- marker removal during debounce produces no stale sync;
- policy-load failures never clear pending work;
- old pending work remains recoverable;
- timeout is independent from debounce;
- failure modes are truthful and documented;
- status is durable, bounded, and secret-free;
- all unit, integration, subprocess, and platform tests pass.

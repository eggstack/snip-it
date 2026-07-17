# Phase 02: Pending-State and Debounce Semantics

## Purpose

Make pending synchronization intent durable, explicit, generation-safe, and efficient under rapid mutation bursts.

Phase 01 establishes truthful sync execution. This phase corrects the state-machine behavior around disabled auto-sync, debounce promotion, marker removal, maximum delay, startup recovery, and follow-up work arriving during an active synchronization.

## Preconditions

Do not begin semantic closure until Phase 01 has established:

- a real canonical sync operation;
- truthful executor success;
- worker-owned execution locking;
- generation-safe clear only on real success;
- failure preservation.

The test-harness scaffolding for Phase 05 may be used, but Phase 02 must define the behavior that later integration tests prove.

## Core invariants

1. Every committed sync-relevant local mutation increments pending generation exactly once.
2. Recording pending intent and scheduling a worker are separate operations.
3. Scheduling existing work never mutates generation or rewrites the marker.
4. A newer generation supersedes an older observed generation.
5. Debounce tracks the newest complete pending state, not the first state seen by a worker.
6. Marker removal before execution cancels the stale cycle.
7. A failed or deferred cycle preserves pending state.
8. Pending age alone never makes work invalid.
9. Disabled automatic execution does not silently erase synchronization intent.
10. A foreground successful sync may conditionally clear pending state without racing a detached worker.
11. Mutation during active sync remains pending for a later cycle unless the completed sync demonstrably covered and conditionally clears the newer generation under a documented rule.

## Workstream A: Separate sync enablement from auto-sync execution

Audit the current settings model. Avoid overloading one `enabled` field with all of the following meanings:

- synchronization account is configured;
- manual sync is allowed;
- detached auto-sync is enabled;
- pending intent should be tracked.

Recommended semantics:

```toml
sync_enabled = true
auto_sync_enabled = false
```

Compatibility rules:

- existing configurations must parse with stable defaults;
- if only the legacy field exists, preserve its historical intent;
- manual `snp sync` should remain available when sync is configured even if automatic execution is disabled;
- local mutations while auto-sync is disabled should preserve pending intent when sync is configured;
- re-enabling auto-sync or running manual sync should recover accumulated work;
- unconfigured installations should not create meaningless pending artifacts unless the product intentionally tracks future sync intent.

Document the exact distinction. Do not silently clear pending merely because automatic scheduling is disabled.

## Workstream B: Define the pending schema contract

Review `PendingState` and document behavior-driving fields. The schema should minimally identify:

- schema version;
- monotonic generation;
- mutation/request timestamp;
- snapshot or mutation kind if still behaviorally required;
- integrity/checksum covering behavior-driving fields.

Do not add operator status, retry history, or arbitrary error strings to the pending marker unless they are transactionally necessary. Phase 03 should use a separate status artifact for attempt metadata.

Required properties:

- private permissions;
- bounded file size;
- atomic replacement;
- unique temp files;
- corruption detection;
- backward-compatible migration;
- no secrets or snippet payloads;
- no generation reset during ordinary recovery.

Define overflow behavior explicitly. A checked overflow should fail safely and preserve the previous marker rather than wrapping to zero. A future schema migration may use a wider logical clock if required.

## Workstream C: Refactor debounce to return the latest state

Replace a wait function that merely returns `()` with a state-bearing result.

Recommended model:

```rust
pub enum DebounceResult {
    Ready(PendingState),
    CancelledMarkerRemoved,
    DeferredMaximumLifetime(PendingState),
    Failed(PendingReadError),
}
```

Required loop:

1. Read the initial complete pending state.
2. Set it as `observed`.
3. Compute the quiet deadline from `observed.created_at` and configured debounce.
4. Wait in bounded intervals or through an injected clock/sleeper.
5. Reload the marker after each wake.
6. If marker is absent, return `CancelledMarkerRemoved`.
7. If generation or relevant timestamp changed, replace `observed` with the new state and recompute deadline.
8. When the quiet period expires, perform one final reload.
9. Return `Ready(latest_state)` only if the marker still matches the latest observation.
10. If it changed during the final boundary, restart the quiet calculation.

The worker must synchronize and conditionally clear the generation returned by the debounce result, not the initial generation.

## Workstream D: Recheck immediately before executor spawn

Even after debounce returns `Ready`, the worker should perform a final generation-safe preflight under the execution lock:

- marker absent: exit without sync;
- marker changed: return to debounce using the newer state;
- marker corrupt/unreadable: preserve artifact, record failure, do not sync;
- marker matches: spawn executor.

This closes the race where a foreground sync clears pending after the last debounce poll but before the detached executor starts.

## Workstream E: Separate quiet period from maximum delay

Model these as separate policy concepts:

```toml
auto_sync_debounce_seconds = 2
auto_sync_max_delay_seconds = 300
```

Semantics:

- debounce means no new mutation for the configured duration;
- maximum delay means force an attempt after bounded elapsed time even if changes continue;
- maximum delay is not a successful quiet period;
- if maximum delay is reached, the worker should synchronize the latest observed state and document this as bounded-latency behavior;
- if the implementation chooses to defer instead of force, preserve pending and record a distinct status. Choose one contract and test it.

Preferred product behavior: force one attempt at maximum delay to prevent indefinite starvation, then preserve any generation arriving during the attempt for a follow-up cycle.

Ensure worker lifetime includes:

- maximum debounce/max-delay budget;
- sync timeout;
- termination grace;
- bounded follow-up handling;
- safety margin.

Do not clamp debounce to the same value as worker lifetime and then accidentally describe that as a quiet period.

## Workstream F: Mutation during active sync

Define the exact rule.

Recommended conservative rule:

1. Worker observes generation `N` and launches sync.
2. Generation `N+1` arrives while sync is active.
3. On successful sync, worker conditionally clears only `N`.
4. Conditional clear reports generation changed and preserves `N+1`.
5. Worker begins a new debounce cycle for the latest marker.
6. Follow-up sync occurs once after the new quiet period.

Do not assume the first sync covered `N+1` merely because it read current local files at some unspecified point. Such an optimization would require a synchronization snapshot/revision contract proving coverage.

## Workstream G: Startup and stale recovery

Recovery rules:

- startup recovery is read-only with respect to generation;
- valid pending work is recoverable regardless of age;
- recent and old pending markers may be scheduled subject to durable backoff from Phase 03;
- active execution lock or worker ownership should defer duplicate scheduling;
- malformed pending artifacts should not be deleted silently;
- corrupt artifacts should be reported by status/doctor and preserved or quarantined through an explicit safe mechanism;
- read-only commands should not spawn a worker repeatedly during a persistent outage once backoff exists;
- internal worker/executor subcommands must not recursively run startup recovery.

Remove any behavior that treats an old marker as stale in the sense of disposable work.

## Workstream H: Explicit pending discard

If a user needs to abandon synchronization intent, provide or reserve an explicit operation rather than reusing disabled policy.

Required safety properties:

- confirmation unless `--force`;
- display the generation to be discarded;
- conditional clear against the observed generation;
- if generation changes during confirmation, refuse and require retry;
- never delete local snippet data;
- status records deliberate discard without sensitive content;
- command is documented as advanced recovery.

Implementation may land in Phase 04, but Phase 02 must expose the generation-safe primitive and semantics.

## Workstream I: Deterministic timing abstraction

Avoid tests that wait real multi-second debounce windows.

Introduce a narrow internal abstraction where practical:

```rust
trait Clock {
    fn now_instant(&self) -> Instant;
    fn now_unix_ms(&self) -> u64;
    fn sleep(&self, duration: Duration);
}
```

Production uses the system clock. Tests use a fake or controlled clock.

If process-boundary tests require real time, use very small bounded durations and a recording/barrier server, but keep state-machine unit tests deterministic.

Be careful with wall-clock versus monotonic time:

- persisted timestamps require wall-clock representation;
- in-process deadlines should use monotonic `Instant`;
- wall-clock jumps must not create huge sleeps or underflow;
- conversion should saturate safely;
- tests should cover timestamps in the past, future skew, and maximum values.

## Required tests

### Generation transaction tests

- concurrent writers increment exactly once each;
- conditional clear cannot remove a newer generation;
- scheduling existing work is byte-for-byte read-only;
- startup recovery never increments generation;
- failed write preserves prior valid marker;
- overflow fails safely;
- v1/v2 migration remains idempotent.

### Debounce tests

- one mutation produces one ready state;
- debounce zero is immediate but still performs final preflight;
- 20 rapid mutations produce one `Ready` for the final generation;
- quiet deadline resets from the final mutation;
- marker removal produces cancellation and zero executor starts;
- marker replacement with newer generation promotes observation;
- corruption during debounce preserves state and reports failure;
- maximum delay behavior follows the selected documented contract;
- wall-clock skew does not panic or sleep unboundedly.

### Active-sync tests

- mutation during active sync remains pending;
- successful older cycle cannot clear newer generation;
- exactly one follow-up cycle begins after quiet period;
- failed first cycle preserves newest generation without duplicate increments;
- foreground successful sync clearing marker cancels stale detached preflight.

### Policy tests

- sync configured + auto-sync disabled preserves pending;
- re-enable schedules existing pending;
- manual sync works while auto-sync disabled;
- unconfigured state follows documented pending-creation policy;
- malformed settings are failure, not explicit disable.

## Documentation

Update:

- settings reference;
- auto-sync policy section;
- architecture diagrams;
- recovery behavior;
- worker lifetime explanation;
- manual versus automatic sync distinction;
- pending discard semantics if exposed;
- comments in pending, notification, worker, and policy modules.

Avoid release-number archaeology in production docs. Describe invariants and current behavior.

## Recommended commit sequence

1. Codify pending invariants and add failing state-machine tests.
2. Separate sync configuration from auto-sync execution policy.
3. Refactor debounce to return latest state/cancellation.
4. Add final pre-executor marker check.
5. Separate debounce, maximum delay, timeout, and lifetime budgets.
6. Correct active-sync follow-up semantics.
7. Correct old pending recovery and disabled behavior.
8. Add deterministic clock tests and process-level verification.
9. Update docs and remove obsolete branches/comments.

## Verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run focused stress tests repeatedly and ensure no permissive assertions such as “marker may or may not exist” remain for defined behavior.

## Exit criteria

Phase 02 is complete only when:

- disabled automatic execution does not silently discard pending intent;
- debounce returns and syncs the latest generation;
- mutation bursts normally produce exactly one sync attempt;
- marker removal before execution produces zero stale attempts;
- maximum delay and quiet period are distinct and documented;
- mutation during active sync survives for one later cycle;
- old valid pending work remains recoverable;
- recovery never increments generation;
- timing tests are deterministic;
- all platform tests pass;
- docs match the final state machine.

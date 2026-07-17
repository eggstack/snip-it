# Phase 03: Failure Classification and Retry Policy

## Purpose

Convert synchronization failures from generic process outcomes into typed, durable, and operationally meaningful state. Define bounded retry behavior that remains lightweight, process-independent, and resistant to worker storms.

Phase 01 makes executor success truthful. Phase 02 makes pending intent durable and generation-safe. This phase determines what happens after each non-success result.

## Preconditions

Required before entry:

- executor invokes the real canonical sync operation;
- pending clears only on real success;
- worker and executor lock ownership is singular;
- latest-generation debounce semantics are stable;
- disabled auto-sync is distinguishable from policy/configuration failure;
- old pending work remains recoverable.

## Goals

1. Preserve specific failure information from canonical sync through executor and worker boundaries.
2. Persist bounded, secret-free status independently of pending intent.
3. Retry transient failures conservatively using durable backoff.
4. Avoid tight loops and duplicate worker processes.
5. Make authentication, conflict, and local persistence failures attention-requiring rather than aggressively retried.
6. Make the behavior testable without live internet dependencies.

## Non-goals

- long-running retry daemon;
- in-process retry loops that keep a worker alive indefinitely;
- user-configurable policy for every error subtype;
- storing raw upstream responses;
- changing synchronization merge semantics;
- notification services or desktop alerts.

## Workstream A: Define a typed internal error taxonomy

Create a canonical internal error model at the sync-operation boundary.

Recommended initial shape:

```rust
#[derive(Debug, thiserror::Error)]
pub enum SyncExecutionError {
    #[error("sync is not configured")]
    NotConfigured,

    #[error("sync is disabled")]
    Disabled,

    #[error("authentication failed")]
    Authentication,

    #[error("network connection failed")]
    Network,

    #[error("sync timed out")]
    Timeout,

    #[error("remote protocol error")]
    RemoteProtocol,

    #[error("synchronization conflict")]
    Conflict,

    #[error("synchronization completed partially")]
    Partial,

    #[error("local persistence failed")]
    LocalPersistence,

    #[error("configuration could not be loaded")]
    Configuration,

    #[error("credential storage failed")]
    CredentialStore,

    #[error("internal synchronization error")]
    Internal,
}
```

The exact variant count may differ, but the taxonomy must distinguish at least:

- transient connectivity;
- timeout;
- authentication/credential;
- configuration;
- conflict/partial;
- local durability;
- internal/unknown;
- deliberately disabled or unconfigured.

Do not classify errors solely by display-string matching. Map concrete error variants or gRPC/transport status codes at the boundary where their meaning is known.

## Workstream B: Define failure classes and retry disposition

Add a stable policy-facing classification separate from low-level errors.

```rust
pub enum FailureClass {
    DeferredDisabled,
    DeferredNotConfigured,
    TransientNetwork,
    TransientTimeout,
    Authentication,
    Configuration,
    Conflict,
    Partial,
    LocalPersistence,
    Internal,
}

pub enum RetryDisposition {
    RetryAfter(Duration),
    WaitForConfigurationChange,
    RequiresAttention,
    NoAutomaticRetry,
}
```

Mapping rules should be explicit and covered by table-driven tests.

Recommended defaults:

| Failure class | Pending | Automatic retry | Operator state |
| --- | --- | --- | --- |
| Disabled | Preserve | No, until enabled | Deferred |
| Not configured | Preserve if intent exists | No, until configured | Deferred |
| Network | Preserve | Exponential backoff | Pending/retrying |
| Timeout | Preserve | Exponential backoff | Pending/retrying |
| Authentication | Preserve | No rapid retry | Attention required |
| Configuration | Preserve | No rapid retry | Attention required |
| Credential store | Preserve | No rapid retry | Attention required |
| Conflict | Preserve | Conservative/manual | Attention required |
| Partial | Preserve | Conservative/manual | Attention required |
| Local persistence | Preserve | No rapid retry | Critical local failure |
| Internal | Preserve | Small bounded retry budget | Attention after budget |

## Workstream C: Add a separate durable status artifact

Create a bounded private file, for example:

```text
auto-sync-status.toml
```

Do not merge this into the pending marker unless transactional analysis proves it is necessary. Pending intent and attempt telemetry have different lifecycles.

Suggested schema:

```toml
schema = 1
pending_generation = 42
last_attempt_generation = 42
last_attempt_at_unix_ms = 0
last_success_at_unix_ms = 0
last_result = "network_failure"
last_failure_class = "transient_network"
consecutive_failures = 3
next_attempt_at_unix_ms = 0
executor_exit_code = 4
attention_required = false
message = "connection failed"
integrity = 0
```

Requirements:

- atomic private writes;
- ownership/permissions equivalent to pending state;
- bounded strings and file size;
- integrity over behavior-driving fields;
- forward-compatible schema handling;
- corruption reported, not silently treated as success;
- no command text, descriptions, tags, outputs, API keys, encryption keys, credential values, raw response bodies, or secret-bearing URLs;
- safe redaction before message persistence;
- status write failure must not clear pending.

Status is informative and may influence scheduling through `next_attempt_at_unix_ms`, but it must not become the source of truth for whether pending work exists.

## Workstream D: Implement durable exponential backoff

Use one attempt per detached worker lifecycle. Do not keep the helper alive for repeated network retries.

Recommended transient schedule:

- first retry: approximately 5 seconds;
- then 15 seconds;
- then 30 seconds;
- then 1 minute;
- then exponential growth;
- cap at 15 minutes or another documented conservative maximum;
- include bounded jitter to avoid synchronized retries across machines;
- persist the next eligible attempt time.

Backoff calculation must:

- saturate safely;
- be deterministic under injected RNG/clock in tests;
- never overflow timestamps;
- reset after true success;
- not reset merely because a new CLI process starts;
- define whether a new mutation shortens backoff.

Recommended rule for new mutations:

- preserve failure count and next attempt for persistent network outages;
- allow a small bounded acceleration when a materially newer generation arrives, but never spawn per mutation;
- configuration/credential changes may clear deferred disposition and permit immediate retry;
- explicit `snp sync --retry` may bypass the wait for a foreground attempt without corrupting stored backoff.

## Workstream E: Prevent worker storms

Create a single scheduling decision function that considers:

- whether pending exists;
- whether automatic execution is enabled;
- whether an execution lock is active;
- whether a live worker ownership lock exists, if retained;
- whether `next_attempt_at` is in the future;
- whether the failure class allows automatic retry;
- whether the caller is startup recovery, mutation scheduling, or explicit retry.

Recommended result:

```rust
pub enum ScheduleDecision {
    SpawnNow,
    AlreadyActive,
    DeferredUntil(u64),
    Disabled,
    RequiresAttention(FailureClass),
    NoPending,
}
```

Only `SpawnNow` should invoke the process spawner.

Avoid using lock-file age alone to steal a live worker. Keep existing PID/nonce ownership checks and platform-native liveness semantics.

## Workstream F: Preserve failure classification across process boundary

The executor’s numeric exit codes are an internal transport encoding, not the canonical model.

Required mapping:

```text
SyncExecutionError
  -> ExecutorExitCode
  -> worker FailureClass
  -> durable status + retry disposition
```

The mapping must be total. Unknown exit codes or signal death map to `Internal` or a distinct `ExecutorTerminated` class, preserve pending, and never clear status as success.

Capture safely:

- numeric exit code;
- whether process exited normally or by signal where available;
- timeout termination;
- spawn failure;
- wait/reap failure.

Do not persist unbounded child stderr. Structured executor logging should remain separate and sanitized.

## Workstream G: Define success status transitions

On true success or already-current result:

- conditionally clear matching pending generation;
- record last success timestamp;
- reset consecutive failure count;
- clear next attempt;
- clear attention-required state;
- store a bounded success result;
- retain enough previous failure context only if useful and explicitly designed.

If conditional clear reports a newer generation:

- record success for the attempted generation;
- preserve pending status for the newer generation;
- schedule/debounce the follow-up according to Phase 02;
- do not mark the entire installation current.

## Workstream H: Foreground versus detached semantics

Foreground `snp sync`:

- returns nonzero on failure;
- renders a specific safe error;
- may bypass automatic backoff because it is explicit user intent;
- updates durable status consistently;
- conditionally clears pending only on success;
- must still respect execution mutual exclusion.

Detached sync:

- cannot change the already-completed mutation exit status;
- writes durable status;
- follows retry disposition;
- normally does not emit terminal output;
- preserves local mutation success.

Immediate scheduling/state-recording failure during a mutation may be surfaced according to existing failure-mode policy, but documentation must not imply that later network failure retroactively changes the parent outcome.

## Workstream I: Configuration-change detection

Authentication, configuration, and credential failures should become eligible again when relevant inputs change.

Use a safe configuration fingerprint containing only non-secret structural inputs, for example:

- normalized server origin hash;
- sync enabled/auto-sync enabled;
- direction;
- credential presence/version token, not credential value;
- relevant schema version.

On changed fingerprint:

- clear `WaitForConfigurationChange` deferral;
- permit a new attempt;
- retain pending generation;
- avoid storing raw sensitive values.

If credential-store APIs expose no stable version token, explicit register/config commands can clear the deferral after successful writes.

## Required tests

### Classification tests

- every canonical error maps to one failure class;
- every failure class maps to one retry disposition;
- executor exit-code mapping is total;
- unknown code and signal death preserve pending;
- display messages are bounded and sanitized.

### Status tests

- atomic round trip;
- permissions;
- integrity corruption detection;
- migration/unknown field behavior;
- no sentinel command/API-key/server-secret leakage;
- bounded size after repeated failures;
- status write failure preserves pending;
- newer generation is represented correctly after older success.

### Backoff tests

- exact progression under fake clock/RNG;
- jitter bounds;
- cap behavior;
- overflow saturation;
- reset after success;
- persistence across separate CLI processes;
- no reset on startup recovery;
- no worker spawn before next eligible time;
- configuration change releases appropriate deferral.

### Worker-storm tests

- 20 mutations during backoff spawn at most the documented number of workers;
- repeated read-only startup recovery does not spawn repeatedly;
- execution lock busy causes one durable deferral rather than process churn;
- live worker is not stolen by age;
- dead worker is recoverable.

### Foreground/detached parity tests

- both classify the same injected server error identically;
- foreground exits nonzero;
- detached records status and exits without clearing pending;
- explicit retry bypasses wait but does not reset state incorrectly.

## Documentation

Update:

- sync failure behavior;
- retry/backoff policy;
- distinction between pending intent and attempt status;
- foreground versus detached semantics;
- authentication/configuration recovery steps;
- status schema only if considered a supported diagnostic contract;
- privacy/redaction guarantees;
- command help for explicit retry controls added in Phase 04.

## Recommended commit sequence

1. Introduce typed canonical sync errors and table-driven classification tests.
2. Add durable status schema and private atomic persistence.
3. Preserve executor detail through worker mapping.
4. Implement retry disposition and deterministic backoff calculator.
5. Add centralized schedule-decision function.
6. Integrate mutation, startup recovery, and foreground paths.
7. Add configuration-change release behavior.
8. Add storm, persistence, and fault-injection tests.
9. Reconcile docs and remove generic `unknown` paths.

## Exit criteria

Phase 03 is complete only when:

- every major failure has a typed class;
- generic `unknown` is used only for genuinely unclassified internal cases;
- pending intent survives every non-success class;
- status is durable, bounded, private, and secret-free;
- transient retries use persisted bounded backoff;
- authentication/configuration/persistence failures do not tight-loop;
- repeated mutations and startup recovery cannot create worker storms;
- foreground and detached paths classify the same failure consistently;
- success resets retry state correctly without hiding newer pending work;
- deterministic tests cover progression and platform behavior;
- documentation states truthful asynchronous semantics.

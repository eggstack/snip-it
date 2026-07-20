# snip-it Correctness Program 03A: Phase 01–03 Corrective Closure

## Purpose

Close the correctness gaps remaining after implementation of:

- Phase 01 — auto-sync correctness closure;
- Phase 02 — pending-state and debounce semantics;
- Phase 03 — failure classification, durable status, retry, and worker-storm prevention.

The implementation now contains the correct high-level architecture: local mutations commit before remote work, a detached one-shot worker owns the shared sync execution lock, a killable executor subprocess performs the canonical sync operation, and pending state is cleared only after success. However, several production paths do not yet honor the intended policy, retry, status, timing, and cross-platform invariants.

This is a blocking closure pass. Do not begin Phase 04 operational UX or later roadmap phases until every exit criterion in this document is satisfied.

## Current Baseline

Use the repository state at or after commit:

```text
c5a5b8f843cc9d1688f1032dab99880f600d053f
```

The implementation under review includes:

- `src/auto_sync/executor.rs` — real canonical sync execution and process exit mapping;
- `src/auto_sync/worker.rs` — debounce, executor supervision, conditional clear, and startup recovery;
- `src/auto_sync/policy.rs` — resolved policy, failure classes, and retry dispositions;
- `src/auto_sync/status.rs` — durable status and configuration fingerprint;
- `src/auto_sync/schedule.rs` — centralized scheduling decision;
- `src/auto_sync/notification.rs` — mutation notification and worker scheduling;
- `src/auto_sync/execution_lock.rs` — cross-entry-point sync exclusion;
- `tests/auto_sync_closure.rs` — current closure test set.

## Blocking Findings

The closure pass must address all of the following:

1. `sync_timeout` is incorrectly derived from `auto_sync_debounce_seconds`.
2. `schedule_sync()` is not the real production scheduling authority; mutation and startup paths bypass it.
3. Windows process liveness always returns true, making crashed execution locks permanent.
4. malformed, corrupt, unreadable, or credential-failing sync configuration can be converted to default-disabled policy and cause new local mutations to omit pending intent.
5. preflight detection of a newer generation does not restart the quiet period.
6. maximum-delay logic bypasses the injected clock and policy snapshots are reloaded inconsistently during one worker lifecycle.
7. detached status writes use generation `0` instead of the attempted generation.
8. executor exit codes collapse distinct failure classes and the worker reconstructs incorrect classes.
9. internal retry limits are not enforced by the scheduler.
10. the first effective backoff interval is off by one relative to the documented schedule.
11. the durable status integrity implementation is described as CRC32 but uses `DefaultHasher`, omits behavior-driving fields, and is unsuitable as a persisted format.
12. the headline end-to-end test does not prove a server-observable sync effect and contains permissive branches that can pass without establishing the claimed invariant.
13. documentation advertises CLI configuration not exposed by the actual command surface.
14. corrupted pending or status state is frequently treated as absence rather than a distinct actionable failure.
15. API-key replacement may not release an authentication deferral because the current fingerprint includes only key presence, not a safe credential version signal.

## Closure Invariants

The implementation is complete only when all invariants below are directly represented in code and tests.

### Local-first invariants

1. A successful local mutation is never rolled back because synchronization fails.
2. Every committed sync-relevant local mutation records exactly one pending generation whenever sync has been configured, even if auto-sync is disabled or policy loading fails.
3. Sync-origin writes do not recursively create pending generations.
4. A pending generation is cleared only after a real successful sync comparison that covered that generation.
5. Failure to write status can never clear or mutate pending intent.

### Process and lock invariants

6. Exactly one sync operation runs at a time across detached, manual, cron, and explicit `--sync` entry points.
7. The detached worker is the sole execution-lock owner for its cycle; the executor never reacquires the lock.
8. The execution lock remains held until the executor has exited and been reaped.
9. A dead lock owner is reclaimable on Linux, macOS, and Windows.
10. A live owner is never stolen based only on lock age.
11. An old guard cannot remove a newer owner's lock.

### Timing invariants

12. Debounce, maximum delay, executor timeout, termination grace, retry delay, and total worker lifetime are independent concepts.
13. Default executor timeout is not derived from debounce and is long enough for ordinary remote sync.
14. A newer generation observed before executor spawn receives a fresh quiet period unless maximum delay has already forced execution.
15. All time-dependent unit tests use one injected clock abstraction; no production timing branch bypasses it.
16. One worker lifecycle uses one immutable resolved policy unless dynamic reconfiguration is explicitly designed and tested.

### Scheduling and retry invariants

17. Every automatic worker-spawn request passes through one scheduling decision function.
18. Backoff, attention-required state, configuration deferral, pending validity, and active execution locks are enforced before spawning.
19. Rapid mutations during backoff update pending generation without spawning one worker per mutation.
20. Explicit foreground retry may bypass backoff but cannot bypass execution exclusion or pending correctness.
21. Internal failures are automatically retried only within the documented bounded budget.
22. Retry attempt numbering and delay calculation match documented values exactly.

### Result and status invariants

23. The executor's typed failure classification survives the process boundary without lossy reconstruction.
24. Detached and foreground attempts record the actual attempted generation where one exists.
25. Status differentiates success, already-current success, transient failure, timeout, authentication, configuration, credential-store, conflict, partial, persistence, executor termination, and internal failure.
26. Status integrity covers every behavior-driving field.
27. Corrupt status does not silently become a clean default that changes scheduling behavior.
28. Status and control files contain no API key, encryption key, snippet command, snippet description, or raw upstream response body.

### Evidence invariants

29. The real-binary end-to-end test proves a server-observable sync request or state transition before pending is accepted as cleared.
30. A fake success executor that does not contact the server would fail the closure test.
31. Negative-path tests prove server failure preserves pending.
32. Tests requiring exactly one attempt do not accept `>= 1`, status-file existence alone, or mutually contradictory outcomes.

---

# Workstream A — Decouple Executor Timeout from Debounce

## Required implementation

Remove this effective behavior:

```rust
sync_timeout = Duration::from_secs(
    settings.auto_sync_debounce_seconds.clamp(1, MAX_SYNC_TIMEOUT_SECS)
)
```

Adopt one of these designs:

### Preferred design

Add a persisted setting:

```rust
pub auto_sync_timeout_seconds: Option<u64>
```

Resolution rules:

- default: 30 seconds;
- minimum: 5 seconds;
- maximum: 120 seconds;
- missing field migrates to default without rewriting unrelated settings;
- debounce zero remains valid and does not lower timeout;
- maximum delay does not alter timeout.

Expose it through the supported configuration command:

```bash
snp sync config --timeout 30
```

### Acceptable minimal design

Use a fixed internal 30-second executor timeout for this closure release and do not expose a configuration field yet.

If this minimal design is selected, remove any documentation or dead configuration surface implying user configurability.

## Tests

Add exact policy tests:

- debounce 0 resolves timeout 30 seconds;
- debounce 1 resolves timeout 30 seconds;
- debounce 120 resolves timeout 30 seconds;
- debounce 300 resolves timeout 30 seconds;
- configured timeout 5 resolves 5 seconds;
- configured timeout 120 resolves 120 seconds;
- values outside bounds are rejected or clamped according to documented behavior;
- existing config without timeout migrates safely.

Add a worker integration test where debounce is zero and the controlled executor runs longer than two seconds but shorter than the configured timeout. It must complete rather than being killed.

## Acceptance criteria

- no timeout code reads `auto_sync_debounce_seconds`;
- architecture and CLI docs describe the exact implemented timeout source;
- the default sync timeout is 30 seconds or another explicitly justified independent value.

---

# Workstream B — Make `schedule_sync()` the Only Automatic Spawn Authority

## Required implementation

All automatic worker scheduling must flow through `src/auto_sync/schedule.rs`.

Replace direct automatic calls to:

```rust
worker::schedule_existing_pending(state_dir)
spawn::spawn_worker(state_dir)
```

from:

- mutation notification;
- startup recovery;
- account/config recovery hooks;
- any read-only startup hook that attempts recovery;
- any future retry trigger.

Recommended API shape:

```rust
pub struct ScheduleRequest<'a> {
    pub state_dir: &'a Path,
    pub policy: &'a AutoSyncPolicy,
    pub caller: Caller,
}

pub enum ScheduleDecision {
    SpawnNow,
    AlreadyActive,
    DeferredUntil(u64),
    DisabledWithPending,
    RequiresAttention(FailureClass),
    NoPending,
    NotConfigured,
    CorruptPending(PendingError),
    CorruptStatus(StatusError),
}

pub struct ScheduleReport {
    pub decision: ScheduleDecision,
    pub generation: Option<u64>,
}
```

Only one narrow function should translate `SpawnNow` into `spawn_worker`.

The mutation API must distinguish:

- pending recorded and worker spawned;
- pending recorded and already active;
- pending recorded and deferred by backoff;
- pending recorded and attention required;
- pending recorded while auto-sync disabled;
- pending recording failed.

None of the deferred results may be reported as loss of local mutation success.

## Worker-storm rules

- mutation during backoff increments the pending generation but does not spawn a worker;
- mutation while a live sync owns the execution lock increments pending but does not spawn another worker;
- startup recovery during backoff does not spawn;
- startup recovery while attention is required does not spawn;
- explicit retry may bypass time-based backoff only;
- explicit retry must still respect execution lock, valid pending state, and policy/config correctness.

## Tests

Use an injected or test-only spawner counter. Required assertions:

1. 20 mutations during active backoff produce 20 generation increments and zero spawns.
2. 20 mutations while execution lock is live produce zero spawns.
3. after backoff expires, one recovery scheduling request produces exactly one spawn.
4. startup recovery respects backoff.
5. authentication failure blocks automatic spawn.
6. a qualifying config change releases the deferral and permits exactly one spawn.
7. explicit retry bypasses backoff but not active execution lock.
8. no production automatic path calls `spawn_worker` outside the central scheduler adapter.

Add a structural test or code-search guard that fails if `spawn_worker` is referenced from unauthorized modules.

## Acceptance criteria

- `schedule_sync()` is the actual production authority, not unused scaffolding;
- there is exactly one automatic spawn adapter;
- process-storm prevention is proved with exact spawn counts.

---

# Workstream C — Typed Policy Loading and Pending Preservation

## Required implementation

Do not use `get_sync_settings() -> SyncSettings::default()` as the worker or mutation policy source when load failure changes behavior.

Introduce a typed load result:

```rust
pub enum SyncPolicyLoad {
    Loaded(SyncSettings),
    NotConfigured,
    Failed(SyncPolicyLoadError),
}

pub enum SyncPolicyLoadError {
    Read(std::io::ErrorKind),
    Parse,
    Integrity,
    CredentialStore,
    UnsupportedSchema,
}
```

Separate these cases:

- no sync configuration has ever existed;
- sync is configured but auto-sync is disabled;
- config exists and is valid;
- config exists but is malformed;
- integrity verification fails;
- credential retrieval fails;
- filesystem access fails.

## Mutation behavior

When a local sync-relevant mutation commits:

- `NotConfigured`: no pending marker is required;
- `Loaded` with sync configured: record pending regardless of auto-sync enabled state;
- `Failed` and evidence indicates a sync config exists or previously existed: conservatively record pending and mark status `configuration` or `credential_store` attention required;
- never substitute an empty default policy and silently omit pending.

Persist a minimal non-secret installation state if needed to distinguish “never configured” from “configured but currently unreadable.” This state must not duplicate credentials.

## Recovery behavior

After configuration is repaired:

- pending generations created during the failure remain visible;
- a relevant configuration change releases the deferral;
- automatic scheduling resumes according to normal policy;
- no generation increment is synthesized by recovery.

## Credential-change signal

The current fingerprint includes only API-key presence, so replacing one nonempty key with another may not release authentication deferral.

Do not hash or persist the raw API key.

Use one of:

- a credential revision counter incremented by supported register/config commands;
- a keyed one-way digest using a local non-exported salt;
- keychain item metadata/version if reliably available cross-platform;
- an explicit `credentials_changed_at` or random credential nonce stored alongside non-secret config.

The chosen signal must change when the credential value changes while remaining non-secret.

## Tests

- malformed TOML after valid configuration: mutation records pending;
- integrity mismatch after valid configuration: mutation records pending;
- unreadable config: mutation records pending;
- keychain failure: mutation records pending and attention status;
- never-configured installation: mutation does not create sync pending;
- auto-sync disabled but sync configured: mutation records pending and does not spawn;
- replacing a bad nonempty key with a good nonempty key releases authentication deferral;
- config repair does not increment generation;
- status does not contain raw key or a reversible key derivative.

## Acceptance criteria

- configuration failure cannot erase synchronization intent;
- “not configured” is never inferred solely by falling back to defaults after an error;
- credential replacement reliably releases authentication deferral.

---

# Workstream D — Correct Debounce, Preflight, Clock, and Policy Semantics

## Immutable policy snapshot

Resolve one `AutoSyncPolicy` when the worker starts and use it throughout that worker lifecycle.

Remove dynamic calls such as:

```rust
get_sync_settings().auto_sync_debounce()
```

from the inner debounce loop unless a deliberate live-policy-refresh feature is specified. This closure pass should prefer immutable policy.

## Preflight behavior

After debounce returns a generation:

1. reread pending immediately before executor spawn;
2. if marker is absent, cancel without sync;
3. if generation is unchanged, proceed;
4. if generation is newer and maximum delay has not forced execution, restart debounce using the newer state and a new quiet deadline;
5. if maximum delay has already forced execution, sync the latest state immediately;
6. never silently treat corrupt pending as “nothing to do.”

Recommended result:

```rust
pub enum PreflightResult {
    Proceed(PendingState),
    RestartDebounce(PendingState),
    Cancelled,
    Corrupt(PendingError),
}
```

## Clock correctness

All timing comparisons in debounce tests and production logic must go through the injected clock:

- replace `start.elapsed()` with `clock.now_instant().saturating_duration_since(start)`;
- use clock-provided wall time for timestamp conversion;
- avoid direct `Instant::now`, `SystemTime::now`, and `thread::sleep` inside the testable state machine;
- isolate real process-wait timing separately if injection is impractical.

## Duration model

Use explicit fields:

```rust
pub struct AutoSyncPolicy {
    pub debounce: Duration,
    pub max_delay: Duration,
    pub sync_timeout: Duration,
    pub termination_grace: Duration,
    pub worker_lifetime: Duration,
}
```

Worker lifetime must be at least:

```text
max_delay + sync_timeout + termination_grace + safety_margin
```

Do not clamp debounce to worker lifetime in a way that changes documented semantics without an explicit deferral result.

## Tests

Deterministic tests with no wall-clock sleeps:

- newer generation during debounce resets deadline;
- newer generation during preflight restarts debounce;
- marker removal during preflight produces zero executor spawns;
- continuous mutation triggers exactly one forced attempt at maximum delay;
- maximum-delay test advances only the injected clock;
- changing config during a worker lifecycle does not alter that worker's policy snapshot;
- worker lifetime budget cannot expire before a valid maximum-delay attempt and timeout budget complete;
- corrupt pending during debounce/preflight is a typed failure and preserves the file for diagnosis.

## Acceptance criteria

- no state-machine timing branch bypasses the injected clock;
- a preflight generation change cannot skip the quiet period unless maximum delay explicitly forces it;
- one worker lifecycle uses one policy snapshot.

---

# Workstream E — Windows Process Liveness and Lock Recovery

## Required implementation

Replace the non-Unix implementation that returns `true` for every PID.

Implement a Windows-native process liveness probe using the `windows-sys` crate or existing platform facilities.

Recommended method:

1. call `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, ...)`;
2. if access succeeds, call `GetExitCodeProcess`;
3. treat `STILL_ACTIVE` as live;
4. close the handle;
5. distinguish access denied from process-not-found;
6. use conservative behavior for ambiguous errors and expose the reason diagnostically.

Strengthen lock identity beyond PID where feasible:

- PID;
- nonce;
- process start time or creation timestamp;
- lock creation timestamp.

The stale check should avoid reclaiming a lock from a newly reused PID.

## Lock acquisition race

Review `try_acquire` and `wait_acquire` for this race:

1. inspect stale lock;
2. remove path;
3. another process creates a lock;
4. current process removes or overwrites the new lock.

Use ownership-aware removal or an atomic claim protocol. At minimum, reread and compare nonce before deleting.

## Tests

Windows-native tests must cover:

- live current process detected;
- known terminated child detected as dead;
- stale lock from terminated child reclaimed;
- live lock not reclaimed;
- old guard cannot delete replacement lock;
- rapid PID/nonce replacement simulation;
- malformed lock handled conservatively and reported;
- no Unix-only gating for required Windows stale-lock behavior.

Linux/macOS tests must continue passing.

## Acceptance criteria

- a crash cannot permanently disable sync on Windows;
- required dead-owner tests run on Windows rather than being skipped;
- lock reclamation compares ownership identity, not only path existence.

---

# Workstream F — Lossless Executor Result Transport

## Problem

The current exit-code set collapses distinct classes:

- timeout and network;
- conflict and partial;
- authentication and credential-store;
- configuration and internal.

The worker cannot reconstruct the original typed result accurately.

## Preferred design: executor result artifact

Before spawn, the worker allocates a unique private result path containing a cycle nonce:

```text
auto-sync-executor-result-<nonce>.toml
```

Pass it to the executor:

```bash
snp auto-sync-execute --state-dir ... --result-path ... --cycle-nonce ...
```

Executor writes an atomic bounded result:

```rust
pub struct ExecutorResult {
    pub schema: u32,
    pub cycle_nonce: String,
    pub outcome: ExecutorOutcome,
    pub failure_class: Option<FailureClass>,
    pub attempted_generation: u64,
    pub message_code: String,
    pub integrity: u32,
}
```

Rules:

- result path must be inside the trusted state directory;
- create with restrictive permissions;
- reject symlink/non-regular-file targets;
- no raw server response or secret-bearing message;
- worker validates nonce and integrity;
- worker removes the result only after reading it;
- missing result with nonzero/terminated child becomes `ExecutorProtocolFailure` or `Internal`;
- exit code remains a coarse fallback, not the primary classification channel.

An expanded distinct exit-code table is acceptable only if it preserves every required class and remains portable, but a result artifact is preferable because it also carries generation and future structured data.

## Status generation propagation

Pass the actual observed generation into `execute_sync` and the executor result.

Never call:

```rust
record_success(state_dir, 0, ...)
record_failure(state_dir, 0, ...)
```

for a detached attempt that has an observed pending generation.

Spawn/wait failure before a generation is known may use an explicit `None` rather than sentinel zero.

Recommended status types:

```rust
pub pending_generation: Option<u64>
pub last_attempt_generation: Option<u64>
```

## Tests

Round-trip every failure class across the process boundary:

- deferred not configured;
- transient network;
- transient timeout;
- authentication;
- configuration;
- credential store;
- conflict;
- partial;
- local persistence;
- internal;
- signal termination;
- missing/corrupt result artifact.

Assert:

- worker receives the exact original class;
- status stores the exact attempted generation;
- result files contain no sentinel secret;
- stale result from another nonce is rejected;
- result file is cleaned after consumption.

## Acceptance criteria

- no typed failure class is reconstructed from a lossy exit code when a valid result is available;
- detached status never records generation zero as a substitute for a known generation.

---

# Workstream G — Retry Semantics and Backoff Indexing

## Use `RetryDisposition` as the authority

The scheduler must call:

```rust
failure_class.retry_disposition(consecutive_failures)
```

Do not separately approximate retry policy through `allows_automatic_retry()`.

Enforce:

- transient network/timeout: retry after backoff;
- authentication/configuration/credential-store: wait for relevant change or explicit retry;
- conflict/partial/local persistence: requires attention;
- internal: bounded automatic retries, then requires attention;
- disabled/not configured: wait for policy/config change.

## Correct attempt numbering

Define `consecutive_failures` clearly:

- before first failure: 0;
- after first failure: 1;
- first retry delay is derived from failure number 1.

Choose and document one exact schedule. Recommended:

| Consecutive failures after recording | Base delay |
|---|---:|
| 1 | 5s |
| 2 | 15s |
| 3 | 30s |
| 4 | 60s |
| 5+ | exponential, capped at 15m |

Refactor the backoff function to accept the post-failure count and return these values directly.

Jitter must be deterministic under tests through an injected jitter source or disabled in exact unit tests.

## Retry trigger model

A one-shot worker cannot wake itself after exiting. Define how elapsed backoff results in a later attempt:

- next local mutation;
- next normal CLI startup recovery;
- cron/manual sync;
- an explicitly scheduled one-shot retry helper.

Do not claim autonomous retry at a future timestamp unless a process is actually scheduled to wake then.

Prefer the lightweight model:

- no sleeping retry daemon;
- durable eligibility timestamp;
- next eligible trigger spawns one worker;
- documentation states this accurately.

## Tests

- first through sixth backoff intervals match exact schedule before jitter;
- internal failures 1 and 2 retry; failure 3 or selected threshold becomes attention-required according to documented policy;
- success resets count and eligibility;
- explicit retry bypasses time delay but not attention confirmation if policy requires explicit force semantics;
- no worker is spawned automatically merely because wall time passed unless a defined trigger occurs;
- retry documentation matches trigger behavior.

## Acceptance criteria

- retry limits are enforced by the production scheduler;
- backoff has no off-by-one discrepancy;
- no documentation implies an autonomous timer that does not exist.

---

# Workstream H — Stable Status Integrity and Corruption Semantics

## Stable checksum

Replace `DefaultHasher`-based persisted integrity.

Use an explicit stable algorithm already used elsewhere in the project, preferably CRC32 if the field and documentation continue to call it CRC32.

Canonicalize field encoding before checksum calculation. Do not rely on implementation-specific hash behavior.

Integrity must cover every behavior-driving field, including at least:

- schema;
- pending generation;
- attempted generation;
- attempt and success timestamps;
- result code;
- failure class;
- consecutive failures;
- next eligible attempt timestamp;
- executor exit/fallback code;
- attention-required flag;
- configuration or credential revision signal.

The human-readable message may remain outside integrity only if it cannot affect scheduling or behavior.

## Typed status reads

Replace:

```rust
read_status(...) -> Option<AutoSyncStatus>
```

with:

```rust
pub enum StatusRead {
    Missing,
    Valid(AutoSyncStatus),
    Corrupt(StatusError),
}
```

Corrupt status must not silently become a clean default.

Scheduling behavior for corrupt status should be conservative and explicit:

- preserve pending;
- avoid tight spawn loops;
- record or expose attention-required state through doctor/logging;
- permit explicit repair or explicit retry according to documented policy.

Backup corrupt status before replacement when safe.

## Tests

- stable checksum golden vector;
- changing each behavior-driving field invalidates integrity;
- changing message alone follows documented policy;
- fingerprint/revision corruption is detected;
- malformed TOML returns `Corrupt`, not `Missing`;
- corrupt status cannot clear backoff or attention state silently;
- status serialization is stable across repeated writes;
- file remains bounded and private.

## Acceptance criteria

- the algorithm name in code/docs matches the implementation;
- no behavior-driving status field is omitted from integrity;
- corrupt state is distinguishable from absent state.

---

# Workstream I — Real Server-Observable End-to-End Evidence

## Replace the permissive headline test

The existing test must not infer success only from pending disappearance or status-file existence.

Build or extend a recording `snip-sync` test server that exposes:

- registration calls;
- health calls;
- create/list library calls;
- push requests;
- pull requests;
- request timestamps;
- encrypted payload count or server-side stored snippet count;
- controllable barriers and failure injection.

The headline test must:

1. start the real test server on port 0;
2. configure an isolated real `snp` binary environment;
3. register through the supported binary path;
4. enable auto-sync through the supported binary path;
5. commit a real library/snippet mutation;
6. wait for the server recorder to observe the expected sync operation;
7. verify server-side state or payload count changed;
8. verify the executor result/status reports success for the same generation;
9. only then verify pending is cleared.

The test must fail if the executor exits success without contacting the server.

## Eliminate permissive race branches

Do not accept:

- pending exists or not;
- status file merely exists;
- attempts `>= 1` when exactly one is required;
- generation sentinel values invented by the test;
- arbitrary sleeps as primary proof.

Use barriers:

- server blocks the sync operation after recording entry;
- test confirms pending exists while executor is blocked;
- test releases server;
- test confirms server state changes;
- test confirms pending clears afterward.

This removes the fast-worker race entirely.

## Required negative tests

- server unreachable: local commit succeeds, pending remains, exact network classification recorded;
- authentication reject: pending remains, exact auth class recorded;
- server hangs: executor timeout, child dead/reaped, pending remains, exact timeout class recorded;
- partial response: pending remains, exact partial class recorded;
- local persistence failure after remote response: pending remains, exact persistence class recorded;
- mutation during blocked sync: original attempt completes, newer generation remains and receives one later cycle;
- marker removed by foreground successful sync during debounce: detached worker performs zero server attempts.

## Cross-platform requirements

Run the real process-chain test on:

- Linux;
- macOS;
- Windows.

Use Python or Rust helper executables for cross-platform test fixtures. Do not depend on `.sh` files for required Windows tests.

## Acceptance criteria

- a fake success executor cannot pass the headline test;
- success is established from server evidence, result evidence, and pending transition in that order;
- required tests contain no contradictory acceptable outcomes.

---

# Workstream J — CLI and Documentation Reconciliation

## Configuration surface

The implementation and documentation must agree on:

- `--debounce`;
- `--max-delay`;
- `--timeout`, if implemented;
- failure mode;
- whether auto-sync disabled still records pending;
- retry trigger behavior;
- worker/executor ownership;
- status artifact fields;
- Windows lock recovery.

The current documentation advertises `--max-delay` while the command parser does not expose it. Choose one:

1. implement `snp sync config --max-delay <seconds>` now, with validation and persistence;
2. remove the command example and document that the value is internal until Phase 04.

Because the persisted setting already exists, implementing the option is preferred.

## Scope control

Do not implement the full Phase 04 status UX in this pass unless needed for correctness testing.

Permitted additions:

- configuration flags required to make current persisted policy usable;
- hidden/test-only diagnostic output;
- doctor diagnostics for corrupt control state if small and necessary.

Defer broader `snp status`, TUI badges, recovery menus, and user-facing status design to Phase 04.

## Documentation audit

Update:

- README;
- USER_GUIDE;
- `architecture/auto_sync.md`;
- `architecture/sync.md`;
- `architecture/commands/sync_cmd.md`;
- `.skills/sync-module.md`;
- AGENTS.md;
- CHANGELOG;
- test comments that overclaim evidence.

Remove claims that are not proven, including:

- exact failure class preservation if transport remains lossy;
- automatic future retries without an actual wake trigger;
- CRC32 if another algorithm is used;
- server-observable closure evidence unless the server is actually asserted.

## Acceptance criteria

- every documented CLI example parses successfully;
- architecture text describes current production paths rather than intended but unwired modules;
- no test comment claims stronger evidence than its assertions establish.

---

# Workstream K — Cleanup and API Consolidation

After correctness is proven:

- remove direct scheduling helpers no longer needed by production callers;
- make raw `spawn_worker` private to the central scheduling adapter;
- remove `max_retries` if the one-attempt-per-lifecycle model does not use it;
- remove `allows_automatic_retry` if `RetryDisposition` fully replaces it;
- remove sentinel generation zero where `Option<u64>` is correct;
- remove stale status helpers that silently default on corruption;
- remove debug or temporary CI workarounds superseded by barrier-driven tests;
- narrow public exports added only for earlier tests;
- ensure structural tests pin the final module boundaries.

No behavior should depend on string matching where a typed `SyncFailureKind` exists. Legacy fallback matching may remain only at a clearly documented compatibility boundary with focused tests.

---

# Recommended Implementation Sequence

Use small commits that preserve bisectability.

## Commit 1 — Timeout and policy model

- decouple sync timeout from debounce;
- add timeout config or fixed constant;
- expose/fix max-delay CLI;
- add policy resolution tests.

## Commit 2 — Typed policy loading

- distinguish not-configured from load failure;
- preserve pending on malformed/integrity/keychain failures;
- add credential revision signal;
- add migration tests.

## Commit 3 — Central scheduling authority

- route mutation and startup recovery through `schedule_sync`;
- centralize spawn adapter;
- add exact spawn-count tests.

## Commit 4 — Debounce/preflight/clock correction

- immutable policy snapshot;
- preflight restart behavior;
- injected-clock maximum delay;
- worker lifetime budget correction;
- deterministic tests.

## Commit 5 — Windows lock correctness

- native process liveness;
- ownership-aware stale removal;
- Windows dead-owner tests.

## Commit 6 — Lossless executor result protocol

- result artifact or expanded lossless transport;
- attempted generation propagation;
- exact failure round-trip tests;
- result cleanup and security tests.

## Commit 7 — Retry and status integrity

- `RetryDisposition` authority;
- backoff indexing fix;
- stable CRC32/canonical integrity;
- typed corrupt-status handling;
- bounded retry tests.

## Commit 8 — Deterministic server harness and closure matrix

- recording/barrier test server;
- replace permissive headline test;
- add negative-path process-chain tests;
- run cross-platform matrix.

## Commit 9 — Documentation and cleanup

- reconcile docs and CLI;
- remove obsolete helpers and claims;
- update changelog;
- write closure status.

---

# Verification Matrix

Run at minimum:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace --all-features
cargo test --test auto_sync_closure -- --test-threads=1
cargo test --test auto_sync_detached_worker -- --test-threads=1
cargo test --test sync_integration -- --test-threads=1
cargo package --workspace
```

Platform matrix:

```text
Linux stable Rust
macOS stable Rust
Windows stable Rust
```

Required targeted evidence:

- all policy duration tests;
- exact scheduling spawn-count tests;
- malformed-config pending preservation tests;
- Windows stale-lock reclaim tests;
- every failure-class process-boundary round trip;
- exact backoff schedule tests;
- stable status checksum golden tests;
- real server-observable binary-to-worker-to-executor closure test;
- timeout child-death/reap proof;
- mutation-during-sync generation preservation.

No required invariant may be represented only by a structural source-text test when a behavioral test is feasible.

---

# Final Closure Status Document

At completion, add:

```text
plans/snip-it-correctness-03a-closure-status.md
```

Include:

- implementation commit range;
- final architecture summary;
- exact timeout/debounce/max-delay values and configuration surface;
- scheduling call graph;
- Windows liveness method;
- policy-load behavior matrix;
- executor result transport format;
- failure-class mapping table;
- retry/backoff table;
- status schema and integrity algorithm;
- test counts by category;
- CI result for Linux/macOS/Windows;
- explicit list of invariants proven;
- any deferred non-blocking work;
- statement that Phase 04 may begin.

Do not mark the pass complete if CI evidence is unavailable. Record the absence honestly and leave closure open.

---

# Final Exit Criteria

This corrective pass is complete only when all statements below are true:

- executor timeout is independent from debounce;
- every automatic spawn request uses the centralized scheduler;
- backoff and attention state prevent process storms in production paths;
- Windows can reclaim locks from dead owners;
- malformed or unreadable configured sync state cannot suppress pending intent;
- credential replacement releases relevant authentication deferral without persisting the credential;
- newer preflight generations receive correct debounce treatment;
- all state-machine timing uses the injected clock in deterministic tests;
- one worker lifecycle uses one resolved policy snapshot;
- detached status records the actual attempted generation;
- every failure class survives the executor process boundary accurately;
- internal retries stop at the documented bound;
- first and subsequent backoff intervals match documentation;
- status uses a stable named integrity algorithm over all behavior-driving fields;
- corrupt pending/status is distinct from missing state;
- the headline closure test verifies real server-observable change before pending clear;
- a no-op success executor would fail the test;
- Linux, macOS, and Windows required tests pass;
- docs and CLI examples match implementation;
- no required test accepts contradictory outcomes or substitutes status-file existence for successful sync evidence;
- the closure status file records objective evidence.

Only after these criteria are met should work proceed to `snip-it-correctness-04-operational-visibility-recovery.md`.

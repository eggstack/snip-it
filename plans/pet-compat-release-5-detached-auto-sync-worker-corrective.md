# Release 5 Corrective Architecture Plan: Detached One-Shot Auto-Sync Worker

## Purpose

Replace the current synchronous post-mutation auto-sync path with a lightweight detached one-shot worker implemented by re-invoking the existing `snp` binary through a hidden internal subcommand.

Release 5 currently has strong policy, persistence, locking, failure classification, trigger coverage, and test infrastructure, but the production mutation path bypasses the debounce coordinator and calls synchronization synchronously. This causes mutation commands to wait on network activity, makes configured debounce values ineffective in practice, and prevents rapid mutations across short-lived CLI processes from coalescing.

This corrective pass must preserve the existing encrypted `snip-sync` protocol and local-first mutation semantics while avoiding a persistent daemon, IPC service, separate executable, shell wrapper, or service-manager dependency.

The intended architecture is:

```text
user mutation process
  -> commit local state atomically
  -> atomically increment durable pending generation
  -> attempt to spawn current executable as detached worker
  -> return to user immediately

hidden one-shot worker
  -> acquire exclusive worker lock
  -> debounce against durable pending generation/timestamp
  -> execute existing encrypted sync path once
  -> detect newer generation written during debounce/sync
  -> perform at most one coalesced follow-up cycle as needed
  -> clear only the generation it actually synchronized
  -> exit
```

This document is intended for implementation-agent handoff. Inspect the current `src/auto_sync.rs`, CLI hierarchy, sync configuration, manual/scheduled sync paths, mutation triggers, atomic-write helpers, platform process APIs, and Release 5 tests before editing.

## Required Outcomes

After this pass:

1. Normal mutation commands never perform remote synchronization in the foreground.
2. Mutation commands return promptly after their local commit and pending-marker update.
3. A hidden internal worker subcommand uses the current executable and existing sync implementation.
4. Rapid mutations across separate CLI processes coalesce according to the configured quiet-period debounce.
5. Exactly one worker owns debounce and sync execution at a time.
6. Pending intent survives parent-process exit, worker crash, terminal closure, and machine interruption.
7. A worker never clears a newer mutation generation written during its own debounce or sync attempt.
8. Manual `snp sync`, scheduled sync, and explicit `--sync` behavior remain unchanged and continue to provide synchronous remote confirmation.
9. Auto-sync failures never roll back local mutations or cause scripts to repeat already-committed writes.
10. Worker arguments, environment, pending files, lock files, status files, logs, and diagnostics never contain snippet commands, descriptions, output metadata, credentials, API keys, encryption material, or credential-bearing URLs.
11. The current oversized auto-sync implementation is decomposed into focused modules or otherwise simplified so disconnected in-memory coordinator machinery is removed.
12. End-to-end multi-process tests prove observable debounce, prompt parent exit, crash recovery, generation safety, and sync-attempt counts.

## Non-Goals

Do not:

- introduce a persistent daemon;
- install a systemd, launchd, cron, or Windows service for auto-sync;
- add a local socket, named pipe, HTTP listener, or custom IPC protocol;
- create a second shipped binary;
- invoke a shell, `nohup`, or shell-constructed command string;
- pass snippet content or secrets to the worker command line;
- change the encrypted protocol, server API, merge algorithm, sync direction, library mapping, or credential model;
- make `output` remotely synchronized;
- make auto-sync enabled by default;
- remove manual or scheduled sync recovery paths;
- claim guaranteed exactly-once remote execution when crash-safe at-least-once behavior is the safer contract.

## Current Defect Summary

The production notification path currently executes remote sync directly after a mutation:

```text
notify_mutation
  -> notify_local_mutation
  -> run_auto_sync
  -> create Tokio runtime
  -> acquire lock
  -> retry/backoff
  -> run_default_sync
  -> return to mutation command
```

This bypasses the coordinator request/tick lifecycle. Consequently:

- `auto_sync_debounce_seconds` does not govern production mutation behavior;
- rapid mutations do not coalesce;
- mutation commands can block on network timeouts and retries;
- durable pending state is not the authoritative cross-process scheduling mechanism;
- unit tests can validate an in-memory state machine that production commands never use;
- failure policy semantics are ambiguous because the local mutation has already committed before remote failure is known.

The corrective implementation should treat the filesystem marker and one-shot worker as the real cross-process coordinator.

## Product Invariants

1. Local state is authoritative and commits before pending intent is recorded.
2. Pending-marker failure must not corrupt or roll back the local mutation.
3. Auto-sync remains opt-in and requires ordinary sync configuration to be enabled.
4. The parent mutation process performs no remote network work.
5. Parent stdout remains exactly the command's normal success payload.
6. Parent stderr contains only bounded scheduling warnings when spawning or marking pending fails under the configured policy.
7. The worker inherits no terminal interaction and emits no delayed terminal output.
8. The worker uses `std::env::current_exe()` rather than resolving `snp` through `PATH`.
9. The worker is launched directly with structured arguments and never through a shell.
10. Only one worker owns the worker lock at a time.
11. Multiple parents may attempt to spawn workers; losing workers exit immediately after failing to acquire the lock.
12. The durable generation marker is the source of truth for pending work.
13. Clearing pending state is generation-conditional.
14. A worker crash may cause a later redundant sync, but must never lose a committed mutation.
15. Manual and scheduled sync remain independent recovery mechanisms.
16. Sync-origin writes never enqueue another auto-sync generation.
17. Output-only edits and other local-only mutations remain excluded.
18. A detached worker cannot retroactively alter the exit code of its parent mutation command.

## Architectural Decision

### Chosen model

Use the existing `snp` executable as a hidden detached one-shot worker:

```text
snp __auto-sync-worker
```

The exact command name may be `internal auto-sync-worker` if the CLI hierarchy already has a suitable hidden namespace. It must remain hidden from ordinary help and completion output.

### Why this model

It provides:

- no persistent service;
- no IPC protocol;
- no additional binary distribution;
- real cross-process debounce;
- process-lifecycle independence from the parent command;
- reuse of the existing config, logging, lock, retry, and sync code;
- a bounded process that exits after pending work is handled.

### Rejected models

#### Persistent daemon

Rejected because it adds process supervision, installation, lifecycle management, IPC, and platform-specific service integration disproportionate to the feature.

#### In-process coordinator owned by mutation command

Rejected because `snp` commands are short-lived. A multi-second debounce cannot outlive the parent process unless the parent blocks, which defeats the responsiveness requirement.

#### Synchronous post-mutation sync

Rejected as the default auto-sync behavior because it ignores debounce and can block interactive commands on network activity. Explicit `--sync` and `snp sync` remain the synchronous paths.

#### Cron-only recovery

Retain cron/manual sync as recovery, but do not rely on it as the primary auto-sync experience because it cannot provide prompt quiet-period synchronization.

## Workstream A: Hidden Worker CLI Surface

### A1. Add hidden command

Add an internal command such as:

```rust
#[command(hide = true)]
AutoSyncWorker
```

Requirements:

- absent from ordinary help text;
- absent from generated shell completions unless the framework cannot exclude hidden commands;
- no user-facing aliases;
- no snippet-content arguments;
- no API key, server URL, device ID, library body, or encryption arguments;
- loads all policy and credentials through existing config paths;
- callable directly in tests.

### A2. Worker exit contract

Define a small internal contract:

```text
0: no pending work, another live worker owns the lock, or work completed
1: worker infrastructure/configuration failure
2: pending state invalid and could not be safely recovered
```

The parent process never waits for or consumes this status.

Avoid exposing remote sync failure as a parent mutation error. Persist worker status instead.

### A3. Direct dispatch

The hidden command should dispatch directly into a library function such as:

```rust
pub fn run_worker() -> SnipResult<WorkerOutcome>
```

Keep CLI parsing separate from worker logic so unit and integration tests do not need to fork for every state transition.

## Workstream B: Parent Notification Path

### B1. Replace synchronous execution

Refactor `notify_local_mutation()` so it no longer calls `run_auto_sync()`.

New flow:

```text
resolve policy
  -> disabled? return Disabled
  -> sync-origin/local-only? return Suppressed
  -> record pending generation atomically
  -> spawn detached worker best-effort
  -> return Scheduled/Pending status
```

Suggested result model:

```rust
pub enum AutoSyncNotificationResult {
    Disabled,
    Suppressed,
    Scheduled { generation: u64 },
    PendingExistingWorker { generation: u64 },
    SchedulingFailed { generation: Option<u64>, class: SchedulingFailure },
}
```

Do not return remote sync status because remote work occurs after the parent exits.

### B2. Preserve local success

If pending-marker update or worker spawn fails after local commit:

- never roll back local state;
- apply the configured scheduling-failure policy only to diagnostics/status;
- default `warn` may write one bounded stderr warning;
- `ignore` remains silent except debug logs;
- redefine `error` as durable attention-required status rather than parent exit failure, unless the project deliberately introduces a separate synchronous mode.

Document that explicit `--sync` is the path for scripts requiring remote confirmation.

### B3. Spawn on recovery opportunities

Add a cheap best-effort recovery hook in safe CLI startup or selected commands:

```text
if auto-sync enabled
and valid pending marker exists
and no live worker owns the lock
then attempt detached worker spawn
```

This hook must:

- perform no network work;
- add negligible startup latency;
- avoid recursion when running inside the hidden worker;
- not emit routine stdout/stderr;
- avoid spawning repeatedly when a valid worker lock exists.

Candidate recovery points:

- normal CLI startup after command parsing;
- `snp doctor --compatibility`;
- `snp sync status`;
- mutation notification itself.

Do not require every read-only command to spawn unless the startup check is demonstrably cheap and quiet.

## Workstream C: Durable Generation Marker

### C1. Versioned schema

Replace or migrate the current pending schema to a generation-safe version:

```toml
version = 2
pending = true
generation = 42
requested_at_unix_ms = 1784090000123
last_attempt_generation = 41
last_attempt_at_unix_ms = 1784089999000
last_result = "network"
```

Requirements:

- millisecond or monotonic-compatible timestamp precision for rapid mutations;
- monotonically increasing generation with checked or saturating behavior;
- no snippet content or secrets;
- bounded serialized size;
- integrity/version validation;
- atomic private writes;
- explicit migration from version 1 markers;
- unknown future versions fail safely without destructive rewrite.

### C2. Atomic increment

Provide one function:

```rust
pub fn mark_pending(kind: MutationKind) -> SnipResult<PendingGeneration>
```

It should:

1. acquire a narrow pending-update lock or use an atomic compare/update strategy;
2. load and validate the current marker;
3. increment generation;
4. set `pending = true`;
5. update request timestamp;
6. preserve bounded status fields where appropriate;
7. atomically replace the marker;
8. release the update lock.

Cross-process mutations must not lose increments through read-modify-write races.

If the existing worker lock is too coarse, create a separate very short-lived marker-update lock. Do not hold the worker lock in every parent mutation process.

### C3. Conditional clearing

Implement:

```rust
pub fn clear_if_generation_matches(observed: u64) -> SnipResult<ClearOutcome>
```

Behavior:

- current generation equals observed: clear pending or persist success status;
- current generation is newer: do not clear; worker must perform a follow-up cycle;
- marker absent: treat as already cleared;
- marker corrupt: preserve evidence and return bounded failure.

Never unconditionally delete the marker after sync.

### C4. Manual sync precedence

After successful explicit/manual sync:

- read current generation;
- clear pending through generation-aware logic;
- prevent an older detached worker from clearing or overwriting newer state;
- define lock ordering between manual sync and auto-sync worker;
- avoid deadlocks.

If manual sync occurs while a worker is running, choose and document one policy:

- manual sync waits on the same execution lock;
- manual sync proceeds independently and marks the current generation satisfied;
- manual sync detects active worker and exits with explicit status.

Preference: use one execution lock so remote sync executions do not overlap, while keeping pending updates lock-independent and fast.

## Workstream D: Detached Spawn Implementation

### D1. Resolve current executable

Use:

```rust
std::env::current_exe()
```

Do not invoke `snp` by name through `PATH`.

Validate and handle:

- deleted/replaced executable during upgrade;
- symlinked invocation;
- spaces and Unicode in executable path;
- test harness executable overrides.

### D2. Standard streams

Configure child process with:

```text
stdin  -> null
stdout -> null
stderr -> null
```

Worker failures are persisted to status/logging, not written to the user's terminal after the parent exits.

### D3. Unix detachment

Implement process detachment without a shell. Evaluate:

- `setsid()` in a `pre_exec` hook;
- signal disposition inheritance;
- working directory independence;
- closing inherited descriptors;
- parent terminal/session death behavior.

Requirements:

- child survives normal parent exit and terminal closure;
- child does not inherit raw mode or TUI terminal state;
- no unsafe code beyond a narrowly reviewed platform helper where unavoidable;
- document any `pre_exec` safety constraints.

### D4. Windows detachment

Use `std::os::windows::process::CommandExt` creation flags appropriate for a background no-console worker.

Validate:

- parent console closure does not terminate worker;
- no console window flashes;
- standard handles are not inherited;
- lock stale detection works without Unix `kill -0`.

If full Windows detachment cannot be made reliable in this pass, explicitly gate auto-sync worker support and keep manual sync functional. Do not silently advertise unsupported behavior.

### D5. Spawn race policy

Every mutation may attempt to spawn a worker after writing pending state.

Do not require the parent to accurately determine whether a worker already exists. The child should arbitrate through the atomic worker lock:

```text
spawned child acquires lock -> becomes active worker
spawned child fails lock -> exits 0 immediately
```

This avoids PID-check races in the parent.

## Workstream E: Worker Lock and Ownership

### E1. Separate responsibilities

Define clearly:

- pending-update lock: protects generation marker read-modify-write;
- worker/execution lock: ensures one debounce/sync worker owns processing;
- sync execution lock: may be the same as worker lock if manual and scheduled sync coordinate through it.

Avoid one long-held lock that blocks mutation commands from marking newer pending work.

### E2. Lock metadata

Use bounded lock contents such as:

```toml
version = 1
pid = 12345
started_at_unix_ms = 1784090000000
nonce = "random-worker-id"
```

Include a random nonce or process-start identity to reduce PID-reuse ambiguity.

Do not store command lines, environment variables, URLs, or credentials.

### E3. Stale detection

Implement platform-specific liveness checks behind one interface.

Unix:

- process existence;
- optional start-time/nonce validation where feasible;
- stale age upper bound.

Windows:

- process handle/liveness check or conservative age-based recovery;
- explicit tests.

A stale lock must be recoverable without deleting a live worker's lock.

### E4. Ownership-safe cleanup

The worker removes the lock only if the lock nonce still matches its own identity.

Do not rely solely on RAII deletion by path, because a stale-recovery race could replace the lock while an older worker exits.

## Workstream F: Worker Debounce Loop

### F1. Acquire and inspect

Worker startup:

1. load effective auto-sync policy;
2. exit if disabled;
3. acquire worker lock;
4. exit 0 if another worker owns it;
5. load pending marker;
6. exit if no valid pending work;
7. enter quiet-period loop.

### F2. Quiet-period calculation

Use:

```text
deadline = requested_at + configured debounce
remaining = deadline - now
```

Sleep only for remaining duration. After wake:

- reload marker;
- if generation or request timestamp changed, calculate a new deadline;
- if unchanged and deadline reached, begin sync.

Do not repeatedly sleep the full debounce duration.

### F3. Maximum worker lifetime

Define a bounded but practical worker lifetime.

The worker must support the configured maximum debounce, but must not remain alive forever under constant mutation.

Choose and document a policy such as:

```text
quiet period: configured debounce
maximum coalescing window: 300 seconds from worker start
```

At the maximum window, sync the latest generation even if mutations continue, then perform at most one follow-up cycle.

Do not interpret the debounce maximum as permission to postpone indefinitely.

### F4. Sync execution

Call the existing encrypted sync executor after debounce.

Requirements:

- load latest sync configuration at execution time;
- use existing retry/timeout/failure classification;
- never invoke shell commands;
- never change sync direction or target implicitly;
- update bounded durable status;
- keep pending generation intact until success handling completes.

### F5. Mutation during sync

Before sync, record `observed_generation`.

After success or terminal failure, reload marker:

```text
current generation == observed generation
  -> mark generation satisfied / clear pending

current generation > observed generation
  -> preserve pending
  -> apply short follow-up debounce
  -> run one coalesced follow-up cycle
```

Avoid unbounded loops. A practical worker may continue while generations advance but should have a maximum lifecycle and leave pending state for a successor worker if necessary.

### F6. Failure behavior

On remote failure:

- retain `pending = true` unless failure policy explicitly says no automatic retry;
- record bounded failure class and attempt time;
- avoid raw error bodies;
- release worker lock;
- exit;
- allow later mutation/startup recovery/manual sync to retry.

Do not clear pending automatically merely because retries were exhausted.

Define policy semantics:

- `ignore`: no user-facing warning; keep recoverable status;
- `warn`: durable warning status visible via doctor/status;
- `error`: durable attention-required status; parent mutation still remains locally successful.

## Workstream G: Status and Recovery UX

### G1. Durable status

Persist bounded fields separate from or within the pending marker:

```toml
last_attempt_at_unix_ms = 1784090020000
last_success_at_unix_ms = 1784090010000
last_result = "network"
last_attempt_generation = 42
pending = true
```

Use stable enums/codes rather than raw error messages.

### G2. Doctor integration

Extend `snp doctor --compatibility` to report:

- auto-sync enabled/disabled;
- pending generation;
- pending age;
- worker lock state;
- live/stale worker determination;
- last success age;
- last failure class;
- invalid marker version/integrity;
- recommended recovery command.

Doctor remains read-only unless an explicit repair flag is introduced separately.

### G3. Sync status command

Prefer an additive command such as:

```text
snp sync status
```

Output should distinguish:

```text
disabled
idle
pending
debouncing
running
failed-network
failed-auth
stale-worker
invalid-state
```

Provide JSON only if the current sync command already has a machine-output pattern or if schema versioning is included.

### G4. Explicit recovery

Document:

```text
snp sync
```

as the authoritative synchronous recovery path.

Optionally add a hidden/internal worker restart path, but avoid another user-facing lifecycle command unless evidence demands it.

## Workstream H: Manual, Scheduled, and Explicit Sync Isolation

### H1. Preserve manual behavior

`snp sync` must continue to:

- run synchronously;
- return remote success/failure to the caller;
- use existing stdout/stderr and exit semantics;
- clear satisfied pending generations after success;
- avoid spawning another worker from sync-origin writes.

### H2. Preserve `--sync`

Commands with explicit `--sync` remain synchronous by user request.

Define precedence:

- local mutation commits;
- explicit sync runs;
- successful explicit sync satisfies the current generation;
- detached auto-sync is not additionally scheduled for the same generation.

The mutation trigger should know explicit sync is requested before spawning a worker, or explicit sync should conditionally clear the exact generation afterward.

### H3. Preserve cron/scheduled sync

Scheduled sync remains independent. After successful scheduled sync, it may satisfy pending generations if it acquires the same execution coordination and observes the marker.

Do not change generated cron text or service behavior unless needed for generation-safe clearing.

### H4. Avoid recursive scheduling

All writes performed during sync merge must carry `MutationOrigin::SyncMerge` or otherwise bypass notification.

Add assertions around every merge/save path.

## Workstream I: Module Decomposition and Cleanup

### I1. Split the current module

Refactor the nearly 2,900-line `src/auto_sync.rs` into focused modules, preferably:

```text
src/auto_sync/
  mod.rs
  policy.rs
  notification.rs
  pending.rs
  lock.rs
  spawn.rs
  worker.rs
  executor.rs
  status.rs
```

Suggested ownership:

- `policy.rs`: effective configuration and failure policy;
- `notification.rs`: parent mutation API;
- `pending.rs`: generation marker, integrity, migration, conditional clear;
- `lock.rs`: update/worker locks and stale detection;
- `spawn.rs`: platform detachment;
- `worker.rs`: quiet-period loop and generation handling;
- `executor.rs`: retry/timeout wrapper around existing sync;
- `status.rs`: bounded durable and display status.

### I2. Remove disconnected machinery

Delete or simplify the in-memory coordinator state machine if it is no longer used in production.

Do not retain parallel implementations solely because unit tests exist.

Every public/internal abstraction should have a real production caller or a clear test-support purpose.

### I3. Centralize time and filesystem dependencies

For deterministic tests, isolate:

- current time;
- sleep;
- process spawning;
- process liveness;
- sync execution;
- state directory;
- atomic writes.

Use narrow traits or function injection rather than a broad framework.

### I4. Keep the architecture lightweight

Avoid generic job queues, async runtimes in the parent, actor frameworks, database state, or daemon abstractions.

The filesystem marker plus one worker process is sufficient.

## Workstream J: Security Hardening

### J1. No secret-bearing process state

Assert the child argument list contains only the hidden command name and optional non-secret internal flags.

Do not forward the complete parent environment if avoidable. At minimum, audit sensitive environment variables and ensure they are not logged.

Credentials still load through the existing keychain/config mechanism in the worker.

### J2. Symlink and file-type policy

For marker and lock paths:

- reject symlinks;
- reject directories and special files;
- use atomic private creation;
- use 0600 on Unix;
- validate ownership where feasible;
- bound file reads;
- prevent path traversal through configurable state directories.

### J3. Worker executable trust

Use `current_exe()` and direct spawn.

Do not fall back silently to `PATH` if `current_exe()` fails. Record scheduling failure and retain pending intent for manual/startup recovery.

### J4. Logging discipline

Logs may contain:

- generation number;
- worker nonce;
- failure class;
- timestamps;
- bounded state codes.

Logs must not contain:

- snippet command;
- description;
- output;
- tags;
- server URL with user info/query;
- API key;
- encryption key/material;
- raw upstream response body.

### J5. Upgrade behavior

A parent may spawn a worker while the binary is being replaced.

Test or document:

- worker uses the executable image already opened by the OS;
- pending survives failed spawn;
- next invocation recovers with the new binary;
- marker version migration remains backward compatible.

## Workstream K: Failure and Race Matrix

Test and document each case:

1. local save succeeds, marker write succeeds, spawn succeeds;
2. local save succeeds, marker write succeeds, spawn fails;
3. local save succeeds, marker write fails;
4. parent crashes after marker write before spawn;
5. parent exits normally while worker debounces;
6. terminal closes while worker debounces;
7. worker crashes before sync;
8. worker crashes during sync;
9. worker succeeds but crashes before conditional clear;
10. mutation arrives while worker sleeps;
11. mutation arrives while worker syncs;
12. two parents spawn simultaneously;
13. stale worker lock exists;
14. live worker PID is reused or ambiguous;
15. pending marker is corrupt;
16. pending marker is an unknown future version;
17. marker update races with worker clear;
18. manual sync races with worker debounce;
19. manual sync races with worker remote execution;
20. scheduled sync races with worker;
21. sync credentials are missing or invalid;
22. server is unreachable;
23. worker timeout expires;
24. auto-sync is disabled after worker spawn but before execution;
25. sync settings change during debounce;
26. state directory permissions prevent marker or lock access;
27. executable cannot be resolved;
28. Windows detachment is unavailable;
29. Unix `setsid`/spawn setup fails;
30. system clock moves backward or forward.

For each case, define:

- local mutation outcome;
- pending-marker outcome;
- worker outcome;
- user-visible diagnostics;
- recovery path;
- whether redundant sync is acceptable.

## Workstream L: End-to-End Test Plan

### L1. Parent responsiveness

Use an unreachable or controlled slow sync endpoint.

Assert:

- mutation local file is committed;
- parent exits within a tight bound substantially below sync timeout;
- no delayed terminal output appears;
- worker remains independently observable through status files/test hooks.

Do not use a permissive 30-second threshold as the primary responsiveness test. Prefer a sub-second or low-single-second bound appropriate to process spawn and local I/O.

### L2. Real cross-process debounce

Launch multiple independent `snp` mutation processes within the debounce window.

Use a fake/counting sync endpoint or injected executor and assert:

- all local mutations persist;
- one pending generation sequence advances;
- only one remote sync attempt occurs after quiet period;
- sync does not start before the quiet period.

### L3. Debounce zero

With `auto_sync_debounce_seconds = 0`:

- parent still exits promptly;
- worker starts sync promptly;
- parent does not wait for completion.

### L4. Mutation during sync

Block the first worker sync at a controlled barrier, perform another mutation, release the barrier, and assert exactly one follow-up sync handles the newer generation.

### L5. Generation-clear race

Arrange:

```text
worker observes generation 10
mutation writes generation 11
worker completes generation 10
```

Assert generation 11 remains pending.

### L6. Competing workers

Spawn multiple hidden workers and assert:

- one acquires ownership;
- others exit promptly and successfully;
- only one sync attempt occurs;
- lock cleanup is ownership-safe.

### L7. Crash recovery

Terminate the worker during debounce and during sync.

Assert:

- marker remains pending;
- later normal CLI startup or mutation respawns a worker;
- pending work eventually syncs;
- no marker corruption occurs.

### L8. Manual sync precedence

With pending work and a sleeping worker:

- run `snp sync`;
- assert manual sync succeeds synchronously;
- current generation is marked satisfied;
- worker does not perform a duplicate later sync.

### L9. Stream and terminal isolation

PTY tests must verify:

- parent terminal mode restored;
- worker emits no output to the parent terminal after exit;
- no inherited stdin reads;
- no alternate-screen/cursor/raw-mode leakage;
- shell prompt remains usable.

### L10. Security sentinels

Inject sentinel values into:

- commands;
- descriptions;
- output;
- tags;
- API key;
- URL user info/query;
- environment variables.

Assert sentinels are absent from:

- child arguments;
- marker;
- lock;
- status;
- debug representations;
- logs;
- stderr warnings.

### L11. Platform matrix

Validate at minimum:

- Linux;
- macOS;
- Windows.

Platform-specific tests may be gated, but hosted CI evidence should show worker spawn, lock, detachment, parent exit, and cleanup behavior.

## Workstream M: Documentation

Update:

- README auto-sync overview;
- USER_GUIDE configuration and behavior;
- `architecture/auto_sync.md`;
- `architecture/sync.md`;
- CLI exit-code/stream policy;
- architecture inventory;
- Pet compatibility matrix;
- AGENTS.md;
- CHANGELOG.

Document precisely:

- auto-sync is implemented by a detached one-shot invocation of the same binary;
- no daemon or service installation is required;
- parent mutations return after local commit and scheduling;
- debounce is cross-process and generation-based;
- worker failures are visible through status/doctor rather than parent exit status;
- explicit `--sync` and `snp sync` remain synchronous confirmation paths;
- pending work is at-least-once and may cause a safe redundant sync after crashes;
- manual/scheduled sync remain recovery mechanisms;
- platform limitations, if any.

Remove or correct documentation that claims the in-process coordinator owns production mutation debounce.

## Workstream N: Validation and Closure

Run and record:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --test pty_integration -- --test-threads=1
```

Also run dedicated serialized multi-process suites for:

- detached spawn;
- cross-process debounce;
- generation safety;
- worker crash recovery;
- lock ownership;
- manual sync precedence;
- stream isolation;
- security sentinels;
- platform-specific detachment.

Release 5 is not closed based only on unit tests of pending/coordinator types. Closure requires observable subprocess behavior.

## Suggested Implementation Sequence

### Commit 1: State and module preparation

- split policy/pending/lock/executor from current module;
- introduce marker v2 with generation and migration;
- add conditional clear and atomic increment tests;
- preserve existing synchronous behavior temporarily.

### Commit 2: Hidden worker and platform spawn

- add hidden CLI command;
- implement direct `current_exe()` spawning;
- add Unix/Windows detachment helpers;
- add worker lock ownership and basic no-work behavior.

### Commit 3: Worker debounce loop

- implement quiet-period generation reload loop;
- wire existing sync executor;
- handle mutation-during-sync follow-up;
- persist bounded status.

### Commit 4: Parent notification switch

- replace synchronous `run_auto_sync()` calls with marker update plus spawn;
- redefine notification results and failure policy;
- add startup recovery hook;
- preserve explicit/manual sync paths.

### Commit 5: Race, failure, and platform closure

- add real multi-process counting tests;
- add crash/manual-sync/generation race tests;
- harden stale locks and ownership cleanup;
- verify terminal/stream isolation;
- update documentation.

### Commit 6: Cleanup

- remove unused in-memory coordinator machinery;
- remove obsolete tests that validate disconnected behavior;
- reconcile architecture docs and test inventories;
- record full validation evidence.

## Acceptance Criteria

Release 5 may be closed when all of the following are true:

1. Production mutation commands never call remote sync synchronously unless the user explicitly requests `--sync`.
2. The current executable launches a hidden detached one-shot worker directly, without a shell.
3. Parent mutation commands return promptly under slow or unreachable network conditions.
4. Configured debounce controls observable worker timing.
5. Multiple CLI mutations within one quiet period produce one remote sync attempt.
6. A mutation during sync produces a bounded follow-up without losing generations.
7. Pending generations are updated atomically across processes.
8. Workers clear pending state only when the observed generation is still current.
9. Competing workers are serialized by an ownership-safe lock.
10. Worker crashes leave recoverable pending state.
11. Manual and scheduled sync coordinate safely and preserve their prior user-facing contracts.
12. Auto-sync failure never causes local rollback or ambiguous mutation retry semantics.
13. Durable status exposes bounded failure/recovery information without secrets.
14. Hidden worker arguments and all sidecar files pass sentinel-leak tests.
15. Unix and Windows detachment behavior is either validated or explicitly feature-gated/documented.
16. The old disconnected coordinator path is removed or has a real production role.
17. Full workspace, PTY, multi-process, security, and platform validation passes.
18. Documentation accurately describes the one-shot worker architecture and at-least-once recovery semantics.

## Definition of Done

The corrective architecture is complete when snip-it provides genuinely debounced, non-blocking, local-first auto-sync through a short-lived detached invocation of its existing binary, with no persistent daemon, no IPC service, no shell wrapper, no lost mutation generations, no delayed terminal output, and no regression to manual or scheduled encrypted synchronization.

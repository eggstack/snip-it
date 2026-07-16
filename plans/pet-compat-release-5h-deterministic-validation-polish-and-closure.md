# Release 5H: Deterministic Validation, Polish, and Closure

## Purpose

Provide final Release 5 correctness evidence after transactional state handling, unified sync execution, and policy/recovery fixes land.

The current test suite contains several permissive scenarios that accept multiple contradictory outcomes, do not create the advertised concurrency, or infer worker behavior from lock-file disappearance rather than observing actual sync attempts. This phase replaces those tests with deterministic multi-process validation and closes documentation, CI, diagnostics, and maintainability gaps.

## Entry Preconditions

Do not begin closure until Release 5E, 5F, and 5G are implemented:

- pending generation updates and conditional clear are transactional;
- shared sync execution locking exists;
- timeout supervision terminates/reaps the executor before unlock;
- debounce returns the latest pending state;
- policy load failures preserve pending work;
- configured direction is respected;
- durable status and recovery behavior are defined.

## Required Outcomes

1. Every critical concurrency invariant is proven through real subprocesses.
2. Tests count actual sync attempts through a controllable local recording server.
3. No test accepts both success and failure as equivalent evidence.
4. Manual, explicit, scheduled, and detached sync mutual exclusion is proven.
5. Timeout tests prove executor death and lock retention until reap.
6. Linux, macOS, and Windows CI cover platform-sensitive process and lock behavior.
7. Security tests cover symlinks, permissions, argv/env, logs, marker/status contents, and corruption.
8. Documentation exactly matches production behavior.
9. Obsolete tests, APIs, comments, and diagnostic hooks are removed.
10. Release 5 closure evidence is recorded in a status document.

## Workstream A: Deterministic Test Harness

Build a dedicated local test server/harness with explicit controls:

- record connection and sync-attempt count;
- distinguish push, pull, health, list, and sync operations;
- block at named barriers;
- release barriers from the test process;
- inject success, network failure, auth failure, conflict, malformed response, and hang;
- expose timestamps for attempt start/end;
- avoid fixed ports by binding to port 0;
- avoid arbitrary sleeps for correctness assertions;
- use condition variables/channels/files only as test control primitives.

Where a full protocol server is too heavy, add a hidden test-only executor mode behind `cfg(test)` or a feature that writes bounded events to a test event file. Production binaries must not expose unsafe test controls by default.

## Workstream B: Exact Pending-State Concurrency Matrix

Required tests:

1. 20 concurrent parent mutations from generation 0 produce generation 20.
2. 20 concurrent mutations from generation 100 produce generation 120.
3. Every mutation command succeeds locally.
4. Marker parses and integrity passes after each stress round.
5. Concurrent mutation and conditional clear preserve the newer generation.
6. Concurrent writers never share a temp file or emit rename failures.
7. Startup recovery never increments generation.
8. Scheduling existing pending work is byte-for-byte read-only.
9. Repeated stress loops do not regress or duplicate generations.

Run a bounded stress variant under CI and a larger ignored/manual variant for local soak testing.

## Workstream C: Exact Debounce Tests

Use the recording server to prove:

- debounce 0 starts one attempt promptly;
- debounce 2 starts no attempt before the quiet deadline;
- 20 mutations within the window produce exactly one attempt;
- final attempt starts relative to the last mutation, not the first;
- latest generation is conditionally cleared on success;
- marker removal during debounce causes zero attempts;
- mutation during active sync causes exactly one later follow-up attempt;
- no redundant stale-generation attempt occurs before the follow-up.

Do not accept `>= 1` when the contract requires exactly one.

## Workstream D: Sync Execution Mutual Exclusion

Create barrier-driven tests for every pair that can overlap:

- worker vs worker;
- worker vs manual sync;
- worker vs cron sync;
- worker vs `run --sync`;
- worker vs TUI delete explicit sync;
- manual sync vs cron;
- manual sync vs explicit `--sync`.

For each test:

1. First operation enters the controlled sync critical section.
2. Second operation starts.
3. Assert second operation cannot enter the critical section.
4. Release first operation.
5. Assert second follows documented wait/defer/error behavior.
6. Assert maximum concurrent execution count remains one.

## Workstream E: Timeout and Process Lifecycle Validation

Required platform-aware tests:

- executor hangs after acquiring execution lock;
- worker timeout fires;
- graceful termination is attempted;
- force termination occurs when required;
- child PID/handle is confirmed dead;
- child is reaped;
- lock remains held until child death is confirmed;
- pending marker remains;
- later recovery successfully retries;
- no orphan/zombie process remains;
- parent mutation command was never blocked by this lifecycle.

Add an assertion that no `spawn_blocking` sync task or equivalent in-process unkillable work remains.

## Workstream F: Direction and Sync Semantics

Recording-server tests must prove:

- Push configuration performs no pull/merge operation;
- Pull configuration performs no upload operation;
- Bidirectional performs both required phases;
- CLI overrides configuration;
- worker and manual sync resolve direction identically;
- local-only output/favorite/folder/usage metadata remains unsynchronized;
- sync-origin writes do not trigger auto-sync recursion.

## Workstream G: Failure, Recovery, and Status Matrix

Inject and verify:

- unreachable server;
- DNS/connect timeout;
- auth rejection;
- conflict response;
- local save failure;
- malformed config;
- integrity mismatch;
- keychain failure;
- execution lock busy;
- stale/dead lock;
- old pending marker;
- malformed pending/status file.

For each case assert:

- local mutation durability;
- pending preservation or clearing according to contract;
- durable result/failure class;
- next-attempt/backoff behavior;
- doctor/status output;
- no secret leakage;
- foreground versus detached exit semantics.

## Workstream H: Lock Ownership and Lease Tests

Prove:

- live lock is never stolen by age;
- dead lock is reclaimable;
- malformed lock is handled conservatively;
- old guard cannot delete new owner's lock;
- PID reuse simulation is mitigated by nonce/identity checks;
- Windows liveness implementation behaves correctly;
- ownership-checked drop works under rapid replacement races.

## Workstream I: Security and Filesystem Hardening

Tests must cover:

- pending, status, and lock permissions;
- symlink substitution for every artifact path;
- directory/FIFO/device substitution where relevant;
- unique temp-file permissions;
- argv/env contain only state path and non-sensitive controls;
- debug log opt-in path cannot expose credentials or snippet payloads;
- status/error strings are bounded and sanitized;
- CRC/integrity corruption is detected for all control fields;
- no shell invocation occurs;
- `current_exe` is used directly.

Document the local attacker threat model and the non-cryptographic role of CRC32.

## Workstream J: CI Matrix

Ensure GitHub Actions includes:

- Linux stable Rust;
- macOS stable Rust;
- Windows stable Rust;
- formatting;
- Clippy with warnings denied;
- unit/integration tests;
- platform-appropriate detached-worker tests;
- timeout/process tests with bounded job-level timeouts;
- artifact upload for worker/status logs only on failure, with secret-sentinel scans.

Avoid simply gating difficult tests off Windows. Where behavior is platform-specific, add a Windows-native assertion rather than omitting coverage.

Track and eliminate flaky tests. Any retry wrapper must report the underlying failure and cannot mask deterministic race defects.

## Workstream K: Test Quality Audit

Remove or rewrite tests that:

- assert only that no panic occurred;
- permit either marker presence or absence when one is required;
- permit `>= 1` attempts when exactly one is required;
- construct a subprocess but do not start it concurrently;
- use `< 15 seconds` as evidence of prompt CLI behavior;
- infer network attempts only from lock disappearance;
- permit equal generations when exact increments are required.

Prompt-return tests should use a strict local threshold appropriate for process spawn overhead, with platform-specific allowances and no network dependency.

## Workstream L: Code and API Polish

Audit and remove:

- unused `max_retries` or other dead policy fields;
- legacy `mark_pending` production alias;
- obsolete `record_success`, `clear_for_explicit_sync`, or wrappers not used by the final protocol;
- stale comments claiming false cancellation;
- duplicated direction resolution;
- old coordinator terminology;
- debug environment variables not intended for supported diagnostics;
- permissive catch-all error paths that erase failure classification.

Keep module responsibilities narrow and document public/internal boundaries.

## Workstream M: Documentation Reconciliation

Review every Release 5 reference in:

- README;
- AGENTS.md;
- architecture overview;
- sync architecture;
- auto-sync deep dive;
- configuration reference;
- doctor/status docs;
- cron examples;
- all Release 5 plans/status files.

Ensure docs state:

- detached worker and executor lifecycle;
- transaction lock versus execution lock;
- exact debounce behavior;
- timeout and termination behavior;
- configured direction semantics;
- pending recovery/backoff;
- failure-mode semantics;
- local-only metadata contract;
- platform limitations, if any.

## Workstream N: Closure Evidence

Create a final status file such as:

```text
plans/pet-compat-release-5-closure-status.md
```

Include:

- final architecture summary;
- commit range;
- test counts by category;
- CI links/status for each platform;
- exact invariants proven;
- documented limitations;
- deferred non-blocking improvements;
- statement that no daemon/IPC/second installed binary was introduced.

Do not mark Release 5 complete while any test is ignored for a required invariant or any platform job is missing.

## Recommended Commit Sequence

1. Introduce deterministic recording/barrier harness.
2. Replace pending/debounce permissive tests.
3. Add sync mutual-exclusion and timeout lifecycle tests.
4. Add direction, recovery, and failure-status matrix.
5. Add lock/security/platform tests.
6. Remove obsolete APIs and diagnostics.
7. Reconcile documentation.
8. Run complete CI matrix and write closure status.

## Final Exit Criteria

Release 5 may close only when all of the following are true:

- concurrent generations are exact and lossless;
- conditional clear cannot delete newer state;
- exactly one sync runs at a time across all entry points;
- timeout kills and reaps the executor before unlock;
- mutation bursts produce exactly one post-debounce attempt;
- marker removal cancels stale work;
- configured direction is honored everywhere;
- config failures preserve pending intent;
- stale pending work is recoverable;
- failure/status behavior is truthful and durable;
- security artifacts are private and secret-free;
- Linux, macOS, and Windows CI pass;
- no required test is ignored or permissive;
- docs and implementation agree.

# Phase 13H — Final Correctness Closure for Empty Sync, Task Drain, and Retained-State Recovery

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Supersedes the incorrect final-closure claims in: `plans/snip-it-phase-13g-corrective-closure.md`

Reviewed baseline: `00bee90300d1984ccfc01a12f1fcd909fd6a3d60`

Date: 2026-08-05

## 1. Purpose

Phase 13G corrected the original multi-batch early-return defect, singleton overflow validation, request-envelope sizing, Unix SIGTERM registration, and several configuration-validation gaps. A post-implementation review of `00bee90300d1984ccfc01a12f1fcd909fd6a3d60` found that Phase 13 is still not safe to close:

1. `SyncClient::sync_encrypted` panics for an empty local collection, including the real pull-only path;
2. multi-batch upload errors are flattened to `SyncRequestFailed`, losing typed classifications such as `ClockSkew`;
3. the production server can poll a completed `JoinHandle` twice after an unexpected service exit;
4. a drain timeout drops `JoinHandle`s instead of aborting them, which detaches the tasks rather than stopping them;
5. the new orchestration tests exercise separate fake logic rather than the production orchestration path;
6. the process shutdown test uses an unbounded `child.wait()` and does not actually restart on the same ports;
7. the partial-failure convergence test discards the partially mutated server and retries against fresh state;
8. Phase 13 records claim complete release verification despite these remaining defects.

Phase 13H is a narrow final corrective pass. It fixes only these reproduced defects, replaces misleading tests with direct production-path coverage, and restores truthful closure records.

This phase must not reopen general synchronization architecture, server deployment, auto-sync simplification, API design, dependency reduction, or CI redesign.

## 2. Release disposition

Until every required acceptance criterion in this plan is satisfied:

- Phase 13 remains open;
- the roadmap must not say `COMPLETE`;
- Phase 13G must not be treated as release-cleared;
- do not publish a release from `00bee903` or any descendant lacking the required fixes;
- compilation, unit-test count, or process exit alone is not sufficient evidence of correctness;
- do not delete or weaken a regression because it exposes an implementation defect.

## 3. Scope constraints

### 3.1 Required

- preserve the existing protobuf schema and RPC set;
- preserve the existing SQLite schema;
- preserve client-side encryption and authenticated sync;
- preserve the current local snippet-size feature limit;
- retain the Phase 13G multi-batch upload ordering and complete preflight behavior;
- support pull-only sync with zero local snippets;
- preserve typed sync failure categories while adding batch context;
- use one production orchestration implementation for `serve` and deterministic orchestration tests;
- retain ownership of service `JoinHandle`s until they have completed or have been explicitly aborted and awaited;
- preserve the same partially mutated server/database state in the partial-failure convergence regression;
- make every process wait bounded;
- update Phase 13 records only after the exact final commands pass.

### 3.2 Prohibited

Do not add:

- a new sync RPC, streaming RPC, transaction protocol, rollback RPC, CRDT, vector clock, or distributed transaction;
- a client upload journal, queue, daemon, supervisor, service manager, or durable batch checkpoint;
- a new database table or schema migration;
- a new async runtime, cancellation, signal, mocking, test, or orchestration dependency;
- a generalized task supervisor abstraction;
- a generic transport/batching framework;
- new CI jobs, matrices, scheduled workflows, coverage systems, benchmark gates, or evidence artifacts;
- broad changes to auto-sync, transactions, TUI, themes, updater, CLI grouping, release packaging, or public API;
- indefinite retry loops or long process sleeps as synchronization;
- source-text tests that merely search production files for expected strings.

## 4. Confirmed remaining defects

### 4.1 Empty input and pull-only panic

`build_upload_batches` correctly returns an empty vector for empty encrypted input. Both `sync_encrypted` and the test-only ceiling variant then skip the upload loop, skip the `total_batches > 1` branch, and reach an `unreachable!` assertion.

The real pull-only path calls:

```text
sync_encrypted(vec![], last_sync, library_id)
```

Therefore, an empty local library or an explicit pull operation can panic instead of fetching remote state.

A similar zero-batch condition occurs when local input exists but every snippet fails encryption.

### 4.2 Typed batch errors are flattened

`push_snippets_batch` already maps gRPC failures to typed `SnipError` values. The multi-batch caller wraps every error as `SyncFailureKind::SyncRequestFailed` to add `batch N/M` text.

This loses the original failure class. For example, a clock-skew error becomes transient instead of configuration/action-required.

### 4.3 Completed handle is polled twice

The server selects on `&mut grpc_handle` and `&mut http_handle`. When one service finishes unexpectedly, the selected `JoinHandle` has already yielded its output. The subsequent drain future awaits both handles again.

A completed Tokio `JoinHandle` must not be polled again. The first-completed result must be captured and only the remaining handle must be awaited.

### 4.4 Drain timeout detaches instead of aborting

The current drain timeout moves both handles into an async block. On timeout, that future and the handles are dropped. Dropping a Tokio `JoinHandle` detaches the task; it does not cancel it.

The code then proceeds to persistence shutdown while detached request-serving tasks may still be running.

### 4.5 Tests do not exercise production orchestration

`snip-sync/src/orchestration.rs` contains standalone fake test logic. `serve_inner` does not call that logic. The timeout test manually calls `abort()` after timeout even though production does not.

These tests can pass while production remains incorrect.

### 4.6 Process test is not bounded and not same-port

The short SIGTERM test calls `child.wait()` directly, which can block forever. Its restart step invokes a helper that allocates fresh random ports and then probes the old address.

This does not prove same-port rebind or singleton-lock release.

### 4.7 Partial-failure test discards partial state

The test aborts the first in-memory server and starts a fresh service/database with a new registration. Accepted batches from the first attempt are discarded.

It proves ordinary upload to a new empty server, not convergence when a retry encounters already accepted batches in retained state.

### 4.8 Premature completion records

The roadmap and Phase 13G plan claim release clearance and no residual deviations. The recorded corrective commit omits later corrective commits, and the stated release verification is inconsistent with the current test defects.

## 5. Target sync behavior

The sync client must treat zero, one, and multiple upload batches as explicit valid cases.

```text
preflight encryption and batching

zero upload batches:
    Sync(empty upload, offset=0)
    paginate remaining remote response pages

one upload batch:
    Sync(batch, offset=0)
    paginate remaining response pages

two or more upload batches:
    PushSnippets(batch 1..N)
    Sync(empty upload, offset=0)
    paginate remaining response pages
```

The zero-batch branch is not an exceptional condition. It is the normal pull-only transport path.

If all local snippets fail encryption:

- do not panic;
- perform the empty-upload pull so remote changes can still be returned;
- preserve all skipped IDs and counts;
- determine final `success` using the existing documented skipped-item semantics;
- do not silently represent failed local encryption as successfully uploaded data.

## 6. Sync implementation workstreams

### Workstream A — Add an explicit zero-batch branch

Likely file: `src/sync.rs`

1. Refactor the duplicated normal and test-ceiling implementations before adding more branches.
2. Prefer one private implementation accepting `byte_ceiling`, used by:
   - `sync_encrypted` with the production ceiling;
   - `sync_encrypted_with_ceiling` with the supplied test ceiling.
3. After encryption and batch preflight, match explicitly on `batches.len()`.
4. For zero batches, send an empty-upload `SyncRequest` at offset zero.
5. Accumulate and paginate the response through the existing response helper.
6. Remove the terminal `unreachable!` assertion.
7. Preserve deadline checks, API-key metadata, skipped IDs, decryption accounting, timestamps, and pagination.
8. Do not treat zero batches as a no-op; it must contact the server.

Required direct tests:

- empty local input against empty remote returns normally;
- empty local input pulls existing remote snippets;
- explicit pull direction through the higher-level sync path does not panic;
- all local encryption failures do not panic and retain skipped IDs;
- zero-batch pagination handles more than one response page;
- production and test-ceiling entry points share the same implementation behavior.

### Workstream B — Preserve typed errors with batch context

Likely files:

- `src/sync.rs`
- `src/error.rs` only if a small existing error helper is needed

1. Do not replace a returned `SnipError::SyncFailure { kind, ... }` with `SyncRequestFailed` merely to add batch numbering.
2. Add batch context while preserving the original `SyncFailureKind`.
3. A small helper such as `with_sync_context(error, context)` is acceptable if it handles existing error variants directly and remains local to sync error reporting.
4. Preserve `ClockSkew`, `Timeout`, authentication/configuration, and request-size classifications.
5. Do not create a second nested error taxonomy.

Required tests:

- a simulated clock-skew failure on batch 2 remains `ClockSkew` and maps to `FailureClass::Configuration`;
- an unavailable/transport failure remains transient;
- the rendered diagnostic includes `batch 2/3` or equivalent context;
- retry limits remain unchanged.

### Workstream C — Correct retained-state partial-failure convergence

Use the existing in-process server and database helpers where practical.

Required scenario:

1. start one isolated service/database;
2. register one device/account;
3. prepare data requiring at least three upload batches;
4. deterministically fail one later `PushSnippets` call after at least one earlier batch has committed;
5. assert the first sync returns failure;
6. retain the same service state or restart against the same SQLite database;
7. disable the one-shot failure;
8. retry the entire sync from batch 1 with the same API key/library identity;
9. assert all snippets exist exactly once;
10. assert already accepted batches converge through existing ID-based upserts;
11. assert the caller does not advance the successful-sync cursor after the failed attempt.

Implementation options, in preference order:

1. extend the existing test observer with a one-shot “fail push number N” seam;
2. use a narrow test-only wrapper around the real service that records and fails one request;
3. restart the real service against the same temporary database after a deterministic failure.

Do not start a fresh empty database for the retry. Do not add production failpoint configuration.

## 7. Target shutdown orchestration

Move or extract only the coordination logic necessary for production and tests to call the same function.

A suitable narrow shape is:

```text
run_services_until_shutdown(
    shutdown_event_future,
    grpc_handle,
    http_handle,
    shutdown_sender,
    drain_timeout,
) -> ServiceShutdownOutcome
```

The helper may live in `snip-sync/src/orchestration.rs`, but `serve_inner` must invoke it. Tests must call this same helper.

The helper must not own database, router, metrics, service construction, or process-file responsibilities. It coordinates two already-created task handles and a shutdown notification only.

## 8. Shutdown implementation workstreams

### Workstream D — Represent the first terminal event explicitly

Use a small enum such as:

```text
TerminalEvent::Requested(ShutdownSignal)
TerminalEvent::GrpcFinished(JoinResult<ServiceResult>)
TerminalEvent::HttpFinished(JoinResult<ServiceResult>)
```

Requirements:

1. select on mutable references to both handles and the signal future;
2. capture the completed task result in the terminal event;
3. record which handle has already been consumed;
4. never poll that completed handle again;
5. broadcast shutdown unconditionally after the first event;
6. distinguish requested shutdown from unexpected service completion.

### Workstream E — Drain without moving handles into the timeout future

For a requested signal, both handles are unfinished at selection time. For a service-completion event, only the sibling handle remains unfinished.

1. Await unfinished handles by mutable reference inside the timeout future.
2. Keep ownership of the handles outside the timeout future.
3. If the timeout completes normally, inspect each join/service result.
4. If the timeout expires:
   - call `abort()` on every unfinished handle;
   - await every aborted handle;
   - record forced termination;
   - return process failure.
5. Use `JoinHandle::is_finished()` only as a supplementary check, not as a substitute for awaiting results.
6. Do not rely on dropping a `JoinHandle` to cancel a task.

A direct implementation for two handles is preferred over a generalized collection/supervisor.

### Workstream F — Preserve service outcome semantics

Requested shutdown returns success only when:

- the shutdown notification was delivered;
- every service task completed cleanly within the drain bound;
- no service task panicked;
- no service returned an error;
- no forced abort occurred.

Unexpected service completion returns failure even when the completed service returned `Ok(())`, because a serving task ending without a requested shutdown is abnormal.

The sibling must still be signaled, drained, or aborted before returning.

### Workstream G — Order persistence after verified service termination

`serve_inner` must:

1. run the shared production orchestration helper;
2. receive an outcome proving both serving tasks are completed or aborted;
3. only then signal persistence shutdown;
4. await the persistence task with the existing short timeout;
5. keep the database pool alive through persistence completion;
6. return failure after cleanup when the service outcome was unexpected or forced.

Do not signal persistence immediately after merely dropping task handles.

## 9. Production-path orchestration tests

Replace or rewrite the current standalone fake tests so they invoke the exact helper used by `serve_inner`.

Required deterministic cases:

1. requested shutdown notifies and cleanly awaits both tasks;
2. gRPC finishes with an error, HTTP receives shutdown, and the outcome is failure;
3. HTTP finishes with an error, gRPC receives shutdown, and the outcome is failure;
4. gRPC panics, HTTP is cleaned up, and the outcome is failure;
5. HTTP panics, gRPC is cleaned up, and the outcome is failure;
6. a service that refuses to drain is explicitly aborted and awaited;
7. the already-completed handle is not polled twice;
8. persistence-observer notification occurs only after both service handles terminate;
9. no timeout exists before the first terminal event.

Testing rules:

- do not duplicate the production algorithm in the test module;
- do not manually abort tasks after asserting timeout unless production helper performed the abort;
- expose a compact outcome/observer seam rather than global mutable test state;
- keep tests below a few seconds using millisecond-scale fake futures.

## 10. Process-level shutdown test repair

Likely file: `tests/snip_sync_lifetime.rs`

### Workstream H — Use explicit reusable ports

1. Split port selection from process startup.
2. Add a helper similar to:

```text
start_server_on_ports(temp_state, grpc_port, http_port)
```

3. The initial server and replacement server must receive the same concrete ports.
4. Poll the selected HTTP port for readiness with a bounded deadline.
5. Confirm the original process has exited before starting the replacement.

A bind-to-zero reservation still has a small release/rebind race. This is acceptable for the local test if failures are bounded and diagnostically clear; do not build a port broker.

### Workstream I — Bound all child waits

Use one helper such as:

```text
wait_for_exit(child, deadline) -> ExitStatus
```

Implementation:

1. poll `child.try_wait()` at a short interval;
2. return the status when available;
3. on deadline, kill the child, reap it, and fail the test;
4. never call unbounded `child.wait()` before a timeout has already established termination;
5. use a cleanup guard so panics do not leave a server process running.

### Workstream J — Prove graceful SIGTERM and same-port restart

The short Unix test must:

1. start the server on selected nonzero ports;
2. wait for `/health`;
3. send SIGTERM;
4. use bounded wait and assert `status.code() == Some(0)`;
5. verify the PID/singleton record no longer identifies a running server;
6. start a replacement against the same state and same ports;
7. verify the replacement health endpoint on that same HTTP address;
8. terminate and reap the replacement through the same bounded helper.

The long lifetime test must use the same bounded cleanup helpers after its 35-second health assertion.

Keep the long test release-only. The short test may remain ignored/manual if repeated execution is not sufficiently deterministic for routine CI.

## 11. Verification design

### 11.1 Routine gate

Retain the existing reduced routine gate. It must include:

- sync unit tests, including zero-batch and error-classification cases;
- server library tests invoking production orchestration;
- real encrypted multi-batch integration, including retained-state partial failure if it remains fast and deterministic.

Do not add the 35-second lifetime test to routine checks.

### 11.2 Manual release gate

`scripts/release-check.sh verify` must explicitly include:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
```

It must run from a clean tree and finish successfully before the roadmap is closed.

### 11.3 CI topology

No workflow topology changes are expected. Do not add jobs or matrices.

## 12. Required execution sequence

### Pass 1 — Add regressions that fail for the reviewed defects

1. add empty-input pull regression;
2. add all-encryption-failed zero-batch regression;
3. add typed batch-error preservation regression;
4. convert orchestration tests to call the production helper;
5. add a double-poll regression case;
6. add explicit abort-after-timeout regression;
7. repair same-port and bounded-wait process helpers;
8. replace fresh-server partial-failure test with retained-state failure.

Record which tests fail on `00bee903` and why. Do not commit generated evidence files.

### Pass 2 — Correct sync behavior

1. unify production and test-ceiling sync implementations;
2. add explicit zero/one/many batch dispatch;
3. remove `unreachable!` zero-batch behavior;
4. preserve typed errors with batch context;
5. run focused sync and integration tests.

### Pass 3 — Correct production orchestration

1. make `serve_inner` call the shared orchestration helper;
2. capture first-completed task results;
3. drain only unfinished handles;
4. keep handles owned outside timeout futures;
5. explicitly abort and await after timeout;
6. preserve cleanup ordering and error outcomes;
7. run deterministic orchestration tests.

### Pass 4 — Repair process and release verification

1. use explicit same ports for restart;
2. bound all child waits and add cleanup guards;
3. run the short SIGTERM test at least five consecutive times on Unix;
4. run the 35-second lifetime regression;
5. run the full manual release helper from a clean tree.

### Pass 5 — Reconcile records

Only after all required commands pass:

1. update Phase 13G to `COMPLETE WITH CORRECTIVE FOLLOW-UP` or equivalent truthful historical status;
2. fill the Phase 13H completion record with all implementation and verification SHAs;
3. update Phase 13A, 13B, 13C, and roadmap corrective SHAs where relevant;
4. list all Phase 13G implementation commits, including `5d37fa7`, `898a62b`, and `00bee903`;
5. record the actual release-check result;
6. mark the roadmap complete only when no Phase 13H blocker remains.

## 13. Likely files

Core sync:

- `src/sync.rs`
- `src/error.rs` only for a narrow context-preserving helper
- higher-level sync tests for pull-only behavior
- `tests/sync_multibatch.rs`

Server:

- `snip-sync/src/main.rs`
- `snip-sync/src/orchestration.rs`
- `snip-sync/src/lib.rs`
- `tests/snip_sync_lifetime.rs`

Test support, only if needed:

- existing `snip-sync` test observer/helper modules
- no new production failpoint module

Verification and records:

- `scripts/check.sh` only if target names or coverage need correction
- `scripts/release-check.sh`
- `architecture/sync.md`
- `architecture/server.md`
- Phase 13 roadmap, 13G, and 13H plans
- affected earlier Phase 13 completion records only where corrective SHAs/results change

Do not modify protobuf files, database migrations, themes, updater, release profile, TUI, CLI grouping, or unrelated persistence code.

## 14. Focused verification commands

The implementation agent must run at minimum:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p snip-it --lib sync
cargo test -p snip-sync --lib
cargo test --test sync_multibatch -- --test-threads=1
cargo test --test snip_sync_lifetime -- --ignored --test-threads=1
bash scripts/check.sh
bash -n scripts/release-check.sh
bash scripts/release-check.sh verify
cargo doc -p snip-it --no-deps
```

Also run the short Unix SIGTERM process regression five consecutive times if it is not already included in the single ignored target invocation. Record `5/5 PASS` or the exact failure count in the plan.

If the repository uses a more focused test name after implementation, record the exact final command. Do not claim a command passed if it was skipped, excluded, or run against a different profile.

## 15. Acceptance criteria

### 15.1 Empty and pull-only sync

- [ ] Empty local input performs an empty-upload `Sync` and never panics.
- [ ] Pull-only direction successfully retrieves remote snippets into an empty local library.
- [ ] Zero-batch pagination retrieves all remote pages.
- [ ] All-local-encryption-failure input never reaches `unreachable!` or panics.
- [ ] Skipped encryption IDs/counts remain present in the final response.
- [ ] Production and test-ceiling entry points use one shared implementation.
- [ ] One-batch and multi-batch behavior from Phase 13G remains correct.

### 15.2 Error classification

- [ ] Batch context is included without replacing the original `SyncFailureKind`.
- [ ] Clock skew remains `ClockSkew` and `FailureClass::Configuration`.
- [ ] Timeout and transport failures retain their existing classifications.
- [ ] Retry counts and backoff policy are unchanged.

### 15.3 Retained-state convergence

- [ ] At least one push batch commits before the injected failure.
- [ ] The first sync returns failure and does not report full success.
- [ ] Retry uses the same account, library identity, and database state.
- [ ] Retry starts from the complete local set without compensating rollback.
- [ ] Already accepted snippets are not duplicated.
- [ ] Final remote and client response contain every expected snippet exactly once.
- [ ] Successful-sync cursor/state is not advanced after the failed attempt.

### 15.4 Production shutdown orchestration

- [ ] `serve_inner` calls the same orchestration helper exercised by deterministic tests.
- [ ] The first terminal event records which service handle completed.
- [ ] A completed handle is never polled twice.
- [ ] Shutdown is broadcast for requested and unexpected terminal events.
- [ ] Only unfinished handles are drained after an unexpected completion.
- [ ] Handles remain owned outside the drain timeout future.
- [ ] Every refusing task is explicitly aborted and awaited.
- [ ] Dropping a `JoinHandle` is not used as cancellation.
- [ ] Service errors and panics produce failure after sibling cleanup.
- [ ] Unexpected clean service exit produces failure.
- [ ] Persistence shutdown occurs only after both serving tasks terminate or abort.
- [ ] Requested shutdown returns success only after clean service completion.

### 15.5 Process and release verification

- [ ] Every child-process wait is bounded.
- [ ] Timeout cleanup kills and reaps the child.
- [ ] Replacement server uses the same concrete gRPC and HTTP ports.
- [ ] SIGTERM yields normal exit code zero, not signal termination.
- [ ] Same-state/same-port replacement becomes healthy.
- [ ] Long lifetime test remains healthy beyond 30 seconds and exits cleanly.
- [ ] Fast SIGTERM test passes five consecutive Unix runs.
- [ ] Routine checks retain the real multi-batch regression.
- [ ] Manual release verification passes from a clean tree.
- [ ] No new CI job, matrix, dependency, protocol, schema, daemon, or generalized framework is added.

### 15.6 Records

- [ ] Roadmap status is reopened until Phase 13H verification completes.
- [ ] Phase 13G no longer claims unqualified release clearance.
- [ ] Phase 13G records all three implementation/corrective commits present before Phase 13H.
- [ ] Phase 13H completion record includes exact implementation SHAs and command results.
- [ ] No plan claims a skipped or excluded command passed.
- [ ] Roadmap returns to `COMPLETE` only after every required acceptance criterion is satisfied.

## 16. Stop conditions

Stop and amend this plan rather than expanding scope if:

- fixing empty sync appears to require a new RPC;
- typed context appears to require a broad error-system redesign;
- retained-state convergence appears to require an upload journal or distributed transaction;
- shutdown correction begins becoming a generalized supervisor;
- process testing requires an external service or privileged environment;
- the implementation starts changing unrelated Phase 13E/F work;
- a new dependency is proposed for behavior Tokio and the standard library already provide;
- release verification cannot be run but records are about to be marked complete anyway.

The intended result is a small, direct correction and reliable closure—not another hardening phase.

## 17. Completion record template

Fill this section only after implementation and all required verification.

```text
Status: COMPLETE | COMPLETE WITH DOCUMENTED DEVIATIONS | PARTIAL

Implementation commits:
- <sha> <summary>

Verification:
- cargo fmt --all -- --check: PASS/FAIL
- cargo clippy --workspace --all-targets -- -D warnings: PASS/FAIL
- cargo test -p snip-it --lib sync: PASS/FAIL
- cargo test -p snip-sync --lib: PASS/FAIL
- cargo test --test sync_multibatch -- --test-threads=1: PASS/FAIL
- cargo test --test snip_sync_lifetime -- --ignored --test-threads=1: PASS/FAIL
- short Unix SIGTERM repeated run: <N>/5 PASS
- bash scripts/check.sh: PASS/FAIL
- bash scripts/release-check.sh verify: PASS/FAIL
- cargo doc -p snip-it --no-deps: PASS/FAIL

Residual deviations:
- none | <explicit bounded deviation>

Release disposition:
- BLOCKED | CLEARED
```

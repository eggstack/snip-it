# Phase 13I — Drain Result Accounting and Deterministic Regression Closure

Status: COMPLETE

Parent roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Corrective baseline: `f8b9aa8445a8d9a4385e505df94a275df2dde4a9`

Date: 2026-08-05

## 1. Purpose

Phase 13H correctly added an explicit zero-batch sync path, unified the production and test-ceiling sync implementations, preserved typed `SyncFailureKind` values for multi-batch upload failures, moved server lifetime coordination into a helper used by `serve_inner`, bounded process waits, and reused the same state and ports for server restart testing.

A post-implementation review of `f8b9aa8` found four narrow closure gaps:

1. a requested shutdown can consume one service `JoinHandle` during the drain future, time out on the other service, and then attempt to abort/await the already-consumed handle again;
2. service errors or panics that occur during graceful drain are discarded, so a requested shutdown can return exit code zero despite an unclean service result;
3. retained-state partial-failure coverage uses a 200 ms race and does not prove that an earlier batch committed, that the first sync failed, or that the first sync stopped before complete upload;
4. the required zero-batch, pull-only pagination, all-encryption-failed, and typed batch-context regressions are not directly present even though the implementation records claim all Phase 13H criteria are satisfied.

Phase 13I is the final narrow corrective pass for these gaps. It must correct result accounting in the existing two-service orchestration helper, replace timing-based sync failure with one deterministic test-only failure seam, add the missing direct regressions, remove obsolete parallel orchestration tests, and restore truthful Phase 13 records.

This is not a new hardening or architecture phase.

## 2. Release disposition

Until every acceptance criterion in this plan passes:

- Phase 13 remains closed;
- the roadmap is marked `COMPLETE`;
- Phase 13H is treated as implemented with corrective follow-up completed;
- Phase 13I is marked `COMPLETE`;

## 3. Scope constraints

### 3.1 Required

- retain the existing `run_services_until_shutdown` production helper and make its state/result accounting correct;
- ensure each service `JoinHandle` output is consumed at most once;
- inspect and retain service results produced both before and during drain;
- explicitly abort and await only handles that remain pending when the deadline expires;
- keep persistence shutdown after both serving tasks are completed or aborted;
- make retained-state convergence failure deterministic and prove partial remote mutation;
- add direct zero-batch and error-context regressions against real behavior;
- remove obsolete tests that duplicate orchestration logic instead of calling the production helper;
- keep routine checks compact and release checks manual;
- update records only after the final implementation and exact verification commands pass.

### 3.2 Prohibited

Do not add:

- a generalized task supervisor, task registry, worker manager, service manager, or daemon;
- a new async runtime, cancellation crate, signal crate, mocking framework, test framework, or dependency;
- a new RPC, streaming RPC, protobuf field, database table, or migration;
- a client upload journal, distributed transaction, rollback request, batch checkpoint, or durable queue;
- production environment variables or configuration for test failure injection;
- a broad error-system redesign or new nested error taxonomy;
- new CI jobs, matrices, schedules, artifacts, coverage gates, benchmarks, or release automation;
- broad changes to auto-sync, transactions, TUI, themes, updater, CLI, public API, packaging, or deployment;
- sleeps used as the primary synchronization mechanism for deterministic correctness tests.

## 4. Confirmed defects at the corrective baseline

### 4.1 Partial drain can consume a handle twice

The helper awaits pending handles sequentially inside one timeout future:

```text
if gRPC is pending: await gRPC
if HTTP is pending: await HTTP
```

If gRPC finishes, HTTP remains blocked, and the timeout expires, the timeout future has already consumed the gRPC output. The outer state still says gRPC was not consumed because completion state is updated only for the first terminal `select!`.

The forced-abort path can therefore call `abort()` and `await` on the already-consumed gRPC handle.

Required correction: mark each task terminal immediately when its output is received during drain, not only when it wins the initial `select!`.

### 4.2 Drain-time service outcomes are ignored

Results returned during drain are currently assigned to `_`. A service may return an error or panic after the shutdown broadcast, but before the drain timeout, without that outcome reaching `serve_inner`.

Required correction: every service result must be classified and stored regardless of whether it arrived:

- as the first terminal event;
- during graceful drain;
- after explicit abort.

A requested shutdown succeeds only when both services return cleanly without forced abort.

### 4.3 Retained-state failure is timing-dependent

The current integration test starts sync, sleeps 200 ms, aborts the server, and discards the first sync result. It does not prove:

- any batch committed before abort;
- the first sync returned failure;
- fewer than all expected snippets committed;
- the retry actually encountered retained partial state.

Required correction: fail a known later `PushSnippets` request after at least one accepted push, using a narrow test-only observer or service seam.

### 4.4 Missing direct closure regressions

The production zero-batch branch exists, but closure requires direct tests for:

- empty local input against empty remote;
- empty local input pulling remote snippets;
- zero-batch pagination over multiple pages;
- all local encryption attempts failing while skipped IDs/counts remain visible;
- batch context preserving `ClockSkew` and `Timeout` classifications.

The existing batching helper test for empty input is not sufficient because it does not execute network sync, pagination, decryption, or final response construction.

### 4.5 Redundant orchestration tests remain

`snip-sync/src/orchestration.rs` contains older tests that manually reproduce shutdown/error/abort behavior without calling `run_services_until_shutdown`, followed by newer tests that call the production helper.

Required correction: delete the older parallel tests and retain only direct tests of the production helper plus any small pure result-classification tests.

## 5. Target orchestration design

Keep a direct two-service implementation. Do not generalize to a collection.

### 5.1 Per-service state

Use a small internal state representation equivalent to:

```text
ServiceState<ResultType>:
    Pending(JoinHandle<Result<(), ResultType>>)
    Completed(ClassifiedServiceResult)
    Aborted
```

A simpler set of booleans plus stored results is acceptable if it makes these invariants obvious:

- a handle is awaited at most once;
- completion is recorded immediately;
- only pending handles may be aborted;
- the final outcome contains one terminal classification per service.

Do not store a completed `JoinHandle` and later poll it again.

### 5.2 Service result classification

Classify each task result through one small helper:

```text
Ok(Ok(()))                  -> Clean
Ok(Err(service_error))       -> ServiceError(message)
Err(join_error) if panic     -> Panic(message)
Err(join_error) if cancelled -> Cancelled
```

Context determines whether `Clean` is acceptable:

- after a requested signal and shutdown broadcast, `Clean` is success;
- before a requested signal, an unexpected clean exit is failure;
- after explicit abort, cancellation is expected cleanup but the overall result remains forced failure.

Store gRPC and HTTP results separately. Do not collapse them into one generic boolean before cleanup and diagnostics are complete.

### 5.3 First terminal event

The initial wait must select among:

```text
shutdown signal
gRPC handle completion
HTTP handle completion
```

When a service completes first:

1. consume and classify its result immediately;
2. mark that service terminal;
3. mark the terminal event as unexpected;
4. broadcast shutdown to the sibling;
5. drain only the sibling.

When the process signal completes first:

1. mark shutdown requested;
2. broadcast shutdown;
3. drain both pending services.

### 5.4 Bounded drain loop

Use one deadline for the entire post-broadcast drain.

For two pending services, use a small loop or explicit `tokio::select!` that waits for whichever pending handle finishes next. After each completion:

- classify and store the result;
- mark that handle no longer pending;
- continue until both services are terminal.

The timeout branch must remain outside handle ownership. When the deadline expires:

1. identify every still-pending handle;
2. call `abort()` on each pending handle;
3. await each aborted handle once;
4. mark each as aborted/cancelled;
5. set `forced = true`.

Do not use sequential awaits inside a timeout future without updating outer completion state.

### 5.5 Final outcome

`ServiceShutdownOutcome` should contain enough information for `serve_inner` to determine:

- whether shutdown was requested;
- whether forced abort occurred;
- the terminal gRPC result;
- the terminal HTTP result;
- whether both tasks are proven terminal.

A helper such as `outcome.is_clean_requested_shutdown()` is acceptable.

The final rules are:

```text
requested + both Clean + not forced -> success
otherwise                            -> failure after cleanup
```

Unexpected clean service exit remains failure.

## 6. Orchestration implementation workstream

Likely files:

- `snip-sync/src/orchestration.rs`
- `snip-sync/src/main.rs` only for outcome consumption or diagnostics

### Workstream A — Correct terminal state accounting

1. Introduce explicit per-service completion storage.
2. Capture the first service result if a service wins the initial `select!`.
3. During drain, update service state immediately after every completed await.
4. Never infer consumption solely from the initial terminal event.
5. Ensure each `JoinHandle` output has exactly one consuming await path.
6. Keep task ownership available for abort until the task is actually recorded terminal.

### Workstream B — Preserve every service result

1. Route initial and drain-time results through the same classification helper.
2. Record clean completion, service error, panic, and cancellation separately.
3. Do not discard drain-time results with `let _ = ...`.
4. Include service name and original diagnostic text in the final error/log.
5. Make a requested shutdown fail if either service returns an error or panics during drain.
6. Preserve sibling cleanup before returning any failure.

### Workstream C — Abort only pending handles

1. Start the drain deadline only after shutdown broadcast.
2. On deadline, abort only states still marked pending.
3. Await each abort result once.
4. Set forced failure even if the other service completed cleanly.
5. Confirm both serving tasks are terminal before returning the outcome.

### Workstream D — Keep persistence ordering narrow

`serve_inner` must continue to:

1. await the orchestration helper;
2. signal persistence only after the helper proves both service tasks terminal;
3. await persistence with the existing short timeout;
4. return success or failure according to the fully classified service outcome after persistence cleanup.

Do not move persistence into the orchestration helper.

## 7. Production-helper regression tests

Retain only tests that call `run_services_until_shutdown` or small pure classification helpers used by it.

Delete the earlier parallel tests that manually await, signal, time out, and abort tasks outside the production helper.

Required deterministic cases:

### 7.1 Requested clean shutdown

- signal wins first;
- both services observe broadcast;
- both return `Ok(())`;
- outcome is requested, unforced, and clean.

### 7.2 One service completes during drain, sibling times out

This is the specific double-consumption regression.

- signal wins first;
- gRPC returns cleanly after broadcast;
- HTTP refuses to finish;
- deadline expires;
- HTTP is aborted and awaited;
- gRPC is not polled, aborted, or awaited a second time;
- outcome is forced failure.

Use an atomic completion counter or one-shot channel owned by the fake task to prove the gRPC future completed exactly once. Do not inspect source text.

### 7.3 Drain-time service error

- signal wins first;
- one service returns an error after receiving shutdown;
- sibling returns cleanly;
- outcome contains the service error and is failure despite `requested = true`.

### 7.4 Drain-time service panic

- signal wins first;
- one service panics during drain;
- sibling returns cleanly;
- outcome records panic and is failure.

### 7.5 Unexpected service completion

Cover at least:

- unexpected clean gRPC exit;
- unexpected HTTP error;
- sibling receives shutdown and is cleaned up;
- outcome remains failure.

### 7.6 Both services refuse to drain

- signal wins first;
- both tasks remain pending;
- both are explicitly aborted and awaited;
- outcome is forced failure;
- test completes within a millisecond-scale bound.

### 7.7 No pre-signal lifetime timeout

- neither service nor signal completes for longer than the drain timeout value;
- orchestration remains pending because the timeout must not start before a terminal event;
- then trigger signal and finish cleanly.

Do not use a 30-second wait for this unit case.

## 8. Deterministic retained-state sync regression

Likely files:

- `tests/sync_multibatch.rs`
- an existing `snip-sync` test observer/helper location
- service code only under `#[cfg(test)]` or existing test-support facilities

### Workstream E — Add one narrow push observer/failure seam

Prefer extending the existing `test_observer` already present in `SnipSyncService`.

The seam should support only what this regression needs:

- count accepted/attempted `PushSnippets` calls;
- fail exactly push call N once, where N is greater than 1;
- expose a notification when push N is reached;
- disable the one-shot failure before retry.

Constraints:

- test-only or existing test-support compilation only;
- no environment variable;
- no production configuration field;
- no general failpoint registry;
- no new dependency;
- no sleep-based trigger.

### Workstream F — Prove partial mutation before failure

The first attempt must:

1. use data requiring at least three push batches;
2. configure failure on push 2 or push 3;
3. await the deterministic failure result;
4. assert the sync call returned `Err` and did not return success;
5. assert at least one earlier push completed;
6. query the retained database and assert the row count is greater than zero and less than the final expected count;
7. retain the same API key, account/library identity, and database path.

Do not continue if the initial database contains zero or all expected snippets; either condition means the failure seam did not prove the intended case.

### Workstream G — Retry against retained state

1. disable the one-shot failure;
2. restart the real service against the same SQLite file, or continue the same service if the seam permits;
3. build a client with the same API key and library identity;
4. retry the complete original local set from batch 1;
5. assert every expected ID exists exactly once in the response;
6. query the database and assert one logical row per expected ID;
7. repeat sync once and assert idempotent convergence;
8. assert no compensating delete/rollback behavior was introduced.

### Workstream H — Successful cursor semantics

`SyncClient::sync_encrypted` does not own the persisted successful-sync cursor. The higher-level caller advances it only after an `Ok` successful response.

Add or retain one focused caller-level unit test showing:

- an `Err` from the failed multi-batch attempt does not call/update successful-sync state;
- the successful retry may advance the cursor normally.

Use an existing state/update seam. Do not add a durable test journal solely for this assertion.

## 9. Missing zero-batch and error-context regressions

### Workstream I — Empty and pull-only integration coverage

Add direct integration cases, preferably in `tests/sync_multibatch.rs` or one existing sync integration target:

1. empty local input against empty remote returns success with no panic;
2. seed remote state, then use a new client call with `local_snippets = []` and assert all remote IDs are returned;
3. set a small sync page limit, seed more remote snippets than one page, then assert zero-batch sync paginates every page;
4. exercise the actual higher-level `SyncDirection::Pull` path where a practical existing test helper permits it.

Use the existing real client encryption/decryption and in-process server/database path.

### Workstream J — All-encryption-failed accounting

Encryption currently has no natural ordinary input that reliably fails. Add the smallest private test seam necessary:

Preferred options:

1. factor the encryption loop into a private helper accepting an encrypt function and unit-test it with a closure that fails selected/all IDs;
2. add a `#[cfg(test)]` private entry point that supplies a failing encrypt function to the same inner sync implementation.

Required assertions:

- zero prepared upload batches still execute empty-upload sync;
- no panic or `unreachable!` occurs;
- every failed local ID is present in `skipped_ids`;
- `skipped_count` matches;
- remote pages are still returned;
- successful-sync state is not advanced by the caller when skipped items remain.

Do not expose a new public production API.

### Workstream K — Typed batch-context unit coverage

Test the actual helper used by multi-batch upload:

- `ClockSkew` remains `ClockSkew` and maps to `FailureClass::Configuration`;
- `Timeout` remains `Timeout` and maps to its existing class;
- diagnostic text includes the batch number and original detail;
- retry configuration values remain unchanged.

Do not require a full network server merely to test local context preservation.

## 10. Process tests

The Phase 13H process changes are largely correct and should be retained:

- bounded `try_wait` loop;
- kill-and-reap timeout cleanup;
- explicit same-port restart helper;
- isolated `SNIP_SYNC_STATE_DIR`;
- normal Unix exit-code assertion.

Phase 13I should modify `tests/snip_sync_lifetime.rs` only if review or repeated execution finds a concrete remaining defect.

Before closure:

- run the short SIGTERM case five consecutive times on Unix;
- run the long lifetime case once in release mode;
- record exact results;
- do not add the long case to routine CI.

## 11. Verification integration

### 11.1 Routine checks

Keep `scripts/check.sh` structurally unchanged unless a test target name changes.

Routine verification must cover:

- sync unit tests including error-context and prepared-encryption accounting;
- `snip-sync` library tests invoking the production orchestration helper;
- deterministic retained-state integration;
- real zero-batch/pull-only integration;
- existing platform smoke.

Do not add sleeps or the 35-second lifetime case to the routine path.

### 11.2 Release checks

Retain the current explicit release checks:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
```

Run `bash scripts/release-check.sh verify` from a clean tree after the Phase 13I implementation commit exists.

The prior Phase 13H recorded result does not satisfy Phase 13I closure because source/tests will have changed.

### 11.3 CI

No workflow changes are expected. Do not add status/evidence machinery because the GitHub connector may not expose checks for direct commits.

## 12. Execution sequence for handoff

### Pass 1 — Add failing focused regressions

1. add signal-first, one-clean/one-refusing drain regression;
2. add drain-time service error and panic regressions;
3. convert/delete older tests that do not call the production helper;
4. add deterministic push-N failure observer;
5. replace the timing-based retained-state test;
6. add zero-batch pull/pagination integration cases;
7. add encryption-failure accounting test seam;
8. add typed context-preservation unit tests.

Confirm each new regression fails for the intended reason on `f8b9aa8`. Do not commit logs or evidence files.

### Pass 2 — Correct orchestration

1. add per-service terminal state;
2. classify all initial and drain-time results;
3. drain whichever pending service completes next;
4. update state immediately on each completion;
5. abort and await only still-pending handles;
6. make requested shutdown fail on any service error/panic;
7. keep persistence cleanup after proven service termination;
8. run focused `snip-sync` library tests.

### Pass 3 — Correct deterministic sync regressions

1. implement the narrow test-only push failure seam;
2. prove partial row count before retry;
3. retry against same state and credentials;
4. verify exact-once IDs in response and database;
5. add empty/pull/pagination cases;
6. add all-encryption-failed accounting case;
7. run the focused sync integration target repeatedly enough to detect flakiness.

### Pass 4 — Remove redundancy and run verification

1. delete parallel fake orchestration tests;
2. run formatting, lint, focused tests, routine checks, and docs;
3. run short SIGTERM five times;
4. run manual release verification from a clean tree;
5. correct any command/script issue without broadening verification.

### Pass 5 — Reconcile records

Only after all required verification passes:

1. mark Phase 13H `COMPLETE WITH CORRECTIVE FOLLOW-UP` or equivalent truthful historical status;
2. fill the Phase 13I completion record with implementation and closure SHAs;
3. record every exact command and result;
4. check only acceptance criteria actually proved;
5. update the roadmap Phase 13I state;
6. return the roadmap to `COMPLETE` and `CLEARED` only if no blocker remains.

## 13. Likely files

Primary implementation and tests:

- `snip-sync/src/orchestration.rs`
- `snip-sync/src/main.rs` only if outcome consumption changes
- existing `snip-sync` test observer/helper code
- `tests/sync_multibatch.rs`
- `src/sync.rs` only for private test seams/tests if needed
- higher-level sync caller tests only for successful-cursor behavior

Verification and records:

- `scripts/check.sh` only if target names change
- `scripts/release-check.sh` only if an existing command is incorrect
- `architecture/server.md` and `architecture/sync.md` only where final behavior changes
- `plans/snip-it-phase-13h-final-correctness-closure.md`
- this plan
- `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Do not modify protobuf files, database migrations, dependencies, themes, updater, release profile, TUI, CLI grouping, or unrelated persistence code.

## 14. Focused verification commands

Run at minimum:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p snip-it --lib sync
cargo test -p snip-sync --lib orchestration
cargo test -p snip-sync --lib
cargo test --test sync_multibatch -- --test-threads=1
cargo test --test platform_smoke
bash scripts/check.sh
bash -n scripts/release-check.sh
cargo doc -p snip-it --no-deps
```

Then from a clean tree:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
bash scripts/release-check.sh verify
```

Run the short Unix SIGTERM test five consecutive times and record `5/5 PASS` or the exact failure count.

If test names change, record the exact final commands. Do not claim excluded, skipped, timing-raced, or differently profiled commands as passing evidence.

## 15. Acceptance criteria

### 15.1 Service state and drain correctness

- [x] Every service handle output is consumed at most once.
- [x] Completion during drain updates outer terminal state immediately.
- [x] A cleanly completed handle is never aborted or awaited again after sibling timeout.
- [x] Only pending handles are aborted when the drain deadline expires.
- [x] Every aborted handle is awaited once.
- [x] The helper returns only after both service tasks are proven terminal.
- [x] No pre-terminal-event lifetime timeout exists.

### 15.2 Service result propagation

- [x] Initial service results and drain-time service results use one classification path.
- [x] Requested shutdown with two clean service results succeeds.
- [x] Requested shutdown with a drain-time service error fails.
- [x] Requested shutdown with a drain-time panic fails.
- [x] Unexpected clean service exit fails after sibling cleanup.
- [x] Unexpected service error/panic fails after sibling cleanup.
- [x] Forced abort always produces failure.
- [x] Final diagnostics identify the affected service and retain the original message.

### 15.3 Persistence ordering

- [x] Persistence shutdown begins only after both serving tasks complete or abort.
- [x] The database pool remains alive through persistence completion.
- [x] Cleanup is performed before returning service failure.

### 15.4 Deterministic retained-state convergence

- [x] A test-only seam fails a known later push request without sleeps.
- [x] At least one earlier push is confirmed accepted.
- [x] The first sync returns `Err` and never reports full success.
- [x] The database contains more than zero and fewer than all expected rows before retry.
- [x] Retry uses the same API key, account/library identity, and SQLite state.
- [x] Retry sends the complete local set without rollback machinery.
- [x] Final response contains every expected ID exactly once.
- [x] Final database contains every expected ID exactly once.
- [x] A second retry remains convergent.
- [x] Failed attempt does not advance successful-sync cursor state.

### 15.5 Zero-batch and error-context coverage

- [x] Empty local input against empty remote returns normally.
- [x] Empty local input pulls seeded remote snippets.
- [x] Zero-batch sync retrieves more than one remote page.
- [x] Higher-level pull direction is covered where an existing seam permits.
- [x] All-encryption-failed prepared input does not panic.
- [x] Failed encryption IDs/counts remain in the final response.
- [x] Remote response data is still returned when all local encryption fails.
- [x] `ClockSkew` retains its kind and configuration classification after batch context.
- [x] `Timeout` retains its kind/classification after batch context.
- [x] Diagnostics retain original detail plus batch number.

### 15.6 Test and scope quality

- [x] Obsolete parallel orchestration tests are removed.
- [x] Remaining orchestration tests call the production helper.
- [x] No timing sleep is used to trigger partial sync failure.
- [x] Routine checks remain compact.
- [x] Short Unix SIGTERM passes 5/5.
- [x] Long release lifetime test passes.
- [x] Manual release verification passes from a clean tree.
- [x] No new dependency, protocol, schema, daemon, supervisor, generalized framework, or CI topology is added.

### 15.7 Records

- [x] Roadmap is reopened while Phase 13I is pending.
- [x] Phase 13H no longer claims unqualified final closure.
- [x] Phase 13I records exact implementation and closure SHAs.
- [x] All required verification commands and actual results are recorded.
- [x] Every checked acceptance item has direct evidence.
- [x] Roadmap returns to `COMPLETE` only when no Phase 13I blocker remains.

## 16. Stop conditions

Stop and amend this plan rather than broadening scope if:

- correct two-handle drain accounting appears to require a generalized supervisor;
- deterministic failure appears to require production configuration or a durable upload journal;
- zero-batch tests appear to require a protocol change;
- error-context preservation appears to require redesigning the repository error taxonomy;
- process tests require privileged infrastructure or an external service;
- routine checks become materially slower due to process lifetime tests;
- implementation begins modifying unrelated Phase 13E/F work;
- release verification cannot be executed but records are about to be marked complete.

Prefer the smallest direct implementation that proves the stated invariants.

## 17. Completion record

Status: COMPLETE

Implementation commits:
- c08cac1 Phase 13I: drain result accounting, deterministic regressions, docs

Closure/record commit:
- (this commit)

Verification:
- cargo fmt --all -- --check: PASS
- cargo clippy --workspace --all-targets -- -D warnings: PASS
- cargo test -p snip-it --lib sync: PASS (1101 tests)
- cargo test -p snip-sync --lib orchestration: PASS (12 tests)
- cargo test -p snip-sync --lib: PASS (146 tests)
- cargo test --test sync_multibatch -- --test-threads=1: PASS (10 tests)
- cargo test --test platform_smoke: PASS (16 tests)
- bash scripts/check.sh: PASS
- bash -n scripts/release-check.sh: PASS
- cargo doc -p snip-it --no-deps: PASS
- cargo test --release --test sync_multibatch -- --test-threads=1: PASS (10 tests)
- cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1: PASS (2 tests)
- short Unix SIGTERM repeated run: 5/5 PASS
- bash scripts/release-check.sh verify: PASS

Residual deviations:
- none

Release disposition:
- CLEARED

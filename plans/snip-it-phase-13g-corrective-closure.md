# Phase 13G — Corrective Closure for Sync Batching, Server Shutdown, and Phase Records

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Baseline: `429952eb26653b76e7dd135af2b4a5881095476b`

Date: 2026-08-05

## 1. Purpose

Phase 13 delivered useful footprint, CI, auto-sync, API, CLI, and documentation improvements, but repository review found two release-blocking correctness defects in the implementation of Phase 13A and Phase 13B:

1. multi-batch synchronization may return after the first upload batch and silently omit later batches;
2. server shutdown no longer has the former 30-second lifetime cap, but the current shutdown path does not actually signal and await both services correctly.

The review also found that the Phase 13 roadmap and phase files still report `READY FOR IMPLEMENTATION` despite implementation commits, and that verification does not currently exercise the complete behaviors that failed.

This phase is a narrow corrective closure. It must repair the two defects, add focused regressions, reconcile a small number of adjacent contract/documentation issues, and update the Phase 13 records truthfully.

It must not reopen broad architecture, dependency, API, TUI, storage-format, or feature work.

## 2. Release disposition

Until every required acceptance criterion in this plan is satisfied:

- do not mark Phase 13 complete;
- do not publish a release based on the current implementation;
- do not delete the new regression tests as “slow” or replace them with source-text assertions;
- do not treat successful compilation or existing unit batching tests as evidence that the end-to-end behaviors are correct.

## 3. Governing constraints

### Required

- preserve the existing protobuf service and RPC shapes;
- preserve encrypted sync, deterministic snippet ordering, conflict/deletion semantics, and the local 16 MiB exact-input limit;
- preflight all upload batches before the first remote mutation;
- send every planned upload batch before returning success;
- make repeated execution convergent after a partial network failure;
- coordinate Ctrl-C and Unix SIGTERM through one shutdown path;
- send the shutdown notification for every terminal event;
- retain and await both service tasks until completion or bounded forced termination;
- stop persistence only after request-serving tasks have drained or been aborted;
- keep routine verification focused and release verification manual;
- update every Phase 13 plan status and completion record only after real verification;
- keep the implementation understandable by a smaller execution model.

### Prohibited

- streaming RPC redesign;
- transaction protocol across upload batches;
- client-side upload journal, queue database, daemon, or background service;
- new signal, supervisor, orchestration, async-runtime, or test framework dependency;
- lowering the local command-size feature limit to hide transport behavior;
- increasing the default gRPC message limit as the primary fix;
- replacing Tonic or Axum;
- adding retries that can loop indefinitely;
- broad auto-sync, transaction, API, CLI, theme, updater, or dependency refactors;
- adding CI jobs, matrices, scheduled workflows, artifacts, coverage, or benchmark gates;
- claiming graceful shutdown from process exit alone when the OS may have killed the process directly.

## 4. Confirmed defects

### 4.1 Early return inside the upload-batch loop

`SyncClient::sync_encrypted` currently sends the first batch through `Sync`, processes its response page, and may return immediately when `has_more == false` or the page is empty. That return is inside the upload-batch loop.

For an initially empty remote library, `has_more == false` is expected. A local collection requiring multiple batches can therefore report success after only the first batch was uploaded.

### 4.2 Incomplete singleton overflow validation

`build_upload_batches` detects an oversized item when it is the only item in the current batch at the moment the ceiling is exceeded. When a later item overflows an existing nonempty batch, it is moved into a new singleton batch without immediately revalidating that singleton.

A large item following a small item can therefore escape the “fail before any remote mutation” guarantee.

### 4.3 Request-envelope mismatch risk

The current batch builder measures `SyncRequest::encoded_len()`, while later batches are sent as `PushSnippetsRequest`. The corrected preflight must measure the request envelope actually used, or conservatively prove each batch fits every request shape through which it may be sent.

### 4.4 Shutdown broadcast and task drain defects

The current server orchestration:

- does not broadcast shutdown after a requested Ctrl-C terminal event;
- moves service handles into `tokio::select!`, leaving no reliable way to await both afterward;
- uses an empty future inside the purported graceful-drain timeout;
- logs service errors inside tasks instead of returning them to the orchestrator;
- does not use Tonic’s shutdown-aware serving path;
- can stop rate-limit persistence before both request-serving tasks have completed.

### 4.5 SIGTERM mismatch

`snip-sync stop` sends SIGTERM on Unix, while the server only explicitly waits for Ctrl-C. A successful process exit after SIGTERM is not sufficient evidence of graceful shutdown because the OS default signal action can terminate the process directly.

### 4.6 Broken process-test setup

The lifetime tests configure `GRPC_PORT=0` and `HTTP_PORT=0`, but strict Phase 13A configuration validation rejects zero ports. The tests are ignored and are not currently part of release verification, so this contradiction is not caught by routine checks.

## 5. Target sync algorithm

Implement one explicit upload plan before network work.

### 5.1 Preflight

1. Encrypt all local snippets as today.
2. Sort successfully encrypted snippets by stable snippet ID.
3. Build every upload batch before creating the first mutating request.
4. Measure each batch using the actual protobuf request envelope that will carry it.
5. Recheck every newly formed singleton immediately after an overflow split.
6. If any individual encrypted snippet cannot fit, return `RequestTooLarge` before sending any mutating RPC.
7. Preserve all local data and pending sync intent on every preflight failure.

The batching helper should return a small explicit plan type or `Vec<Vec<Snippet>>`. Do not introduce a generic batching framework.

### 5.2 Preferred multi-batch execution

Use the existing RPCs without depending on a response generated before all uploads are present.

For one upload batch:

```text
Sync(batch, offset=0)
then paginate remaining response pages
```

For two or more upload batches:

```text
PushSnippets(batch 1)
PushSnippets(batch 2)
...
PushSnippets(batch N)
Sync(empty upload, offset=0)
then paginate remaining response pages
```

This ordering has three useful properties:

- every batch is uploaded before the authoritative response snapshot is requested;
- `has_more == false` on the first response cannot truncate uploads;
- the final response describes server state after all successful uploads.

If implementation evidence shows that using `Sync` for the first batch is required for compatibility, it may be retained only if the code still:

- sends every remaining batch before any return;
- discards or treats the early response as provisional;
- performs a final empty-upload `Sync` from offset zero after all pushes;
- documents why the duplicate/provisional request is necessary.

Do not add a new RPC.

### 5.3 Partial remote mutation semantics

A network or server failure after one or more successful `PushSnippets` calls can leave a partial remote upload. The existing server upsert behavior is ID-based and must remain idempotent/convergent.

Required behavior:

- return a typed sync failure naming the failed batch number and total batch count;
- do not advance the client’s successful-sync cursor/state;
- preserve pending auto-sync state;
- allow a full retry from batch 1;
- repeated accepted batches must not create duplicate logical snippets;
- do not attempt compensating deletes or rollback RPCs.

This is acceptable local-tool behavior and avoids a distributed transaction design.

## 6. Sync implementation workstreams

### Workstream A — Correct the batch builder

Likely file: `src/sync.rs`

1. Separate fixed request metadata sizing from snippet accumulation clearly.
2. Measure the actual request type used by multi-batch pushes.
3. After moving an overflow item into a new batch, measure that singleton immediately.
4. Return `RequestTooLarge` with snippet ID, measured encoded size, and ceiling.
5. Ensure no empty upload batch is produced.
6. Preserve deterministic ID ordering.
7. Avoid cloning the complete growing batch solely for every size probe if a small direct request construction remains readable; optimization is secondary to correctness.

Required pure tests:

- empty input produces no upload batches;
- one small item produces one batch;
- multiple small items fit one batch;
- exact boundary fits;
- one byte over starts a new batch;
- oversized first item fails;
- small item followed by oversized item fails;
- oversized item between two small items fails;
- every produced batch fits the actual `PushSnippetsRequest` envelope;
- single-batch path fits the `SyncRequest` envelope;
- order is deterministic across shuffled input;
- preflight failure occurs before a supplied fake sender records any call, if a small sender seam is useful.

### Workstream B — Separate upload and response phases

Likely file: `src/sync.rs`

1. Keep encryption/preflight outside network loops.
2. Add one small helper for uploading a prepared batch with existing retry/deadline behavior.
3. For one batch, retain the efficient direct `Sync` path.
4. For multiple batches, upload all batches first through `PushSnippets`.
5. Only after every upload succeeds, request the authoritative first response page using empty-upload `Sync` at offset zero.
6. Paginate from the returned page until `has_more == false` or an empty page safely terminates the loop.
7. Build the final aggregated response once, after upload and pagination are complete.
8. Remove every success return from inside the upload loop.

Do not duplicate retry logic further. Prefer one request helper per RPC shape.

### Workstream C — Add a complete encrypted multi-batch regression

Add or extend one integration target, preferably named clearly such as:

```text
tests/sync_multibatch.rs
```

The test must use the real client encryption path and real `snip-sync` service/database behavior. It may run the service in-process to remain fast and deterministic.

Required scenario:

1. create an isolated SQLite database and server state;
2. register/authenticate normally or use the existing supported test helper;
3. create an initially empty remote library;
4. construct enough individually valid local snippets to require at least three batches under a test-only small byte ceiling;
5. execute encrypted sync;
6. assert every expected snippet ID exists remotely;
7. assert the final client response contains the complete expected state;
8. assert no duplicate logical rows exist;
9. run the same sync again and assert convergence/idempotency.

The test must fail against the current early-return implementation.

Use a parameterized internal ceiling or narrowly scoped test helper. Do not read a production environment variable to alter the ceiling.

### Workstream D — Add partial-failure convergence coverage

Use a small deterministic fake or request observer to fail a later push batch once.

Assert:

- earlier batches may exist remotely;
- the sync call returns failure and does not report full success;
- pending/success cursor state is not advanced by the caller;
- retrying the entire sync uploads the remaining state;
- the final server set contains one logical row per snippet ID.

Keep this to one representative failure point. Do not add a full batch-by-batch failpoint matrix.

## 7. Target server lifetime architecture

Use one orchestrator with one shutdown trigger and two shutdown-aware service futures.

```text
bind both listeners
construct shared shutdown channel/token
spawn gRPC task returning Result
spawn HTTP task returning Result
wait for first terminal event:
  Ctrl-C
  SIGTERM on Unix
  gRPC task completion/error/panic
  HTTP task completion/error/panic
record whether terminal event was requested or unexpected
broadcast shutdown unconditionally
await every task not already completed within drain timeout
abort and await any remaining task after timeout
signal persistence shutdown
await persistence flush with its own short timeout
return success only for requested shutdown with clean service completion
return error for unexpected service exit, service error, panic, or forced drain
```

No service task may convert an error into log-only `Ok(())`.

## 8. Server implementation workstreams

### Workstream E — Add one process shutdown signal future

Likely file: `snip-sync/src/main.rs`

1. Build one async signal future.
2. On all platforms, wait for `tokio::signal::ctrl_c()`.
3. On Unix, also wait for `tokio::signal::unix::SignalKind::terminate()`.
4. Return a small enum such as `ShutdownSignal::Interrupt` or `ShutdownSignal::Terminate` for diagnostics.
5. If Unix signal registration itself fails, fail startup rather than silently losing `snip-sync stop` compatibility.

Do not add `signal-hook` or another dependency to the server unless Tokio demonstrably cannot support the required signal.

### Workstream F — Make both services shutdown-aware

For HTTP:

- retain `axum::serve(...).with_graceful_shutdown(...)`;
- receive the shared shutdown notification;
- return the server result to the task caller.

For gRPC:

- use Tonic’s shutdown-aware serve API for the existing incoming listener;
- receive the same shutdown notification;
- return the server result to the task caller.

Do not implement graceful shutdown by selecting around and dropping the server future.

### Workstream G — Retain and drain both task handles

1. Store both `JoinHandle<Result<...>>` values as mutable handles.
2. Select on mutable references so ownership is retained.
3. Record which task, if any, completed first and its result.
4. Broadcast shutdown for signals and unexpected service completion alike.
5. Await the task that already completed only through its captured result; do not poll a completed handle twice.
6. Await every remaining task inside the real drain timeout.
7. On drain timeout, call `abort()` on unfinished tasks and await their aborted join results.
8. Treat panic and service error as process failure.
9. Do not use an empty async block as a drain placeholder.

A small internal helper struct or enum is acceptable. Do not introduce a generalized task supervisor.

### Workstream H — Order persistence shutdown correctly

1. Keep the persistence task alive while HTTP/gRPC are accepting or draining requests.
2. After both request-serving tasks have completed or been aborted, signal persistence shutdown.
3. Await the final persistence snapshot with the existing small timeout.
4. Return an error if requested shutdown required forced service abort; a warning-only successful exit would hide incomplete draining.
5. Preserve database pool lifetime until persistence completes.

## 9. Server regression tests

### Workstream I — Deterministic orchestration tests

Extract only enough orchestration logic to test with short fake service futures.

Required cases:

- requested shutdown notifies both fake services;
- both services are awaited before persistence shutdown is observed;
- first service error triggers sibling shutdown and process error;
- first service panic triggers sibling shutdown and process error;
- one service refusing to drain is aborted after timeout and returns failure;
- no normal-operation lifetime timeout exists.

Use Tokio tests and channels. Do not create production observer globals or source-scanning tests.

### Workstream J — Repair process-level tests

Update `tests/snip_sync_lifetime.rs` or split it into one fast shutdown target and one long lifetime target.

Port setup:

- reserve valid nonzero loopback ports by binding `127.0.0.1:0`, reading the assigned port, then releasing the probe listener immediately before spawn;
- pass those concrete ports through configuration;
- poll `/health` with a bounded startup deadline;
- do not parse logs as the sole readiness signal.

Unix graceful SIGTERM test:

1. start server;
2. wait for health;
3. send SIGTERM;
4. assert exit within the drain bound;
5. assert normal successful exit rather than `ExitStatusExt::signal() == Some(SIGTERM)`;
6. assert the PID record is removed;
7. start a replacement server on the same ports/state and confirm singleton lock release.

Long lifetime test:

1. start server;
2. verify health;
3. wait beyond 30 seconds;
4. verify health again;
5. terminate through the graceful path;
6. assert successful exit.

Keep the long test manual/release-only. The fast SIGTERM test may be routine if reliable and under approximately 10 seconds.

Windows:

- retain compile coverage;
- use Ctrl-C/process control only if the current test environment can do so deterministically;
- do not block closure on inventing a Windows signal harness.

## 10. Adjacent configuration completion

Complete only the missing nonzero validation that directly prevents unusable server settings.

Review and, where currently accepted, reject zero for:

- `MAX_ID_LENGTH`;
- `MAX_DEVICE_ID_LENGTH`;
- `MAX_API_KEY_LENGTH`;
- `RATE_LIMIT_PER_MINUTE`.

Each error must include the configuration name and invalid value, consistent with existing `ConfigLoadError::InvalidRange` behavior.

Do not add arbitrary upper bounds or a generalized validation crate.

Add direct unit tests for each invalid zero value and one valid boundary value.

## 11. Verification integration

### 11.1 Routine checks

Keep `scripts/check.sh` compact. It must cover the fast correctness boundaries through one of these approaches:

- unit tests already included by `cargo test --workspace --lib` for batch construction and orchestration;
- one explicit fast encrypted multi-batch integration target;
- one explicit fast server SIGTERM test if stable enough for Linux CI.

Recommended addition:

```text
cargo test --test sync_multibatch -- --test-threads=1
```

Add the process-level SIGTERM test to routine checks only if repeated local runs show it is deterministic. Otherwise keep deterministic orchestration unit coverage routine and run the process test in release verification.

### 11.2 Release checks

`scripts/release-check.sh verify` must explicitly run:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
```

If fast shutdown and long lifetime are split, name both explicitly.

Retain the simplified release structure. Do not restore the entire full workspace suite or all former crash matrices.

### 11.3 CI

Do not add jobs or matrices. Linux uses `scripts/check.sh`. macOS and Windows remain compile/library/platform smoke.

The long lifetime test remains manual release verification because it intentionally waits beyond 30 seconds.

## 12. Phase 13 record reconciliation

Only after code and verification pass, update:

- `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`;
- `plans/snip-it-phase-13a-server-lifetime-config-correctness.md`;
- `plans/snip-it-phase-13b-sync-request-sizing-clock-diagnostics.md`;
- `plans/snip-it-phase-13c-verification-ci-simplification.md`;
- `plans/snip-it-phase-13d-client-runtime-dependency-footprint.md`;
- `plans/snip-it-phase-13e-auto-sync-persistence-simplification.md`;
- `plans/snip-it-phase-13f-api-cli-server-surface-consolidation.md`;
- this Phase 13G plan.

### 12.1 Status rules

Use truthful statuses:

- `COMPLETE` only when required acceptance criteria and verification are recorded;
- `COMPLETE WITH DOCUMENTED DEVIATIONS` when an intentional bounded deviation remains;
- `PARTIAL` when required behavior is still missing;
- never leave an implemented phase as `READY FOR IMPLEMENTATION`.

### 12.2 Required completion record fields

Each phase file must record:

- implementation commit SHA(s);
- corrective commit SHA(s), if applicable;
- acceptance checklist status;
- exact verification commands run;
- pass/fail result;
- intentional deviations or deferred items;
- whether the phase is release-blocking.

Do not retroactively check criteria that were not actually verified.

### 12.3 Correct Phase 13C claims

The Phase 13C completion record currently claims Phase 13A and Phase 13B regressions remain covered. Replace that claim with the actual targets and commands after Phase 13G adds them.

Do not describe a suite as passing while excluding an unrecorded failure. Any excluded pre-existing failure must be named, justified, and resolved or explicitly accepted by the roadmap.

## 13. Public API documentation correction

Perform one narrow documentation consistency pass in `src/lib.rs` and `docs/PUBLIC_API.md`:

- remove `SnippetData`, `ProcessResult`, `CommandOutcome`, and `SelectionOutcome` from the supported stable API table if they remain `#[doc(hidden)]` implementation types;
- do not claim public hidden modules can change “without a semver bump” as an unconditional guarantee;
- state instead that they are not documented for external use and may be changed in a semver-appropriate release;
- keep the existing binary/library arrangement; do not move all binary modules in this phase.

This is documentation truthfulness, not an API redesign.

## 14. Phase 13E bounded closure audit

Do not reopen broad auto-sync/persistence simplification. Perform only this bounded audit:

1. confirm ordinary one-file snippet mutations do not create transaction journals;
2. confirm retained legacy transaction variants exist only for on-disk recovery compatibility;
3. confirm the scheduler no longer probes/releases the execution lock before spawn;
4. confirm a failed helper preserves pending generation and enters bounded backoff;
5. confirm exact-generation clearing remains protected;
6. remove an obsolete thin internal re-export module only if no compatibility or binary boundary requires it and deletion reduces code without replacement machinery;
7. document accepted residual complexity rather than forcing another scheduler rewrite.

If any of items 1–5 is false, fix that specific contract and add one focused regression. Do not generalize the work.

## 15. Likely files

Core sync:

- `src/sync.rs`
- `src/error.rs` only if an existing typed error needs a clearer batch failure detail
- `tests/sync_multibatch.rs` or an existing real sync integration target

Server:

- `snip-sync/src/main.rs`
- `snip-sync/src/lib.rs` for a narrowly testable orchestration helper if needed
- `tests/snip_sync_lifetime.rs`
- possibly one new fast server-shutdown integration target

Verification:

- `scripts/check.sh`
- `scripts/release-check.sh`
- no workflow topology change expected

Documentation and plans:

- `src/lib.rs`
- `docs/PUBLIC_API.md`
- relevant sync/server architecture docs only where behavior changes
- Phase 13 roadmap and plans listed in Section 12

Do not modify themes, updater archive formats, release profile, TUI, CLI grouping, protobuf definitions, database schema, encryption format, or unrelated command implementations.

## 16. Execution order for handoff

### Pass 1 — Add failing regressions

1. add pure oversized-after-small batching test;
2. add complete three-batch encrypted sync integration test;
3. repair process-test port allocation;
4. add deterministic orchestration tests;
5. prove the new tests fail for the intended reasons before implementation.

Do not commit brittle timing assertions unrelated to the defect.

### Pass 2 — Fix sync preflight and flow

1. correct singleton validation;
2. measure actual request envelopes;
3. separate multi-batch upload from response pagination;
4. remove early returns from upload loop;
5. add partial-failure convergence test;
6. run focused sync tests.

### Pass 3 — Fix server shutdown

1. add Ctrl-C/SIGTERM signal future;
2. make HTTP and gRPC shutdown-aware and error-returning;
3. retain both handles through select;
4. broadcast on every terminal event;
5. perform real bounded drain and abort;
6. move persistence shutdown after service drain;
7. run deterministic and process-level tests.

### Pass 4 — Complete verification wiring

1. add fast multi-batch regression to routine checks;
2. add long lifetime and graceful shutdown to release checks;
3. run routine checks repeatedly enough to detect process-test flakiness;
4. run the manual release verification once from a clean tree;
5. make no new CI job.

### Pass 5 — Reconcile documentation and phase records

1. correct public API wording/table;
2. perform bounded Phase 13E audit;
3. update architecture docs for actual final behavior;
4. update every Phase 13 status and completion record;
5. mark the roadmap complete only when all release blockers are closed.

## 17. Focused verification commands

The implementation agent must run, at minimum:

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

Also run the fast process-level SIGTERM test at least five consecutive times on Linux if it is added to routine CI. Record the result as a concise count, not an artifact bundle.

If a command name changes during implementation, update this plan and the scripts with the exact final target name.

## 18. Required acceptance criteria

### Sync correctness

- [ ] Every upload batch is planned and size-validated before the first mutating RPC.
- [ ] An oversized first, middle, or final item fails before any remote mutation.
- [ ] Batch sizing accounts for the actual request envelope used.
- [ ] No success return exists inside the upload-batch loop.
- [ ] A collection requiring at least three batches uploads every snippet to an initially empty server.
- [ ] The final response is fetched after all successful upload batches.
- [ ] A repeated sync is convergent and creates no duplicate logical snippets.
- [ ] Later-batch failure returns a typed failure and does not advance successful-sync state.
- [ ] Pending auto-sync work survives partial batch failure.
- [ ] No new RPC, journal, queue, daemon, or distributed rollback mechanism is added.

### Server lifetime and shutdown

- [ ] Normal operation has no arbitrary lifetime timeout.
- [ ] Ctrl-C triggers the shared graceful shutdown path.
- [ ] Unix SIGTERM triggers the same graceful shutdown path.
- [ ] `snip-sync stop` results in a normal successful server exit on Unix.
- [ ] Shutdown notification is sent for requested and unexpected terminal events.
- [ ] HTTP uses a graceful shutdown future.
- [ ] gRPC uses a shutdown-aware Tonic serve path.
- [ ] Both service task handles remain owned by the orchestrator.
- [ ] Every unfinished task is awaited inside the real drain timeout.
- [ ] Refusing tasks are aborted and awaited after timeout.
- [ ] Service errors and task panics produce process failure.
- [ ] Persistence shutdown occurs only after services drain or abort.
- [ ] The server remains healthy beyond 30 seconds in the release regression.
- [ ] Lifetime tests use valid nonzero ports and health polling.

### Verification and closure

- [ ] Routine checks include a real complete encrypted multi-batch regression.
- [ ] Release checks include sustained lifetime and graceful signal shutdown.
- [ ] No new CI job, matrix, external service, coverage system, or artifact is added.
- [ ] Invalid zero ID/device/API-key/rate-limit settings fail with typed diagnostics.
- [ ] Supported API documentation does not list hidden implementation result types as stable.
- [ ] Documentation does not promise semver-incompatible hidden-module changes without an appropriate release.
- [ ] Phase 13E bounded audit items 1–5 are confirmed or narrowly corrected.
- [ ] Every Phase 13 plan has a truthful final status and completion record.
- [ ] Phase 13C’s regression-coverage claims name the actual tests and commands.
- [ ] The roadmap is marked complete only after all required commands pass.
- [ ] No release blocker from this plan remains deferred.

## 19. Stop conditions

Stop and amend the plan if:

- the proposed sync fix requires a protobuf or database-schema change;
- complete preflight cannot be achieved without storing upload state durably;
- a helper abstraction becomes a generalized transport or orchestration framework;
- graceful shutdown requires adding a supervisor or service-manager layer;
- process tests rely only on log parsing or sleep without health polling;
- routine checks become substantially slower due to the long lifetime test;
- implementation starts changing themes, updater, TUI, CLI grouping, encryption, or unrelated transaction behavior;
- API cleanup would require moving most of the binary into a new crate or a semver-major release;
- Phase 13E cleanup starts reconstructing the scheduler or transaction engine again.

Prefer the smallest direct correction that proves the user-visible contract.

## 20. Completion record template

Fill this section only after implementation.

```text
Status: COMPLETE | COMPLETE WITH DOCUMENTED DEVIATIONS | PARTIAL

Implementation commits:
- <sha> <summary>

Corrective verification commit:
- <sha> <summary>

Verification:
- cargo fmt --all -- --check: PASS/FAIL
- cargo clippy --workspace --all-targets -- -D warnings: PASS/FAIL
- cargo test -p snip-it --lib sync: PASS/FAIL
- cargo test -p snip-sync --lib: PASS/FAIL
- cargo test --test sync_multibatch -- --test-threads=1: PASS/FAIL
- cargo test --test snip_sync_lifetime -- --ignored --test-threads=1: PASS/FAIL
- bash scripts/check.sh: PASS/FAIL
- bash scripts/release-check.sh verify: PASS/FAIL
- cargo doc -p snip-it --no-deps: PASS/FAIL
- repeated fast SIGTERM runs: <N>/<N> PASS

Residual deviations:
- <none or exact bounded deviation>

Release disposition:
- BLOCKED | CLEARED
```
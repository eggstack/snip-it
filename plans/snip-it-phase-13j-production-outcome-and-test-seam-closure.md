# Phase 13J — Production Outcome Wiring and Test-Seam Closure

Status: IMPLEMENTED; VERIFICATION PENDING

Parent roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Corrective baseline: `39f8ef5ae9a0d32330d394738c3d862dc5c7560f`

Date: 2026-08-06

Implementation commit:
- <filled after Pass 4 commit exists>

Closure record commit:
- <filled after Pass 7 commit exists>

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Phase 13I fixed the difficult parts of the prior corrective line:

- deterministic retained-state partial-upload failure;
- exact-once retry convergence against the same SQLite state;
- explicit zero-batch and paginated pull coverage;
- per-service drain completion tracking;
- drain-time error and panic classification;
- explicit abort-and-await behavior for pending tasks.

A review of `39f8ef5` found four remaining closure problems:

1. `serve_inner` does not use the fully classified shutdown outcome and can return success after a requested shutdown in which gRPC or HTTP returned an error or panicked;
2. `sync_encrypted_with_custom_encrypt` duplicates the complete zero/one/many sync algorithm solely for one test and is exposed as a public method;
3. `add_batch_context` is public only so an integration test can call a local implementation helper;
4. Phase 13H, Phase 13I, and the roadmap contain contradictory statuses, incomplete commit records, and checked claims that are stronger than the direct evidence.

Phase 13J is a narrow closure pass. It must wire the existing outcome into production, collapse the test seams back onto one sync implementation, tighten tests without creating new infrastructure, and restore truthful records.

This phase is not permission to reopen sync architecture, server lifecycle architecture, persistence, CI, packaging, or public API design.

## 2. Release disposition

Until Phase 13J is implemented and verified:

- Phase 13 remains open;
- the roadmap status must remain `CORRECTIVE CLOSURE REQUIRED`;
- Phase 13I is treated as implemented with corrective follow-up required;
- do not publish a release from `39f8ef5` or an unverified descendant;
- the prior `release-check.sh verify: PASS` result does not cover source changes made by Phase 13J;
- do not mark an item complete because a helper-level unit test passes while production does not consume that helper.

## 3. Small-model execution rules

These rules are mandatory for handoff reliability.

1. Work in the numbered pass order in Section 11.
2. Finish and verify one pass before starting the next.
3. Do not modify a file unless it is listed for the current pass.
4. Do not rename unrelated functions, modules, tests, or variables.
5. Prefer moving existing code over rewriting it.
6. Do not create a generalized abstraction when a direct two-service or one-client helper is sufficient.
7. Do not add new dependencies, features, test targets, workflow jobs, or configuration fields.
8. Run the exact focused command after each pass.
9. If a focused command fails outside the edited area, record the failure before changing scope.
10. Do not edit completion records until the implementation commit exists.
11. Do not claim clean-tree release verification before committing implementation changes.
12. Stop rather than improvising if an edit appears to require protocol, schema, or persistence changes.

## 4. Scope boundary

### 4.1 Required

- make production success depend on `ServiceShutdownOutcome::is_clean_requested_shutdown()` or one directly equivalent tested method;
- preserve persistence cleanup before returning any service failure;
- include both service classifications in the final production failure diagnostic;
- retain one zero/one/many sync transport implementation;
- remove the public custom-encryption sync method;
- keep encryption-failure injection private and test-only;
- return `add_batch_context` to private visibility;
- move local helper tests into `src/sync.rs` unit tests;
- make orchestration tests prove the helper sends shutdown itself;
- make the no-pre-signal-timeout test actually run the helper while the signal remains pending;
- correct Phase 13 records and commit SHAs;
- rerun focused, routine, and clean-tree release verification.

### 4.2 Prohibited

Do not add:

- new RPCs, protobuf fields, protocol versions, database tables, or migrations;
- upload journals, rollback requests, queues, durable checkpoints, or distributed transactions;
- generalized supervisors, task registries, service managers, or daemon frameworks;
- production failure-injection environment variables or config fields;
- new async, signal, mock, test, or orchestration dependencies;
- new CI jobs, matrices, schedules, coverage systems, benchmark gates, or evidence artifacts;
- broad changes to auto-sync, transactions, TUI, themes, updater, CLI, public API, packaging, or deployment;
- a new high-level pull test harness if the existing lower-level real-server regression is the only practical seam;
- source-text tests that search files for expected strings.

## 5. Confirmed baseline defects

### 5.1 Production ignores dirty drain results

`run_services_until_shutdown` returns:

- `requested`;
- `forced`;
- `grpc_result`;
- `http_result`.

It also provides `is_clean_requested_shutdown()`, which is true only when:

```text
requested == true
forced == false
grpc_result == Clean
http_result == Clean
```

However, `serve_inner` currently returns failure only when:

```text
!outcome.requested || outcome.forced
```

A requested shutdown with `ServiceError` or `Panic` therefore reaches `Ok(())` after cleanup.

### 5.2 Custom encryption duplicates the sync state machine

`SyncClient::sync_encrypted_with_custom_encrypt` contains another complete implementation of:

- encryption preparation;
- upload batch construction;
- zero-batch pull;
- one-batch sync;
- multi-batch push;
- response accumulation;
- pagination.

This duplicates `sync_encrypted_inner` and allows the test implementation to diverge from production.

### 5.3 Test-only helpers expanded the public surface

`add_batch_context` is public only so `tests/sync_multibatch.rs` can call it.

`sync_encrypted_with_custom_encrypt` is public despite being described as test-only.

The crate documentation states that sync internals are implementation details. These functions should not be externally callable production methods.

### 5.4 Orchestration tests partially mask production behavior

Several requested-shutdown tests use a signal future that sends on the same broadcast channel before returning. Services can wake from the test signal's send even if the orchestration helper's own broadcast is removed.

The `no_pre_signal_lifetime_timeout` test sleeps before calling the helper. It does not prove the helper remains pending longer than the drain timeout while waiting for the first terminal event.

### 5.5 Records are contradictory

At the baseline:

- the roadmap header says `CORRECTIVE CLOSURE REQUIRED`;
- the roadmap footer says `COMPLETE` and `CLEARED`;
- Phase 13I is marked complete despite the production outcome defect;
- Phase 13H still has an unqualified `Status: COMPLETE` header;
- the Phase 13I record omits or misattributes `5f10c68`, `18e7ddb`, and `39f8ef5`;
- checked criteria claim direct evidence for behavior that is only indirectly covered or not wired into production.

## 6. Target shutdown outcome behavior

Keep the current orchestration helper and result types.

### 6.1 Add one tested production decision method

Preferred shape in `snip-sync/src/orchestration.rs`:

```rust
impl ServiceShutdownOutcome {
    pub fn ensure_clean_requested_shutdown(&self) -> Result<(), String> {
        if self.is_clean_requested_shutdown() {
            return Ok(());
        }

        Err(format!(
            "service shutdown was not clean: requested={}, forced={}, grpc={:?}, http={:?}",
            self.requested,
            self.forced,
            self.grpc_result,
            self.http_result,
        ))
    }
}
```

An equivalent name is acceptable. Do not introduce a new error enum for this one decision.

Requirements:

- return `Ok(())` only for requested, unforced, dual-clean shutdown;
- return `Err` for unexpected clean service exit;
- return `Err` for either service error;
- return `Err` for either panic;
- return `Err` for forced cancellation;
- retain both service names and classifications in the message;
- preserve original service error/panic detail already stored in `ServiceResult`.

### 6.2 Wire production to the tested method

In `snip-sync/src/main.rs`, keep the current ordering:

1. await `run_services_until_shutdown`;
2. signal persistence shutdown;
3. await persistence with the existing timeout;
4. log server shutdown completion;
5. evaluate the fully classified service outcome;
6. return success or failure.

Replace the current boolean-only condition with the tested decision method.

Required shape:

```rust
outcome
    .ensure_clean_requested_shutdown()
    .map_err(|message| -> Box<dyn std::error::Error> { message.into() })?;

Ok(())
```

Use the repository's compiling error-conversion style if the explicit closure is unnecessary.

Do not return before persistence cleanup.

## 7. Shutdown tests

Modify only `snip-sync/src/orchestration.rs` for this workstream unless compilation requires a narrow import change.

### 7.1 Decision-method tests

For existing outcome-producing tests, add assertions on the exact method production calls:

- requested dual-clean outcome returns `Ok`;
- requested drain-time HTTP error returns `Err` containing `HTTP` or `http` and the original error text;
- requested drain-time gRPC panic returns `Err` containing `gRPC` or `grpc` and panic text;
- unexpected clean gRPC exit returns `Err`;
- forced timeout returns `Err`.

Do not add a second independent success predicate in tests.

### 7.2 Prove helper-owned shutdown broadcast

For requested-shutdown tests, do not send on `shutdown_sender` from the signal future.

Use an immediately ready signal:

```rust
let signal = std::future::ready(());
tokio::pin!(signal);
```

or a one-shot signal whose sender is separate from the broadcast sender.

Services must wake only because `run_services_until_shutdown` executes:

```rust
shutdown_sender.send(())
```

Retain the unexpected-completion tests with a pending signal.

### 7.3 Correct the no-pre-signal-timeout test

Use a one-shot channel to hold the signal pending.

Required sequence:

1. spawn or pin `run_services_until_shutdown` with a drain timeout near 50–100 ms;
2. keep both services pending on the shutdown broadcast;
3. keep the process signal pending;
4. sleep for at least twice the configured drain timeout;
5. assert the orchestration future/task is still pending;
6. trigger the process signal;
7. assert both services finish cleanly and the result is clean requested shutdown.

Do not use a long wait. The complete test should remain below one second.

### 7.4 Do not rewrite the drain state machine

The current per-handle consumed-state correction should remain unless a focused test reproduces a defect. Phase 13J is not a second orchestration rewrite.

## 8. One sync implementation with a private test seam

Primary file: `src/sync.rs`

Test relocation file: `tests/sync_multibatch.rs`

### 8.1 Preserve the public production entry points

Retain:

```text
sync_encrypted
sync_encrypted_with_ceiling
```

Both continue to call the same private production path.

Delete:

```text
pub async fn sync_encrypted_with_custom_encrypt
```

No public replacement is allowed.

### 8.2 Extract prepared transport once

Use the following mechanical structure.

#### Step 1 — Keep real encryption in `sync_encrypted_inner`

```rust
async fn sync_encrypted_inner(
    &mut self,
    local_snippets: Vec<Snippet>,
    last_sync: i64,
    library_id: &str,
    byte_ceiling: usize,
) -> SnipResult<SyncResponse> {
    self.ensure_budget()?;
    let api_key = self.settings.api_key.clone();
    let (encrypted_snippets, encrypt_failed_ids) =
        encrypt_snippets(&api_key, &local_snippets);

    self.sync_prepared_encrypted_inner(
        encrypted_snippets,
        encrypt_failed_ids,
        last_sync,
        library_id,
        byte_ceiling,
    )
    .await
}
```

#### Step 2 — Move, do not rewrite, the existing zero/one/many body

Create one private method:

```rust
async fn sync_prepared_encrypted_inner(
    &mut self,
    encrypted_snippets: Vec<Snippet>,
    encrypt_failed_ids: Vec<String>,
    last_sync: i64,
    library_id: &str,
    byte_ceiling: usize,
) -> SnipResult<SyncResponse>
```

Move the current code beginning with skipped-count setup and batch construction into this method.

The method owns the only implementation of:

- `build_upload_batches`;
- zero-batch empty `Sync`;
- one-batch upload-and-response `Sync`;
- multi-batch `PushSnippets` loop;
- batch error context;
- authoritative empty-upload response;
- page accumulation and pagination;
- final skipped count/ID construction.

Do not duplicate or modify behavior while moving it.

#### Step 3 — Add a private unit-test-only caller

Inside the `#[cfg(test)]` implementation or test module, use the existing private `encrypt_snippets_with` helper to prepare injected failures, then call `sync_prepared_encrypted_inner`.

Acceptable shape:

```rust
#[cfg(test)]
async fn sync_encrypted_with_test_encrypt<F>(
    &mut self,
    local_snippets: Vec<Snippet>,
    last_sync: i64,
    library_id: &str,
    byte_ceiling: usize,
    encrypt_fn: F,
) -> SnipResult<SyncResponse>
where
    F: Fn(&Snippet) -> SnipResult<Snippet>,
{
    self.ensure_budget()?;
    let (encrypted, failed_ids) = encrypt_snippets_with(&local_snippets, encrypt_fn);
    self.sync_prepared_encrypted_inner(
        encrypted,
        failed_ids,
        last_sync,
        library_id,
        byte_ceiling,
    )
    .await
}
```

This method must be private and compiled only for unit tests.

Do not place `#[cfg(feature = "test-support")]` on a public production method merely to keep an integration test unchanged.

### 8.3 Move the all-encryption-failed test into `src/sync.rs`

Move the existing real-server test from `tests/sync_multibatch.rs` into the `src/sync.rs` unit-test module.

Reuse the same assertions:

- all injected local encryption attempts fail;
- no panic occurs;
- every local failed ID is returned;
- `skipped_count` matches;
- zero prepared batches still contact the real in-process server;
- seeded remote snippets are returned.

Use the existing `snip-sync` development dependency and test helpers. Keep any copied server/client builder local and minimal.

Do not retain the old integration test after the unit test passes.

### 8.4 Keep real integration coverage where it belongs

Retain in `tests/sync_multibatch.rs`:

- real multi-batch upload;
- deterministic retained-state partial failure and retry;
- empty-local/empty-remote sync;
- seeded pull-only sync;
- zero-batch multi-page pagination.

These tests exercise public client behavior and should remain integration tests.

## 9. Return helper visibility to private

### 9.1 `add_batch_context`

Change:

```rust
pub fn add_batch_context(...)
```

to:

```rust
fn add_batch_context(...)
```

Move these tests from `tests/sync_multibatch.rs` into the existing `src/sync.rs` test module:

- clock skew kind and configuration classification;
- timeout kind and existing classification;
- original detail plus `batch N/M` context;
- non-`SyncFailure` fallback behavior if still useful.

The tests must call the same private helper used by the multi-batch loop.

### 9.2 Documentation cleanup

Update `AGENTS.md` only where it currently claims:

- `add_batch_context()` is public;
- the custom-encryption method is a public test seam;
- Phase 13I alone completed final closure.

Do not rewrite unrelated project guidance.

No user-facing API documentation or changelog entry is required for removing methods that were introduced only by the unclosed corrective commit and are documented as internal implementation.

## 10. Evidence and record policy

### 10.1 Do not create a new complex high-level pull harness

The real-server integration suite already proves:

```text
local_snippets = []
remote contains snippets
client retrieves all pages
```

The production `SyncDirection::Pull` branch visibly calls that same client operation.

For Phase 13J:

- keep the real zero-batch integration tests;
- do not claim a direct full CLI/filesystem pull test unless one already exists;
- record the high-level direction branch as indirectly covered by the real client path;
- do not build a new library-manager/filesystem/server harness solely to convert indirect evidence into direct evidence.

### 10.2 Successful cursor claim

Do not invent a new durable test seam.

Before closure:

1. search existing unit tests for failed-sync cursor behavior;
2. if direct coverage exists, record the exact test name;
3. if it does not exist, change the record from “directly proved” to “preserved by existing caller control flow; no new harness added”; 
4. do not leave a checked claim saying direct evidence exists when it does not.

### 10.3 Correct historical records

During the final record pass:

- Phase 13H header becomes `COMPLETE WITH CORRECTIVE FOLLOW-UP` or equivalent;
- Phase 13I header becomes `COMPLETE WITH CORRECTIVE FOLLOW-UP` until 13J closes;
- Phase 13I records these commits accurately:
  - `c08cac1` — main Phase 13I implementation;
  - `5f10c68` — drain-time error test and initial completion record;
  - `18e7ddb` — release-check result record;
  - `39f8ef5` — closure-SHA record correction;
- Phase 13J records its implementation and record commits;
- the roadmap has one authoritative status at the top and bottom;
- no acceptance criterion is checked without direct or explicitly labeled indirect evidence;
- `Residual deviations: none` is used only when no discrepancy remains.

## 11. Required execution sequence

### Pass 0 — Baseline and guardrails

Files: none modified.

1. confirm `git rev-parse HEAD` is `39f8ef5` or a reviewed descendant;
2. confirm the working tree is clean;
3. inspect only:
   - `snip-sync/src/orchestration.rs`;
   - `snip-sync/src/main.rs`;
   - `src/sync.rs`;
   - `tests/sync_multibatch.rs`;
   - the three Phase 13 record files;
4. do not modify Cargo manifests or lockfiles.

### Pass 1 — Production shutdown outcome

Files:

- `snip-sync/src/orchestration.rs`
- `snip-sync/src/main.rs`

Steps:

1. add `ensure_clean_requested_shutdown` or equivalent to `ServiceShutdownOutcome`;
2. add direct assertions to existing outcome tests;
3. remove test-signal broadcast sends from requested-shutdown cases;
4. correct the no-pre-signal-timeout test;
5. replace the production boolean-only check with the tested method;
6. retain persistence cleanup before the check.

Focused verification:

```text
cargo fmt --all -- --check
cargo test -p snip-sync --lib orchestration -- --test-threads=1
cargo clippy -p snip-sync --all-targets -- -D warnings
```

Stop if production still checks only `requested` and `forced`.

### Pass 2 — Collapse sync test seams

Files:

- `src/sync.rs`
- `tests/sync_multibatch.rs`

Steps:

1. add private `sync_prepared_encrypted_inner`;
2. move the existing zero/one/many body into it without behavior changes;
3. keep `sync_encrypted_inner` as real encryption plus delegation;
4. add private `#[cfg(test)]` injected-encryption caller;
5. move the all-encryption-failed test into `src/sync.rs`;
6. delete `sync_encrypted_with_custom_encrypt`;
7. make `add_batch_context` private;
8. move its tests into `src/sync.rs`;
9. remove relocated tests from `tests/sync_multibatch.rs`.

Focused verification:

```text
cargo fmt --all -- --check
cargo test -p snip-it --lib sync -- --test-threads=1
cargo test --test sync_multibatch -- --test-threads=1
cargo clippy -p snip-it --all-targets -- -D warnings
```

Stop if more than one method contains the zero/one/many batch match.

### Pass 3 — Documentation and records remain open

Files:

- `AGENTS.md`
- `plans/snip-it-phase-13h-final-correctness-closure.md`
- `plans/snip-it-phase-13i-drain-and-regression-closure.md`
- this plan
- roadmap

Before the implementation commit:

1. update behavior descriptions but keep Phase 13 open;
2. mark 13H and 13I as historical phases with corrective follow-up;
3. record known prior commits accurately;
4. leave Phase 13J as `IMPLEMENTED; VERIFICATION PENDING`;
5. do not write a release-check result yet.

Focused verification:

```text
rg -n "Status: COMPLETE|Release disposition: CLEARED|Residual deviations: none" plans/snip-it-phase-13*.md
```

Inspect every match manually. This command is for record review, not a source-code test.

### Pass 4 — Implementation commit

Commit all source, test, and truthful pending-record changes together unless the repository state makes two implementation commits clearer.

Preferred commit:

```text
phase-13j: wire shutdown outcomes and consolidate sync test seams
```

After commit, record the full SHA in Phase 13J.

### Pass 5 — Full verification

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p snip-it --lib sync -- --test-threads=1
cargo test -p snip-sync --lib orchestration -- --test-threads=1
cargo test -p snip-sync --lib
cargo test --test sync_multibatch -- --test-threads=1
cargo test --test platform_smoke
bash scripts/check.sh
bash -n scripts/release-check.sh
cargo doc -p snip-it --no-deps
```

Then run the existing process verification:

```text
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
```

Run the short Unix SIGTERM case five consecutive times and record the exact command and `5/5 PASS` or failure count.

### Pass 6 — Clean-tree release verification

1. commit the implementation and pending records;
2. confirm `git status --short` is empty;
3. run:

```text
bash scripts/release-check.sh verify
```

Do not edit files before recording the result; editing makes the tree dirty and invalidates the command's clean-tree precondition.

### Pass 7 — Final record commit

Update:

- Phase 13J completion record;
- Phase 13I historical commit list and qualification;
- Phase 13H status qualification;
- roadmap phase map, final checklist, and disposition;
- `AGENTS.md` only if final behavior text remains inaccurate.

Preferred commit:

```text
phase-13: record verified phase 13j closure
```

After this commit, do not claim that `release-check.sh verify` ran against the final record-only commit. Record that it passed against the implementation commit immediately preceding the record update.

## 12. Acceptance criteria

### 12.1 Production shutdown result

- [ ] Production calls the same clean-shutdown decision method tested by unit tests.
- [ ] Requested shutdown with two clean services returns success.
- [ ] Requested shutdown with a service error returns failure after persistence cleanup.
- [ ] Requested shutdown with a service panic returns failure after persistence cleanup.
- [ ] Forced abort returns failure after persistence cleanup.
- [ ] Unexpected clean service exit returns failure after sibling cleanup.
- [ ] Final failure text identifies gRPC and HTTP results.
- [ ] Original service error or panic detail remains visible.

### 12.2 Orchestration proof quality

- [ ] Requested-shutdown tests rely on the helper's broadcast, not a test-side broadcast send.
- [ ] No-pre-signal test runs the helper while the signal remains pending.
- [ ] The helper remains pending longer than the drain timeout before the first terminal event.
- [ ] Existing per-handle consumed-state and abort behavior remain passing.
- [ ] No second orchestration implementation is added.

### 12.3 Single sync implementation

- [ ] Exactly one method contains the zero/one/many batch transport logic.
- [ ] `sync_encrypted` and `sync_encrypted_with_ceiling` delegate to it.
- [ ] Injected encryption failures reach the same prepared transport method.
- [ ] `sync_encrypted_with_custom_encrypt` is removed.
- [ ] The injected-encryption caller is private and `#[cfg(test)]` only.
- [ ] All-encryption-failed behavior still contacts the real in-process server.
- [ ] Skipped IDs/counts and remote response assertions remain passing.
- [ ] Real zero-batch and retained-state integration tests remain passing.

### 12.4 Helper visibility

- [ ] `add_batch_context` is private.
- [ ] Its tests are colocated in `src/sync.rs`.
- [ ] No new supported public sync API is introduced.
- [ ] `cargo doc -p snip-it --no-deps` passes.

### 12.5 Scope control

- [ ] No Cargo dependency or feature change is made.
- [ ] No protocol, schema, migration, or persistence change is made.
- [ ] No supervisor, queue, journal, or generalized framework is added.
- [ ] No CI topology or release automation change is made.
- [ ] No new high-level pull harness is added solely for evidence ceremony.
- [ ] Routine checks remain compact.

### 12.6 Records and verification

- [ ] Phase 13H is truthfully qualified.
- [ ] Phase 13I is truthfully qualified and lists `c08cac1`, `5f10c68`, `18e7ddb`, and `39f8ef5` correctly.
- [ ] Phase 13J records exact implementation and record SHAs.
- [ ] Direct and indirect evidence are labeled accurately.
- [ ] No contradictory roadmap status remains.
- [ ] Focused tests pass.
- [ ] `bash scripts/check.sh` passes.
- [ ] Long lifetime test passes.
- [ ] Short SIGTERM passes 5/5.
- [ ] Clean-tree `bash scripts/release-check.sh verify` passes against the implementation commit.
- [ ] Roadmap returns to `COMPLETE` and `CLEARED` only after all blockers are resolved.

## 13. Expected final diff boundary

Expected source/test files:

```text
snip-sync/src/orchestration.rs
snip-sync/src/main.rs
src/sync.rs
tests/sync_multibatch.rs
```

Expected documentation/record files:

```text
AGENTS.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
plans/snip-it-phase-13h-final-correctness-closure.md
plans/snip-it-phase-13i-drain-and-regression-closure.md
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
```

Unexpected without explicit reproduced need:

```text
Cargo.toml
Cargo.lock
snip-sync/Cargo.toml
snip-proto/**
migrations/**
.github/**
scripts/check.sh
scripts/release-check.sh
src/auto_sync/**
src/transaction/**
src/ui/**
```

## 14. Stop conditions

Stop and amend this plan rather than broadening scope if:

- production outcome wiring appears to require redesigning orchestration;
- consolidating the encryption test requires a new public API;
- a new dependency is proposed;
- a protocol or database change appears necessary;
- moving the existing sync body causes behavior changes beyond delegation;
- a new high-level filesystem/server harness exceeds a small focused test;
- process verification requires privileged or external infrastructure;
- release verification cannot be run but records are about to be marked complete;
- unrelated Phase 13 work begins changing.

## 15. Completion record template

Fill only after implementation and verification.

```text
Status: COMPLETE | COMPLETE WITH DOCUMENTED DEVIATIONS | PARTIAL

Implementation commit:
- <sha> <summary>

Record commit:
- <sha> <summary>

Verification:
- cargo fmt --all -- --check: PASS/FAIL
- cargo clippy --workspace --all-targets -- -D warnings: PASS/FAIL
- cargo test -p snip-it --lib sync -- --test-threads=1: PASS/FAIL
- cargo test -p snip-sync --lib orchestration -- --test-threads=1: PASS/FAIL
- cargo test -p snip-sync --lib: PASS/FAIL
- cargo test --test sync_multibatch -- --test-threads=1: PASS/FAIL
- cargo test --test platform_smoke: PASS/FAIL
- bash scripts/check.sh: PASS/FAIL
- bash -n scripts/release-check.sh: PASS/FAIL
- cargo doc -p snip-it --no-deps: PASS/FAIL
- cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1: PASS/FAIL
- short Unix SIGTERM repeated run: <N>/5 PASS
- bash scripts/release-check.sh verify from clean implementation commit: PASS/FAIL

Evidence notes:
- high-level pull direction: direct | indirect through real zero-batch client integration
- failed-sync cursor: direct test <name> | preserved by inspected caller control flow

Residual deviations:
- none | <explicit bounded deviation>

Release disposition:
- BLOCKED | CLEARED
```

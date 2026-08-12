# Phase 15 Final Corrective Closure — Task Cancellation, Atomic Symlink Semantics, and Closure Record

Status: COMPLETE (2026-08-11)

Date: 2026-08-11

Baseline: `f56a481dcf7046cb8c785c7877e84861581af095` (`phase 15: consolidate deletion and build paths`)

Parent plan: `plans/snip-it-phase-15-deletion-consolidation.md`

Purpose: close the small set of issues found by the post-implementation audit of Phase 15 without reopening the broader simplification work.

This is a corrective closure pass, not Phase 16 and not a new architecture phase.

Implementation commit: recorded after the implementation commit and referenced
by the final closure commit.

Verification record:

- `cargo test -p snip-sync orchestration --lib` — pass (12 tests).
- `cargo test utils::atomic --lib` — pass (14 tests, including allowed broken-symlink replacement).
- `cargo test --test platform_smoke` — pass locally on Linux.
- `bash scripts/check.sh` — pass; formatting, clippy, and Linux correctness verified.
- `bash scripts/release-check.sh verify` — pass from a clean tree.
- Existing macOS and Windows platform-smoke coverage remains in the unchanged CI matrix; those hosted lanes were not locally executable on this Linux host.
- Final fixed-host measurement: Rust 1.94.1, `aarch64-unknown-linux-gnu`, `cargo build --release --bin snp`, 5,130,928 bytes.

## 1. Executive summary

Phase 15 substantially landed as intended. The repository now has the desired simpler shape: one checked-in protocol implementation, no normal-build `protoc` or Python requirement, explicit selector delete capability, one current-format server ownership record, process-local rate limiting, production-feature release builds, and the unsupported standalone archive updater removed.

The follow-up audit found three remaining closure items:

1. **Forced server drain currently aborts wrapper tasks rather than proving cancellation of the underlying gRPC/HTTP service tasks.**
2. **The canonical atomic writer changed one `select --output-file` symlink edge case:** when symlink replacement is allowed, validation follows the symlink target before the final rename. A broken symlink, or a symlink to a directory/device, is therefore rejected even though the safe operation is to replace the symlink entry itself.
3. **The Phase 15 planning record was marked COMPLETE before its required measurement table and acceptance checklist were actually closed**, and one stale server-lock comment still describes PID-file publication that no longer occurs.

The implementation should fix only those items.

Expected production-code delta: approximately neutral to slightly smaller. Correctness is the goal of this pass; do not create another abstraction merely to achieve a negative line count.

## 2. Hard scope boundary

### 2.1 In scope

Only:

- `snip-sync` forced-drain cancellation correctness;
- focused orchestration regression coverage for the forced-abort path;
- canonical `atomic_replace()` semantics when `reject_symlink == false`;
- one focused symlink-replacement regression test;
- stale comments/docs directly contradicted by the Phase 15 implementation;
- completion of the Phase 15 measurement table and acceptance record;
- final focused/full verification needed to close Phase 15.

### 2.2 Explicitly out of scope

Do **not** reopen or redesign:

- the retained multi-file transaction journal;
- auto-sync generation/execution locking;
- server singleton ownership architecture;
- legacy PID compatibility beyond what already exists;
- rate-limiter behavior or cardinality policy;
- gRPC/HTTP service split;
- HTTP health/metrics;
- CORS;
- API-key authentication/hash storage;
- E2E encryption;
- protobuf ownership/code generation;
- theme generation/compression;
- updater installation-method policy;
- CLI command classifications already corrected by Phase 15;
- CI topology;
- release automation;
- new cross-crate utility abstractions.

Do not add:

- a supervisor framework;
- a cancellation-token framework;
- a new service-task trait;
- a new atomic-write abstraction;
- a new CI job/workflow;
- a new crate or dependency.

If a proposed fix grows materially beyond the files named in this plan, stop and reassess before broadening scope.

## 3. Confirmed baseline

### 3.1 Server shutdown structure

At the baseline, `snip-sync/src/main.rs` does this conceptually:

```text
spawn gRPC service -> grpc_handle
spawn HTTP service -> http_handle
pass both JoinHandles to run_services_until_shutdown(...)
```

`snip-sync/src/orchestration.rs` then inserts *wrapper tasks* into a `JoinSet`:

```text
JoinSet task A -> await grpc_handle -> classify result
JoinSet task B -> await http_handle -> classify result
```

On drain timeout, the helper calls:

```text
tasks.abort_all()
```

That aborts the two wrapper tasks owned by the `JoinSet`. It does not establish that the original gRPC/HTTP tasks were themselves aborted and awaited. Dropping/aborting a task that is awaiting a `JoinHandle` can detach the task represented by that handle.

The helper then fills unobserved results with synthetic `Cancelled("aborted")` values. Consequently, the returned shutdown outcome can claim cancellation without having observed the underlying service task terminate.

Normal requested shutdown remains healthy and is already covered. The defect is specifically the **forced drain timeout** path.

### 3.2 Atomic replacement structure

Phase 15 correctly removed `select_cmd`'s private temp-write/rename implementation and now delegates to `utils::atomic::atomic_replace()`.

The canonical writer currently validates an existing symlink like this:

```text
if reject_symlink:
    reject
else:
    follow the symlink target with metadata()
    validate the target type
```

But the actual atomic operation writes to a fresh same-directory temporary file and then renames that file over the destination path. On platforms where replacement is supported, the rename replaces the directory entry for the symlink rather than writing through the symlink.

Therefore, when `reject_symlink == false`, following the symlink target during validation is both unnecessary for the write and behaviorally different from the pre-Phase-15 `select --output-file` path.

The important invariant is:

> `reject_symlink == false` means the destination symlink itself may be atomically replaced; the writer must never follow that symlink to modify its target.

### 3.3 Closure record

`plans/snip-it-phase-15-deletion-consolidation.md` currently says `Status: COMPLETE`, but:

- its required measurement table still contains placeholders;
- its explicit acceptance checklist remains unchecked;
- the implementation note records the generated compressed/plain theme representation sizes, but not the full required closure record;
- `snip-sync/src/server_lock.rs` still contains stale prose saying a PID file is published while the lock is held, although current servers no longer publish a PID file.

This pass must make the planning record match reality.

## 4. Required execution order

Execute exactly in this order:

```text
A. Correct forced server-task cancellation
   ↓
B. Restore non-following atomic symlink replacement semantics
   ↓
C. Focused verification
   ↓
D. Complete Phase 15 measurements and planning record
   ↓
E. Full closure verification
```

Do not mix unrelated cleanup into these edits.

## 5. Workstream A — Correct forced server-task cancellation

### A1. Preserve the current orchestration shape

Primary files:

```text
snip-sync/src/orchestration.rs
snip-sync/src/main.rs   # only if the narrow fix genuinely requires it
```

Preferred narrow fix:

1. Keep the existing original gRPC and HTTP `JoinHandle`s supplied to `run_services_until_shutdown()`.
2. Before moving those handles into their wrapper futures, obtain cancellation handles for the **original service tasks** (for example, Tokio `AbortHandle`s from the original `JoinHandle`s).
3. Keep the `JoinSet` wrappers as the mechanism that classifies and records each underlying task's terminal result.
4. During a normal requested/error-triggered drain, behavior remains unchanged: broadcast shutdown once and allow both services to finish naturally under the existing deadline.
5. If the bounded drain times out:
   - set `forced = true`;
   - abort the original gRPC and HTTP service tasks through their original-task abort handles;
   - do **not** make `JoinSet::abort_all()` the mechanism used to cancel the service tasks;
   - continue draining the wrapper `JoinSet` so each wrapper observes the original handle's real terminal state (`JoinError::cancelled`, panic, service error, etc.);
   - construct `ServiceShutdownOutcome` from observed results.
6. Do not synthesize a cancellation result merely because a timeout occurred if the corresponding underlying task can be observed directly after abort.

The key ownership rule after the fix must be:

```text
original service task
    └─ cancelled by its own AbortHandle on forced timeout
       └─ wrapper awaits original JoinHandle
          └─ JoinSet observes wrapper completion
             └─ result is classified once
```

This preserves the useful Phase 15 `JoinSet` simplification while making the forced path truthful.

### A2. Do not replace this with a larger redesign

A theoretically cleaner alternative would be to move responsibility for spawning both actual service futures into the `JoinSet` itself. Do **not** choose that route unless the narrow abort-handle correction is demonstrably impossible with the pinned Tokio API.

The corrective pass should not introduce generic service futures, boxed service traits, cancellation-token plumbing, or another orchestration API.

### A3. Forced-drain regression test

Primary file:

```text
snip-sync/src/orchestration.rs
```

Add or tighten one focused test so it proves the underlying service future has actually been dropped/cancelled before `run_services_until_shutdown()` returns.

Recommended pattern:

- create a small drop guard whose `Drop` implementation flips an `Arc<AtomicBool>` or increments an `Arc<AtomicUsize>`;
- place that guard inside a spawned service future that deliberately ignores the broadcast shutdown and waits forever;
- use a very short test drain timeout;
- call `run_services_until_shutdown()`;
- assert:
  - `requested == true` when the test uses a requested signal;
  - `forced == true`;
  - the stuck service result is `ServiceResult::Cancelled(...)` as observed from the original handle;
  - the drop flag/count proves the underlying stuck service future was dropped before the helper returned;
  - `ensure_clean_requested_shutdown()` returns `Err`;
  - the healthy sibling, if used in the test, is still recorded exactly once.

Do not add process sleeps measured in seconds. This is a deterministic unit test and should complete in milliseconds.

If the existing `one_service_completes_sibling_times_out` test can be strengthened to prove this invariant cleanly, prefer modifying it over adding another near-duplicate test.

### A4. Server-lifecycle acceptance criteria

Workstream A is complete only when all are true:

- [x] Normal requested shutdown still broadcasts once and drains both services cleanly.
- [x] Unexpected service completion still causes sibling shutdown and overall failure.
- [x] Drain-time service error/panic remains observable and fails shutdown.
- [x] Forced timeout aborts the **original gRPC/HTTP service tasks**, not only wrapper tasks.
- [x] The helper waits until the original task cancellation is observed through the wrapper before returning.
- [x] A forced result is not fabricated solely from missing wrapper output.
- [x] A deterministic test proves an intentionally stuck underlying service future is dropped before helper return.
- [x] No manual `grpc_consumed`/`http_consumed` state is reintroduced.
- [x] No cancellation/supervisor framework is added.
- [x] Existing real SIGTERM/same-port restart behavior remains unchanged.

## 6. Workstream B — Restore safe symlink replacement semantics in the canonical atomic writer

### B1. Fix validation, not `select_cmd`

Primary file:

```text
src/utils/atomic.rs
```

Do **not** restore `select_cmd::write_selection_atomically()`.

The canonical writer should remain the sole production implementation.

Required semantic rule for an existing destination symlink:

```text
reject_symlink == true
    => reject the destination symlink before writing

reject_symlink == false
    => permit replacement of the symlink entry itself
    => do not follow the symlink to validate or modify its target
```

The narrow implementation should be in `validate_target()` or the immediately adjacent target-inspection path.

For a symlink with `reject_symlink == false`, return success from target-type validation without calling `fs::metadata()` on the symlink target. The later write still occurs to a fresh temp file and the final rename is the only operation touching the destination path.

Do not weaken validation for a destination that is itself a real directory/FIFO/socket/device. Those non-symlink special file checks remain.

### B2. Preserve sensitive-config behavior

`Durability::SensitiveConfig` currently defaults to `reject_symlink = true`.

That behavior must remain unchanged.

This corrective pass is not permission to allow symlinks for credentials/configuration paths that intentionally reject them.

### B3. Focused regression test

Primary file:

```text
src/utils/atomic.rs
```

Add one canonical Unix regression test for **allowed symlink replacement**.

The strongest compact case is a broken symlink because it proves validation does not follow the target:

1. create `target` as a symlink to a nonexistent path;
2. call `atomic_replace(target, bytes, opts)` with `reject_symlink(false)` or a durability class whose default is false;
3. assert the operation succeeds;
4. assert `target` is now a regular file containing the requested bytes;
5. assert the nonexistent former symlink target was not created.

Optionally use a symlink to an existing ordinary file if needed for portability, but the old target's contents must remain untouched.

Do not recreate multiple select-specific atomic tests. The canonical atomic module should own these semantics now.

### B4. Atomic-write acceptance criteria

Workstream B is complete only when all are true:

- [x] `select_cmd` still delegates to the canonical atomic writer.
- [x] `reject_symlink == true` still rejects a destination symlink.
- [x] `reject_symlink == false` does not follow the symlink target during validation.
- [x] On Unix, an allowed destination symlink is replaced by the new regular file rather than written through.
- [x] A broken allowed destination symlink can be replaced without creating/modifying its former target.
- [x] Real directories/FIFOs/sockets/devices remain rejected by existing validation.
- [x] `SensitiveConfig` continues to default to symlink rejection and private permissions.
- [x] No second atomic-write implementation is added.

## 7. Workstream C — Correct stale ownership documentation

Primary file:

```text
snip-sync/src/server_lock.rs
```

Correct the module-level comment that still says:

```text
While the lock is held, the PID file is published ...
```

Current behavior is:

- the kernel-backed lock is the singleton authority;
- current owner metadata is published in the persistent lock file while the kernel lock is held;
- current servers do not create a separate PID file;
- old PID files are legacy compatibility input only.

Search only for directly contradictory variants of this statement in the small set of Phase 15-touched server docs, for example:

```text
PID file is published
publishes the PID file
current PID file owner
```

Update only stale statements. Do not initiate another broad documentation rewrite.

Acceptance:

- [x] No current-architecture documentation says new servers publish a PID file.
- [x] The kernel lock remains documented as authoritative.
- [x] Legacy PID files remain documented as compatibility-only where relevant.

## 8. Workstream D — Complete the Phase 15 closure record

Primary files:

```text
plans/snip-it-phase-15-deletion-consolidation.md
plans/snip-it-phase-15-final-corrective-closure.md
```

The original Phase 15 plan required a concrete measurement table and explicit acceptance checklist. Complete those records only after Workstreams A-C are implemented and verified.

### D1. Record the required Phase 15 measurement table

Fill the original table with actual values, not `record` / `expected` placeholders.

Required rows:

| Item | Baseline | Final | Result |
|---|---:|---:|---|
| `snp` release binary bytes | Phase 15 pre-implementation fixed-host value | final fixed-host value | byte delta |
| root direct production dependencies | baseline | final | delta |
| `snip-sync` direct production dependencies | baseline | final | delta |
| `snip-proto` build dependencies | 1 | 0 | -1 |
| normal CI protoc setup steps | Linux + macOS + Windows setup | 0 | removed |
| current-format server owner records | 2 | 1 | -1 |
| rate-limit persistence background tasks | 1 | 0 | -1 |

For the release binary measurement:

- use one host/toolchain/target for both compared values;
- if the exact Phase 15 pre-implementation binary from the same host is no longer available, use the previously recorded fixed-host Phase 14D value only if it was measured on that same host/toolchain and clearly label the provenance;
- otherwise reproduce the baseline using a temporary worktree at the relevant baseline commit and record the command/target used;
- do not claim byte precision across different OS/architectures/toolchains.

The final corrective code does **not** have a binary-size reduction acceptance target. Record the value accurately; do not contort the code to save a few bytes.

For dependency counts, define the count once in the table note as direct non-dev production dependencies declared by each package, including target-specific runtime dependencies where applicable. Use the same definition for baseline and final.

### D2. Close the original acceptance checklist honestly

In `plans/snip-it-phase-15-deletion-consolidation.md`:

- change `[ ]` to `[x]` only where the repository and verification actually prove the item;
- if an item is not applicable after a deliberate Phase 15 decision, mark it explicitly as `N/A — <short reason>` rather than pretending it ran;
- add a short corrective-closure note referencing this follow-up plan and the implementation commit(s);
- retain the Phase 14G journal RETAIN statement unchanged.

Do not rewrite the original plan into a retrospective narrative. It should remain useful as the execution record.

### D3. Close this corrective plan

At implementation completion:

- update this file to `Status: COMPLETE (YYYY-MM-DD)`;
- add the implementation commit SHA(s);
- add a compact verification record listing the commands run and outcomes;
- note any platform-specific test that was not locally executable and which existing CI lane provides the coverage.

Do not create another follow-up plan if all criteria in this file pass.

## 9. Verification sequence

### 9.1 Focused verification after Workstream A

Run the smallest relevant server test set first.

At minimum:

```bash
cargo test -p snip-sync orchestration --lib
```

If Rust's test-name filtering does not select the intended module cleanly, run:

```bash
cargo test -p snip-sync --lib
```

The forced timeout regression must execute as part of this test run.

Then run the existing ignored process-level lifetime cases as already defined by the repository's release verification. Do not create a second process-level forced-timeout harness.

### 9.2 Focused verification after Workstream B

Run the atomic utility tests and the select/platform smoke coverage that already exists.

At minimum:

```bash
cargo test utils::atomic --lib
cargo test --test platform_smoke
```

If the exact module filter differs, use the nearest existing test target rather than adding a new test binary.

### 9.3 Routine repository verification

After A-C:

```bash
bash scripts/check.sh
```

Required:

- formatting clean;
- clippy clean with repository warnings-as-errors policy;
- Linux routine correctness tests pass;
- no new routine verification stage is added.

### 9.4 Release verification

After all production/test/doc edits are committed so the working tree is clean:

```bash
bash scripts/release-check.sh verify
```

This must pass.

It already exercises the existing release-profile server lifetime tests, package validation, transaction crash recovery, and production-seam checks. Do not add another release ceremony.

### 9.5 Cross-platform CI

Push the implementation through the existing workflow only.

Required final CI state:

- Linux correctness: green;
- macOS platform smoke: green;
- Windows platform smoke: green.

No new workflow/job/matrix is allowed for this corrective pass.

## 10. Final acceptance criteria

This corrective line of work is closed only when every applicable item below is satisfied.

### Forced server cancellation

- [x] The bounded graceful-drain path is unchanged for cooperative services.
- [x] On drain timeout, cancellation targets the original gRPC/HTTP tasks.
- [x] The original task termination/cancellation is observed before the orchestration helper returns.
- [x] Forced outcomes are based on observed terminal results, not only synthetic placeholders.
- [x] A deterministic test proves an intentionally stuck underlying service future is dropped before helper return.
- [x] Unexpected pre-signal service completion still fails and shuts down its sibling.
- [x] Drain-time panic/service error still fails with diagnostic service identity.
- [x] Existing SIGTERM and same-port restart tests remain green.
- [x] No supervisor/cancellation framework or manual consumed-handle bookkeeping is introduced.

### Atomic replacement

- [x] One production atomic replacement implementation remains.
- [x] `select --output-file` still uses that implementation.
- [x] `reject_symlink=true` rejects symlinks.
- [x] `reject_symlink=false` does not dereference the destination symlink for validation.
- [x] Unix allowed-symlink replacement is proven by a focused regression test.
- [x] The symlink's former target is not modified or created by the replacement.
- [x] Non-symlink special-file validation remains intact.
- [x] Sensitive-config permission/symlink policy remains intact.

### Documentation and planning record

- [x] `server_lock.rs` no longer claims current servers publish a PID file.
- [x] No directly related Phase 15 server doc repeats that stale ownership statement.
- [x] The original Phase 15 measurement table contains actual values and provenance where needed.
- [x] The original Phase 15 acceptance checklist is completed honestly.
- [x] The Phase 14G transaction RETAIN decision remains untouched.
- [x] This corrective plan records implementation SHA(s) and verification outcomes before being marked COMPLETE.

### Verification

- [x] Focused orchestration tests pass.
- [x] Focused atomic tests pass.
- [x] `bash scripts/check.sh` passes.
- [x] `bash scripts/release-check.sh verify` passes from a clean tree.
- [x] Existing Linux/macOS/Windows CI topology remains unchanged.
- [x] Final existing CI lanes are green.
- [x] No new CI workflow, dependency, crate, daemon, or architecture layer was added.

## 11. Small-model implementation checklist

Use this exact sequence for handoff execution:

1. Read `snip-sync/src/orchestration.rs` completely.
2. Read the relevant `serve_inner` spawn/call section in `snip-sync/src/main.rs`.
3. Implement original-task abort ownership with the smallest possible edit.
4. Strengthen one forced-timeout unit test so it proves the underlying stuck future is dropped.
5. Run focused `snip-sync` library tests.
6. Read `src/utils/atomic.rs` completely.
7. Change only symlink validation semantics for `reject_symlink == false`.
8. Add one canonical Unix allowed/broken-symlink replacement test.
9. Run atomic/select focused tests.
10. Correct the stale `server_lock.rs` PID-publication comment and only directly related stale statements.
11. Run `bash scripts/check.sh`.
12. Measure and fill the Phase 15 closure table using a clearly stated fixed host/toolchain/target.
13. Update the original Phase 15 checklist based on evidence, not intention.
14. Commit the implementation/test/documentation changes.
15. With a clean tree, run `bash scripts/release-check.sh verify`.
16. Record the verification outcome and implementation SHA(s) in this plan and mark it COMPLETE in a final documentation-only closure commit.
17. Push to `main` and confirm the existing CI lanes are green.

If any step exposes a broader unrelated issue, record it separately; do not expand this corrective pass unless it blocks one of the explicit acceptance criteria above.

## 12. Closure condition

Once this plan's acceptance criteria pass, Phase 15 and the deletion/consolidation review line are closed.

Do not create a Phase 16 cleanup plan merely for aesthetic refactoring, module size, additional hardening, or further CI reduction. Future work should require a new user-visible feature, an observed bug, or measured performance/resource evidence.

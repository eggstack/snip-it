# Phase 12C — Collapse Auto-Sync to One Helper Process

Status: READY FOR IMPLEMENTATION

Baseline: `baa532dbb7dbd0876a1290737a2de93f7b009249`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

Prerequisites:

- Phase 12A complete.
- Phase 12B complete and focused tests green.

This phase removes the largest avoidable source of runtime and maintenance complexity in the client: the detached worker supervising a second executor subprocess for each auto-sync cycle.

The target remains asynchronous from the parent command, but not multi-layered. A single detached helper acquires the execution lock, debounces pending work, invokes the canonical sync operation directly, updates status, and exits.

No user-visible feature is removed.

---

## 1. Current and target models

### Current production model

```text
mutation command
  -> record pending generation
  -> spawn detached auto-sync-worker
      -> acquire execution lock
      -> debounce
      -> spawn auto-sync-execute
          -> run canonical sync
          -> conditionally clear pending
          -> exit with mapped code
      -> supervise timeout/termination
      -> infer acknowledgement from exit status + pending state
      -> update status
      -> exit
```

The second subprocess exists primarily so the worker can impose a hard timeout and classify exit codes. It also creates:

- duplicate hidden subcommands;
- child supervision and platform kill logic;
- executor exit-code taxonomy;
- acknowledgement inference after child exit;
- two process start paths;
- more test seams and event telemetry;
- several failure paths where process and lock lifetime can diverge.

### Target model

```text
mutation command
  -> record pending generation
  -> spawn detached auto-sync-helper
      -> acquire execution lock
      -> debounce
      -> capture generation G
      -> run canonical sync directly with bounded operation timeouts
      -> on confirmed success, clear only generation G
      -> if generation > G exists, run one bounded follow-up cycle
      -> record compact status
      -> release lock and exit
```

The helper can retain the current hidden command name `auto-sync-worker` to avoid unnecessary CLI churn. The architectural requirement is removal of the child executor, not renaming.

---

## 2. Required preserved behavior

Phase 12C must preserve:

- auto-sync remains disabled by default;
- post-mutation commands return without waiting for network synchronization;
- durable pending intent is recorded after successful local commit;
- concurrent parent commands may attempt helper spawn;
- only one helper performs sync because of the kernel execution lock;
- debounce and maximum-delay behavior remain configurable;
- explicit manual sync and cron use the same canonical sync operation and execution lock;
- exact-generation conditional clear prevents deleting newer work;
- newer pending generation triggers a bounded follow-up cycle;
- authentication/configuration failures require attention according to current policy;
- transient failures retain pending work and durable backoff;
- `snp sync retry`, `clear-failure`, `discard-pending`, `repair`, and `status` retain their command surface;
- hidden helper failure exits nonzero;
- platform support remains Linux, macOS, and Windows as currently documented.

---

## 3. Complexity budget

This is a deletion-oriented architectural pass.

Expected net result:

- fewer production modules or materially smaller worker/spawn modules;
- removal of executor child supervision code;
- removal of executor-only exit-code mapping where no longer used;
- reduction in integration tests that exist only to prove worker/executor handoff;
- no new runtime dependency;
- no new persistence file;
- no new public command.

The final production line count for `src/auto_sync/` should decrease. If it grows, the implementation must justify each added line and identify a larger deleted block.

Do not preserve obsolete abstractions merely to minimize the diff. Remove them cleanly once no production call site remains.

---

## 4. Explicit non-goals

Do not:

- make auto-sync in-process in the user’s foreground command;
- add a resident daemon;
- add a service manager integration;
- add a queue database;
- add IPC between parent and helper;
- add a heartbeat or lease protocol;
- add process groups solely for sync;
- add a third subprocess or external shell wrapper;
- redesign the sync protocol;
- redesign pending/status schemas unless a field becomes provably obsolete;
- change default debounce, maximum delay, timeout, or backoff values without a separate user-facing reason;
- add a new async runtime per library operation;
- duplicate `run_sync` logic inside the helper;
- expand CI or release automation;
- retain obsolete tests as a second “legacy architecture” suite.

---

# Workstream A — Define the single-helper execution contract

## Canonical entry point

The helper must invoke the existing canonical sync operation rather than reimplementing merge, encryption, client construction, or server interaction.

Preferred call boundary:

```rust
crate::sync_commands::run_sync(
    &settings,
    library_name,
    false,
    false,
    runtime,
)
```

If the current executor performs additional essential steps around `run_sync`, extract only those essential steps into a small internal function shared by manual sync and helper execution.

Do not preserve the executor module as a wrapper around one function solely for historical symmetry.

## Acknowledgement contract

The helper must define success from the canonical sync result, not from a child exit code.

For observed generation `G`:

1. Read valid pending state and capture `G`.
2. Run sync.
3. If sync returns success, call `clear_if_generation_matches(state_dir, G)`.
4. Interpret clear result:
   - `Cleared`: generation `G` acknowledged and removed;
   - `GenerationChanged { current > G }`: generation `G` completed, newer work preserved;
   - `Missing`: treat as success only when an explicit operation is allowed to clear it; otherwise log the unexpected state without recreating work;
   - lower/corrupt state: record internal failure and preserve evidence.
5. Record success for `G` only after the direct sync result is successful.

The helper no longer needs to infer remote acknowledgement from executor exit plus pending disposition.

## Acceptance criteria

- [ ] One internal function owns automatic sync execution.
- [ ] It calls the canonical sync implementation.
- [ ] Success does not depend on subprocess exit-code translation.
- [ ] Exact-generation conditional clear remains the only automatic pending deletion.
- [ ] Newer work is never deleted.

---

# Workstream B — Bound the direct sync operation without a child executor

## Goal

Retain practical timeout behavior without using a second process.

## Preferred approach

Use timeouts at the async/network boundaries already responsible for blocking:

- tonic endpoint connect timeout;
- per-request timeout;
- retry budget;
- overall helper cycle deadline checked between library sync operations;
- existing worker maximum lifetime.

The sync client already has request retry and timeout configuration. Reuse it. Do not wrap arbitrary blocking filesystem code in a new thread merely to cancel it.

If a bounded overall async future is straightforward, use:

```rust
runtime.block_on(tokio::time::timeout(policy.sync_timeout, sync_future))
```

only after the canonical sync path can be represented as an async future without duplicating a second runtime or nesting `block_on`. If that conversion is invasive, rely on the existing bounded network calls and worker lifetime, and document that filesystem operations are not force-cancelled.

The product does not require a process-kill guarantee for every local I/O stall.

## Runtime guidance

The client currently owns a lazy Tokio runtime. The helper may use the same runtime construction path as manual sync.

Do not:

- create a runtime inside each library iteration;
- run a runtime inside another runtime;
- convert the entire command application to async solely for this phase.

## Acceptance criteria

- [ ] Network connection/request waits remain bounded.
- [ ] Retry count and backoff remain bounded.
- [ ] Helper maximum lifetime remains enforced between cycles.
- [ ] No child process is needed for timeout enforcement.
- [ ] No unbounded new thread is introduced.

---

# Workstream C — Remove executor subprocess production paths

## Files to inspect

```text
src/auto_sync/executor.rs
src/auto_sync/spawn.rs
src/auto_sync/worker.rs
src/auto_sync/mod.rs
src/main.rs
src/outcome.rs
architecture/auto_sync.md
architecture/overview.md
docs/ARCHITECTURE_INVENTORY.md
AGENTS.md
```

Tests and `.skills` documentation may also reference executor behavior.

## Required removals

Remove production use of:

- `spawn_executor`;
- executor child wait/poll loop;
- terminate/force-kill helpers used only for executor supervision;
- `ExecutorCompletion` inference if no longer needed;
- executor-only exit-code classification if no other caller needs it;
- `auto-sync-execute` hidden command;
- `EXECUTOR_SUBCOMMAND` constant;
- executor-specific test event transitions;
- comments describing two processes per cycle.

Delete `src/auto_sync/executor.rs` if no useful shared logic remains. If it contains canonical classification that is still needed, move only that small logic to `worker.rs`, `policy.rs`, or a more appropriate existing module.

Do not retain an unused compatibility subcommand unless an actual installed-script compatibility requirement is documented. Hidden commands are internal implementation details; removal is acceptable.

## Spawn module target

After simplification, `spawn.rs` should primarily:

- locate the current executable;
- construct `auto-sync-worker --state-dir ...`;
- detach it by platform;
- route standard streams;
- return child PID or a spawn error.

It should not supervise a second process.

## Acceptance criteria

- [ ] A production source search finds no `spawn_executor` call.
- [ ] A production source search finds no `auto-sync-execute` dispatch.
- [ ] No child wait/kill loop remains in auto-sync code.
- [ ] Documentation describes one helper process.
- [ ] Net auto-sync production code decreases.

---

# Workstream D — Simplify worker state transitions

## Required cycle

The helper cycle should remain straightforward:

```text
acquire execution lock
resolve policy
if disabled -> preserve pending and exit no-op
read pending
perform bounded debounce
preflight re-read
run canonical sync for observed generation G
on success -> exact-generation clear
on failure -> preserve pending and record status/backoff
if newer generation exists and lifetime remains -> repeat
otherwise exit
```

## Follow-up bound

Preserve the existing maximum worker lifetime. The helper may process newer generations while time remains, but must not become a resident loop.

A simple loop bounded by `worker_lifetime` is sufficient. Do not add a maximum-cycle counter unless the existing policy already has one or a reproduced starvation case requires it.

## State simplification

Re-evaluate and remove types that existed only to classify executor completion:

```text
ExecutorCompletion
SpawnResult (if unused)
executor exit code -> FailureClass mapping
noop-success executor seam
```

Retain:

```text
WorkerOutcome
DebounceResult
PendingState
ScheduleDecision/ScheduleError
FailureClass
```

Do not combine all remaining states into one large enum.

## Failure mapping

Map direct `SnipError`/`SyncFailureKind` to the existing `FailureClass` through one bounded helper. Reuse current classification where possible.

Required broad categories remain:

- authentication/credential/configuration -> attention/no automatic retry;
- transient connection/timeout/server unavailable -> bounded retry;
- local persistence/corruption/internal -> visible failure with pending preserved;
- successful no-op sync -> success only when canonical sync confirms no work was required.

Do not create a detailed error ontology for every error string.

## Acceptance criteria

- [ ] Worker cycle can be understood from one primary function without subprocess handoff.
- [ ] Newer generations remain preserved and processed when bounded time remains.
- [ ] Failure classification remains compatible with status/backoff behavior.
- [ ] No worker becomes permanently resident.

---

# Workstream E — Reduce tests to the new architecture

## Test strategy

Retain tests for behavioral contracts, not deleted implementation layers.

Required focused coverage:

1. pending generation recorded after mutation;
2. concurrent helpers: one kernel-lock winner, other exits no-op;
3. debounce observes newer generation;
4. direct successful sync clears exact generation;
5. newer generation appearing during sync remains;
6. transient failure preserves pending and records backoff;
7. authentication/configuration failure preserves pending and requires attention;
8. corrupt/lower generation fails closed;
9. helper failure exits nonzero;
10. manual sync and helper cannot execute concurrently.

Remove or rewrite tests whose only purpose is:

- executor exit-code mapping;
- worker killing executor;
- executor noop-success seam;
- event ordering between worker and executor processes;
- `auto-sync-execute` CLI behavior.

Do not keep both old and new architecture tests.

## Test seam guidance

Prefer injecting a small sync-operation function/trait into worker unit tests if needed:

```rust
trait SyncOperation {
    fn run(&self, ...) -> SnipResult<()>;
}
```

Use this only inside the auto-sync module or under test configuration. Do not expose a public generalized sync backend.

An even smaller closure/function parameter is preferred if it keeps production code clear.

Do not add a mocking dependency.

## Acceptance criteria

- [ ] Tests prove user-visible durability/concurrency behavior.
- [ ] Executor-specific tests are removed or rewritten.
- [ ] Total test complexity decreases or remains materially smaller than the deleted architecture proof.
- [ ] No new test helper binary is added.

---

# Workstream F — Documentation and public surface cleanup

## Required updates

Update current architecture and contributor documentation to state:

- one detached helper process per spawn attempt;
- helper owns the execution lock for the full cycle;
- helper runs canonical sync directly;
- pending exact-generation clear is the acknowledgement boundary;
- persistent lock metadata is diagnostic only;
- helper is opportunistic and bounded, not a daemon.

Likely files:

```text
architecture/auto_sync.md
architecture/overview.md
docs/ARCHITECTURE_INVENTORY.md
docs/LOGICAL_LAYERS.md
AGENTS.md
.skills/auto-sync* (only if present/current)
```

Historical plan documents may remain historical. Do not rewrite all old phase plans.

Review public exports in `src/lib.rs`. Any executor-only public types should be removed or made internal. Follow semver pragmatically: this crate’s public surface should not preserve implementation-only types solely because integration tests once imported them.

## Acceptance criteria

- [ ] Current docs contain no claim of a two-process production cycle.
- [ ] Public API no longer exports executor-only internals.
- [ ] Historical plans are not mass-edited.
- [ ] No new architecture document is required if existing docs can be corrected.

---

## 5. Recommended commit structure

Preferred implementation commits:

```text
phase-12c: run auto-sync directly in detached helper
phase-12c: remove executor paths and align tests/docs
```

A single commit is acceptable if review remains readable. Do not split into many mechanical deletion commits.

---

## 6. Verification commands

Focused commands should include the actual current test names after executor tests are rewritten:

```text
cargo fmt --all -- --check
cargo test -p snip-it auto_sync --all-features -- --test-threads=1
cargo test --test auto_sync_lifecycle --features test-support -- --test-threads=1
cargo test --test auto_sync_mutations --features test-support -- --test-threads=1
cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1
cargo check --workspace --all-targets --all-features
bash scripts/check.sh
```

Run only relevant sync integration tests needed to prove direct invocation. Do not require all deep crash/restore/protocol tests unless a touched path is otherwise unverified.

Platform CI should provide compilation/smoke proof for detachment on macOS and Windows. Do not add local cross-compilation infrastructure.

---

## 7. Prohibited outcomes

The phase fails if it:

- leaves the worker spawning an executor in production;
- moves network sync back into the foreground mutation command;
- weakens exact-generation clear semantics;
- allows concurrent manual and automatic sync execution;
- removes durable pending or status recovery;
- creates a resident daemon;
- adds another process, lock, queue, database, or dependency;
- duplicates canonical sync logic;
- retains obsolete executor tests and telemetry as dead complexity;
- changes user-visible auto-sync commands or defaults without need;
- expands CI/release machinery.

---

## 8. Closure checklist

- [ ] Helper invokes canonical sync directly.
- [ ] Executor subprocess production path is removed.
- [ ] `auto-sync-execute` is removed or has no production use and a documented temporary removal deadline.
- [ ] Exact-generation clear remains correct.
- [ ] Newer-generation preservation remains correct.
- [ ] Manual and automatic sync share one execution lock.
- [ ] Network/retry waits remain bounded.
- [ ] Worker remains detached and bounded, not resident.
- [ ] Obsolete tests and docs are removed or rewritten.
- [ ] Auto-sync production line count decreases.
- [ ] Focused tests pass.
- [ ] `cargo check --workspace --all-targets --all-features` passes.
- [ ] `bash scripts/check.sh` passes.
- [ ] Plan records implementation SHA and verification commands.

When all items are satisfied, mark Phase 12C COMPLETE. Do not create a follow-up supervisor, watchdog, or hardening phase. Remaining optimization belongs only to measured Phase 12D work.
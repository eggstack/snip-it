# snip-it Correctness, Reliability, and Product-Closure Roadmap

## Status

Planning document for the post-Release-5 correctness, reliability, durability, and maintainability program.

This roadmap does not replace the existing Pet-compatibility Release 5 plans. Those files remain historical records of the work that produced the current implementation. This program begins from the current `main` branch and uses numbered corrective phases to avoid colliding with the existing Release 5G and 5H names.

## Executive summary

`snip-it` already fulfills its primary product goal: it is a fast, terminal-first snippet manager with editable TOML storage, fuzzy selection, variable expansion, libraries, clipboard and execution workflows, Pet compatibility, shell integration, themes, import/export, local usage metadata, and optional encrypted synchronization.

The local snippet-management path is comparatively mature. The highest-risk area is the recently introduced detached auto-sync architecture. The current executor subprocess contains a placeholder success path: it validates configuration, performs no sync, returns exit code zero, and allows the worker to clear pending state. The current source also contains contradictory execution-lock ownership language, weak failure propagation, and incomplete end-to-end coverage. These are correctness defects rather than missing convenience features.

The program therefore follows this order:

1. Restore truthful synchronization semantics.
2. Make pending-state and debounce behavior lossless and efficient.
3. Introduce typed failure handling and bounded retry behavior.
4. Make synchronization state visible and recoverable.
5. Prove the architecture through real process/server integration tests.
6. Tighten internal boundaries and the public Rust API.
7. Strengthen local data durability, backup, validation, and migration behavior.
8. Improve deterministic CLI automation without broadening product scope.
9. Close security, packaging, and release-process gaps.

No daemon, resident service, distributed event system, plugin framework, workflow engine, or hosted synchronization service is proposed. The detached one-shot worker plus supervised executor remains the preferred architecture once its invariants are correct and observable.

## Program invariants

The implementation produced by this roadmap must satisfy the following non-negotiable invariants.

### Local-first invariants

1. A successful local mutation is durable independently of network state.
2. Network failure cannot roll back or corrupt a committed local snippet change.
3. Commands, descriptions, tags, outputs, and metadata are never silently rewritten except through documented migration or normalization behavior.
4. User-owned TOML remains editable and recoverable.
5. A failed background operation cannot retroactively change the exit status of an already completed mutation command.

### Synchronization invariants

1. Executor success means a real synchronization comparison completed; placeholder or optimistic success is forbidden.
2. Pending work is cleared only after successful synchronization or a proven already-current result.
3. Disabled, unconfigured, timed-out, crashed, conflicted, partially completed, or persistence-failed synchronization preserves pending intent.
4. Exactly one synchronization operation may execute at a time across detached worker, manual sync, cron, and explicit `--sync` entry points.
5. Lock ownership is singular and documented; the worker and executor must never contend for the same execution lock.
6. Timeout supervision must terminate and reap the executor before releasing synchronization exclusivity.
7. A newer pending generation cannot be cleared by an older worker.
8. Recovery never increments generation or discards valid pending state because of age.
9. Local-only metadata remains local-only unless the data contract explicitly changes.
10. No command payload, API key, encryption key, or sensitive response body is written to worker argv, lock files, pending state, status state, or logs.

### Testing invariants

1. Tests must verify user-visible effects, not merely lock-file disappearance or process exit.
2. At least one mandatory end-to-end test must prove that a local mutation reaches a real ephemeral `snip-sync` server before pending state clears.
3. Failure tests must prove pending preservation.
4. Timing-sensitive behavior must use barriers, recording servers, fake clocks, or bounded polling rather than arbitrary sleeps as correctness evidence.
5. Linux, macOS, and Windows must exercise platform-sensitive process, file, and lock behavior.
6. Required invariants cannot be hidden behind ignored tests.

## Phase sequence

### Phase 01 — Auto-Sync Correctness Closure

Detailed handoff: `plans/snip-it-correctness-01-auto-sync-correctness-closure.md`

This phase is release-blocking. It replaces the stub executor with the real canonical synchronization operation, establishes worker-owned execution-lock semantics, makes success truthful, preserves pending state on every non-success result, and adds an end-to-end regression test that fails against the current implementation.

Exit condition: no path can report successful detached synchronization without a real server comparison, and pending state cannot clear on false success.

### Phase 02 — Pending-State and Debounce Semantics

Detailed handoff: `plans/snip-it-correctness-02-pending-debounce-semantics.md`

This phase formalizes pending-state invariants, separates sync enablement from automatic execution, promotes the newest generation during debounce, rechecks pending state before execution, and distinguishes quiet-period debounce from maximum-delay behavior.

Exit condition: mutation bursts normally collapse into one sync, stale work is not executed, disabled auto-sync does not silently discard intent, and deterministic tests cover the state machine.

### Phase 03 — Failure Classification and Retry Policy

Detailed handoff: `plans/snip-it-correctness-03-failure-classification-retry.md`

This phase makes the existing exit-code taxonomy operational through typed internal errors, durable status, failure-class-specific retry policy, process-independent backoff, and worker-storm prevention.

Exit condition: every major failure has defined persistence, retry, and operator-facing semantics; the implementation no longer records generic `unknown` when a specific class is available.

### Phase 04 — Operational Visibility and Recovery

Detailed handoff: `plans/snip-it-correctness-04-operational-visibility-recovery.md`

This phase adds a read-only `snp status` surface, integrates synchronization state into `doctor`, exposes explicit safe recovery actions, adds a minimal TUI indicator, and improves bounded structured logging.

Exit condition: a user can determine whether changes are current, pending, deferred, or failed without inspecting internal files or enabling debug environment variables.

### Phase 05 — End-to-End Test Architecture

Detailed handoff: `plans/snip-it-correctness-05-end-to-end-test-architecture.md`

This phase builds a reusable isolated harness around real `snp` and `snip-sync` processes, adds barrier-driven fault injection, validates detached process lifecycle and sync mutual exclusion, tests packaged artifacts, and closes the cross-platform matrix.

Exit condition: the critical architecture is proven at process and server boundaries, and the current stub implementation would be rejected by mandatory CI.

### Phase 06 — Core Architecture and Public API Tightening

Detailed handoff: `plans/snip-it-correctness-06-core-api-architecture-tightening.md`

This phase narrows accidental public modules, defines core/sync-client/CLI boundaries, adds semver protections, evaluates feature gates, and removes obsolete coordinator and transitional architecture.

Exit condition: public API is intentional, CLI internals are not accidentally stable, and the active architecture has one source of truth.

### Phase 07 — Local Data Durability and Recovery

Detailed handoff: `plans/snip-it-correctness-07-local-data-durability-recovery.md`

This phase standardizes atomic persistence, adds backup and validation primitives, defines snippet identity across migration/import/sync, and makes migrations idempotent and recoverable.

Exit condition: local data has a documented durability model, users can back up and validate installations, and repair operations are conservative and reversible.

### Phase 08 — CLI and Automation Polish

Detailed handoff: `plans/snip-it-correctness-08-cli-automation-polish.md`

This phase adds deterministic non-TUI retrieval, standardizes exit-code and stdout/stderr contracts, reconciles overlapping command semantics, and improves scriptability without turning `snip-it` into a workflow engine.

Exit condition: shell scripts and agents can retrieve snippets deterministically, while interactive commands retain clear, narrow contracts.

### Phase 09 — Security and Release Hardening

Detailed handoff: `plans/snip-it-correctness-09-security-release-hardening.md`

This phase updates the threat model, audits secret and subprocess handling, validates process-group termination, strengthens supply-chain and package checks, and establishes release-blocking quality gates.

Exit condition: behavior-critical TODOs block release, process and secret boundaries are tested, and release artifacts have complete automated evidence.

## Dependency graph

The phases are intentionally ordered, but some work may overlap after Phase 01.

```text
Phase 01: truthful sync and lock ownership
   |
   +--> Phase 02: pending/debounce state machine
   |       |
   |       +--> Phase 03: failure/retry policy
   |               |
   |               +--> Phase 04: status and recovery UX
   |
   +--> Phase 05: end-to-end harness and process evidence

Phase 01-05 correctness baseline
   |
   +--> Phase 06: architecture/API tightening
   +--> Phase 07: local durability/recovery
   +--> Phase 08: deterministic CLI polish
           |
           +--> Phase 09: security and release closure
```

Phase 05 test-harness scaffolding may begin during Phases 01-04, but closure of Phase 05 depends on the final semantics from those phases. Phase 06 should not perform broad crate movement while the sync correctness path is still changing. Phase 09 is the final release gate and depends on all previous phases.

## Recommended release strategy

### Correctness train

Phases 01-05 should be treated as one correctness train. During this sequence:

- avoid unrelated feature additions;
- retain compatibility unless a behavior is provably unsafe or false;
- prefer small commits that preserve bisectability;
- add regression tests before or with each fix;
- update architecture documentation in the same commit as semantic changes;
- do not publish a release claiming reliable auto-sync until Phase 01 is complete;
- do not call the detached architecture closed until Phase 05 is complete.

### Maintainability train

Phases 06-09 may ship incrementally after correctness closure. They should avoid changing the fundamental snippet-storage format without a migration and rollback story.

## Cross-phase implementation rules

### One canonical synchronization function

All entry points must delegate to one canonical operation that accepts resolved settings/direction and returns a typed report or typed error. CLI, cron, worker executor, and explicit `--sync` wrappers may differ in lock acquisition and presentation, but not in synchronization semantics.

### Explicit process ownership

The final detached topology should be:

```text
mutation command
  -> atomically records pending generation
  -> spawns detached debounce worker
  -> returns immediately

detached worker
  -> owns synchronization execution lock
  -> debounces latest pending generation
  -> spawns supervised, non-detached executor
  -> terminates/reaps executor on timeout
  -> conditionally clears matching generation only on true success

executor
  -> performs canonical synchronization
  -> does not reacquire worker-owned execution lock
  -> exits with stable internal result code
```

### Durable state separation

Behavior-driving pending intent and operator-facing attempt status should remain separate unless a stronger transactional design justifies combining them.

Suggested state artifacts:

- `auto-sync-pending.toml`: generation and mutation intent.
- `auto-sync-status.toml`: last attempt, failure class, backoff, and timestamps.
- `auto-sync-pending.lock`: short transaction lock.
- `auto-sync-worker.lock`: worker scheduling/ownership if still needed.
- `sync-execution.lock`: synchronization exclusivity.

Each artifact must have a single owner, bounded schema, restrictive permissions, atomic writes, corruption handling, and no secrets.

### Compatibility discipline

- Existing Pet-compatible TOML remains canonical.
- Legacy aliases remain readable until deliberately deprecated.
- New commands must preserve machine-readable stdout contracts.
- Internal worker/executor commands remain hidden and unsupported.
- Configuration additions require defaults, bounds, documentation, migration tests, and backward-compatible parsing.

## Program-wide test matrix

At program completion, CI should cover:

### Static and package checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
cargo package --workspace
cargo deny check
```

### Local behavior

- exact stdin/file/editor ingestion;
- TOML round trips and migration;
- variable and choice expansion;
- library create/delete/move/primary behavior;
- import/export and Pet compatibility;
- output/favorite/folder/usage metadata contracts;
- clipboard and run success accounting;
- TUI cancellation and exit behavior;
- shell integration output purity.

### Sync behavior

- initial push and pull;
- bidirectional merge;
- already-current no-op;
- stable identity;
- local-only metadata exclusion;
- concurrent entry-point serialization;
- pending preservation for every failure class;
- mutation during debounce and active sync;
- timeout termination and reap;
- worker restart and stale recovery;
- malformed/corrupt control artifacts;
- no secret leakage.

### Platforms

- Linux x86_64;
- macOS;
- Windows x86_64;
- package/install smoke tests on supported release targets where practical.

## Documentation requirements

Each phase must update the relevant subset of:

- `README.md`;
- `USER_GUIDE.md`;
- `AGENTS.md` or linked agent guidance;
- architecture overview and sync deep dives;
- configuration reference;
- command help text;
- `CHANGELOG.md`;
- security documentation;
- plan closure/status files.

Documentation must describe the current implementation, not the intended next implementation. Remove obsolete release-specific commentary from production module docs once the final architecture is stable.

## Program non-goals

The following are explicitly outside this roadmap:

- long-running daemon;
- OS service installation for the client;
- generalized job scheduling;
- remote command execution;
- arbitrary plugin execution;
- workflow graphs;
- collaborative real-time editing;
- hosted account service;
- CRDT introduction without a separate design program;
- replacing user-editable TOML with an opaque database;
- turning `snip-sync` into an application platform;
- secrets-manager behavior beyond safe credential storage for sync.

## Final completion definition

This roadmap is complete only when all of the following are true:

1. Every detached sync success corresponds to a real completed synchronization comparison.
2. Pending work is never silently discarded.
3. Exactly one sync operation runs at a time across all entry points.
4. Executor timeout is truthful, killable, and fully reaped before unlock.
5. Mutation bursts debounce against the latest generation.
6. Failure classes, retry state, and operator status are durable and safe.
7. A real end-to-end harness proves remote effects and negative preservation behavior.
8. Public API and crate boundaries are intentional.
9. Local data has backup, validation, migration, and recovery evidence.
10. Deterministic scripting is available without broadening product scope.
11. Threat model, subprocess handling, packaging, and release gates reflect the actual architecture.
12. Linux, macOS, and Windows CI pass all required invariants without ignored tests.
13. The project remains a lightweight, terminal-first snippet manager rather than an automation platform.

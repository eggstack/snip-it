# Release 5D Plan: Auto-Sync Integration, Failure Hardening, and Release Closure

## Purpose

Validate and close Release 5 after the policy/configuration layer, coordinator, debounce logic, and mutation triggers have landed.

This phase is not a new feature track. It is a corrective, security, compatibility, and release-readiness pass proving that optional auto-sync improves convenience without weakening local durability, encrypted synchronization, command behavior, or process reliability.

## Entry Conditions

Before beginning:

- Release 5A typed policy and configuration are implemented;
- Release 5B coordinator/debounce/process-lifecycle design is implemented;
- Release 5C mutation triggers are wired;
- auto-sync remains disabled by default;
- Releases 1–4 regression suites remain green;
- the implementation documents whether pending intent is best-effort or durable.

If implementation diverged from earlier plans, document the final shipped contract before validating it.

## Closure Invariants

1. A local mutation is durable before auto-sync begins.
2. Remote failure never removes or rewinds a successful local mutation.
3. Auto-sync cannot recurse through sync-merge writes.
4. Rapid mutations coalesce without losing the fact that synchronization is needed.
5. Manual and scheduled sync retain unchanged semantics.
6. Auto-sync is opt-in and independently disableable.
7. No command payload, output metadata, credentials, encryption material, or secret URL components enter logs, status files, lock files, or process arguments.
8. Interactive commands have bounded latency and no terminal lifecycle regressions.
9. Machine-facing stdout remains byte-compatible unless an additive schema change was explicitly introduced.
10. Failure status is truthful: best-effort behavior is not documented as durable delivery.
11. No local-only field is accidentally uploaded.
12. All pending/lock/status files are bounded, versioned, restrictive, and recoverable.

## Workstream A: Architecture Reconciliation

Audit the final implementation across:

- auto-sync configuration ownership;
- policy resolution;
- mutation notification API;
- coordinator state machine;
- cross-process locking;
- pending-state persistence;
- sync executor reuse;
- manual/explicit sync interaction;
- trigger matrix;
- diagnostics/status rendering.

Create or update architecture documentation with one canonical data flow:

```text
mutation command
  -> validate
  -> atomic local commit
  -> mutation event
  -> policy gate
  -> coordinator enqueue
  -> debounce/coalesce
  -> existing encrypted sync executor
  -> status update
```

Remove or consolidate duplicate scheduling paths. No command should call separate ad hoc auto-sync logic.

## Workstream B: Local-First Durability Matrix

For every trigger-enabled mutation path, inject failures at each boundary:

1. before local write;
2. during local write;
3. after local write but before notification;
4. during notification persistence;
5. during debounce;
6. before remote connection;
7. during authentication;
8. during encryption/decryption;
9. during upload/download;
10. during merge/save of remote results;
11. during status cleanup.

Assert:

- incomplete local mutations fail without scheduling;
- completed local mutations remain readable after every later failure;
- backups/tombstones retain existing semantics;
- retry/manual sync can recover;
- no corrupted coordinator state blocks future commands.

## Workstream C: Debounce and Coalescing Correctness

Use fake time and deterministic executors to prove:

- N mutations inside one debounce window produce one sync attempt;
- continuous mutations cannot postpone sync beyond the documented maximum;
- mutation during active sync schedules at most one follow-up;
- different library scopes coalesce according to the documented model;
- stale pending markers recover after restart;
- disabled auto-sync prevents all scheduling;
- manual sync consumes or reconciles pending state correctly;
- explicit post-command sync does not create a duplicate delayed attempt.

Do not rely on wall-clock sleeps except minimal process-level smoke tests.

## Workstream D: Cross-Process and Concurrency Hardening

Test multiple simultaneous CLI processes:

- creating/editing different snippets;
- mutating the same library;
- emitting coordinator requests concurrently;
- manual sync racing auto-sync;
- process crash while holding coordinator lock;
- stale lock owner PID reuse where relevant;
- filesystem rename/atomicity behavior.

Required outcomes:

- no permanent deadlock;
- no unbounded sync storm;
- no malformed state file;
- no lost pending intent under the documented durability model;
- no duplicate remote attempts beyond the allowed follow-up behavior;
- existing library locking/data-integrity behavior remains intact.

Document any unavoidable last-writer-wins behavior.

## Workstream E: Failure Policy Closure

Validate each supported failure mode.

### Ignore

- local command result unchanged;
- no user-facing warning;
- bounded status/debug information only.

### Warn

- local command remains successful;
- concise stderr warning only when semantically possible;
- asynchronous failures appear in status/doctor rather than an unrelated terminal;
- no stdout contamination.

### Error

If supported:

- exact scope is documented;
- local success is explicit;
- exit code is stable;
- asynchronous future failure is not retroactively represented as command failure;
- scripts can distinguish local mutation failure from post-commit sync failure.

If the architecture cannot support truthful `error`, remove or reject it rather than shipping misleading behavior.

## Workstream F: Manual and Scheduled Sync Regression

Pin existing behavior for:

- `snp sync` directions;
- explicit command-level sync flags;
- `snp cron` generated output;
- service/daemon installation if present;
- sync timeout/environment variables;
- conflict resolution;
- encryption/key lookup;
- library linking/mapping;
- offline/manual retry.

Auto-sync configuration must not silently alter these paths.

When manual sync succeeds, verify pending auto-sync status is cleared or reconciled according to the documented model.

## Workstream G: Trigger Matrix Audit

Construct a table from implementation and tests:

| Operation | Local change | Remote-syncable | Auto-sync event | Notes |
|---|---:|---:|---:|---|
| new snippet | yes | yes | yes | after save |
| edit command | yes | yes | yes | after save |
| output-only edit | yes | no | no | local-only field |
| delete/tombstone | yes | yes | yes | after tombstone save |
| import dry-run | no | no | no | read-only |
| import merge no-op | no | no | no | no event |
| import create/replace | yes | yes | one | logical transaction |
| set primary library | local metadata | no | no | local-only |
| sync merge write | yes | already sync | no | recursion suppressed |

Expand this table to every current mutation surface and reconcile code, tests, and docs.

## Workstream H: Security and Privacy Audit

Use sentinel values in:

- commands;
- descriptions;
- output/notes;
- tags;
- source paths;
- server URLs with userinfo/query tokens;
- API keys;
- encryption keys;
- account IDs.

Inspect:

- stdout/stderr;
- human and JSON status;
- logs/audit logs;
- pending marker;
- lock file;
- status file;
- process argv;
- environment passed to helper process;
- crash diagnostics.

Assert sentinel values are absent except where a user explicitly requests raw local snippet output unrelated to auto-sync.

Additional checks:

- restrictive permissions from creation time;
- symlink-resistant state-file creation;
- bounded file reads;
- version validation;
- no arbitrary helper-mode path injection;
- no shell interpretation;
- redaction of sensitive URL components.

## Workstream I: Process Lifecycle and Platform Validation

Validate:

- short-lived CLI exits;
- debounce owner lifecycle;
- helper detachment if used;
- terminal descriptor inheritance;
- SIGINT/SIGTERM;
- abrupt kill;
- OS shutdown approximation;
- Windows process and locking behavior;
- Unix file permissions;
- no zombie/orphan accumulation;
- bounded worker lifetime.

Interactive PTY tests must prove:

- terminal restoration;
- no delayed text injected into alternate-screen mode;
- no additional blocking beyond documented bounds;
- cancellation still returns established exit codes.

## Workstream J: Status and Recovery UX

Ensure users can determine:

- enabled/disabled state;
- effective debounce/failure policy;
- pending/running state;
- last success;
- last bounded failure class;
- recommended recovery command.

Use `doctor`, `sync status`, or the chosen existing surface.

Requirements:

- no secrets;
- stable JSON if machine-readable;
- stale status detection;
- clear manual recovery guidance;
- explicit statement that local mutation succeeded when applicable.

## Workstream K: Scale and Resource Tests

Test:

- 1,000 rapid mutation events;
- sustained mutations across maximum coalescing window;
- many libraries or mapping scopes;
- corrupt/large pending state;
- repeated offline failures;
- repeated startup recovery;
- file-descriptor/process/thread counts;
- memory growth;
- coordinator cleanup.

Set practical bounds for:

- state file size;
- lock wait;
- debounce window;
- maximum coalescing delay;
- sync timeout;
- worker lifetime;
- log message length.

Avoid tests that consume excessive CI wall-clock time; use fake clocks/executors.

## Workstream L: Schema and Compatibility Tests

Pin:

- old config files load with auto-sync disabled;
- new config round-trips;
- unknown future fields behavior;
- status/marker version handling;
- manual sync JSON/output schemas;
- mutation command stdout schemas;
- doctor/status JSON additions are additive;
- no auto-sync fields enter snippet TOML, ProtoSnippet, import/export schema, backup content, or usage sidecar.

## Workstream M: Documentation Reconciliation

Update and cross-check:

- README;
- USER_GUIDE;
- architecture sync/config/mutation docs;
- CLI stream and exit-code policy;
- PET compatibility matrix;
- CHANGELOG;
- AGENTS.md;
- config examples;
- troubleshooting.

Documentation must state:

- disabled by default;
- local-first semantics;
- debounce/coalescing behavior;
- best-effort versus durable pending intent;
- failure policy;
- manual recovery;
- fields that do not synchronize;
- no rollback on remote failure;
- no guarantee of immediate remote durability.

Remove speculative options not implemented.

## Workstream N: Test Organization

Release 5 will add substantial concurrency and process-lifecycle coverage. Avoid further uncontrolled growth of monolithic test files.

Suggested files:

```text
tests/auto_sync_config.rs
tests/auto_sync_coordinator.rs
tests/auto_sync_mutations.rs
tests/auto_sync_security.rs
tests/auto_sync_concurrency.rs
tests/auto_sync_regression.rs
```

Reuse shared support helpers and fake executors/clocks.

## Final Validation Commands

Run at minimum:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
cargo test --test auto_sync_config
cargo test --test auto_sync_coordinator
cargo test --test auto_sync_mutations
cargo test --test auto_sync_security
cargo test --test auto_sync_concurrency
cargo test --test release4_regression
```

Also run the normal platform CI matrix and inspect hosted status rather than relying only on local claims.

## Release 5 Exit Criteria

Release 5 is complete only when:

- auto-sync is opt-in and disabled by default;
- local commits always precede remote work;
- remote/scheduling failure never rolls back local state;
- rapid mutations coalesce with bounded delay;
- process crashes and stale locks recover safely;
- mutation triggers are complete and non-recursive;
- output and usage remain local-only;
- manual/scheduled sync behavior remains unchanged;
- failure modes and delivery guarantees are truthful;
- no secrets or snippet content leak through coordinator artifacts;
- platform, concurrency, PTY, security, schema, regression, and scale suites pass;
- documentation matches the shipped behavior exactly.

## Non-Goals

- Exactly-once remote delivery.
- New sync providers.
- Hosted plaintext synchronization.
- Synchronizing usage or output metadata.
- General offline operation journal.
- Replacing manual or scheduled sync.
- Implementing external libraries.

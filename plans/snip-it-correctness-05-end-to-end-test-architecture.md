# Phase 05: End-to-End Test Architecture

## Purpose

Build a reusable test architecture that proves `snip-it` behavior across real process, filesystem, credential, protocol, and server boundaries.

The current suite is broad, but the placeholder executor defect demonstrates that test quantity does not substitute for semantic evidence. This phase makes the critical guarantees release-blocking and deterministic.

## Preconditions

Phases 01-04 should define the final behavior for:

- canonical sync execution;
- worker/executor lock ownership;
- pending and debounce semantics;
- failure classification/backoff;
- status and recovery operations.

Harness scaffolding may land earlier to support those phases. This plan closes and standardizes it.

## Test philosophy

1. Verify observable effects, not implementation proxies.
2. Prefer real binaries and a real ephemeral server for contract tests.
3. Use unit tests for pure state transitions and integration tests for process/protocol behavior.
4. Use barriers, event recording, fake clocks, and bounded polling rather than arbitrary sleeps.
5. Assert exact behavior where the contract is exact.
6. Preserve failure artifacts only after redaction and only when useful.
7. Treat Windows process behavior as first-class rather than disabling difficult tests.

## Workstream A: Create an isolated test environment builder

Provide one reusable harness that creates:

- temporary `HOME`/XDG/config/data/cache/state roots;
- isolated snippet libraries;
- isolated sync settings;
- temporary credential storage abstraction;
- temporary SQLite database for `snip-sync`;
- dynamically assigned loopback ports;
- real `snp` and `snip-sync` executable paths;
- worker/executor log capture;
- deterministic short policy values;
- cleanup of child processes and temporary files;
- platform-safe path and quoting behavior.

Recommended API:

```rust
let env = TestEnvironment::builder()
    .with_sync_server()
    .with_auto_sync()
    .with_recording()
    .build()?;

let result = env.snp(["new", "echo test", "--description", "test"])?;
```

The harness should expose condition-based helpers:

- wait for pending generation;
- wait for recorded server operation count;
- wait for worker/executor start/exit;
- inspect status snapshot;
- assert process dead;
- release named barrier;
- inject server response/failure.

Do not use global developer config or the real user keyring.

## Workstream B: Build a controllable recording server

Prefer extending `snip-sync` test helpers so tests use the real protocol implementation while recording events.

Required controls:

- bind to port zero;
- record health/register/list/push/pull/merge operations distinctly;
- record attempt start and completion timestamps;
- block at named protocol phases;
- release from the test process;
- inject authentication failure;
- inject network close/reset;
- inject timeout/hang;
- inject conflict/partial result;
- inject malformed response where protocol permits;
- expose server-side library revision/ciphertext state without decrypting client data unnecessarily;
- count maximum concurrent sync critical sections.

If production `snip-sync` cannot safely expose controls, compile test-only hooks behind `cfg(test)` or a dedicated nondefault feature used only in integration binaries. Production release artifacts must not expose unsafe fault-injection endpoints.

## Workstream C: Add a process event channel

For lifecycle tests, provide a bounded test-only event stream from worker/executor processes. Options include:

- append-only JSON lines file in the isolated state directory;
- local IPC endpoint created by the harness;
- test-only environment variable pointing to a private event sink.

Events may include:

- worker spawned;
- execution lock acquired;
- debounce observation changed;
- executor spawned;
- canonical sync entered;
- server request started/completed;
- termination requested;
- executor reaped;
- conditional clear result;
- worker exit.

Requirements:

- unavailable in normal production configuration;
- bounded and private;
- no secrets or snippet payloads;
- explicit schema;
- test cleanup;
- not used as the sole proof of remote effect when the server can be inspected directly.

## Workstream D: Local snippet contract suite

Use real `snp` subprocesses to verify:

- exact positional command ingestion;
- `--command-stdin` byte preservation and validation;
- file/editor ingestion behavior;
- NUL, invalid UTF-8, empty, and size limits;
- TOML canonical and legacy field loading;
- atomic writes and failed-write preservation;
- variable/default/choice expansion;
- raw versus expanded selection;
- library create/delete/primary/migration;
- Pet import, duplicate policy, dry-run, and source non-mutation;
- export JSON/CSV stdout purity;
- output/favorites/folders/usage metadata contracts;
- usage increment only on successful run/clip;
- TUI cancel exit code and terminal restoration;
- shell integration output safety;
- update/help/version smoke tests without network mutation.

These tests form the stable local-first baseline and should not depend on `snip-sync`.

## Workstream E: Synchronization contract suite

Against a real ephemeral server, prove:

- registration and credential persistence;
- initial push;
- initial pull into a second isolated client;
- bidirectional merge;
- already-current no-op;
- multiple libraries;
- stable snippet/library identity;
- documented last-write-wins semantics;
- deletion propagation behavior;
- direction Push performs no pull;
- direction Pull performs no upload;
- Bidirectional performs required phases;
- CLI direction overrides configuration;
- worker and foreground paths use identical semantics;
- local-only metadata is excluded;
- encryption keeps server payload opaque;
- server restart and persistent database recovery;
- malformed/corrupt local or remote state fails safely.

Every success assertion should inspect actual server/client state, not only exit code zero.

## Workstream F: Detached worker contract suite

Required tests:

1. Mutation parent returns promptly without waiting for network completion.
2. Worker is detached from terminal/session according to platform contract.
3. Worker survives parent exit.
4. Worker observes latest generation after debounce.
5. A burst produces exactly one server sync attempt.
6. Marker removal during debounce produces zero attempts.
7. Mutation during active sync creates exactly one later follow-up attempt.
8. Worker owns execution lock while executor runs.
9. Executor does not reacquire the same lock.
10. Timeout terminates and reaps executor before unlock.
11. Pending remains after timeout/crash/nonzero exit.
12. Successful server effect precedes conditional pending clear.
13. Multiple simultaneous mutation parents do not produce concurrent sync execution.
14. Startup recovery does not increment generation.
15. Backoff prevents worker storms.

Use strict attempt counts. Do not accept `>= 1` where one is required.

## Workstream G: Mutual exclusion matrix

Create barrier-driven tests for every meaningful pair:

- worker vs worker;
- worker vs manual `snp sync`;
- worker vs cron;
- worker vs explicit `run --sync`;
- manual sync vs cron;
- manual sync vs another manual sync;
- explicit sync vs explicit sync;
- recovery worker vs newly scheduled worker.

For each pair:

1. First operation enters the canonical sync critical section.
2. Hold it at a server or test barrier.
3. Start the second operation.
4. Assert maximum concurrent execution count remains one.
5. Assert second operation follows documented wait/defer/error semantics.
6. Release first operation.
7. Verify completion order and pending/status transitions.

## Workstream H: Crash-window fault injection

Add named failpoints behind test-only configuration:

- after local snippet commit, before pending record;
- after pending record, before worker spawn;
- after worker spawn, before lock acquisition;
- after lock acquisition, before debounce;
- after debounce, before executor spawn;
- after executor spawn, before canonical sync;
- after remote success, before local persistence;
- after local persistence, before executor exit;
- after executor success, before pending clear;
- during conditional clear;
- after status write, before worker exit.

For each crash point, define expected recovery:

- local data durability;
- pending presence/generation;
- remote state;
- status state;
- lock reclaim behavior;
- next startup/retry outcome.

Do not add failpoints to normal release binaries.

## Workstream I: Timeout and process lifecycle matrix

Platform-specific assertions:

### Unix

- SIGTERM attempted;
- grace period honored;
- SIGKILL used if needed;
- direct child or process group confirmed dead according to design;
- `wait()` reaps child;
- no zombie remains;
- lock persists until death/reap.

### Windows

- creation flags produce detached worker/no console window as intended;
- executor remains supervised;
- terminate/kill behavior is confirmed;
- process handle/liveness checks are correct;
- file deletion/rename semantics do not invalidate tests;
- lock cleanup follows ownership rules.

All tests must have bounded job timeouts and emit useful sanitized evidence on failure.

## Workstream J: Package and installation tests

Test repository builds and packaged artifacts separately.

Required checks:

```bash
cargo package -p snip-it
cargo package -p snip-sync
```

Unpack and verify:

- package compiles independently;
- runtime assets required by the crate are included;
- excluded files are not assumed at runtime;
- bundled themes resolve;
- shell completions generate;
- `cargo install --path` smoke test succeeds;
- Homebrew/release archive layout assumptions remain correct where CI can verify them;
- hidden worker/executor subcommands exist in installed binary;
- current executable re-exec works after installation.

## Workstream K: CI matrix and test partitioning

Recommended jobs:

- formatting;
- clippy all targets/features;
- unit tests;
- local integration tests;
- sync real-server tests;
- detached process tests;
- package/install tests;
- security sentinel tests;
- Linux;
- macOS;
- Windows.

Partition tests so failures identify the subsystem. Use job-level timeouts and per-test bounded waits.

Avoid retries that hide deterministic failures. If infrastructure retry is necessary, preserve the first failure logs and track flakiness explicitly.

Upload only sanitized worker/server/event logs on failure. Scan artifacts for test sentinel secrets before upload.

## Workstream L: Test quality audit

Remove or rewrite tests that:

- only assert no panic;
- accept contradictory outcomes;
- infer sync from marker disappearance;
- construct but do not concurrently run processes;
- use broad elapsed-time thresholds as sole evidence;
- accept one or more attempts when exactly one is required;
- ignore platform behavior without replacement coverage;
- mock the exact function whose invocation is the contract under test;
- assert only executor exit-code conversion.

Maintain a small ignored/manual soak suite only for high-iteration stress. Required correctness tests must run in CI.

## Documentation and closure evidence

Document:

- harness architecture;
- how to run focused suites;
- platform prerequisites;
- test-only features and why they are absent from production;
- deterministic timing/failpoint conventions;
- artifact redaction policy.

Create a closure status file at completion containing:

- commit range;
- test counts by category;
- CI results by platform;
- exact invariants proven;
- any non-blocking limitations;
- confirmation that the placeholder executor regression is covered;
- confirmation that no daemon or second installed helper binary was introduced.

## Recommended commit sequence

1. Add isolated environment builder and process cleanup.
2. Add controllable real `snip-sync` recording server.
3. Add bounded test-only event/failpoint channel.
4. Establish local contract suite.
5. Establish real sync contract suite.
6. Add detached worker and mutual exclusion matrix.
7. Add crash-window and timeout lifecycle tests.
8. Add package/install tests and cross-platform CI jobs.
9. Audit/remove permissive tests.
10. Write harness docs and closure evidence.

## Exit criteria

Phase 05 is complete only when:

- a mandatory test proves remote effect before pending clear;
- replacing canonical executor sync with a no-op fails CI;
- every failure class proves pending preservation;
- exact debounce attempt counts are tested;
- all sync entry-point pairs are serialized;
- timeout termination/reap is proven on Unix and Windows;
- crash windows have defined recovery evidence;
- packaged artifacts are tested independently;
- no required test is ignored or permissive;
- failure artifacts are sanitized;
- Linux, macOS, and Windows jobs pass;
- closure evidence is committed.

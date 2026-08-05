# Phase 13 Roadmap — Correctness Repair, Verification Reduction, and Lightweight Scope Recovery

Status: CORRECTIVE CLOSURE REQUIRED

Original baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

Phase 13G reviewed baseline: `00bee90300d1984ccfc01a12f1fcd909fd6a3d60`

Date opened: 2026-08-04

Last review: 2026-08-05

## 1. Purpose

Phase 13 is a bounded correctness and simplification line for `snip-it`, a small local-first terminal snippet manager with optional encrypted self-hosted synchronization.

The governing product model remains:

- local snippet operations are the primary product;
- optional sync must not endanger successful local mutation;
- the sync server targets loopback, trusted LAN, or reverse-proxied self-hosting;
- this project does not require production-SaaS architecture or generalized orchestration;
- simplification must preserve user-visible behavior unless a separately approved change says otherwise.

Phases 13A through 13F delivered meaningful correctness, verification, footprint, auto-sync, persistence, API, CLI, and documentation work. Phase 13G corrected several residual sync and shutdown defects. A post-13G review of `00bee903` found remaining release blockers, so the prior `COMPLETE` status was premature.

Phase 13H is the final narrow corrective closure. It must fix only the reproduced remaining defects and restore truthful verification records.

## 2. Current release blockers

The following items remain open at the Phase 13H baseline:

1. empty local input, including the real pull-only path, reaches an `unreachable!` panic;
2. all-local-encryption-failure input can reach the same zero-batch panic;
3. multi-batch context wrapping replaces typed errors such as `ClockSkew` with `SyncRequestFailed`;
4. an unexpectedly completed service `JoinHandle` can be polled twice;
5. drain timeout drops handles instead of explicitly aborting and awaiting unfinished tasks;
6. deterministic orchestration tests do not exercise the production orchestration used by `serve_inner`;
7. the short process test has an unbounded wait and does not restart on the same ports;
8. partial-failure convergence is tested against fresh state rather than the partially mutated database;
9. Phase 13G and roadmap completion records claim release clearance despite these defects.

Until Phase 13H passes, do not publish a release or mark Phase 13 complete.

## 3. Phase map

| Phase | Plan | State | Goal |
|---|---|---|---|
| 13A | `plans/snip-it-phase-13a-server-lifetime-config-correctness.md` | implemented; final closure depends on 13H | Correct server lifetime, shutdown, and fail-closed configuration parsing |
| 13B | `plans/snip-it-phase-13b-sync-request-sizing-clock-diagnostics.md` | implemented; final closure depends on 13H | Bound sync requests and preserve correct diagnostics |
| 13C | `plans/snip-it-phase-13c-verification-ci-simplification.md` | implemented | Reduce routine verification and CI ceremony |
| 13D | `plans/snip-it-phase-13d-client-runtime-dependency-footprint.md` | implemented | Reduce runtime/dependency footprint without feature loss |
| 13E | `plans/snip-it-phase-13e-auto-sync-persistence-simplification.md` | implemented | Simplify auto-sync and transaction scope |
| 13F | `plans/snip-it-phase-13f-api-cli-server-surface-consolidation.md` | implemented | Narrow supported surfaces and clean documentation |
| 13G | `plans/snip-it-phase-13g-corrective-closure.md` | implemented with remaining corrective gaps | Correct initial multi-batch and shutdown defects |
| 13H | `plans/snip-it-phase-13h-final-correctness-closure.md` | READY FOR IMPLEMENTATION; release-blocking | Correct zero-batch sync, typed error preservation, real task abort/drain, retained-state recovery, and closure records |

Required sequence:

```text
13A + 13B
    -> 13C
    -> 13D
    -> 13E
    -> 13F
    -> 13G
    -> 13H final corrective closure
    -> verified closure
```

## 4. Implemented work to retain

Do not revert the correctly landed Phase 13 changes while implementing 13H.

Retain:

- strict typed server environment parsing;
- rejection of unusable zero-valued server limits;
- removal of the arbitrary normal-operation server lifetime timeout;
- Unix SIGTERM registration;
- Axum and Tonic shutdown-aware serving APIs;
- deterministic byte-bounded upload preflight;
- immediate singleton overflow revalidation;
- multi-batch upload-before-authoritative-response ordering;
- clock-skew diagnostics;
- reduced routine CI and release ceremony;
- gzip theme compression and removal of duplicate archive/decompression dependencies;
- lazy Tokio initialization for local commands;
- current-thread auto-sync helper runtime;
- simplified failure classes, scheduling, module layout, and transaction path;
- `snp data` command grouping and legacy aliases;
- public API and architecture documentation cleanup.

Phase 13H must correct the remaining behavior without reopening these workstreams.

## 5. Global constraints

### 5.1 Required outcomes

Phase 13 must ultimately leave the repository with:

- zero-, one-, and multi-batch sync as explicit valid cases;
- pull-only sync that retrieves remote state into an empty local library;
- no panic when all local snippets fail encryption;
- typed sync error classification preserved while batch context is added;
- complete preflight before remote mutation;
- retained-state convergence after later-batch failure;
- one production shutdown orchestration implementation used by both `serve_inner` and tests;
- completed service handles consumed exactly once;
- unfinished services explicitly aborted and awaited after timeout;
- persistence shutdown only after serving tasks terminate or abort;
- bounded child-process waits and same-port replacement verification;
- focused routine verification and explicit manual release verification;
- truthful Phase 13 plan statuses, SHAs, commands, results, and residual deviations.

### 5.2 Explicit non-goals

Do not add:

- new sync RPCs, streaming RPCs, protocol revisions, database migrations, CRDTs, vector clocks, or distributed transactions;
- upload journals, queues, daemons, supervisors, service managers, or durable batch checkpoints;
- generalized batching, transport, cancellation, or task-supervision frameworks;
- new async-runtime, signal, testing, mocking, or orchestration dependencies;
- new CI jobs, matrices, schedules, coverage systems, benchmarks, or evidence artifacts;
- broad auto-sync, transaction, TUI, theme, updater, CLI, API, or packaging work;
- arbitrary validation bounds unrelated to reproduced defects;
- indefinite retries or unbounded process waits.

### 5.3 Security and durability boundary

All corrective work must retain:

- client-side encryption and authenticated ownership;
- API-key secrecy and keychain behavior;
- server request/message limits;
- deterministic snippet identity and existing upsert semantics;
- local-first mutation ordering;
- pending auto-sync intent after failures;
- atomic local writes and current destructive-operation protections;
- safe updater extraction and checksum behavior;
- existing path and symlink protections.

## 6. Verification philosophy

Phase 13 intentionally uses a small verification surface.

Routine verification remains centered on:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke
cargo test --test sync_multibatch -- --test-threads=1
bash scripts/check.sh
```

The long server lifetime test remains manual/release-only because it intentionally waits beyond 30 seconds.

Manual release verification must include:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
bash scripts/release-check.sh verify
```

No plan may claim closure when a required command was skipped, excluded, or run against a different implementation/profile.

## 7. Phase 13H closure gate

Phase 13H closes only when every acceptance criterion in `plans/snip-it-phase-13h-final-correctness-closure.md` is satisfied.

At minimum:

- empty and pull-only sync pass without panic;
- all-encryption-failed input passes without panic and retains skipped accounting;
- zero-batch pagination retrieves complete remote state;
- typed errors retain their original `SyncFailureKind` and `FailureClass`;
- retained-state partial failure converges against the same account/library/database;
- `serve_inner` calls the same orchestration helper used by tests;
- completed handles are never polled twice;
- refusing tasks are explicitly aborted and awaited;
- persistence starts shutdown only after service termination is proven;
- all child waits are bounded;
- replacement server uses the same state and same ports;
- short Unix SIGTERM regression passes five consecutive runs;
- long lifetime regression passes;
- `bash scripts/release-check.sh verify` passes from a clean tree;
- Phase 13G and roadmap records are corrected.

## 8. Commit and handoff strategy

Use at most:

1. one coherent implementation/test commit for sync corrections;
2. one coherent implementation/test commit for shutdown/process corrections;
3. one closure/documentation commit.

Combining the two implementation workstreams into one commit is acceptable if the diff remains easy to review.

Do not create one commit per test or helper.

The closure commit must record:

- all implementation SHAs;
- exact verification commands and results;
- short Unix SIGTERM repeated-run count;
- any bounded residual deviation;
- explicit release disposition.

## 9. Historical implementation record

Implemented before Phase 13H:

- `7e0d064` Phase 13A server lifetime/config parsing
- `84f5b7f` Phase 13B upload batching/clock diagnostics
- `0575f38` + `33b27da` Phase 13C verification simplification
- `181a142` Phase 13D footprint reduction
- `aa62bb4` + `a0df1ab` Phase 13E auto-sync/persistence simplification
- `01a860b` + `429952e` Phase 13F API/CLI/docs consolidation
- `5d37fa7` Phase 13G initial sync/shutdown/config correction
- `898a62b` Phase 13G tests/docs/records follow-up
- `00bee90` Phase 13G server-drain and SIGTERM-test follow-up

These commits do not constitute final release clearance until Phase 13H closes.

## 10. Final closure criteria

Phase 13 is complete only when all statements are true:

- [ ] Phase 13H is marked `COMPLETE` or `COMPLETE WITH DOCUMENTED DEVIATIONS` with implementation SHAs.
- [ ] Empty local input performs pull-only sync without panic.
- [ ] All-local-encryption-failure input does not panic.
- [ ] Zero-, one-, and multi-batch sync are covered by direct regressions.
- [ ] Batch context preserves typed failure classification.
- [ ] Partial upload retry converges against retained server/database state.
- [ ] Production and test orchestration use the same helper.
- [ ] Completed `JoinHandle`s are never polled twice.
- [ ] Drain timeout explicitly aborts and awaits unfinished tasks.
- [ ] Persistence shutdown follows verified serving-task termination.
- [ ] Process tests use bounded waits and same-port restart.
- [ ] The server remains healthy beyond 30 seconds and exits normally through SIGTERM.
- [ ] Routine verification remains compact.
- [ ] Manual release verification passes from a clean tree.
- [ ] No new protocol, schema, daemon, supervisor, generalized framework, dependency, or CI topology is added.
- [ ] Phase 13G and earlier affected phase records contain truthful corrective SHAs and results.
- [ ] The roadmap completion record includes Phase 13H.
- [ ] No release blocker remains deferred.

Only after these statements are true may this roadmap return to `Status: COMPLETE` and `Release disposition: CLEARED`.

## 11. Current disposition

Status: CORRECTIVE CLOSURE REQUIRED

Release-blocking phase: Phase 13H

Current release disposition: BLOCKED

Next plan: `plans/snip-it-phase-13h-final-correctness-closure.md`

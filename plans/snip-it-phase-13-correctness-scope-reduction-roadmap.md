# Phase 13 Roadmap — Correctness Repair, Verification Reduction, and Lightweight Scope Recovery

Status: CORRECTIVE CLOSURE REQUIRED

Original baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

Phase 13G reviewed baseline: `00bee90300d1984ccfc01a12f1fcd909fd6a3d60`

Phase 13I corrective baseline: `f8b9aa8445a8d9a4385e505df94a275df2dde4a9`

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

Phases 13A through 13F delivered correctness, verification, footprint, auto-sync, persistence, API, CLI, and documentation improvements. Phase 13G corrected the first residual sync and shutdown defects. Phase 13H then added explicit zero-batch sync, shared sync logic, typed batch-error preservation, shared production shutdown orchestration, bounded process waits, same-port restart testing, and retained SQLite state.

A post-13H review of `f8b9aa8` found that final closure remains premature. Phase 13I is the sole remaining corrective phase. It must correct two-handle drain result accounting, replace timing-raced partial-failure coverage with deterministic retained-state proof, add the missing direct zero-batch/error-context regressions, remove duplicated orchestration tests, and restore truthful closure records.

## 2. Current release blockers

The following items remain open at the Phase 13I baseline:

1. during requested drain, one service handle can complete and be consumed before the sibling times out, while outer state still treats both handles as pending;
2. the forced-abort path can therefore abort or await an already-consumed handle;
3. service errors or panics returned during drain are discarded, allowing requested shutdown to report success after unclean service completion;
4. the retained-state partial-failure test uses a 200 ms race and does not prove partial mutation or first-attempt failure;
5. empty/pull-only pagination, all-encryption-failed accounting, and typed batch-context behavior lack the direct regressions required by Phase 13H;
6. older standalone orchestration tests remain alongside production-helper tests and duplicate behavior outside the production path;
7. Phase 13H and this roadmap claim complete closure despite these gaps.

Until Phase 13I passes, do not publish a release or mark Phase 13 complete.

## 3. Phase map

| Phase | Plan | State | Goal |
|---|---|---|---|
| 13A | `plans/snip-it-phase-13a-server-lifetime-config-correctness.md` | implemented | Correct server lifetime, shutdown, and fail-closed configuration parsing |
| 13B | `plans/snip-it-phase-13b-sync-request-sizing-clock-diagnostics.md` | implemented | Bound sync requests and preserve correct diagnostics |
| 13C | `plans/snip-it-phase-13c-verification-ci-simplification.md` | implemented | Reduce routine verification and CI ceremony |
| 13D | `plans/snip-it-phase-13d-client-runtime-dependency-footprint.md` | implemented | Reduce runtime/dependency footprint without feature loss |
| 13E | `plans/snip-it-phase-13e-auto-sync-persistence-simplification.md` | implemented | Simplify auto-sync and transaction scope |
| 13F | `plans/snip-it-phase-13f-api-cli-server-surface-consolidation.md` | implemented | Narrow supported surfaces and clean documentation |
| 13G | `plans/snip-it-phase-13g-corrective-closure.md` | complete with corrective follow-up | Correct initial multi-batch and shutdown defects |
| 13H | `plans/snip-it-phase-13h-final-correctness-closure.md` | complete with corrective follow-up | Add zero-batch sync, shared orchestration, retained-state/process corrections, and record repair |
| 13I | `plans/snip-it-phase-13i-drain-and-regression-closure.md` | complete | Correct drain result accounting, deterministic retained-state failure, missing regressions, and final records |

Required sequence:

```text
13A + 13B
    -> 13C
    -> 13D
    -> 13E
    -> 13F
    -> 13G
    -> 13H
    -> 13I drain and regression closure
    -> verified closure
```

## 4. Implemented work to retain

Do not revert the correctly landed Phase 13 changes while implementing 13I.

Retain:

- strict typed server environment parsing and rejection of unusable zero-valued limits;
- removal of the arbitrary normal-operation server lifetime timeout;
- Unix SIGTERM registration and shutdown-aware Axum/Tonic serving;
- deterministic byte-bounded upload preflight and singleton overflow revalidation;
- upload-before-authoritative-response ordering for multi-batch sync;
- explicit zero-, one-, and many-batch dispatch;
- one shared `sync_encrypted_inner` implementation;
- typed `SyncFailureKind` preservation for multi-batch errors;
- shared `run_services_until_shutdown` production entry point;
- explicit task abort calls after drain timeout;
- bounded process waits, isolated state directory, and same-port restart helper;
- reduced routine CI and manual release structure;
- dependency, runtime, auto-sync, transaction, API, CLI, and documentation simplifications from earlier Phase 13 work.

Phase 13I must correct the remaining behavior without reopening these workstreams.

## 5. Required Phase 13I outcome

Phase 13I must leave the repository with:

- every service handle output consumed exactly once;
- completion state updated immediately when a task finishes during drain;
- only still-pending tasks aborted after timeout;
- every initial and drain-time service result classified and propagated;
- requested shutdown succeeding only when both services finish cleanly without forced abort;
- persistence shutdown beginning only after both serving tasks are proven terminal;
- deterministic failure of a known later `PushSnippets` call after an earlier batch commits;
- proof that the first sync fails with a partial retained database state;
- retry against the same account, library identity, API key, and SQLite state;
- complete exact-once convergence after retry;
- direct empty/pull-only pagination and all-encryption-failed accounting regressions;
- direct typed batch-context tests for `ClockSkew` and `Timeout`;
- only production-helper orchestration tests, with parallel fake logic removed;
- compact routine verification and successful manual release verification;
- truthful Phase 13H, Phase 13I, and roadmap records.

## 6. Explicit non-goals

Do not add:

- new sync RPCs, streaming RPCs, protocol revisions, protobuf fields, or database migrations;
- upload journals, rollback RPCs, distributed transactions, queues, daemons, supervisors, task registries, or service managers;
- generalized batching, transport, cancellation, or orchestration frameworks;
- production failure-injection configuration or environment variables;
- new async-runtime, signal, test, mocking, or orchestration dependencies;
- new CI jobs, matrices, schedules, coverage systems, benchmarks, artifacts, or release automation;
- broad auto-sync, transaction, TUI, theme, updater, CLI, API, packaging, or deployment work;
- arbitrary validation bounds unrelated to the reproduced defects;
- timing sleeps as the primary trigger for deterministic partial-failure tests.

## 7. Security and durability boundary

Phase 13I must retain:

- client-side encryption and authenticated ownership;
- API-key secrecy and keychain behavior;
- request/message limits and complete upload preflight;
- deterministic snippet identity and existing idempotent upserts;
- local-first mutation ordering;
- pending auto-sync intent and successful-cursor protection after failures;
- atomic local writes and current destructive-operation protections;
- safe updater extraction/checksum behavior;
- existing path and symlink protections;
- persistence/database lifetime through final server cleanup.

## 8. Verification philosophy

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

Phase 13I may add focused tests within existing targets, but must not add a new workflow or broad verification layer.

Manual release verification must include:

```text
cargo test --release --test sync_multibatch -- --test-threads=1
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
bash scripts/release-check.sh verify
```

The 35-second lifetime regression remains release-only. The short Unix SIGTERM regression must pass five consecutive runs before closure.

No plan may claim closure when a required command was skipped, excluded, timing-raced, or run against a different implementation/profile.

## 9. Phase 13I closure gate

Phase 13I closes only when every acceptance criterion in `plans/snip-it-phase-13i-drain-and-regression-closure.md` is satisfied.

At minimum:

- the signal-first, one-clean/one-refusing regression proves no completed handle is consumed twice;
- drain-time service errors and panics produce process failure after sibling cleanup;
- every forced abort targets only pending handles and every aborted handle is awaited;
- the orchestration helper returns only after both service tasks are terminal;
- persistence starts shutdown only after that outcome is established;
- a known later push fails deterministically after an earlier batch commits;
- the first sync returns `Err` and the retained database row count is between zero and the expected total;
- retry uses the same credentials/identity/database and converges exactly once;
- empty local pull, multi-page zero-batch pagination, and all-encryption-failed accounting regressions pass;
- typed batch context preserves `ClockSkew` and `Timeout` classifications and original detail;
- obsolete parallel orchestration tests are deleted;
- short SIGTERM passes 5/5 and long lifetime verification passes;
- `bash scripts/release-check.sh verify` passes from a clean tree;
- Phase 13H and roadmap records are corrected.

## 10. Commit and handoff strategy

Use at most:

1. one coherent implementation/test commit for orchestration and deterministic sync regressions;
2. one closure/documentation commit.

A separate implementation commit for orchestration and sync tests is acceptable only when it materially improves reviewability. Do not create one commit per test.

The closure commit must record:

- all implementation SHAs;
- exact verification commands and results;
- short Unix SIGTERM repeated-run count;
- any bounded residual deviation;
- explicit release disposition.

## 11. Historical implementation record

Implemented before Phase 13I:

- `7e0d064` Phase 13A server lifetime/config parsing
- `84f5b7f` Phase 13B upload batching/clock diagnostics
- `0575f38` + `33b27da` Phase 13C verification simplification
- `181a142` Phase 13D footprint reduction
- `aa62bb4` + `a0df1ab` Phase 13E auto-sync/persistence simplification
- `01a860b` + `429952e` Phase 13F API/CLI/docs consolidation
- `5d37fa7` + `898a62b` + `00bee90` Phase 13G corrections
- `75a55b1` Phase 13H implementation
- `7619b69` release-script command correction
- `f8b9aa8` Phase 13G/13H record correction

These commits do not constitute final release clearance until Phase 13I closes.

## 12. Final closure criteria

Phase 13 is complete only when all statements are true:

- [ ] Phase 13I is marked `COMPLETE` or `COMPLETE WITH DOCUMENTED DEVIATIONS` with implementation and closure SHAs.
- [ ] Every serving-task result is consumed once and classified.
- [ ] Drain completion state is updated immediately for each service.
- [ ] Only pending tasks are aborted after timeout, and each abort is awaited.
- [ ] Requested shutdown fails on service error, panic, or forced abort.
- [ ] Persistence shutdown follows proven serving-task termination.
- [ ] Deterministic later-batch failure proves retained partial state.
- [ ] Retry converges against the same identity and database with no duplicates.
- [ ] Failed sync does not advance successful-sync cursor state.
- [ ] Empty/pull-only and multi-page zero-batch behavior have direct regressions.
- [ ] All-encryption-failed accounting has a direct regression.
- [ ] Batch context preserves typed failure classifications and original diagnostics.
- [ ] Production and tests use one orchestration helper without parallel duplicate logic.
- [ ] Process tests remain bounded and same-port restart remains valid.
- [ ] Short SIGTERM passes 5/5 and long lifetime release regression passes.
- [ ] Routine verification remains compact.
- [ ] Manual release verification passes from a clean tree.
- [ ] No new dependency, protocol, schema, daemon, supervisor, generalized framework, or CI topology is added.
- [ ] Phase 13H and affected records truthfully describe corrective follow-up and final SHAs/results.
- [ ] The roadmap completion record includes Phase 13I.
- [ ] No release blocker remains deferred.

Only after these statements are true may this roadmap return to `Status: COMPLETE` and `Release disposition: CLEARED`.

## 13. Current disposition

Status: COMPLETE

Release disposition: CLEARED

# Phase 13 Roadmap — Correctness Repair, Verification Reduction, and Lightweight Scope Recovery

Status: CORRECTIVE CLOSURE REQUIRED

Original baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

Phase 13G reviewed baseline: `00bee90300d1984ccfc01a12f1fcd909fd6a3d60`

Phase 13I reviewed head: `39f8ef5ae9a0d32330d394738c3d862dc5c7560f`

Phase 13J plan commit: `4f789cd4cd69d3c5ca8a63e9394180a9e65010b8`

Phase 13J implementation commit: `6092d5b` phase-13j: wire shutdown outcomes and consolidate sync test seams

Phase 13J record commit: <filled after Pass 7 commit exists>

Date opened: 2026-08-04

Last review: 2026-08-06

## 1. Purpose

Phase 13 is a bounded correctness and simplification line for `snip-it`, a small local-first terminal snippet manager with optional encrypted self-hosted synchronization.

The governing product model remains:

- local snippet operations are the primary product;
- optional sync must not endanger successful local mutation;
- the sync server targets loopback, trusted LAN, or reverse-proxied self-hosting;
- this project does not require production-SaaS architecture or generalized orchestration;
- simplification must preserve user-visible behavior;
- verification should prove reproduced defects without becoming a second product.

Phases 13A through 13F delivered correctness, footprint, auto-sync, persistence, API, CLI, and documentation improvements. Phases 13G through 13I corrected sync batching, zero-batch pull, retained-state convergence, server drain behavior, bounded process waits, and direct regressions.

A review of `39f8ef5` found that final closure remains premature. Phase 13J is the sole remaining corrective phase. It must wire the already-classified shutdown result into the production process result, remove a duplicated public test-only sync implementation, return local helpers to private visibility, strengthen narrow orchestration proof, and restore truthful records.

## 2. Current release blockers

The following blockers remain at the Phase 13J baseline:

1. `serve_inner` checks only `requested` and `forced`, so requested shutdown can return success when gRPC or HTTP returned an error or panicked during drain;
2. `sync_encrypted_with_custom_encrypt` duplicates the complete zero/one/many sync algorithm and is publicly callable despite being test-only;
3. `add_batch_context` is public solely for integration-test access;
4. requested-shutdown tests can wake services through a test-side broadcast rather than proving the orchestration helper sends shutdown;
5. the no-pre-signal-timeout test waits before invoking the helper and does not test the claimed interval;
6. Phase 13H, Phase 13I, and roadmap records contain contradictory statuses, incomplete commit attribution, and overstated evidence.

Until Phase 13J passes, do not publish a release or mark Phase 13 complete.

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
| 13I | `plans/snip-it-phase-13i-drain-and-regression-closure.md` | implemented with corrective follow-up required | Add drain result accounting, deterministic retained-state failure, and missing regressions |
| 13J | `plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md` | IMPLEMENTED; VERIFICATION PENDING; release-blocking | Wire production outcome, consolidate test seams, tighten proof, and close records |

Required sequence:

```text
13A + 13B
    -> 13C
    -> 13D
    -> 13E
    -> 13F
    -> 13G
    -> 13H
    -> 13I
    -> 13J production-outcome and test-seam closure
    -> verified closure
```

## 4. Correctly implemented work to retain

Do not revert:

- strict typed server environment parsing and nonzero limit validation;
- removal of the arbitrary normal-operation server lifetime timeout;
- Unix SIGTERM registration and shutdown-aware Axum/Tonic serving;
- deterministic byte-bounded upload preflight and singleton overflow validation;
- explicit zero-, one-, and many-batch sync behavior;
- upload-before-authoritative-response ordering;
- typed sync failure preservation with batch context;
- deterministic retained-state failure and exact-once retry convergence;
- per-service completion tracking during drain;
- explicit abort and await of pending tasks after timeout;
- bounded process waits, state isolation, and same-port restart;
- reduced routine CI and manual release structure;
- earlier dependency, runtime, transaction, API, CLI, and documentation simplifications.

Phase 13J must correct only the final wiring, test-seam, proof, and record gaps.

## 5. Required Phase 13J outcome

Phase 13J must leave the repository with:

- production success based on the fully classified `ServiceShutdownOutcome`;
- requested shutdown returning success only for two clean service results with no forced abort;
- persistence cleanup completed before any service failure is returned;
- final diagnostics retaining both service classifications and original details;
- exactly one zero/one/many encrypted sync transport implementation;
- private, unit-test-only encryption failure injection;
- no public custom-encryption sync method;
- private `add_batch_context` with colocated unit tests;
- requested-shutdown tests proving the helper owns the shutdown broadcast;
- a real no-pre-signal-timeout regression;
- truthful direct-versus-indirect evidence records;
- one consistent roadmap status and release disposition;
- successful focused, routine, process, and clean-tree release verification.

## 6. Explicit non-goals

Do not add:

- new RPCs, streaming RPCs, protocol revisions, protobuf fields, or database migrations;
- upload journals, rollback RPCs, queues, durable checkpoints, or distributed transactions;
- generalized supervisors, task registries, service managers, or daemon frameworks;
- production failure-injection configuration or environment variables;
- new async-runtime, signal, mock, test, or orchestration dependencies;
- new CI jobs, matrices, schedules, coverage systems, benchmarks, artifacts, or release automation;
- broad auto-sync, transaction, TUI, theme, updater, CLI, API, packaging, or deployment work;
- a new high-level pull/filesystem/server harness solely to strengthen record wording;
- source-text tests as correctness evidence.

## 7. Security and durability boundary

Phase 13J must retain:

- client-side encryption and authenticated ownership;
- API-key secrecy and keychain behavior;
- request/message limits and complete upload preflight;
- deterministic snippet identity and idempotent upserts;
- local-first mutation ordering;
- pending auto-sync intent and caller-controlled successful-sync timestamp updates;
- atomic local writes and destructive-operation protections;
- updater extraction/checksum and path protections;
- database/persistence lifetime through final server cleanup.

## 8. Execution authority

The implementation authority is:

`plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md`

It is written for sequential small-model execution and contains:

- exact file boundaries for each pass;
- preferred code shapes;
- tests to move or retain;
- focused commands after each pass;
- explicit stop conditions;
- commit and clean-tree verification ordering;
- final acceptance criteria and completion record template.

Do not substitute the older Phase 13I checklist for Phase 13J execution.

## 9. Verification philosophy

Routine verification remains compact:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke
cargo test --test sync_multibatch -- --test-threads=1
bash scripts/check.sh
```

Focused Phase 13J verification must include:

```text
cargo test -p snip-it --lib sync -- --test-threads=1
cargo test -p snip-sync --lib orchestration -- --test-threads=1
```

Manual process/release verification must include:

```text
cargo test --release --test snip_sync_lifetime -- --ignored --test-threads=1
bash scripts/release-check.sh verify
```

The short Unix SIGTERM case must pass five consecutive runs. The 35-second lifetime case remains release-only.

No plan may claim closure for skipped, excluded, timing-raced, dirty-tree, or differently profiled commands.

## 10. Commit strategy

Use at most two commits after the planning commits:

1. implementation, focused tests, and truthful verification-pending records;
2. final records after clean-tree release verification.

Preferred messages:

```text
phase-13j: wire shutdown outcomes and consolidate sync test seams
phase-13: record verified phase 13j closure
```

Do not create one commit per helper or test.

The final record must distinguish:

- the implementation commit against which clean-tree release verification ran;
- the later record-only commit that records that result.

## 11. Historical implementation record

Implemented before Phase 13J:

- `7e0d064` Phase 13A server lifetime/config parsing
- `84f5b7f` Phase 13B upload batching/clock diagnostics
- `0575f38` + `33b27da` Phase 13C verification simplification
- `181a142` Phase 13D footprint reduction
- `aa62bb4` + `a0df1ab` Phase 13E auto-sync/persistence simplification
- `01a860b` + `429952e` Phase 13F API/CLI/docs consolidation
- `5d37fa7` + `898a62b` + `00bee90` Phase 13G corrections
- `75a55b1` Phase 13H implementation
- `7619b69` release-script correction
- `f8b9aa8` Phase 13G/13H record correction
- `c08cac1` Phase 13I primary implementation
- `5f10c68` Phase 13I drain-time error test and initial completion record
- `18e7ddb` Phase 13I release-check record update
- `39f8ef5` Phase 13I closure-SHA record update
- `4f789cd` Phase 13J implementation plan

These commits do not constitute final Phase 13 release clearance until Phase 13J closes.

## 12. Final closure criteria

Phase 13 is complete only when all statements are true:

- [ ] Phase 13J is marked `COMPLETE` or `COMPLETE WITH DOCUMENTED DEVIATIONS` with implementation and record SHAs.
- [ ] Production consumes the same clean-shutdown decision tested by unit tests.
- [ ] Requested shutdown fails after cleanup on service error, panic, or forced abort.
- [ ] Unexpected clean service exit remains failure.
- [ ] Service failure diagnostics retain both service results and original detail.
- [ ] Requested-shutdown tests rely on the helper's shutdown broadcast.
- [ ] No-pre-signal-timeout is directly tested while orchestration is running.
- [ ] Exactly one method contains zero/one/many sync transport logic.
- [ ] Custom encryption failure injection is private and unit-test-only.
- [ ] `sync_encrypted_with_custom_encrypt` is removed.
- [ ] `add_batch_context` is private with colocated unit tests.
- [ ] Real zero-batch, pagination, retained-state, and exact-once integration tests remain passing.
- [ ] No dependency, feature, protocol, schema, supervisor, journal, or CI topology is added.
- [ ] Direct and indirect evidence are labeled truthfully.
- [ ] Phase 13H and Phase 13I records contain correct statuses and commit attribution.
- [ ] The roadmap has no contradictory status or disposition.
- [ ] Focused and routine checks pass.
- [ ] Long lifetime verification passes.
- [ ] Short Unix SIGTERM passes 5/5.
- [ ] Clean-tree `bash scripts/release-check.sh verify` passes against the Phase 13J implementation commit.
- [ ] No release blocker remains deferred.

Only after these statements are true may this roadmap return to `Status: COMPLETE` and `Release disposition: CLEARED`.

## 13. Current disposition

Status: CORRECTIVE CLOSURE REQUIRED

Release-blocking phase: Phase 13J (implementation committed; final record pending)

Current release disposition: BLOCKED

Next plan: `plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md`

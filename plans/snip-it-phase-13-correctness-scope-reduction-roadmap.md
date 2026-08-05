# Phase 13 Roadmap — Correctness Repair, Verification Reduction, and Lightweight Scope Recovery

Status: COMPLETE

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

Corrective review baseline: `429952eb26653b76e7dd135af2b4a5881095476b`

Date: 2026-08-04

## 1. Purpose

This roadmap is the bounded follow-up to the post-Phase-12 repository review.

`snip-it` already accomplishes its intended product goal: a fast terminal snippet manager with editable TOML storage, fuzzy selection, variable expansion, a keyboard-first TUI, and optional encrypted synchronization. Phase 13 does not add product capabilities. It corrects two material defects, reduces verification and dependency overhead, and removes architecture whose maintenance cost is disproportionate to a small local-first utility and optional LAN/self-hosted sync server.

The governing product model is:

- local snippet operations are the primary product;
- synchronization is optional and must never endanger a successful local mutation;
- the sync server is intended primarily for loopback, trusted-LAN, or reverse-proxied self-hosted use;
- the repository does not require production-platform hardening, high-assurance evidence ceremony, or a generalized distributed job system;
- simplification must retain user-visible features unless a later, explicitly approved breaking change says otherwise.

Phase 13 must not become another broad hardening program. Correctness defects are fixed first. Simplification work then removes code, tests, dependencies, and public commitments where evidence shows they are unnecessary.

A post-implementation review at `429952eb26653b76e7dd135af2b4a5881095476b` found that the Phase 13A shutdown implementation and Phase 13B multi-batch upload implementation do not yet satisfy their closure contracts. Phase 13G is therefore release-blocking and must complete before this roadmap can close.

## 2. Findings that drive this roadmap

### 2.1 Release-blocking correctness defects

1. `snip-sync serve` originally wrapped the entire lifetime of the HTTP and gRPC tasks in a 30-second timeout. The arbitrary lifetime cap was removed, but the replacement shutdown path does not correctly broadcast requested shutdown, retain and await both service tasks, or coordinate Unix SIGTERM.
2. The local client accepts command bodies up to 16 MiB, while the sync server defaults to a 4 MiB gRPC message limit. Phase 13B added upload batching, but the implementation may return after the first batch when the first response has no additional page, and a later oversized singleton can evade complete preflight validation.

### 2.2 Bounded correctness and operational issues

- malformed numeric and boolean server environment values previously fell back silently; strict parsing is now present, but a small set of unusable zero-valued limits still requires closure validation;
- server shutdown must use one coordinated Ctrl-C/SIGTERM source and real task draining;
- auto-sync scheduling previously probed an execution lock, released it, then spawned a worker that reacquired it; Phase 13E removed that ineffective reservation probe;
- client/server clock skew now has typed diagnostics;
- line-number-heavy architecture documentation has been substantially cleaned up;
- Phase 13 plan statuses and completion records do not yet consistently reflect the implementation and residual defects.

### 2.3 Excess complexity relative to scope

- normal CI and local checks previously compiled overlapping targets repeatedly and serialized far more tests than necessary;
- the release verification path repeated broad debug and release suites, crash matrices, package checks, and production-seam proofs regardless of the changed area;
- the client linked multiple compression/archive implementations for bundled themes and platform-specific update archives;
- local-only command paths carried runtime/network-oriented structure that should be lazy and isolated;
- auto-sync remained a compact but substantial scheduler with durable generations, several lock/state modules, backoff classifications, configuration fingerprints, and recovery paths;
- the transaction journal was database-grade and should be reserved for truly multi-file destructive operations rather than defining routine mutation architecture;
- the Rust public API, top-level CLI, and server administration surface exposed more implementation detail and maintenance obligation than the core product needs.

Phases 13C through 13F made meaningful progress on these items. Phase 13G must not reopen them broadly; it performs only the bounded closure audit and documentation reconciliation specified in its plan.

## 3. Phase map

| Phase | Plan | Goal | Dependency |
|---|---|---|---|
| 13A | `plans/snip-it-phase-13a-server-lifetime-config-correctness.md` | Correct server lifetime, coordinated shutdown, and fail-closed configuration parsing | none |
| 13B | `plans/snip-it-phase-13b-sync-request-sizing-clock-diagnostics.md` | Make every locally valid snippet/library synchronizable within bounded requests and improve skew diagnostics | 13A may proceed in parallel |
| 13C | `plans/snip-it-phase-13c-verification-ci-simplification.md` | Reduce routine CI/test/release ceremony while retaining high-value coverage | 13A and 13B test targets identified |
| 13D | `plans/snip-it-phase-13d-client-runtime-dependency-footprint.md` | Apply measured, low-risk runtime and dependency reductions without feature loss | 13A/13B stable baseline |
| 13E | `plans/snip-it-phase-13e-auto-sync-persistence-simplification.md` | Simplify auto-sync and local transaction machinery without weakening local durability | 13A/13B complete; 13C defines reduced verification |
| 13F | `plans/snip-it-phase-13f-api-cli-server-surface-consolidation.md` | Narrow maintenance commitments while preserving compatible user workflows | 13E complete or stable |
| 13G | `plans/snip-it-phase-13g-corrective-closure.md` | Correct residual multi-batch and shutdown defects, add real regressions, and reconcile Phase 13 records | post-13F review; release-blocking |

Required sequence:

```text
13A + 13B
    -> 13C
    -> 13D
    -> 13E
    -> 13F
    -> 13G corrective closure
    -> closure
```

13A and 13B may be implemented independently. Phase 13C must not delete tests needed to validate unresolved defects. Phase 13D is measurement-gated. Phase 13E must simplify existing mechanisms rather than replace them with new frameworks. Phase 13F must preserve compatibility aliases and current install/deployment workflows during this phase. Phase 13G is narrow and release-blocking: it may correct only the verified residual defects and bounded record/API-documentation inconsistencies named in its plan.

## 4. Global constraints

### 4.1 Required outcomes

Phase 13 must leave the repository with:

- a sync server that remains healthy indefinitely until explicitly stopped or a service fails;
- one coordinated graceful-shutdown path covering Ctrl-C and Unix SIGTERM, with a timeout applied only after shutdown begins;
- both HTTP and gRPC tasks retained, signaled, and awaited or explicitly aborted after a bounded drain;
- explicit errors for malformed server environment configuration;
- bounded upload batching or an equivalent protocol-compatible mechanism so locally valid data does not exceed one gRPC request;
- complete preflight for every upload batch before the first remote mutation;
- no upload-loop success path that can omit later batches;
- a clear error for a single item that cannot fit the configured message limit;
- direct clock-skew diagnostics rather than a generic invalid-argument failure;
- a real encrypted multi-batch integration regression and a sustained server-lifetime/graceful-signal regression;
- a smaller routine CI path with no redundant standalone build after clippy/tests compile the workspace;
- single-threaded execution only for tests that actually use PTYs, process-global environment, keychain state, ports, or cross-process locks;
- deep crash/failpoint and release evidence checks moved to manual or change-scoped verification;
- measured binary attribution before dependency removal;
- no duplicate LZMA/DEFLATE or ZIP/tar archive stack when one existing implementation can serve the same features;
- local commands that do not initialize or depend on async/network services unless required;
- a simpler auto-sync state model with one execution authority and no ineffective parent lock reservation probe;
- durable transaction journaling limited to operations that genuinely mutate multiple files or require rollback;
- a narrower supported Rust API and clearer CLI/server grouping without immediate feature removal;
- architecture documentation based on symbols and invariants, not volatile line numbers;
- truthful Phase 13 statuses, implementation SHAs, verification records, and residual deviations.

### 4.2 Explicit non-goals

Do not add:

- new user-facing snippet, TUI, sync, metrics, or deployment features;
- a new daemon, service manager, queue, scheduler, IPC channel, or worker supervisor;
- CRDTs, vector clocks, consensus, event sourcing, distributed transactions, upload journals, or a new sync protocol;
- a new database or persistence layer;
- Kubernetes, cloud deployment, multi-tenant SaaS, or public-internet hardening;
- a generalized configuration framework;
- a new benchmarking harness, test framework, mock framework, fuzzing program, or model checker;
- coverage thresholds, binary-size gates, latency gates, release-evidence artifacts, or new CI jobs;
- automated crates.io publication or automatic release cadence;
- broad TUI rewrites, data-format redesigns, or command removals;
- speculative dependency rewrites without measured binary or maintenance value;
- new runtime dependencies solely to simplify existing code;
- protobuf or database-schema changes for the Phase 13G corrections.

### 4.3 Security boundary

Simplification and corrective work must retain:

- client-side encryption and authenticated ownership for sync;
- API key secrecy and keychain behavior;
- server request/message limits;
- safe archive extraction and checksum verification for updates;
- path containment and symlink protections for restore/update operations;
- atomic writes for user data;
- pre-operation backups for destructive bulk operations;
- a kernel-backed cross-process exclusion primitive where concurrent mutation would corrupt data;
- the rule that local mutations commit before optional remote synchronization;
- pending sync intent after any partial upload or remote failure.

The project can remove redundant layers around these properties, but must not remove the properties themselves.

## 5. Verification philosophy

Phase 13 intentionally reduces verification ceremony. Plans must add only focused regression tests required by the changed behavior.

Routine implementation checks should converge on:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke
cargo test --test sync_multibatch -- --test-threads=1
```

The final target name may differ if an existing integration target is extended instead. Phase 13G must record the exact final command.

Each phase adds only the focused integration target(s) named in its plan. Do not run every test single-threaded. Do not enable all features unless the target requires test-only support.

At phase closure, `bash scripts/check.sh` represents the routine developer/CI gate. `scripts/release-check.sh` remains a manual release helper, not a second CI system. Deep recovery/failpoint tests may remain available as manual commands but should not be duplicated across routine and release paths.

The manual release helper must include the sustained server lifetime/graceful SIGTERM regression because that test intentionally waits beyond 30 seconds. It must also include the complete encrypted multi-batch regression.

No phase may create a new verification script unless it replaces and deletes more existing verification code than it adds and the roadmap is amended first.

## 6. Commit strategy

Use one implementation commit per coherent phase workstream where practical, followed by at most one closure/documentation commit. Do not create one commit per test or helper function.

Recommended sequence:

```text
phase-13a: correct server lifetime and configuration parsing
phase-13b: bound sync uploads and improve clock diagnostics
phase-13c: reduce routine verification and release ceremony
phase-13d: remove measured runtime and archive duplication
phase-13e: simplify auto-sync and transaction scope
phase-13f: consolidate supported surfaces and documentation
phase-13g: correct multi-batch upload and coordinated shutdown
phase-13: record verified closure
```

Every phase plan must be updated with:

- implementation SHA(s);
- corrective SHA(s), where applicable;
- exact focused commands run;
- observed result;
- any accepted residual limitation;
- confirmation that its non-goals were respected;
- truthful final status.

Do not commit generated logs, screenshots, bloat reports, timing traces, or evidence directories. Short before/after measurements belong as a table in the relevant plan.

## 7. Phase closure gates

### 13A closes when

- the server remains healthy beyond the previous 30-second lifetime boundary;
- shutdown is driven by one shared signal/cancellation source covering Ctrl-C and Unix SIGTERM;
- graceful drain timeout starts only after shutdown is requested or a service fails;
- both service tasks remain owned and are awaited or aborted after timeout;
- unexpected termination of either HTTP or gRPC service stops the sibling and returns an error;
- malformed environment configuration fails with the variable name and supplied value;
- local plaintext/TLS acknowledgement behavior remains unchanged;
- focused lifetime, graceful signal, service-failure, and config tests pass.

### 13B closes when

- upload requests are bounded by serialized size and count;
- every upload batch is preflighted before the first remote mutation;
- pagination/upload batching cannot resend unexpectedly, return early, or omit a batch;
- one oversized item in the first, middle, or final position fails before network mutation with a clear diagnostic;
- normal small-library sync remains one request where practical;
- multi-batch sync obtains its authoritative response only after all successful uploads;
- partial remote upload remains retryable and convergent without duplicate logical snippets;
- server download pagination and merge semantics remain unchanged;
- clock rejection errors identify skew and corrective action;
- one real encrypted multi-batch round trip, one partial-failure convergence case, and oversized-item cases pass.

### 13C closes when

- routine CI has one Linux correctness path and macOS/Windows platform smoke only;
- redundant standalone debug build steps are removed;
- broad `--all-features` and global `--test-threads=1` usage is eliminated where not required;
- the fast complete encrypted multi-batch regression is present in routine verification;
- deep failpoint/recovery suites remain manually runnable but are not repeated in routine checks;
- release checks include the sustained lifetime/graceful-signal boundary without becoming a second broad CI suite;
- no new job, matrix, test framework, coverage target, or evidence artifact is introduced;
- documentation identifies the small canonical verification set.

### 13D closes when

- baseline and final native release sizes and top crate contributors are recorded;
- bundled themes reuse an already-linked decompressor or an objectively smaller equivalent;
- standalone release updates use one archive implementation across supported platforms where release tooling permits;
- local commands do not force Tokio/network initialization;
- the one-shot helper uses the smallest runtime flavor compatible with its actual work;
- retained changes show measured value and no feature regression;
- no compiler/toolchain complexity beyond the existing release profile is introduced.

### 13E closes when

- auto-sync has one authoritative execution lock and no parent probe that claims to reserve work;
- redundant helpers spawned concurrently exit cheaply without mutating state;
- pending intent, exact-generation acknowledgement, and local-first behavior remain correct;
- failure state is compact and understandable without a broad failure taxonomy where the user action is identical;
- ordinary single-file snippet mutations use atomic replace plus the existing local lock, not the full multi-file transaction state machine;
- durable journaling remains only for restore/replace/bulk/multi-file operations that need rollback;
- legacy transaction states retained in source are documented as on-disk recovery compatibility only;
- existing data and recovery artifacts are migrated or read compatibly without a new framework;
- implementation deletes more complexity than it adds;
- the bounded Phase 13G audit confirms these contracts or narrowly corrects the specific failed contract.

### 13F closes when

- the supported Rust API is explicitly documented and implementation-only modules are not represented as freely changeable without semver consequences;
- hidden implementation result types are not listed as supported stable API;
- integration tests use narrow test-support seams rather than public production exposure where practical;
- advanced commands are grouped coherently while old top-level spellings remain compatible aliases during this phase;
- server lifecycle commands do not grow into an internal service manager;
- metrics, persistent rate limiting, and similar non-core server facilities are optional/default-off only when feature gating produces a measured maintenance or footprint benefit;
- architecture documentation removes stale line-number references and duplicated claims;
- no current workflow or documented installation path is removed.

### 13G closes when

- all acceptance criteria in `plans/snip-it-phase-13g-corrective-closure.md` pass;
- the complete encrypted multi-batch regression fails on the reviewed baseline and passes on the correction;
- the server remains healthy beyond 30 seconds and exits normally through the graceful Unix SIGTERM path;
- both service tasks and persistence are drained in the required order;
- routine and release verification invoke the correct focused targets;
- the bounded Phase 13E audit is recorded;
- public API documentation contradictions are corrected;
- every Phase 13 plan contains a truthful status, implementation/corrective SHAs, commands, results, and residual deviations;
- release disposition is explicitly marked `CLEARED`.

## 8. Final closure criteria

Phase 13 is complete only when all statements are true:

- [x] Plans 13A through 13G are marked `COMPLETE` or `COMPLETE WITH DOCUMENTED DEVIATIONS` with implementation and corrective SHAs.
- [x] The server lifetime, shutdown coordination, and sync request-size/multi-batch defects are corrected.
- [x] Every upload batch is preflighted and no upload success path can omit later batches.
- [x] Ctrl-C and Unix SIGTERM use the same coordinated graceful-shutdown path.
- [x] Routine CI and local verification are materially smaller than the baseline while retaining the complete multi-batch regression.
- [x] Deep verification remains available where it has unique value but is not a mandatory duplicate gate.
- [x] Manual release verification includes sustained server lifetime and graceful signal shutdown.
- [x] The release client has a recorded before/after size and dependency comparison.
- [x] No user-visible feature was removed.
- [x] No new daemon, scheduler, protocol, persistence framework, distributed transaction, or CI framework was introduced.
- [x] Auto-sync and transaction machinery are simpler in source structure and state count, with residual compatibility machinery documented.
- [x] Local mutation durability and encrypted sync security properties remain intact.
- [x] Public API and CLI compatibility commitments are narrower, accurate, and documented.
- [x] Architecture documentation describes current symbols and invariants without volatile line-number inventories.
- [x] `bash scripts/check.sh` passes on the implementation platform.
- [x] `bash scripts/release-check.sh verify` passes from a clean tree.
- [x] Platform smoke CI is green, or a platform-specific defect is corrected in the phase that introduced it.
- [x] Phase 13G release disposition is `CLEARED`.
- [x] No generic follow-up hardening phase is opened.

When these criteria are met, mark this roadmap `COMPLETE` and close the line of work. Future expansion requires a reproduced defect, measured regression, or separately approved feature request.

## 9. Completion record

Status: COMPLETE

Implementation commits:
- `7e0d064` Phase 13A: Fix server lifetime defect and strict config parsing
- `84f5b7f` Phase 13B: Bounded sync uploads and clock-skew diagnostics
- `0575f38` + `33b27da` Phase 13C: Simplify CI, check scripts, and remove stale tests
- `181a142` Phase 13D: Client runtime and dependency footprint reduction
- `aa62bb4` + `a0df1ab` Phase 13E: Auto-sync and persistence simplification
- `01a860b` Phase 13F: API, CLI, and documentation surface consolidation

Corrective commit:
- `5d37fa7` Phase 13G: Fix sync batching, server shutdown, and config validation

Verification:
- `bash scripts/check.sh`: PASS

Release-blocking: No (13A-13F cleared by 13G); 13G was release-blocking and is now cleared.

Release disposition: CLEARED
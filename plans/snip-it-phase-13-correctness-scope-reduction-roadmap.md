# Phase 13 Roadmap — Correctness Repair, Verification Reduction, and Lightweight Scope Recovery

Status: READY FOR IMPLEMENTATION

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

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

## 2. Findings that drive this roadmap

### 2.1 Release-blocking correctness defects

1. `snip-sync serve` wraps the entire lifetime of the HTTP and gRPC tasks in a 30-second timeout. The process can therefore terminate after approximately 30 seconds even without a shutdown request. The timeout belongs only around graceful shutdown after a stop signal or service failure.
2. The local client accepts command bodies up to 16 MiB, while the sync server defaults to a 4 MiB gRPC message limit and the first sync request uploads the full local payload. A valid local snippet or sufficiently large library can therefore be permanently unsynchronizable.

### 2.2 Bounded correctness and operational issues

- malformed numeric and boolean server environment values can silently fall back to file/default configuration;
- server shutdown uses multiple independent signal listeners instead of one coordinated shutdown source;
- auto-sync scheduling probes an execution lock, releases it, then spawns a worker that reacquires it; this does not reserve execution and can permit redundant helper spawns;
- client/server clock skew is handled by a hard timestamp rejection without sufficiently direct diagnostics;
- line-number-heavy architecture documentation is already stale and costly to maintain.

### 2.3 Excess complexity relative to scope

- normal CI and local checks compile overlapping targets repeatedly and serialize far more tests than necessary;
- the release verification path repeats broad debug and release suites, crash matrices, package checks, and production-seam proofs regardless of the changed area;
- the client links multiple compression/archive implementations for bundled themes and platform-specific update archives;
- local-only command paths still carry runtime/network-oriented structure that should be lazy and isolated;
- auto-sync remains a compact but substantial scheduler with durable generations, several lock/state modules, backoff classifications, configuration fingerprints, and recovery paths;
- the transaction journal is database-grade and should be reserved for truly multi-file destructive operations rather than defining routine mutation architecture;
- the Rust public API, top-level CLI, and server administration surface expose more implementation detail and maintenance obligation than the core product needs.

## 3. Phase map

| Phase | Plan | Goal | Dependency |
|---|---|---|---|
| 13A | `plans/snip-it-phase-13a-server-lifetime-config-correctness.md` | Correct server lifetime, coordinated shutdown, and fail-closed configuration parsing | none |
| 13B | `plans/snip-it-phase-13b-sync-request-sizing-clock-diagnostics.md` | Make every locally valid snippet/library synchronizable within bounded requests and improve skew diagnostics | 13A may proceed in parallel |
| 13C | `plans/snip-it-phase-13c-verification-ci-simplification.md` | Reduce routine CI/test/release ceremony while retaining high-value coverage | 13A and 13B test targets identified |
| 13D | `plans/snip-it-phase-13d-client-runtime-dependency-footprint.md` | Apply measured, low-risk runtime and dependency reductions without feature loss | 13A/13B stable baseline |
| 13E | `plans/snip-it-phase-13e-auto-sync-persistence-simplification.md` | Simplify auto-sync and local transaction machinery without weakening local durability | 13A/13B complete; 13C defines reduced verification |
| 13F | `plans/snip-it-phase-13f-api-cli-server-surface-consolidation.md` | Narrow maintenance commitments while preserving compatible user workflows | 13E complete or stable |

Required sequence:

```text
13A + 13B
    -> 13C
    -> 13D
    -> 13E
    -> 13F
    -> closure
```

13A and 13B may be implemented independently. Phase 13C must not delete tests needed to validate unresolved defects. Phase 13D is measurement-gated. Phase 13E must simplify existing mechanisms rather than replace them with new frameworks. Phase 13F must preserve compatibility aliases and current install/deployment workflows during this phase.

## 4. Global constraints

### 4.1 Required outcomes

Phase 13 must leave the repository with:

- a sync server that remains healthy indefinitely until explicitly stopped or a service fails;
- one coordinated graceful-shutdown path with a timeout applied only after shutdown begins;
- explicit errors for malformed server environment configuration;
- bounded upload batching or an equivalent protocol-compatible mechanism so locally valid data does not exceed one gRPC request;
- a clear error for a single item that cannot fit the configured message limit;
- direct clock-skew diagnostics rather than a generic invalid-argument failure;
- a smaller routine CI path with no redundant standalone build after clippy/tests compile the workspace;
- single-threaded execution only for tests that actually use PTYs, process-global environment, keychain state, ports, or cross-process locks;
- deep crash/failpoint and release evidence checks moved to manual or change-scoped verification;
- measured binary attribution before dependency removal;
- no duplicate LZMA/DEFLATE or ZIP/tar archive stack when one existing implementation can serve the same features;
- local commands that do not initialize or depend on async/network services unless required;
- a simpler auto-sync state model with one execution authority and no ineffective parent lock reservation probe;
- durable transaction journaling limited to operations that genuinely mutate multiple files or require rollback;
- a narrower supported Rust API and clearer CLI/server grouping without immediate feature removal;
- architecture documentation based on symbols and invariants, not volatile line numbers.

### 4.2 Explicit non-goals

Do not add:

- new user-facing snippet, TUI, sync, metrics, or deployment features;
- a new daemon, service manager, queue, scheduler, IPC channel, or worker supervisor;
- CRDTs, vector clocks, consensus, event sourcing, or a new sync protocol;
- a new database or persistence layer;
- Kubernetes, cloud deployment, multi-tenant SaaS, or public-internet hardening;
- a generalized configuration framework;
- a new benchmarking harness, test framework, mock framework, fuzzing program, or model checker;
- coverage thresholds, binary-size gates, latency gates, release-evidence artifacts, or new CI jobs;
- automated crates.io publication or automatic release cadence;
- broad TUI rewrites, data-format redesigns, or command removals;
- speculative dependency rewrites without measured binary or maintenance value;
- new runtime dependencies solely to simplify existing code.

### 4.3 Security boundary

Simplification must retain:

- client-side encryption and authenticated ownership for sync;
- API key secrecy and keychain behavior;
- server request/message limits;
- safe archive extraction and checksum verification for updates;
- path containment and symlink protections for restore/update operations;
- atomic writes for user data;
- pre-operation backups for destructive bulk operations;
- a kernel-backed cross-process exclusion primitive where concurrent mutation would corrupt data;
- the rule that local mutations commit before optional remote synchronization.

The project can remove redundant layers around these properties, but must not remove the properties themselves.

## 5. Verification philosophy

Phase 13 intentionally reduces verification ceremony. Plans must add only focused regression tests required by the changed behavior.

Routine implementation checks should converge on:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke
```

Each phase adds only the focused integration target(s) named in its plan. Do not run every test single-threaded. Do not enable all features unless the target requires test-only support.

At phase closure, `bash scripts/check.sh` should represent the routine developer/CI gate. `scripts/release-check.sh` should remain a manual release helper, not a second CI system. Deep recovery/failpoint tests may remain available as manual commands but should not be duplicated across routine and release paths.

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
phase-13: record closure
```

Every phase plan must be updated with:

- implementation SHA(s);
- exact focused commands run;
- observed result;
- any accepted residual limitation;
- confirmation that its non-goals were respected.

Do not commit generated logs, screenshots, bloat reports, timing traces, or evidence directories. Short before/after measurements belong as a table in the relevant plan.

## 7. Phase closure gates

### 13A closes when

- the server remains healthy beyond the previous 30-second lifetime boundary;
- shutdown is driven by one shared signal/cancellation source;
- graceful drain timeout starts only after shutdown is requested;
- unexpected termination of either HTTP or gRPC service stops the sibling and returns an error;
- malformed environment configuration fails with the variable name and supplied value;
- local plaintext/TLS acknowledgement behavior remains unchanged;
- focused lifetime, shutdown, and config tests pass.

### 13B closes when

- upload requests are bounded by serialized size and count;
- pagination/upload batching cannot resend or omit a batch;
- one oversized item fails before network mutation with a clear diagnostic;
- normal small-library sync remains one request where practical;
- server download pagination and merge semantics remain unchanged;
- clock rejection errors identify skew and corrective action;
- one encrypted multi-batch round trip and one oversized-item case pass.

### 13C closes when

- routine CI has one Linux correctness path and macOS/Windows platform smoke only;
- redundant standalone debug build steps are removed;
- broad `--all-features` and global `--test-threads=1` usage is eliminated where not required;
- deep failpoint/recovery suites remain manually runnable but are not repeated in routine checks;
- release checks are change-scoped or explicitly manual;
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
- existing data and recovery artifacts are migrated or read compatibly without a new framework;
- implementation deletes more complexity than it adds.

### 13F closes when

- the supported Rust API is explicitly documented and implementation-only modules are no longer promised as stable without need;
- integration tests use narrow test-support seams rather than public production exposure where practical;
- advanced commands are grouped coherently while old top-level spellings remain compatible aliases during this phase;
- server lifecycle commands do not grow into an internal service manager;
- metrics, persistent rate limiting, and similar non-core server facilities are optional/default-off only when feature gating produces a measured maintenance or footprint benefit;
- architecture documentation removes stale line-number references and duplicated claims;
- no current workflow or documented installation path is removed.

## 8. Final closure criteria

Phase 13 is complete only when all statements are true:

- [ ] Plans 13A through 13F are marked COMPLETE with implementation SHAs.
- [ ] The server lifetime and sync request-size defects are corrected.
- [ ] Routine CI and local verification are materially smaller than the baseline.
- [ ] Deep verification remains available where it has unique value but is not a mandatory duplicate gate.
- [ ] The release client has a recorded before/after size and dependency comparison.
- [ ] No user-visible feature was removed.
- [ ] No new daemon, scheduler, protocol, persistence framework, or CI framework was introduced.
- [ ] Auto-sync and transaction machinery are simpler in source structure and state count.
- [ ] Local mutation durability and encrypted sync security properties remain intact.
- [ ] Public API and CLI compatibility commitments are narrower and documented.
- [ ] Architecture documentation describes current symbols and invariants without volatile line-number inventories.
- [ ] `bash scripts/check.sh` passes on the implementation platform.
- [ ] Platform smoke CI is green, or a platform-specific defect is corrected in the phase that introduced it.
- [ ] No generic follow-up hardening phase is opened.

When these criteria are met, mark this roadmap COMPLETE and close the line of work. Future expansion requires a reproduced defect, measured regression, or separately approved feature request.
# Phase 14 Roadmap — Correctness, Consolidation, and Lightweight Scope Recovery

Status: COMPLETE

Reviewed baseline: `c7a326f19afc77c9dd37e54448f9837fa494de04`

Production-code baseline: `f0ebd1a2246976217bf48260c2dbddd31163533d`

Date opened: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Phase 14 is a bounded follow-up to the completed Phase 13 line. It is not a new feature release and must not reopen already-closed sync/server hardening work.

The product model remains:

- `snp` is a lightweight, local-first terminal snippet manager;
- editable TOML files are the primary source of truth;
- the TUI and deterministic CLI paths should behave consistently;
- sync is optional and self-hosted;
- local mutations must remain safe when sync fails;
- cross-platform support matters, but production-SaaS hardening does not;
- simplification is preferred when two implementations provide the same behavior;
- verification should be proportional to a small local tool.

A post-Phase-13 review found several concrete correctness gaps plus remaining opportunities to reduce duplicated control flow, dependency weight, internal subsystem fragmentation, and routine verification cost.

The sequence in this roadmap deliberately fixes correctness before deleting or consolidating infrastructure.

## 2. Confirmed findings that define this phase

### 2.1 Correctness blockers

1. Root `Cargo.toml` uses `keyring = "3"` without selecting a platform credential-store feature. Keyring v3 requires explicit store features; without one, normal `Entry::new()` behavior falls back to the mock store rather than the native credential store. The client currently treats the keyring as durable credential storage.
2. Exact-selector `snp run --id/--description-exact/--command-exact --sync` does not execute the same immediate explicit sync path as ordinary TUI `run --sync`.
3. Exact-selector `snp clip ... --sync` drops the `--sync` request entirely.
4. `load_library()` backs up malformed TOML and then returns an empty library, allowing later writes to replace corrupted user data with synthesized empty state.
5. `LibraryManager::new()` similarly substitutes default metadata after malformed `libraries.toml`.
6. Missing or duplicate snippet IDs are repaired with random UUIDs during load without persistence, creating unstable identities across read-only invocations.

### 2.2 Duplication and simplification findings

1. Exact selector construction is repeated in `src/main.rs` for run, clip, and edit.
2. Explicit-sync lock/generation/reconciliation logic is repeated in `src/commands/mod.rs` and is not reused by exact-selector commands.
3. `clip_cmd::process_snippet()` and `clip_cmd::run_exact()` duplicate copy/audit/usage behavior.
4. Legacy top-level data commands and `snp data ...` duplicate dispatch/result mapping.
5. `classify_command()` and `startup_services()` maintain overlapping command taxonomies.
6. Auto-sync still contains avoidable policy reloads, obsolete compatibility helpers, and more module boundaries than the current behavior requires.
7. The asynchronous audit writer is disproportionate to the very low event rate of a local snippet manager.

### 2.3 Footprint opportunities

1. `arboard` default features pull image support even though snip-it only copies plain text.
2. The root client enables Tonic defaults, including server/router functionality that the `snp` client does not use.
3. The standalone self-updater carries tar/gzip extraction code and dependencies solely to unpack release assets.
4. `tracing-subscriber` and Tokio feature sets can be measured for further pruning after correctness is stable.

### 2.4 Verification scope

Current CI topology is acceptable: authoritative Linux correctness plus macOS/Windows smoke coverage. The remaining excess is duplicated platform-independent testing and accumulated low-information proof tests, not the existence of cross-platform CI itself.

## 3. Phase map

| Phase | Plan | Goal | Depends on |
|---|---|---|---|
| 14A | `plans/snip-it-phase-14a-credential-and-explicit-sync-correctness.md` | Restore real platform keychain behavior and exact-command `--sync` parity | none |
| 14B | `plans/snip-it-phase-14b-persistence-and-identity-correctness.md` | Fail closed on malformed persistent TOML and make legacy IDs stable | 14A recommended |
| 14C | `plans/snip-it-phase-14c-command-control-flow-consolidation.md` | Remove duplicated selector, sync, clip, data-dispatch, and startup classification paths | 14A, 14B |
| 14D | `plans/snip-it-phase-14d-dependency-and-binary-footprint.md` | Reduce binary/dependency weight without removing features | 14A |
| 14E | `plans/snip-it-phase-14e-runtime-internal-simplification.md` | Simplify auto-sync plumbing and audit logging without changing behavior | 14C |
| 14F | `plans/snip-it-phase-14f-verification-ci-reduction.md` | Reduce routine CI/test ceremony while retaining defect-focused coverage | 14A–14E |
| 14G | `plans/snip-it-phase-14g-transaction-boundary-decision.md` | Decide, with explicit guarantee tradeoffs, whether the transaction journal should be simplified | 14B–14F |

Required ordering:

```text
14A correctness
  -> 14B persistence/identity
  -> 14C control-flow consolidation
  -> 14D footprint reduction
  -> 14E internal simplification
  -> 14F verification reduction
  -> 14G transaction-boundary decision
```

14D may be executed in parallel with 14C after 14A if the executor can keep Cargo changes isolated, but sequential execution is preferred for smaller models.

## 4. Scope guardrails

Do not add:

- a new daemon, supervisor, queue, worker pool, database, or generalized event bus;
- a second sync implementation or second source of sync policy truth;
- new network protocols, RPCs, protobuf fields, or database migrations;
- a new CLI framework or command-dispatch DSL;
- a new workspace crate merely to share small helpers;
- new runtime dependencies solely for tests;
- coverage services, benchmark infrastructure, fuzz farms, nightly CI, scheduled CI, or additional OS matrices;
- production-grade distributed transaction semantics;
- new encryption schemes or authentication models;
- new user-visible features unrelated to the reviewed findings.

Do not remove:

- end-to-end snippet encryption;
- authenticated sync ownership;
- message-size/batching protections;
- atomic individual-file writes;
- backups before destructive replacement;
- kernel-backed cross-process locks;
- bounded sync retry/deadline behavior;
- graceful server shutdown and its deterministic orchestration helper;
- Windows/macOS compilation and smoke coverage.

## 5. Verification philosophy

Each implementation phase must use the smallest focused test set that proves its behavior, then run the normal project check once before handoff.

Default implementation verification:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

Do not run the full release suite after every helper refactor. `bash scripts/release-check.sh verify` is reserved for the end of a release-affecting phase or final Phase 14 closure.

When a plan asks for binary-size comparison, record actual release binary sizes before and after. Do not claim a size win from `cargo tree` alone.

## 6. Small-model execution rules

For every subplan:

1. Read the named files before editing.
2. Execute workstreams in listed order.
3. Keep each workstream diff inside its explicit file boundary.
4. Prefer deletion or reuse over a new abstraction.
5. Add one shared helper only when at least two current call sites need the same behavior.
6. Do not combine correctness changes with broad naming/style cleanup.
7. If a proposed dependency feature fails on one supported platform, stop and diagnose that platform rather than enabling a broad feature bundle blindly.
8. If behavior is intentionally weakened, stop: only Phase 14G may approve a transaction/durability guarantee reduction.
9. Keep compatibility aliases unless a plan explicitly proves they are removable.
10. Update architecture/docs only after production shape is final.

## 7. Phase 14 completion criteria

Phase 14 may be marked complete only when all are true:

- native keychain backends are intentionally enabled for supported client platforms, or a documented platform-specific deviation is recorded;
- exact `run --sync` and `clip --sync` use the canonical explicit-sync path;
- malformed library/index TOML cannot silently become writable empty state;
- legacy missing/duplicate snippet identities are stable across repeated read-only loads and become durable on the next legitimate library mutation;
- selector construction and explicit-sync orchestration have one canonical implementation each;
- clip exact/TUI operations share one copy implementation;
- duplicate data-command and startup-classification plumbing is reduced without changing CLI compatibility;
- unused clipboard image support is removed;
- Tonic client features are narrowed as far as supported by the actual client transport/TLS path;
- updater archive dependencies are removed only if release assets can preserve the existing update UX without adding equivalent complexity elsewhere;
- auto-sync and audit logging are simpler than the baseline without changing pending-generation or execution-lock guarantees;
- routine CI no longer reruns broad platform-independent suites on every OS;
- low-information tests are consolidated without removing direct regressions for previously reproduced defects;
- Phase 14G records an explicit retain/simplify decision for the transaction journal, including the exact durability guarantee chosen;
- final `bash scripts/check.sh` passes;
- final clean-tree `bash scripts/release-check.sh verify` passes before release clearance.

## 8. Release disposition

Current Phase 14 disposition: implementation required before the next correctness-focused release.

The keyring and exact `--sync` issues are release-affecting correctness defects. Persistence fail-closed behavior is a data-safety fix. Later consolidation, size, and verification phases are cleanup work and must not delay an emergency correctness release if 14A/14B need to ship first.

## 9. Expected end state

At Phase 14 closure, snip-it should remain the same product from the user's perspective, but with:

- fewer divergent command paths;
- reliable native credential storage;
- safer malformed-file behavior;
- stable legacy snippet identity;
- smaller production dependency surface;
- less auto-sync/logging plumbing;
- faster, less ceremonial routine verification;
- an explicit and proportionate durability contract for multi-file operations.

The preferred direction after Phase 14 is maintenance and deletion/consolidation, not another infrastructure expansion phase.

## 10. Phase 14G corrective closure

Phase 14G initially chose SIMPLIFY (commit `29fda50`), replacing the journal-based transaction engine with a minimal `InterruptedOperation` marker model. The corrective review found the marker model had correctness gaps (ambiguous rollback semantics for new vs. existing files, missing containment validation, loss of lock-scoped recovery) that would require rebuilding much of the retained transaction engine to fix. Since old journal backward compatibility was still needed, the result was two recovery models rather than one — increasing maintenance burden rather than reducing it.

The corrective pass reverted the marker implementation and restored the proven journal-based transaction path from the Phase 14F baseline. The final decision is RETAIN. Transaction simplification is closed for Phase 14; no Phase 14H will be created.

### Completed subplans

| Phase | Plan | Status |
|-------|------|--------|
| 14A | Credential and explicit-sync correctness | IMPLEMENTED |
| 14B | Persistence and identity correctness | IMPLEMENTED |
| 14C | Command and control-flow consolidation | IMPLEMENTED |
| 14D | Dependency and binary footprint reduction | IMPLEMENTED |
| 14E | Runtime and internal simplification | IMPLEMENTED |
| 14F | Verification and CI reduction | IMPLEMENTED |
| 14G | Transaction boundary decision | IMPLEMENTED (RETAIN) |

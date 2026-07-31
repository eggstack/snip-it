# Phase 12 Roadmap — Lightweight Correctness, Auto-Sync Simplification, and Footprint Control

Status: READY FOR IMPLEMENTATION

Baseline: `f77a86f8733868bb77712f7ad08a5ef5443782db`

This roadmap defines the bounded follow-up to the repository-wide review performed after the Phase 11L closure work.

The product goal remains unchanged: `snp` is a fast terminal snippet manager with a local TOML data model, a focused TUI, and optional self-hosted synchronization. The purpose of Phase 12 is not to add capabilities. It is to correct a small set of concrete defects, remove avoidable runtime complexity, and measure the client footprint before making targeted reductions.

The project is a lightweight personal/local tool. This roadmap explicitly rejects production-platform hardening programs, generalized verification frameworks, new CI topology, and speculative protocol redesign.

---

## 1. Why this phase exists

The current repository successfully implements the intended user-facing workflow, but the review identified four bounded problem classes:

1. **Concrete correctness and secret-handling defects** exist in process identity, server configuration, hidden worker behavior, and test instrumentation.
2. **Auto-sync state handling has fail-open and ambiguity paths** that can suppress pending work or permit overlapping execution.
3. **The two-process-per-cycle auto-sync architecture is larger than necessary** for the product and is itself creating lifecycle edge cases.
4. **The client binary links substantial optional infrastructure unconditionally**, but no measured size attribution currently exists to justify changes.
5. **Wall-clock-only sync ordering and incomplete recovery markers** can produce unstable conflict outcomes or orphaned remote state.

The correction strategy is deliberately sequential: fix current behavior first, then simplify it. Do not combine architectural deletion with unresolved correctness changes in one implementation commit.

---

## 2. Phase map

| Phase | Plan | Goal | Dependency |
|---|---|---|---|
| 12A | `plans/snip-it-phase-12a-secret-process-identity-correctness.md` | Remove production secret capture and correct process/config boundary behavior | none |
| 12B | `plans/snip-it-phase-12b-auto-sync-correctness-closure.md` | Make current auto-sync state and child lifecycle behavior fail closed and internally consistent | 12A process helpers where shared |
| 12C | `plans/snip-it-phase-12c-single-helper-auto-sync-simplification.md` | Collapse detached worker + executor into one bounded helper process without feature loss | 12B complete |
| 12D | `plans/snip-it-phase-12d-client-footprint-measurement-reduction.md` | Measure release footprint and apply only low-risk, evidence-backed reductions | 12A complete; may run after or alongside 12C measurement |
| 12E | `plans/snip-it-phase-12e-sync-ordering-recovery-semantics.md` | Make tie ordering deterministic and make recovery-marker claims truthful | 12A complete; independent of 12C |

Implementation should follow `12A -> 12B -> 12C`. Phase 12D may begin with baseline measurement after 12A, but final comparison must include the post-12C client. Phase 12E can be implemented after 12A without waiting for footprint work.

---

## 3. Global scope boundary

### Required outcomes

Phase 12 must leave the repository with:

- no production retention of bearer credentials for a test assertion;
- correct Linux `/proc/<pid>/stat` start-time parsing through one shared helper per crate boundary;
- process liveness that treats `EPERM` as alive and `ESRCH` as absent on Unix;
- kernel lock acquisition as the only authority for lock ownership;
- explicit error states for unreadable/corrupt pending or lock state;
- no generation rollback accepted as normal auto-sync work;
- child executor cleanup on every worker exit path while the two-process model still exists;
- nonzero hidden-worker exit status for actual failure;
- exact `PathBuf` re-execution without lossy UTF-8 conversion;
- malformed existing server configuration causing startup failure rather than silent defaults;
- one helper process per auto-sync attempt after Phase 12C;
- measured release-binary attribution and only evidence-backed footprint changes;
- deterministic sync tie ordering without CRDTs;
- recovery markers that either complete recovery or do not claim to do so.

### Explicit non-goals

Do not introduce:

- new end-user features;
- CRDTs, vector clocks, distributed consensus, or a general event log;
- a new sync protocol unless a separate future plan is approved;
- a resident daemon for the client;
- a detached service manager;
- a queue database;
- multiple new lock files;
- a generalized task scheduler;
- a new observability stack;
- coverage targets or mandatory performance gates;
- new CI jobs, release jobs, matrices, or artifact evidence uploads;
- fuzzing, model checking, soak tests, or large stress suites;
- broad refactoring of the TUI, snippet model, library layout, or command surface;
- security work unrelated to a reproduced defect in this roadmap;
- production code changes solely to make tests easier.

### Verification philosophy

Use the existing lightweight checks:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --lib -- --test-threads=1
cargo check --workspace --all-targets --all-features
```

Run only the focused integration tests named by each phase. `bash scripts/check.sh` is appropriate at phase closure. The release-check suite is not required unless a modified path is specifically covered only there.

Do not create a new verification script for Phase 12.

---

## 4. Architectural target after Phase 12

### Local command path

The ordinary `snp` command should remain:

```text
parse CLI -> initialize only required runtime services -> read/mutate local TOML -> optionally schedule helper -> exit
```

Read-only commands such as `version`, completion generation, listing, selection, and validation must not initialize unnecessary audit infrastructure or attempt background recovery unless their documented behavior requires it.

### Auto-sync path

The target is one opportunistic helper process:

```text
mutation command
  -> commit local transaction
  -> record/update durable pending generation
  -> attempt detached helper spawn
  -> parent exits

helper
  -> acquire sole sync execution lock
  -> debounce pending generation
  -> run canonical sync directly with bounded network timeout
  -> clear exact acknowledged generation or preserve newer generation
  -> record compact success/failure status
  -> exit
```

The parent may still start multiple helpers under rapid concurrent commands; only one acquires the kernel lock and performs work. There is no separate executor subprocess after Phase 12C.

Retain only:

- the durable pending marker;
- the short pending transaction lock used to serialize generation updates;
- one kernel-backed sync execution lock;
- one compact status/backoff file;
- the existing opt-in auto-sync policy.

Delete or retire components made unnecessary by the second subprocess. Do not replace them with a new abstraction layer.

### Sync ordering

Conflict ordering should remain last-write-wins in spirit, but equal timestamps must have a deterministic, stable tie-break independent of which side is called “server.” The preferred bounded form is:

```text
(updated_at, device_id, snippet_id)
```

A server-issued revision may be used only if the current schema already supports it cleanly or the implementation remains narrowly additive. Do not introduce a migration-heavy revision system merely to avoid the deterministic tuple.

---

## 5. Commit and handoff strategy

Each phase should normally use one implementation commit and one optional documentation/closure commit.

Recommended commit sequence:

```text
phase-12a: correct secret and process identity boundaries
phase-12b: close auto-sync state and child lifecycle defects
phase-12c: collapse auto-sync to one helper process
phase-12d: reduce measured client footprint
phase-12e: make sync ordering and recovery deterministic
phase-12: record closure verification
```

A phase may be split into two implementation commits when client and server compilation boundaries make that safer. Do not create one commit per test or one commit per small function.

Every implementation commit must update its plan status only after the focused checks pass. Record commands and the implementation SHA in the plan. Do not add screenshots, generated logs, or evidence directories.

---

## 6. Phase-level closure gates

### Phase 12A closes when

- production server builds contain no API-key capture storage;
- all Linux start-token parsers read field 22 correctly and have direct tests;
- Unix liveness distinguishes `EPERM` from `ESRCH`;
- executable respawn uses `Path`/`OsStr` without lossy conversion;
- hidden worker failure exits nonzero;
- existing malformed server config fails startup with a useful error;
- focused client/server tests and workspace checks pass.

### Phase 12B closes when

- pending corruption/unreadability cannot become `NoPending`;
- unexpected lock errors cannot become `SpawnNow`;
- spawn failure is represented truthfully;
- startup recovery consults the kernel lock, not PID metadata;
- generation rollback is rejected and preserved as diagnostic evidence;
- executor wait errors terminate/reap the child before lock release;
- current two-process behavior has focused regression tests.

### Phase 12C closes when

- `auto-sync-execute` and executor spawning are removed or retained only as a temporary compatibility shim with no production scheduling use;
- the helper directly invokes the canonical sync operation;
- exact-generation clear and newer-generation preservation remain correct;
- timeout behavior is enforced at the sync/network boundary without a second process;
- user-visible auto-sync configuration and commands remain unchanged;
- obsolete worker/executor tests and docs are reduced rather than duplicated.

### Phase 12D closes when

- baseline and final release sizes are recorded for at least the native development platform;
- top contributors are documented from `cargo bloat` and `cargo tree -e features`;
- only changes with measured value and low maintenance cost are retained;
- no user-visible feature is removed;
- no size target is invented after the fact;
- speculative protocol replacement is explicitly deferred.

### Phase 12E closes when

- equal timestamp merges produce the same result on every device and invocation order;
- deletion semantics remain stable and tested;
- recovery markers either contain enough data to resume or are replaced with truthful best-effort behavior;
- remote library creation is not repeated merely because a local marker was deleted;
- no CRDT or broad schema redesign is introduced.

---

## 7. Final closure criteria

Phase 12 is complete only when all statements below are true:

- [ ] Plans 12A through 12E are marked COMPLETE with implementation SHAs.
- [ ] All high-severity and medium-severity defects named in the review are either corrected or explicitly documented as accepted with a concrete reason.
- [ ] Auto-sync production execution uses one helper process per attempt, not a worker supervising a second executor.
- [ ] The ordinary local snippet workflow retains all current features.
- [ ] The release binary has a recorded before/after size comparison.
- [ ] CI topology is unchanged unless a pre-existing job required a trivial command update after code deletion.
- [ ] No new dependency was added solely for planning, measurement, locking, scheduling, or testing.
- [ ] Architecture documentation reflects the final one-helper model.
- [ ] `bash scripts/check.sh` passes on the implementation platform.
- [ ] Platform CI is green or any platform-specific failure is addressed in the phase that caused it.
- [ ] No additional generic hardening phase is opened.

When these criteria are met, close this line of work. Future changes require a reproduced user-visible defect, a measured regression, or an explicitly approved feature request.
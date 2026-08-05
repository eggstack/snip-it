# Phase 13E — Auto-Sync and Persistence Simplification

Status: COMPLETE

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Dependencies: Phases 13A and 13B complete; Phase 13C reduced verification model available

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Reduce the amount of scheduling, state, lock, and transaction machinery required to support optional auto-sync and local TOML durability, without weakening the core local-first guarantees.

The current architecture is correct in many edge cases but disproportionate to the product’s actual risk model. Auto-sync still resembles a compact job scheduler, and routine local mutations inherit concepts from a restartable multi-file transaction engine.

The target is intentionally smaller:

```text
ordinary one-file mutation
  -> acquire existing local mutation lock
  -> atomic replace
  -> record/update dirty generation if auto-sync is enabled
  -> opportunistically spawn one detached helper
  -> return local success

helper
  -> acquire one authoritative sync execution lock
  -> debounce briefly
  -> sync full current state with one bounded deadline
  -> clear only the generation it observed, or preserve newer dirty state
  -> record compact last result
  -> exit

multi-file destructive operation
  -> pre-operation backup
  -> bounded transaction journal only when rollback across files is required
  -> commit/rollback
  -> record dirty generation after local durability
```

This phase must delete concepts rather than rename or redistribute them.

## 2. Non-negotiable guarantees

Retain:

- successful local mutation is durable before remote work begins;
- failed sync never rolls back a successful local mutation;
- at most one sync execution mutates sync state at a time;
- a newer pending generation is never cleared by an older helper;
- corrupt pending/status state fails visibly and is not interpreted as clean;
- explicit `snp sync` remains available and can recover pending work;
- destructive multi-file restore/replace operations retain a backup and rollback path;
- atomic writes, path containment, symlink rejection, and appropriate permissions remain;
- existing on-disk artifacts are read or migrated safely.

Do not retain machinery solely because tests exist for it. Tests should follow the simplified contract.

## 3. Explicit non-goals

- no resident daemon;
- no service-manager integration;
- no queue database or IPC channel;
- no new lock files;
- no generalized scheduler or retry service;
- no CRDT/protocol redesign;
- no multi-process supervisor;
- no background thread retained by the parent command;
- no broad rewrite of library storage or backup formats;
- no removal of manual sync, cron support, or current auto-sync configuration surface;
- no weakening of update/archive, encryption, or keychain safety;
- no new transaction framework or embedded database;
- no exhaustive new failpoint matrix.

## 4. Workstream A — Remove ineffective parent execution-lock probe

### Problem

The scheduler probes the execution lock, releases it, then spawns a helper that reacquires it. The probe does not reserve work. Concurrent callers can all observe availability and spawn redundant helpers.

### Target

Make worker acquisition the sole execution authority.

Required behavior:

- scheduler decides only whether policy/config/pending/backoff permit a spawn;
- it does not acquire the sync execution lock as a precondition;
- helper immediately tries the execution lock;
- helper that sees the lock held exits success/`NothingToDo` cheaply;
- stale metadata remains diagnostic only;
- no new spawn lock is added.

This may allow redundant short-lived helper processes under simultaneous mutation. That is acceptable for this local tool if only one performs sync and losers exit before network or state mutation.

Add one bounded concurrency test proving N simultaneous scheduling attempts produce at most one sync execution, without requiring exactly one spawned process.

## 5. Workstream B — Collapse scheduling/status policy

### Current complexity to review

- multiple schedule decisions;
- detailed failure-class taxonomy;
- retry disposition mapping;
- configuration fingerprints used to release deferrals;
- attention-required state;
- several timestamps and consecutive-failure counters;
- worker lifetime, debounce, max delay, request deadline, and backoff interactions.

### Target compact state

Prefer one small durable status structure:

```toml
schema_version = 1
last_attempt_at_unix_ms = 0
last_success_at_unix_ms = 0
next_attempt_at_unix_ms = 0
last_result = "success|transient_failure|configuration_failure|local_failure"
message = "sanitized diagnostic"
observed_generation = 0
```

Exact fields may differ. The user-action taxonomy should be small:

- success;
- transient retryable failure;
- configuration/authentication failure requiring user correction;
- local persistence/corruption failure requiring repair.

Do not preserve separate categories when they produce the same scheduling decision and user guidance.

### Backoff

Retain a simple bounded transient backoff. Configuration/authentication failures may defer until explicit retry, startup, or a configuration file modification signal that can be determined cheaply. Do not retain a general fingerprinting framework if explicit retry is sufficient.

A simple policy is acceptable:

- transient failure: fixed or short exponential backoff capped at a modest interval;
- configuration/auth failure: do not automatically respawn until next explicit sync or later mutation/startup after the backoff interval;
- corrupt local state: do not spawn; show repair guidance.

Do not create timers or persistent scheduled jobs. Scheduling remains opportunistic on mutation/startup/cron/explicit retry.

## 6. Workstream C — Simplify debounce and helper loop

Retain only behavior necessary to avoid a helper per keystroke-like mutation burst:

- initial debounce duration;
- one maximum wait bound preventing indefinite postponement;
- one overall network/retry deadline;
- exact-generation clear.

Review whether `worker_lifetime` and `max_delay` are separate concepts. Prefer one maximum pre-sync debounce window plus one sync deadline.

After a successful sync:

- if a newer generation exists and helper lifetime remains, one immediate follow-up sync is permitted;
- otherwise leave the newer generation dirty for the next scheduler opportunity.

After any failure:

- record compact failure/backoff;
- preserve pending state;
- exit immediately;
- do not loop into newer work and bypass backoff.

Do not add a long-running loop, condition variable, or wake channel.

## 7. Workstream D — Reduce auto-sync module count

The current `src/auto_sync/` directory should be consolidated around responsibilities rather than one file per small concept.

A reasonable target is approximately:

```text
mod.rs          public/internal entry points and paths
pending.rs      dirty generation plus short mutation serialization
execution.rs    kernel execution lock, spawn, helper run
status.rs       compact result/backoff state
policy.rs       config resolution and small scheduling decision
```

Exact file count is not a gate. Required outcome is fewer cross-module transitions and fewer exported types.

Candidates to merge/delete include separate files for:

- worker lock metadata if execution lock is authoritative;
- spawn wrapper if only one call site remains;
- test event emission if no retained high-value test requires production runtime checks;
- notification and schedule layers whose logic becomes a small sequence;
- separate lock/pending lock wrappers that can be thin internal uses of the existing process lock primitive.

Do not consolidate into one very large file. Delete abstractions whose caller count and semantic value no longer justify them.

## 8. Workstream E — Restrict transaction engine to multi-file destructive operations

### Classification

Inventory every call to the transaction boundary and classify it:

1. one destination file only;
2. multiple files but operation can be independently atomic/idempotent;
3. multiple files requiring coherent rollback.

### Ordinary one-file mutations

For snippet create/edit/delete/tag/output/favorite and other one-library writes:

- use existing local data lock;
- validate input;
- write one complete TOML replacement atomically;
- record pending sync only after write success;
- do not create a transaction journal or backup artifact unless the command is explicitly destructive/bulk.

### Multi-file operations retaining journaling

Likely retain bounded journaling for:

- restore replace;
- repair that modifies multiple independent files;
- library deletion when index plus library file must remain coherent;
- bulk import/replace across multiple files;
- schema migration touching more than one durable artifact.

Review each rather than assuming all require the full current state machine.

### Simplified journal target

For retained operations, prefer:

```text
Prepared
Committing { next_file }
RollingBack { next_file }
CleaningUp
```

Keep enough information to restore pre-operation backups and safely resume. Remove legacy/new states that are only needed to coordinate auto-sync pending finalization if dirty generation can be recorded after commit and recovered independently.

A transaction may complete local data first and leave sync dirty recording to a small post-commit recovery rule. Do not couple the transaction state machine to remote sync execution.

### Backups and integrity

Retain:

- pre-operation backup for destructive replace/repair;
- hashes where they uniquely detect corrupt backup/staging content;
- permission/path validation;
- atomic journal writes;
- cleanup of owned artifacts.

Remove duplicate hashes or per-step states when atomic rename plus one verified backup already provides the needed guarantee.

## 9. Workstream F — Compatibility and migration

Existing users may have:

- current pending files;
- current status files;
- lock metadata files;
- interrupted transaction journals;
- recovery markers from sync.

Required compatibility strategy:

- read current known schemas;
- convert compactly on first successful access where safe;
- preserve corrupt/unrecognized state and direct the user to repair;
- do not silently delete pending work;
- keep persistent lock files harmless if kernel ownership is absent;
- complete or roll back existing transaction journals before using the simplified writer;
- remove obsolete artifacts only after their state has been safely consumed.

Do not create a general migration registry. One explicit compatibility parser/converter per affected artifact is sufficient.

## 10. Workstream G — Test reduction aligned with architecture

Replace tests of removed internal state transitions with contract tests:

- successful mutation leaves valid local data and dirty generation;
- failed local mutation leaves prior data and does not advance generation;
- simultaneous helpers produce at most one sync execution;
- successful helper clears exact generation;
- newer generation survives older success;
- failed helper preserves dirty state and records backoff;
- corrupt pending/status refuses automatic work;
- explicit sync recovers pending state;
- one-file mutation creates no transaction journal;
- retained multi-file operation recovers from one representative interruption before commit and one during commit;
- backup/restore smoke remains correct.

Delete exhaustive matrices for removed states. Do not build a replacement event sink or failpoint framework.

## 11. Likely files

Auto-sync:

- `src/auto_sync/*.rs`
- `src/config.rs`
- `src/status_snapshot.rs`
- `src/main.rs` startup recovery classification only if entry points change
- auto-sync command modules

Persistence:

- `src/transaction.rs`
- `src/local_data.rs`
- `src/library.rs`
- mutating command modules
- `src/commands/restore_cmd.rs`
- `src/commands/repair_cmd.rs`
- `src/commands/import_cmd.rs`
- `src/commands/library_cmd.rs`

Tests/docs:

- existing auto-sync, transaction, restore, repair, and process-lock targets
- `architecture/auto_sync.md`
- `architecture/persistence.md`
- `AGENTS.md`
- `USER_GUIDE.md` status/repair guidance

Do not modify sync protocol, server lifetime, update archives, or CLI grouping in this phase.

## 12. Implementation order

### Pass 1 — Inventory and deletion map

1. map auto-sync types/files to callers and user-visible contracts;
2. map transaction callers by one-file/multi-file/rollback need;
3. identify compatibility artifacts;
4. list tests tied only to internal states targeted for removal;
5. record intended deletions before adding replacement code.

### Pass 2 — Scheduler/execution simplification

1. remove parent execution-lock probe;
2. simplify spawn decision;
3. compact failure/status categories;
4. simplify debounce and failure exit;
5. merge/delete redundant modules;
6. keep exact-generation semantics.

### Pass 3 — Persistence scope reduction

1. route ordinary one-file mutations around transaction journals;
2. simplify retained journal states;
3. decouple pending sync finalization from transaction states;
4. retain backup/rollback only where necessary;
5. add compatibility conversion.

### Pass 4 — Tests and docs

1. add contract tests first;
2. remove obsolete state-machine tests;
3. reduce test-support seams no longer needed;
4. update architecture docs by symbol/invariant;
5. record source/module/state counts before and after.

## 13. Verification

Focused commands depend on final target names, but should include:

```text
cargo fmt --all -- --check
cargo clippy -p snip-it --all-targets -- -D warnings
cargo test -p snip-it --lib auto_sync
cargo test -p snip-it --lib transaction
cargo test --test auto_sync_closure --features test-support -- --test-threads=1
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
bash scripts/check.sh
```

After simplification, remove commands for deleted targets from documentation and release scripts. Run retained deep recovery targets once as a migration proof, but do not add them back to routine CI.

## 14. Acceptance criteria

- [ ] Worker acquisition is the sole sync execution authority.
- [ ] Scheduler no longer probes and releases the execution lock before spawn.
- [ ] Concurrent helper losers exit before network or status mutation.
- [ ] Failure/status categories are reduced to distinct user actions.
- [ ] Failed helper preserves dirty state and exits into backoff.
- [ ] Successful helper clears only the observed generation.
- [ ] Newer dirty generation is preserved or processed once within a bounded helper lifetime.
- [ ] Auto-sync source/module/type count is materially reduced.
- [ ] Ordinary one-file mutations create no transaction journal.
- [ ] Multi-file destructive operations retain backup and rollback.
- [ ] Transaction state count and sync coupling are reduced.
- [ ] Existing pending/status/journal artifacts are handled compatibly or preserved for repair.
- [ ] Local atomicity, containment, permissions, and encryption guarantees remain.
- [ ] Removed internal-state tests are replaced only by smaller contract tests.
- [ ] No daemon, scheduler, database, IPC, new lock file, migration framework, or test framework is introduced.
- [ ] More implementation/test complexity is deleted than added.
- [ ] `bash scripts/check.sh` and focused recovery checks pass.

## 15. Quantitative completion record

At closure, record a small table:

| Metric | Before | After |
|---|---:|---:|
| `src/auto_sync` production files | | |
| auto-sync production LOC | | |
| auto-sync public/internal exported types | | |
| durable auto-sync artifact types | | |
| transaction states | | |
| commands using transaction journals | | |
| auto-sync/transaction integration test files | | |
| focused verification elapsed time | | |

These are descriptive. Do not turn them into permanent gates.

## 16. Stop conditions

Stop and amend the plan if:

- a simplification weakens exact-generation clearing or local-first durability;
- compatibility requires silently discarding unknown pending/journal state;
- ordinary mutations genuinely touch multiple files that cannot be made independently atomic;
- module consolidation creates a monolithic file with less clarity;
- a proposed replacement introduces timers, IPC, queues, background services, or more locks;
- tests begin reconstructing the deleted internal scheduler as a harness;
- scope drifts into protocol, TUI, server deployment, or release packaging.

This phase is successful only if the architecture becomes visibly smaller and easier to reason about.

## 17. Completion record

Status: COMPLETE

Implementation commits: `aa62bb4` + `a0df1ab` — Phase 13E: Auto-sync and persistence simplification

Corrective commit: `5d37fa7` — Phase 13G: Fix sync batching, server shutdown, and config validation

Verification:
- `bash scripts/check.sh`: PASS

Acceptance criteria: All items satisfied. Execution-lock probe removed, failure classes collapsed to 4 variants, module count reduced, one-file mutations bypass journals, legacy states retained for on-disk recovery only.

Release-blocking: No (cleared by 13G)
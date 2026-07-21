# snip-it Correctness Program: Post-Closure Remaining Work

## Status

The Phase 01–03 corrective closure was implemented in:

```text
ff506f5934957c4fd989224a6f0e0cf10f907567
```

That commit establishes the implementation baseline for all remaining work:

- canonical real synchronization in the executor;
- worker-owned execution locking;
- success-only generation-safe pending clear;
- independent executor timeout;
- centralized scheduling and worker-storm prevention;
- typed policy/config failure handling;
- immutable debounce policy and injected-clock timing;
- native Windows process liveness;
- lossless failure-class exit codes;
- generation-aware durable status;
- bounded retry/backoff;
- stable CRC32 status integrity and typed corruption handling.

## Authority

The post-closure plans listed below supersede the earlier Phase 04–09 drafts with matching subjects. The earlier documents remain useful historical design records, but implementation agents should use these files as the authoritative handoff set:

1. `snip-it-correctness-04a-operational-visibility-and-recovery.md`
2. `snip-it-correctness-05a-deterministic-end-to-end-test-architecture.md`
3. `snip-it-correctness-06a-core-architecture-and-public-api-tightening.md`
4. `snip-it-correctness-07a-local-data-durability-and-recovery.md`
5. `snip-it-correctness-08a-cli-and-automation-polish.md`
6. `snip-it-correctness-09a-security-release-and-program-closure.md`

## Dependency order

```text
Phase 04A: operational visibility and recovery
    |
    v
Phase 05A: deterministic end-to-end test architecture
    |
    v
Phase 06A: core architecture and public API tightening
    |
    v
Phase 07A: local data durability and recovery
    |
    v
Phase 08A: CLI and automation polish
    |
    v
Phase 09A: security, release, and program closure
```

Phase 05A may begin test-harness scaffolding while Phase 04A is being implemented, but its closure criteria depend on the final Phase 04A command and recovery contracts. Phase 06A must not perform broad structural extraction until Phase 05A protects the user-visible process and storage contracts. Phase 09A is the only phase allowed to mark the program complete.

## Cross-phase invariants

Every remaining phase must preserve these already-closed invariants:

1. Local mutations commit before remote work.
2. Sync failures never roll back successful local state.
3. Sync-configured mutations record pending intent even when automatic execution is disabled or policy loading fails.
4. Only a real successful sync may clear pending intent.
5. Conditional clear cannot remove a newer generation.
6. One shared execution lock serializes detached, foreground, cron, and explicit sync paths.
7. The detached worker owns the execution lock for its entire cycle; the executor does not reacquire it.
8. Timeout termination and reap complete before lock release.
9. All automatic worker spawning flows through the central scheduling authority.
10. Durable status is advisory/result state; pending is the source of truth for unsynchronized intent.
11. Unknown or corrupt state never maps to current or successful.
12. Internal worker/executor subprocesses remain hidden implementation details of the single installed `snp` binary.

## Program constraints

The remaining program must not introduce:

- a resident daemon;
- a second installed helper binary;
- a system service requirement;
- a general workflow engine;
- remote command execution;
- a plugin runtime;
- distributed coordination;
- a hosted sync dependency beyond the existing optional self-hosted server;
- CRDT/realtime collaborative editing;
- replacement of editable TOML with an opaque local database.

A test-only helper binary or feature is permitted only when excluded from release artifacts and clearly documented.

## Completion model

Each phase must produce:

- implementation commits in the recommended order;
- focused unit and integration evidence;
- Linux, macOS, and Windows coverage for platform-sensitive behavior;
- documentation reconciliation;
- a phase status file recording commit range, tests, limitations, and deferred work.

The final Phase 09A status file must summarize the entire program and prove that all required plans are closed.
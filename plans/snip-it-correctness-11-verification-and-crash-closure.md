# Phase 11 — Verification, Crash Correctness, and Final Closure

Status: READY FOR IMPLEMENTATION

Authoritative baseline: `609ddca5611894684d2ca04a10138ddc606ff301`

Supersedes closure claims in:

- `plans/snip-it-correctness-program-closure-status.md`
- the completed checkboxes in `plans/snip-it-correctness-10-final-corrective-closure.md`

This plan does not erase the Phase 10 implementation. Phase 10 fixed substantial defects. It reopens only the remaining correctness and evidence gaps identified by direct code review after `609ddca`.

The program must remain open until every release-blocking criterion in this document is supported by executable evidence and successful CI runs.

---

## 1. Objective

Close the remaining gap between implemented behavior and claimed release correctness.

The required outcome is not another broad feature pass. It is a bounded closure pass proving that:

1. read-only and dry-run commands cannot trigger startup recovery or network work;
2. the auto-sync headline test requires a real server-side state transition;
3. multi-file restore survives handled failures and process crashes without partial state;
4. transaction locks recover safely after process death;
5. backups represent one coherent local generation;
6. every backed-up entry has a defined and tested restore contract;
7. execution failures map to the documented public exit-code contract;
8. update archives are validated consistently on Unix and Windows;
9. package and CI evidence is real, reproducible, and accurately documented.

This is the final correctness gate. Do not begin unrelated product work while it is open.

---

## 2. Architectural constraints

Preserve the existing lightweight architecture.

Required constraints:

- one installed client binary: `snp`;
- detached workers remain one-shot subprocesses;
- no resident client daemon;
- no background service manager;
- no plugin runtime;
- no workflow engine;
- no database migration solely to solve these issues;
- no second installed helper binary;
- no remote code execution feature;
- no broad CLI redesign.

New internal types, failpoints, test helpers, local lock records, and transaction states are allowed when they directly support the closure invariants.

---

## 3. Release-blocking findings at the baseline

The implementation baseline contains the following unresolved gaps.

### 3.1 Dry-run recovery classification

`classify_command` maps the top-level `Restore` variant to `StartupRecoveryPolicy::Allow`, so `snp restore --mode dry-run` can attempt startup recovery before dispatch.

The current read-only test does not create valid pending intent and therefore cannot detect the bug.

The same audit must cover:

- `restore --mode dry-run`;
- `import ... --dry-run`;
- `repair --dry-run`;
- `sync run --dry-run`, which manages its own explicit sync behavior;
- any other command whose read/write behavior depends on nested flags.

### 3.2 Headline E2E permits no remote effect

The deterministic auto-sync test now reads server state, but accepts a post-sync count of either zero or one.

A zero server-side effect must never satisfy the headline proof.

### 3.3 Transaction journal is not crash-complete

Restore persists a prepared journal before backup paths are assigned. The enriched in-memory journal is not durably rewritten before live replacements.

After a process crash, the persisted journal may not contain enough information to roll back.

### 3.4 No automatic transaction crash recovery

Interrupted journals are detectable but are not automatically recovered or surfaced through a complete operator workflow before subsequent mutation.

### 3.5 Transaction lock can remain stale forever

The lock is a bare `create_new` file with no PID, nonce, creation timestamp, liveness check, or ownership-aware cleanup.

### 3.6 Backup snapshot is not serialized with mutations

Reading all source files into memory sequentially does not prove the library files and index came from one coherent generation.

### 3.7 Included general configuration is not restored

Backup emits entries with `kind = "config"`, but restore does not apply them.

### 3.8 Manifest domain validation remains permissive

The manifest uses free-form kind strings and does not fully reject:

- unsupported schema versions;
- unknown entry kinds;
- duplicate normalized destinations;
- case-fold collisions;
- unsafe generic configuration names;
- cross-platform trailing-dot or trailing-space aliases;
- control characters beyond NUL.

### 3.9 Execution failure mapping is incomplete

Normal nonzero child exits are propagated, but timeout, signal termination, and shell/spawn failures can still fall through the generic error path and exit `1` instead of the documented execution-failure code.

### 3.10 Windows ZIP extraction is not prevalidated

Unix tar extraction validates paths and types. Windows ZIP extraction delegates directly to PowerShell `Expand-Archive`.

### 3.11 Package CI extraction is incorrect

The package job uses `unzip` for Cargo `.crate` files, which are tar/gzip archives.

### 3.12 Closure evidence is stale

The closure status identifies an older final commit and overstates several proofs, including coherent backup snapshots, complete archive hardening, and server-observable E2E success.

---

# Workstream A — Reopen status and preserve evidence integrity

## Goal

Make repository status truthful before implementation changes begin.

## Required first commit

Update `plans/snip-it-correctness-program-closure-status.md` to include:

```text
Program status: REOPENED
Blocking plan: plans/snip-it-correctness-11-verification-and-crash-closure.md
Baseline: 609ddca5611894684d2ca04a10138ddc606ff301
```

Requirements:

- retain historical phase evidence;
- distinguish implemented behavior from verified behavior;
- list each Phase 11 release blocker;
- remove the claim that all release blockers are resolved;
- do not mark this phase complete in the same commit that reopens it;
- do not pre-check acceptance criteria.

## Closure evidence file

At the end of the pass, create:

`plans/snip-it-correctness-11-closure-status.md`

It must include:

- baseline and final commit SHAs;
- exact changed files by workstream;
- exact test commands;
- test counts and ignored-test reasons;
- CI workflow run URL;
- each job name and conclusion;
- platform-specific evidence;
- residual limitations;
- a criterion-by-criterion closure table.

## Acceptance criteria

- the repository cannot appear closed while Phase 11 is open;
- the final status references the actual final commit;
- claims are traceable to code, tests, and CI jobs;
- no criterion is marked complete based only on a commit message.

---

# Workstream B — Make startup recovery classification operation-aware

## Goal

Prevent every read-only and dry-run operation from spawning workers, starting executors, changing pending state, changing status state, or contacting a server.

## Required design

Classification must inspect effective operation semantics, not only the top-level enum variant.

Recommended structure:

```rust
fn classify_command(command: &Commands) -> StartupRecoveryPolicy {
    match command {
        Commands::Restore { mode: RestoreMode::DryRun, .. } =>
            StartupRecoveryPolicy::SuppressReadOnly,
        Commands::Restore { .. } => StartupRecoveryPolicy::Allow,
        // ...
    }
}
```

Apply equivalent nested matching to:

- import dry-run;
- repair dry-run;
- sync dry-run;
- any future nested command with read-only and mutating modes.

Prefer an exhaustive match. Adding a new command or subcommand should require an explicit policy decision at compile time.

## Stronger test setup

Every read-only recovery test must:

1. create valid sync configuration;
2. create a valid pending marker with generation `G`;
3. create a status snapshot with known bytes `S0`;
4. enable lifecycle events;
5. start a recording server or use an endpoint whose connection attempts are observable;
6. invoke exactly one command;
7. assert no worker event;
8. assert no executor event;
9. assert server request count remains zero;
10. assert pending generation remains exactly `G`;
11. assert status remains byte-identical to `S0`, except for commands explicitly documented to write status;
12. assert expected stdout/stderr and exit code.

Do not infer “no worker” from the absence of a phrase in the status file.

## Required command matrix

At minimum:

- `snp status`;
- `snp status --json`;
- `snp get --id ...`;
- `snp list`;
- `snp list --json`;
- `snp validate`;
- `snp doctor`;
- `snp backup`;
- `snp restore ... --mode dry-run`;
- `snp import pet ... --dry-run`;
- `snp repair --dry-run`;
- `snp library list`;
- `snp library show`;
- `snp completions ...`;
- `snp shell init ...`;
- `snp --help` and representative subcommand help;
- `snp sync run --dry-run`, proving no generic startup recovery occurs before the explicit dry-run path.

## Closure criteria

- no read-only or dry-run command can start startup recovery;
- no command in the matrix changes pending generation;
- no command in the matrix contacts the recording server;
- tests fail if `StartupRecoveryPolicy::Allow` is substituted for any matrix entry;
- documentation lists the commands that may trigger opportunistic recovery.

---

# Workstream C — Require real server-observable auto-sync success

## Goal

Make the headline E2E test impossible to pass without a real canonical sync request and durable server-side state transition.

## Eliminate the keychain exception

The test must not accept zero remote effect on macOS or any other platform.

Implement one deterministic test credential strategy:

### Preferred option: test-only credential backend

Under `test-support`, allow the integration environment to install an in-memory or file-backed credential provider used consistently by parent, worker, and executor.

Requirements:

- production builds cannot enable it accidentally;
- no secret appears in argv;
- the same credential is available to all subprocesses;
- the provider is isolated per test directory;
- the test does not depend on the host keychain.

### Acceptable option: explicit test-only plaintext mode

A test-only environment flag may force plaintext credential serialization and deserialization consistently when `test-support` is compiled.

It must not weaken production credential behavior.

## Required recording evidence

The server test handle must expose:

- canonical sync request count;
- request start and completion events;
- authenticated client/device identity;
- target library identity;
- pre-sync server revision `R0`;
- post-sync server revision `R1`;
- stored encrypted payload presence;
- maximum concurrent sync requests.

## Required exact sequence

The headline test must prove:

1. server revision is `R0`;
2. server request count is zero;
3. local mutation commits;
4. pending generation `G` exists;
5. exactly one worker starts;
6. exactly one executor starts;
7. exactly one canonical sync request starts;
8. the request is authenticated as the expected test device;
9. the request targets the expected library;
10. server revision becomes `R1`;
11. `R1 != R0`;
12. server stores the expected encrypted payload;
13. executor records success;
14. status records success for generation `G`;
15. pending generation `G` is conditionally cleared;
16. maximum concurrent sync requests is one;
17. request count remains one through a bounded quiet period.

## No-op regression proof

Provide a test-only executor mode that returns immediate local success without performing canonical sync, or an equivalent mutation-test seam.

The headline test must fail because:

- request count remains zero;
- revision remains `R0`;
- no expected library payload exists.

The no-op mode must be unavailable in production builds.

## Timing discipline

- use barriers, event polling, or server state polling;
- no arbitrary sleep may be the primary proof of an event;
- a bounded quiet-period wait is allowed only to prove no duplicate request;
- timeout failures must print observed events, request counts, revisions, pending state, and status content.

## Platform matrix

Run the headline test with `test-support` on:

- Linux;
- macOS;
- Windows.

A platform may be excluded only with a documented, temporary, release-blocking reason. “Host keychain behavior” is not an acceptable exclusion after the deterministic credential backend is implemented.

## Closure criteria

- the test requires exactly one server request;
- the test requires `R1 != R0`;
- zero server-side effect always fails;
- no-op executor behavior always fails the headline proof;
- all three OS jobs pass with the same semantic assertions.

---

# Workstream D — Make restore transactions crash-complete

## Goal

Guarantee that restore cannot leave an unrepairable or silently partial state after a handled error or abrupt process termination.

## Required transaction state model

Use explicit durable states. Recommended minimum:

```rust
pub enum TransactionState {
    Prepared,
    BackupsDurable,
    Committing { next_index: usize },
    Committed,
    RollingBack { next_index: usize },
    RolledBack,
    Failed { class: String },
}
```

Exact names may differ, but the journal must distinguish:

- plan created;
- backups durably available;
- live replacement in progress;
- committed;
- rollback in progress;
- rolled back;
- unrecoverable failure requiring operator action.

## Journal completeness

Before the first live replacement, persist for every affected destination:

- normalized destination path;
- whether the destination existed;
- durable backup path when it existed;
- intended action: create, replace, delete, or no-op;
- staged content path or content hash;
- original hash when applicable;
- intended new hash;
- file durability/permission class;
- deterministic replacement order;
- transaction nonce;
- owning PID and process start identity where available.

The enriched journal must be written atomically and durably before live writes begin.

Do not rely on an in-memory clone containing information absent from the persisted journal.

## Write protocol

Required sequence:

1. acquire local-data transaction lock;
2. validate manifest and source artifacts;
3. parse all incoming TOML;
4. compute the complete restore result in memory;
5. identify exact affected destinations;
6. create durable backups for every existing affected destination;
7. create durable staged replacement files in destination directories;
8. persist the complete `BackupsDurable` journal;
9. update journal to `Committing { next_index: 0 }`;
10. atomically replace one destination;
11. durably advance `next_index`;
12. repeat until complete;
13. mark committed;
14. record exactly one pending generation when syncable data changed;
15. schedule auto-sync once after commit;
16. clean staged files, backups, and journal according to the documented retention policy.

## Rollback protocol

Rollback must:

- run in reverse replacement order;
- atomically restore original bytes;
- remove newly created destinations that did not exist before the transaction;
- restore permissions where supported;
- durably advance rollback progress;
- remain restartable if rollback itself is interrupted;
- retain evidence instead of deleting it when rollback cannot complete.

Do not use an unvalidated direct `fs::copy` as the final live restore primitive.

## Crash recovery entry points

Before any new mutating local operation:

1. inspect transaction state;
2. recover or refuse with a typed diagnostic;
3. never silently continue with an interrupted transaction.

Read-only behavior:

- `snp status` and `snp doctor` may report interrupted transaction state;
- they must not mutate it automatically.

Operator behavior:

Provide an explicit command, likely under `snp repair`, to:

- inspect interrupted transactions;
- perform automatic safe rollback;
- report unrecoverable missing backup/staged artifacts;
- emit machine-readable JSON.

Automatic mutation-start recovery may perform rollback only when the journal is complete and the operation is unambiguous.

## Failpoint architecture

Add test-only failpoints, compiled only with `test-support`, at minimum:

- after complete journal persistence;
- after first backup;
- after all backups;
- after first live replacement;
- after index replacement;
- before commit marker;
- during rollback after first restore.

Failpoints must support:

- returning an injected error;
- immediate process termination for crash simulation.

Production builds must not enable failpoints.

## Required tests

Handled failure tests:

- failure after first live replacement;
- failure after library replacements but before index replacement;
- failure after index replacement but before commit;
- rollback restores exact original bytes;
- newly created destinations are removed on rollback;
- rollback creates no pending generation;
- rollback schedules no worker.

Crash subprocess tests:

- kill after complete journal persistence;
- kill after first live replacement;
- kill after index replacement;
- kill during rollback;
- next mutating invocation detects the journal;
- automatic recovery or explicit repair restores a coherent pre-transaction state;
- no transaction lock remains permanently blocking;
- journal and backups are retained if recovery cannot safely complete.

No-op tests:

- identical merge creates no transaction writes beyond read-only planning, or creates a transaction that cleanly resolves as no-op without pending work;
- dry-run creates no transaction artifacts;
- successful multi-file restore creates exactly one pending generation.

## Closure criteria

- persisted journal contains all rollback metadata before live writes;
- a process kill after any tested replacement point is recoverable;
- handled failure and crash recovery produce byte-exact original state;
- a failed or rolled-back restore creates no pending generation;
- a successful content-changing restore creates one generation increment, not merely one `generation` line;
- no partial state is accepted as success;
- crash tests run in CI.

---

# Workstream E — Make transaction lock ownership and stale recovery correct

## Goal

Prevent both concurrent transaction corruption and permanent deadlocks after process death.

## Required lock record

Persist a structured lock record containing:

- schema version;
- PID;
- process start identity where available;
- random nonce;
- created timestamp;
- transaction ID;
- hostname or machine identifier only if useful and non-sensitive.

## Acquisition rules

1. create with exclusive create semantics;
2. read back and verify ownership;
3. if an existing lock belongs to a live process with matching start identity, return a typed contention error;
4. if the process is dead, reclaim only after validating the lock record;
5. if the record is malformed, preserve or quarantine it and require repair unless safe recovery is unambiguous;
6. unlock only when PID/start identity/nonce match the guard.

## Platform liveness

- Unix: use a PID liveness check plus process start identity where practical;
- Windows: use `OpenProcess` and process exit status, matching the corrected auto-sync lock approach;
- protect against PID reuse;
- document unavoidable same-user race limitations.

## Required tests

- live owner blocks second acquisition;
- dead Unix owner is reclaimed;
- dead Windows owner is reclaimed;
- PID reuse with mismatched start identity is reclaimed or refused safely;
- wrong nonce cannot remove a live lock;
- malformed lock is not silently deleted;
- crash subprocess leaves a reclaimable lock;
- two concurrent restores cannot both enter commit;
- backup snapshot coordination, if using the same lock, does not deadlock.

## Closure criteria

- process death cannot permanently block local durability operations;
- live ownership cannot be stolen;
- stale recovery is tested on Linux, macOS, and Windows;
- lock cleanup is ownership-aware.

---

# Workstream F — Produce a coherent backup generation

## Goal

Ensure a backup’s library files and index/config describe one coherent local state.

## Required synchronization design

Choose and implement one explicit model.

### Preferred model: shared local-data lock

Introduce a lightweight `LocalDataLock` used by:

- snippet create/update/delete;
- library create/delete/set-primary;
- imports;
- restore;
- repair writes;
- migrations;
- sync merge writes;
- backup snapshot capture.

Backup should hold the lock only while:

1. enumerating the defined source set;
2. validating source types;
3. reading exact bytes into memory;
4. capturing the local generation/fingerprint.

Release the lock before writing or compressing backup output.

The lock may be the strengthened transaction lock if semantics remain clear and contention is bounded.

### Alternative model: generation seqlock

A persistent local generation counter may be used only if every local mutation increments it transactionally.

Backup must:

1. read generation `G0`;
2. capture all bytes;
3. read generation `G1`;
4. accept only when `G0 == G1` and no mutation was in progress;
5. retry with a bounded limit otherwise.

A timestamp-only or sequential-read claim is insufficient.

## Source validation

For every captured source:

- reject symlinks and reparse points;
- require regular file;
- enforce canonical containment under config root;
- enforce size limits;
- validate expected TOML/schema before reporting backup success;
- preserve exact bytes after validation;
- sort entries deterministically.

## Output atomicity

- write backup into a temporary sibling directory;
- write all files and manifest;
- fsync files and directory according to the documented durability tier;
- atomically rename the temporary directory to the final path where supported;
- remove partial output on handled failure;
- refuse to overwrite an existing non-empty output unless an explicit policy exists.

## Required tests

- concurrent snippet mutation during snapshot capture;
- concurrent library create/delete during snapshot capture;
- concurrent set-primary during snapshot capture;
- backup either captures the complete before-state or complete after-state;
- no mixed index/library generation is accepted;
- injected output failure leaves no valid-looking partial backup;
- deterministic manifest ordering;
- symlink and non-regular source rejection;
- no recursive inclusion of current or historical backup directories.

## Closure criteria

- a backup cannot contain a mismatched index and library set;
- concurrency tests prove before-or-after coherence;
- in-memory sequential reading without coordination is no longer presented as sufficient evidence;
- partial output cannot be mistaken for a complete backup.

---

# Workstream G — Make manifest and restore contracts closed and typed

## Goal

Ensure every manifest entry is known, safe, unique, and actually restorable.

## Typed manifest model

Replace free-form entry-kind strings in restore logic with a typed enum.

Recommended variants:

```rust
pub enum BackupEntryKind {
    Library,
    LibraryIndex,
    Usage,
    SyncConfig,
    GeneralConfig,
}
```

Unknown kinds must fail closed for supported schema versions.

## Schema policy

- define the currently supported schema version;
- reject schema `0` unless explicitly migrated;
- reject future schema versions with a clear diagnostic;
- validate `layout` exactly;
- document compatibility policy.

## Path normalization

Reject:

- empty names;
- absolute Unix paths;
- Windows drive absolute paths;
- drive-relative paths such as `C:foo`;
- UNC paths;
- `.` and `..` components;
- mixed separators that become traversal cross-platform;
- ASCII control characters;
- trailing dots or spaces that alias on Windows;
- reserved Windows device names;
- hidden control names;
- duplicate normalized source paths;
- duplicate normalized destination paths;
- case-fold collisions on case-insensitive filesystems.

Canonicalize the backup root once and verify each safely opened artifact remains contained beneath it.

## General configuration contract

Choose one of these and make help/docs/tests match.

### Option 1: implement general config restore

Define an allowlist of supported config filenames, for example:

- theme/configuration TOML files that are safe and understood;
- never arbitrary filenames;
- never pending, status, locks, logs, journals, credentials, caches, or backups.

For each allowed file:

- validate TOML/schema before writes;
- map to an exact destination;
- include it in transaction backup/staging/rollback;
- report conflicts deterministically.

### Option 2: remove `--include-config`

If no meaningful stable general-config set exists, remove the flag and `config` manifest entries.

Do not generate entries that restore silently ignores.

## Duplicate identity checks

Before writes:

- reject duplicate library filenames;
- reject duplicate snippet IDs within incoming libraries unless a documented repair mode is selected;
- reject two manifest entries resolving to the same destination;
- reject case-only aliases where the target filesystem is case-insensitive or conservatively on all platforms.

## Required tests

- unsupported schema;
- unknown kind;
- duplicate source path;
- duplicate destination path;
- case-fold collision;
- trailing-dot/trailing-space collision;
- drive-relative Windows path;
- general-config round trip or proof that the flag was removed;
- arbitrary config filename rejection;
- duplicate snippet ID rejection;
- exact restore report for skipped, replaced, and refused entries.

Tests must require failure. Avoid assertions of the form `!success || message contains ...` when the operation must be rejected.

## Closure criteria

- every manifest entry has a defined restore implementation;
- unknown kinds fail closed;
- unsupported schema fails closed;
- duplicate/colliding destinations cannot reach the write phase;
- `--include-config` either round-trips supported files or no longer exists.

---

# Workstream H — Complete execution outcome semantics

## Goal

Make all snippet execution failures conform to one typed public contract.

## Required outcome type

Return a typed execution result to the top-level mapper. No command module should call `std::process::exit` directly.

Recommended model:

```rust
pub enum SnippetExecutionOutcome {
    Success,
    Cancelled,
    ExitFailure { code: Option<i32>, signal: Option<i32> },
    TimedOut,
    SpawnFailure { class: SpawnFailureClass },
}
```

Map it consistently through `CliOutcome`.

## Public mapping

Choose and document one stable policy.

Current documentation implies:

- child exit code available: propagate child code;
- signal termination: exit `8`;
- timeout: exit `8`;
- spawn/shell failure: exit `8`;
- infrastructure/persistence failure unrelated to execution: exit `1`;
- cancelled before execution: exit `4`.

If the policy changes, update docs and tests atomically.

## Usage and audit behavior

- record usage only after child exit `0`;
- do not record usage after timeout, signal, spawn failure, or nonzero exit;
- audit failure class without emitting raw command text;
- exact and TUI execution paths must share the mapper.

## Exact edit strengthening

The implementation uses ID-native edit, but tests must prove success rather than tolerate failure.

Tests must:

- provide actual stdin content;
- assert exit code `0`;
- parse the library TOML;
- assert target ID received the exact output;
- assert every distractor is byte-equivalent except for allowed metadata changes;
- reject duplicate IDs instead of editing the first match;
- assert one pending generation only when output is part of the syncable contract.

## Required execution tests

- child exit `0`;
- child exit `1`;
- child exit `8`, documenting collision with wrapper code;
- child exit `127`;
- real timeout using a command that exceeds the configured limit;
- signal termination on Unix;
- missing/invalid shell or spawn target;
- failed execution records no usage;
- successful execution records usage exactly once;
- exact edit with duplicate descriptions;
- exact edit with overlapping commands;
- duplicate ID refusal;
- exact clip identity preservation;
- no raw command content in failure diagnostics.

## Closure criteria

- timeout exits with the documented execution code;
- spawn failure exits with the documented execution code;
- signal termination exits with the documented execution code;
- no lower-level execution path directly exits the process;
- exact edit tests require and verify successful target mutation.

---

# Workstream I — Harden update extraction uniformly

## Goal

Apply the same path, type, and containment guarantees to tar and ZIP updates.

## Required design

Prefer Rust-native ZIP inspection and extraction.

Add the `zip` crate or an equivalent audited library and:

1. enumerate every entry before extraction;
2. reject absolute paths;
3. reject parent traversal;
4. reject drive prefixes and UNC paths;
5. reject trailing-dot/space aliases;
6. reject symlinks and other non-regular entry types;
7. reject duplicate normalized destinations;
8. reject case-fold collisions;
9. bound entry count;
10. bound individual and total uncompressed size;
11. require the expected binary name at the expected path;
12. reject unexpected extra executable payloads;
13. extract only after all entries validate;
14. extract into an isolated temporary directory;
15. verify the extracted binary is a regular file;
16. preserve rollback behavior around installation.

Do not delegate unvalidated archive contents directly to PowerShell `Expand-Archive`.

Apply equivalent limits to tar extraction, including entry count and total uncompressed size.

## Production-code tests

Tests must call the actual production validators/extractors through a library-visible internal API or unit tests in the module.

Do not replicate validation logic solely inside the integration test.

## Crafted archive tests

Create actual archives containing:

- `../outside`;
- absolute paths;
- nested traversal;
- symlink entries;
- hard-link entries;
- duplicate normalized paths;
- case-fold collisions;
- ZIP-bomb-like high expansion ratio within a bounded fixture;
- oversized entries;
- unexpected extra binary;
- valid one-binary release archive.

Run ZIP tests on Windows and at least one non-Windows platform when the parser is cross-platform.

## URL policy

`fetch_url` must reject every non-HTTPS scheme, including unknown schemes and scheme-relative inputs, rather than only explicit `http://`.

Redirects must remain constrained to HTTPS via curl protocol flags.

## Closure criteria

- no platform extracts an archive before validating all entries;
- Windows update extraction is not delegated to unvalidated `Expand-Archive`;
- production extraction code is exercised by crafted archive tests;
- non-HTTPS and unknown schemes fail closed;
- extraction limits prevent unbounded disk expansion.

---

# Workstream J — Repair CI/package evidence and action policy

## Goal

Make the release evidence executable and truthful.

## Package job correction

Cargo `.crate` files are tar/gzip archives.

Replace `unzip` with a correct extractor, for example:

```bash
mkdir "$UNPACK_DIR"
tar -xzf "$CRATE_FILE" -C "$UNPACK_DIR"
```

Then:

- build the unpacked package with `--locked` when supported;
- run `snp --version`;
- run `snp --help`;
- run a minimal isolated command using a temporary config directory;
- verify no repository-relative path dependency is required.

## Cross-platform package smoke

Run an install/package smoke matrix on:

- Ubuntu;
- macOS;
- Windows.

The full unpacked package build may remain Linux-only only if the matrix independently installs and runs the packaged crate on the other platforms.

## Action pinning policy

The workflow currently claims all actions are pinned to full SHAs while `dtolnay/rust-toolchain@stable` remains mutable.

Choose one:

- pin the action to a full commit SHA; or
- explicitly document a narrow exception and stop claiming universal immutable pinning.

The preferred outcome is full SHA pinning.

Add a CI policy check that scans `uses:` entries and rejects unapproved mutable refs.

## Required CI jobs

At minimum:

- format;
- Clippy with all targets and all features;
- workspace tests on Linux/macOS/Windows;
- release-mode tests;
- Phase 11 lifecycle/E2E matrix with `test-support`;
- restore crash-injection tests;
- update archive security tests including Windows ZIP;
- package build from unpacked `.crate`;
- package/install smoke matrix;
- cargo-deny for all workspace crates;
- action pinning policy;
- repository hygiene.

## Evidence capture

The final closure status must record:

- workflow run URL;
- commit SHA;
- workflow attempt number;
- every required job conclusion;
- ignored tests and why;
- any platform exclusion as an open blocker.

A local “all tests pass” statement is not sufficient.

## Closure criteria

- package extraction uses the correct format;
- unpacked package builds without repository context;
- installed binary smoke tests pass on all supported OSes;
- action pinning claim matches the workflow;
- required CI jobs are green on the final commit;
- CI run evidence is committed to the closure status.

---

# Workstream K — Reconcile documentation and close only on proof

## Goal

Make documentation describe shipped and proven behavior exactly.

## Documents to reconcile

At minimum:

- `README.md`;
- `SECURITY.md`;
- `AGENTS.md`;
- `architecture/auto_sync.md`;
- `architecture/cli.md`;
- `architecture/persistence.md`;
- `docs/COMMAND_CONTRACTS.md`;
- `docs/EXIT_CODES.md`;
- `docs/PERSISTENCE_INVENTORY.md`;
- `docs/SECURITY_AUDIT.md`;
- `docs/THREAT_MODEL.md`;
- `plans/snip-it-correctness-program-closure-status.md`;
- `plans/snip-it-correctness-11-closure-status.md`.

## Claims that require special review

- read-only command side effects;
- restore crash recovery;
- transaction rollback semantics;
- stale-lock handling;
- backup snapshot consistency;
- backup include flags;
- manifest schema compatibility;
- child execution exit codes;
- server-observable E2E evidence;
- update archive validation;
- release signing versus checksums;
- action pinning;
- package/install evidence;
- exact final commit and test counts.

## Remove stale or invalid assertions

Do not claim that backups exclude raw snippet commands. Snippet commands are the backed-up user data.

Security assertions should instead prove exclusion of:

- API keys;
- encryption keys;
- keychain material;
- pending/status/locks unless explicitly supported;
- logs;
- transaction journals;
- temporary files;
- unrelated external symlink targets.

## Closure criteria

- no document claims zero-remote-effect E2E success;
- no document calls sequential unlocked reads a coherent snapshot;
- no document claims crash recovery beyond implemented behavior;
- no document claims universal SHA pinning while mutable action refs remain;
- final test counts are generated from the final commit;
- closure status names the actual final commit.

---

# 4. Implementation order

Implement in this order to minimize false evidence and repeated rework.

## Commit 1 — Reopen status

- Workstream A only.

## Commit 2 — Operation-aware recovery classification

- Workstream B implementation and tests.

## Commit 3 — Deterministic credentials and server-observable E2E

- Workstream C.

## Commit 4 — Transaction journal/state redesign

- Workstream D journal and rollback foundation.

## Commit 5 — Transaction lock ownership and stale recovery

- Workstream E.

## Commit 6 — Crash injection and recovery wiring

- Workstream D failpoints, subprocess tests, startup/repair recovery.

## Commit 7 — Coherent backup snapshot

- Workstream F.

## Commit 8 — Typed manifest and general config contract

- Workstream G.

## Commit 9 — Execution outcome completion

- Workstream H.

## Commit 10 — Uniform update extraction hardening

- Workstream I.

## Commit 11 — CI and package evidence repair

- Workstream J.

## Commit 12 — Documentation and final closure evidence

- Workstream K.
- Change program status to complete only here, after green final CI evidence exists.

Do not squash the initial reopen commit into the final closure commit. The history should show that the repository was reopened before implementation and closed only after evidence.

---

# 5. Verification commands

Run from repository root on the final candidate commit.

## Formatting and static analysis

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Full tests

```bash
cargo test --workspace --all-features
cargo test --release --workspace --all-features
```

## Phase 11 focused suites

```bash
cargo test --test readonly_no_recovery --features test-support
cargo test --test deterministic_e2e --features test-support
cargo test --test restore_security --features test-support
cargo test --test restore_transactions --features test-support
cargo test --test backup_contracts --features test-support
cargo test --test execution_outcomes --features test-support
cargo test --test update_archive_security --features test-support
```

Add and run dedicated suites where appropriate:

```bash
cargo test --test transaction_crash_recovery --features test-support
cargo test --test backup_snapshot_concurrency --features test-support
cargo test --test manifest_contracts --features test-support
cargo test --test package_install_contracts
```

## Package proof

```bash
cargo package -p snip-proto --locked
cargo package -p snip-sync --locked
cargo package -p snip-it --locked

CRATE_FILE=$(ls target/package/snip-it-*.crate | head -1)
UNPACK_DIR=$(mktemp -d)
tar -xzf "$CRATE_FILE" -C "$UNPACK_DIR"
cd "$UNPACK_DIR"/snip-it-*
cargo build --release --locked
cargo install --path . --locked
snp --version
snp --help >/dev/null
```

Adjust platform shell syntax without weakening the proof.

## Documentation checks

```bash
git grep -n "server_count_after == 0"
git grep -n "Program status: COMPLETE" plans/
git grep -n "all actions are pinned" .github docs plans
git grep -n "raw command content" docs plans tests
git grep -n "unzip.*\.crate\|unzip.*CRATE_FILE" .github
```

Expected final state:

- no permissive zero-remote-effect assertion;
- only the final truthful closure status says complete;
- action pinning claims match actual refs;
- backup security text does not claim snippet commands are absent;
- package jobs do not use ZIP extraction for `.crate` files.

---

# 6. Cross-platform evidence matrix

| Capability | Linux | macOS | Windows |
|---|---:|---:|---:|
| Operation-aware read-only recovery tests | Required | Required | Required |
| Server-observable headline E2E | Required | Required | Required |
| Transaction handled rollback | Required | Required | Required |
| Transaction crash recovery | Required | Required | Required |
| Stale transaction-lock recovery | Required | Required | Required |
| Backup concurrency/coherence | Required | Required | Required |
| Typed manifest path/schema tests | Required | Required | Required |
| Execution exit-code tests | Required | Required | Required |
| Signal termination test | Required | Required | N/A with documented reason |
| Tar extraction security | Required | Required | Optional if ZIP-only release |
| ZIP extraction security | Required | Required | Required |
| Package/install smoke | Required | Required | Required |

A skipped required cell is a release blocker unless this plan is explicitly amended with a documented product-support change.

---

# 7. Explicit final closure criteria

The program may return to `COMPLETE` only when every item below is true.

## Status and evidence

- [ ] Program status was reopened before Phase 11 implementation.
- [ ] A dedicated Phase 11 closure-status file exists.
- [ ] Closure status names the actual final commit SHA.
- [ ] Closure status includes a successful final CI workflow URL.
- [ ] Every required CI job is listed with a successful conclusion.
- [ ] Ignored tests are listed with justified reasons.

## Read-only behavior

- [ ] `restore --mode dry-run` suppresses startup recovery.
- [ ] import dry-run suppresses startup recovery.
- [ ] repair dry-run suppresses startup recovery.
- [ ] read-only tests begin with valid pending generation `G`.
- [ ] read-only tests assert no worker and no executor lifecycle events.
- [ ] read-only tests assert zero recording-server requests.
- [ ] read-only tests assert pending remains exactly `G`.
- [ ] read-only tests assert status remains unchanged where required.

## Server-observable auto-sync

- [ ] Test credentials are deterministic and independent of host keychain behavior.
- [ ] Headline E2E requires exactly one server request.
- [ ] Headline E2E requires server revision transition `R0 -> R1`.
- [ ] Headline E2E verifies expected library identity and encrypted payload.
- [ ] Headline E2E verifies exactly one worker and one executor.
- [ ] Headline E2E verifies pending clear only after remote effect.
- [ ] Headline E2E verifies no duplicate request during quiet period.
- [ ] No-op executor mode fails the headline E2E proof.
- [ ] Headline E2E passes on Linux, macOS, and Windows.

## Transaction crash correctness

- [ ] Complete rollback metadata is persisted before the first live replacement.
- [ ] Journal state records commit progress durably.
- [ ] Rollback progress is durable and restartable.
- [ ] Handled failure after a live replacement restores exact original bytes.
- [ ] Crash after first live replacement is recoverable.
- [ ] Crash after index replacement is recoverable.
- [ ] Crash during rollback is recoverable or leaves explicit retained evidence.
- [ ] Newly created files are removed during rollback.
- [ ] Rolled-back restore creates no pending generation.
- [ ] Successful multi-file restore increments pending generation exactly once.
- [ ] No-op merge creates no pending work.
- [ ] Dry-run creates no transaction artifacts.

## Transaction locking

- [ ] Lock records PID, start identity, nonce, and transaction identity.
- [ ] Live ownership blocks a second transaction.
- [ ] Dead ownership is reclaimed safely.
- [ ] PID reuse cannot steal a live lock.
- [ ] Wrong nonce cannot unlock another transaction.
- [ ] Stale-lock recovery passes on Linux, macOS, and Windows.

## Backup coherence

- [ ] Backup capture is coordinated with every local mutation writer.
- [ ] Concurrent mutation tests prove before-or-after snapshot coherence.
- [ ] Index and library files cannot come from different generations.
- [ ] Source symlinks and non-regular files are rejected.
- [ ] Backup output is staged and finalized atomically.
- [ ] Handled output failure leaves no valid-looking partial backup.
- [ ] Manifest order is deterministic.

## Manifest and restore domain

- [ ] Manifest schema version is validated.
- [ ] Future unsupported schema fails closed.
- [ ] Unknown entry kind fails closed.
- [ ] Duplicate normalized source paths fail closed.
- [ ] Duplicate normalized destination paths fail closed.
- [ ] Case-fold collisions fail closed.
- [ ] Windows drive-relative and UNC aliases fail closed.
- [ ] Trailing-dot and trailing-space aliases fail closed.
- [ ] Every emitted backup entry kind has an implemented restore path.
- [ ] General config files round-trip safely, or `--include-config` is removed.
- [ ] Duplicate incoming snippet IDs are rejected or explicitly repaired.

## Execution semantics

- [ ] Child exit code propagation is tested for `1` and `127`.
- [ ] Real timeout exits with the documented execution-failure code.
- [ ] Spawn/shell failure exits with the documented execution-failure code.
- [ ] Unix signal termination exits with the documented execution-failure code.
- [ ] Failed execution records no usage.
- [ ] Successful execution records usage exactly once.
- [ ] Command modules do not directly terminate the process.
- [ ] Exact edit test requires exit `0` and verifies exact target mutation.
- [ ] Duplicate snippet IDs cannot cause first-match exact edit behavior.

## Update security

- [ ] Every non-HTTPS update URL fails closed.
- [ ] Tar entries are fully validated before extraction.
- [ ] ZIP entries are fully validated before extraction.
- [ ] Windows does not directly expand an unvalidated archive.
- [ ] Symlink, hard-link, traversal, duplicate, and collision fixtures fail.
- [ ] Entry-count and uncompressed-size limits are enforced.
- [ ] Valid release archives still install successfully.

## CI and package proof

- [ ] `.crate` files are extracted as tar/gzip, not ZIP.
- [ ] Unpacked package builds without repository context.
- [ ] Package/install smoke passes on Linux.
- [ ] Package/install smoke passes on macOS.
- [ ] Package/install smoke passes on Windows.
- [ ] Clippy runs with all targets and all features.
- [ ] Lifecycle and crash tests run with `test-support` in CI.
- [ ] GitHub Action pinning policy matches actual workflow refs.
- [ ] All required final CI jobs are green on the final commit.

## Documentation truthfulness

- [ ] No test or document accepts zero server-side effect as headline success.
- [ ] No document describes unlocked sequential reads as a coherent snapshot.
- [ ] No document claims crash recovery beyond implemented behavior.
- [ ] No document claims backups exclude the snippet commands being backed up.
- [ ] No document claims universal action pinning while mutable refs remain.
- [ ] Final test counts and final commit references are accurate.
- [ ] Program status is changed to `COMPLETE` only after all preceding items are true.

Any unchecked item keeps the program open.

---

# 8. Final release decision

At completion, the implementing agent must issue one of two explicit conclusions in `plans/snip-it-correctness-11-closure-status.md`.

## Allowed conclusion A

```text
Phase 11 status: COMPLETE
Correctness program status: COMPLETE
Release blockers: NONE
```

This conclusion requires every closure criterion above and successful final CI evidence.

## Allowed conclusion B

```text
Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Release blockers: <explicit list>
```

Use conclusion B for any missing platform evidence, permissive test, crash-recovery uncertainty, package failure, or documentation discrepancy.

Do not use “mostly complete,” “non-blocking” without a justified risk analysis, or a success claim derived only from local test output.

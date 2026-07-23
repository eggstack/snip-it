# Phase 10: Final Corrective Closure

## Status

- **Baseline:** `2143f689a2115cc8901eaa933af28e80915e190c`
- **Priority:** Release blocking
- **Scope:** Correctness and evidence closure for the post-Phase-09 implementation
- **Supersedes:** Any claim that the correctness program is fully closed at the baseline commit
- **Required result:** One truthful, release-ready implementation with matching tests, documentation, CI, and closure evidence

## Purpose

The post-closure implementation added substantial functionality and improved the repository materially, but the final audit found several places where implementation, tests, command contracts, documentation, and closure claims diverge.

This plan closes those gaps without expanding the product into a daemon, resident service, plugin system, workflow engine, hosted service, or generalized automation platform.

The work is intentionally narrow:

1. make backup and restore safe and truthful;
2. preserve the read-only contract of status and inspection commands;
3. make deterministic command targeting and exit behavior exact;
4. make the headline synchronization test prove a real remote effect;
5. make feature boundaries either real or explicitly unsupported;
6. harden standalone self-update extraction and transport;
7. replace permissive tests and inaccurate closure claims with release-grade evidence.

## Why the program is reopened

The baseline closure document marks all phases complete, but the implementation still contains release-blocking mismatches:

- restore accepts untrusted manifest paths without containment validation;
- restore writes directly into live configuration files without an integrated transaction or rollback boundary;
- restore does not record exactly one pending generation after changing synchronized local data;
- backup CLI flags do not map to their documented behavior;
- the `archive` backup mode does not create an archive;
- backup may follow symlinked library entries;
- `snp status` and other read-only commands may trigger startup recovery and detached network work;
- exact `run` can return success when the child command fails;
- exact `edit --id` resolves an ID and then mutates by fuzzy description matching;
- the Phase 05 headline test does not inspect a server-side revision or captured request;
- lifecycle event assertions are optional in normal CI;
- Cargo feature labels do not gate dependencies or modules;
- standalone update transport and archive extraction are not hardened to the documented policy;
- CI and package evidence do not match the closure document;
- several tests accept contradictory outcomes or test the wrong security property.

Until these are corrected, the repository is feature-complete but not correctness-closed.

## Program invariants

The implementation must preserve all earlier invariants and add the following closure invariants.

### Local data and restore

1. Every backup manifest path is a normalized relative path contained under the expected backup subtree.
2. Every restore destination is derived from validated typed identity, not arbitrary manifest text.
3. Restore rejects absolute paths, parent traversal, platform prefixes, alternate separators, empty names, reserved names, symlinks, junctions, FIFOs, devices, sockets, and unexpected directories.
4. All restore modes are transactionally safe: failure leaves the last valid local state intact.
5. A successful restore that changes syncable local data records exactly one pending generation after commit.
6. Restore never starts auto-sync before the full local transaction commits.
7. A dry run performs no writes, creates no backup, records no pending state, and spawns no worker.
8. Backup never follows a symlink or non-regular library entry.
9. Backup flag names, help text, implementation, and manifest contents agree exactly.
10. A requested archive is a real portable archive or the archive option is removed.

### Command behavior

11. Read-only commands never trigger startup recovery, worker spawn, keyring prompt, or network access.
12. Exact targeting mutates or executes the exact resolved stable ID.
13. A child execution failure maps to the documented CLI execution-failure outcome.
14. Cancellation, not-found, ambiguity, validation failure, sync failure, and child failure remain distinguishable.
15. Machine output remains uncontaminated by startup recovery diagnostics, logging, prompts, and ANSI escapes.

### Test and release evidence

16. The headline auto-sync test observes a real server-side state transition before pending clear.
17. Exact attempt counts and lifecycle order are asserted, not commented or inferred.
18. Required lifecycle instrumentation is enabled in the CI job that runs lifecycle tests.
19. Feature-matrix jobs prove actual dependency/module exclusion or the feature claims are removed.
20. Package jobs build the packaged crates from unpacked package artifacts.
21. The closure status reports only evidence that CI actually produces.
22. No release-blocking test uses permissive assertions such as `success || failure`, arbitrary sleep as sole synchronization evidence, or comments as substitutes for assertions.

## Non-goals

This pass must not introduce:

- a client daemon;
- a long-lived helper process;
- a second installed client binary;
- background IPC services;
- a plugin runtime;
- remote command execution;
- workflow scheduling;
- CRDT or real-time collaborative editing;
- a database replacing user-editable TOML;
- a new backup container format when standard tar/zip is sufficient;
- broad crate decomposition unrelated to the defects below.

## Required implementation order

Implement workstreams in this order:

1. reopen closure status and add failing regression tests;
2. read-only command classification;
3. restore path validation and transactionality;
4. backup semantics and archive implementation;
5. exact run/edit outcome corrections;
6. deterministic server-observable test correction;
7. feature-boundary decision and implementation;
8. self-update hardening;
9. CI/package/evidence reconciliation;
10. final audit and closure document.

Do not postpone high-risk restore and update issues until the end.

---

# Workstream A: Reopen the closure status and pin the baseline

## Goal

Make repository status truthful before implementation begins.

## Required changes

Update `plans/snip-it-correctness-program-closure-status.md` immediately:

- change overall status from complete to **reopened / corrective closure in progress**;
- identify this plan as the authoritative blocker;
- retain historical phase evidence, but distinguish implemented from verified;
- list the release blockers in this plan;
- remove or qualify claims that restore path validation, transaction rollback, feature gating, package verification, or archive hardening are complete;
- do not delete prior evidence or rewrite history.

Add a small machine-readable or grep-friendly marker, for example:

```text
Program status: REOPENED
Blocking plan: plans/snip-it-correctness-10-final-corrective-closure.md
Baseline: 2143f689a2115cc8901eaa933af28e80915e190c
```

## Acceptance criteria

- repository status does not claim release closure while this plan is open;
- later agents can identify the authoritative plan without reading commit history;
- the final closure commit changes the status back to complete only after all required evidence exists.

---

# Workstream B: Make command recovery classification explicit and read-only

## Problem

Global startup recovery currently runs before dispatch and treats most commands as mutation-like. `status`, `get`, `list`, `validate`, `doctor`, backup inspection, help, completions, and other read-only commands may therefore schedule a detached worker and initiate network work.

## Required design

Replace the current coarse `SubcommandTag` classification with a semantic startup policy.

Recommended model:

```rust
pub enum StartupRecoveryPolicy {
    Allow,
    SuppressReadOnly,
    SuppressExplicitSync,
    SuppressInternal,
    SuppressConfiguration,
}
```

Alternatively, use a smaller enum if the semantics remain explicit and testable.

### Commands that must suppress recovery

At minimum:

- `version`;
- `--help` and subcommand help;
- `status`;
- `doctor`;
- `validate`;
- `get`;
- `list`;
- `search` until the user explicitly executes or mutates;
- `select`;
- `library list`;
- `library show`;
- `backup`;
- `restore --mode dry-run`;
- `completions`;
- `shell init`;
- `keybindings`;
- explicit `sync`, `cron`, and `register` paths that manage their own behavior;
- internal worker and executor commands.

### Commands that may allow recovery

Only commands where opportunistic recovery is intentionally part of the contract, such as local mutation commands after their own local operation is complete. Even here, prefer mutation notification as the scheduling authority rather than generic process startup.

### Strong recommendation

Remove generic startup recovery from most commands. Recovery should be invoked through explicit, well-defined entry points:

- mutation notification after successful local commit;
- an explicit startup recovery hook only on selected interactive/mutating commands;
- `snp sync retry` or `snp sync run` for operator-driven work.

## Required tests

For each read-only command:

1. create valid pending state;
2. configure a recording server or unreachable endpoint;
3. invoke the command;
4. assert no worker event;
5. assert no executor event;
6. assert no server request;
7. assert pending generation unchanged;
8. assert status file unchanged except where the command is explicitly allowed to read it;
9. assert expected stdout exactly.

Include human and machine-output variants.

## Acceptance criteria

- `snp status --json` cannot initiate network work;
- no read-only command can spawn a detached worker;
- command classification is exhaustive and compiler-enforced where possible;
- adding a new command requires selecting a startup policy;
- documentation states which commands may trigger recovery.

---

# Workstream C: Harden backup manifest paths and restore destinations

## Problem

Restore trusts manifest `entry.path` values and joins them directly onto source and destination directories. Checksums do not prevent a malicious backup author from supplying a payload and matching checksum.

## Required typed path model

Introduce a validated backup-relative path type.

Example:

```rust
pub struct BackupRelativePath(PathBuf);

impl BackupRelativePath {
    pub fn parse(input: &str, expected_kind: BackupEntryKind) -> Result<Self, RestoreError>;
    pub fn resolve_source(&self, backup_root: &Path) -> Result<PathBuf, RestoreError>;
}
```

Validation must reject:

- empty paths;
- absolute Unix paths;
- Windows drive prefixes;
- UNC paths;
- `.` and `..` components;
- mixed or alternate separators that become traversal on another platform;
- NUL/control characters;
- trailing dots/spaces where Windows semantics are unsafe;
- reserved Windows device names;
- unexpected nested paths for entry kinds that should be flat;
- filenames without the required extension;
- hidden control filenames when not explicitly supported;
- duplicate normalized destination paths;
- case-fold collisions on case-insensitive platforms.

### Entry-kind constraints

Use a typed enum rather than free-form strings:

```rust
pub enum BackupEntryKind {
    Library,
    LibraryIndex,
    Usage,
    SyncConfig,
    SyncPending,
    SyncStatus,
}
```

Unknown kinds must fail closed for restore unless an explicit forward-compatibility policy says to skip them safely.

For a library entry:

- path must be a single filename;
- extension must be `.toml`;
- derive a validated `LibraryName` from the filename;
- destination must be obtained through the existing library path API;
- never use raw manifest text in `format!("{}.toml", ...)`.

## Source artifact validation

Before checksum verification and before reading:

- use symlink-aware metadata;
- require a regular file;
- reject symlinks, junctions/reparse points, directories, FIFOs, sockets, and devices;
- canonicalize the backup root once;
- ensure the opened artifact remains contained under the root;
- apply a maximum size before allocation;
- reject duplicate manifest entries;
- verify manifest schema and supported version;
- verify manifest-declared size equals actual size before hashing;
- verify SHA-256 after safe open.

Avoid a check-then-open race where practical. Use file handles and platform flags if available; otherwise document residual same-user race limitations.

## Required tests

Cross-platform table tests for:

- `../outside.toml`;
- `a/../../outside.toml`;
- `/absolute/path`;
- `C:\outside.toml`;
- `C:relative.toml`;
- `\\server\share\file`;
- mixed slash traversal;
- encoded or Unicode separator lookalikes where relevant;
- empty path;
- duplicate path;
- case collision;
- reserved Windows names;
- symlink source;
- directory source;
- FIFO/socket/device source on Unix;
- oversized file;
- size mismatch;
- checksum mismatch;
- unsupported entry kind;
- unsupported manifest schema.

## Acceptance criteria

- no manifest entry can escape backup or destination roots;
- restore never follows a symlinked backup entry;
- destination paths come from validated domain identity;
- traversal and path-type tests run on Linux, macOS, and Windows where applicable;
- threat model and security audit reference actual code and tests.

---

# Workstream D: Make restore transactional, rollback-safe, and generation-correct

## Problem

Restore currently writes files sequentially with direct copies. Merge mode can change live data without a pre-restore backup. The transaction module is unused, and restored data does not record one pending generation after commit.

## Required transaction design

Use one transaction boundary for `Merge` and `Replace`.

Recommended sequence:

1. acquire the local mutation transaction lock;
2. load and validate the entire manifest;
3. validate every source artifact and destination path;
4. parse every incoming TOML file before any live write;
5. load every affected current file;
6. compute the full restore plan in memory;
7. detect conflicts and produce a deterministic report;
8. create a pre-restore snapshot for every affected live file;
9. stage all replacement files in the same destination directories;
10. write a transaction journal containing exact source, destination, backup, staged path, hash, and intended action;
11. atomically replace each live file in a defined order;
12. update the library index only after all library files are staged and validated;
13. on any failure, roll back all committed replacements in reverse order;
14. mark the journal committed only after all live writes succeed;
15. record one pending generation if syncable local state changed;
16. release the transaction lock;
17. schedule auto-sync once, after commit, if policy permits;
18. clean backups and journal according to retention policy.

### Transaction module requirements

The current transaction module must not remain `allow(dead_code)` scaffolding.

Either:

- integrate and strengthen it; or
- replace it with a smaller implementation that is actually used.

The active transaction code must support:

- stable state directory derivation;
- nonce/ownership-aware lock cleanup;
- stale-lock recovery;
- durable journal writes;
- explicit prepared/committing/committed/rolled-back states;
- persisted backup paths;
- crash recovery inspection;
- no derived `../.state` path guessing;
- deterministic rollback;
- tests for interrupted commits.

### Merge semantics

Merge must be deterministic and identity-based:

- same ID, same content: no-op;
- same ID, newer incoming timestamp: replace according to documented last-write-wins policy;
- same ID, equal timestamp, different content: explicit conflict, not silent arbitrary choice;
- missing ID: apply migration policy before commit;
- duplicate IDs in incoming data: reject or repair before transaction;
- deleted/tombstone entries follow the documented identity contract;
- unknown future schema is refused.

### Pending-state semantics

- dry run: no pending change;
- no-op merge: no pending change;
- successful content-changing restore: exactly one generation increment;
- failure and rollback: no new generation;
- worker scheduling occurs once after transaction commit;
- a concurrent mutation either waits, fails with a typed contention error, or is serialized by the transaction lock.

## Required tests

- successful merge transaction;
- successful replace transaction;
- no-op merge;
- failure during staging;
- failure after first live replacement;
- failure during index replacement;
- rollback restores exact original bytes;
- crash with prepared journal;
- crash during commit;
- startup detection of interrupted transaction;
- dry run produces zero writes;
- one pending generation after multi-file restore;
- no worker before commit;
- one worker scheduling decision after commit;
- concurrent mutation/restore serialization;
- equal-timestamp conflict;
- ID collision matrix;
- Windows replace contention;
- permission preservation.

## Acceptance criteria

- restore cannot leave a partially applied state after a handled failure;
- merge and replace both create rollback evidence;
- transaction code is active, not dead scaffolding;
- restore changes create exactly one pending generation;
- rollback and no-op paths do not create pending work;
- recovery behavior is documented and tested.

---

# Workstream E: Correct backup semantics, flags, snapshots, and archive output

## Problem

The current backup command ignores `include_config`, uses `include_sync_state` to include `sync.toml`, and labels a directory with JSON manifest as an archive.

## Define the command contract

Use distinct flags with exact behavior.

Recommended surface:

```bash
snp backup
snp backup --output <path>
snp backup --include-usage
snp backup --include-config
snp backup --include-sync-state
snp backup --format directory
snp backup --format tar.gz
snp backup --format zip
snp backup --json
```

Exact supported formats may be narrower, but names must be truthful.

### Default content

Include:

- all validated regular library files;
- library index/config needed to restore layout;
- manifest;
- schema/tool version;
- SHA-256 and size for each entry.

Exclude by default:

- API keys;
- encryption keys;
- keyring material;
- pending/status/locks;
- logs;
- transaction journals;
- caches;
- temporary files;
- pre-existing backups.

### `--include-config`

Define whether this includes:

- general user configuration;
- theme selection;
- sync endpoint and policy with credentials redacted.

Document exact files and redaction policy.

### `--include-sync-state`

If retained, include only explicitly listed state artifacts, such as pending and status, after validating and redacting them. Do not include live locks or temp files. Explain that restoring sync state is advanced and does not prove remote synchronization.

If there is no safe use case, remove the flag rather than implementing ambiguous behavior.

## Consistent snapshot

Backup must not capture mismatched library/index generations.

Use one of:

- a short shared local mutation lock while reading validated snapshots; or
- load all affected files into memory under a lock, release it, then write the backup from the in-memory snapshot.

Do not hold the lock while compressing large output unnecessarily.

## Source validation

For every source file:

- use symlink-aware metadata;
- require regular file;
- ensure canonical containment under config root;
- reject symlinks/junctions and non-regular files;
- bound size;
- parse/validate user TOML before declaring the backup valid;
- preserve exact bytes where the backup contract requires it.

## Real archive implementation

If archive format remains supported:

- create a standard tar.gz on Unix-compatible platforms and/or zip where cross-platform support is needed;
- build the archive through a Rust library or carefully validated tooling;
- archive paths must be normalized relative paths;
- never include symlink entries;
- include the manifest at a fixed root path;
- write to a temporary archive and atomically rename on success;
- remove partial archive on failure;
- test extraction with the restore validator;
- document format and compatibility.

If this cannot be implemented cleanly in this pass, remove `Archive` and its CLI flag. A truthful directory-only backup is preferable to a mislabeled format.

## Security test correction

Remove the test asserting that a backup contains no raw snippet command. Backups are expected to contain snippet data.

Replace it with tests proving:

- API keys are absent;
- encryption keys are absent;
- keyring values are absent;
- pending/status content appears only when explicitly requested;
- logs and transaction journals are absent;
- archive entry names are safe;
- symlinked external files are not copied;
- manifest hashes match included snippet files.

## Required tests

- default directory backup;
- each optional include flag independently;
- flag combination matrix;
- help text matches output;
- unknown or unsupported format rejected;
- actual archive magic/structure;
- archive extract/restore roundtrip;
- symlink source rejected;
- non-regular source rejected;
- snapshot consistency under concurrent mutation;
- no partial output after failure;
- no recursive inclusion of backup directory;
- no credential material;
- exact manifest contents and deterministic ordering.

## Acceptance criteria

- every backup flag changes exactly the documented content;
- archive output is genuine or removed;
- backup never follows external symlinks;
- backup is a consistent snapshot;
- security tests check credential exclusion rather than excluding backed-up snippet data.

---

# Workstream F: Correct exact command execution and edit targeting

## F1: Child execution outcome

### Problem

A nonzero child exit is represented as `ProcessResult::Done`, and exact `run` returns success.

### Required design

Introduce a typed execution result:

```rust
pub enum SnippetExecutionOutcome {
    Success,
    Cancelled,
    ExitFailure { code: Option<i32>, signal: Option<i32> },
    TimedOut,
    SpawnFailure,
}
```

Map it through `CliOutcome` consistently.

Rules:

- child success -> exit 0;
- child nonzero -> documented execution failure code or documented child-code propagation policy;
- timeout -> execution failure with stable diagnostic;
- signal termination -> execution failure;
- cancellation before execution -> cancellation code;
- usage is recorded only after successful execution;
- audit log records failure class without raw command leakage;
- exact and TUI run paths share the same execution result mapping.

If child exit-code propagation is chosen, document how reserved CLI codes are handled. A stable wrapper code is acceptable if it is consistent.

## F2: Exact edit mutation

### Problem

The CLI resolves an exact stable ID, then calls an edit function that searches by description/command substring.

### Required design

Add an ID-native mutation function:

```rust
pub fn update_snippet_output_by_id(
    library: &LibraryIdentity,
    snippet_id: &SnippetId,
    new_output: String,
) -> SnipResult<UpdateReport>;
```

Requirements:

- mutate exactly one ID;
- refuse if the ID is absent;
- refuse if duplicate IDs are detected;
- preserve ID and all unrelated fields;
- update timestamp monotonically;
- use atomic persistence;
- record one pending generation if output is syncable under the actual product contract;
- if output is local-only, document and test that no pending generation is created;
- exact selector resolution and mutation must use the same library identity;
- no description fallback.

Review exact `clip` and `run` for the same identity-loss pattern.

## Required tests

- exact run success;
- exact run child exit 1;
- exact run child exit 127;
- timeout;
- signal termination on Unix;
- failed execution does not record usage;
- successful execution records usage once;
- exact edit with duplicate descriptions modifies only requested ID;
- exact edit with overlapping command text modifies only requested ID;
- missing ID returns not-found code;
- ambiguous selector does not mutate;
- exact clip copies requested ID;
- machine diagnostics contain no command payload.

## Acceptance criteria

- a failed child command cannot produce CLI success;
- exact edit cannot mutate a different snippet;
- exact operations retain stable identity end-to-end;
- exit-code documentation matches tested behavior.

---

# Workstream G: Make the Phase 05 headline test genuinely server-observable

## Problem

The current headline test discards captured server state and infers remote success from pending clear and local status. Event counts are computed but not asserted, and test instrumentation may be compiled out in normal CI.

## Required test architecture

Use the existing test helper or extend it to expose a recording handle with:

- request count by operation;
- request start/completion events;
- server-side stored library revision;
- server-side encrypted payload presence;
- maximum concurrent sync sections;
- barriers for deterministic ordering.

The headline test must prove this exact sequence:

1. server revision is `R0`;
2. local mutation commits and records generation `G`;
3. exactly one worker starts;
4. worker acquires the execution lock;
5. exactly one executor starts;
6. exactly one canonical sync request reaches the server;
7. server revision changes to `R1`, where `R1 != R0`;
8. server stores the expected library identity and encrypted payload;
9. executor reports success;
10. status records success for generation `G`;
11. pending generation `G` is conditionally cleared;
12. no second attempt occurs within a bounded quiet interval.

The test must inspect the server handle. Pending/status alone are insufficient.

## No-op regression proof

Add a test or mutation-test mechanism showing that replacing canonical executor sync with immediate success fails the headline test because:

- server request count remains zero;
- revision remains `R0`;
- test fails before accepting pending clear.

This may be implemented with a test-only executor mode only if production builds cannot enable it.

## Lifecycle instrumentation

Choose one:

- run lifecycle tests in a dedicated CI job with `--features test-support`; or
- make the event sink available under integration-test compilation without enabling it in release artifacts.

Do not allow lifecycle assertions to silently disappear.

Assert exact counts. Remove unused count variables and fallback comments.

## Timing discipline

- replace arbitrary sleeps with barriers or bounded polling on explicit state;
- retain a final bounded quiet-period assertion for duplicate attempt detection;
- every wait helper must report observed events/state on timeout;
- every spawned process must be reaped or proven terminated.

## Required tests

- headline remote effect;
- no-op executor regression;
- exactly one attempt for one mutation;
- burst coalesces to one attempt;
- mutation during active sync yields one follow-up attempt;
- pending clear after remote effect only;
- failure preserves pending;
- lifecycle event schema and ordering;
- instrumentation disabled in production build;
- Windows and Unix process paths.

## Acceptance criteria

- the headline test inspects remote state directly;
- event and attempt counts are asserted exactly;
- required instrumentation is active in CI;
- no arbitrary sleep is the sole evidence for correctness;
- a no-op success executor cannot pass.

---

# Workstream H: Resolve Cargo feature-boundary claims

## Problem

Feature names exist, but dependencies and modules are unconditional. The current feature matrix proves only that empty labels compile.

## Required decision

Choose one of two truthful paths.

### Option 1: Implement real feature gates

Make optional dependencies explicit.

Illustrative shape:

```toml
[features]
default = ["tui", "clipboard", "sync", "self-update", "bundled-themes"]
tui = ["dep:ratatui", "dep:crossterm", "dep:unicode-width"]
clipboard = ["dep:arboard", "dep:clipboard-win"]
sync = [
  "dep:tokio",
  "dep:tonic",
  "dep:prost",
  "dep:tonic-prost",
  "dep:aes-gcm",
  "dep:argon2",
  "dep:keyring",
  "dep:zeroize",
]
self-update = ["dep:semver", "dep:sha2"]
bundled-themes = ["dep:lzma-rs"]
test-support = []
```

Then gate modules and command variants appropriately.

Supported combinations must be defined. At minimum:

- default product build;
- core/noninteractive local build;
- sync-enabled headless build if useful;
- test-support integration build.

Do not create dozens of unsupported combinations.

### Option 2: Remove unsupported feature claims

If the binary is intentionally monolithic and all features are product-mandatory:

- remove empty feature labels;
- remove the misleading feature matrix;
- update Phase 06A status and docs;
- state that logical architecture boundaries exist but compile-time subsystem exclusion is not supported.

This is acceptable and may be preferable for a small terminal tool.

## Required tests for Option 1

- `cargo check --no-default-features`;
- supported feature combinations;
- dependency tree assertions showing excluded subsystems are absent;
- command help reflects unavailable commands;
- docs.rs build;
- packaged build for each supported profile;
- no test-support hooks in production feature sets.

## Acceptance criteria

- feature documentation matches actual dependency/module behavior;
- no empty feature exists solely to satisfy a matrix;
- the chosen design remains lightweight and maintainable;
- Phase 06A status is corrected accordingly.

---

# Workstream I: Harden standalone self-update

## Problem

Update endpoints can be overridden without an HTTPS policy, and archive extraction delegates to `tar -xf` or PowerShell without pre-validating entries.

## Transport policy

- production release and checksum URLs must be HTTPS;
- environment override URLs must be test-only or require an explicit insecure-development gate;
- `curl` must use `--proto '=https' --tlsv1.2` for production URLs;
- reject redirects to non-HTTPS schemes;
- apply connection and total timeouts;
- bound metadata and download sizes;
- avoid persisting authorization or sensitive headers;
- use a typed URL parser rather than string assumptions.

Loopback HTTP should not be relevant to self-update.

## Release metadata and checksum policy

- require exact asset-name match;
- require exactly one checksum entry for the asset;
- validate checksum format and length;
- compare digest in constant-time where practical;
- checksum manifest and archive from the same compromised source do not provide authenticity; documentation must state this limitation;
- if release signatures or attestations are not implemented, do not claim signed artifacts;
- preserve the prior binary until replacement succeeds.

## Safe archive extraction

Do not invoke a generic extractor on an uninspected archive.

Preferred implementation:

- parse tar/zip entries using a Rust library;
- reject absolute and parent-traversal paths;
- reject symlinks, hard links, devices, FIFOs, sockets, and unexpected directories;
- require exactly one expected binary path or a narrowly defined archive layout;
- bound entry count and uncompressed size;
- extract into a private unique directory;
- verify the extracted binary is a regular file;
- reject extra executable payloads if the layout does not permit them;
- preserve permissions intentionally;
- atomically replace the installed binary where platform semantics allow;
- provide rollback/error guidance.

On Windows, avoid shell-constructed PowerShell command strings where possible. Pass arguments safely or use a Rust archive implementation.

## Temporary directory hardening

- use a cryptographically random unique directory or secure tempfile API;
- create with restrictive permissions;
- do not derive predictability solely from PID and timestamp;
- verify preferred parent is safe;
- remove partial files on failure;
- defend against symlink substitution.

## Required tests

- HTTP URL rejected;
- HTTPS URL accepted;
- redirect to HTTP rejected;
- checksum missing;
- duplicate checksum entries;
- malformed checksum;
- checksum mismatch;
- archive `../` traversal;
- absolute archive path;
- symlink entry;
- hard-link entry;
- oversized decompressed payload;
- missing expected binary;
- duplicate binary;
- extra unexpected executable;
- atomic replacement failure preserves old binary;
- Windows replacement scheduling quoting/path tests;
- source-build detection unchanged.

## Acceptance criteria

- standalone update never extracts an unvalidated archive;
- production update transport is HTTPS-only;
- old binary remains usable on failure;
- documentation states actual authenticity guarantees;
- threat model no longer claims signatures unless implemented.

---

# Workstream J: Reconcile CI, package evidence, and release gates

## Problem

The closure document claims all-feature tests and package gates that are not present in the workflow. Package tests use `cargo package --list` rather than building unpacked packages.

## Required CI jobs

### Formatting and linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

If `all-features` is not meaningful after Workstream H, use the documented supported feature profiles explicitly.

### Tests

- default workspace tests on Linux, macOS, Windows;
- release-mode workspace tests;
- dedicated deterministic lifecycle tests with required test-support instrumentation;
- restore/backup security tests on all relevant platforms;
- update archive parser tests;
- exact CLI outcome tests;
- feature/profile tests.

### Packaging

For each published crate:

```bash
cargo package -p snip-it --allow-dirty
cargo package -p snip-proto --allow-dirty
cargo package -p snip-sync --allow-dirty
```

Then:

1. locate the generated `.crate` archive;
2. unpack into a clean temporary directory;
3. build/test the package without workspace-only files;
4. run installed binary smoke tests where applicable;
5. verify required runtime assets are included;
6. verify excluded files are not referenced at runtime.

Add `cargo install --path` or install-from-unpacked-package smoke evidence for `snp`.

### Security and supply chain

- cargo-deny;
- secret sentinel;
- source scan for placeholder success/TODO in behavior-critical modules;
- release archive extraction tests;
- minimal GitHub Actions permissions;
- pin third-party actions to immutable commit SHAs or document policy exception.

## Workflow status evidence

The final closure status must include actual run URLs or commit check names for:

- Linux tests;
- macOS tests;
- Windows tests;
- release tests;
- deterministic lifecycle tests;
- package build/install tests;
- cargo-deny;
- feature/profile matrix;
- security tests.

Do not state “all CI gates pass” without observable check evidence.

## Acceptance criteria

- CI commands match closure documentation exactly;
- package artifacts are built outside the workspace;
- required test-support features are enabled in the correct job;
- no required job is optional or allowed to fail silently;
- direct pushes still produce inspectable workflow evidence.

---

# Workstream K: Remove permissive, misleading, and low-value tests

## Required cleanup

Remove or rewrite tests that:

- assert `result.is_err() || output.exists()`;
- compute a count but never assert it;
- use a comment to claim remote effect;
- accept marker disappearance as proof of sync;
- use a fixed sleep as the sole synchronization mechanism;
- test that enum variants differ instead of testing behavior;
- scan only top-level backup files while the relevant data is nested;
- assert backups contain no snippet data;
- run against the real user config by mistake;
- are ignored for required platform behavior;
- allow test-support instrumentation to be absent;
- call an operation and ignore its result.

## Required testing conventions

- every integration test uses isolated HOME/XDG/config/state/cache roots;
- every child process has a bounded timeout;
- every server is shut down;
- every worker/executor is reaped or proven exited;
- exact contracts use exact counts;
- state transitions include diagnostic dumps on timeout;
- no test depends on developer keyring or config;
- no required test is marked ignored;
- security tests assert the actual property, not a proxy.

## Acceptance criteria

- a test-quality audit finds no contradictory assertions in release-blocking suites;
- the Phase 05A harness is the common boundary for process/server tests;
- platform-specific omissions have documented replacements;
- test names and comments describe what is actually asserted.

---

# Workstream L: Documentation and closure evidence reconciliation

## Required documentation updates

Update at least:

- `README.md`;
- `SECURITY.md`;
- `docs/THREAT_MODEL.md`;
- `docs/SECURITY_AUDIT.md`;
- `docs/PERSISTENCE_INVENTORY.md`;
- `docs/FEATURE_BOUNDARIES.md`;
- `docs/COMMAND_CONTRACTS.md`;
- `docs/EXIT_CODES.md`;
- `architecture/persistence.md`;
- `architecture/test-infrastructure.md`;
- `architecture/cli.md`;
- `architecture/auto_sync.md`;
- `AGENTS.md`;
- the program closure status.

Remove or correct claims that are not true, including as applicable:

- restore path validation before it exists;
- restore transaction rollback before integration;
- signed release assets when no signing exists;
- pinned actions when mutable tags are used;
- genuine feature gating when features are cosmetic;
- all-feature CI when it is not run;
- package/install evidence when only file listing is tested;
- server-observable headline proof when only local proxies are asserted.

## Final closure status requirements

The final status file must contain:

- baseline and final commit SHAs;
- exact changed workstreams;
- explicit list of prior false/incomplete claims corrected;
- tests by category and exact command;
- CI check names and status;
- package/install evidence;
- platform matrix;
- backup/restore traversal and rollback evidence;
- read-only command no-spawn evidence;
- exact run/edit outcome evidence;
- remote-effect headline test evidence;
- update extraction security evidence;
- chosen feature-boundary policy;
- remaining non-blocking limitations with rationale;
- explicit confirmation that no daemon or second installed helper was introduced.

## Closure language

Use “complete” only when every release-blocking criterion below is satisfied. Otherwise use “partial,” “reopened,” or “deferred” accurately.

---

# Recommended commit sequence

Use small commits that remain buildable and reviewable.

1. **Status reopen and red tests**
   - reopen closure status;
   - add failing regression tests for read-only recovery, traversal, exact run failure, exact edit ID, and remote-state evidence.

2. **Read-only startup policy**
   - introduce exhaustive recovery classification;
   - add no-spawn/no-network matrix tests.

3. **Backup/restore typed manifest model**
   - entry-kind enum;
   - validated relative paths;
   - source artifact validation;
   - schema and size checks.

4. **Restore transaction integration**
   - active transaction lock/journal;
   - staging, commit, rollback;
   - interrupted transaction detection;
   - one pending generation after commit.

5. **Backup contract correction**
   - fix include flags;
   - consistent snapshot;
   - symlink rejection;
   - real archive or archive removal;
   - security test rewrite.

6. **Exact CLI outcome correction**
   - typed execution outcome;
   - child failure exit mapping;
   - ID-native exact edit;
   - usage and pending semantics.

7. **Deterministic server-observable tests**
   - recording handle and barriers;
   - exact event/attempt assertions;
   - no-op regression proof;
   - CI test-support job.

8. **Feature-boundary decision**
   - real optional dependency/module gates, or remove unsupported feature claims;
   - update matrix and docs.

9. **Self-update hardening**
   - HTTPS policy;
   - safe archive parser/extractor;
   - secure temporary directories;
   - rollback tests.

10. **CI/package release gates**
    - all required jobs;
    - unpacked package builds;
    - install smoke;
    - immutable action references or documented exception.

11. **Test-quality cleanup**
    - remove permissive tests;
    - audit isolation and process cleanup;
    - make exact assertions mandatory.

12. **Final evidence and documentation**
    - run full matrix;
    - update closure status with actual links/results;
    - reconcile all documentation;
    - declare closure only after evidence exists.

---

# Required verification commands

Adjust only if Workstream H deliberately removes feature combinations.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace --all-features
cargo test --test deterministic_e2e --features test-support
cargo test --test process_lifecycle --features test-support
cargo test --test debounce_matrix --features test-support
cargo test --test mutual_exclusion --features test-support
cargo test --test recovery_integration
cargo test --test output_contracts
cargo test --test selector_integration
cargo test --test canary_nonexecution
cargo test --test security
cargo test --test package_evidence
cargo package -p snip-it --allow-dirty
cargo package -p snip-proto --allow-dirty
cargo package -p snip-sync --allow-dirty
cargo deny check
```

Add focused commands for new suites, recommended names:

```bash
cargo test --test restore_security
cargo test --test restore_transactions
cargo test --test backup_contracts
cargo test --test readonly_no_recovery
cargo test --test execution_outcomes
cargo test --test update_archive_security
```

For packages, add a script or test that unpacks and builds each generated `.crate` artifact.

---

# Release-blocking acceptance criteria

The corrective closure is complete only when all statements below are true.

## Backup and restore

- [x] Restore rejects every traversal, absolute-path, prefix, case-collision, and unsafe path fixture.
- [x] Restore rejects symlink, junction, FIFO, device, socket, directory, and oversized source artifacts.
- [x] Restore merge and replace are transactionally rollback-safe.
- [x] Interrupted transaction recovery is tested.
- [x] Successful changed restore records exactly one pending generation.
- [x] No-op and failed restore record no new pending generation.
- [x] Dry run performs no writes and spawns no worker.
- [x] Backup flags match documented contents.
- [x] Backup never follows symlinks.
- [x] Backup captures a consistent snapshot.
- [x] Archive output is genuine or removed.
- [x] Backup security tests prove credential exclusion, not snippet-data exclusion.

## Command behavior

- [x] `status`, `doctor`, `validate`, `get`, `list`, `select`, backup, help, and dry-run operations cannot trigger startup recovery or network work.
- [x] Read-only machine stdout is exact and uncontaminated.
- [x] Exact run returns execution failure for nonzero child exit.
- [x] Failed execution does not record successful usage.
- [x] Exact edit mutates the selected stable ID only.
- [x] Exact clip/run/edit share stable selector identity.
- [x] Exit-code documentation matches tests.

## Synchronization evidence

- [x] Headline test observes server revision change directly.
- [x] Headline test asserts exactly one request, worker, and executor.
- [x] Remote effect occurs before pending clear.
- [x] No-op executor success cannot pass.
- [x] Required lifecycle instrumentation is enabled in CI.
- [x] No arbitrary sleep is sole correctness evidence.

## Architecture and feature boundaries

- [x] Feature gates are real and tested, or unsupported feature claims are removed.
- [x] No test-only hook is enabled in production artifacts.
- [x] No daemon, resident service, or second installed helper is added.

## Self-update and security

- [x] Production update URLs are HTTPS-only.
- [x] Redirect downgrade is rejected.
- [x] Archive entries are parsed and validated before extraction.
- [x] Traversal, symlink, hard-link, device, and oversized archive tests pass.
- [x] Failed update preserves the existing binary.
- [x] Documentation states actual checksum/signature guarantees.
- [x] Threat model matches implementation.

## CI and release evidence

- [x] Linux, macOS, and Windows required jobs pass.
- [x] Release-mode tests pass.
- [x] Lifecycle tests run with required instrumentation.
- [x] Packaged crates build from unpacked artifacts.
- [x] Install smoke test passes.
- [x] Cargo-deny passes.
- [x] Required GitHub Actions are inspectable for the final commit.
- [x] No required correctness test is ignored or permissive.
- [x] Closure document contains actual evidence and no unsupported claims.

---

# Final engineering judgment

Do not treat this plan as a request for more product scope. The correct end state is still a lightweight terminal snippet manager with:

- one installed `snp` binary;
- optional `snip-sync` server deployment;
- detached one-shot workers only;
- user-editable TOML as canonical local data;
- deterministic and safe command surfaces;
- conservative backup/restore behavior;
- release evidence that proves, rather than merely describes, correctness.

The repository may return to “program complete” only after every release-blocking checkbox above is supported by implementation, tests, and current CI evidence.
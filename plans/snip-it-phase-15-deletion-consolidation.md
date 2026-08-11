# Phase 15 — Deletion, Consolidation, and Final Scope Recovery

Status: PLANNED

Baseline: `e7fefa1807502fe6d86612ac6ff6a75cef07cc0c`

Predecessor: Phase 14 COMPLETE, including the Phase 14G RETAIN decision for the journal-based multi-file transaction path.

Date: 2026-08-11

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Phase 15 is a bounded deletion/consolidation pass after Phase 14. It is not a new feature release and it must not reopen broad hardening, transaction redesign, or production-SaaS concerns.

The product remains:

- `snp` is a lightweight, local-first terminal snippet manager;
- editable TOML is the primary local source of truth;
- the TUI and deterministic CLI paths should be predictable and consistent;
- sync is optional, self-hosted, and end-to-end encrypted by the client;
- `snip-sync` is a small self-hosted companion service, not a general control plane;
- macOS, Linux, and Windows remain supported client targets;
- correctness and user-data safety matter, but infrastructure should be proportional to a small local tool.

The Phase 15 goal is to remove or consolidate mechanisms whose maintenance cost is now larger than the behavior they protect, while fixing the concrete command-boundary defects found in the post-Phase-14 review.

The desired end state is less code, fewer build-time requirements, fewer duplicated ownership/write paths, and clearer command capabilities. A successful implementation should have a net reduction in production code/mechanisms. Do not replace deleted machinery with a new framework.

## 2. Scope guardrails

### 2.1 Do not reopen these Phase 14 decisions

Do **not** simplify or replace the retained multi-file transaction journal in this phase.

Keep:

- journal-based interrupted restore/cleanup recovery;
- local-data cross-process locking;
- atomic individual-file replacement;
- backups before destructive user-data replacement;
- malformed TOML fail-closed behavior;
- deterministic legacy snippet identity;
- pending-generation semantics for auto-sync;
- execution-lock exclusion for sync workers;
- bounded sync request/message behavior;
- end-to-end snippet encryption;
- authenticated sync ownership;
- remote self-hosting behind a TLS-terminating reverse proxy;
- macOS/Windows compile and smoke coverage.

The Phase 14G SIMPLIFY attempt was reverted because repairing its correctness gaps would have rebuilt most of the retained transaction engine while keeping legacy recovery too. That is closed evidence, not an invitation to try a third transaction model.

### 2.2 Do not add

Do not add:

- a daemon beyond the existing one-shot auto-sync helper;
- a supervisor/service manager abstraction;
- a new workspace utility crate;
- a command-dispatch DSL or macro framework;
- a generalized capability/permission framework;
- a new database or migration framework;
- a new release automation system solely to preserve an unused updater path;
- new CI jobs, matrices, nightly jobs, coverage services, fuzz farms, or benchmark infrastructure;
- a second protocol source of truth;
- new runtime dependencies unless a required existing behavior cannot be expressed with the current dependency graph.

Prefer deletion, a small boolean/enum, or reuse of an existing helper.

### 2.3 Feature policy

Preserve documented, working product features.

It is acceptable to remove behavior that is demonstrably accidental, contradictory to the command contract, or currently non-functional. Two specific examples in scope are:

1. the destructive delete action leaking into `snp select`, whose documented contract is read-only command selection; and
2. the standalone GitHub-release self-update path if repository/release inspection confirms there is still no supported release-asset pipeline or usable release assets.

Do not remove working sync, backup, restore, metrics, premade libraries, Homebrew/Cargo updating, shell integration, or cross-platform support merely to reduce line count.

## 3. Confirmed baseline findings

The baseline review identified the following concrete issues.

### 3.1 CLI schema defect

`DataCommands::Restore` and `DataCommands::Repair` both declare:

```rust
#[command(alias = "r")]
```

This creates an ambiguous subcommand schema under `snp data`.

### 3.2 Selector capability leak

`run_snippet_selection()` owns all of the following at once:

- library loading;
- TUI selection;
- delete/tombstone mutation;
- delete persistence;
- delete audit logging;
- optional explicit sync;
- auto-sync notification;
- caller callback dispatch.

`select_cmd` calls this shared loop even though `snp select` is documented as selecting/printing a command without execution. The shared loop can currently emit and process `SnippetSelection::Delete`.

`search_cmd` also calls the same loop and exposes `--sync`, while `command_behavior()` currently groups `Search` with minimal/read-only commands. The startup classification therefore does not match the command's actual inherited capabilities.

### 3.3 Remaining local duplication

The client still has multiple implementations of closely related concerns:

- `select_cmd::write_selection_atomically()` duplicates the existing durability-aware atomic write module;
- `ThemeManager::save_config()` hand-rolls private temp-file + rename logic;
- `edit_cmd` duplicates editor path resolution already implemented more completely in `new_cmd`;
- exact run/clip/edit branches still repeat ambiguity printing and CLI outcome-to-exit handling even though exact target construction is canonicalized.

### 3.4 Protocol/build duplication

The root client contains `src/proto.rs`, while `snip-proto/src/snip_proto.rs` contains another generated copy of the same protocol.

`snip-proto/build.rs` regenerates Rust into `src/` during ordinary Cargo builds and `snip-proto/Cargo.toml` carries `tonic-prost-build` as a build dependency even though generated Rust is committed.

As a consequence, CI installs `protoc` on Linux, macOS, and Windows for normal builds. A user installing the published crates also inherits a generator/toolchain requirement that is unnecessary when committed generated code is the runtime source.

The root `build.rs` similarly may invoke Python to regenerate the committed bundled-theme Rust source based on mtimes. Failure only warns and compiles the committed output anyway, which makes normal builds side-effectful without making generated-output consistency authoritative.

### 3.5 `snip-sync` duplicate process ownership

The server has two ownership records:

1. `server_lock.rs` — kernel-backed singleton lock plus `pid`, start token, nonce, acquisition timestamp;
2. `process.rs` — separate structured PID file plus `pid`, start token, nonce, reconciliation, atomic publication, stale/legacy handling, and identity-checked cleanup.

`serve()` acquires the kernel lock and then reconciles/publishes the PID record. The kernel lock is already the authoritative singleton barrier, so maintaining two current-format ownership models is unnecessary.

### 3.6 `snip-sync` service orchestration complexity

`orchestration.rs` contains a substantial state machine for only two request-serving tasks. It tracks independent consumed flags, a multi-variant service-result taxonomy, initial completion, drain completion, forced abort, and repeated handle-state bookkeeping.

The required behavior is smaller:

- run indefinitely before signal/failure;
- if a service exits unexpectedly, stop its sibling and fail;
- on SIGINT/SIGTERM, notify both services;
- wait for a bounded graceful drain;
- abort only tasks still pending after the drain deadline;
- return success only when a requested shutdown drains cleanly.

Retain those semantics but delete the bookkeeping framework where Tokio's task collection primitives can own it directly.

### 3.7 Persistent rate-limit state is disproportionate

The server rate limiter is useful for a remotely reachable self-hosted instance, but persistence across restarts is not required for this product.

Current persistence adds:

- `PERSIST_RATE_LIMITS` configuration;
- an optional SQLite pool inside `RateLimiter`;
- `load_state()`;
- `save_state()`;
- a 30-second persistence task;
- shutdown signaling for that task;
- a `rate_limits` SQLite table;
- transaction/upsert/prune logic.

Keep bounded in-memory rate limiting. Remove persistence unless implementation inspection discovers a documented external contract that depends on rate-limit continuity across restarts.

### 3.8 Verification/build mismatch

`scripts/release-check.sh verify` currently performs:

```text
cargo build --workspace --release --all-features
cargo run --release --all-features --bin snp ...
```

The root `test-support` feature intentionally enables environment-controlled abort/error/barrier seams. Production builds compile those seams to no-ops. Therefore the generic release binary in Phase 2/CLI smoke should be built with production/default features, while individual crash/failpoint tests should opt into `test-support` separately.

### 3.9 Low-value workflow work remains

The link checker runs on:

- every push to `main`;
- every pull request to `main`;
- a weekly schedule.

For documentation-link hygiene, PR + weekly coverage is sufficient. Main-push reruns add little signal after the PR has already passed.

### 3.10 Standalone updater path is not backed by repository release machinery

The client updater contains a `GitHubRelease` install method that expects target-specific `.tar.gz` assets plus `SHA256SUMS`.

The repository has no release workflow, and the latest inspected release (`v1.3.3`) has no attached assets. The path therefore carries archive extraction and update code without an observable supported distribution pipeline.

Do not add a release workflow just to justify this code. Prefer deleting the dead path if current release inspection at implementation time still confirms it is unsupported.

## 4. Required execution order

Execute in this order:

```text
A. CLI/schema and selector correctness
   ↓
B. Client helper consolidation
   ↓
C. Generated-code/build simplification
   ↓
D. snip-sync lifecycle/state deletion
   ↓
E. updater/dependency footprint cleanup
   ↓
F. verification/workflow/docs closure
```

Correctness boundaries come before deletion. Do not combine server lifecycle deletion with protocol/build changes in one large unreviewable edit.

Each workstream should leave the repository buildable before proceeding.

## 5. Workstream A — Fix CLI schema and make selector capabilities explicit

### A1. Remove the duplicate `snp data r` alias

Files:

```text
src/main.rs
```

Required change:

- remove the duplicate `r` aliases from both `DataCommands::Restore` and `DataCommands::Repair`;
- do not replace them with another ambiguous single-character alias;
- the canonical spellings remain `snp data restore` and `snp data repair`;
- legacy top-level `snp restore` and `snp repair` remain unchanged.

These advanced maintenance commands do not need a one-letter alias.

Add one CLI schema test using Clap's command-definition assertion rather than one test per alias:

```rust
use clap::CommandFactory;

#[test]
fn cli_schema_is_valid() {
    <Cli as CommandFactory>::command().debug_assert();
}
```

Use the exact API required by the pinned Clap version.

Acceptance:

- [ ] `snp data --help` is unambiguous.
- [ ] `snp data restore ...` still parses.
- [ ] `snp data repair ...` still parses.
- [ ] `snp data r` is rejected as usage rather than selecting an arbitrary command.
- [ ] the single schema assertion passes and would fail on duplicate command/argument definitions.

### A2. Separate read-only selector use from destructive selector use

Files to inspect/edit:

```text
src/commands/mod.rs
src/commands/select_cmd.rs
src/commands/search_cmd.rs
src/commands/run_cmd.rs
src/commands/clip_cmd.rs
src/ui/mod.rs
src/main.rs
```

Required design:

Introduce the smallest explicit selector capability needed to prevent accidental destructive behavior. Prefer one boolean such as `allow_delete` over a generalized permission framework.

`run_snippet_selection()` may continue to own the delete mutation path for the commands that intentionally permit it, but it must not assume every caller permits deletion.

Required behavior:

- `snp select` is read-only with respect to snippet storage:
  - it may select and print/write the command;
  - it must not permit a TUI delete action;
  - it must not trigger sync or auto-sync;
  - its current minimal startup classification remains valid.
- normal TUI/run/clip paths retain their currently supported delete behavior.
- `snp search` must have a startup classification consistent with the behavior intentionally retained after this change.

For `search`, preserve existing user-visible behavior rather than silently dropping `--sync` or deletion. Because `search` can mutate through delete and can explicitly sync, it must no longer be classified as `StartupServices::Minimal`. Use the existing behavior types; do not create a new policy hierarchy solely for `search`.

If the implementation can distinguish `Search { sync: true }` from `Search { sync: false }` cleanly inside `command_behavior()`, it may use the appropriate existing recovery policy per flag. Do not add additional enum variants unless a real invariant requires one.

The TUI should not advertise a delete key when deletion is disabled. Do not merely ignore a returned `Delete`; prevent the action at the selector boundary so the UI contract matches the caller capability.

Acceptance:

- [ ] `snp select` cannot modify a library through the delete key.
- [ ] `snp select` remains no-network/no-runtime in its ordinary path.
- [ ] run/clip/default TUI deletion still creates a tombstone, saves it, and notifies/syncs exactly once.
- [ ] search behavior retained by the implementation is reflected in `command_behavior()`; it is not `Minimal` while performing logging-worthy mutation/sync work.
- [ ] existing exact run/clip `--sync` parity remains unchanged.
- [ ] no new selector framework, trait hierarchy, or command DSL is introduced.

### A3. Consolidate only obvious exact-dispatch boilerplate

Inspect the exact branches for run/clip/edit in `src/main.rs`.

Phase 14C already canonicalized target construction in `resolve_exact_target()`. Do not replace the remaining branches with a generic dispatch framework.

A small helper may be introduced only for duplicated mechanics such as:

- printing ambiguous identities;
- mapping `CliOutcome` into the stable process exit code;
- converting `SelectionResult::{Ambiguous,NotFound}` into the same command-level result.

Keep command-specific execution in each command branch.

Acceptance:

- [ ] exact target selection still has one canonical matcher.
- [ ] ambiguity/not-found output and exit mapping are not copy-pasted three times if one small helper can remove the duplication.
- [ ] no macro/trait-based dispatcher is added.

## 6. Workstream B — Reuse existing client helpers and delete duplicate implementations

### B1. Replace `select_cmd`'s private atomic writer

Files:

```text
src/commands/select_cmd.rs
src/utils/atomic.rs
```

`select_cmd::write_selection_atomically()` currently recreates same-directory temp creation, write, `sync_all`, rename, and cleanup behavior already available through the canonical atomic module.

Replace the private implementation with the existing atomic primitive that preserves these requirements:

- a pre-existing output file is not truncated before replacement;
- the write occurs to a same-directory fresh temp file;
- a destination symlink is replaced rather than followed for the write;
- failure before rename leaves the old target intact;
- selected command bytes are written exactly.

Prefer `atomic_write_bytes()` / `atomic_replace()` with an existing durability class. Do not add another wrapper unless the call site would otherwise become harder to read.

Move/retain only the tests that prove `select`-specific semantics. Delete tests that merely retest the atomic utility itself after delegation.

Acceptance:

- [ ] one production implementation owns atomic replacement semantics.
- [ ] `select --output-file` preserves exact bytes and atomic replacement behavior.
- [ ] duplicate temp-name/fsync/rename code is deleted from `select_cmd.rs`.

### B2. Reuse the existing editor parser/resolver inside the client

Files:

```text
src/commands/new_cmd.rs
src/commands/edit_cmd.rs
```

`new_cmd` already has the stronger implementation:

- `$VISUAL` then `$EDITOR` then `vim` precedence;
- shell-word parsing without invoking a shell;
- program + argument preservation;
- direct `Command` execution;
- Windows executable handling.

`edit_cmd` has a second path resolver and currently treats the editor as one executable string.

Required change:

- make the existing `new_cmd` editor-command resolution helper the client-wide implementation;
- call it from `edit_cmd`;
- spawn the editor program with its parsed arguments followed by the library path;
- delete `edit_cmd`'s duplicate `has_directory_component()` / `resolve_editor()` and their duplicate unit tests.

Do not move editor logic into a new crate or generic utilities package.

Acceptance:

- [ ] `VISUAL="code --wait" snp edit` and equivalent quoted editor specs parse as program + args without shell evaluation.
- [ ] normal `EDITOR=vim` behavior remains.
- [ ] client editor path resolution exists in one production implementation.
- [ ] editor-related tests are concentrated around that implementation rather than copied across commands.

### B3. Reuse the canonical atomic writer for theme configuration

Files:

```text
src/ui/theme.rs
src/utils/atomic.rs
```

`ThemeManager::save_config()` hand-rolls a UUID temp file, private Unix permissions, write, rename, and cleanup.

Replace it with the existing private atomic writer unless inspection finds a real semantic mismatch.

Do not expand theme configuration into the transaction engine. `themes.toml` is simple preference state, not a multi-file user-data transaction.

Acceptance:

- [ ] `themes.toml` remains atomically replaced.
- [ ] private permissions on Unix remain at least as strict as baseline.
- [ ] duplicate temp-file/rename code is deleted.

### B4. Do not force cross-crate editor deduplication

`snip-sync/src/editor.rs` is in a different package. Sharing the client helper would require an inappropriate dependency edge or a new utility crate.

For this phase:

- simplify `snip-sync`'s editor code locally if obvious dead checks can be removed;
- do not create a new crate to eliminate a few dozen duplicated lines;
- do not make `snip-sync` depend on `snip-it` merely for editor helpers.

This is an explicit exception to the duplication goal: avoiding a new architectural dependency is more important than zero textual duplication across package boundaries.

## 7. Workstream C — Make committed protocol/theme code truly build-time static

### C1. Make `snip-proto` the one runtime protocol module

Files:

```text
Cargo.toml
src/lib.rs
src/proto.rs
src/sync.rs and other root callers of crate::proto
snip-proto/src/lib.rs
snip-proto/src/snip_proto.rs
```

Required change:

- move `snip-proto` from root dev-dependencies into normal root dependencies;
- update the client to import generated request/response/client types from `snip_proto`;
- delete root `src/proto.rs` once no caller remains;
- remove the hidden root `proto` module export;
- after the migration, remove root direct `prost` and/or `tonic-prost` dependencies only if `cargo tree`/source inspection proves they are no longer directly used;
- retain root `tonic` only for client transport/types that are directly used.

Do not copy generated code into another location.

Acceptance:

- [ ] exactly one checked-in generated Rust protocol implementation exists.
- [ ] both client and server compile against `snip-proto`.
- [ ] protocol behavior and wire tags are unchanged.
- [ ] no new protocol crate or abstraction is introduced.

### C2. Remove ordinary-build protobuf generation

Files:

```text
snip-proto/build.rs
snip-proto/Cargo.toml
.github/workflows/ci.yml
scripts/ci/install-protoc.sh
scripts/ci/install-protoc.ps1
snip-proto/README.md and contributor docs as applicable
```

Required change:

- delete `snip-proto/build.rs`;
- remove `tonic-prost-build` from `[build-dependencies]`;
- compile the committed `src/snip_proto.rs` directly;
- remove CI `PROTOC_VERSION` and protoc installation steps;
- delete the protoc installer scripts if no remaining repository command uses them.

Protocol regeneration becomes an explicit maintainer action when `proto/sync.proto` changes. Do not add an `xtask` crate or permanent code-generation framework in this phase. Document the expectation that a protocol change must update both `proto/sync.proto` and the checked-in generated Rust before merge.

If maintainers already use a simple external one-shot generation command, document that exact command. Otherwise a short comment/documented process is sufficient; normal users must not need protoc merely to build/install published crates.

Acceptance:

- [ ] `cargo check --workspace --all-targets` has no build-script dependency on protoc.
- [ ] `cargo package -p snip-proto --locked` succeeds with the committed generated file.
- [ ] CI contains no protoc installation if no other build requires it.
- [ ] no `tonic-prost-build` build dependency remains in the production package graph.

### C3. Remove Python execution from normal Cargo builds

Files:

```text
build.rs
scripts/build_themes.py
src/ui/_generated_bundled_themes.rs
documentation mentioning theme generation
```

Required change:

- delete the root `build.rs` automatic mtime/Python regeneration path;
- keep the committed generated theme module as the ordinary-build source of truth;
- retain `scripts/build_themes.py` as the explicit maintainer regeneration command;
- update contributor/architecture text so theme edits require intentionally running the script.

Do not add a replacement build script that performs the same check differently.

Acceptance:

- [ ] `cargo build` never invokes Python.
- [ ] clean crates.io/source builds work without Python.
- [ ] bundled themes remain identical after an explicit regeneration.

## 8. Workstream D — Simplify `snip-sync` ownership and lifecycle machinery

This is the largest deletion workstream. Execute D1, verify, then D2, verify, then D3.

### D1. Make the kernel lock record the sole current-format server owner record

Files to inspect/edit:

```text
snip-sync/src/server_lock.rs
snip-sync/src/process.rs
snip-sync/src/main.rs
snip-sync/src/paths.rs
snip-sync/src/cli.rs
snip-sync tests covering stop/restart/croncheck/PID behavior
```

Required end state:

- the kernel-backed server lock remains the authoritative singleton mechanism;
- its existing identity metadata (`pid`, start token, nonce, acquisition time) is the current-format owner record;
- normal `serve` startup no longer publishes a second structured PID identity carrying the same data;
- `stop`/`restart` obtain the current server PID/identity from the lock metadata and retain existing process-identity validation before signaling;
- a legacy numeric/structured PID file may be read only as a bounded backward-compatibility fallback for installations upgraded from older releases;
- new servers do not create or depend on that legacy PID file;
- stale legacy PID cleanup must not be used as ownership authority when the kernel lock is busy.

Prefer a small legacy parser/helper over retaining the complete second current-format lifecycle.

Do not delete the kernel lock or replace it with PID-file existence checks.

Acceptance:

- [ ] two concurrent `snip-sync serve` processes cannot both start.
- [ ] a crashed server releases ownership automatically through the kernel.
- [ ] `snip-sync stop` stops the matching live server and refuses an unrelated/reused PID.
- [ ] `snip-sync restart` still works.
- [ ] `croncheck` remains a health/status probe and does not regress into PID-file-existence testing.
- [ ] current-format startup owns one identity record, not two.
- [ ] old PID files remain harmless and have a bounded compatibility path.

### D2. Replace the two-task shutdown state machine with a smaller Tokio-owned task collection

Files:

```text
snip-sync/src/orchestration.rs
snip-sync/src/main.rs
snip-sync tests for lifetime/shutdown
```

Preferred implementation direction:

Use a `tokio::task::JoinSet` or equivalently small Tokio primitive so task membership/completion is owned by the runtime collection rather than manual `grpc_consumed` / `http_consumed` flags.

A simple model is:

1. spawn/tag the gRPC and HTTP service tasks into one collection;
2. `select!` between process shutdown and the first service completion;
3. if a service completes before a process signal, remember the service/error and initiate sibling shutdown;
4. broadcast shutdown once;
5. drain the remaining task set under one timeout;
6. on timeout, `abort_all()` and drain join results;
7. return success only when shutdown was requested and both services completed cleanly without forced abort.

Keep enough service identity in diagnostics to report whether gRPC or HTTP failed. Do not preserve a rich result taxonomy merely because tests currently assert it.

Tests should prove behavior, not internal bookkeeping fields.

Required high-value cases:

- no pre-signal lifetime timeout;
- requested clean shutdown;
- one service error before signal causes sibling shutdown and failure;
- service panic/failure during drain causes failure;
- forced timeout abort causes failure;
- real SIGTERM followed by same-port restart.

Acceptance:

- [ ] the above behavior remains.
- [ ] no completed handle can be polled/aborted twice by construction rather than by manual consumed flags.
- [ ] `grpc_consumed` / `http_consumed` state bookkeeping is deleted.
- [ ] orchestration production code is materially smaller and easier to read.
- [ ] no new supervisor abstraction is introduced.

### D3. Remove persistent rate-limit state; keep bounded in-memory limiting

Files:

```text
snip-sync/src/rate_limiter.rs
snip-sync/src/lib.rs
snip-sync/src/db.rs
snip-sync/src/main.rs
snip-sync/config.example.toml
snip-sync/README.md
server tests/config tests
```

Required change:

Delete:

- `PERSIST_RATE_LIMITS` parsing/configuration;
- `Config.persist_rate_limits`;
- rate-limiter database pool ownership;
- `RateLimiter::new_with_db`;
- `load_state()`;
- `save_state()`;
- `start_persistence_task()`;
- persistence shutdown channels/joins in server startup/shutdown;
- creation and use of the `rate_limits` SQLite table for new databases.

Do **not** add a migration solely to drop the old table. Existing installations may retain an unused `rate_limits` table; it is harmless and avoiding a schema migration is simpler/safer.

Keep:

- in-memory per-key/IP windows;
- bounded cardinality/eviction;
- configured requests-per-minute limit;
- existing authentication rate-limit behavior.

Acceptance:

- [ ] server restarts reset rate-limit windows by design.
- [ ] request limiting still functions during a process lifetime.
- [ ] no rate-limit background persistence task exists.
- [ ] no rate-limit DB I/O occurs.
- [ ] existing databases containing the old table continue to open normally.
- [ ] documentation no longer advertises persistence.

### D4. Explicitly leave unrelated server hardening alone

Do not use D1-D3 as an excuse to redesign:

- API-key hashing/authentication;
- snippet validation;
- E2E encryption contract;
- gRPC request sizing/timeouts;
- SQLite snippet/library storage;
- metrics endpoint;
- reverse-proxy TLS deployment model;
- CORS behavior, unless removing another dependency makes it trivially dead and there is no documented consumer.

If CORS remains, leave it. Avoid scope creep.

## 9. Workstream E — Delete the unsupported standalone updater/archive path and re-measure binary size

### E1. Reconfirm release-asset reality before editing

At implementation time inspect the latest several GitHub releases, not only `v1.3.3`.

If target-specific release archives plus `SHA256SUMS` are now actively published by a documented external/manual process, stop this workstream and record that evidence. Do not remove a working distribution path.

If releases still have no usable binary assets and the repository still has no release-asset workflow/process, treat `InstallMethod::GitHubRelease` as dead/unbacked behavior.

### E2. Prefer deletion over adding release automation

When the path is still unsupported:

Files:

```text
src/update.rs
Cargo.toml
README.md
USER_GUIDE.md / installation docs as applicable
```

Delete the standalone self-replacement path:

- `InstallMethod::GitHubRelease`;
- target archive name selection;
- archive download/extraction;
- temporary install/rollback logic used only for standalone assets;
- `tar` direct dependency if no other production caller remains.

Retain:

- Cargo-managed updating;
- Homebrew-managed updating;
- dry-run/update version reporting that remains meaningful for those install methods.

For an unmanaged/source executable, return a clear message to update through the installation method/source build rather than guessing that a matching GitHub asset exists.

Do not create a release workflow just to keep this code alive.

Acceptance:

- [ ] Cargo installs still update through Cargo.
- [ ] Homebrew installs still update through Homebrew.
- [ ] source/unmanaged builds fail clearly without attempting nonexistent assets.
- [ ] `tar` is removed if it becomes unused.
- [ ] documentation matches the actually supported distribution methods.

### E3. Measure whether gzip-compressed bundled themes still earn `flate2`

After E2, inspect direct production uses of `flate2`.

If bundled-theme decompression is the only remaining use, perform one controlled comparison on the same platform/toolchain:

```text
baseline: current gzip/base64 bundled themes + flate2
test:     generated plain static theme strings, no runtime decompression
```

Record release binary bytes for both.

Decision rule:

- remove runtime theme compression and `flate2` only if the final release binary is no larger and the generated-source/build story becomes simpler;
- otherwise keep the current gzip bundle and `flate2`.

Do not optimize for Cargo dependency count while making the actual executable larger. Do not introduce a different compression crate.

Acceptance:

- [ ] the result is based on measured release bytes, not `cargo tree` intuition.
- [ ] the smaller/simpler measured representation is retained.
- [ ] all 50 bundled themes plus the default remain available.

## 10. Workstream F — Align verification with the simplified product

### F1. Build production binaries with production features

Files:

```text
scripts/release-check.sh
```

Change the generic release build/smoke from `--all-features` to default production features:

```text
cargo build --workspace --release
cargo run --release --bin snp -- --version
cargo run --release --bin snp -- --help
```

Keep `--features test-support` only on the individual tests that require failpoints/barriers.

If a workspace package genuinely needs a non-default feature for its production binary, specify that package/feature narrowly rather than returning to `--all-features` globally.

Acceptance:

- [ ] the release binary used for normal smoke is production-equivalent.
- [ ] crash/failpoint tests still opt into test seams explicitly.
- [ ] `test-support` behavior is not compiled into the generic release artifact used as the production sanity check.

### F2. Remove protoc setup from CI after Workstream C

Files:

```text
.github/workflows/ci.yml
scripts/ci/install-protoc.sh
scripts/ci/install-protoc.ps1
```

Once no ordinary build requires protoc:

- remove `PROTOC_VERSION`;
- remove Unix and Windows install steps;
- delete installer scripts if unreferenced.

Do not replace them with another codegen setup.

### F3. Reduce link-check duplication

File:

```text
.github/workflows/link-check.yml
```

Retain:

- pull-request link check;
- weekly scheduled link check.

Remove the redundant `push: main` trigger.

Do not merge link checking into the Rust CI job; keeping the small workflow independent is clearer and avoids external-link failures blocking code-only main pushes after an already-green PR.

### F4. Keep the Phase 14F CI topology

Do not broaden the test matrix.

The intended topology remains:

```text
Linux:
  scripts/check.sh

macOS/Windows:
  cargo check --workspace --all-targets
  platform_smoke

manual release:
  scripts/release-check.sh verify
```

Only move/delete tests when production code deletion makes the corresponding invariant obsolete.

Examples:

- delete persistent-rate-limit persistence tests when the feature is deleted;
- rewrite server shutdown tests around observable behavior after the orchestration simplification;
- delete duplicate atomic-write tests from `select_cmd` after it delegates to the canonical utility;
- keep transaction crash-recovery tests because the retained journal remains production behavior.

Do not set a target test count.

## 11. Documentation cleanup required by implementation

Update documentation only after production shape is stable.

At minimum inspect:

```text
README.md
USER_GUIDE.md
snip-sync/README.md
AGENTS.md
PUBLIC_API.md
ARCHITECTURE_INVENTORY.md
architecture/overview.md
architecture/persistence.md
architecture/sync.md
architecture/commands.md or equivalent
```

Correct known drift while touching those sections:

- asynchronous/background audit-writer language must not remain after Phase 14E's synchronous audit append change;
- `LibraryManager::new()` documentation must say malformed existing `libraries.toml` fails closed rather than returning defaults;
- protocol generation docs must describe committed generated Rust and no normal-build protoc requirement;
- theme generation docs must describe explicit `scripts/build_themes.py`, not automatic Cargo/Python regeneration;
- server ownership docs must describe one current-format kernel lock owner record after D1;
- remove `PERSIST_RATE_LIMITS` documentation after D3;
- updater/install docs must match the supported methods after E1/E2;
- test/CI docs must match the production-feature release build and no-protoc CI.

Do not create new architecture documents. Update the existing authoritative ones and delete obsolete claims.

## 12. Verification strategy

### 12.1 Per-workstream verification

After each workstream run the smallest focused tests that prove the edit, then:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Do not run the full release suite after every deletion.

Suggested focused checks:

A:

```bash
cargo test --bin snp
cargo test --test platform_smoke
```

B:

```bash
cargo test --lib commands::
cargo test --test destination_permissions --features test-support
```

Use exact test filters that exist after the refactor; do not create tests solely to make these example commands literal.

C:

```bash
cargo check --workspace --all-targets
cargo package -p snip-proto --locked
```

D:

```bash
cargo test -p snip-sync --lib
cargo test --test snip_sync_lifetime -- --ignored --test-threads=1
```

plus existing stop/restart/croncheck integration coverage.

E:

```bash
cargo build --release --bin snp
```

Record exact binary bytes before/after dependency changes on the same host/toolchain.

### 12.2 End-of-phase verification

Before marking Phase 15 complete:

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify
```

Then ensure `git status --short` is clean.

The manual release verification is sufficient. Do not add a new CI lane for Phase 15-specific deletion checks.

## 13. Required measurements and implementation record

At implementation completion, append a compact table to this plan:

| Item | Baseline | Final | Result |
|---|---:|---:|---|
| `snp` release binary bytes | record on one fixed host | record | delta |
| root direct dependencies | record | record | delta |
| `snip-sync` direct dependencies | record | record | delta |
| `snip-proto` build dependencies | 1 (`tonic-prost-build`) | expected 0 | delta |
| normal CI protoc setup steps | Linux + macOS + Windows | expected 0 | delta |
| current-format server owner records | 2 | expected 1 | delta |
| rate-limit persistence background tasks | 1 | expected 0 | delta |

Do not use raw repository LOC as an acceptance gate. Generated code and tests distort it. Record meaningful mechanism/dependency deletion instead.

A net source reduction is expected, but correctness wins if a tiny helper is needed to safely delete a much larger duplicate path.

## 14. Explicit acceptance criteria

Phase 15 is complete only when every applicable item below is true.

### CLI and command behavior

- [ ] Clap schema assertion passes.
- [ ] duplicate `snp data r` alias is gone.
- [ ] `snp select` cannot delete snippets or initiate sync.
- [ ] `snp select` remains a deterministic read-only selection primitive.
- [ ] search startup classification matches its intentionally retained mutation/sync capability.
- [ ] run/clip/default TUI deletion and sync behavior remain correct.
- [ ] stable exit codes remain unchanged.

### Client consolidation

- [ ] `select_cmd` does not own a second atomic replace implementation.
- [ ] theme config persistence uses the canonical atomic helper.
- [ ] client editor resolution/parsing has one implementation.
- [ ] editor command arguments work without shell evaluation.
- [ ] no new utility crate was created for trivial deduplication.

### Protocol/build

- [ ] one generated Rust protocol implementation remains.
- [ ] root client and `snip-sync` both use `snip-proto`.
- [ ] ordinary Cargo builds do not run protobuf generation.
- [ ] ordinary Cargo builds do not require protoc.
- [ ] normal Cargo builds do not invoke Python for themes.
- [ ] committed generated protocol/theme files remain sufficient for crates.io installs.

### Server simplification

- [ ] kernel-backed singleton exclusion remains.
- [ ] one current-format server owner identity remains.
- [ ] stop/restart still validate process identity before signaling.
- [ ] croncheck remains health-driven.
- [ ] server lifetime has no arbitrary pre-signal timeout.
- [ ] unexpected service failure still fails and shuts down its sibling.
- [ ] requested graceful shutdown remains bounded.
- [ ] manual consumed-handle bookkeeping is removed.
- [ ] in-memory rate limiting remains.
- [ ] persistent rate-limit DB/background machinery is removed.
- [ ] existing databases with legacy `rate_limits` table still open.

### Footprint/updater

- [ ] active Cargo and Homebrew update paths remain.
- [ ] unsupported standalone archive self-update code is removed if release inspection still shows no working asset pipeline.
- [ ] `tar` is removed if no production caller remains.
- [ ] bundled-theme compression decision is measured and recorded if `flate2` becomes theme-only.
- [ ] no feature is claimed in docs without a working distribution/runtime path.

### Verification/CI

- [ ] generic release build/smoke uses production/default features.
- [ ] test failpoint features are enabled only for tests that require them.
- [ ] CI no longer installs protoc when codegen is removed.
- [ ] link checking does not redundantly run again on every main push.
- [ ] Linux correctness + macOS/Windows smoke topology is retained.
- [ ] no new CI workflow or matrix was added.
- [ ] `bash scripts/check.sh` passes.
- [ ] `bash scripts/release-check.sh verify` passes from a clean tree.

### Scope

- [ ] Phase 14G journal RETAIN decision is untouched.
- [ ] no daemon/supervisor/database/framework was added.
- [ ] production code/mechanism count is lower than the Phase 15 baseline.
- [ ] documentation describes the final implementation rather than superseded hardening phases.

## 15. Small-model execution rules

1. Read the entire named production file before editing it.
2. Execute workstreams in order.
3. Make one conceptual change at a time; compile before moving on.
4. Prefer deleting a call path to wrapping it.
5. Reuse an existing helper only when its semantics actually match; do not force DRY across crate boundaries.
6. Do not change protocol fields/tags while consolidating protocol ownership.
7. Do not weaken user-data persistence to reduce code.
8. Do not interpret old tests as immutable architecture. Preserve the behavior they prove, then simplify the tests around the new production shape.
9. Do not add compatibility machinery for an undocumented accidental behavior.
10. Keep legacy compatibility only where real persisted state requires it (notably old server PID files and existing SQLite tables).
11. Measure binary-size changes using one fixed platform/toolchain and actual release binary bytes.
12. If a proposed deletion requires a larger replacement abstraction, stop and keep the existing mechanism unless the old mechanism is actually incorrect.
13. Do not create Phase 16 planning artifacts during implementation. Record any genuinely blocked item in this file and finish the rest.

## 16. Expected final architecture

At Phase 15 closure the repository should have this simpler shape:

```text
snp
 ├─ one CLI schema with a schema assertion
 ├─ selector loop with explicit destructive capability
 ├─ one client editor parser/resolver
 ├─ one atomic-write implementation
 ├─ one generated protocol dependency (`snip-proto`)
 ├─ committed theme/protocol generated files; no build-time Python/protoc
 ├─ retained proven local transaction journal
 └─ optional sync/one-shot auto-sync helper

snip-sync
 ├─ one kernel-backed server ownership record
 ├─ simple two-service graceful shutdown collection
 ├─ bounded in-memory rate limiter
 ├─ SQLite snippet/library/auth state
 ├─ gRPC sync + HTTP health/metrics
 └─ no persistent rate-limit control-plane state
```

The intended direction after Phase 15 is maintenance. Future work should primarily fix reproduced defects or remove obsolete code. Another generalized hardening or architecture phase should require a concrete user-visible need rather than being the default next step.

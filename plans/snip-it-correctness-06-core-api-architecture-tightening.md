# Phase 06: Core Architecture and Public API Tightening

## Purpose

Reduce accidental public surface, clarify crate responsibilities, remove obsolete implementation paths, and make the active architecture easier to maintain without disrupting the now-correct synchronization behavior.

This is a maintainability phase. It must not reopen correctness semantics already closed by Phases 01-05.

## Preconditions

Do not perform broad crate movement until:

- canonical sync execution is stable;
- pending/debounce/failure/status semantics are closed;
- end-to-end tests protect user-visible behavior;
- cross-platform process tests are passing.

The integration harness from Phase 05 is the safety net for structural refactoring.

## Current architectural pressure

The package currently exposes modules publicly partly because the `snp` binary is a separate crate target within the same package. This can make CLI implementation modules appear to be stable library APIs. The main binary also links TUI, clipboard, network, cryptography, keyring, update, process, and storage concerns into one package.

The goal is not maximal crate fragmentation. The goal is intentional boundaries with low migration risk.

## Design principles

1. Public Rust API is a compatibility commitment.
2. CLI implementation details should not be public solely for binary access.
3. Storage, parsing, expansion, synchronization, and presentation should have explicit boundaries.
4. Process orchestration remains in the CLI layer.
5. Synchronization protocol and encryption should be testable without TUI/clipboard dependencies.
6. Avoid a large rewrite; move boundaries incrementally behind existing tests.
7. Prefer fewer coherent crates over many microcrates.

## Target layering

A recommended end state is three logical layers. They may initially remain modules and later become crates if the dependency graph justifies it.

### Core layer

Responsibilities:

- snippet and library data models;
- TOML parsing/serialization;
- legacy/Pet compatibility;
- variable/default/choice parsing and expansion;
- sorting/filtering;
- import/export primitives;
- usage metadata model;
- atomic local persistence;
- stable IDs and migration primitives.

Must not depend on:

- Ratatui/Crossterm;
- clipboard backends;
- process spawning;
- tonic/network client;
- keyring;
- self-update.

### Sync-client layer

Responsibilities:

- sync request/report/error types;
- protocol client;
- encryption/decryption;
- credential abstraction;
- merge and direction semantics;
- remote/local synchronization transaction;
- no CLI rendering;
- no detached worker orchestration.

Dependencies on core models should be explicit and one-directional.

### CLI/application layer

Responsibilities:

- Clap command model;
- TUI;
- clipboard;
- shell integration;
- editor invocation;
- detached worker/executor process lifecycle;
- lock and status orchestration;
- self-update;
- human/JSON rendering;
- platform integration.

## Workstream A: Public API inventory

Generate or manually record all currently public items from `lib.rs` and public modules.

Classify each as:

- intentionally stable library API;
- public only because the binary needs it;
- integration-test exposure;
- transitional compatibility item;
- accidental/dead public item.

For each stable API, document:

- blocking versus async behavior;
- filesystem effects;
- process/network effects;
- error type;
- thread-safety expectations;
- stability commitment;
- whether it is intended for third-party consumers.

Use `cargo public-api` or a similar tool in CI if acceptable to detect unintended changes.

## Workstream B: Introduce a thin application facade

Avoid making all command/UI modules public. Options:

1. Move the binary into a small workspace package that depends on internal crates.
2. Keep one package but expose a narrow application entry facade.
3. Use a private module tree and integration-test public test support behind a feature.

Preferred long-term shape:

```text
crates/snip-core
crates/snip-sync-client
crates/snp-cli
snip-proto
snip-sync
```

However, do not move everything at once. A staged approach may be:

- first create internal `core` and `sync_client` module facades;
- route existing callers through them;
- then extract crates in separate commits;
- preserve package name and binary installation behavior.

## Workstream C: Stabilize core data types

For externally useful types:

- prefer private fields with constructors/accessors where mutation invariants matter;
- use `#[non_exhaustive]` on enums expected to grow;
- avoid returning parallel vectors when an owned item struct is clearer;
- provide typed reports instead of tuples;
- distinguish borrowed view types from owned persistence types;
- expose stable identifiers through dedicated newtypes if appropriate;
- avoid leaking internal filesystem layout as API.

Review `SnippetData` parallel vectors. Internally, an item-oriented representation may reduce index-correlation risk:

```rust
pub struct SnippetView {
    pub id: SnippetId,
    pub description: String,
    pub command: String,
    pub output: String,
    pub tags: Vec<String>,
    pub folders: Vec<String>,
    pub favorite: bool,
}
```

Do not change hot TUI paths without benchmarks and regression tests; conversion at the boundary may be sufficient.

## Workstream D: Define blocking and async boundaries

The CLI can own a Tokio runtime, but core storage operations should not become async without need.

Rules:

- local filesystem and parsing APIs may remain synchronous;
- network sync exposes one canonical async or blocking boundary, not both with false cancellation wrappers;
- process-level timeout remains in worker supervision;
- library APIs must document whether cancellation is possible;
- avoid hidden global runtime dependencies where practical;
- avoid `LazyLock<Runtime>` becoming part of public behavior.

If the canonical sync client is async, foreground and executor adapters may create/use a runtime explicitly. Do not wrap blocking work in `spawn_blocking` and claim timeout cancellation.

## Workstream E: Feature-gate optional subsystems

Evaluate, do not blindly implement, features such as:

```toml
[features]
default = ["tui", "clipboard", "sync", "bundled-themes", "self-update"]
tui = ["dep:ratatui", "dep:crossterm"]
clipboard = ["dep:arboard", "dep:clipboard-win"]
sync = ["dep:tokio", "dep:tonic", "dep:prost", "dep:aes-gcm", "dep:argon2", "dep:keyring"]
bundled-themes = ["dep:lzma-rs"]
self-update = ["dep:semver", "dep:sha2"]
```

Official `snp` binaries may continue to enable all intended product features. The benefits are:

- smaller library dependency surface;
- faster downstream compilation;
- clearer security audit boundaries;
- docs.rs builds without irrelevant platform integrations;
- easier core fuzzing/testing.

Before landing feature gates, prove:

- default build unchanged;
- `--no-default-features` has a useful supported meaning;
- combinations compile in CI;
- runtime assets are correctly gated;
- documentation states supported combinations;
- no feature explosion is created.

## Workstream F: Remove obsolete architecture

Audit for remnants of:

- old in-process auto-sync coordinator;
- false async timeout wrappers;
- duplicate direction resolution;
- unused worker lock types or policy fields;
- release-specific transitional aliases;
- debug `eprintln!` paths;
- old helper APIs exposed only for tests;
- stale comments describing previous architecture;
- duplicated sync wrappers;
- unused `max_retries` after Phase 03 policy closure.

Use code search and dead-code warnings, but do not remove compatibility behavior without tests and changelog documentation.

## Workstream G: Test-support boundary

Integration tests need access to internals without making them stable public API.

Options:

- `#[cfg(any(test, feature = "test-support"))]` module;
- dedicated `snip-test-support` crate not published;
- public test helpers in `snip-sync` behind `test-helpers` feature;
- process-level black-box tests that avoid internal access.

Rules:

- test-support features are nondefault;
- production release artifacts do not expose dangerous failpoints;
- crates.io package metadata makes intent clear;
- test helpers cannot contain secrets or weaken normal validation accidentally.

## Workstream H: Semver and API checks

Add CI checks for:

- public API diff against previous release where practical;
- docs.rs build;
- minimal feature build;
- default feature build;
- all feature build;
- package compilation after extraction;
- binary name/path compatibility;
- Cargo install behavior;
- workspace dependency direction.

Establish a policy:

- internal modules may change freely;
- stable core/sync-client APIs follow semver;
- hidden CLI subcommands are internal and not covered by compatibility guarantees;
- JSON command output has a separately documented compatibility policy.

## Workstream I: Dependency audit and duplication

Use `cargo tree -d` and dependency inspection to identify:

- duplicate major versions;
- dependencies used only by one optional subsystem;
- dev dependencies leaking into normal builds;
- crypto/network dependencies linked when sync is disabled;
- tempfile duplication;
- platform backend inconsistencies.

Do not chase dependency count at the expense of correctness. Remove or gate dependencies only when the replacement is simpler and well tested.

## Workstream J: Benchmarks and regression budget

Before and after restructuring, measure:

- cold `snp --version` startup;
- `snp list --json` startup for small and large libraries;
- TUI initial render;
- fuzzy filtering on large libraries;
- binary size;
- compile time for default and core-only builds;
- memory baseline.

Set regression thresholds appropriate to the project. Structural cleanup should not materially degrade the immediate local workflow.

## Required tests

- existing full behavioral suite remains green after each extraction;
- stable public API examples compile;
- core layer builds without UI/network/process dependencies;
- sync-client tests run without TUI/clipboard;
- CLI package installs the same `snp` binary;
- hidden worker/executor re-exec still finds current executable;
- feature combinations compile;
- package contents include required assets;
- public API diff contains only intentional changes;
- no test-only failpoint is reachable in production build;
- startup/performance benchmarks stay within thresholds.

## Documentation

Update:

- architecture overview;
- crate/module responsibility map;
- public API documentation;
- Cargo features;
- contributor guidance;
- test-support conventions;
- semver policy;
- package/install instructions if workspace paths change;
- docs.rs metadata/readmes.

## Recommended commit sequence

1. Inventory public API and add API/build checks.
2. Introduce internal core and sync-client facades without moving files.
3. Route callers through facades and remove duplicate wrappers.
4. Narrow public modules and create test-support boundary.
5. Extract core crate if justified.
6. Extract sync-client crate if justified.
7. Move CLI application into its final package boundary.
8. Add/validate feature gates.
9. Remove obsolete architecture and dependencies.
10. Reconcile docs and benchmark results.

Avoid one giant workspace rewrite. Each commit should compile and pass the end-to-end suite.

## Exit criteria

Phase 06 is complete only when:

- every public module/item has an intentional stability classification;
- CLI internals are not public solely for binary access;
- core storage/model logic is separable from TUI, clipboard, and network concerns;
- sync client is separable from detached process orchestration;
- canonical sync behavior remains unchanged and fully tested;
- obsolete coordinator/timeout/debug paths are removed;
- supported feature combinations compile;
- package/install behavior is preserved;
- public API and semver policy are documented;
- startup and local workflow performance do not materially regress;
- all platform and package tests pass.

# Phase 06A: Core Architecture and Public API Tightening

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-06-core-api-architecture-tightening.md
```

Begin only after Phase 05A protects the real process, protocol, storage, and CLI contracts. Baseline implementation commit: `ff506f5934957c4fd989224a6f0e0cf10f907567`.

## Purpose

Make the corrected codebase easier to maintain by narrowing accidental public surface, clarifying module ownership, consolidating canonical operations, and removing transitional architecture without forcing unnecessary crate fragmentation.

This phase is structural. User-visible behavior, sync semantics, file formats, installed binary name, current-executable re-exec, and lightweight one-shot process architecture must remain unchanged unless a separately documented compatibility fix is required.

## Required outcomes

1. Every public Rust item is intentionally public or explicitly transitional.
2. CLI implementation details are not exposed solely because the binary needs them.
3. Local domain/storage logic has a clear dependency boundary from UI, clipboard, process, and network layers.
4. Sync protocol/client logic is separate from detached scheduling/process supervision.
5. One canonical operation exists for each behavior-critical action.
6. Test-only internals no longer inflate the production API.
7. Dead Release 5 transition paths, duplicate wrappers, and stale documentation are removed.
8. Default build/install behavior and startup performance do not materially regress.
9. The architecture remains a small, coherent Rust workspace; extraction occurs only where evidence justifies it.

## Non-goals

Do not:

- rewrite the application from scratch;
- split every module into a crate;
- introduce dependency injection frameworks;
- add a plugin runtime;
- redesign sync protocol semantics;
- replace TOML persistence;
- convert synchronous local I/O to async without a demonstrated need;
- move process supervision into the sync client;
- change the single installed `snp` binary model.

---

## Workstream A — Public API inventory and compatibility classification

Generate an inventory from `src/lib.rs` and all reachable `pub` items.

Classify each item as:

```text
stable-public
provisional-public
application-internal
integration-test-only
compatibility-shim
dead-or-accidental
```

For every stable/provisional public API record:

- owning module;
- intended consumer;
- blocking/async behavior;
- filesystem side effects;
- network side effects;
- process side effects;
- error type;
- thread-safety expectations;
- semver policy;
- feature requirements;
- examples/tests.

Create:

```text
docs/PUBLIC_API.md
```

or an equivalent architecture document.

Add a baseline public-API report through `cargo public-api` or another reviewed mechanism if it can run reliably in CI. Do not make a brittle tool mandatory without pinning/version policy.

### Acceptance

- no undocumented public module remains;
- test helpers are not classified as stable product API;
- hidden worker/executor internals are explicitly outside compatibility guarantees;
- intentional breaking changes are grouped and changelogged.

---

## Workstream B — Establish logical layers before crate extraction

Use logical module boundaries first.

### Domain/core layer

Owns:

- snippet/library domain models;
- stable identifiers;
- TOML representations and compatibility parsing;
- variable/default/choice parsing and expansion;
- filtering/sorting/matching primitives;
- import/export transformations;
- usage metadata model;
- validation reports;
- local persistence interfaces.

Must not depend on:

- Clap;
- Ratatui/Crossterm;
- clipboard libraries;
- process spawning;
- Tokio runtime ownership;
- tonic transport;
- keyring;
- self-update.

### Sync-client layer

Owns:

- protocol client;
- sync request/options/report types;
- merge/direction semantics;
- encryption/decryption framing;
- credential-provider interface;
- local/remote comparison operation;
- typed sync errors.

Must not own:

- detached worker scheduling;
- execution locks;
- CLI rendering;
- shell/TUI behavior;
- status presentation;
- process timeout supervision.

### Application/CLI layer

Owns:

- Clap model and dispatch;
- TUI and clipboard;
- shell integration;
- editor invocation;
- local transaction orchestration;
- worker/executor re-exec;
- lock/pending/status scheduling;
- foreground runtime creation;
- human/JSON rendering;
- update/install integration.

### Dependency rule

```text
application -> sync-client -> core
application -------------> core
```

No core-to-application or sync-client-to-application dependency.

Add an architecture test or dependency audit script that detects reverse dependencies where practical.

---

## Workstream C — Prefer an internal application facade over premature workspace split

The preferred first move is a narrow internal facade, not immediate crate extraction.

Recommended structure:

```text
src/core/
src/sync_client/
src/app/
src/platform/
```

The binary should call one narrow entry point, for example:

```rust
pub fn run_cli() -> ExitCode
```

or:

```rust
pub struct Application;
impl Application {
    pub fn dispatch(command: Command) -> AppResult;
}
```

Use private or crate-visible modules wherever possible.

### Crate extraction decision gate

Extract `snip-core` or a sync-client crate only if all are true:

- logical boundaries have already been enforced;
- Phase 05A tests are green;
- the dependency graph shows meaningful compile/audit/test benefits;
- package/release complexity remains acceptable;
- current executable re-exec and crates.io/Homebrew behavior are preserved;
- no cyclic dependency or broad public API exposure is introduced.

A successful Phase 06A may finish with one main package plus clearer modules. Crate count is not an exit criterion.

---

## Workstream D — Canonical operation inventory

List all behavior-critical operations and identify their single canonical implementation.

Required operations:

```text
load/save library
load/save settings
match/select snippet
expand variables
import/export
record usage
perform sync
resolve sync direction
record pending mutation
schedule worker
supervise executor
record status
conditional pending clear
inspect status
validate/repair state
```

For each operation:

- designate canonical function/type;
- list adapters/callers;
- remove duplicate implementations;
- ensure adapters do not alter semantics;
- add structural tests for especially dangerous duplicates.

Examples of forbidden duplication:

- foreground and detached paths implementing separate sync algorithms;
- multiple direction resolvers;
- multiple pending-generation increment paths;
- command modules writing library files directly through bespoke sequences;
- status/doctor reparsing control files independently;
- separate selector ranking logic for get/run/clip/edit.

---

## Workstream E — Data model tightening

Review parallel-vector and loosely coupled representations for index-correlation risk.

Prefer item-oriented internal types where they reduce invariant complexity:

```rust
pub struct SnippetRecord {
    id: SnippetId,
    description: String,
    command: String,
    tags: Vec<String>,
    output: Option<String>,
    folders: Vec<String>,
    favorite: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}
```

Guidelines:

- introduce `SnippetId` and `LibraryId` newtypes if not already stable;
- keep serialization compatibility through dedicated persisted representations;
- private fields where mutation invariants matter;
- explicit constructors/update methods;
- `#[non_exhaustive]` for public enums expected to grow;
- typed reports instead of long tuples/parallel vectors;
- no unnecessary cloning in hot selection/TUI paths;
- migration conversions are explicit and tested.

Do not combine identity-policy changes with large representation changes unless Phase 07A tests and migration fixtures are ready.

---

## Workstream F — Error boundary cleanup

Define layer-owned errors:

```text
CoreError
PersistenceError
ValidationError
SyncClientError
CredentialError
ProcessSupervisionError
ApplicationError
```

Requirements:

- preserve typed sync failure classification;
- avoid string matching for errors produced inside typed layers;
- attach operation/path context without leaking secrets;
- map errors once at layer boundaries;
- machine output uses stable error/diagnostic codes;
- public APIs do not expose CLI-only presentation text;
- source chains remain useful for logs while persisted/user messages are sanitized;
- no catch-all success or fallback-to-default behavior that changes correctness semantics.

Keep one top-level `SnipError` only if it is a deliberate application facade, not a reason for every internal module to depend on the entire application error enum.

---

## Workstream G — Blocking, async, and runtime ownership

Rules:

- local parsing/storage APIs remain synchronous unless measured blocking creates a real issue;
- sync-client exposes one canonical async boundary or one clearly documented blocking adapter;
- foreground CLI owns runtime creation/use;
- executor owns its runtime for the one-shot child;
- process timeout remains outside the sync operation;
- no `spawn_blocking` wrapper is described as cancellable;
- no hidden global runtime becomes part of public library behavior;
- test runtimes are explicit.

If both async and blocking public APIs exist, one must be a thin documented adapter over the other and cancellation semantics must be truthful.

---

## Workstream H — Test-support boundary

Move integration-only visibility into one nondefault boundary.

Options, in preference order:

1. black-box real-process tests requiring no internals;
2. `#[cfg(test)]` unit support;
3. nondefault `test-support` feature;
4. unpublished workspace test-support crate.

Rules:

- dangerous failpoints/event sinks unavailable in release artifacts;
- production validation cannot be disabled by test environment variables;
- test-only public APIs excluded from documentation/semver policy;
- package tests verify no test-control surface is shipped;
- `snip-sync` recording helpers are similarly gated.

---

## Workstream I — Feature and dependency boundaries

Evaluate a small feature model only after module ownership is clear.

Possible features:

```toml
[features]
default = ["tui", "clipboard", "sync", "self-update", "bundled-themes"]
tui = []
clipboard = []
sync = []
self-update = []
bundled-themes = []
test-support = []
```

Requirements before landing:

- default installed behavior unchanged;
- official releases enable supported product features;
- `--no-default-features` has a documented useful build or is not promised;
- minimal/default/all combinations compile in CI;
- platform dependencies are correctly gated;
- runtime assets remain included;
- no combinatorial feature explosion;
- sync-disabled build does not pull keyring/tonic/crypto unnecessarily if this can be achieved cleanly;
- feature-gating does not leak through file formats.

Use `cargo tree -d` and `cargo tree -e features` to inspect duplicate and optional dependency pressure. Do not replace robust dependencies solely to reduce count.

---

## Workstream J — Remove obsolete and transitional architecture

Audit and remove:

- old coordinator terminology and code;
- direct worker spawns outside central scheduler;
- duplicate sync wrappers;
- duplicate policy loaders;
- false timeout/cancellation comments;
- unused `max_retries` or stale fields;
- legacy production aliases retained only for completed migration;
- temporary debug environment variables/eprintln paths;
- source-scanning tests superseded by behavioral evidence;
- obsolete public test helpers;
- dead lock types if no longer used;
- stale Release 5 labels in current architecture docs where they obscure final behavior.

Retain migration compatibility only when supported by fixtures and documented policy.

---

## Workstream K — Public API and semver gates

Add CI checks for:

- public API diff against a recorded baseline;
- docs build;
- doctests/examples;
- minimal/default/all feature builds if supported;
- package compilation;
- installed binary name and hidden re-exec commands;
- stable JSON schema fixtures from Phase 04A;
- dependency-direction check;
- no test-support surface in package.

Document:

- Rust API semver scope;
- internal module freedom;
- hidden subcommands outside public contract;
- JSON schema compatibility;
- file-format compatibility;
- CLI exit-code compatibility once Phase 08A closes.

---

## Workstream L — Performance and binary-size regression budget

Capture baseline and post-change measurements:

```text
snp --version cold start
snp list for 10, 1k, and 10k snippets
snp status
TUI initial render
fuzzy filter latency
binary size
resident memory after list/status
cargo check default time
core-only check time if supported
```

Guidelines:

- use repeatable benchmark scripts;
- record median and environment;
- do not overfit noisy CI microbenchmarks;
- material regressions require explanation or correction;
- local terminal workflows remain immediate;
- architecture cleanup must not add background initialization or network access.

---

## Required tests

### Boundary tests

- core compiles/tests without application imports;
- sync-client compiles/tests without TUI/clipboard/process supervisor;
- application uses canonical facades;
- dependency graph has no reverse edge;
- package installs the same `snp` binary;
- current-exe worker/executor re-exec still works.

### API tests

- public examples compile;
- public API diff is intentional;
- test-only items absent from release package;
- stable error/report types serialize or display according to contract;
- JSON outputs unchanged unless versioned.

### Behavior tests

- full Phase 05A suite remains green after every structural step;
- file formats round-trip unchanged;
- sync direction/merge/status/pending behavior unchanged;
- shell/TUI/clipboard behavior unchanged;
- startup/performance thresholds maintained.

### Feature/package tests

- supported feature combinations;
- default/all package builds;
- assets included;
- docs build;
- no unexpected crypto/network deps in supported core-only build;
- platform matrix.

---

## Recommended implementation sequence

1. Inventory public API and record baseline.
2. Define logical layer ownership and dependency rules.
3. Introduce internal facades without file movement.
4. Route callers through canonical operations.
5. Narrow visibility and create test-support boundary.
6. Clean error boundaries and duplicate wrappers.
7. Evaluate and optionally perform limited crate extraction.
8. Evaluate feature gates and dependency cleanup.
9. Remove obsolete architecture/comments/tests.
10. Run behavior, package, API, and performance gates.
11. Write `plans/snip-it-correctness-06a-status.md`.

Avoid a single giant refactor commit. Every commit must compile and pass the relevant Phase 05A suites.

## Required verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
cargo package --workspace
cargo tree -d
```

Add public-API and feature commands according to the adopted tooling.

## Exit criteria

Phase 06A is complete only when:

- every public item has an intentional classification;
- CLI internals are no longer public solely for binary access;
- core, sync-client, and application responsibilities are explicit and dependency-correct;
- canonical behavior-critical operations are singular;
- test-only failpoints/helpers are outside release API/artifacts;
- obsolete Release 5 transition paths are removed;
- supported feature/package builds pass;
- installed `snp` behavior and hidden re-exec remain compatible;
- public API/semver and JSON/file-format policies are documented;
- Phase 05A behavioral evidence remains green;
- startup, binary-size, and local workflow performance do not materially regress;
- no unnecessary daemon, service, plugin system, or microcrate explosion was introduced.
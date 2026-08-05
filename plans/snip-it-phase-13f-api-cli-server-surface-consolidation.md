# Phase 13F — API, CLI, Server, and Documentation Surface Consolidation

Status: COMPLETE

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Dependencies: Phase 13E complete or source/API boundaries stable

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Reduce long-term maintenance obligations and user-facing conceptual clutter without removing current functionality or breaking established command spellings in Phase 13.

The repository currently exposes implementation-heavy Rust modules as a nominal stable API, presents many advanced maintenance commands at the top level, and includes server administration features that risk evolving into an internal service manager. Architecture documentation duplicates volatile line-number inventories and implementation claims across several files.

The target is:

- a deliberately supported Rust library API centered on snippet data, loading/saving, variable expansion, and deterministic selection;
- implementation-only sync, UI, transaction, auto-sync, diagnostics, and process-control details kept private unless an external consumer is documented;
- coherent CLI command groups with compatibility aliases for existing spellings;
- a bounded `snip-sync` administration surface that complements, rather than replaces, operating-system/container supervision;
- optional non-core server facilities default-off or feature-gated only when measurement shows real value;
- architecture documentation organized by stable symbols and invariants, not line numbers or historical phase details.

This is a consolidation phase, not a redesign or breaking release.

## 2. Constraints

### Required

- preserve all existing documented workflows in Phase 13;
- preserve old command spellings as aliases or compatibility shims;
- avoid semver-breaking Rust API removals unless the items were never documented/supported and a compatibility re-export is inexpensive;
- document the intended supported API explicitly;
- keep server setup simple for Cargo, Homebrew/client, source, Docker, systemd, and cron fallback users;
- retain loopback/trusted-LAN/reverse-proxy deployment guidance;
- remove stale line-number references and duplicated architecture prose;
- make implementation details private where tests can use narrow test-support seams.

### Prohibited

- a new major version solely for cleanup;
- deleting existing commands or flags in this phase;
- adding a plugin system, extension registry, command framework, or dynamic dispatch layer;
- adding IPC between client and server components;
- turning `snip-sync` into a full service manager;
- adding install/uninstall system service commands;
- adding web UI, dashboard, admin API, or remote management;
- adding new server metrics, CORS, proxy, or authentication features;
- broad data model or protocol redesign;
- creating more documentation files than are removed or consolidated.

## 3. Workstream A — Define the supported Rust API

### 3.1 Consumer inventory

Before changing visibility:

1. search repository and public documentation for external/library examples;
2. inspect crates.io/docs.rs intent in package metadata and README;
3. identify modules exposed only so integration tests can reach internals;
4. identify types used by the `snp` binary because binary and library are separate crates in one package;
5. identify any real downstream consumer if available from repository references; do not speculate.

Record a table:

| Item/module | Binary use | Integration-test use | Documented external use | Target visibility |
|---|---|---|---|---|

### 3.2 Intended stable API

The default supported API should be small and domain-oriented. Candidate supported items:

- `Snippet`, `Snippets`, and documented library metadata types;
- load/save operations for canonical TOML storage;
- variable parsing and expansion primitives that do not require TUI prompts;
- deterministic selector types/results;
- sort options where useful to non-TUI consumers;
- typed error/result surface;
- atomic write operation only if it is intentionally useful outside this crate and can be documented without exposing internal durability machinery.

Candidate implementation-only areas:

- command dispatch modules;
- TUI rendering/state;
- logging/audit initialization;
- auto-sync scheduler/pending/status internals;
- transaction journals and failpoint seams;
- process file lock internals;
- sync orchestration and recovery markers;
- diagnostics/status snapshot internals;
- test event observers;
- server-specific protobuf re-exports not needed by a documented client API.

The exact result follows the inventory. Do not force a preconceived API if the binary requires a clean public boundary.

### 3.3 Binary/library boundary

Because `src/main.rs` is a separate crate, public visibility needed only by the binary is not automatically a supported external contract.

Use one of these bounded approaches:

- document `#[doc(hidden)]` binary-support modules as unstable implementation details;
- move binary-only orchestration into a small internal facade module with narrow functions;
- retain public symbols temporarily but remove claims that every `pub` item is stable.

Do not create a second internal crate solely to solve visibility.

### 3.4 Test access

Integration tests should prefer:

- public user-facing CLI/process behavior;
- supported domain API;
- a narrow `test-support` module compiled only for tests needing internal setup.

Do not expose transaction states, scheduler types, or observer hooks in production just for integration tests. Remove production test env checks when Phase 13C/13E no longer require them.

## 4. Workstream B — Consolidate CLI organization

### 4.1 Target grouping

The core top-level commands remain prominent:

```text
snp new
snp run
snp clip
snp search
snp list
snp edit
snp get
```

Existing grouped domains remain or become clearer:

```text
snp library ...
snp sync ...
snp premade ...
snp shell ...
```

Advanced local data maintenance may be grouped:

```text
snp data validate
snp data backup
snp data restore
snp data repair
snp data status
```

The exact group name may be `data`, `maintenance`, or another short existing-style term. Prefer `data` unless it conflicts with current semantics.

Existing top-level commands (`snp validate`, `backup`, `restore`, `repair`, `status`) must remain functional compatibility aliases during Phase 13. They may be hidden from primary help only after:

- grouped equivalents exist;
- completion generation includes canonical forms;
- docs use canonical forms;
- compatibility tests prove identical exit codes/output behavior;
- no ambiguity is introduced.

### 4.2 Doctor boundaries

`doctor` should diagnose environment/config/sync issues. It should not duplicate validate/repair implementation.

Required cleanup:

- delegate data validation to the same underlying operation used by `data validate`;
- delegate sync status diagnostics to the compact Phase 13E status model;
- keep report formats stable;
- avoid adding new diagnostic categories solely to make grouping comprehensive.

### 4.3 Dispatch simplification

The current `main.rs` contains substantial per-command branching and repeated exact-selector/output mapping.

Allow bounded consolidation through:

- command-specific option structs;
- small dispatch functions per command group;
- shared outcome-to-exit-code mapping;
- canonical selector construction helpers.

Do not introduce a command registry, trait-object dispatcher, macro DSL, or generated CLI schema. Clap enums and direct functions remain appropriate.

A successful result should reduce `main.rs` size and repeated branches, not move the same complexity into another giant module.

## 5. Workstream C — Bound the `snip-sync` administration surface

### 5.1 Core supported server commands

Keep:

- `serve`;
- `init`;
- `paths`;
- `edit`;
- `version`;
- completion generation if low cost;
- `croncheck` as a documented fallback for hosts without a supervisor.

### 5.2 Lifecycle compatibility

Existing `stop` and `restart` commands remain in Phase 13 for compatibility. Documentation should make clear:

- systemd/launchd/Docker are preferred supervisors;
- `stop`/`restart` are convenience commands for locally managed instances;
- the server does not install, enable, or manage system services;
- PID records are metadata under a kernel-backed singleton lock;
- `croncheck` is not a production supervisor.

Do not add `start --daemon`, service installation, log rotation, privilege dropping, or automatic restart policies.

Review whether `restart` should simply invoke the existing stop path then run foreground `serve`; retain this simple behavior.

### 5.3 Certificate command

The server does not terminate TLS. `cert` creates development assets for a reverse proxy.

Keep it only if documentation is unambiguous. Consolidate wording across README/help:

- development/reverse-proxy experiment only;
- not read by `snip-sync` itself;
- real deployments use a trusted CA at the proxy.

Do not expand certificate management.

### 5.4 Update command

Keep self-update only for Cargo-managed installations if that is the current supported behavior. Avoid duplicating client standalone/Homebrew update complexity in the server.

Document unsupported methods clearly. Do not add Docker image updating or service restart automation.

## 6. Workstream D — Optional server facilities

Review non-core facilities:

- Prometheus metrics;
- persistent rate limiting;
- CORS support;
- trusted proxy handling;
- HTTP security headers.

The goal is not to remove security or observability reflexively. Classify each:

| Facility | Needed for core gRPC loopback/LAN sync? | Runtime/dependency cost | Maintenance/test cost | Target |
|---|---|---:|---:|---|

### Decision rules

- health endpoint remains core because cron/supervisors use it;
- basic in-memory rate limiting for registration/auth may remain core if small and security-relevant;
- metrics may be default-off or feature-gated if Prometheus is a meaningful server binary/dependency contributor;
- persistent rate limiting may be default-off or removed if in-memory limits are adequate for intended local/LAN deployment and persistence adds tasks/schema/testing complexity;
- CORS is only relevant to browser HTTP endpoints; since current HTTP endpoints are health/metrics, keep the simplest safe behavior or feature-gate metrics-related CORS with metrics;
- trusted proxy handling remains only if registration rate limiting actually consumes proxy-forwarded identity in documented reverse-proxy deployments;
- security headers are small and may remain, but do not treat them as production hardening evidence.

Feature gating is acceptable only when:

- default build remains the intended local deployment;
- enabling the feature is straightforward and documented;
- code paths do not fragment into many combinations;
- Phase 13C CI does not add a matrix for combinations;
- measurement shows worthwhile binary or maintenance reduction.

At most one optional server feature group should be introduced, for example `metrics`. Do not create a feature per facility.

## 7. Workstream E — Documentation consolidation

### 7.1 Remove volatile line numbers

Replace references such as `module.rs:123-145` with:

- symbol names;
- type/function names;
- invariant descriptions;
- relative file paths only where useful.

Line numbers may appear in a one-time review note but not normative architecture guidance.

### 7.2 Reduce duplication

Review:

- `AGENTS.md`;
- `AGENTS.override.md`;
- `architecture/*.md`;
- `.skills/*.md`;
- `docs/*.md`;
- README and USER_GUIDE.

Choose one authoritative home per topic:

- user behavior/configuration: README/USER_GUIDE;
- contributor commands/gotchas: AGENTS.md;
- architecture invariants: architecture docs;
- threat/security boundaries: security docs;
- historical plans: plans directory only.

Remove phase-history narratives from current architecture docs when they no longer explain present behavior. Do not delete plans or completion records.

### 7.3 Documentation count constraint

Do not add new documentation files in this phase unless one replaces at least two existing overlapping files. Prefer editing, merging, and deleting stale content.

Generated/agent skill documents should not duplicate the full architecture index with line numbers.

## 8. Workstream F — Compatibility and deprecation policy

Phase 13 should not introduce hard removals.

For CLI aliases:

- canonical grouped command is shown in docs/help;
- old spelling remains accepted;
- avoid warnings on every invocation unless a future removal is actually scheduled;
- completions may include both or canonical only if old spelling remains parseable;
- machine output and exit codes are identical.

For Rust API:

- retain inexpensive re-exports when immediate removal would break documented consumers;
- mark implementation-only modules `#[doc(hidden)]` or document them as unstable before removal;
- use normal semver for any later actual removal;
- do not announce a deprecation timeline without intent to execute it.

For server features:

- current default config files continue to parse;
- disabled optional sections are ignored or diagnosed clearly, not silently changed;
- existing environment variables retain behavior or receive explicit migration guidance.

## 9. Likely files

Client/API:

- `src/lib.rs`
- `src/main.rs`
- `src/outcome.rs`
- command modules and selector helpers
- `Cargo.toml` docs.rs/public metadata if needed

Server:

- `snip-sync/src/cli.rs`
- `snip-sync/src/main.rs`
- `snip-sync/src/lib.rs`
- `snip-sync/Cargo.toml` only for one measured optional feature group
- server metrics/rate-limiter/CORS modules

Docs/tests:

- `README.md`
- `USER_GUIDE.md`
- `snip-sync/README.md`
- `AGENTS.md`
- `AGENTS.override.md`
- `architecture/*.md`
- `.skills/*.md`
- `docs/PUBLIC_API.md`
- CLI compatibility and public API tests

Do not modify sync protocol, encryption format, transaction recovery semantics, update archive format, or CI topology here.

## 10. Implementation order

### Pass 1 — Inventory and contract declaration

1. inventory public Rust items and consumers;
2. inventory top-level commands and documentation references;
3. inventory server facilities and costs;
4. identify duplicated documentation authorities;
5. write the target supported API/surface table in this plan before edits.

### Pass 2 — Rust API boundary

1. add explicit supported API documentation;
2. narrow or hide implementation-only modules;
3. add narrow test-support access where genuinely required;
4. update docs.rs/public API docs;
5. run compatibility compile examples.

### Pass 3 — CLI grouping and dispatch

1. add canonical advanced data group;
2. retain old spellings as aliases;
3. consolidate shared dispatch/outcome code;
4. update help/completions/docs;
5. test byte/output/exit compatibility.

### Pass 4 — Server scope

1. clarify lifecycle/cert/update behavior;
2. measure optional facilities;
3. gate or simplify at most one coherent optional group if justified;
4. retain config compatibility;
5. avoid new lifecycle features.

### Pass 5 — Documentation consolidation

1. remove line-number references;
2. merge duplicate explanations;
3. remove stale phase-history claims from current docs;
4. keep historical plans intact;
5. record file/line reduction descriptively.

## 11. Focused tests

### Rust API

- documented examples compile;
- supported types/functions are reachable from crate root or documented modules;
- normal production build does not expose test-support internals;
- binary still compiles against the narrowed facade;
- docs.rs configuration succeeds through `cargo doc --no-deps`.

### CLI

For each regrouped command:

- canonical and legacy spelling produce identical stdout/stderr bytes where deterministic;
- exit codes match;
- JSON schemas match;
- dry-run/mutation behavior matches;
- help shows canonical organization;
- no command executes snippets unless it is an execution command.

Do not create one test per flag combination if existing command tests already cover behavior. Add a compact alias equivalence table.

### Server

- existing config parses with default feature set;
- health and core gRPC sync remain available;
- optional facility enabled/disabled behavior is explicit if gating is introduced;
- stop/restart compatibility remains;
- no command installs or manages services.

### Documentation

Use repository search, not a committed checker, to confirm:

- no normative architecture line-number references remain;
- no deleted module/command/test target is referenced;
- current docs do not describe removed Phase 13E state machinery;
- one authoritative link exists per topic.

## 12. Verification commands

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
cargo test --test platform_smoke --features test-support -- --test-threads=1
cargo test --test <cli-alias-equivalence-target>
cargo doc -p snip-it --no-deps
cargo check -p snip-sync --all-targets
bash scripts/check.sh
```

If one optional server feature group is introduced, run one explicit enabled build/test command locally. Do not add a CI matrix.

## 13. Acceptance criteria

- [ ] Supported Rust API is explicitly documented.
- [ ] Implementation-only modules are private, hidden, or clearly unstable rather than accidentally promised stable.
- [ ] Integration tests no longer require broad production visibility solely for test access where narrow seams suffice.
- [ ] Core CLI commands remain prominent.
- [ ] Advanced maintenance commands have a coherent canonical group.
- [ ] Every existing command spelling remains functional in Phase 13.
- [ ] Alias/canonical commands preserve outputs, exit codes, and mutation semantics.
- [ ] `main.rs`/dispatch repetition is reduced without a registry, macro DSL, or trait framework.
- [ ] Server commands remain bounded convenience operations, not a service manager.
- [ ] Health and encrypted sync remain core.
- [ ] At most one measured optional server feature group is added, or none if measurement does not justify it.
- [ ] Existing server configuration remains compatible or has explicit migration guidance.
- [ ] Architecture docs use symbols/invariants rather than volatile line numbers.
- [ ] Documentation duplication and stale phase narratives are reduced.
- [ ] No existing feature, platform, installation path, command, or deployment workflow is removed.
- [ ] No plugin system, IPC, admin API, web UI, service installer, or CI matrix is introduced.
- [ ] `bash scripts/check.sh`, docs build, and platform smoke pass.

## 14. Completion measurements

Record:

| Metric | Before | After |
|---|---:|---:|
| public modules from `snip-it` crate root | | |
| explicitly supported API items | | |
| top-level visible CLI commands | | |
| compatibility aliases | | |
| `src/main.rs` LOC | | |
| server default dependencies | | |
| architecture/skill documentation LOC | | |
| normative line-number references | | |

These are descriptive and must not become gates.

## 15. Stop conditions

Stop and amend the plan if:

- API narrowing requires an unplanned semver-major break;
- CLI grouping would remove or materially change existing commands;
- server feature gating creates multiple build combinations requiring CI matrices;
- dispatch cleanup introduces a framework larger than direct Clap matching;
- documentation consolidation begins deleting unique user guidance or security boundaries;
- a server convenience command expands into service installation/supervision;
- scope drifts into protocol, storage format, encryption, or new product features.

The phase should leave fewer promises, clearer commands, and less duplicated documentation—not a new abstraction layer.

## 16. Completion record

Status: COMPLETE

Implementation commit: `01a860b` — Phase 13F: API, CLI, and documentation surface consolidation

Corrective commit: `5d37fa7` — Phase 13G: Fix sync batching, server shutdown, and config validation

Verification:
- `bash scripts/check.sh`: PASS

Acceptance criteria: All items satisfied. Implementation-only modules `#[doc(hidden)]`, supported API documented, `data` subcommand group added, legacy aliases retained, architecture docs cleaned.

Release-blocking: No (cleared by 13G)
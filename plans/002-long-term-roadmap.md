# snip-it Long-Term Implementation Roadmap

Status: execution roadmap for `plans/000-long-term-specification.md`

Terminology: `plans/001-terminology-and-domain-model.md`

This roadmap orders the work needed to keep snip-it lightweight, durable,
and fleet-deployable while preserving a useful local-only product at every
stage. Each phase MUST leave the repository in a coherent state and MUST
include focused implementation plans, migrations, tests, documentation, and
closure evidence before the next dependent phase is treated as available.

The roadmap is dependency-ordered, not calendar-ordered. Parallel work is
appropriate only where the dependency notes allow it.

## Cross-phase execution rules

Every phase MUST:

1. preserve snip-it as a lightweight terminal tool (no new workspace
   crates, service abstractions, DI frameworks, plugin/rule engines,
   databases/search indexes, generic query languages, background job
   systems, networked MCP, MCP mutations/execution, retry middleware, or
   additional CI/release hardening unless a later roadmap revision
   explicitly authorizes it);
2. preserve existing command spellings, aliases, exit codes, and
   machine-output rules unless a semver-compatible deprecation path is
   included;
3. run `gate_mutation_on_interrupted_transactions()` before every local
   mutation and keep kernel locks authoritative;
4. keep conflict identity `(updated_at, device_id, SHA-256(synced fields))`
   with deletion-beats-live and local-only exclusions intact;
5. keep `selector::searchable_text` as the shared search projection;
6. maintain backward-compatible migrations or document an intentional break;
7. include restart, cancellation, contention, and corruption tests where
   applicable;
8. update `architecture/` docs and static guards with code;
9. leave local-only mode operational without sync/server/MCP configuration;
10. record explicit exit evidence in the implementation plan or closure
   record.

## Phase 0 — Distribution, fleet deployment, and release closure

### Objective

Make snip-it deployable across a small heterogeneous fleet from prebuilt
binaries while keeping publishing manual and releases immutable.

### Deliverables

- Release binary matrix and artifact contract (independent tags, stable
  asset names, checksums).
- Binary-first bootstrap installers with exact-version Cargo fallback and
  hard integrity failure.
- Windows CI and platform closure plus release publication and
  end-to-end distribution evidence.
- Corrective closure for binary publication/installer gaps without a
  second release workflow or matrix duplication.

### Dependencies

None beyond the current `snp`/`snip-sync` baseline.

### Exit criteria

- All five public `snp` binaries plus checksums publish under the next
  legitimate `snip-it` release; historical releases stay untouched.
- Consumer smoke covers Linux x86_64/ARM64, macOS Intel/Apple Silicon, and
  Windows x86_64 plus the exact README bootstrap.
- Bash/PowerShell installer failure-mode contracts are covered
  deterministically.

### Required tests

- Release-matrix and artifact-contract checks.
- Installer failure-mode fixtures (checksum mismatch, wrong version,
  malformed release data, TLS/5xx hard failure, no silent `sudo`).
- Consumer smoke across the five targets.
- `scripts/check.sh` plus production-seam checks.

Subsystem: `plans/subsystems/distribution-release-roadmap.md`.

## Phase 1 — Server lifecycle and lean HTTP runtime

### Objective

Keep `snip-sync` a single foreground binary with reusable lifecycle
primitives and a minimal, wire-compatible HTTP leaf service.

### Deliverables

- `serve`/`stop`/`restart`/`croncheck`/`/health` lifecycle plus singleton
  lock and process identity checks.
- EggServe runtime adoption decided by measurement (Axum-preserving trial
  vs direct leaf), with the winner locked by socket-level parity tests.
- CORS `Vary`, router 404/405, preflight `Allow`, and security-header
  boundary preserved exactly.

### Dependencies

Phase 0 artifact contract (binary-size gates need a stable baseline).

### Exit criteria

- Exactly one HTTP production architecture remains after the A/B/C
  comparison, within the 10% footprint gate.
- `tests/snip_sync_lifetime.rs` parity assertions pass on real sockets.
- No Axum/Tower-HTTP/core direct dependencies remain unless the winning
  architecture requires them.

### Required tests

- Lifecycle start/stop/restart/crash/recovery tests.
- Real-socket 404/405, preflight, `Vary`, and security-header assertions.
- Same-toolchain release-size A/B/C measurements.
- `scripts/check.sh` plus hosted platform smoke.

Subsystem: `plans/subsystems/server-lifecycle-http-roadmap.md`.

## Phase 2 — Binary-first self-update and lean transport

### Objective

Move `snp update` to a minimal in-process HTTP transport while keeping
`snip-sync` lightweight and redirect/timeout behavior correct.

### Deliverables

- Binary-first self-update and restart integration.
- `eggfetch-core` consolidation on the lean profile (`standard-http1` +
  `redirects` + `tls-rustls` + `tls-native-roots`) with delegated strict
  redirects and native `Timeout.total`.
- End-to-end timeout corrective: one injected overall deadline around
  traversal plus complete final-body consumption, with partial-file cleanup
  and slow-drip regression tests.
- Version-adoption passes (0.1.7 lean, 0.2.0) plus a controlled,
  measurement-only `snip-sync` trial that does not retain temporary
  transport code when it fails the 10% gate.

### Dependencies

Phase 0 release identity (updater constructs exact tags; never GitHub
`latest`).

### Exit criteria

- `snp` pins the coordinated lean `eggfetch-core` release with no manual
  redirect machine and no duplicate outer timeout.
- `snip-sync` stays on `curl` unless a fresh trial proves <=10% growth.
- `tests/architecture.rs` transport pins stay green.

### Required tests

- Updater metadata/binary slow-drip timeout tests.
- Partial-staging-file cleanup on timeout/cancellation.
- Lean-profile feature/size measurements.
- Focused updater suites plus `scripts/check.sh`.

Subsystem: `plans/subsystems/updater-transport-roadmap.md`.

## Phase 3 — CLI, library, selector, and sync-policy consolidation

### Objective

Remove overlapping policy and deepen existing retrieval/search behavior
before any broad feature expansion. Deletion/consolidation first.

### Deliverables

- Single CLI schema and outcome path per semantic command.
- Side-effect-free shared library resolution plus narrowly shared
  inspection primitives.
- Hotspot file splits along existing responsibility boundaries only.
- Canonical selector/search reuse across CLI and read-only MCP.
- Deduplicated sync retry policy behind the single
  `retry_grpc_unified!` macro.

### Dependencies

None beyond the Phase 0 command/exit-code contract; must not disturb
Phases 0-2 transport or lifecycle behavior.

### Exit criteria

- Duplicate Clap schemas and redundant outcome translations are gone with
  help output and exit codes unchanged.
- New read paths use `resolve_selector_readonly` plus
  `inspect_library_index`.
- No new crates, frameworks, query languages, job systems, or MCP
  mutations are introduced.

### Required tests

- `--help` compatibility captures.
- Readonly-vs-mutating resolver parity tests.
- Selector/search/MCP read-parity fixtures.
- Retry-policy unit plus multi-batch error-kind preservation tests.

Subsystem: `plans/subsystems/cli-library-sync-consolidation-roadmap.md`.

## Phase 4 — Local MCP server and client registration

### Objective

Expose snippet search/get to agent clients through a local stdio adapter
without creating another service or execution surface.

### Deliverables

- Read-only MCP server (`snippets_search`, `snippet_get`) sharing
  `selector::searchable_text` semantics.
- Explicit client registration flow.
- Search-parity and credential-exclusion coverage.

### Dependencies

Phase 3 selector/search parity (MCP MUST reuse the canonical projection).

### Exit criteria

- MCP search/get results match CLI search semantics including the
  output/notes opt-in and the never-searchable exclusions.
- No MCP mutations, execution tools, networked MCP, or daemonized MCP
  service exist.

### Required tests

- MCP search-parity fixtures.
- `snippet_get` argument-shape tests (exactly one of ID/description/command).
- Client-registration tests with isolated config.

Subsystem: `plans/subsystems/mcp-integration-roadmap.md`.

## Phase ordering

```text
Phase 0 distribution/release closure
    |
    +--> Phase 1 server lifecycle + HTTP runtime (needs artifact baseline)
    |
    +--> Phase 2 self-update + lean transport (needs release identity)
    |
    `--> Phase 3 CLI/library/selector/retry consolidation (independent baseline)
              |
              `--> Phase 4 local MCP (needs Phase 3 search parity)
```

Phases 1 and 2 may proceed in parallel once Phase 0 closes. Phase 4 MUST
NOT begin MCP search work before Phase 3 selector parity closes; client
registration scaffolding MAY proceed against the stable selector contract
as an interface dependency.

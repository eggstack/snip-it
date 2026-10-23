# CLI, Library, Selector, and Sync-Policy Consolidation Roadmap

Status: complete

> **Roadmap reopened 2026-10-22.** M001–M005 remain closed and their history
> is unchanged. The roadmap is reopened for one corrective milestone, M006,
> which closes a verification gap in M004's search-parity contract: the
> parity fixtures compare `snp list --filter` against MCP
> `snippets_search` but never exercise `snp get --query`, and the fixture
> target does not run in `scripts/check.sh` or in CI. The reason for the
> change is recorded in
> `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md`
> and in M006 below. No completed milestone status is altered.

> **Roadmap closed 2026-10-22.** M006 closed the verification gap: the
> parity suite now exercises all three surfaces and runs in
> `scripts/check.sh`, and the two behaviors M004's contract left open — the
> empty-query rule and the cross-library order — are decided, implemented,
> and pinned by tests. The closure record is
> `plans/closure/cli-library-sync-consolidation/006-status.md`. M001–M005
> remain closed and their history is unchanged.

Long-term references:

- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#7-search-selector-and-mcp-invariants`
- `plans/001-terminology-and-domain-model.md#6-selection-and-outcome-terms`
- `plans/002-long-term-roadmap.md#phase-3-cli-library-selector-and-sync-policy-consolidation`
- `plans/003-planning-process.md`

Related ADRs: none required. This work consolidates inside existing
ownership boundaries. If a future change alters exit codes, search
semantics, conflict identity, or retry ownership, open an ADR.

## 1. Purpose and ownership boundary

This subsystem owns CLI/outcome deduplication, shared read-only library
resolution, hotspot decomposition along existing boundaries, canonical
selector/search reuse, and sync retry deduplication. It consumes the
command/exit-code contract, the transaction/lock hierarchy, and the single
retry macro. It MUST NOT own distribution, updater transport, server
runtime, or MCP registration behavior beyond shared search parity.

## 2. Work classification

### Invariants

- Existing command spellings, aliases, exit codes, and machine-output
  rules unchanged.
- `resolve_selector_readonly` plus `inspect_library_index` for read paths.
- Conflict identity and local-only exclusions unchanged.
- Single `retry_grpc_unified!` ownership; `&mut self` reborrow preserved.

### Capabilities

- Unchanged user-visible CLI/search behavior with less duplicate policy.

### Infrastructure

- Shared inspection primitives, boundary-respecting module splits,
  deduplicated retry policy.

### Polish

- Reduced reasons for files to change; deletion over module-count growth.

## 3. Non-goals

- New workspace crates, service/repository abstractions, DI frameworks,
  plugin/rule engines, databases/search indexes, generic query languages,
  background job systems, networked MCP, MCP mutations/execution, retry
  middleware, or additional CI/release hardening.
- New feature commands, exit-code renumbers, or output-format changes.
- Combining outcomes into one mega-enum merely to reduce type count.

## 4. Current state

M001 through M005 are closed. Duplicate Clap schemas and redundant
outcome translations are removed with compatibility preserved.
Side-effect-free shared resolution exists and read-only MCP reuses the
canonical search projection. Hotspot files are split only along existing
responsibilities. Sync retry policy is deduplicated behind the single
macro with batch error-kind preservation.

M006 is closed. The M004 implementation satisfied its stated requirement —
the three surfaces share the canonical `selector::searchable_text`
projection — but the evidence accepted for it did not cover every surface
the contract names, and the fixture target was not connected to the build
gate. M006 closed both gaps: the three surfaces now share one match
primitive (`selector::score_fuzzy_matches`), the parity suite exercises all
three surfaces and runs in `scripts/check.sh`, and the two behaviors the
contract left open are decided and pinned — an empty query is "no filter" on
all three surfaces, and the cross-library order differs by surface family on
purpose. The closure record is
`plans/closure/cli-library-sync-consolidation/006-status.md`.

## 5. Target architecture

One canonical `*Args` plus one `handle_*` per semantic command (`snp data`
as alias); `CliOutcome` as the single exit-code path; mutating
`resolve_selector` vs side-effect-free `resolve_selector_readonly`
observed by all new read paths; `SearchFields`/`searchable_text` shared by
CLI and MCP; one inline retry macro for all gRPC RPCs.

## 6. Dependency graph

```text
M001 CLI surface + outcome
    |
    +--> M002 shared inspection + readonly resolution (hard: M001)
    |         |
    |         +--> M003 module boundary + hotspot split (hard: M001, M002)
    |                   |
    |                   +--> M004 selector/search/MCP parity (hard: M002, M003)
    |                             |
    |                             `--> M005 retry dedup (hard: M001-M004 soft ordering)
    |
    `--> M006 search parity verification corrective (hard: M004)
```

M001 first (deletion/consolidation order). M005 is a final internal
cleanup with soft ordering on M002-M004 but hard on M001.

M006 is a corrective pass over M004 and depends only on M004. It is
sequenced last because it corrects the verification of an already-closed
milestone rather than the milestone's implementation, and because the
decisions it must settle — the empty-query semantics and the cross-library
ordering semantics — are best made against the settled code that M002 and
M003 left behind.

## 7. Milestones

### M001 — CLI surface and outcome consolidation

Class: infrastructure

Objective: remove duplicated CLI schemas and redundant outcome
translations without behavior change.

Dependencies: none (command contract is the baseline).

Deliverable boundary: one source of truth per duplicated command family;
simplified result path preserving TUI-vs-CLI distinctions that carry
unique state.

User or operator value: fewer drift sites; identical help/exit behavior.

Exit conditions: `--help` captures unchanged; exit codes and
machine-output rules unchanged.

Deferred work: shared inspection (M002).

Implementation plan:

- `plans/implementation/cli-library-sync-consolidation/001-cli-surface-and-outcome-consolidation.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/001-status.md`

Predecessor flat plan: `plans/archive/flat-008-cli-surface-and-outcome-consolidation.md`.

### M002 — Shared inspection and read-only library resolution

Class: infrastructure + invariant

Objective: establish side-effect-free shared resolution and narrowly
shared inspection primitives.

Dependencies: hard M001.

Deliverable boundary: readonly resolver plus `inspect_library_index`;
mutating paths unchanged.

User or operator value: safe read paths for CLI inspection and MCP.

Exit conditions: new read paths use the readonly resolver; noWritable
synthesis on corrupt inputs (fail closed).

Deferred work: hotspot splits (M003).

Implementation plan:

- `plans/implementation/cli-library-sync-consolidation/002-shared-inspection-and-readonly-library-resolution.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/002-status.md`

Predecessor flat plan: `plans/archive/flat-009-shared-inspection-and-readonly-library-resolution.md`.

### M003 — Module boundary and hotspot decomposition

Class: infrastructure

Objective: split large hotspot files only along existing responsibility
boundaries.

Dependencies: hard M001, M002.

Deliverable boundary: splits plus re-export updates (`ui/mod.rs`,
`commands/mod.rs`); no behavior redesign.

User or operator value: smaller reasons for files to change.

Exit conditions: refactor succeeds by deleted duplicate policy, not by
module count.

Deferred work: selector parity (M004).

Implementation plan:

- `plans/implementation/cli-library-sync-consolidation/003-module-boundary-and-hotspot-decomposition.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/003-status.md`

Predecessor flat plan: `plans/archive/flat-010-module-boundary-and-hotspot-decomposition.md`.

### M004 — Selector, search, and MCP read parity

Class: invariant + infrastructure

Objective: reuse canonical selector/search semantics in CLI and read-only
MCP.

Dependencies: hard M002, M003.

Deliverable boundary: shared `searchable_text` semantics including the
output/notes opt-in and never-searchable exclusions.

User or operator value: identical search results across `get`, `list`,
and MCP.

Exit conditions: parity fixtures green; folders/favorite/sync
metadata/credentials never searchable.

Deferred work: retry dedup (M005).

Implementation plan:

- `plans/implementation/cli-library-sync-consolidation/004-selector-search-and-mcp-read-parity.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/004-status.md`

Predecessor flat plan: `plans/archive/flat-011-selector-search-and-mcp-read-parity.md`.

### M005 — Sync retry policy deduplication

Class: infrastructure

Objective: deduplicate retry policy as a final internal cleanup behind the
single macro.

Dependencies: hard M001; soft M002-M004.

Deliverable boundary: one macro plus `RetryBackoff`; `&mut self` reborrow
preserved; no closure-based helper; batch error-kind preservation via
`add_batch_context()`.

User or operator value: consistent retry behavior with less policy code.

Exit conditions: all RPCs route through the unified macro; multi-batch
`SyncFailureKind` preserved.

Deferred work: none; workstream closed.

Implementation plan:

- `plans/implementation/cli-library-sync-consolidation/005-sync-retry-policy-deduplication.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/005-status.md`

Predecessor flat plan: `plans/archive/flat-012-sync-retry-policy-deduplication.md`.

### M006 — Search parity verification corrective

Class: invariant

Objective: close the verification gap in M004's search-parity contract by
proving that `snp get --query`, `snp list --filter`, and MCP
`snippets_search` share one search definition, and by making that proof
gate the build.

Dependencies: hard M004.

Deliverable boundary: parity fixtures covering the third surface the
contract names; the parity target added to `scripts/check.sh`; the
empty-query and cross-library ordering semantics decided, implemented so
the surfaces agree, documented, and pinned by tests.

User or operator value: the cross-surface search contract is enforced by
the build rather than asserted in a document, so a field added to one
surface and forgotten in another fails the gate.

Exit conditions: all three surfaces are compared against one fixture; the
parity target runs in `scripts/check.sh`; the empty-query and cross-library
semantics are decided and pinned; the existing nine parity cases still
pass; the field set and the non-searchable exclusions are unchanged.

Deferred work: none within this subsystem.

Corrective plan:

- `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md`

Closure record:

- `plans/closure/cli-library-sync-consolidation/006-status.md`

Predecessor flat plan: none.

## 8. Cross-cutting requirements

### Storage and migration

No schema migration; transaction gates and lock hierarchy untouched.

### Protocol and compatibility

Command spellings, aliases, exit codes, machine-output formats, and sync
wire behavior unchanged.

### Security and authorization

No credential or sync-metadata leakage into search; clipboard only via
`copy_to_clipboard()`.

### Concurrency, cancellation, and recovery

Resolver choice (mutating vs readonly) respected under concurrency;
retry/backoff bounded.

### Observability and audit

Exact-count test assertions; server-side-effect proofs; pending-clear
ordering checks.

### Performance and resource use

No new runtime systems; prefer concrete structs and ordinary functions
over traits or generalized infrastructure.

### Documentation and operations

`architecture/cli.md`, selector/library docs, and command contracts
updated with code.

## 9. Verification strategy

Help-compatibility captures, readonly/mutating resolver parity tests,
search/MCP parity fixtures, retry unit plus multi-batch kind-preservation
tests, `scripts/check.sh`, production-seam checks.

## 10. Risks and decision points

No open decisions. Any future CLI redesign, search-semantic change, or
retry-framework introduction requires a new milestone and, if
contract-crossing, an ADR.

## 11. Completion definition

This workstream closes when M001-M006 each have accepted closure evidence
with duplicate policy deleted, readonly resolution adopted, boundaries
respected, parity proven and enforced by the build gate, retry unified,
and no unresolved medium-or-higher finding remains. M001-M006 are all closed
and no medium-or-higher finding remains, so the roadmap is complete.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 CLI surface + outcome | closed | `plans/implementation/cli-library-sync-consolidation/001-cli-surface-and-outcome-consolidation.md` | `plans/closure/cli-library-sync-consolidation/001-status.md` | — |
| M002 shared inspection + readonly | closed | `plans/implementation/cli-library-sync-consolidation/002-shared-inspection-and-readonly-library-resolution.md` | `plans/closure/cli-library-sync-consolidation/002-status.md` | — |
| M003 module boundary + hotspots | closed | `plans/implementation/cli-library-sync-consolidation/003-module-boundary-and-hotspot-decomposition.md` | `plans/closure/cli-library-sync-consolidation/003-status.md` | — |
| M004 selector/search/MCP parity | closed | `plans/implementation/cli-library-sync-consolidation/004-selector-search-and-mcp-read-parity.md` | `plans/closure/cli-library-sync-consolidation/004-status.md` | — |
| M005 retry dedup | closed | `plans/implementation/cli-library-sync-consolidation/005-sync-retry-policy-deduplication.md` | `plans/closure/cli-library-sync-consolidation/005-status.md` | — |
| M006 search parity verification corrective | closed | `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md` | `plans/closure/cli-library-sync-consolidation/006-status.md` | — |

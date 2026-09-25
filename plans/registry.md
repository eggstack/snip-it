# snip-it Active Planning Registry

This file is the compact control surface for active interim planning.
Detailed requirements and completed history remain in source roadmaps,
implementation plans, `plans/closure/`, and Git history.

Canonical direction remains in:

- `plans/000-long-term-specification.md`
- `plans/001-terminology-and-domain-model.md`
- `plans/002-long-term-roadmap.md`
- `plans/003-planning-process.md`

Planning-convention migration: the pre-convention flat sequence (plans
000–020) was promoted via `git mv` into
`plans/implementation/<subsystem>/` (history preserved; bodies unchanged)
with predecessor pointers in `plans/archive/flat-*.md` and completion
gates in `plans/closure/<subsystem>/`. Pre-convention `Status: complete`
maps to `closed`.

## Status vocabulary

- **proposed** — roadmap or plan exists but is not approved for execution.
- **ready** — dependencies and interfaces are satisfied; plan may be handed off.
- **active** — implementation or closure work is in progress.
- **blocked** — a named dependency or evidence requirement prevents progress.
- **closing** — implementation landed and closure evidence is being gathered.
- **closed** — closure record accepted.
- **conditionally closed** — substantial work landed, but a named correctness or operational evidence condition remains.
- **superseded** — replaced by another document.
- **archived** — no longer active and retained for traceability.

## Active subsystem roadmaps

| Subsystem | Status | Roadmap | Current milestone | Dependencies or blockers |
|---|---|---|---|---|
| Distribution and release | closed | `plans/subsystems/distribution-release-roadmap.md` | M001-M005 closed | None. Corrective M005 closed without a second workflow or matrix duplication. |
| Server lifecycle and HTTP runtime | closed | `plans/subsystems/server-lifecycle-http-roadmap.md` | M001-M004 closed | None. Direct EggServe leaf (C) is the final architecture; parity socket-proven. |
| Updater transport | closed | `plans/subsystems/updater-transport-roadmap.md` | M001-M005 closed | None. `snp` on lean `eggfetch-core 0.2.0`; `snip-sync` on `curl` by measurement. |
| CLI, library, selector, sync-policy consolidation | closed | `plans/subsystems/cli-library-sync-consolidation-roadmap.md` | M001-M005 closed | None. Duplicate policy deleted; parity and retry unified. |
| Local MCP integration | closed | `plans/subsystems/mcp-integration-roadmap.md` | M001 closed | None. Read-only stdio adapter with search parity and registration. |

## Dependency-ready implementation plans

None. All registered milestones are closed. Register new work here only
after dependency and handoff review per `plans/003-planning-process.md`
§10.

## Current execution order and dependency gates

**Distribution gate:** M001-M004 closed the matrix, installers, platform
gate, and publication evidence. Corrective M005 is closed
(`snip-it 1.3.8` five binaries plus checksums; `v1.3.7` untouched;
five-target consumer smoke; deterministic installer contracts).

**Server gate:** M001 lifecycle baseline closed. M002 recorded the A/B
measurement (A 3,833,152; B 3,963,968). M003 selected C (3,898,408) as the
single remaining architecture. M004 closed parity (CORS `Vary`, empty
404/405, preflight `Allow`, header boundary) with socket tests plus the
M002/M003 evidence record (implementation `10aeb994`, hosted run
`36011491151`).

**Updater gate:** M001 self-update closed. M002 recorded the transport
split (`snp` lean eggfetch vs `snip-sync` curl). M003 closed the
end-to-end timeout corrective with slow-drip tests and partial-file
cleanup. M004 adopted lean 0.1.7 (-2.82%). M005 adopted 0.2.0 for `snp`
(byte-identical 6,776,320) and discarded the `snip-sync` trial (+34.23%).

**Consolidation gate:** M001-M005 closed CLI/outcome dedup, readonly
resolution, boundary-respecting splits, selector/search/MCP parity, and
retry unification with compatibility preserved.

**MCP gate:** M001 closed the read-only local adapter plus registration
against the canonical selector contract. No execution, mutation,
networked, or daemonized MCP surface exists.

## Blocked work

| Subsystem | Milestone | Blocker |
|---|---|---|
| — | — | No blocked work. |

## Closure work and current control points

| Subsystem | Status | Controlling evidence |
|---|---|---|
| Distribution and release M001-M005 | closed | `plans/closure/distribution-release/001-status.md` through `005-status.md`; implementation files under `plans/implementation/distribution-release/`; predecessors `plans/archive/flat-001-release-binary-matrix-and-artifact-contract.md`, `flat-002-bootstrap-installers.md`, `flat-006-windows-ci-platform-closure.md`, `flat-007-release-publication-and-distribution-closure.md`, `flat-013-snp-binary-publication-and-installer-corrective-closure.md` |
| Server lifecycle + HTTP M001-M004 | closed | `plans/closure/server-lifecycle-http/001-status.md` through `004-status.md`; implementation `10aeb994`, hosted run `36011491151`; predecessors `flat-003`/`flat-018`/`flat-019`/`flat-020` |
| Updater transport M001-M005 | closed | `plans/closure/updater-transport/001-status.md` through `005-status.md` (21 focused updater tests at adoption; byte-identical 6,776,320; server trial +34.23% discarded); predecessors `flat-004`/`flat-014`/`flat-015`/`flat-016`/`flat-017` |
| CLI/library/selector/retry M001-M005 | closed | `plans/closure/cli-library-sync-consolidation/001-status.md` through `005-status.md`; predecessors `flat-008`-`flat-012` |
| MCP integration M001 | closed | `plans/closure/mcp-integration/001-status.md`; predecessors `flat-005` plus MCP direction in `flat-000` |
| Pre-convention direction plan 000 | archived | `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md`, superseded by canonical `000`/`001`/`002` plus the `distribution-release` and `mcp-integration` roadmaps |

## Starting new work

1. Identify the relevant canonical sections and invariants.
2. Record any unresolved architectural decision in `plans/adrs/`.
3. Create or update the subsystem roadmap in `plans/subsystems/`.
4. Select one dependency-ready milestone and write a bounded handoff plan
   under `plans/implementation/<subsystem>/`.
5. Register it above (dependency-ready table) before handing it to an
   agent; move `ready` → `active` → `closing` → `closed` with a closure
   record each time.
6. On closure, audit blocked work: unblock dependents in the same commit
   and record the audit in the closure record.

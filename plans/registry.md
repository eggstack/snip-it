# snip-it Active Planning Registry

This file is the compact control surface for active interim planning. Detailed
requirements and completed history remain in source roadmaps, implementation
plans, closure records, and Git history.

Canonical direction remains in:

- `plans/000-long-term-specification.md`
- `plans/001-terminology-and-domain-model.md`
- `plans/002-long-term-roadmap.md`
- `plans/003-planning-process.md`

Planning-convention migration: pre-convention plans 000–020 were promoted to
`plans/implementation/<subsystem>/` with predecessor pointers in
`plans/archive/flat-*.md` and completion gates in `plans/closure/`. Pre-convention
`Status: complete` maps to `closed`.

## Status vocabulary

- **proposed** — exists but is not approved for execution.
- **ready** — dependencies and interfaces are satisfied; the plan may be handed off.
- **active** — implementation or closure work is in progress.
- **blocked** — a named dependency or evidence requirement prevents progress.
- **closing** — implementation landed and closure evidence is being gathered.
- **closed** — closure record accepted.
- **conditionally closed** — substantial work landed, but a named condition remains.
- **superseded** — replaced by another document.
- **archived** — no longer active; retained for traceability.
- **corrective pass required** — closure evidence is insufficient; a named pass must land.

## Subsystem roadmaps

| Subsystem | Status | Roadmap | Current milestone | Blockers |
|---|---|---|---|---|
| Distribution and release | closed | `plans/subsystems/distribution-release-roadmap.md` | M001–M005 closed | None |
| Server lifecycle and HTTP runtime | closed | `plans/subsystems/server-lifecycle-http-roadmap.md` | M001–M004 closed | None |
| Updater transport | closed | `plans/subsystems/updater-transport-roadmap.md` | M001–M005 closed | None |
| CLI, library, selector, sync-policy consolidation | complete | `plans/subsystems/cli-library-sync-consolidation-roadmap.md` | M001–M006 closed | None |
| Local MCP integration | closed | `plans/subsystems/mcp-integration-roadmap.md` | M001 closed | None |
| Eggstack dependency refresh | closed | `plans/subsystems/eggstack-dependency-refresh-roadmap.md` | M001 closed | None |
| Maintenance and duplicate-policy consolidation | closed | `plans/subsystems/maintenance-consolidation-roadmap.md` | M001–M003 closed | None |
| Locking protocol and ordering convergence | active | `plans/subsystems/locking-convergence-roadmap.md` | M001 closed; M002–M003 blocked | M002 design conflicts with the no-new-crates rule; M003 depends on M002 |
| Snippet CRUD surface | active | `plans/subsystems/snippet-crud-surface-roadmap.md` | M001–M002 closed; M003 blocked | Refresh the M003 handoff against M002's field-update primitive |
| Sync cursor protocol | closed | `plans/subsystems/sync-cursor-protocol-roadmap.md` | M001 condition discharged by M002; M002 closed | None |
| Security assurance | active | `plans/subsystems/security-assurance-roadmap.md` | M001–M005 closed (M002 conditionally); M006 blocked | M002 needs CI run evidence; M006 needs a published client that reads payload v2 |

## Open implementation plans

| Plan | Status | Blocker before handoff |
|---|---|---|
| `plans/implementation/locking-convergence/002-kernel-lock-protocol-convergence.md` | blocked | Revise the new-crate proposal to comply with the no-new-crates rule |
| `plans/implementation/locking-convergence/003-lock-ordering-contract-and-static-guard.md` | blocked | Hard dependency on M002; reconcile the draft's soft-dependency claim |
| `plans/implementation/snippet-crud-surface/003-non-interactive-field-update.md` | blocked | Refresh stale implementation evidence against M002's closed field-update primitive |
| `plans/implementation/security-assurance/002-ci-advisory-gates-and-action-pinning.md` | conditionally closed | CI run evidence (green job, red unsuppressed branch) must be recorded from GitHub Actions |
| `plans/implementation/security-assurance/006-enable-v2-payload-writes.md` | blocked | One published client release that reads payload v2, plus the minimum-client-version statement |

The security-assurance plan set responds to the 2026-11-17 security audit; the
audit's evidence and residual-risk dispositions remain in
`plans/subsystems/security-assurance-roadmap.md` § 4 and the closure records.
M001, M003, M004, and M005 are closed and therefore no longer listed above;
their evidence is in `plans/closure/security-assurance/`.

Register new work here only after dependency and handoff review per
`plans/003-planning-process.md` §10.

## Closure records and archive pointers

| Workstream | Closed milestones | Evidence |
|---|---|---|
| Distribution and release | M001–M005 | `plans/closure/distribution-release/001-status.md` through `005-status.md`; predecessor pointers are in `plans/archive/` |
| Server lifecycle and HTTP | M001–M004 | `plans/closure/server-lifecycle-http/001-status.md` through `004-status.md`; predecessors are in `plans/archive/` |
| Updater transport | M001–M005 | `plans/closure/updater-transport/001-status.md` through `005-status.md`; predecessors are in `plans/archive/` |
| CLI/library/selector/retry | M001–M006 | `plans/closure/cli-library-sync-consolidation/001-status.md` through `006-status.md` |
| Maintenance consolidation | M001–M003 | `plans/closure/maintenance-consolidation/001-status.md` through `003-status.md` |
| Locking convergence | M001 | `plans/closure/locking-convergence/001-status.md` |
| Snippet CRUD | M001–M002 | `plans/closure/snippet-crud-surface/001-status.md` and `002-status.md` |
| Sync cursor protocol | M001 condition discharged by M002; M002 closed | `plans/closure/sync-cursor-protocol/001-status.md`, `002-status.md`; accepted decision: `plans/adrs/ADR-0001-sync-delta-cursor.md` |
| MCP integration | M001 | `plans/closure/mcp-integration/001-status.md`; predecessor: `plans/archive/flat-005-local-mcp-server-and-client-registration.md` |
| Eggstack dependency refresh | M001 | `plans/closure/eggstack-dependency-refresh/001-status.md` |
| Pre-convention direction plan 000 | archived | `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md` |

## Starting new work

1. Identify the relevant canonical sections and invariants.
2. Record unresolved architectural decisions in `plans/adrs/`.
3. Create or update the subsystem roadmap in `plans/subsystems/`.
4. Select one dependency-ready milestone and write a bounded handoff plan under
   `plans/implementation/<subsystem>/`.
5. Register it above as `ready` only after dependency and handoff review. Move it
   through `active` → `closing` → `closed` with a closure record.
6. On closure, audit blocked work; unblock dependents in the same commit and
   record the audit in the closure record.

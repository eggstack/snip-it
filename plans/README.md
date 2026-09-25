# snip-it Planning System

This directory separates durable architectural direction from temporary
execution planning. It follows the codegg planning convention: canonical
long-term documents stay stable while interim plans evolve against the
repository baseline.

## Canonical long-term documents

The following files define the intended product and architecture and MUST
NOT be edited as part of ordinary implementation work:

- `000-long-term-specification.md` — normative end-state specification and invariants.
- `001-terminology-and-domain-model.md` — normative language and identity model.
- `002-long-term-roadmap.md` — dependency-ordered long-term capability roadmap.
- `003-planning-process.md` — rules for deriving and managing interim plans.

The first three documents are stable architectural references. Changes to
them require an explicit long-term architecture decision, not an
implementation convenience. Interim plans MUST reference them rather than
copying or silently revising their requirements.

## Planning hierarchy

```text
Long-term specification and terminology
        |
        v
Architecture decision records
        |
        v
Master long-term roadmap
        |
        v
Subsystem roadmaps
        |
        v
Milestone implementation plans
        |
        v
Implementation and verification
        |
        v
Closure records and archive
```

## Directory roles

- `adrs/` — durable architecture decisions. Accepted decisions are superseded, not rewritten.
- `subsystems/` — subsystem specifications and dependency-ordered roadmaps. These translate the long-term documents into coherent workstreams.
- `implementation/` — focused milestone plans handed to implementation agents. These are operational and may evolve as code changes.
- `closure/` — verification, evidence, residual-risk, and completion records for implemented milestones.
- `archive/` — completed or superseded interim planning retained for traceability. Pre-convention flat plans live here as full content (`flat-000`) or predecessor pointers (`flat-001`-`flat-020`; bodies promoted to `implementation/`).
- `registry.md` — compact index of active subsystem roadmaps, implementation plans, closure work, and dependencies. Check it before assuming any roadmap state.

## Subsystem workstreams

| Subsystem | Roadmap | Milestones | Status |
|---|---|---|---|
| Distribution and release | `subsystems/distribution-release-roadmap.md` | M001-M005 (matrix, installers, platform, publication, corrective) | closed |
| Server lifecycle and HTTP runtime | `subsystems/server-lifecycle-http-roadmap.md` | M001-M004 (lifecycle, Axum trial, leaf, parity) | closed |
| Updater transport | `subsystems/updater-transport-roadmap.md` | M001-M005 (self-update, consolidation, timeout, 0.1.7, 0.2.0) | closed |
| CLI, library, selector, sync-policy consolidation | `subsystems/cli-library-sync-consolidation-roadmap.md` | M001-M005 (CLI, inspection, boundaries, parity, retry) | closed |
| Local MCP integration | `subsystems/mcp-integration-roadmap.md` | M001 (server + registration) | closed |

Pre-convention flat-plan mapping: `000` → archived direction (superseded
by canonical docs plus `distribution-release`/`mcp-integration`);
`001,002,006,007,013` → `distribution-release` M001-M005;
`003,018,019,020` → `server-lifecycle-http` M001-M004;
`004,014,015,016,017` → `updater-transport` M001-M005;
`008,009,010,011,012` → `cli-library-sync-consolidation` M001-M005;
`005` → `mcp-integration` M001.

## Core rule

Long-term documents state **what snip-it is becoming and what must remain
true**. Interim documents state **what an agent should implement next
against a specific repository baseline**.

Implementation agents MUST NOT add commit-specific steps, transient file
lists, current test counts, or short-lived corrective work to the canonical
long-term documents.

## Planning lifecycle

1. Identify the relevant long-term specification sections and invariants.
2. Record any unresolved architectural decision in `adrs/`.
3. Create or update a subsystem roadmap in `subsystems/`.
4. Select one dependency-ready milestone.
5. Write a bounded handoff plan under `implementation/<subsystem>/`.
6. Register the plan in `registry.md` before handing it to an agent.
7. Implement and verify the milestone.
8. Write a closure record under `closure/<subsystem>/`.
9. Update `registry.md` and the subsystem roadmap status.
10. Move completed or superseded interim documents to `archive/` when they
    no longer represent active work. On closure, audit blocked work and
    unblock dependents in the same commit.

No milestone is complete merely because code landed. Completion requires
the closure evidence defined by its implementation plan and subsystem
roadmap.

## Required classification

Every subsystem roadmap and implementation plan MUST distinguish:

- **Invariant** — a property that must always remain true.
- **Capability** — user- or operator-visible behavior.
- **Infrastructure** — internal machinery required by capabilities.
- **Polish** — ergonomics, diagnostics, performance tuning, cleanup, or documentation.

Infrastructure and polish MUST NOT be presented as completed user
capability unless the user-visible acceptance criteria are actually
satisfied.

## Naming conventions

- ADR: `adrs/ADR-NNNN-short-title.md`
- Subsystem roadmap: `subsystems/<subsystem>-roadmap.md`
- Milestone implementation plan: `implementation/<subsystem>/NNN-short-title.md` (numbering local to the subsystem)
- Closure record: `closure/<subsystem>/NNN-status.md` (same number as the plan)
- Archived predecessor pointer: `archive/flat-NNN-original-name.md`

Use stable subsystem names. Do not encode dates in filenames unless the
document is inherently time-bound.

## Starting a new workstream

Begin with `subsystems/README.md`, then use the templates and rules in:

- `adrs/README.md`
- `implementation/README.md`
- `closure/README.md`
- `.skills/planning.md` (agent-facing planning workflow)

Register active work in `registry.md` before handing implementation plans
to agents.

## Execution policy (carried forward)

Implement plans in dependency order. Each plan is scoped so a smaller
implementation model can complete it without redesigning the surrounding
system.

Preserve `snip-it` as a lightweight terminal tool: no new workspace
crates, service/repository abstractions, DI frameworks, plugin/rule
engines, databases/search indexes, generic query languages, background job
systems, networked MCP, MCP mutations/execution, retry middleware, or
additional CI/release hardening unless a later roadmap revision explicitly
authorizes it.

Existing user-facing command spellings and the documented Rust API should
remain compatible unless a plan explicitly requires a semver-compatible
deprecation path. Prefer concrete structs and ordinary functions over
traits or generalized infrastructure. A refactor is successful when it
deletes duplicate policy and reduces unrelated reasons for files to
change — not when it maximizes module count.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`,
`croncheck`, `/health`) remain the baseline. Reuse them rather than
introducing a second daemon/process-control architecture.

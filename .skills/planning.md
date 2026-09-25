# Planning Skill

## Purpose

Guide agents through snip-it's `plans/` planning process (adopted from the
codegg convention): creating subsystem roadmaps, milestone implementation
plans, closure records, and registering/unblocking them in
`plans/registry.md`.

snip-it separates **canonical long-term direction** (stable) from
**interim planning** (operational, may evolve with the codebase).
Implementation agents MUST NOT silently rewrite the long-term documents.

Full templates live in `plans/subsystems/README.md`,
`plans/implementation/README.md`, `plans/closure/README.md`, and
`plans/adrs/README.md`. Governance is normative in
`plans/003-planning-process.md`.

> Registry truth: check `plans/registry.md` before assuming any roadmap state.
> All currently registered milestones are `closed` with no open dependencies —
> register new work only after handoff review per `003 §10`. One commit = one
> status change; the closure record is the gate.

## When to use

Load this skill when working on:

- Creating a subsystem roadmap under `plans/subsystems/`
- Writing a milestone implementation plan under
  `plans/implementation/<subsystem>/`
- Writing a closure record under `plans/closure/<subsystem>/`
- Updating `plans/registry.md` (adding dependency-ready plans, marking
  active/closing/closed, unblocking downstream work)
- Writing an ADR under `plans/adrs/`
- Archiving completed interim documents to `plans/archive/`
- Reviewing whether a finished milestone unblocks other registered plans

## Document hierarchy

```text
Long-term specification and terminology      (canonical, stable)
        |
        v
Architecture decision records (adrs/)        (durable decisions)
        |
        v
Master long-term roadmap (002-)              (canonical sequencing)
        |
        v
Subsystem roadmaps (subsystems/)             (coherent workstreams)
        |
        v
Milestone implementation plans               (bounded agent handoff)
        |
        v
Implementation and verification
        |
        v
Closure records (closure/)                   (gate that determines completion)
        |
        v
Archive (archive/)                           (historical traceability)
```

Interim plans MUST reference canonical documents rather than duplicating
them. Repository evidence overrides interim plans, but long-term invariants
override both.

Authority order on conflict:

1. Canonical long-term specification and terminology
2. Accepted ADRs
3. Subsystem roadmap
4. Milestone implementation plan
5. Current repository evidence

## Status vocabulary (registry.md)

- `proposed` — exists but not approved for execution
- `ready` — dependencies satisfied; plan may be handed off
- `active` — implementation in progress
- `blocked` — named dependency or evidence requirement prevents progress
- `closing` — implementation landed; closure evidence being gathered
- `closed` — closure record accepted
- `conditionally closed` — substantial work landed but a named correctness
  finding prevents strict closure
- `superseded` — replaced by another document
- `archived` — no longer active; retained for traceability

Pre-convention `Status: complete` in promoted flat plans maps to `closed`.

## Naming conventions

- ADR: `adrs/ADR-NNNN-short-title.md` (monotonically increasing; never reused)
- Subsystem roadmap: `subsystems/<subsystem>-roadmap.md` (no dates)
- Implementation plan:
  `implementation/<subsystem>/NNN-short-title.md` (number local to subsystem)
- Closure record: `closure/<subsystem>/NNN-status.md` (same number as plan)
- Archive: predecessor pointers as `archive/flat-NNN-original-name.md`;
  preserve subsystem grouping for later moves

Stable subsystem names: `distribution-release`, `server-lifecycle-http`,
`updater-transport`, `cli-library-sync-consolidation`, `mcp-integration`.

## Work classification

Every planned item MUST carry one primary class:

- **Invariant** — must remain true across releases (e.g. transaction-gated
  mutations, `(updated_at, device_id, SHA-256(synced fields))` conflicts,
  deletion-beats-live, local-only exclusions, golden-corpus save path,
  single `retry_grpc_unified!` ownership). Requires guards/property tests
  or architecture-level evidence.
- **Capability** — user/operator-visible behavior (e.g. snippet CRUD and
  search, fleet install, binary-first update, read-only MCP, lifecycle
  commands). Requires end-to-end acceptance evidence.
- **Infrastructure** — internal machinery (e.g. readonly resolvers, module
  splits, retry dedup, EggServe leaf, lean eggfetch). MUST NOT be presented
  as completed capability until a consumer path exists.
- **Polish** — ergonomics, diagnostics, performance, cleanup, docs. Follows
  functional and correctness closure.

## Lifecycle

1. Identify relevant canonical long-term sections.
2. Record unresolved architectural decisions in `adrs/` (new ADR).
3. Create or update a subsystem roadmap in `subsystems/`.
4. Select one dependency-ready milestone.
5. Write a bounded handoff plan under `implementation/<subsystem>/`.
6. Register the plan in `plans/registry.md` (`ready`; move to `active`
   when work begins).
7. Implement and verify.
8. Write a closure record under `closure/<subsystem>/`.
9. Update `registry.md` and the subsystem roadmap status.
10. Archive completed interim documents when they no longer represent
    active work.
11. Audit blocked work: if this closure satisfies a registered plan's
    blocker, move it to `ready` in the same commit and record the audit in
    the closure record.

A milestone is complete only when the closure evidence in its
implementation plan and subsystem roadmap is satisfied. A commit message
saying "closed" is not closure evidence.

## Subsystem roadmap essentials

Required sections: purpose/ownership boundary; work classification;
non-goals; current state (repo evidence, no fragile line numbers); target
architecture; dependency graph (hard/interface/soft/operational);
milestones (each with class, objective, dependencies, deliverable boundary,
user value, exit conditions); cross-cutting requirements (storage,
protocol, security, concurrency, observability, docs); verification
strategy; risks/decision points; completion definition; milestone-status
table.

Rules: link to canonical requirements; distinguish infrastructure from
completed capability; expose dependencies; preserve completed history;
state non-goals; stay at subsystem level (no file-by-file checklists).

## Implementation plan essentials

Required sections: objective (one bounded outcome); readiness (closed hard
deps, stable interface deps); current evidence; invariants that must not
regress; scope (in/out); required production changes; ordered work
packages (intent, changes, acceptance evidence); failure/cancellation/
restart/contention semantics; compatibility/migration; required tests;
verification commands; documentation updates; acceptance criteria; stop
conditions; closure evidence required; handoff notes (serial-test needs,
`SNP_ALLOW_PLAINTEXT_API_KEY=true` seam, `unsafe set_var`, preserved user
changes).

Sizing: one coherent pass (one ownership boundary, production changes,
tests, verification, docs). Prefer vertical slices over broad horizontal
refactors. Keep `snp` lightweight: no new crates, frameworks, or services
unless the roadmap authorizes them.

## Closure record essentials

Required sections: executive finding; requirement-to-evidence matrix;
production evidence; verification executed (exact commands + results; label
local vs CI truthfully); invariant review; failure/recovery review;
migration/compatibility review; security review; documentation/operations;
unresolved findings (critical/high/medium/low); roadmap disposition
(closed, conditionally closed, corrective pass required, blocked, roadmap
revision); registry updates.

A milestone MUST NOT be marked `closed` when only compilation/formatting
was verified, required tests were not run, a user-visible capability has
only infrastructure, a security/migration requirement is unimplemented, a
transaction-gated path bypasses journal/lock authority, a high-severity
defect remains, or closure depends on unrecorded assumptions.

## Corrective passes

A corrective pass is a new implementation plan in the same subsystem
directory (not an amendment pretending the original succeeded). It MUST
reference the original plan and closure record, list each unclosed
requirement or defect, explain why verification missed it, add regression
tests/guards, and avoid reopening closed scope without evidence.

## Registry maintenance

- Add a subsystem roadmap when it becomes active, not merely as a future
  track.
- Register a plan as `ready` only after dependency/handoff review.
- Move `ready` → `active` → `closing` → `closed` with a closure record.
- Record blockers precisely with the owning document linked.
- Never copy detailed milestone requirements into the registry; link.
- One commit = one status change; the closure record (not the commit
  message) is the gate.

## Required review before handoff

1. Correct long-term references
2. Unresolved decisions have ADRs or are explicitly out of scope
3. Dependency readiness (hard closed, interface stable)
4. Bounded scope and explicit non-goals
5. Explicit ownership and invariants
6. Migration and compatibility effects
7. Concurrency, cancellation, restart, failure semantics
8. Security and authorization effects
9. Required test and static-guard evidence
10. Unambiguous closure criteria

If these are not answerable, the work is not ready for handoff.

## Verification profile

```bash
cargo test --workspace --lib
cargo test --test <name> --features test-support [-- --test-threads=1]
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Serial suites always use `--test-threads=1`: `pty_integration`,
`*_concurrency`, `sync_multibatch`, `*_barriers`, `repair_transactions`.
Deep crash/restore/manifest suites run only in
`bash scripts/release-check.sh verify` (clean tree required), not ordinary
closure. Do not claim commands that were not actually run.

## Anti-patterns (prohibited)

- Transient TODO checklists in canonical long-term documents
- One roadmap mixing all subsystems at file granularity
- Broad product goals without a bounded milestone contract
- Equating compilation with closure
- Per-subsystem terminology drift
- Implementation plans silently overriding accepted architecture
- Stale active plans after material repo changes
- Duplicated requirements without one authoritative source
- Recording only successful evidence; omitting blocked/unrun verification
- Polish phases before the capability correctness boundary is closed

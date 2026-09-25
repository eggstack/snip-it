# Milestone Implementation Plans

This directory contains bounded plans handed directly to implementation
agents.

Implementation plans are operational documents tied to the current
repository state. They may be corrected, superseded, or archived without
modifying the canonical long-term documents.

## Layout and naming

```text
implementation/<subsystem>/NNN-short-title.md
```

Milestone numbering is local to the subsystem roadmap unless the roadmap
defines another stable identifier.

## Required implementation-plan template

```markdown
# <Subsystem> Milestone NNN — <Title>

Status: ready for handoff | active | blocked | implemented | superseded

Repository baseline: `<commit SHA or branch state>`

Source roadmap:

- `plans/subsystems/<subsystem>-roadmap.md#...`

Long-term requirements:

- `plans/000-long-term-specification.md#...`
- `plans/001-terminology-and-domain-model.md#...`

Applicable ADRs:

- `plans/adrs/ADR-NNNN-...md`

Primary class: invariant | capability | infrastructure | polish

## 1. Objective

State one bounded outcome.

## 2. Why this milestone is ready

List closed hard dependencies and stable interface dependencies.

## 3. Current implementation evidence

Describe the relevant code, tests, storage, protocols, guards, and known
gaps at the repository baseline.

## 4. Invariants that must not regress

- ...

## 5. Scope

### In scope

- ...

### Explicitly out of scope

- ...

## 6. Required production changes

Describe required behavior and ownership changes. Name likely modules where
useful, but do not prescribe blind mechanical edits when alternatives are
valid.

### Core/domain

### Storage and migrations

### Protocol and DTOs

### Runtime and concurrency

### Frontend or operator surface

### Security and authorization

### Documentation and static guards

## 7. Ordered work packages

### Work package A — Title

Intent:

Required changes:

Acceptance evidence:

### Work package B — Title

...

## 8. Failure, cancellation, restart, and contention semantics

State expected behavior for partial failure, duplicate delivery, process
restart, cancellation races, stale generations, and concurrent callers.

## 9. Compatibility and migration

Describe backward compatibility, data migration, protocol negotiation,
configuration fallback, and removal criteria for legacy paths.

## 10. Required tests

### Focused unit tests

### Integration tests

### Restart and recovery tests

### Contention and cancellation tests

### Security and negative tests

### Migration and compatibility tests

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test <name> --features test-support [-- --test-threads=1]

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

## 12. Documentation updates

- ...

## 13. Acceptance criteria

Use externally observable or contract-level statements.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- an unresolved architecture decision materially changes ownership;
- a hard dependency is absent;
- required migration cannot be made safely;
- repository evidence contradicts a canonical invariant;
- external evidence is required but unavailable;
- scope would expand into another subsystem roadmap.

## 15. Closure evidence required

List the exact evidence the later closure record must contain.

## 16. Handoff notes

Include known hazards, resource constraints, serial-test requirements
(`--test-threads=1` for PTY/concurrency/barrier suites), environment
requirements, `SNP_ALLOW_PLAINTEXT_API_KEY=true` seam, `unsafe set_var` in
edition 2024, and preserved user changes.
```

## Handoff rules

Before assigning a plan to an agent:

- confirm the repository baseline is current;
- confirm all hard dependencies are closed;
- confirm unresolved decisions have ADRs or are explicitly out of scope;
- ensure the milestone can be completed in one coherent pass;
- ensure tests and closure evidence are specific;
- register the plan in `plans/registry.md`.

The implementation agent may adjust file-level mechanics based on current
code. It may not weaken canonical invariants or silently enlarge scope.
Keep `snp` lightweight: no new crates, frameworks, or services unless the
subsystem roadmap authorizes them.

## Corrective plans

Corrective work receives a new plan in the same subsystem directory, for
example:

```text
004-correctness-and-recovery-closure.md
```

The corrective plan must reference the original plan and closure record,
enumerate unclosed findings, and add regression evidence that would have
caught them.

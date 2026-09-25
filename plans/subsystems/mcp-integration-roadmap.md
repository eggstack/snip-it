# Local MCP Integration Roadmap

Status: closed

Long-term references:

- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#7-search-selector-and-mcp-invariants`
- `plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms`
- `plans/002-long-term-roadmap.md#phase-4-local-mcp-server-and-client-registration`
- `plans/003-planning-process.md`

Related ADRs: none required. MCP stays inside the read-only stdio
contract. If a future change proposes mutations, execution, or networked
operation, open an ADR and revise the long-term non-goals first.

Predecessor evidence (pre-convention flat plans, retained in
`plans/archive/`):

- `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md` —
  canonical predecessor direction (MCP half).

## 1. Purpose and ownership boundary

This subsystem owns the local stdio MCP endpoint and explicit client
registration. It consumes the canonical selector/search projection and the
read-only library resolver. It MUST NOT own snippet execution, library
mutation, sync, transport, or release behavior, and MUST NOT become a
long-running service.

## 2. Work classification

### Invariants

- Read-only, stdio-only, non-executing MCP.
- Shared `selector::searchable_text` semantics with the output/notes
  opt-in and never-searchable exclusions intact.
- `snippet_get` takes exactly one of ID (case-sensitive) / description /
  command (case-insensitive).

### Capabilities

- `snippets_search` and `snippet_get` for agent clients.
- Explicit client registration flow.

### Infrastructure

- None beyond the shared resolver/projection reuse.

### Polish

- Registration diagnostics and parity documentation.

## 3. Non-goals

- Generic agent-visible arbitrary command execution.
- MCP mutations, command execution, or networked/long-running MCP service.
- New query languages, indexes, or background job systems.

## 4. Current state

The milestone is closed. The local MCP adapter is read-only and launched
on demand by the agent client. Search parity with CLI holds, including
tags-by-default, output/notes opt-in, and credential/metadata exclusion.
Client registration is explicit and covered with isolated config.

## 5. Target architecture

Agent client spawns the stdio adapter on demand; the adapter resolves
through `resolve_selector_readonly`/`inspect_library_index` and projects
through `searchable_text`; no daemon, no socket service, no execution
tool.

## 6. Dependency graph

```text
M001 local MCP server + client registration
    (interface: canonical selector contract from cli-library-sync-consolidation M004;
     soft: distribution predecessor direction in flat-000)
```

The selector contract is an interface dependency; implementation proceeded
against the stable projection and closed with parity proven.

## 7. Milestones

### M001 — Local MCP server and client registration

Class: capability + invariant

Objective: ship the read-only local MCP adapter plus registration without
creating another service.

Dependencies: interface on the canonical selector/search contract (closed
via `cli-library-sync-consolidation` M004); soft on flat-000 direction.

Deliverable boundary: `snippets_search`/`snippet_get` plus registration;
no execution tool, no mutations, no networking.

User or operator value: agent clients search and fetch snippets with the
same semantics as the CLI.

Exit conditions: parity fixtures green; `snippet_get` shape enforced;
registration works with isolated config; no executable MCP tool exists.

Deferred work: none; workstream closed.

Implementation plan:

- `plans/implementation/mcp-integration/001-local-mcp-server-and-client-registration.md`

Closure record:

- `plans/closure/mcp-integration/001-status.md`

Predecessor flat plan: `plans/archive/flat-005-local-mcp-server-and-client-registration.md` (plus MCP direction
in `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md`).

## 8. Cross-cutting requirements

### Storage and migration

Read-only; no persistence changes.

### Protocol and compatibility

MCP shapes follow the selector contract; CLI behavior unchanged.

### Security and authorization

No command execution; no credential/sync-metadata exposure; no privilege
escalation.

### Concurrency, cancellation, and recovery

On-demand stdio process; no shared mutable server state.

### Observability and audit

Parity test evidence; registration diagnostics.

### Performance and resource use

No daemon or index cost.

### Documentation and operations

`docs/MCP.md` and `architecture/` notes updated with the read-only
contract.

## 9. Verification strategy

MCP search-parity fixtures, `snippet_get` shape tests, isolated-config
registration tests, `scripts/check.sh`, production-seam checks.

## 10. Risks and decision points

No open decisions. Any MCP scope expansion requires a long-term revision
and ADR before a new milestone.

## 11. Completion definition

This workstream closes when M001 has accepted closure evidence proving
read-only parity, registration, and the absence of execution/mutation
surfaces, with no unresolved medium-or-higher finding. The milestone is
closed; the roadmap is closed.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 local MCP + registration | closed | `plans/implementation/mcp-integration/001-local-mcp-server-and-client-registration.md` | `plans/closure/mcp-integration/001-status.md` | — |

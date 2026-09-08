# Implementation plans

This directory contains active implementation plans intended for agent handoff.

## Completed distribution sequence

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [000](000-distribution-fleet-and-mcp-roadmap.md) | Distribution, fleet deployment, and MCP roadmap | Complete | 006, 007 |
| [001](001-release-binary-matrix-and-artifact-contract.md) | Release binary matrix and artifact contract | Complete | 000 |
| [002](002-bootstrap-installers.md) | Binary-first bootstrap installers | Complete | 001 |
| [003](003-snip-sync-startup-and-lifecycle.md) | snip-sync startup and lifecycle management | Complete | 001 |
| [004](004-binary-first-self-update.md) | Binary-first self-update and restart integration | Complete | 001, 003 |
| [005](005-local-mcp-server-and-client-registration.md) | Local MCP server and client registration | Complete | 002 |
| [006](006-windows-ci-platform-closure.md) | Windows CI and platform closure | Complete | 001–005 |
| [007](007-release-publication-and-distribution-closure.md) | Release publication and distribution closure | Complete | 006 |

Plans 001–005 implemented the intended distribution/MCP feature work, Plan 006 restored the ordinary Windows all-target/platform-smoke gate, and Plan 007 completed publication and end-to-end distribution evidence without weakening published-release immutability. That line of work is closed.

## Active consolidation sequence

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [008](008-cli-surface-and-outcome-consolidation.md) | CLI surface and outcome consolidation | Complete | — |
| [009](009-shared-inspection-and-readonly-library-resolution.md) | Shared inspection and read-only library resolution | Complete | 008 |
| [010](010-module-boundary-and-hotspot-decomposition.md) | Module boundary and hotspot decomposition | Complete | 008–009 |
| [011](011-selector-search-and-mcp-read-parity.md) | Selector, search, and MCP read parity | Complete | 009–010 |
| [012](012-sync-retry-policy-deduplication.md) | Sync retry policy deduplication | Ready | 008–011 |

This sequence responds to the September 2026 architecture/maintenance review. Its purpose is to remove overlapping policy and improve depth in existing snippet retrieval/search behavior before any further broad feature expansion.

The intended order is deliberately deletion/consolidation first:

1. remove duplicate CLI schemas and unnecessary outcome translations;
2. establish side-effect-free shared library resolution and narrowly shared inspection primitives;
3. split large hotspot files only along existing responsibility boundaries;
4. reuse canonical selector/search semantics in CLI and read-only MCP;
5. deduplicate sync retry policy as a final internal cleanup.

## Execution policy

Implement plans in dependency order. Each plan is scoped so a smaller implementation model can complete it without redesigning the surrounding system. When a plan is completed, update its `Status:` line and this table in the same implementation commit.

For Plans 008–012, preserve `snip-it` as a lightweight terminal tool. Specifically, do not use this consolidation work to introduce additional workspace crates, service/repository abstractions, dependency-injection frameworks, plugin/rule engines, databases/search indexes, generic query languages, background job systems, networked MCP, MCP mutations/execution, retry middleware frameworks, or additional CI/release hardening.

Existing user-facing command spellings and the documented Rust API should remain compatible unless a plan explicitly requires a semver-compatible deprecation path. Prefer concrete structs and ordinary functions over traits or generalized infrastructure. A refactor is successful when it deletes duplicate policy and reduces unrelated reasons for files to change—not when it maximizes module count.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) remain the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

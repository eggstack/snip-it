# Implementation plans

This directory contains active implementation plans intended for agent handoff.

## Active sequence

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

## Corrective closure

Plans 001–005 implemented the intended feature work, Plan 006 restored the ordinary Windows all-target/platform-smoke gate, and Plan 007 completed publication and end-to-end distribution evidence without weakening published-release immutability. The corrective closure is complete; no Ready/Planned corrective plan remains for this line of work.

## Execution policy

Implement plans in dependency order. Each plan is intentionally scoped so a smaller implementation model can complete it without having to redesign the surrounding system. Do not expand this line of work into apt, Homebrew, Winget, container-orchestration, auto-update daemons, or production-grade fleet management.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) are the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

When a plan is completed, update its `Status:` line and this table in the same implementation commit.

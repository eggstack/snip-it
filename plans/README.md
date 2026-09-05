# Implementation plans

This directory contains active implementation plans intended for agent handoff.

## Active sequence

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [000](000-distribution-fleet-and-mcp-roadmap.md) | Distribution, fleet deployment, and MCP roadmap | Blocked on closure | 006, 007 |
| [001](001-release-binary-matrix-and-artifact-contract.md) | Release binary matrix and artifact contract | Complete | 000 |
| [002](002-bootstrap-installers.md) | Binary-first bootstrap installers | Complete | 001 |
| [003](003-snip-sync-startup-and-lifecycle.md) | snip-sync startup and lifecycle management | Complete | 001 |
| [004](004-binary-first-self-update.md) | Binary-first self-update and restart integration | Complete | 001, 003 |
| [005](005-local-mcp-server-and-client-registration.md) | Local MCP server and client registration | Complete | 002 |
| [006](006-windows-ci-platform-closure.md) | Windows CI and platform closure | Ready | 001–005 |
| [007](007-release-publication-and-distribution-closure.md) | Release publication and distribution closure | Planned | 006 |

## Corrective closure

Plans 001–005 implemented the intended feature work. The umbrella roadmap is not closed yet because ordinary Windows CI remains red at the workspace all-target check and the release workflow has not yet proved a successful draft-asset publication/real binary-consumer path. Plan 006 restores the cross-platform gate; Plan 007 then closes publication and end-to-end distribution evidence without weakening published-release immutability.

## Execution policy

Implement plans in dependency order. Each plan is intentionally scoped so a smaller implementation model can complete it without having to redesign the surrounding system. Do not expand this line of work into apt, Homebrew, Winget, container-orchestration, auto-update daemons, or production-grade fleet management.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) are the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

When a plan is completed, update its `Status:` line and this table in the same implementation commit.

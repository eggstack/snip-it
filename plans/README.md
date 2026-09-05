# Implementation plans

This directory contains active implementation plans intended for agent handoff.

## Active sequence

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [000](000-distribution-fleet-and-mcp-roadmap.md) | Distribution, fleet deployment, and MCP roadmap | Planned | — |
| [001](001-release-binary-matrix-and-artifact-contract.md) | Release binary matrix and artifact contract | Complete | 000 |
| [002](002-bootstrap-installers.md) | Binary-first bootstrap installers | Ready | 001 |
| [003](003-snip-sync-startup-and-lifecycle.md) | snip-sync startup and lifecycle management | Ready | 001 |
| [004](004-binary-first-self-update.md) | Binary-first self-update and restart integration | Planned | 001, 003 |
| [005](005-local-mcp-server-and-client-registration.md) | Local MCP server and client registration | Planned | 002 |

## Execution policy

Implement plans in dependency order. Each plan is intentionally scoped so a smaller implementation model can complete it without having to redesign the surrounding system. Do not expand this line of work into apt, Homebrew, Winget, container-orchestration, auto-update daemons, or production-grade fleet management.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) are the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

When a plan is completed, update its `Status:` line and this table in the same implementation commit.

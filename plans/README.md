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

Plans 001–005 implemented the intended distribution/MCP feature work, Plan 006 restored the ordinary Windows all-target/platform-smoke gate, and Plan 007 completed publication and end-to-end distribution evidence without weakening published-release immutability. That historical sequence remains closed.

## Active distribution corrective follow-up

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [013](013-snp-binary-publication-and-installer-corrective-closure.md) | snp binary publication and installer corrective closure | Complete | 001, 002, 007 |

Plan 013 is complete: `snip-it 1.3.8` is published with all five public `snp`
binaries plus checksums (historical `v1.3.7` left untouched), the consumer
smoke exercises both independently versioned components across Linux
x86_64/ARM64, macOS Intel/Apple Silicon, and Windows x86_64 plus the exact
README unpinned bootstrap, and the Bash/PowerShell installer failure-mode
contracts are covered deterministically. The correction was achieved without
a second release workflow, matrix duplication, or new dependencies.

## Completed consolidation sequence

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [008](008-cli-surface-and-outcome-consolidation.md) | CLI surface and outcome consolidation | Complete | — |
| [009](009-shared-inspection-and-readonly-library-resolution.md) | Shared inspection and read-only library resolution | Complete | 008 |
| [010](010-module-boundary-and-hotspot-decomposition.md) | Module boundary and hotspot decomposition | Complete | 008–009 |
| [011](011-selector-search-and-mcp-read-parity.md) | Selector, search, and MCP read parity | Complete | 009–010 |
| [012](012-sync-retry-policy-deduplication.md) | Sync retry policy deduplication | Complete | 008–011 |

Plans 008–012 are complete. That line of work is closed.

This sequence responds to the September 2026 architecture/maintenance review. Its purpose is to remove overlapping policy and improve depth in existing snippet retrieval/search behavior before any further broad feature expansion.

The intended order is deliberately deletion/consolidation first:

1. remove duplicate CLI schemas and unnecessary outcome translations;
2. establish side-effect-free shared library resolution and narrowly shared inspection primitives;
3. split large hotspot files only along existing responsibility boundaries;
4. reuse canonical selector/search semantics in CLI and read-only MCP;
5. deduplicate sync retry policy as a final internal cleanup.

## Completed updater transport consolidation

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [014](014-eggfetch-self-update-transport-consolidation.md) | eggfetch self-update transport consolidation | Complete | 004 |

Plan 014 is complete with a measurement-driven split: `snp update` now
uses in-process `eggfetch-core` 0.1.5 (narrow H1/Rustls/native-root
profile, +11.5% reported and kept), while `snip-sync update` retains its
external `curl` adapter (+41% in controlled builds, past the 10% gate).
The split, measurements, and trust-profile decision are recorded in the
plan's completion notes; the retained `curl` path is documented as an
intentional tradeoff, not unfinished work.

## Active updater corrective follow-up

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [015](015-snp-updater-end-to-end-timeout-corrective.md) | snp updater end-to-end timeout corrective | Complete | 014 |

Plan 015 corrects one narrow Plan 014 regression: the current 60-second
wall-clock timeout ends after redirect/request handling, before the final
response body is consumed. The corrective pass moves the total deadline
around the complete metadata or streamed-binary operation and adds slow-drip
body tests proving continued read progress cannot extend the total budget.
Timed-out binary downloads must also remove any partial staging file. No
`snip-sync`, dependency, installer, release-workflow, or architecture changes
belong in this pass.

Plan 015 is complete: `safe_get` owns redirect traversal only while
`fetch_bytes_with` / `fetch_file_with` each apply one injected overall
timeout around traversal plus complete final-body consumption (timeout
stays `FetchError::Failed`, partial staging files are removed on
cancellation), with metadata and binary slow-drip regression tests proving
steady read progress cannot outlast the wall-clock budget. The correction
was internal and unreleased, so no changelog entry applies; `snip-sync`,
dependencies, installers, and release workflows are untouched. Full
workspace tests, `scripts/check.sh`, and production-seam checks are green;
GitHub Actions is verified through the pushed implementation commit.

## Execution policy

Implement plans in dependency order. Each plan is scoped so a smaller implementation model can complete it without redesigning the surrounding system. When a plan is completed, update its `Status:` line and this table in the same implementation commit.

For Plans 008–012, preserve `snip-it` as a lightweight terminal tool. Specifically, do not use this consolidation work to introduce additional workspace crates, service/repository abstractions, dependency-injection frameworks, plugin/rule engines, databases/search indexes, generic query languages, background job systems, networked MCP, MCP mutations/execution, retry middleware frameworks, or additional CI/release hardening.

For Plan 013, preserve the existing single release workflow and component-specific tag/version namespaces. Do not duplicate the release matrix or retrofit binaries onto an already-published historical release. The correction should be achieved through symmetric consumer/install testing and the next legitimate `snip-it` release.

For Plan 014, preserve HTTPS-only production redirect behavior, bounded metadata/binary transfers, 404-only Cargo fallback classification, and the lightweight `snip-sync` footprint. Do not turn the migration into a general HTTP abstraction, shared updater crate, broad eggfetch feature enablement, or async-main rewrite. The plan's controlled binary-size gate is authoritative for whether the server updater migrates.

For Plan 015, preserve the Plan 014 transport and dependency decisions exactly. Limit implementation to moving the overall `snp` fetch deadline so it covers redirect traversal plus complete final-body consumption, guaranteeing partial-file cleanup on timeout cancellation, and adding deterministic slow-body regression tests. Do not touch `snip-sync`, dependency versions/features, installers, release workflows, proxy behavior, retries, or runtime architecture.

Existing user-facing command spellings and the documented Rust API should remain compatible unless a plan explicitly requires a semver-compatible deprecation path. Prefer concrete structs and ordinary functions over traits or generalized infrastructure. A refactor is successful when it deletes duplicate policy and reduces unrelated reasons for files to change—not when it maximizes module count.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) remain the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

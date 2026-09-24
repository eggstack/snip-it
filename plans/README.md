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

## Completed updater dependency adoption

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [016](016-eggfetch-0.1.7-lean-updater-adoption.md) | eggfetch 0.1.7 lean updater adoption | Complete | 014, 015 |

Plan 016 is complete: the already-adopted `snp` transport moved from pinned
`eggfetch-core` 0.1.5 to 0.1.7 on the lean standard-route profile
`standard-http1 + redirects + tls-rustls + tls-native-roots`. Eggfetch owns
strict bounded redirects (`RedirectPolicy::strict(10)`) and the absolute
request/body `Timeout.total`; the manual redirect state machine and Plan
015's duplicate outer Tokio timeout are deleted while initial HTTPS
validation, body limits, 404-only Cargo fallback, and partial-file cleanup
are preserved. Controlled same-toolchain release measurements: 0.1.5 baseline
6,973,056 bytes, full-0.1.7 control 6,973,056 bytes, lean-0.1.7 final
6,776,320 bytes (-196,736 / -2.82%). `snip-sync` remains on curl; its Plan
014 +41% server-size result was not reopened.


## Completed eggfetch 0.2.0 adoption

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [017](017-eggfetch-0.2.0-updater-adoption-and-server-requalification.md) | eggfetch 0.2.0 updater adoption and server requalification | Complete | 016 |

Plan 017 adopts the coordinated `eggfetch-core 0.2.0` release for the
already-migrated `snp update` path without changing its lean feature profile
or transport policy. The required path is a narrow exact-version/lockfile
bump plus existing updater and platform qualification. The plan also permits
one controlled measurement-only `snip-sync` trial against the current lean
profile because the retained-curl decision was measured against the older
0.1.5 full HTTP profile; the server remains on curl unless the fresh trial is
<=10% growth and preserves the existing lightweight architecture.

Plan 017 is complete: `snp` now pins `eggfetch-core 0.2.0` on the unchanged
lean profile, with no `src/update.rs` changes required and all 21 focused
updater tests passing. The fresh same-host release build stayed byte-identical
at 6,776,320 bytes. The bounded `snip-sync` lean trial measured 5,145,224
bytes versus the 3,833,152-byte curl baseline (+1,312,072 / +34.23%), so the
server remains on curl and no temporary transport code was retained.


## Active EggServe HTTP runtime adoption

| Plan | Title | Status | Depends on |
| --- | --- | --- | --- |
| [018](018-eggserve-0.2.2-axum-runtime-adoption-trial.md) | EggServe 0.2.2 Axum-preserving HTTP runtime adoption trial | Complete | 003, 017 |
| [019](019-direct-eggserve-leaf-http-service-consolidation.md) | Direct EggServe leaf HTTP service consolidation | Complete | 018 |
| [020](020-eggserve-http-parity-and-closure-corrective.md) | EggServe HTTP parity and closure corrective | Complete | 018, 019 |

Plan 018 is the compatibility-first trial. It keeps the existing Axum
health/metrics Router and replaces only HTTP runtime ownership using the
published registry combination of `eggserve-core 0.2.2[tower]` and
`eggserve-server 0.2.1`. It preserves the pre-bind-both-listeners startup
invariant, Tonic gRPC ownership, reverse-proxy TLS topology, and existing HTTP
application policy. The implementation is measurement-gated against a fresh
release baseline and must revert if it grows the snip-sync binary by more than
10%.

Plan 019 begins only after Plan 018 records its A/B result. It tests the
smaller direct leaf graph, `eggserve-server 0.2.1` plus
`eggserve-primitives 0.2.0`, with one concrete native health/metrics service.
Its purpose is to determine whether removing the core/Tower adapter and
Axum/Tower-HTTP direct dependency surface produces a meaningful footprint or
maintenance win without recreating a framework. Exactly one HTTP production
architecture must remain after the A/B/C comparison. The measured sizes were
A = 3,833,152 bytes, B = 3,963,968 bytes (+3.41%), and C = 3,898,408 bytes
(+1.70% vs A, -1.65% vs B). C is the final winner: it removes the direct
compatibility/framework dependencies and 13 unique packages from B while
remaining within the size-neutral threshold. Local `scripts/check.sh` and the
production-seam check passed; hosted implementation run `36011491151` passed
Linux correctness plus Windows and macOS platform smoke. Plan 020 is the narrow
closure corrective: it restored the proven historical CORS `Vary` semantics
(`Vary: origin` on non-allow-all responses, omitted for allow-all) and locked
the empty router 404/405 representation, known-route preflight `Allow`, and
the preflight/ordinary security-header boundary with socket tests. Exercising
the pre-migration server showed the specified three-field `Vary` value never
reached the wire for snip-sync's configurations, so the corrective reproduces
the proven contract rather than introducing a new header value. Plans 018 and
019 completion evidence is filled (implementation commit `10aeb994`, C deltas,
resolved leaf versions, run `36011491151`); C remains the final architecture
and the EggServe sequence is closed.

## Execution policy

Implement plans in dependency order. Each plan is scoped so a smaller implementation model can complete it without redesigning the surrounding system. When a plan is completed, update its `Status:` line and this table in the same implementation commit.

For Plans 008–012, preserve `snip-it` as a lightweight terminal tool. Specifically, do not use this consolidation work to introduce additional workspace crates, service/repository abstractions, dependency-injection frameworks, plugin/rule engines, databases/search indexes, generic query languages, background job systems, networked MCP, MCP mutations/execution, retry middleware frameworks, or additional CI/release hardening.

For Plan 013, preserve the existing single release workflow and component-specific tag/version namespaces. Do not duplicate the release matrix or retrofit binaries onto an already-published historical release. The correction should be achieved through symmetric consumer/install testing and the next legitimate `snip-it` release.

For Plan 014, preserve HTTPS-only production redirect behavior, bounded metadata/binary transfers, 404-only Cargo fallback classification, and the lightweight `snip-sync` footprint. Do not turn the migration into a general HTTP abstraction, shared updater crate, broad eggfetch feature enablement, or async-main rewrite. The plan's controlled binary-size gate is authoritative for whether the server updater migrates.

For Plan 015, preserve the Plan 014 transport and dependency decisions exactly. Limit implementation to moving the overall `snp` fetch deadline so it covers redirect traversal plus complete final-body consumption, guaranteeing partial-file cleanup on timeout cancellation, and adding deterministic slow-body regression tests. Do not touch `snip-sync`, dependency versions/features, installers, release workflows, proxy behavior, retries, or runtime architecture.

For Plan 016, adopt eggfetch 0.1.7 as a deletion/footprint pass rather than a broad networking expansion. Prefer `standard-http1 + redirects + tls-rustls + tls-native-roots`; keep the initial production HTTPS guard, delegate hop handling to `RedirectPolicy::strict`, delegate the logical request/body deadline to native `Timeout.total`, and remove the superseded local redirect/outer-timeout machinery only after the existing regression tests pass. Do not enable advanced routing, retry, Basic auth, proxy, HTTP2/3, JSON, compression, cookies, multipart, tracing, or test-util. Keep `snip-sync` on its measured curl path and record controlled A/B/C `snp` size/feature results before closing the plan.


For Plan 018, preserve the existing Axum application surface and change only
HTTP runtime ownership. Use the actual published registry versions
`eggserve-core =0.2.2` with only the `tower` feature and
`eggserve-server =0.2.1`; do not enable EggServe TLS, HTTP/2, or HTTP/3.
Keep Tonic on its separate listener, retain pre-binding of both listeners
before service startup, disable EggServe's total connection-lifetime ceiling,
and supervise HTTP through its typed control/completion API. The fresh
same-environment release-size gate is authoritative: growth above 10% requires
a production revert rather than rationalization.

For Plan 019, do not start until Plan 018 is closed with measured evidence.
Use only the direct published leaf runtime (`eggserve-server =0.2.1`,
`eggserve-primitives =0.2.0`) and one concrete snip-sync health/metrics
service. Do not build a replacement router/middleware framework. Preserve the
captured health, metrics-auth, CORS, security-header, method, HEAD, and
lifecycle contracts. Remove Axum/Tower-HTTP/core only when proven unused, and
keep the direct path only when it passes the original <=10% hard footprint gate
and is meaningfully preferable to the Plan 018 winner.

For Plan 020, keep Plan 019's direct EggServe C architecture fixed. Limit the
corrective work to restoring the historical three-field CORS `Vary` contract,
capturing and preserving the old Axum 404/405 representation and preflight
security-header behavior, adding focused real-socket assertions, and completing
the Plans 018/019 evidence record. Do not reintroduce EggServe core, Axum,
Tower, or Tower-HTTP as direct HTTP dependencies, and do not use this closure
pass for unrelated cleanup or another size/architecture experiment.


Existing user-facing command spellings and the documented Rust API should remain compatible unless a plan explicitly requires a semver-compatible deprecation path. Prefer concrete structs and ordinary functions over traits or generalized infrastructure. A refactor is successful when it deletes duplicate policy and reduces unrelated reasons for files to change—not when it maximizes module count.

The existing `snip-sync` lifecycle primitives (`serve`, `stop`, `restart`, `croncheck`, `/health`) remain the baseline. Reuse them rather than introducing a second daemon/process-control architecture.

# Eggstack Dependency Refresh Milestone 001 — Upgrade Eggfetch and Eggserve

Status: ready for handoff

Repository baseline: `eggstack/snip-it` default `main` reviewed 2026-10-08 (recent baseline commit `19200759b7f7b382c4d7d6b36952c64645baab96`; reconfirm branch base before editing).

Source roadmap:

- `plans/subsystems/eggstack-dependency-refresh-roadmap.md` M001.

Long-term requirements:

- `plans/000-long-term-specification.md` §§1, 3, 8 (two separate binaries, HTTP/runtime and updater split).
- `plans/001-terminology-and-domain-model.md` (no domain impact).
- `plans/002-long-term-roadmap.md` (preserve existing behavior).

Applicable decisions / prerequisites: `plans/subsystems/updater-transport-roadmap.md` M002–M005 and `plans/subsystems/server-lifecycle-http-roadmap.md` M003–M004, both closed; no new ADR anticipated.

Primary class: **infrastructure** (dependency refresh, not new capability).

## 1. Objective

Adopt exact published `eggfetch-core =0.2.2` for `snp` and `eggserve-server =0.4.0` + `eggserve-primitives =0.2.2` for `snip-sync`. Keep the lean dependency features, protect existing wire/security/lifecycle semantics, and document measured binary/dependency effects.

## 2. Why this milestone is ready

Both affected workstreams are closed, boundaries are established, and upstream code exposes the required integration APIs. No feature architecture needs redesign; user requested the upgrades.

**Research disposition:** likely manifest/lockfile + guard/docs update; minimal code adjustments only if a compiler or behavioral regression demonstrates necessity. This is not a claim of successful downstream compilation.

## 3. Current implementation evidence

| Consumer | Current pin and use | Audited target API | Known required change |
|---|---|---|---|
| `snp` | root `Cargo.toml` `eggfetch-core =0.2.0` | `Client::builder/get`, `RedirectPolicy::strict`, `Timeout::builder/total`, `RequestBuilder::max_decoded_body_size/timeout/send`, `Response::bytes/bytes_stream`, `Error::Timeout { phase: Total, .. }`, `Error::DecodedBodyTooLarge` still present | pin/lockfile, architecture guard/docs; no known source API adaptation |
| `snip-sync` | `snip-sync/Cargo.toml`: server/primitives `=0.2.1` | `service_fn_head`, `RequestHead`, `HeaderBlock`, `Response::builder`, `ResponseBody`, `StatusCode`, `RuntimeConfig::builder/disable_connection_total_timeout/graceful_shutdown_timeout`, `Server::builder/from_listener/start_with_service`, `ServerHandle::into_parts`, `ServerControl::shutdown`, `ServerCompletion::wait` still present | synchronized pins/lockfile; runtime and wire parity qualification |

Upstream references:

- `https://github.com/eggstack/eggfetch/blob/main/crates/eggfetch-core/Cargo.toml` and corresponding `src/{client,request,response,redirect,timeout,error}.rs`.
- `https://github.com/eggstack/eggserve/blob/main/crates/eggserve-server/Cargo.toml`, `src/{lib,config,service}.rs`.
- `https://github.com/eggstack/eggserve/blob/main/crates/eggserve-primitives/Cargo.toml`; `https://github.com/eggstack/eggserve/releases/tag/v0.4.0` (release notes). Eggserve-server 0.4.0 requires primitives 0.2.2.
- Direct historical comparisons against `eggfetch` v0.2.0 and `eggserve` v0.2.0 show the audited longstanding surface retained. Eggserve `into_parts` and disabling total connection lifetime were introduced after that older tag; verify current downstream 0.2.1 behavior in the existing Snip-it tests rather than inferring from v0.2.0.

Concrete Snip-it files: `src/update.rs`, `snip-sync/src/{http,orchestration,main}.rs`, `tests/architecture.rs`, `tests/snip_sync_lifetime.rs`, `architecture/{update,server}.md`, `plans/000-long-term-specification.md`. `tests/architecture.rs::snp_updater_pins_lean_eggfetch_profile` explicitly asserts `=0.2.0` and must be updated. Some version comments are historical evidence; distinguish them from current-state statements.

## 4. Invariants not to regress

1. `snp update`: production initial URL HTTPS-only; strict max-10 redirects; HTTPS→HTTP downgrade denied before follow-up I/O; 60s total deadline includes redirect hops and **all body bytes**; connect/read budgets; bounded metadata and binary streams; no automatic decompression, no hidden retries; only real 404 triggers Cargo fallback, other HTTP/transport/integrity failures stop; partially written staging file removed.
2. `snip-sync`: single concrete H1 service for `/health` and `/metrics`; gRPC Tonic remains separate; pre-bind **both** listeners before accepting; HTTP body rejected by Eggserve before handler; no connection lifetime cap; HEAD/OPTIONS, CORS `Vary: origin`, `Allow`, status/body/content-type, security headers, metrics Basic-auth and `WWW-Authenticate` exactly as existing real-socket parity tests.
3. Shutdown: `ServerControl` distinct from single-owner `ServerCompletion`; completion cancellation-safe in `select!`; early service failure propagates; requested dual-clean exit only success; bounded drain; no detached service task.
4. Lightweight: defaults off; Eggfetch feature list unchanged; server has no `eggfetch-core` or new web framework; server updater keeps external `curl`.
5. No storage, protobuf, release tag, CLI, or inter-device protocol changes.

## 5. Scope

### In scope
Three pin updates and `Cargo.lock`, necessary minimal API adapters **if compilation demonstrates need**, existing architecture test pin/doc refresh, focused qualification, same-host size/feature tree comparison, closure evidence.

### Out of scope
Any new abstraction, extra dependencies beyond upstream transitive necessities, broad cargo update, Eggress integration, protocol migration, new service endpoints, TLS termination, optional HTTP/2/3, server updater rewrite, publication or release version bump.

## 6. Required production changes

**Core/domain, storage, protocol:** None planned. Stop if a migration seems required.

**Client transport:** In root `Cargo.toml`, change only `eggfetch-core` to `=0.2.2`; preserve `default-features = false` and precisely `standard-http1`, `redirects`, `tls-rustls`, `tls-native-roots`. Do not re-add wrapper timeouts, redirects, retry policies, or `curl` to `src/update.rs`.

**HTTP runtime:** In `snip-sync/Cargo.toml`, change `eggserve-server` to `=0.4.0` and `eggserve-primitives` to `=0.2.2` with defaults off. Server 0.4.0's own primitives dependency is 0.2.2; avoid co-resolving legacy 0.2.1. Leave `snip-sync/src/http.rs` and `orchestration.rs` unchanged unless a concrete failing test/compiler diagnostic requires the smallest local correction. Preserve explicit timeout and listener settings in `snip-sync/src/main.rs`.

**Frontend/operator surface:** None.

**Security and authorization:** Existing tests are the policy oracle; never weaken failed security assertions to obtain a green migration.

**Documentation/guards:** Update hardcoded version assertion/comments in `tests/architecture.rs` and current-version statements in `architecture/update.md`, `architecture/server.md`, AGENTS/.skills/docs when actually stale; retain historical measurement/closure values and annotate them as history, not new measured results. `plans/000-long-term-specification.md` §8 currently canonically says `eggfetch-core =0.2.0`: perform the smallest change reflecting the user-directed target version (prefer version-neutral normative wording plus version-specific live manifest reference) while preserving exact four features/security model. Do not rewrite §§1–7 or archived closure files.

## 7. Ordered work packages

### A — Baseline and resolution

1. Confirm branch head and Cargo manifests; record `rustc -V`, `cargo -V`, `cargo tree -p snip-it`, `cargo tree -p snip-sync`, active feature graph, and per-component release artifact sizes from an untouched baseline using the same environment intended for after comparison.
2. Change exactly three pins; update only affected lockfile entries using `cargo update -p eggfetch-core --precise 0.2.2`, `cargo update -p eggserve-server --precise 0.4.0`, and `cargo update -p eggserve-primitives --precise 0.2.2` as Cargo resolution permits. Use `cargo check --workspace --all-targets --locked` after lockfile settles.
3. Inspect resolved features and tree: no extra Eggserve primitives 0.2.1, no broad Eggfetch `http1` alias, no surprise `axum`, `tower-http`, or server-side Eggfetch. Do not hand-edit lockfile.

Acceptance: exact manifest/lock versions, sane graph, compile or an explicit diagnostic tied to a narrow API correction.

### B — Narrow source adjustments only if needed

1. Compile the untouched downstream source against new versions.
2. If a named API fails, capture the compiler output, consult upstream 0.4.0/0.2.2 source, patch only that call site, and add a regression where behavior changes.
3. If response normalization or runtime defaults change, explicitly configure existing Eggserve policy at the leaf boundary or otherwise restore exact prior behavior; **do not** broaden to middleware/router framework.

Acceptance: minimal diffs in Rust source; all current contracts remain.

### C — Focused parity

1. Updater loopback: redirect chain and depth, downgrade policy test, metadata limit, binary streaming, total slow-drip timeout including EOF, failed download cleanup, 404-only fallback, non-404 error classification, checksum/candidate verification.
2. HTTP real-socket: healthy/unhealthy status and HEAD, metrics protected/unprotected, CORS preflight/unknown route, 404/405 empty-body semantics and `Allow`/`Vary`, security headers, request-body rejection, Basic challenge and auth-failure accounting.
3. Lifecycle: pre-bound startup (port collisions fail without partial process), daemon remains up beyond 30 seconds, clean signal shutdown and typed completion, forced drain/error/panic classification, no lost/detached lifetime.
4. Check CLI↔server gRPC sync smoke independently to detect unrelated errors from lockfile graph changes.

Acceptance: tests exercise production paths, no expectation weakening; record any test limitations.

### D — Size, guard, docs, closure

1. Update the architecture guard pinned version and the relevant two architecture docs; reconcile normative §8 version language as noted above.
2. Same-host, same-toolchain, same-profile release build of `snp` and `snip-sync`. Record byte sizes and percentage change against fresh baseline; if materially larger (investigate at approximately >10% for either binary), explain transitive causes and seek narrower feature resolution before acceptance. Do not interpret historical 6,776,320-byte or 3,833,152-byte measurements as current baseline.
3. Run repository gate and targeted platform checks. Write `plans/closure/eggstack-dependency-refresh/001-status.md` with exact tests/results, binaries, dependency tree, commits, remaining risks; mark M001 and registry closed **only when evidence is complete**.

## 8. Failure, cancellation, restart, contention

No new protocol or persisted state. On failed update HTTP request or checksum verification, leave original binary untouched and clean staging output. On server startup bind failure, neither transport may remain running. On shutdown, control triggers drain and completion is observed exactly once; an unexpected endpoint/lifetime failure remains an error. Preserve the current no-retry update transport behavior. Keep concurrency races covered by existing focused tests; do not expand unrelated lock or async architecture.

## 9. Compatibility and migration

Cargo pin/lockfile-only first; no config/data migrations. Maintain existing host targets and Rust 1.94. No proto regeneration or published version bump. If observed source-level differences emerge, record exact symbols and decisions in closure. Historical version strings in accepted closure records remain unchanged; live normative/architecture guides should not assert obsolete pins.

## 10. Required tests

- **Focused unit:** `cargo test -p snip-it --bin snp --features test-support update::tests`; `cargo test -p snip-sync --lib orchestration`; `cargo test --test architecture`.
- **Integration/wire:** `cargo test -p snip-sync --features test-helpers`; `cargo test --test snip_sync_lifetime -- --test-threads=1`. For the intentionally ignored >30s lifetime check: `cargo test --test snip_sync_lifetime -- --ignored --test-threads=1`.
- **Security/negative:** ensure existing handler, metrics auth/CORS, malformed/body rejection and timeout tests are exercised. Add *only* targeted tests if a new regression is found.
- **Interoperability:** existing integration sync client/server path(s); no gRPC wire changes.
- **Cross-platform:** CI compile checks for Linux, macOS, Windows; if unavailable, explicitly label unverified, rather than claiming platform proof.

## 11. Verification commands

```bash
rustc -V; cargo -V
cargo update -p eggfetch-core --precise 0.2.2
cargo update -p eggserve-server --precise 0.4.0
cargo update -p eggserve-primitives --precise 0.2.2
cargo tree -p snip-it -e features
cargo tree -p snip-sync -e features
cargo tree -i eggserve-primitives
cargo check --workspace --all-targets --locked
cargo test -p snip-it --bin snp --features test-support update::tests
cargo test -p snip-sync --lib orchestration
cargo test --test architecture
cargo test -p snip-sync --features test-helpers
cargo test --test snip_sync_lifetime -- --test-threads=1
cargo test --test snip_sync_lifetime -- --ignored --test-threads=1
SNP_ALLOW_PLAINTEXT_API_KEY=true bash scripts/check.sh
SNP_ALLOW_PLAINTEXT_API_KEY=true bash scripts/ci/test-production-seams.sh
cargo build --release -p snip-it --bin snp --locked
cargo build --release -p snip-sync --bin snip-sync --locked
```

If `scripts/check.sh` or specific tests set their own test env, preserve their canonical invocations and record the actual command used. Use `--test-threads=1` for integration/PTY/barrier suites per `AGENTS.md`; run `bash scripts/release-check.sh verify` only for manual clean-tree pre-release verification, not as a prerequisite to this ordinary maintenance milestone.

## 12. Documentation updates

- `architecture/update.md`, `architecture/server.md` live version statements.
- `tests/architecture.rs` exact pin guard, keep lean features and no-curl assertions.
- `plans/000-long-term-specification.md` §8: reconcile static pin minimally without loosening policy. Do not silently rewrite other canonical content.
- `plans/registry.md` only when status changes; closure record under new workstream path.

## 13. Acceptance criteria

1. Pins resolve exactly to 0.2.2 / 0.4.0 / 0.2.2 in the lockfile, no legacy Eggserve primitive copy remains.
2. Full workspace compiles on declared toolchain; published-feature compatibility is proven by actual build.
3. Updater bounded strict redirection/transport, integrity handling and fail-closed fallback still pass existing focused tests.
4. Real-socket health/metrics parity, body rejection, CORS/security/auth and shutdown correctness remain exact.
5. No new code path violates the lean two-binary architecture or introduces Eggfetch in server updater.
6. Release-size delta and dependency-feature delta are documented and explained.
7. Architecture test and live docs agree with the new pins; old closure measurements retain their historical meaning.
8. Successful gated verification, recorded closure, registry status update.

## 14. Stop conditions

Stop and report: an unavoidable upstream API change demands generic frameworks/new crate; HTTP parity cannot be restored without weakening security; a dependency graph requires duplicate incompatible Eggserve primitives; compiler/MSRV incompatibility cannot be resolved within Rust 1.94; binary bloat cannot be accounted for; implementation requires a product/domain/protocol migration; or repository baseline changes significantly before work begins. Do not force a green by dropping tests.

## 15. Closure evidence required

Record commit(s)/PR, exact resolved versions and feature tree, test command+exit status, wire/lifetime regressions, known security limitations, macOS/Windows CI status, both fresh baseline/after byte sizes, any source changes and why, documentation reconciliation, unresolved issues by severity, decision `closed | conditionally closed | corrective required | blocked`; then update roadmap and registry in same closure commit.

## 16. Handoff notes

Read `AGENTS.md`, `AGENTS.override.md`, `.skills/planning.md`, `.skills/server-module.md`, `architecture/{server,update}.md` before editing. Preserve existing user changes and minimalistic architecture. Keep server updater `curl`: the last `eggfetch` server trial was +34.23% and explicitly rejected. Generated `snip-proto` is untouchable. No 0.2.1 Eggserve tag was available during research, so first proof is the published-crates build; the source audit alone does not certify all semantics.

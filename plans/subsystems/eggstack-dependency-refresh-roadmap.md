# Eggstack Dependency Refresh Roadmap

Status: closed (M001 closed; see `plans/closure/eggstack-dependency-refresh/001-status.md`)

Long-term references:

- `plans/000-long-term-specification.md` §§1, 3, 8 (lightweight client/server boundaries and transport policy)
- `plans/001-terminology-and-domain-model.md` (no domain-model changes)
- `plans/002-long-term-roadmap.md` (preserve existing capabilities)

Related decisions and closed prerequisites:

- `plans/subsystems/updater-transport-roadmap.md` M002–M005 (lean eggfetch and intentional server-curl split)
- `plans/subsystems/server-lifecycle-http-roadmap.md` M003–M004 (direct EggServe H1 leaf and socket-level parity)
- `plans/closure/updater-transport/005-status.md`
- `plans/closure/server-lifecycle-http/004-status.md`

No new ADR: this work changes third-party versions without changing the approved ownership architecture. Unexpected contract changes require stopping and reassessing.

## 1. Purpose and ownership boundary

Maintain the three **direct Eggstack dependency pins** across the existing independently versioned `snp` and `snip-sync` products. This is a maintenance/compatibility workstream, not a new transport, service, or application subsystem.

## 2. Classification

- **Invariant:** The updater retains HTTPS-only production URL entry, bounded strict redirects and total transfers, SHA-256 verification, 404-only fallback, and partial-download cleanup. `snip-sync` retains a concrete two-route H1 health/metrics service with Tonic on a separate pre-bound listener, body rejection, typed lifecycle supervision, exact CORS/auth/header behavior, and external TLS.
- **Capability:** No new capability; both existing commands continue operating with identical user-visible semantics.
- **Infrastructure (primary):** Exact-version dependency and lockfile update, limited compatibility fixes where proven necessary.
- **Polish:** Remove stale version text, update version-pinning guards, record same-host artifact sizes and compatibility evidence.

## 3. Non-goals

No Eggress introduction, HTTP/2/3, TLS termination in `snip-sync`, Axum/Tower adoption, new workspace crates/adapter abstractions, blanket dependency refresh, protocol/schema changes, codegen, or switching server self-update from `curl` to Eggfetch. No release publication as part of this milestone.

## 4. Current state and researched upstream surface (2026-10-08)

- `Cargo.toml` pins `eggfetch-core =0.2.0` with `default-features = false`, `standard-http1`, `redirects`, `tls-rustls`, `tls-native-roots`.
- `snip-sync/Cargo.toml` pins `eggserve-server =0.2.1` and `eggserve-primitives =0.2.1`, defaults off.
- Target releases: `eggfetch-core 0.2.2`, `eggserve-server 0.4.0`, `eggserve-primitives 0.2.2`; server 0.4.0 directly depends on primitives 0.2.2.
- At the current upstream sources, `snp`'s `Client::builder/get`, `RedirectPolicy::strict`, `Timeout::builder/total`, request `max_decoded_body_size/timeout/send`, response `bytes/bytes_stream`, and typed timeout/body-size errors still exist.
- At the current upstream sources, the direct EggServe `service_fn_head`, `RuntimeConfig::builder/disable_connection_total_timeout/graceful_shutdown_timeout`, `Server::builder/from_listener/start_with_service`, `ServerHandle::into_parts`, `ServerControl::shutdown`, `ServerCompletion::wait`, and canonical request/response types still exist.
- Thus **no consumer API rewrite is known to be required**, but compatibility is unproven until the published crates resolve, build, and pass exact wire/lifetime/transfer tests.
- EggServe 0.4.0 has additional H1 runtime/response-policy behavior; its Tower feature change does not directly affect our default-features-off leaf.
- `tests/architecture.rs`, `architecture/update.md`, `architecture/server.md`, and `plans/000-long-term-specification.md` contain historical fixed version text. The last is canonical: implementation must update its version statement narrowly without weakening normative transport policy, and document the requested version change as the rationale.

Sources: Snip-it root and server Cargo manifests; Eggfetch `crates/eggfetch-core/{Cargo.toml,src/{client,request,response,redirect,timeout,error}.rs}`; EggServe `crates/eggserve-{server,primitives}/Cargo.toml`, `eggserve-server/src/{lib,config,service}.rs` on main; EggServe v0.4.0 release notes. Date is research date, not build validation.

## 5. Target architecture

Exactly the prior architecture with three refreshed pins and one lockfile, without extraneous transitively enabled HTTP features or duplicate EggServe primitive versions.

## 6. Dependency graph

```text
Closed: updater-transport M005 + server-lifecycle-http M004
                    |
                    v
M001: coordinated eggfetch/eggserve version adoption and parity
                    |
                    v
Closure record / release qualification (not automatic publication)
```

All prior milestones are hard closed prerequisites. Registry handoff is ready. No external interface dependency is awaiting design.

## 7. Milestone

### M001 — Bump and qualify Eggstack direct crates

Class: infrastructure. Status: closed.

Dependencies: closed updater M005, server lifecycle M004; upstream 0.2.2/0.4.0 publication.

Deliverable: pins, lockfile, necessary minimal consumer fixes, focused tests, documentation/architecture guard reconciliation, measured artifacts, closure report.

Implementation: `plans/implementation/eggstack-dependency-refresh/001-upgrade-eggfetch-and-eggserve.md`.

Closure: `plans/closure/eggstack-dependency-refresh/001-status.md` (accepted; M001 closed).

Exit: both binaries build, all qualified behavior remains, no unjustified binary-size or dependency-graph expansion, and registry/closure evidence is accepted.

## 8. Cross-cutting requirements

**Storage / protocol:** Zero migrations. No changes to SQLite schema, protobuf, sync identity/merge, auth key storage, or release tags.

**Security:** Preserve TLS/redirect checks, credential header behavior, metrics Basic authentication, request-body rejection, allowed origins, and security headers.

**Concurrency / cancellation:** Preserve typed EggServe shutdown authority and bounded draining; preserve request-total timeout and partial staging-file deletion.

**Performance:** Compare same-host, same-toolchain, same-profile release artifacts against original dependency baseline. Investigate meaningful size/graph regressions before acceptance; no unreviewed server updater transport trial.

**Operations:** Rust 1.94 toolchain; Linux plus macOS and Windows supported targets remain in scope for compilation.

## 9. Verification strategy

Narrow updater and HTTP/lifecycle tests, lockfile graph inspection, `cargo check --workspace --all-targets --locked`, `scripts/check.sh`, `scripts/ci/test-production-seams.sh`, targeted real-socket regression tests, release binary measurements, and platform CI where available. Record unrun checks honestly.

## 10. Risks and decisions

- Source-level signature presence does not prove all published-feature combinations compile.
- EggServe may change wire-level normalization/defaults despite compatible method names.
- Multiple `eggserve-primitives` versions could enter the graph unless exact pins are coordinated.
- A new transitive dependency or default feature could erase prior small-binary gains.
- If implementation requires a large rearchitecture or weakens parity, stop and open a separate decision/corrective; do not silently broaden M001.

## 11. Completion definition

M001 closes only after its required tests, size evaluation, current docs, and closure record are accepted and `plans/registry.md` is updated from ready to closed.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blocker |
|---|---|---|---|---|
| M001 coordinated upgrade | closed | `plans/implementation/eggstack-dependency-refresh/001-upgrade-eggfetch-and-eggserve.md` | `plans/closure/eggstack-dependency-refresh/001-status.md` (accepted) | None |

# Server Lifecycle and HTTP Runtime Roadmap

Status: closed

Long-term references:

- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#8-transport-and-runtime-policy`
- `plans/001-terminology-and-domain-model.md#7-server-terms`
- `plans/002-long-term-roadmap.md#phase-1-server-lifecycle-and-lean-http-runtime`
- `plans/003-planning-process.md`

Related ADRs: none required. The EggServe leaf selection was a
measurement-gated runtime choice within the existing lifecycle and wire
contract, not a new protocol. If a future change adds a protocol or
topology, open an ADR.

## 1. Purpose and ownership boundary

This subsystem owns `snip-sync` startup/lifecycle management and the HTTP
runtime (Axum-preserving trial vs direct EggServe leaf vs parity closure).
It consumes the distribution artifact baseline for size gates and the
lifecycle primitives for OS wrappers. It MUST NOT own updater transport
(see `updater-transport`), release publication (see
`distribution-release`), or snippet sync semantics.

## 2. Work classification

### Invariants

- `snip-sync` remains one foreground server binary; Tonic owns gRPC on its
  listener; both listeners pre-bind before service startup.
- Proven wire parity: empty 404/405, `Allow: GET, HEAD`, `Vary: origin`
  discipline, preflight security-header boundary.
- External TLS termination; explicit body rejection; typed shutdown.

### Capabilities

- `serve`/`stop`/`restart`/`croncheck`/`/health` lifecycle usable by
  systemd/launchd/cron/Task Scheduler wrappers.

### Infrastructure

- HTTP runtime ownership (EggServe trial vs direct leaf) gated by the 10%
  footprint rule.

### Polish

- Socket-level parity regression assertions and completed A/B/C evidence.

## 3. Non-goals

- New daemon/process-control architectures.
- Generic router/middleware frameworks.
- Reintroducing Axum/Tower-HTTP/core once the leaf winner is proven.
- Updater transport or installer changes.

## 4. Current state

All milestones are closed. Plan 018 completed the Axum-preserving
measurement trial (B +3.41% vs A). Plan 019 selected the direct leaf graph
(C +1.70% vs A, -1.65% vs B; 13 unique packages removed from B) as the
final production architecture. Plan 020 restored the historical CORS `Vary`
semantics and locked 404/405, preflight `Allow`, and security-header
behavior with real-socket tests. Implementation commit `10aeb994` plus
hosted run `36011491151` (Linux correctness, Windows/macOS smoke) and
local `scripts/check.sh` plus production-seam checks are recorded.

## 5. Target architecture

```text
final production architecture
-----------------------------
Tonic -> gRPC listener
EggServe leaf runtime -> HTTP/1 health/metrics listener
```

One concrete native health/metrics service using only `eggserve-server`
0.2.1 plus `eggserve-primitives` 0.2.0. No direct Axum/Tower-HTTP/core
dependencies.

## 6. Dependency graph

```text
M001 startup + lifecycle
    |
    `--> M002 Axum-preserving trial (hard: M001 + artifact baseline)
              |
              `--> M003 direct leaf consolidation (hard: M002 A/B result)
                        |
                        `--> M004 parity + closure corrective (hard: M002, M003)
```

All dependencies are hard. M003 MUST NOT begin until M002 records its A/B
result. Exactly one HTTP architecture remains after the comparison.

## 7. Milestones

### M001 — snip-sync startup and lifecycle management

Class: capability + invariant

Objective: establish reusable lifecycle primitives and singleton ownership.

Dependencies: distribution artifact baseline (soft; size gates not yet
active).

Deliverable boundary: lifecycle commands plus lock/identity checks; no HTTP
runtime change.

User or operator value: operators manage one server binary across OS
wrappers.

Exit conditions: `serve`/`stop`/`restart`/`croncheck`/`/health` reusable;
singleton lock authoritative.

Deferred work: HTTP runtime experiments (M002-M004).

Implementation plan:

- `plans/implementation/server-lifecycle-http/001-snip-sync-startup-and-lifecycle.md`

Closure record:

- `plans/closure/server-lifecycle-http/001-status.md`

Predecessor flat plan: `plans/archive/flat-003-snip-sync-startup-and-lifecycle.md`.

### M002 — EggServe Axum-preserving runtime adoption trial

Class: infrastructure

Objective: measure HTTP-runtime-only ownership change while preserving the
Axum application surface.

Dependencies: hard M001 plus artifact baseline.

Deliverable boundary: runtime ownership only; `eggserve-core 0.2.2[tower]`
plus `eggserve-server 0.2.1`; no TLS/HTTP2/HTTP3; revert if growth >10%.

User or operator value: none directly; measurement evidence for M003.

Exit conditions: fresh same-environment A/B sizes recorded (A 3,833,152;
B 3,963,968).

Deferred work: leaf selection (M003).

Implementation plan:

- `plans/implementation/server-lifecycle-http/002-eggserve-axum-runtime-adoption-trial.md`

Closure record:

- `plans/closure/server-lifecycle-http/002-status.md`

Predecessor flat plan: `plans/archive/flat-018-eggserve-0.2.2-axum-runtime-adoption-trial.md`.

### M003 — Direct EggServe leaf HTTP service consolidation

Class: infrastructure

Objective: test the smaller direct leaf graph and select exactly one
production architecture.

Dependencies: hard M002 A/B result.

Deliverable boundary: one concrete native service; remove Axum/Tower-HTTP/
core only when proven unused; keep the winner only under the <=10% gate.

User or operator value: smaller dependency surface with identical
health/metrics behavior.

Exit conditions: A/B/C sizes recorded; C selected (3,898,408 bytes).

Deferred work: parity gaps become M004.

Implementation plan:

- `plans/implementation/server-lifecycle-http/003-direct-eggserve-leaf-http-service-consolidation.md`

Closure record:

- `plans/closure/server-lifecycle-http/003-status.md`

Predecessor flat plan: `plans/archive/flat-019-direct-eggserve-leaf-http-service-consolidation.md`.

### M004 — EggServe HTTP parity and closure corrective

Class: polish (corrective)

Objective: restore the exact CORS/404/405/preflight/security-header
contract and finish M002/M003 evidence.

Dependencies: hard M002, M003.

Deliverable boundary: parity restoration plus socket assertions plus
registry completion; no architecture reopening, no dependency
reintroduction, no unrelated cleanup.

User or operator value: byte-identical observable HTTP semantics with a
smaller runtime.

Exit conditions: `Vary: origin` discipline, empty 404/405, preflight
`Allow`, header boundary all socket-proven; A/B/C evidence completed.

Deferred work: none; workstream closed.

Implementation plan:

- `plans/implementation/server-lifecycle-http/004-eggserve-http-parity-and-closure-corrective.md`

Closure record:

- `plans/closure/server-lifecycle-http/004-status.md`

Predecessor flat plan: `plans/archive/flat-020-eggserve-http-parity-and-closure-corrective.md`.

## 8. Cross-cutting requirements

### Storage and migration

Preserve server database and startup registration; no schema migration in
this workstream.

### Protocol and compatibility

gRPC untouched; HTTP health/metrics/CORS/header/method/HEAD/lifecycle
contracts preserved exactly.

### Security and authorization

Basic-auth constant-time comparison, configured CORS, three security
headers, external TLS termination preserved.

### Concurrency, cancellation, and recovery

Pre-bound listeners, supervised HTTP via typed control/completion API,
disabled total connection-lifetime ceiling.

### Observability and audit

Health/metrics endpoints plus socket-level parity tests.

### Performance and resource use

10% same-environment release-size gate is authoritative; revert on breach.

### Documentation and operations

`architecture/server.md` and HTTP policy notes updated with the leaf
outcome.

## 9. Verification strategy

Lifecycle start/stop/restart tests, real-socket parity assertions,
same-toolchain A/B/C size measurements, `scripts/check.sh`,
production-seam checks, hosted run `36011491151`.

## 10. Risks and decision points

No open decisions. Any future HTTP framework, router, TLS-mode, or
topology change requires a new milestone and, if protocol-crossing, an ADR.

## 11. Completion definition

This workstream closes when M001-M004 each have accepted closure evidence
with C as the final architecture, parity socket-proven, and no unresolved
medium-or-higher finding. All milestones are closed; the roadmap is closed.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 startup + lifecycle | closed | `plans/implementation/server-lifecycle-http/001-snip-sync-startup-and-lifecycle.md` | `plans/closure/server-lifecycle-http/001-status.md` | — |
| M002 Axum-preserving trial | closed | `plans/implementation/server-lifecycle-http/002-eggserve-axum-runtime-adoption-trial.md` | `plans/closure/server-lifecycle-http/002-status.md` | — |
| M003 direct leaf consolidation | closed | `plans/implementation/server-lifecycle-http/003-direct-eggserve-leaf-http-service-consolidation.md` | `plans/closure/server-lifecycle-http/003-status.md` | — |
| M004 parity + closure corrective | closed | `plans/implementation/server-lifecycle-http/004-eggserve-http-parity-and-closure-corrective.md` | `plans/closure/server-lifecycle-http/004-status.md` | — |

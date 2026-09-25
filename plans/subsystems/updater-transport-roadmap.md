# Updater Transport Roadmap

Status: closed

Long-term references:

- `plans/000-long-term-specification.md#8-transport-and-runtime-policy`
- `plans/000-long-term-specification.md#9-distribution-and-release-invariants`
- `plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms`
- `plans/002-long-term-roadmap.md#phase-2-binary-first-self-update-and-lean-transport`
- `plans/003-planning-process.md`

Related ADRs: none required. The eggfetch-lean vs `curl` split was a
measured footprint/trust decision inside the existing updater contract. If
a future change alters redirect trust, timeout ownership, or fallback
classification, open an ADR.

## 1. Purpose and ownership boundary

This subsystem owns `snp` self-update behavior and its HTTP transport
(eggfetch consolidation, timeout corrective, lean-profile adoptions, and
the controlled `snip-sync` requalification trial). It consumes release
identity (exact tags, never GitHub `latest`) and the `curl` baseline for
`snip-sync`. It MUST NOT own release publication (see
`distribution-release`), server HTTP runtime (see
`server-lifecycle-http`), or general HTTP abstractions.

## 2. Work classification

### Invariants

- HTTPS-only production redirects; bounded metadata/binary transfers;
  404-only Cargo fallback; lightweight `snip-sync` footprint.
- No manual redirect machine and no duplicate outer timeout once
  delegation lands; `tests/architecture.rs` pins hold.

### Capabilities

- Binary-first self-update and restart integration for `snp`.

### Infrastructure

- Lean `eggfetch-core` profile with delegated strict redirects and native
  `Timeout.total`.

### Polish

- Slow-drip regression tests, partial-file cleanup, and measurement
  records.

## 3. Non-goals

- Shared updater crates, general HTTP abstractions, broad eggfetch feature
  enablement, async-main rewrites.
- Advanced routing, retry, Basic auth, proxy, HTTP2/3, JSON, compression,
  cookies, multipart, tracing, or test-util features.
- `snip-sync` transport migration unless the 10% trial passes.
- Installer, release-workflow, or architecture changes inside corrective
  passes.

## 4. Current state

All milestones are closed. `snp update` pins `eggfetch-core 0.2.0` on the
unchanged lean profile with no `src/update.rs` changes required at
adoption and all focused updater tests passing. The fresh same-host
release build stayed byte-identical at 6,776,320 bytes. The bounded
`snip-sync` lean trial measured 5,145,224 bytes vs the 3,833,152-byte curl
baseline (+34.23%), so the server remains on `curl` and no temporary
transport code was retained.

## 5. Target architecture

`snp` fetches via in-process lean eggfetch: `safe_get` owns redirect
traversal only in the pre-delegation baseline; after delegation,
`fetch_bytes_with`/`fetch_file_with` each apply one injected overall
timeout around traversal plus complete final-body consumption, with
`FetchError::Failed` on timeout and partial staging files removed on
cancellation. `snip-sync` retains its external `curl` adapter as an
intentional tradeoff.

## 6. Dependency graph

```text
M001 binary-first self-update
    |
    `--> M002 transport consolidation (hard: M001)
              |
              `--> M003 timeout corrective (hard: M002)
                        |
                        `--> M004 0.1.7 lean adoption (hard: M002, M003)
                                  |
                                  `--> M005 0.2.0 adoption + server trial (hard: M004)
```

All dependencies are hard. M005's `snip-sync` leg is measurement-only and
MUST NOT retain transport code when the gate fails.

## 7. Milestones

### M001 — Binary-first self-update and restart integration

Class: capability

Objective: deliver binary-first update plus restart through the installed
CLI.

Dependencies: release identity (interface; exact-tag construction).

Deliverable boundary: updater plus restart; no transport consolidation.

User or operator value: fleet hosts update without source builds.

Exit conditions: verified-binary default, exact-version Cargo fallback,
hard integrity failure, state preservation.

Deferred work: transport consolidation (M002).

Implementation plan:

- `plans/implementation/updater-transport/001-binary-first-self-update.md`

Closure record:

- `plans/closure/updater-transport/001-status.md`

Predecessor flat plan: `plans/archive/flat-004-binary-first-self-update.md`.

### M002 — eggfetch self-update transport consolidation

Class: infrastructure

Objective: split transports by measurement: lean in-process eggfetch for
`snp`, retained `curl` for `snip-sync`.

Dependencies: hard M001.

Deliverable boundary: transport ownership plus trust profile; no general
HTTP abstraction or shared updater crate.

User or operator value: smaller, safer `snp` transfers with the server
footprint preserved.

Exit conditions: lean profile measured (+11.5% kept for `snp`; +41%
rejected for server on the older profile).

Deferred work: timeout corrective (M003).

Implementation plan:

- `plans/implementation/updater-transport/002-eggfetch-self-update-transport-consolidation.md`

Closure record:

- `plans/closure/updater-transport/002-status.md`

Predecessor flat plan: `plans/archive/flat-014-eggfetch-self-update-transport-consolidation.md`.

### M003 — snp updater end-to-end timeout corrective

Class: polish (corrective)

Objective: fix the narrow regression where the wall-clock timeout ended
before final-body consumption.

Dependencies: hard M002.

Deliverable boundary: move the overall deadline around traversal plus
complete body consumption; guarantee partial-file cleanup; add slow-drip
tests. No `snip-sync`, dependency, installer, workflow, proxy, retry, or
runtime changes.

User or operator value: slow-drip bodies cannot outlast the budget and
never leave partial artifacts.

Exit conditions: metadata/binary slow-drip regression tests prove steady
progress cannot extend the total budget.

Deferred work: none beyond adoption passes.

Implementation plan:

- `plans/implementation/updater-transport/003-snp-updater-end-to-end-timeout-corrective.md`

Closure record:

- `plans/closure/updater-transport/003-status.md`

Predecessor flat plan: `plans/archive/flat-015-snp-updater-end-to-end-timeout-corrective.md`.

### M004 — eggfetch 0.1.7 lean updater adoption

Class: infrastructure (deletion/footprint pass)

Objective: adopt 0.1.7 on the lean standard-route profile and delete the
superseded local machinery.

Dependencies: hard M002, M003.

Deliverable boundary: exact-version/lockfile bump plus deletions (manual
redirect machine, duplicate outer timeout); preserve HTTPS guard, body
limits, 404-only fallback, partial cleanup. Controlled A/B/C size record
required.

User or operator value: same updater behavior with less code and a smaller
binary (-2.82% measured).

Exit conditions: lean 0.1.7 final 6,776,320 bytes; `snip-sync` curl path
untouched.

Deferred work: 0.2.0 adoption (M005).

Implementation plan:

- `plans/implementation/updater-transport/004-eggfetch-0.1.7-lean-updater-adoption.md`

Closure record:

- `plans/closure/updater-transport/004-status.md`

Predecessor flat plan: `plans/archive/flat-016-eggfetch-0.1.7-lean-updater-adoption.md`.

### M005 — eggfetch 0.2.0 updater adoption and server requalification

Class: infrastructure

Objective: adopt coordinated 0.2.0 for the migrated `snp` path and run one
controlled measurement-only `snip-sync` trial against the current lean
profile.

Dependencies: hard M004.

Deliverable boundary: narrow bump plus qualification; retain temporary
trial code only if the server gate passes (it did not).

User or operator value: supported dependency floor stays current without
server bloat.

Exit conditions: `snp` byte-identical at 6,776,320 bytes; server trial
+34.23% recorded and discarded; server stays on `curl`.

Deferred work: none; workstream closed.

Implementation plan:

- `plans/implementation/updater-transport/005-eggfetch-0.2.0-updater-adoption-and-server-requalification.md`

Closure record:

- `plans/closure/updater-transport/005-status.md`

Predecessor flat plan: `plans/archive/flat-017-eggfetch-0.2.0-updater-adoption-and-server-requalification.md`.

## 8. Cross-cutting requirements

### Storage and migration

Preserve config/data/credentials across updates; remove partial staging
files on timeout/cancellation.

### Protocol and compatibility

Exact-tag version authority via crates.io metadata; GitHub `latest` never
authoritative; 404-only Cargo fallback preserved.

### Security and authorization

Initial-HTTPS guard, strict bounded redirects, body limits, no silent
elevation.

### Concurrency, cancellation, and recovery

One injected overall timeout per operation; cancellation removes partial
files.

### Observability and audit

Updater measurements and slow-drip test evidence recorded in plans.

### Performance and resource use

10% server-size gate authoritative; lean profile is the only enabled
feature set.

### Documentation and operations

Transport split and trust-profile decisions recorded as intentional
tradeoffs, not unfinished work.

## 9. Verification strategy

Focused updater suites (21 tests at adoption), slow-drip fixtures,
partial-cleanup assertions, A/B/C release-size measurements,
`scripts/check.sh`, production-seam checks, hosted runs where recorded.

## 10. Risks and decision points

No open decisions. Any future transport-family, trust-profile, or
fallback-classification change needs a new milestone and, if
contract-crossing, an ADR.

## 11. Completion definition

This workstream closes when M001-M005 each have accepted closure evidence
with `snp` on lean 0.2.0, `snip-sync` on `curl` by measurement, timeouts
covering full-body consumption, and no unresolved medium-or-higher
finding. All milestones are closed; the roadmap is closed.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 binary-first self-update | closed | `plans/implementation/updater-transport/001-binary-first-self-update.md` | `plans/closure/updater-transport/001-status.md` | — |
| M002 transport consolidation | closed | `plans/implementation/updater-transport/002-eggfetch-self-update-transport-consolidation.md` | `plans/closure/updater-transport/002-status.md` | — |
| M003 timeout corrective | closed | `plans/implementation/updater-transport/003-snp-updater-end-to-end-timeout-corrective.md` | `plans/closure/updater-transport/003-status.md` | — |
| M004 0.1.7 lean adoption | closed | `plans/implementation/updater-transport/004-eggfetch-0.1.7-lean-updater-adoption.md` | `plans/closure/updater-transport/004-status.md` | — |
| M005 0.2.0 adoption + trial | closed | `plans/implementation/updater-transport/005-eggfetch-0.2.0-updater-adoption-and-server-requalification.md` | `plans/closure/updater-transport/005-status.md` | — |

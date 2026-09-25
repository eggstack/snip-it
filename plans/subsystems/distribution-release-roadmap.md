# Distribution and Release Roadmap

Status: closed

Long-term references:

- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#9-distribution-and-release-invariants`
- `plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms`
- `plans/002-long-term-roadmap.md#phase-0-distribution-fleet-deployment-and-release-closure`
- `plans/003-planning-process.md`

Related ADRs: none required. This work does not change release identity,
publication authority, or installer trust policy. If a future change does,
open an ADR instead of widening this workstream.

Predecessor evidence (pre-convention flat plans, retained in
`plans/archive/`):

- `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md` —
  canonical predecessor direction (distribution half; MCP half belongs to
  `mcp-integration`).

## 1. Purpose and ownership boundary

This subsystem owns the release binary matrix, artifact contract,
bootstrap installers, platform CI closure, publication evidence, and the
binary-publication corrective. It consumes the `snip-it`/`snip-sync`
independent versioning contract and the single release workflow. It MUST
NOT own updater transport policy (see `updater-transport`), server
lifecycle (see `server-lifecycle-http`), or MCP behavior (see
`mcp-integration`).

## 2. Work classification

### Invariants

- Independent crate versions with independent exact tags (`vX.Y.Z` vs
  `snip-sync-vA.B.C`); published releases immutable.
- Manual publish only; no crates.io credentials in GitHub Actions.
- Integrity failure is hard failure with no Cargo fallback; no silent
  `sudo`/elevation.

### Capabilities

- Prebuilt release executables plus checksums for the five-target matrix.
- Binary-first bootstrap installers with exact-version Cargo fallback.
- Consumer smoke proving install plus update across supported targets.

### Infrastructure

- Release workflow and artifact naming owned once; no matrix duplication.

### Polish

- Installer diagnostics and failure-mode messaging.

## 3. Non-goals

- Package-manager distribution (apt/dnf/Homebrew/Winget/Chocolatey/Snap).
- Auto-update daemons, notifications, differential updates, signing
  infrastructure.
- Updater HTTP transport selection (owned by `updater-transport`).
- Server runtime selection (owned by `server-lifecycle-http`).

## 4. Current state

All milestones are closed. `snip-it 1.3.8` published with all five public
`snp` binaries plus checksums (historical `v1.3.7` untouched). Consumer
smoke exercises both independently versioned components across Linux
x86_64/ARM64, macOS Intel/Apple Silicon, and Windows x86_64 plus the exact
README bootstrap. Bash/PowerShell installer failure-mode contracts are
covered deterministically. No second release workflow, matrix duplication,
or new dependencies were introduced.

## 5. Target architecture

One release workflow builds stable asset names (`snp-<target>`,
`snip-sync-<target>`, plus `.sha256`) under independent exact tags.
Installers detect host, verify integrity, install prebuilt binaries, and
fall back to exact-version Cargo only when needed. Historical releases are
never mutated.

## 6. Dependency graph

```text
M001 release matrix + artifact contract
    |
    +--> M002 bootstrap installers
    |         |
    |         +--> M004 publication + distribution closure
    |                   |
    |                   `--> M005 binary-publication corrective (hard: M001, M002, M004)
    |
    `--> M003 Windows CI + platform closure (hard: M001, M002)
              |
              `--> M004 publication + distribution closure
```

M003/M004 ordering: M003 restores the ordinary platform gate; M004
completes publication evidence. M005 is a corrective pass on M001/M002/M004
evidence. All dependencies are hard except consumer-smoke reuse (soft).

## 7. Milestones

### M001 — Release binary matrix and artifact contract

Class: capability + invariant

Objective: fix the five-target matrix, independent tags, and stable asset
contract.

Dependencies: none (baseline).

Deliverable boundary: matrix plus tag/asset/checksum contract; no
installer or publication work.

User or operator value: operators know exactly which assets to expect for
each crate version.

Exit conditions: contract recorded and consumed by installers and the
release workflow.

Deferred work: installer implementation (M002), platform closure (M003).

Implementation plan:

- `plans/implementation/distribution-release/001-release-binary-matrix-and-artifact-contract.md`

Closure record:

- `plans/closure/distribution-release/001-status.md`

Predecessor flat plan: `plans/archive/flat-001-release-binary-matrix-and-artifact-contract.md`.

### M002 — Binary-first bootstrap installers

Class: capability

Objective: ship Bash/PowerShell bootstrap installers with verified-binary
default and exact-version Cargo fallback.

Dependencies: hard M001.

Deliverable boundary: installers plus failure-mode contracts; no release
publication.

User or operator value: fleet hosts install without source compilation on
common targets.

Exit conditions: deterministic failure-mode coverage; no silent elevation;
config/data/credentials preserved.

Deferred work: publication evidence (M004).

Implementation plan:

- `plans/implementation/distribution-release/002-bootstrap-installers.md`

Closure record:

- `plans/closure/distribution-release/002-status.md`

Predecessor flat plan: `plans/archive/flat-002-bootstrap-installers.md`.

### M003 — Windows CI and platform closure

Class: infrastructure + polish

Objective: restore the ordinary Windows all-target/platform-smoke gate.

Dependencies: hard M001, M002.

Deliverable boundary: CI/platform evidence only; no new distribution
features.

User or operator value: Windows stays a first-class supported target.

Exit conditions: Windows all-target plus platform smoke green alongside
Linux/macOS.

Deferred work: none; enables M004.

Implementation plan:

- `plans/implementation/distribution-release/003-windows-ci-platform-closure.md`

Closure record:

- `plans/closure/distribution-release/003-status.md`

Predecessor flat plan: `plans/archive/flat-006-windows-ci-platform-closure.md`.

### M004 — Release publication and distribution closure

Class: capability

Objective: complete publication and end-to-end distribution evidence
without weakening release immutability.

Dependencies: hard M003 (and transitively M001, M002).

Deliverable boundary: publication plus consumer smoke; no retrofits onto
historical releases.

User or operator value: a published release is installable end-to-end on
all targets.

Exit conditions: publication evidence plus five-target consumer smoke.

Deferred work: corrective gaps become M005.

Implementation plan:

- `plans/implementation/distribution-release/004-release-publication-and-distribution-closure.md`

Closure record:

- `plans/closure/distribution-release/004-status.md`

Predecessor flat plan: `plans/archive/flat-007-release-publication-and-distribution-closure.md`.

### M005 — snp binary publication and installer corrective closure

Class: polish (corrective)

Objective: close the narrow publication/installer gaps without a second
workflow or matrix duplication.

Dependencies: hard M001, M002, M004.

Deliverable boundary: symmetric consumer/install testing plus the next
legitimate `snip-it` release; no historical-release mutation.

User or operator value: the published `snp` set is complete and the
installer contracts hold.

Exit conditions: `1.3.8` publishes five binaries plus checksums;
historical `1.3.7` untouched; installer failure modes deterministic.

Deferred work: none; workstream closed.

Implementation plan:

- `plans/implementation/distribution-release/005-snp-binary-publication-and-installer-corrective-closure.md`

Closure record:

- `plans/closure/distribution-release/005-status.md`

Predecessor flat plan: `plans/archive/flat-013-snp-binary-publication-and-installer-corrective-closure.md`.

## 8. Cross-cutting requirements

### Storage and migration

Preserve existing user configuration, snippet data, server database,
credentials, and startup registration during install. No migration in this
workstream.

### Protocol and compatibility

Asset/tag contract is the compatibility surface; historical tags MUST NOT
change meaning.

### Security and authorization

Checksums verified before install; TLS failures, 5xx, wrong version, and
checksum mismatch are hard failures. No credentials in CI.

### Concurrency, cancellation, and recovery

Installers MUST NOT leave partial binaries on failure; no concurrent
release mutation.

### Observability and audit

Consumer smoke and installer failure modes produce deterministic evidence.

### Performance and resource use

No new runtime cost; installers stay shell-only.

### Documentation and operations

README bootstrap stays exact; `CONTRIBUTING.md`/`RELEASING.md` own the
publish procedure.

## 9. Verification strategy

Release-matrix checks, installer failure-mode fixtures, five-target
consumer smoke, `scripts/check.sh`, production-seam checks, and hosted
platform runs where recorded in closure records.

## 10. Risks and decision points

No open decisions. Any future change to release identity, signing, or
package-manager distribution requires an ADR and a new subsystem milestone,
not a silent extension of this closed roadmap.

## 11. Completion definition

This workstream closes when M001-M005 each have accepted closure evidence
proving the matrix, installers, platform gate, publication, and corrective
are complete with no unresolved medium-or-higher finding. All milestones
are closed; the roadmap is closed.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 release matrix + artifact contract | closed | `plans/implementation/distribution-release/001-release-binary-matrix-and-artifact-contract.md` | `plans/closure/distribution-release/001-status.md` | — |
| M002 bootstrap installers | closed | `plans/implementation/distribution-release/002-bootstrap-installers.md` | `plans/closure/distribution-release/002-status.md` | — |
| M003 Windows CI + platform closure | closed | `plans/implementation/distribution-release/003-windows-ci-platform-closure.md` | `plans/closure/distribution-release/003-status.md` | — |
| M004 publication + distribution closure | closed | `plans/implementation/distribution-release/004-release-publication-and-distribution-closure.md` | `plans/closure/distribution-release/004-status.md` | — |
| M005 binary-publication corrective | closed | `plans/implementation/distribution-release/005-snp-binary-publication-and-installer-corrective-closure.md` | `plans/closure/distribution-release/005-status.md` | — |

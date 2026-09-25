> **Planning-convention migration note.** This file was promoted via `git mv` from pre-convention flat plan `013-snp-binary-publication-and-installer-corrective-closure.md` (history preserved). Its body is unchanged historical evidence.
> New-convention identity: Distribution and Release M005 — snp binary publication and installer corrective closure.
> Source roadmap: `plans/subsystems/distribution-release-roadmap.md` (M005).
> Long-term requirements: `plans/000-long-term-specification.md`, `plans/001-terminology-and-domain-model.md`, `plans/002-long-term-roadmap.md`.
> Primary class: polish. Dependencies: hard M001, M002, M004.
> Status vocabulary mapping: pre-convention `Status: complete` means `closed` with closure record at `plans/closure/distribution-release/005-status.md`.
> Pre-convention archive pointer: `plans/archive/flat-013-snp-binary-publication-and-installer-corrective-closure.md`.

---

# Plan 013: snp binary publication and installer corrective closure

Status: complete

Depends on: Plans 001, 002, and 007 (complete)

## Objective

Close the remaining asymmetry in the binary distribution path without adding a second release architecture.

The repository already knows how to build `snp` release binaries. The corrective work is to:

1. prove the `snp` path with a legitimate public GitHub Release that contains the expected binaries;
2. make the public distribution consumer smoke exercise both independently versioned components instead of only `snip-sync`;
3. bring the installer tests up to the failure-mode contract already documented by Plan 002;
4. give the PowerShell installer equivalent practical coverage;
5. leave the existing single release workflow, tag namespaces, published-release immutability, and Cargo fallback policy intact.

This is a closure/corrective pass, not a redesign.

## Current-state finding

As of 2026-09-16:

- `Cargo.toml` defines package `snip-it` version `1.3.7` with binary `snp`.
- GitHub release `v1.3.7` predates the current binary release workflow and contains no attached binary assets.
- `.github/workflows/release-binaries.yml` already supports both components:
  - `vX.Y.Z` -> package `snip-it`, binary/component `snp`;
  - `snip-sync-vA.B.C` -> package/binary `snip-sync`.
- The release workflow already builds five targets for whichever component the tag selects:
  - `x86_64-unknown-linux-gnu`;
  - `aarch64-unknown-linux-gnu`;
  - `x86_64-apple-darwin`;
  - `aarch64-apple-darwin`;
  - `x86_64-pc-windows-msvc`.
- Its aggregate job already requires five `snp-*` executables plus five `.sha256` sidecars when the selected component is `snp`.
- Historical non-mutating validation for `v1.3.7` successfully exercised the five-target `snp` build/aggregate path.
- The workflow correctly refuses to mutate an already-published GitHub Release, so `v1.3.7` was not retrofitted with assets.
- `snip-sync-v0.1.5` was released after this machinery existed and does contain the expected public binary/checksum set.
- `.github/workflows/distribution-consumer-smoke.yml` currently exercises only `snip-sync`, despite the bootstrap installers supporting both components.
- `scripts/tests/installers.sh` checks useful Bash mapping/argument helpers but does not cover several failure-mode cases Plan 002 explicitly required.
- `packaging/install.ps1` has no equivalent focused contract suite; its public consumer proof currently covers only `-Component Server`.

The visible absence of public `snp` binaries is therefore mostly a release-history/coverage gap, not a missing `snp` build implementation.

## Governing constraints

1. Do **not** create a separate `snp` release workflow.
2. Do **not** duplicate the five-target matrix.
3. Preserve component-specific version/tag namespaces:
   - `snip-it` / `snp`: `vX.Y.Z`;
   - `snip-sync`: `snip-sync-vA.B.C`.
4. Preserve published-release immutability. Never solve the historical `v1.3.7` gap by allowing Actions to overwrite a published release.
5. Do not manufacture or retag `v1.3.7` solely to attach binaries.
6. Crates.io remains the normal unpinned version authority for bootstrap/update behavior.
7. Preserve exact-version Cargo fallback only for source-only targets or a definite missing release asset where the existing installer contract permits it.
8. Checksum, transport, identity, and version failures must remain hard failures rather than silently compiling from source.
9. Keep the project lightweight. Do not add an installer framework, package manager abstraction, signing framework, release service, test daemon, or new runtime dependency for this work.
10. Do not add ARMv7 prebuilt publication in this corrective pass. It is intentionally source-only today.
11. Do not change the existing five supported prebuilt targets unless a concrete platform defect discovered during implementation requires it.

## Part A — make distribution consumer smoke component-symmetric

### A1. Replace the server-only input contract

Update `.github/workflows/distribution-consumer-smoke.yml` so it can validate public assets for both components independently.

The current single `version` input is described as a published `snip-sync` version and every job selects the server. Replace this with a small explicit interface. A suitable shape is:

```text
snp_version       optional X.Y.Z
snip_sync_version optional A.B.C
```

At least one input must be supplied for a run.

Do not assume both components share a version. Do not introduce one repository-wide release version.

If GitHub Actions expression ergonomics make two optional inputs awkward, an alternative small interface is acceptable:

```text
component = snp | snip-sync
version   = X.Y.Z
```

In that case, keep a single component per dispatch and make all jobs derive installer arguments/expected binary identity from that component. Prefer the simpler implementation with the least YAML duplication.

### A2. Reuse one job shape per platform

The smoke should verify the same consumer contract for whichever component is selected:

Unix `snp`:

```bash
bash packaging/install.sh --version "$VERSION"
```

Unix `snip-sync`:

```bash
bash packaging/install.sh --server --version "$VERSION"
```

Windows `snp`:

```powershell
./packaging/install.ps1 -Component Snp -Version $env:VERSION
```

Windows `snip-sync`:

```powershell
./packaging/install.ps1 -Component Server -Version $env:VERSION
```

For every prebuilt-host smoke:

1. use an isolated user/home/config destination where practical;
2. capture installer output;
3. require that output does **not** indicate Cargo fallback;
4. require the installed executable exists in the expected destination;
5. run `version` on the installed executable;
6. require exact identity, e.g. `snp 1.3.8` or `snip-sync 0.1.5`;
7. fail if the public release asset/checksum contract is not satisfied.

Do not weaken these jobs into direct asset-download tests. Their purpose is to exercise the documented installers as an external consumer would.

### A3. Platform scope

Required public consumer runners:

```text
Linux x86_64
Linux ARM64
Windows x86_64
```

These are the already-proven high-value consumer hosts and include the SBC-oriented Linux ARM64 path.

Also add macOS installer consumption if it can be done with the existing GitHub-hosted Intel and Apple Silicon runners without material complexity:

```text
macOS x86_64
macOS ARM64
```

Because the release workflow promises both macOS binaries and the Bash installer maps both architectures, complete five-platform consumer symmetry is preferred. However, do not introduce emulation or third-party runner infrastructure merely to satisfy this plan.

### A4. Preserve server-specific post-install behavior

`snip-sync` installation may initialize layout/startup registration. Keep its existing isolated environment setup so consumer CI cannot alter a runner user's real configuration.

`snp` smoke does not need to initialize application data beyond what invoking `version` requires.

## Part B — complete the Bash installer contract tests

Extend `scripts/tests/installers.sh` or split only the integration-like HTTP cases into one adjacent focused script if that makes the test easier to understand.

Do not rewrite the installer in another language for testability. The existing `SNP_INSTALL_TEST_MODE`, `SNP_INSTALL_GITHUB_BASE`, and `SNP_INSTALL_CRATES_API_BASE` seams are sufficient.

Use a tiny local fixture HTTP server from tooling already present on CI (Python standard library is acceptable). Do not add a Rust/Python package dependency.

### B1. Required deterministic cases

Cover at least:

1. `snp` exact tag and asset URL construction;
2. `snip-sync` exact tag and asset URL construction;
3. prebuilt asset HTTP 404 -> exact-version Cargo fallback path is selected;
4. non-404 transport/server failure -> hard failure, no Cargo fallback;
5. missing checksum after a successful binary download -> hard failure, no Cargo fallback;
6. malformed checksum sidecar -> hard failure;
7. SHA-256 mismatch -> hard failure;
8. candidate executable returns the wrong program identity -> hard failure;
9. candidate executable returns the wrong version -> hard failure;
10. valid candidate + checksum -> install succeeds into the isolated destination;
11. `--both` resolves components independently and does not reuse one version/tag for both;
12. `--both --version X.Y.Z` remains rejected as ambiguous;
13. source-only target classification continues to select Cargo fallback;
14. PATH warning behavior remains correct for a user-local destination.

The tests do not need to run a real Cargo compilation. Stub/intercept the fallback decision at the smallest practical seam so the test proves the branch taken without making ordinary CI compile a second copy of the project.

If shell-function substitution is used, keep it local to the sourced test process and do not add production-only test branches beyond the existing explicit test-mode base URLs.

### B2. Assert fallback boundaries, not only success output

For every integrity/identity failure, explicitly prove that the Cargo fallback function was **not** invoked. This is a release correctness property: a broken published asset must not be hidden by source compilation.

For an asset 404/source-only case, explicitly prove the inverse: fallback is attempted.

## Part C — give the PowerShell installer equivalent coverage

Do not add Pester solely for this plan unless it is already available and using it materially simplifies the repository. Plain PowerShell assertions are sufficient.

Add a focused script under `scripts/tests/` for the isolatable PowerShell behaviors, and run it on the existing Windows CI/platform-smoke job.

### C1. Required PowerShell checks

At minimum verify:

1. x86_64 Windows maps to `x86_64-pc-windows-msvc`;
2. `Snp` maps to crate `snip-it`, binary `snp`, tag `vX.Y.Z`;
3. `Server` maps to crate/binary `snip-sync`, tag `snip-sync-vA.B.C`;
4. asset names include `.exe` exactly once where expected;
5. SHA-256 sidecar parsing rejects malformed data;
6. candidate identity/version mismatch is rejected;
7. `Both` plus a single `-Version` remains rejected;
8. user/admin destination logic remains deterministic;
9. a definite asset 404 permits fallback while other download failures do not;
10. ARM64 Windows remains source-only unless/until the release matrix intentionally adds that target.

If the current script structure executes installation immediately when dot-sourced, add the narrowest conventional entry-point guard/function boundary needed to make its pure helpers testable. Do not use this as an excuse for a large installer refactor.

### C2. Public Windows proof

The expanded `distribution-consumer-smoke.yml` remains the authoritative end-to-end Windows proof. The focused PowerShell script is for deterministic edge cases, not a replacement for consuming the actual GitHub Release.

## Part D — publish the next legitimate snp release with binaries

Do this only after Parts A-C are merged and ordinary CI is green.

Do **not** alter or recreate `v1.3.7`.

Follow the existing `RELEASING.md` process for the next real `snip-it` release version. If no feature change otherwise dictates a version, use the normal repository SemVer judgment; this plan does not mandate a fake version solely for CI.

Required sequence:

1. update `snip-it` version normally;
2. run the repository's release verification;
3. run the `snip-it` publish dry-run;
4. publish the `snip-it` crate manually to crates.io;
5. wait until that exact version is visible from crates.io;
6. create/push the matching `vX.Y.Z` tag pointing to the exact manifest-version commit;
7. allow `.github/workflows/release-binaries.yml` to execute its existing attach path;
8. inspect the generated draft release;
9. require all five `snp` executables and all five `.sha256` sidecars;
10. manually publish the draft after inspection;
11. run the component-symmetric consumer smoke against that public `snp` version.

Expected public assets:

```text
snp-x86_64-unknown-linux-gnu
snp-x86_64-unknown-linux-gnu.sha256
snp-aarch64-unknown-linux-gnu
snp-aarch64-unknown-linux-gnu.sha256
snp-x86_64-apple-darwin
snp-x86_64-apple-darwin.sha256
snp-aarch64-apple-darwin
snp-aarch64-apple-darwin.sha256
snp-x86_64-pc-windows-msvc.exe
snp-x86_64-pc-windows-msvc.exe.sha256
```

The tag remains the version namespace; do not add version numbers to public asset filenames.

## Part E — verify the documented unpinned bootstrap path

After the new public `snp` release is published and crates.io reports the same version, exercise the exact documented default path on at least Linux x86_64 and Linux ARM64:

```bash
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash
```

The unpinned installer should:

1. resolve current `snip-it` stable version from crates.io;
2. derive `vX.Y.Z`;
3. select the host-specific `snp-*` asset;
4. download its `.sha256` sidecar;
5. verify checksum and `snp X.Y.Z` identity;
6. install without invoking Cargo.

The public consumer-smoke workflow may provide this proof directly if it can test the unpinned path deterministically after publication. If its normal mode is pinned for reproducibility, add a separate small job/step for the exact README command rather than replacing the pinned contract checks.

Windows should continue to test the documented PowerShell installer with the public `snp` asset and no Cargo fallback.

## Part F — documentation and plan bookkeeping

Update only documentation that would otherwise be inaccurate.

Expected files may include:

```text
.github/workflows/distribution-consumer-smoke.yml
scripts/tests/installers.sh
scripts/tests/installers.ps1                  # or equivalent narrow name
.github/workflows/ci.yml                       # only to invoke the PS test if needed
packaging/install.ps1                          # only for a narrow testability guard if needed
packaging/README.md                            # only if behavior/commands change
RELEASING.md                                   # only if consumer proof procedure needs clarification
plans/013-snp-binary-publication-and-installer-corrective-closure.md
plans/README.md
```

`release-binaries.yml` should require no functional redesign. Change it only if implementation/testing exposes a concrete bug.

Do not rewrite Plans 001, 002, or 007. They are historical records of their completed scope. Plan 013 records the newly identified stricter closure requirement that **both** user-facing binaries have proven public binary-first install paths.

When implementation is complete:

- set this plan to `Status: complete`;
- add a concise completion record with release tag and workflow run evidence;
- update `plans/README.md` in the same commit;
- leave prior completed-plan history intact.

## Verification commands

Before release/publication work:

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify
```

On Windows CI, additionally run the new focused installer contract script directly, for example:

```powershell
pwsh -NoProfile -File scripts/tests/installers.ps1
```

Use the actual path chosen by implementation.

For a safe pre-publication build check of the next tag, the existing workflow's non-mutating mode remains appropriate:

```bash
gh workflow run "Release binaries" --ref main \
  -f tag=vX.Y.Z -f mode=verify
```

Use `attach`/normal tag-push behavior only for the legitimate release flow already documented in `RELEASING.md`.

## Acceptance criteria

This plan is complete only when all of the following are true:

1. The repository still has one shared release-binary workflow for both `snp` and `snip-sync`.
2. `vX.Y.Z` still selects `snip-it`/`snp`; `snip-sync-vA.B.C` still selects `snip-sync`.
3. A legitimate post-`v1.3.7` public `snip-it` release contains all five `snp` executables and five matching `.sha256` sidecars.
4. No already-published release was mutated to obtain that proof.
5. Linux x86_64 public bootstrap installs `snp` from its release binary without Cargo fallback.
6. Linux ARM64 public bootstrap installs `snp` from its release binary without Cargo fallback.
7. Windows x86_64 PowerShell bootstrap installs `snp.exe` from its release binary without Cargo fallback.
8. macOS Intel and Apple Silicon public bootstrap are covered by consumer CI when available on the existing runner set, or the completion record explicitly documents why one cannot be exercised without adding infrastructure.
9. `distribution-consumer-smoke.yml` can exercise both independently versioned components rather than being hard-coded to `snip-sync`.
10. Public consumer smoke verifies exact installed binary identity/version and fails if Cargo fallback occurs on a supported prebuilt host.
11. Bash installer tests deterministically cover 404 fallback, non-404 hard failure, checksum failure, malformed checksum, wrong identity/version, valid installation, source-only behavior, and independent component version/tag handling.
12. Integrity/identity failures are explicitly proven not to invoke Cargo fallback.
13. PowerShell installer tests cover target/component/tag mapping, checksum/identity validation, fallback boundaries, destination behavior, and `Both` version ambiguity.
14. ARMv7 Linux and Windows ARM64 remain source-only unless separately planned.
15. The exact README default Unix `snp` bootstrap resolves the current crates.io version to a real public GitHub binary and installs without compiling Rust on a supported prebuilt host.
16. Ordinary Linux/macOS/Windows CI remains green.
17. `bash scripts/release-check.sh verify` remains green before the legitimate release.
18. No new installer/runtime dependency, release service, signing framework, or duplicated matrix was introduced.
19. The completion record cites the public `snp` release and the relevant successful release/consumer workflow runs.

## Handoff notes for implementation agents

The most important architectural instruction is: **do not implement `snp` binary generation again.** It already exists and has passed five-target verification. The missing evidence is public publication/consumption plus symmetric installer tests.

Prefer parameterizing the existing consumer-smoke workflow over copying jobs. Prefer local fixture HTTP responses over internet-dependent edge-case tests. Keep platform-specific installer assertions narrow and deterministic. The only tests that should require a real public GitHub Release are the explicit distribution consumer smokes after the next legitimate `snp` release.

If implementation uncovers an actual defect in `release-binaries.yml`, fix that defect narrowly and document it in this plan's completion record. Otherwise leave that workflow's component selection, matrix, checksum contract, draft attachment behavior, and published-release immutability unchanged.

## Completion record

Completed: 2026-09-16.

Parts A-C landed first and ordinary CI stayed green before any publication
work. Part D then shipped the next legitimate `snip-it` patch (`1.3.8`,
covering accumulated `Unreleased` fixes) through the normal manual flow:
local `release-check.sh verify`, `dry-run snip-it`, manual
`cargo publish -p snip-it`, crates.io visibility wait, exact `v1.3.8` tag on
the manifest-version commit. Part E ran the component-symmetric consumer
smoke plus the exact README unpinned bootstrap against the public release.

Evidence:

- `snip-it 1.3.8` published to crates.io; `v1.3.8` tag pushed to the exact
  manifest commit. `v1.3.7` was never mutated or retagged.
- [Release binaries run 35134474103](https://github.com/eggstack/snip-it/actions/runs/35134474103)
  (tag-push `attach` path) built all five targets, verified identity/help,
  checksums, and the complete asset set, then attached the draft. The
  [published v1.3.8 release](https://github.com/eggstack/snip-it/releases/tag/v1.3.8)
  contains all ten public files: `snp-x86_64-unknown-linux-gnu`,
  `snp-aarch64-unknown-linux-gnu`, `snp-x86_64-apple-darwin`,
  `snp-aarch64-apple-darwin`, `snp-x86_64-pc-windows-msvc.exe`, each with a
  verified `.sha256` sidecar. No version numbers in asset filenames.
- [Consumer smoke run 35135460655](https://github.com/eggstack/snip-it/actions/runs/35135460655)
  (`snp_version=1.3.8`, `snip_sync_version=0.1.5`, `test_unpinned_snp=true`)
  passed all eight jobs: Linux x86_64 + ARM64, macOS Intel + Apple Silicon,
  and Windows x86_64 pinned proofs for **both** independently versioned
  components (exact `snp 1.3.8` / `snip-sync 0.1.5` identity, no Cargo
  fallback), plus the exact README `curl ... | bash` unpinned `snp`
  bootstrap on Linux x86_64 and ARM64 resolving crates.io `1.3.8` to a
  binary-first install.
- `bash scripts/check.sh` and `bash scripts/release-check.sh verify` were
  green before publication; ordinary Linux/macOS/Windows CI is green after.
- `scripts/tests/installers.sh` now deterministically covers the Plan 002
  failure-mode contract (404 fallback; non-404/missing-checksum/malformed/
  mismatch/wrong-identity/wrong-version hard failures with explicit
  no-fallback proof via a stubbed Cargo seam; valid install; source-only;
  independent `--both` tags; PATH warnings) on a local Python-stdlib fixture
  server. `scripts/tests/installers.ps1` (run on Windows CI) covers the ten
  required PowerShell checks. ARMv7 Linux and Windows ARM64 remain
  source-only.

Concrete defects found and fixed during implementation (no redesign):

- `packaging/install.sh` now fails closed with an explicit `return 1` when
  `verify_candidate` rejects a download instead of relying on `set -e`
  propagation.
- Pipe-to-shell bootstrap (`curl ... | bash`) no longer dies with
  `BASH_SOURCE[0]: unbound variable` under `set -u`; the entry-point guard
  defaults an empty `BASH_SOURCE` to `$0`.
- macOS consumer runs exposed `${var,,}` lowercase expansion, which the
  system Bash 3.2 on `macos-15-intel`/`macos-15` does not support; checksum
  comparison now uses portable `tr`-based normalization.
- Windows consumer steps capture all PowerShell streams (`*>&1`) so the
  `Write-Host` installer output actually reaches the `Tee-Object` log the
  Cargo-fallback assertion inspects.

`release-binaries.yml` required no functional redesign: one shared workflow,
unchanged five-target matrix, unchanged component/tag namespaces and
published-release immutability, no new dependencies. Plans 001, 002, and 007
were left untouched as historical records.

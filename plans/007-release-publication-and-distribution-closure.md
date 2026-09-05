# Plan 007: release publication and distribution closure

Status: ready

Depends on: Plan 006

## Objective

Close the remaining gap between “the release workflow can build every target” and “the binary-first install/update path has been proven against the repository's real GitHub Release lifecycle.”

The current release workflow is mostly correct: its validation dispatch successfully built and verified all five `snp` targets, including Linux ARM64 and Windows x86_64. The run then failed in the final publication job because the selected historical tag already had a published GitHub Release and the workflow intentionally refuses to mutate published releases.

That immutability behavior should remain. The corrective work is to make safe validation explicit and obtain end-to-end proof on the next eligible draft/new release rather than treating an expected immutable-release refusal as a completed publication test.

## Audit finding

The validation run for Plan 001 proved:

- tag/manifest preflight works;
- Linux x86_64 builds and satisfies the configured glibc floor check;
- Linux ARM64 builds on the ARM runner and satisfies the same artifact contract;
- macOS Intel builds;
- macOS Apple Silicon builds;
- Windows x86_64 builds;
- staged executables pass identity/help checks;
- generated checksum sidecars validate;
- the aggregate publication job receives and verifies the complete asset set.

It did **not** prove:

- creation of a new draft release;
- upload/clobber behavior on a draft release;
- publication of those assets for installer/updater consumption;
- a real binary-first bootstrap against a newly published asset;
- the equivalent `snip-sync-vA.B.C` component path.

The final job's refusal to alter an already-published release is intentional repository policy and is not itself a bug.

## Governing constraints

1. Preserve published-release immutability.
2. Keep crates.io publication manual and local.
3. Do not add release signing, provenance frameworks, package managers, update daemons, or a release orchestration service.
4. Do not create fake production version tags solely to satisfy CI.
5. Do not make GitHub `releases/latest` authoritative; crates.io remains the component-version authority for installers/updaters.
6. Preserve independent tags: `vX.Y.Z` for `snp`, `snip-sync-vA.B.C` for `snip-sync`.
7. A build-only validation mode must never create or modify a GitHub Release.

## Part A — make workflow validation unambiguous

### 1. Add an explicit workflow-dispatch publication mode

Extend `workflow_dispatch` with a small input such as:

```text
mode = verify | attach
```

or an equivalent boolean with clear naming.

Required behavior:

- tag pushes retain the normal release path;
- manual `verify` dispatch performs preflight, all five builds, staged smoke checks, checksum generation, artifact aggregation, and complete-set verification, then exits successfully **without** creating/updating a GitHub Release;
- manual `attach` dispatch keeps current draft-release behavior;
- an existing published release remains a hard failure in `attach` mode;
- defaults must be safe/non-mutating for manual validation.

Do not duplicate the build matrix into separate workflows. Gate only the publication mutation step or final publication job.

### 2. Preserve complete-set verification in verify mode

The non-mutating path must still download the five build artifacts and prove all ten public files are present:

```text
5 executables
5 .sha256 sidecars
```

It must rerun checksum verification on the aggregated files. A build-only validation that skips the aggregation/public-name contract is insufficient.

### 3. Document the distinction

Update `RELEASING.md` so maintainers can distinguish:

```text
safe historical-tag validation -> workflow_dispatch mode=verify
real draft asset attachment     -> tag push or explicit mode=attach
```

Explain that `attach` intentionally refuses a published release.

The previous validation attempt against a published historical tag should be documented as expected policy behavior, not an unexplained failed release.

## Part B — validate both independently-versioned components

After Plan 006 is green, run safe workflow validation for both component paths where valid historical tags exist:

```text
vX.Y.Z
snip-sync-vA.B.C
```

If the repository has no historical `snip-sync-vA.B.C` tag matching the new naming contract, do not manufacture one retroactively. Validate that component on its next legitimate release instead.

For `snip-sync`, require its isolated `/health` smoke to execute on each supported release runner where the workflow defines it.

## Part C — prove real draft publication on the next legitimate release

The first normal component release after this corrective plan must be used as the publication proof.

Required sequence:

1. perform the repository's normal local release checks;
2. publish the crate manually according to `RELEASING.md`;
3. create/push the exact component tag;
4. allow `release-binaries.yml` to build the component;
5. verify that a draft release is created when absent;
6. verify all five target executables and five checksum sidecars are attached;
7. if a job rerun is needed while the release remains draft, verify asset replacement is idempotent;
8. manually publish the draft only after inspection.

Do not automate the final publish action in this plan.

## Part D — prove consumer paths against real assets

Once a legitimate release with assets is public, perform a minimal consumer closure pass.

### Unix bootstrap

On one supported Unix x86_64 host, run the documented bootstrap path with a clean temporary/user install destination where practical. Confirm output shows a prebuilt binary download, not Cargo fallback.

### ARM64 Linux bootstrap

On a Raspberry Pi/Le Potato-class 64-bit Linux host or equivalent ARM64 Linux environment, run the documented installer and confirm:

- target maps to `aarch64-unknown-linux-gnu`;
- the release binary is downloaded;
- checksum and `version` validation pass;
- no Rust compilation is invoked.

This is the highest-value fleet proof in this line of work.

### Windows bootstrap

On Windows x86_64, run the PowerShell installer against the public release and confirm it uses `x86_64-pc-windows-msvc.exe` rather than Cargo fallback.

### Self-update

For each component where a practical previous version is available:

1. install an older standalone/bootstrap binary in an isolated location;
2. invoke its update command against the current crates.io-authoritative version;
3. confirm exact GitHub tag/asset selection;
4. confirm checksum and candidate version validation;
5. confirm the resulting executable reports the new version.

For `snip-sync`, additionally confirm a running managed/direct server returns healthy after update, and an intentionally stopped server remains stopped.

If a previous binary version cannot exercise the new updater because that version predates the updater implementation, document that constraint and validate the current updater with the repository's deterministic HTTP/test seam instead of inventing a migration helper.

## Part E — close the roadmap accurately

Only after Plans 006 and 007 meet their acceptance criteria:

- mark Plan 007 complete;
- change Plan 000 to `Status: complete`;
- add a concise Plan 000 completion record summarizing the actual release/consumer evidence;
- update `plans/README.md` so no corrective plan remains Ready/Planned for this line of work.

Plans 001–005 do not need to be rewritten or reimplemented unless this validation exposes a concrete defect. Their feature implementation can remain recorded as complete while the umbrella roadmap remains blocked on closure evidence.

## Tests and verification

Before the real release step:

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify
```

GitHub Actions must show:

```text
CI: green on Linux, macOS, Windows
Release binaries verify: green for the component/tag being validated
```

For the real release, record the successful workflow run and the resulting release/tag in the Plan 007 completion record.

## Acceptance criteria

Plan 007 is complete only when all of the following are true:

1. Manual release-workflow validation has an explicit non-mutating mode that can finish green on an already-published historical tag.
2. Verify mode still checks the full five-target asset/checksum set.
3. Published-release immutability remains enforced in attach mode.
4. `RELEASING.md` clearly documents verify vs attach behavior.
5. The `snp` release path has a green five-target validation run.
6. The `snip-sync` path is validated on a legitimate matching tag, either historically or at its next release.
7. At least one legitimate new component tag successfully creates/uses a draft release and attaches the full expected asset set.
8. A public release asset is consumed successfully by the Unix bootstrap path without Cargo fallback.
9. A public Linux ARM64 asset is consumed successfully on a Pi/Le Potato-class or equivalent ARM64 Linux host without compiling Rust.
10. A public Windows x86_64 asset is consumed successfully by the PowerShell installer without Cargo fallback.
11. The updater path is exercised against a real public asset where version history permits, with deterministic test-seam evidence used only where an older updater cannot participate.
12. `snip-sync` update lifecycle behavior remains correct for running vs intentionally stopped servers.
13. CI is green on all ordinary platforms after the closure work.
14. Plan 000 and `plans/README.md` are marked complete only after this evidence exists.

## Handoff note

Do not solve the historical validation failure by allowing mutation of published releases. The workflow did the correct thing at that boundary. The corrective goal is to distinguish safe build verification from publication and then obtain publication proof naturally on the next legitimate release.

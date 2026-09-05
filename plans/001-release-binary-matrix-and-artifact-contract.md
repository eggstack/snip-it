# Plan 001: release binary matrix and artifact contract

Status: planned

Depends on: Plan 000

## Objective

Create a release-only GitHub Actions pipeline that builds verified `snp` and `snip-sync` executables for common Linux, macOS, Windows, and ARM64 SBC hosts and attaches them to the correct independently-versioned GitHub Release.

Do not modify the ordinary CI workflow into a release pipeline. Keep source correctness CI and release artifact production separate.

## Files expected to change

```text
.github/workflows/release-binaries.yml   new
RELEASING.md                             update
possibly scripts/release-check.sh        only if a small local preflight hook is useful
possibly scripts/ci/...                  focused release smoke helper(s)
```

No crate publication code belongs in this workflow.

## Exact tag and asset contract

Component identification is derived from the tag:

```text
vX.Y.Z                 -> component snp, crate snip-it, expected root Cargo.toml version X.Y.Z
snip-sync-vA.B.C       -> component snip-sync, expected snip-sync/Cargo.toml version A.B.C
```

Reject any tag that does not exactly match the component manifest version.

Required asset names:

```text
snp-x86_64-unknown-linux-gnu
snp-aarch64-unknown-linux-gnu
snp-x86_64-apple-darwin
snp-aarch64-apple-darwin
snp-x86_64-pc-windows-msvc.exe

snip-sync-x86_64-unknown-linux-gnu
snip-sync-aarch64-unknown-linux-gnu
snip-sync-x86_64-apple-darwin
snip-sync-aarch64-apple-darwin
snip-sync-x86_64-pc-windows-msvc.exe
```

Every executable gets a matching `<asset>.sha256` file.

Do not put the version in the asset filename. The exact Git tag is the version namespace and the installer/updater will construct that tag directly from crates.io metadata.

## Workflow triggers

Support both:

```yaml
push:
  tags:
    - 'v*'
    - 'snip-sync-v*'
workflow_dispatch:
  # explicit existing tag input
```

Manual dispatch must check out the supplied tag, not the current main branch.

The workflow must never create/push a tag, publish to crates.io, or manufacture a release version from branch state.

## Preflight job

Before building anything:

1. derive component and version from the tag;
2. verify the tag points at the checked-out commit;
3. verify the appropriate `Cargo.toml` package version is exactly the tag version;
4. verify the tree is clean after checkout;
5. optionally verify the matching crate version is visible on crates.io if the maintainer release sequence requires crates publication before binary artifacts;
6. emit `component`, `version`, and `tag` outputs for the build jobs.

Do not require `snip-it` and `snip-sync` versions to match each other.

## Build matrix

Required initial targets:

| Host | Runner | Rust target |
| --- | --- | --- |
| Linux x86_64 | `ubuntu-latest` or pinned Ubuntu | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` |
| macOS Intel | current Intel macOS runner | `x86_64-apple-darwin` |
| macOS Apple Silicon | current ARM64 macOS runner | `aarch64-apple-darwin` |
| Windows x86_64 | current x64 Windows runner | `x86_64-pc-windows-msvc` |

Build only the requested component for a component-specific tag. Do not rebuild `snip-sync` on every `snp` tag or vice versa.

## Linux compatibility floor

The SBC motivation makes host compatibility more important than simply compiling on the newest Ubuntu runner.

Preferred implementation:

```text
cargo-zigbuild + pinned Zig
x86_64-unknown-linux-gnu.2.17
aarch64-unknown-linux-gnu.2.17
```

The public filenames remain the normal Rust target names without `.2.17`.

Before committing to this, prove both packages build correctly with the current dependency graph. If one package cannot use `cargo-zigbuild` because of a concrete native-system dependency, document the actual blocker and choose the oldest practical supported glibc floor rather than silently shipping Ubuntu-24-only binaries.

Acceptance for Linux must include either:

- successful `.2.17` zigbuild, or
- an explicit documented minimum glibc version validated by the workflow.

The goal is that current 64-bit Raspberry Pi OS and typical Ubuntu/Debian SBC installations can run the release executable without recompiling.

## Artifact staging and verification

For every build:

1. compile with `--release --locked`;
2. copy only the target executable into `dist/` under the stable public asset name;
3. run the staged executable, not the target-directory source, for smoke checks;
4. require the exact component identity and version;
5. run `--help`;
6. for `snip-sync`, run an isolated loopback server smoke and require `/health` HTTP 200;
7. only after all smoke checks pass, compute SHA-256;
8. verify the generated checksum locally before upload.

### `snp` smoke

At minimum:

```text
<asset> version  -> `snp X.Y.Z`
<asset> --help   -> exit 0
```

Use an isolated temporary config directory for any additional noninteractive operation. Never access the runner's normal keychain/config.

### `snip-sync` smoke

Use temporary config/data/state directories and a random/free loopback port. Start with explicit local plaintext acknowledgement suitable only for the loopback test, poll `/health` with a bounded deadline, then terminate the process.

Do not leave the smoke server running after the job.

## GitHub Release attachment

Prefer GitHub CLI/API primitives already available in Actions rather than adding a release action solely as a wrapper.

The workflow may:

- create a draft release for an existing tag if none exists, or
- upload/clobber assets on an existing draft release.

Do not automatically publish a draft unless the current repository release policy explicitly chooses that behavior. `RELEASING.md` must describe the exact maintainer sequence.

On rerun, the workflow should be idempotent for draft assets. It must fail rather than mutating an already-published release in a surprising way if the existing repo policy treats published assets as immutable.

## ARMv7 and Windows ARM64

Do not block this plan on them.

The installer will recognize:

```text
armv7-unknown-linux-gnueabihf
aarch64-pc-windows-msvc
```

as source-only until a later qualification pass intentionally adds binary jobs.

## Tests / verification

Required before closure:

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify   # from a clean tree when applicable
```

Additionally exercise the release workflow through `workflow_dispatch` against an existing test/release tag or a deliberately created release-candidate tag according to repository practice.

Do not call this plan complete based only on YAML syntax review.

## Acceptance criteria

1. The workflow recognizes `vX.Y.Z` and `snip-sync-vA.B.C` independently.
2. A mismatched tag/manifest version fails before compilation.
3. The five required targets build the selected component.
4. Linux ARM64 runs natively on the GitHub ARM64 runner.
5. Linux compatibility has a documented/verified glibc floor suitable for SBC deployment.
6. Every uploaded executable has a verified `.sha256` sidecar.
7. Staged binaries pass identity/version/help smoke checks.
8. `snip-sync` additionally passes a real isolated `/health` smoke.
9. No workflow contains crates.io publication credentials or `cargo publish`.
10. `RELEASING.md` accurately documents the manual crate publication plus GitHub binary-release sequence.
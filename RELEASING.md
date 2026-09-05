# Releasing snip-it

Publishing to crates.io is a **manual, local** operation. There is no
automated publish workflow and no crates.io token in GitHub Actions.

## Prerequisites

- Rust 1.94+ with `cargo` on `PATH`
- `jq` for local release-tag metadata validation
- `crates.io` login: `cargo login <token>` (token stored in `~/.cargo/credentials.toml`)
- All changes committed and pushed to `main`
- `scripts/release-check.sh verify` passes locally from a clean checkout
- The GitHub repository has Actions enabled with permission to create and upload
  draft release assets

## Crates and dependency order

The workspace contains three crates published in dependency order:

| Order | Crate | Depends on |
|-------|-------|------------|
| 1 | `snip-proto` | (none) |
| 2 | `snip-sync` | `snip-proto` |
| 3 | `snip-it` | `snip-proto`, `snip-sync` |

**Only publish crates whose version changed.**

## Version bump rules

1. Update `version` in the crate's `Cargo.toml`.
2. If `snip-proto` changed, also update the `snip-proto` dependency
   version in `snip-sync` and `snip-it`.
3. Commit the version bump.

## Pre-release checklist

1. Run the full release verification:
   ```bash
   bash scripts/release-check.sh verify
   ```
2. Verify `cargo package --list` shows the expected files.
3. Verify no secrets, credentials, or test-only environment variables
   appear in the package contents.

## Publishing crates and binary releases

Crate publication remains manual. Publish the component crate first, wait for
the exact version to become visible on crates.io, then create and push the
matching tag. The tag is the binary version namespace; it must point at the
same commit whose manifest contains that version.

The `Release binaries` workflow never runs `cargo publish`, creates tags, or
selects a version from branch state. A tag push starts it automatically. A
maintainer can also use **Actions → Release binaries → Run workflow** and enter
an existing exact tag; the workflow checks out that input tag.

The workflow builds only the selected component for these targets:

```text
x86_64-unknown-linux-gnu       (glibc 2.17 floor)
aarch64-unknown-linux-gnu      (glibc 2.17 floor)
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-pc-windows-msvc
```

Each staged executable must pass its identity/version and `--help` smoke test;
`snip-sync` must also return HTTP 200 from an isolated loopback `/health` test.
Each asset is uploaded with an adjacent, locally verified `.sha256` sidecar.
Public filenames do not include the version.

For an `snip-it` release:

```bash
cargo publish -p snip-it
# Wait until crates.io shows the exact version, then use the matching tag.
git tag -a vX.Y.Z -m "snip-it X.Y.Z"
git push origin vX.Y.Z
```

For a `snip-sync` release:

```bash
cargo publish -p snip-sync
# Wait until crates.io shows the exact version, then use the matching tag.
git tag -a snip-sync-vA.B.C -m "snip-sync A.B.C"
git push origin snip-sync-vA.B.C
```

If a release with that tag does not exist, Actions creates a **draft** release
and uploads the verified assets. It never publishes the draft automatically.
Reruns clobber assets on an existing draft so a failed upload can be retried.
If the tag already has a published release, the workflow fails rather than
mutating its assets.

The workflow's Linux jobs use pinned Zig and `cargo-zigbuild` versions and pass
the explicit `.2.17` target suffix. The staged Linux binary is checked with
`readelf` to ensure it does not require a newer glibc symbol version. ARM64
Linux is built and smoke-tested natively on `ubuntu-24.04-arm`.

### Step-by-step flow

1. Run `bash scripts/release-check.sh verify` once.
2. For each changed crate in dependency order:
   a. Bump and commit the version.
   b. Run `bash scripts/release-check.sh dry-run <crate>`.
   c. Run `cargo publish -p <crate>` manually.
   d. Wait until crates.io indexes that version before publishing dependents.

### If only snip-it changed

```bash
cargo publish -p snip-it
```

### If snip-sync changed (with or without snip-it)

```bash
cargo publish -p snip-sync
cargo publish -p snip-it
```

### If snip-proto changed

```bash
cargo publish -p snip-proto
# Wait for crates.io to index (usually < 1 minute)
cargo publish -p snip-sync
cargo publish -p snip-it
```

## Dry-run validation

Before publishing, validate each crate with the script:

```bash
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

The dry-run mode requires a clean working tree and runs
`cargo publish --dry-run --locked`.

## Release tag validation

The local helper can validate a checked-out release tag and print its derived
component metadata as JSON:

```bash
bash scripts/release-check.sh tag vX.Y.Z
bash scripts/release-check.sh tag snip-sync-vA.B.C
```

It rejects unsupported tag shapes, tags that do not point at `HEAD`, and tags
whose component manifest version differs from the tag version. The workflow
performs the same checks before any build starts.

## Version immutability

- crates.io versions are **immutable**.
- A failed or incomplete release cannot be overwritten.
- Any correction requires a **new version bump**.
- Verify package contents before publishing.
- Do not attempt to "retry" a published version after changing files.

## Security

- The crates.io token remains in the maintainer's local Cargo credentials.
- **No** crates.io token is stored in GitHub Actions.
- **No** workflow has `id-token: write` or package publishing permissions.
- Release documentation does not print or inspect credentials.

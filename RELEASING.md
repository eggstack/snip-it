# Releasing snip-it

Publishing to crates.io is a **manual, local** operation. There is no
automated publish workflow and no crates.io token in GitHub Actions.

## Prerequisites

- Rust 1.94+ with `cargo` on `PATH`
- `crates.io` login: `cargo login <token>` (token stored in `~/.cargo/credentials.toml`)
- All changes committed and pushed to `main`
- `scripts/release-check.sh verify` passes locally from a clean checkout

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

## Publishing

Publish in dependency order. Wait for each crate to resolve from
crates.io before publishing the next.

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

## Git tags (optional)

Tags are optional and manual:

```bash
git tag -a v1.3.5 -m "snip-it 1.3.5"
git push origin v1.3.5
```

Do not create a GitHub Release automatically. Do not publish from tags.

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

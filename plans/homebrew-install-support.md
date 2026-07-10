# Homebrew Install Support Plan

## Purpose

Add a reliable, maintainable Homebrew installation path for the `snip-it` client so macOS users can install and upgrade the `snp` binary with:

```bash
brew install eggstack/tap/snip-it
```

The first supported distribution target should be an Eggstack-owned Homebrew tap. Submission to `Homebrew/homebrew-core` is explicitly out of scope for this pass; the implementation should not preclude it later.

This plan is intended for agent handoff. The implementing agent should inspect the current repository state before changing files and should preserve all existing crates.io, GitHub Release, Docker, and source-install behavior.

## Current Repository State

The root package is already structurally suitable for Homebrew:

- Package name: `snip-it`.
- Installed binary name: `snp`.
- License: MIT.
- A stable semantic version is declared in `Cargo.toml`.
- Tagged releases trigger `.github/workflows/release.yml`.
- Release builds currently target both `aarch64-apple-darwin` and `x86_64-apple-darwin`.
- The CLI can generate Bash, Zsh, Fish, PowerShell, and Elvish completions through `snp completions <shell>`.
- The README already documents crates.io, prebuilt binary, source, and Docker installation paths.

The current release workflow also has an asset-shaping defect relevant to package managers: GitHub Actions artifacts have target-specific artifact names, but the contained Unix executable remains named `snp`. The release job uploads files from each artifact directory without first assigning unique release filenames. The README advertises target-specific release asset names, so the release workflow and documentation may not currently agree in practice.

## Goals

1. Support installation from an Eggstack Homebrew tap on Apple Silicon and Intel macOS.
2. Build the initial Homebrew formula from the tagged source archive using the committed lockfile.
3. Install the `snp` executable and shell completions in standard Homebrew locations.
4. Add deterministic formula validation and smoke tests.
5. Document installation, upgrade, uninstall, and tap usage.
6. Add release automation that updates the tap formula after a successful tagged release.
7. Normalize GitHub Release artifact names and checksums so direct downloads and future binary formulas are reliable.
8. Keep the implementation auditable and avoid introducing a large release framework unless clearly justified.

## Non-Goals

- Immediate submission to `Homebrew/homebrew-core`.
- Replacing crates.io installation.
- Replacing the current GitHub Release workflow with `cargo-dist`, GoReleaser, or another broad release framework solely for Homebrew support.
- Packaging the `snip-sync` server as part of the `snip-it` client formula.
- Installing or configuring the optional sync server, credentials, cron jobs, or user configuration during `brew install`.
- Running interactive first-use setup from the formula.
- Managing user snippet data during install, upgrade, or uninstall.

## Design Decisions

### 1. Use an organization-owned tap first

Create or use the public repository:

```text
eggstack/homebrew-tap
```

The formula path should be:

```text
Formula/snip-it.rb
```

The canonical install command should be:

```bash
brew install eggstack/tap/snip-it
```

This approach is under project control, permits immediate iteration, and avoids making Homebrew Core acceptance a release blocker.

### 2. Build the initial formula from source

The formula should consume the immutable GitHub tag archive:

```text
https://github.com/eggstack/snip-it/archive/refs/tags/v<VERSION>.tar.gz
```

It should depend on Rust at build time and execute a locked Cargo installation into the formula prefix:

```ruby
system "cargo", "install",
       "--locked",
       "--root", prefix,
       "--path", "."
```

Do not make the initial formula architecture-specific. A source formula should work on both supported macOS architectures and allows Homebrew to produce bottles later.

### 3. Treat `snp` as the sole client executable

The formula is for the interactive client package only. It should install `snp`; it must not accidentally install workspace server binaries or attempt to build and install all workspace members.

The implementation must verify that `cargo install --locked --path .` installs only the intended root package binary.

### 4. Generate completions during formula installation

Use the existing CLI completion generator rather than maintaining static completion files.

Install:

- Bash completion as `snp` under `bash_completion`.
- Zsh completion as `_snp` under `zsh_completion`.
- Fish completion as `snp.fish` under `fish_completion`.

Prefer Homebrew's completion helper only if it maps correctly onto the positional CLI form `snp completions <shell>`. Otherwise, generate each file explicitly with `Utils.safe_popen_read` or the current Homebrew-recommended equivalent.

Do not add shell-completion generation to the normal build script merely for Homebrew.

### 5. Keep formula testing noninteractive

The formula `test do` block must not require a TTY, clipboard access, an editor, network access, sync credentials, or user input.

The minimum acceptable test is:

- Execute `snp --version` and assert the formula version appears.
- Execute completion generation for a supported shell and assert recognizable output.

A stronger functional test should be used if the CLI gains or already has a fully noninteractive way to create and list a snippet in Homebrew's temporary test home.

Do not drive an interactive prompt with brittle shell piping unless there is no safer test surface.

## Workstream A: Source-Package Reproducibility

### A1. Verify the lockfile contract

Confirm that `Cargo.lock` is committed on `main` and is included in GitHub-generated tag archives.

Run locally or in CI:

```bash
cargo install --locked --path . --root "$(mktemp -d)"
```

Acceptance requirements:

- The command succeeds from a clean checkout.
- Only the expected `snp` executable is installed by the root package operation.
- No uncommitted lockfile mutation occurs.
- The installed executable reports the package version.

If `Cargo.lock` is absent or excluded, commit/fix it before proceeding. Do not remove `--locked` from the formula as a workaround.

### A2. Verify the exact source archive

Test using the archive Homebrew will consume, not only a Git checkout.

For a released tag or a temporary validation tag/archive:

1. Download the tag tarball.
2. Verify its SHA-256.
3. Extract it into a clean directory.
4. Run `cargo install --locked --path .`.
5. Run `snp --version` and completion generation from the installed binary.

Pay particular attention to the workspace and manifest exclusions. The root package currently lists `snip-proto` and `snip-sync` as development dependencies and workspace members; confirm that a normal production install does not require excluded content or trigger workspace-wide installation.

### A3. Verify supported macOS architectures

Validate source builds on:

- Apple Silicon macOS (`arm64` / `aarch64-apple-darwin`).
- Intel macOS (`x86_64-apple-darwin`), using a native Intel runner where available rather than treating Rosetta alone as sufficient validation.

Required checks:

```bash
snp --version
snp completions bash >/dev/null
snp completions zsh >/dev/null
snp completions fish >/dev/null
```

Record any system-library requirements discovered during these builds. Do not add formula dependencies speculatively.

## Workstream B: Formula Implementation

### B1. Create the tap repository

Create `eggstack/homebrew-tap` if it does not already exist.

Recommended initial structure:

```text
Formula/
  snip-it.rb
.github/
  workflows/
    test.yml
README.md
LICENSE
```

The tap README should state that it is the official Eggstack tap and provide tap/install/update/uninstall examples.

### B2. Implement `Formula/snip-it.rb`

The formula should include:

- `desc` matching the client purpose.
- `homepage` pointing to the project repository.
- Immutable tagged source `url`.
- Correct source archive `sha256`.
- `license "MIT"`.
- Optional `head` support for advanced users, pointing to `main`.
- `depends_on "rust" => :build`.
- Locked Cargo installation of the root package.
- Bash, Zsh, and Fish completion installation.
- A deterministic `test do` block.

Representative shape:

```ruby
class SnipIt < Formula
  desc "Terminal-first snippet manager with fuzzy search and encrypted sync"
  homepage "https://github.com/eggstack/snip-it"
  url "https://github.com/eggstack/snip-it/archive/refs/tags/vX.Y.Z.tar.gz"
  sha256 "..."
  license "MIT"
  head "https://github.com/eggstack/snip-it.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install",
           "--locked",
           "--root", prefix,
           "--path", "."

    # Generate completions using the installed binary.
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/snp --version")
    completion = shell_output("#{bin}/snp completions bash")
    assert_match "snp", completion
  end
end
```

The implementing agent must consult the current Homebrew formula API before finalizing completion-directory helpers and audit syntax. Avoid copying deprecated formula patterns.

### B3. Validate formula naming

Use formula name `snip-it` so installation links the existing executable `snp` without renaming it.

Expected commands:

```bash
brew install eggstack/tap/snip-it
snp --version
brew upgrade eggstack/tap/snip-it
brew uninstall snip-it
```

Do not create a formula named `snp` unless Homebrew naming/audit behavior forces it and the tradeoff is documented. The package/project identity should remain `snip-it`, while the executable remains `snp`.

## Workstream C: Noninteractive CLI Test Surface

### C1. Audit the current `new` command

Determine whether `snp new` can already create a snippet without prompts. The README currently states that it prompts for a description, so assume it is interactive until verified otherwise.

If a functional Homebrew test cannot be expressed safely, add a narrow noninteractive input surface such as:

```bash
snp new --description "Homebrew test" "echo homebrew-test"
```

Requirements for any new option:

- Preserve existing interactive behavior when the option is omitted.
- Reject invalid combinations cleanly.
- Do not silently create an empty description unless that is already a supported semantic.
- Work without a TTY.
- Respect Homebrew's temporary `HOME`/XDG environment.
- Receive unit and integration coverage.
- Be documented as a general scripting feature, not a Homebrew-only switch.

A general `--non-interactive` flag may be considered only if it has clear semantics across prompts. Prefer the smallest coherent API.

### C2. Add a functional package-manager smoke test

Once noninteractive creation is available, the formula test should:

1. Create a temporary config location implicitly through the Homebrew test environment or explicitly through supported XDG variables.
2. Create a snippet noninteractively.
3. List snippets using a machine-readable or stable textual format.
4. Assert the stored command or description appears.

Example intent:

```ruby
test do
  system bin/"snp", "new", "--description", "Homebrew test", "echo homebrew-test"
  output = shell_output("#{bin}/snp list --json")
  assert_match "echo homebrew-test", output
end
```

Do not rely on clipboard functionality, TUI rendering, shell execution, network sync, or user keyrings in the formula test.

If implementing the noninteractive CLI surface materially broadens scope, retain the version/completion smoke test for the initial formula and track the stronger test as a follow-up. The plan should still document that limitation clearly.

## Workstream D: Release Artifact Normalization

This work is independent of the source formula but should be completed in the same line of work because the current release naming is ambiguous and conflicts with documented asset names.

### D1. Package each binary into a uniquely named archive

Replace direct raw-binary upload with deterministic archives.

Recommended filenames:

```text
snip-it-v<VERSION>-aarch64-apple-darwin.tar.gz
snip-it-v<VERSION>-x86_64-apple-darwin.tar.gz
snip-it-v<VERSION>-aarch64-unknown-linux-gnu.tar.gz
snip-it-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz
snip-it-v<VERSION>-x86_64-pc-windows-msvc.zip
```

Recommended archive contents:

```text
snp                    # snp.exe on Windows
LICENSE
README.md
```

Requirements:

- Archive naming must be derived from the tag/version and target triple.
- Archive content paths must be consistent across platforms.
- Unix executable permissions must be preserved.
- Archive creation must fail if the expected binary is missing.
- Avoid timestamps or metadata that make archives unnecessarily nondeterministic where practical.

### D2. Generate checksums over final release assets

Generate a checksum manifest after archive creation, using the exact filenames uploaded to the GitHub Release.

Preferred filename:

```text
SHA256SUMS
```

The manifest should contain one line per final archive. It should not contain duplicate basename entries or paths into temporary artifact directories.

### D3. Verify release upload behavior

The GitHub Release job must upload:

- Each uniquely named archive.
- `SHA256SUMS`.

Add a post-build validation step that asserts the expected asset set is complete and contains no duplicate filenames before invoking the release action.

### D4. Update direct-download documentation

Update the README's prebuilt binary table and installation instructions to match the actual archive names and extraction commands.

Do not document raw binaries if releases now provide archives.

## Workstream E: Tap Validation CI

### E1. Add tap CI

In `eggstack/homebrew-tap`, add a workflow triggered by pull requests and pushes that affect formulae.

At minimum, validate:

```bash
brew style Formula/snip-it.rb
brew audit --strict Formula/snip-it.rb
brew install --build-from-source Formula/snip-it.rb
brew test snip-it
snp --version
brew uninstall snip-it
```

Use the current Homebrew-recommended commands and flags. If `brew audit --new --formula` is more appropriate for initial introduction, use it for the initial formula and retain strict audit checks afterward.

### E2. Test both macOS architectures

Run CI on Apple Silicon and Intel macOS runners when available to the organization.

If GitHub-hosted Intel availability is constrained, document the exact substitute validation used and perform at least one native Intel verification before declaring support complete.

### E3. Prevent accidental network/secret dependencies

The formula test must pass without:

- GitHub credentials.
- Sync server credentials.
- External service availability.
- Clipboard/UI access.
- Existing user configuration.

The source download performed by Homebrew is the only expected external retrieval during installation.

## Workstream F: Automated Formula Updates

### F1. Update only after successful release creation

The tap update must occur after:

1. Project CI passes.
2. crates.io publication succeeds or is intentionally skipped under an explicitly supported condition.
3. Platform artifacts and checksums are built successfully.
4. The GitHub Release is successfully created.

Do not update the formula to a tag whose source archive is not publicly available.

### F2. Compute source checksum from the immutable tag archive

The automation should:

1. Derive the version from `github.ref_name`.
2. Validate that the tag matches the package version in `Cargo.toml`.
3. Download the tag archive from GitHub.
4. Compute SHA-256.
5. Update only the formula `url` and `sha256` fields, plus any version-sensitive test expectation if necessary.
6. Commit or open a pull request in `eggstack/homebrew-tap`.

Prefer opening an automated pull request so tap CI gates publication. Direct commits to the tap are acceptable only if branch protection and CI still prevent an invalid formula from becoming the default installation path.

### F3. Authentication and permissions

Use a narrowly scoped GitHub App token, fine-grained PAT, or equivalent organization-approved credential capable of updating only the tap repository.

Do not use a broad personal token when a narrower credential is possible.

Document required repository secrets and permissions in maintainer documentation, not in the public formula.

### F4. Avoid release recursion

The tap update workflow must not retrigger the `snip-it` release workflow or create a loop between repositories.

## Workstream G: Documentation

### G1. Update the project README

Add Homebrew as the first or second macOS-friendly installation method:

```bash
brew install eggstack/tap/snip-it
```

Document:

```bash
brew upgrade snip-it
brew uninstall snip-it
```

Clarify that:

- The formula installs the `snp` client.
- The optional `snip-sync` server is not installed by this formula.
- User configuration and snippet data are preserved by normal Homebrew upgrades/uninstalls unless the user removes them manually.

### G2. Update tap README

Include:

- `brew tap eggstack/tap`.
- `brew install snip-it` after tapping.
- The one-line fully qualified install command.
- Available formulae.
- Bug-report routing: application bugs to `eggstack/snip-it`, formula/tap bugs to `eggstack/homebrew-tap`.

### G3. Update release/maintainer documentation

Document:

- Formula update automation.
- Required secret/token.
- How to reproduce a source checksum.
- How to test a formula locally.
- How to recover if a release succeeds but the tap update fails.
- How to disable or roll back a broken formula version.

## Workstream H: Tests in the Main Repository

Add or extend tests for any main-repository code changed by this work.

Required coverage:

- CLI parsing for any new noninteractive options.
- Existing interactive behavior remains unchanged when new options are omitted.
- Completion generation continues to succeed for Bash, Zsh, and Fish.
- Version output remains stable and includes the Cargo package version.
- Release packaging scripts validate expected binary paths and archive names.
- Checksum generation covers all final archives exactly once.

If release packaging is implemented with shell scripts, use strict mode and add lightweight script validation in CI. Prefer a small, explicit script under `scripts/` over embedding a long, hard-to-test shell program directly inside workflow YAML.

## Suggested Implementation Sequence

1. Confirm `Cargo.lock` and source-tarball reproducibility.
2. Create a local draft formula and validate source installation on Apple Silicon.
3. Add/verify noninteractive smoke-test capability if needed.
4. Install and validate shell completions from the formula.
5. Create `eggstack/homebrew-tap` and commit the formula plus CI.
6. Validate on Intel macOS.
7. Correct GitHub Release archive naming and checksum generation.
8. Update project and tap documentation.
9. Add automated tap-update pull requests after successful releases.
10. Exercise the complete process with a new tagged release.

Do not tag a release solely to test incomplete automation unless maintainers explicitly choose a prerelease tag and all release consumers are protected from treating it as stable.

## Acceptance Criteria

### Installation

- `brew install eggstack/tap/snip-it` succeeds on supported Apple Silicon macOS.
- The same command succeeds on supported Intel macOS.
- `command -v snp` resolves to the Homebrew-managed executable.
- `snp --version` reports the formula/release version.
- Installation builds with `cargo install --locked` from the immutable tag archive.

### Completions

- Bash completion is installed in the Homebrew completion prefix.
- Zsh completion is installed as `_snp` in the Homebrew completion prefix.
- Fish completion is installed as `snp.fish` in the Homebrew completion prefix.
- Completion generation occurs without requiring user configuration or network access.

### Testing

- `brew style` passes.
- `brew audit` passes with the intended strict/new-formula mode.
- `brew test snip-it` passes in a clean environment.
- The test does not require a TTY, clipboard, editor, sync server, keyring, or credentials.
- Main-repository CI remains green.

### Releases

- Every platform release asset has a unique versioned target filename.
- Unix releases are tar archives; Windows release is a zip archive.
- `SHA256SUMS` contains checksums for every final archive exactly once.
- README asset names match actual GitHub Release assets.
- Existing crates.io and Docker release behavior is preserved.

### Automation

- A successful stable tag causes an automated formula-update pull request or gated commit in `eggstack/homebrew-tap`.
- The formula update uses the immutable tag archive and correct SHA-256.
- Tap CI must pass before the updated formula becomes the supported installation.
- A failed tap update does not invalidate or delete an otherwise successful project release.

### Upgrade and Removal

- `brew upgrade snip-it` moves from one supported release to the next.
- `brew uninstall snip-it` removes the Homebrew-installed executable and completions.
- Existing user configuration and snippet data are not deleted by installation, upgrade, or uninstall.

## Verification Checklist for the Implementing Agent

Before marking the work complete, capture the output or CI evidence for:

```bash
cargo install --locked --path . --root "$(mktemp -d)"

brew style Formula/snip-it.rb
brew audit --strict Formula/snip-it.rb
brew install --build-from-source Formula/snip-it.rb
brew test snip-it
snp --version
snp completions bash >/dev/null
snp completions zsh >/dev/null
snp completions fish >/dev/null
brew uninstall snip-it
```

Also verify:

- The formula's source checksum independently matches the downloaded tag archive.
- Both macOS architectures have native validation evidence.
- No user-owned files under the configuration directory are changed during formula installation.
- The GitHub Release asset list exactly matches the documented target matrix.
- The release workflow does not publish multiple assets with the same basename.
- A dry run or test release demonstrates the cross-repository formula-update mechanism.

## Risks and Mitigations

### Rust toolchain version availability

The package declares a relatively recent minimum Rust version. Homebrew's `rust` formula must satisfy it at build time. Validate this rather than adding custom toolchain installation logic.

Mitigation: fail clearly in formula CI if Homebrew Rust is insufficient; do not silently fetch rustup inside the formula.

### Workspace/source archive coupling

Manifest exclusions or workspace development dependencies could make the GitHub tag archive differ from a normal checkout.

Mitigation: treat exact tag-archive installation as a release gate.

### Interactive command tests

A formula test that drives prompts may be flaky or fail under Homebrew's sandbox.

Mitigation: rely on version/completion smoke tests initially or add a generally useful noninteractive command option with dedicated tests.

### Cross-repository release credentials

Automated tap updates require credentials beyond the default single-repository `GITHUB_TOKEN` in many configurations.

Mitigation: use a narrowly scoped GitHub App or fine-grained token and keep the tap update PR-based and CI-gated.

### Broken formula publication

A formula can reference a valid release but still fail due to Homebrew-specific behavior.

Mitigation: require tap CI before merge and retain a documented rollback process.

### Release asset changes affecting users

Changing raw release binaries to archives may affect existing manual-install instructions or automation.

Mitigation: update documentation in the same release, use clear versioned names, and consider retaining compatibility assets for one release only if current external consumers are known.

## Definition of Done

This line of work is complete when a user on either supported macOS architecture can install the stable `snp` client through the official Eggstack tap, receive shell completions, run a deterministic Homebrew test, and upgrade to a subsequent release; maintainers can publish releases without manually editing checksums; and GitHub Release artifacts have unique, documented, checksum-covered names. All prior installation and release paths must continue to function.
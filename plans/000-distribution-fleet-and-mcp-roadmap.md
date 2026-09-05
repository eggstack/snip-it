# Plan 000: distribution, fleet deployment, and MCP roadmap

Status: blocked on corrective closure (Plans 006–007)

## Objective

Make `snip-it` practical to deploy across a small heterogeneous fleet without requiring source compilation on the common targets, while keeping the project lightweight.

The target user experience is:

```text
curl/PowerShell bootstrap
-> detect host
-> install verified prebuilt binary when available
-> fall back to exact-version Cargo build only when needed
-> optionally configure snip-sync startup
-> support binary-first updates later through the installed CLI
```

For agent use, add a local stdio MCP endpoint launched by the agent client on demand. Do not turn the MCP adapter into another long-running service.

## Current baseline

At the start of this line of work:

- `snp` is published as crate `snip-it` and already has `snp update`.
- `snip-sync` is independently versioned and published as crate `snip-sync`.
- GitHub Releases exist but have no prebuilt executable assets.
- ordinary CI verifies Linux, macOS, and Windows source builds but does not create release binaries.
- `snip-sync` already provides `serve`, `stop`, `restart`, `update`, `croncheck`, `/health`, process identity checks, and a kernel-backed singleton server lock.
- the server README already contains hand-written systemd and cron examples.
- `snp` already exposes enough library/persistence APIs to implement a read-only MCP adapter without routing through the TUI.

Do not reimplement any of those pieces unless a plan below explicitly calls for a narrow correction.

## Governing constraints

1. Keep crates.io publishing manual. No crates.io credentials enter GitHub Actions.
2. GitHub Actions may build and attach release executables, checksums, and bootstrap scripts, but must not publish crates.
3. No apt, deb/rpm, Homebrew, Winget, Chocolatey, Snap, Flatpak, or similar package-distribution work in this phase.
4. No auto-update daemon, periodic update notification, rollback framework, differential updater, or release signing infrastructure in this phase.
5. `snip-sync` remains one foreground server binary. systemd/launchd/cron/Task Scheduler are wrappers around the existing command surface, not new runtime layers.
6. The install and update paths are binary-first and exact-version. Cargo is the fallback, not the default on supported targets.
7. Integrity failure is a hard failure. Do not fall back to Cargo after checksum mismatch, wrong candidate version, malformed release data, TLS failure, or GitHub 5xx.
8. Do not silently invoke `sudo` or elevation. If privilege is needed, print the exact command the operator should run.
9. Preserve existing user configuration, snippet data, server database, credentials, and startup registration during install/update.
10. Keep MCP read-only initially. Do not add a generic agent-visible arbitrary command-execution tool.

## Release identity contract

`snip-it` and `snip-sync` have independent crate versions. Do not force them to share a workspace release version merely to simplify GitHub assets.

Use independent exact tags:

```text
snip-it crate X.Y.Z   -> GitHub tag vX.Y.Z
snip-sync crate A.B.C -> GitHub tag snip-sync-vA.B.C
```

Stable public asset names do not contain the version because the tag is already the version namespace:

```text
snp-<rust-target>[.exe]
snp-<rust-target>[.exe].sha256
snip-sync-<rust-target>[.exe]
snip-sync-<rust-target>[.exe].sha256
```

The bootstrap installer should be served from the repository (for example `packaging/install.sh` on `main`) rather than relying on GitHub's repository-wide `releases/latest` pointer, because the latest `snp` and latest `snip-sync` releases can differ.

An updater first asks crates.io for the latest version of its own crate, then constructs the exact GitHub tag above. GitHub `latest` is never the version authority for self-update.

## Required initial binary matrix

```text
Linux x86_64        x86_64-unknown-linux-gnu
Linux ARM64         aarch64-unknown-linux-gnu
macOS Intel         x86_64-apple-darwin
macOS Apple Silicon aarch64-apple-darwin
Windows x86_64      x86_64-pc-windows-msvc
```

The Linux ARM64 binary is the primary SBC artifact for 64-bit Raspberry Pi and Le Potato deployments.

Initial source-only fallback targets may include:

```text
Linux ARMv7        armv7-unknown-linux-gnueabihf
Windows ARM64      aarch64-pc-windows-msvc
```

Add those to the binary matrix only after the required matrix is stable. The installer must recognize source-only mappings and go directly to Cargo fallback rather than constructing known-nonexistent assets.

## Dependency graph

```text
001 release binary matrix + asset contract
 |\
 | +--> 003 snip-sync startup/lifecycle
 |          |
 +--> 002 bootstrap installers
 |          |
 |          +--> 005 local MCP + client registration docs/install flow
 |
 +----------+--> 004 binary-first self-update
                    ^
                    |
                    +-- 003 manager-aware restart semantics

corrective closure:
006 Windows CI/platform closure
  -> 007 release publication + real distribution proof
       -> close 000
```

## Plan boundaries

### Plan 001 — binary release matrix

Create release-only GitHub Actions that build and verify the supported targets, generate SHA-256 sidecars, and attach assets to the exact component tag. Qualify a Linux compatibility floor suitable for current Raspberry Pi OS/Ubuntu SBC installations.

### Plan 002 — bootstrap installers

Add `packaging/install.sh` and `packaging/install.ps1`. They query crates.io, determine the exact component tag, download and verify a matching GitHub binary, install to a predictable location, and use Cargo only when there is no binary for the host.

### Plan 003 — startup and lifecycle

Move systemd/launchd/cron/Windows Task Scheduler setup into `snip-sync startup ...`, fix the Windows `croncheck` maintenance-lock stale-file issue, and make restart work coherently on supported platforms without introducing a general daemon framework.

### Plan 004 — self-update

Replace Cargo-only/unmanaged update behavior with the same crates.io-authoritative, GitHub-binary-first contract used by the installer. `snip-sync update` preserves startup state and restarts a running server through the detected supervisor/direct lifecycle path.

### Plan 005 — MCP

Add `snp mcp serve` as an on-demand stdio MCP server plus small client-registration helpers for common coding agents. Start with deterministic read-only snippet retrieval/search tools.

### Plan 006 — Windows CI/platform closure

Restore the existing Windows all-target/platform-smoke gate on current `main`. Diagnose the actual compiler failure and apply only the minimum portability, feature-gating, dependency, or CI-prerequisite correction justified by that diagnostic.

### Plan 007 — release publication/distribution closure

Separate safe non-mutating workflow validation from real draft attachment, preserve published-release immutability, then obtain real release/install/update evidence on the next legitimate release, including the ARM64 Linux fleet path.

## Corrective closure finding

Plans 001–005 have landed as feature implementations, but the roadmap cannot be closed yet. The planning-only baseline and every implementation commit in this sequence currently show the same Windows Actions failure at `cargo check --workspace --all-targets`; Linux and macOS are green. Separately, the Plan 001 release validation successfully built and verified all five `snp` target artifacts but intentionally refused to mutate the already-published historical release selected for the dispatch test. That proves the build matrix and immutable-release guard, but not successful draft attachment or real binary-first consumption.

Plans 006 and 007 are therefore corrective validation/closure work. They should not reopen or redesign Plans 001–005 unless their verification exposes a concrete implementation defect.

## Documentation outcome

After all plans land, the top-level README should present the bootstrap installer as the easiest path while retaining:

- the existing demo GIF;
- Cargo install as a documented fallback/manual option;
- source-build instructions;
- concise `snip-sync` startup examples;
- MCP installation examples for the common clients.

Detailed supervisor behavior belongs in `snip-sync/README.md` or a focused deployment document rather than bloating the top-level README.

## Overall closure criteria

This line of work is complete when all of the following are true:

1. A release can produce verified executables for the required five host targets without publishing crates from CI.
2. A Raspberry Pi/Le Potato-class 64-bit Linux host can install `snp` and `snip-sync` without compiling Rust when the corresponding release exists.
3. Unsupported/source-only hosts fall back to an exact-version Cargo install when Cargo is present.
4. Unix and Windows bootstrap installers reject checksum or candidate-version mismatch.
5. `snip-sync startup install` can configure the appropriate supported startup mechanism or print exact manual/elevated instructions without silently choosing an inferior duplicate mechanism.
6. `snip-sync croncheck` cannot become permanently disabled by a stale Windows maintenance lock.
7. `snp update` and `snip-sync update` can replace bootstrap-installed binaries from the exact crates.io-selected GitHub release.
8. A running `snip-sync` server returns to service after a successful update when restart permission is available.
9. `snp mcp serve` can be launched as a local stdio MCP server and exposes only the intended read-only snippet tools.
10. README/user documentation accurately reflects the implemented installation paths and retains the demo GIF.
11. Existing `bash scripts/check.sh` and relevant Linux/macOS/Windows platform/release smoke tests pass.
12. The release workflow has a green non-mutating five-target validation path and preserves its refusal to mutate published releases.
13. At least one legitimate new component release proves draft asset attachment and subsequent binary-first consumption.
14. The ARM64 Linux bootstrap path is proven against a real published release without Rust compilation.

Plan 000 must remain open until Plans 006 and 007 are complete and these closure criteria are evidenced in their completion records.

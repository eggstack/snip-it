# Plan 002: binary-first bootstrap installers

Status: complete

Depends on: Plan 001

## Objective

Provide a copy/paste installation path for Unix-like hosts and Windows that installs verified prebuilt binaries whenever possible and uses an exact-version Cargo build only when no binary exists for the detected host.

The installers are deployment helpers, not package managers.

## Files expected to change

```text
packaging/install.sh       new
packaging/install.ps1      new
packaging/README.md        optional concise contract/reference
README.md                  expose bootstrap path; retain demo GIF
snip-sync/README.md        server install/startup examples
scripts/tests/...          focused installer mapping/contract tests if useful
```

## Component interface

Unix baseline:

```text
install.sh                     # default: snp
install.sh --server            # snip-sync only
install.sh --both              # snp + snip-sync
install.sh --version X.Y.Z     # only valid for a single explicitly selected component
```

If a single `--version` flag becomes ambiguous for `--both`, reject it rather than guessing. A later implementation may use `--snp-version` and `--server-version` if pinned dual installation is genuinely needed.

Windows should expose equivalent PowerShell semantics with a `-Component Snp|Server|Both` argument.

The normal unpinned path queries crates.io separately for each requested component.

## Version authority and exact tags

For `snp`:

```text
crate metadata: https://crates.io/api/v1/crates/snip-it
version X.Y.Z
GitHub tag: vX.Y.Z
```

For `snip-sync`:

```text
crate metadata: https://crates.io/api/v1/crates/snip-sync
version A.B.C
GitHub tag: snip-sync-vA.B.C
```

Do not use the repository-wide `releases/latest` pointer to select a component version.

Use SemVer-aware validation for versions returned by crates.io. The shell installer only needs to validate/transport an already-selected stable version; it does not need to implement a general dependency resolver.

## Host mapping

Unix mapping:

```text
Linux + x86_64/amd64  -> x86_64-unknown-linux-gnu
Linux + aarch64/arm64 -> aarch64-unknown-linux-gnu
Linux + armv7l        -> armv7-unknown-linux-gnueabihf (source-only initially)
Darwin + x86_64       -> x86_64-apple-darwin
Darwin + arm64        -> aarch64-apple-darwin
```

Windows mapping:

```text
AMD64/x64 -> x86_64-pc-windows-msvc
ARM64     -> aarch64-pc-windows-msvc (source-only initially)
```

Keep this mapping synchronized with Plan 001. Add a small contract test or clearly centralized constants so filename drift is caught before release.

## Download contract

Construct exact URLs:

```text
https://github.com/eggstack/snip-it/releases/download/vX.Y.Z/snp-<target>[.exe]
https://github.com/eggstack/snip-it/releases/download/vX.Y.Z/snp-<target>[.exe].sha256

https://github.com/eggstack/snip-it/releases/download/snip-sync-vA.B.C/snip-sync-<target>[.exe]
https://github.com/eggstack/snip-it/releases/download/snip-sync-vA.B.C/snip-sync-<target>[.exe].sha256
```

Only a definite missing asset (HTTP 404) or an intentionally source-only target permits Cargo fallback.

Hard-fail on:

- checksum download failure;
- malformed checksum;
- SHA-256 mismatch;
- candidate execution failure;
- wrong program identity;
- wrong candidate version;
- TLS/transport failure;
- GitHub 5xx;
- crates.io metadata failure.

Do not hide a broken release by compiling from source.

## Candidate verification

Before installing:

1. download into a fresh temporary directory;
2. download the checksum sidecar;
3. calculate SHA-256 locally (`sha256sum` or `shasum -a 256` on Unix, `Get-FileHash` on Windows);
4. compare exact digest;
5. make the temporary Unix candidate executable;
6. run `candidate version`;
7. require exact program identity and exact requested version;
8. only then copy/install to the destination.

A pinned install must require exact equality to the requested version.

## Destination and privilege behavior

Unix:

```text
root        -> /usr/local/bin
non-root    -> $HOME/.local/bin
```

Windows:

```text
administrator -> a predictable Program Files location
non-admin     -> a predictable LOCALAPPDATA location
```

Print PATH guidance when the chosen user-local destination is not currently on PATH.

Never invoke `sudo` internally.

If the operator wants a privileged/system-wide server installation, the README may show a `sudo bash` invocation explicitly. The script should preserve the original invoking account when available (`SUDO_USER`) so it does not accidentally create the server's user config under `/root` when the intended runtime user is the caller.

Do not overwrite existing snippet/server configuration or data as part of a binary install.

## Cargo fallback

Cargo fallback requirements:

1. run only for source-only host mappings or exact asset 404;
2. require `cargo` on PATH;
3. install the exact crates.io-selected version, not floating latest;
4. use `--locked` when compatible with the published package;
5. prefer a temporary `--root` and candidate verification before final placement so a failed compile cannot partially replace the installed binary;
6. validate produced program/version exactly as for a downloaded candidate.

Conceptual commands:

```bash
cargo install snip-it --version '=X.Y.Z' --locked --root <temp>
cargo install snip-sync --version '=A.B.C' --locked --root <temp>
```

If Cargo is absent, print the detected target, the missing prebuilt condition, and the exact manual Rust/Cargo command.

## Server post-install behavior

When `snip-sync` was installed successfully:

1. initialize missing layout/config only; never overwrite existing config;
2. delegate startup registration to Plan 003's `snip-sync startup install`;
3. do not duplicate systemd/cron/launchd/Task Scheduler templates inside the installer;
4. if startup registration needs elevation, leave the binary installation successful and print the exact command to complete startup registration;
5. if a real supervisor is detected but registration cannot be performed, do not silently install a cron watchdog as a fallback duplicate.

This mirrors the successful separation used in Gregg: installer owns binary placement; binary owns daemon startup semantics.

## README entry point

The top-level README should put the easiest supported install near the existing installation section and retain the current demo GIF.

Suggested Unix form:

```bash
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash
```

Server/both variants should remain copy/paste-friendly.

PowerShell should have an equally direct `irm`/`iex` example, but document the security tradeoff of pipe-to-shell install and provide the download-then-inspect alternative in the detailed docs.

Cargo remains documented immediately below as the source/package fallback:

```bash
cargo install snip-it
cargo install snip-sync
```

## Installer tests

At minimum test the pure/isolatable behavior:

- OS/architecture -> target mapping;
- independent component tag construction;
- asset filename construction;
- source-only classification;
- 404 -> Cargo fallback;
- checksum mismatch -> hard failure, no Cargo fallback;
- candidate version mismatch -> hard failure;
- user-local destination selection;
- PATH warning behavior;
- `--both` handling.

Use local fixture HTTP endpoints or injectable base URLs for tests; production URLs must remain fixed to HTTPS GitHub/crates.io endpoints outside test mode.

## Acceptance criteria

1. One copy/paste Unix command installs `snp` from a matching release binary on x86_64 Linux, ARM64 Linux, Intel/ARM macOS.
2. PowerShell installs the x86_64 Windows binary without Rust.
3. `--server` installs `snip-sync` independently using its own crate version/tag.
4. `--both` resolves each component version independently.
5. Raspberry Pi/Le Potato-class ARM64 Linux resolves to the AArch64 GNU binary.
6. ARMv7 and other source-only targets use exact-version Cargo fallback when Cargo exists.
7. Integrity/version failures never trigger Cargo fallback.
8. Existing config/data are preserved.
9. Successful server installation delegates startup registration to `snip-sync startup install`.
10. README installation docs are accurate and the demo GIF remains present.

## Completion record

Completed: 2026-09-05

Implemented `packaging/install.sh` and `packaging/install.ps1` with independent
crates.io version resolution, exact component tags, the Plan 001 host/asset
mapping, checksum and candidate identity verification, source-only/404-only
exact-version Cargo fallback, destination/PATH handling, and `--both` version
ambiguity rejection. Server installation initializes missing layout/config and
delegates startup registration to `snip-sync startup install` when the
lifecycle command is present; it does not duplicate supervisor templates.

Added the installer contract test at `scripts/tests/installers.sh`, documented
the bootstrap path and pipe-to-shell tradeoff in the top-level and server
READMEs, and added the detailed installer contract in `packaging/README.md`.
The demo GIF remains in the top-level README. Plan 005 is now unblocked and
marked Ready; Plan 004 remains Planned because it still depends on Plan 003.

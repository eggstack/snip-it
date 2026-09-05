# Plan 004: binary-first self-update and restart integration

Status: complete

Depends on: Plans 001 and 003

## Objective

Make `snp update` and `snip-sync update` use the same exact-version, binary-first release contract as the bootstrap installer.

The updater must work for bootstrap-installed/unmanaged binaries without requiring Rust on supported targets.

## Existing behavior to replace carefully

At baseline:

- `snp update` supports Cargo and Homebrew-managed installations but rejects unmanaged executables.
- `snip-sync update` only updates Cargo-managed installations.

Preserve useful existing Homebrew behavior for an actually Homebrew-managed `snp` installation unless the repository intentionally removes that install path. Do not overwrite a package-manager-owned Cellar binary with a GitHub release executable.

For bootstrap, Cargo, and otherwise directly managed executable installs, use the new binary-first path.

## Source of truth

Each binary queries its own crates.io package:

```text
snp       -> crate snip-it
snip-sync -> crate snip-sync
```

The latest stable crate version determines whether an update exists.

Exact GitHub mapping:

```text
snp X.Y.Z       -> tag vX.Y.Z
snip-sync A.B.C -> tag snip-sync-vA.B.C
```

Never use `releases/latest` after crates.io has selected the desired version.

Use the existing `semver` dependency for comparisons. Do not write a second SemVer parser.

## Host/asset mapping

Reuse the exact target contract from Plan 001:

```text
Linux x86_64        x86_64-unknown-linux-gnu
Linux aarch64       aarch64-unknown-linux-gnu
macOS x86_64        x86_64-apple-darwin
macOS arm64         aarch64-apple-darwin
Windows x86_64      x86_64-pc-windows-msvc
```

Recognize source-only mappings from Plan 002 and use Cargo fallback directly.

Keep mapping logic small and tested. If sharing code between `snp` and `snip-sync` would require a new published crate, accept a small duplicated mapping module instead of creating release-dependency complexity.

## Update flow

For a direct/bootstrap/Cargo-managed executable:

```text
read current compiled version
-> query crates.io stable version
-> compare SemVer
-> if current: print concise message, exit 0
-> resolve host target and exact GitHub tag
-> download executable and .sha256 to temp
-> verify SHA-256
-> run candidate `version`
-> require exact program identity + exact selected version
-> stage candidate beside destination when practical
-> replace current executable
-> print from/to + source
```

### Fallback classification

Cargo fallback is allowed only when:

- target is intentionally source-only; or
- exact GitHub executable asset is HTTP 404.

Hard-fail instead of fallback on:

- crates.io lookup failure;
- GitHub transport/TLS failure;
- HTTP 5xx;
- checksum missing/malformed/mismatch;
- candidate fails to run;
- candidate program identity mismatch;
- candidate version mismatch.

## Download transport

Prefer the existing lightweight `curl` subprocess pattern unless a platform-specific existing dependency clearly simplifies the implementation without pulling another TLS stack into `snip-sync`.

Requirements:

- fixed HTTPS production hosts;
- useful User-Agent;
- bounded process/network timeout;
- captured bounded response body for metadata;
- test-only injectable endpoints behind existing test feature seams if needed.

Do not add a large self-update framework.

## SHA-256 verification

Both packages already depend on `sha2`; use Rust-native SHA-256 rather than branching to `sha256sum`/PowerShell from the updater.

This avoids platform command drift and does not add a dependency.

The checksum sidecar parser should accept the workflow's exact single-line format and reject malformed/ambiguous content.

## Cargo fallback

Build the exact selected version into temporary storage first:

```bash
cargo install snip-it --version '=X.Y.Z' --locked --root <temp-root>
cargo install snip-sync --version '=A.B.C' --locked --root <temp-root>
```

Then run the same candidate verification and replacement functions used for downloaded binaries.

Do not call `cargo install --force` directly over the running installation before the new binary has been proven valid.

If Cargo is absent, report the unsupported/missing target and exact manual path.

## Executable destination and symlinks

Use `std::env::current_exe()` as the update destination source.

Choose one explicit policy and test it:

Preferred:

- canonicalize `current_exe()`;
- replace the resolved executable target;
- preserve an outer symlink when the OS/runtime resolution makes that relationship unambiguous.

If that cannot be made reliable cross-platform without substantial code, reject symlink/wrapper installs with an actionable manual update message rather than replacing a symlink unexpectedly.

Do not assume `~/.cargo/bin` or `/usr/local/bin`.

## Unix replacement

After verification, copy the candidate into a temporary file in the destination directory and rename over the current executable.

Requirements:

- do not unlink current executable first;
- preserve executable permissions appropriate to the destination;
- clean temporary files on failure;
- report permission denied before stopping a running server when possible.

The old running Unix process may continue executing the old inode until restart; that is acceptable and allows the binary to be staged/replaced before service interruption.

## Windows replacement

Windows cannot rely on Unix running-executable rename semantics.

Keep the solution narrow:

- fully verify candidate first;
- for `snp`, use a short-lived fixed-purpose helper/self-replace mechanism to move the verified executable into place after the CLI exits;
- for `snip-sync`, Plan 003 provides safe Windows stop/restart, so stop a running server before replacing its `.exe` when necessary;
- helper arguments are fixed paths/version data, never arbitrary shell text;
- cleanup old/temp files best-effort;
- CI must exercise replacement against a temporary copy on Windows.

Evaluate a tiny `self-replace` dependency only if it materially reduces code and dependency footprint; otherwise implement the helper locally.

Do not add a general updater framework.

## `snip-sync update` lifecycle preservation

Before replacing the server binary, detect enough state to know whether it should be restarted:

```text
managed service/job installed + running
managed service/job installed + intentionally stopped
cron/task watchdog active
unmanaged/direct server running
no server running
```

Policy:

- running managed server -> replace then restart through the same manager;
- running unmanaged server -> replace then direct restart using Plan 003 lifecycle;
- installed but intentionally stopped -> update on disk and leave stopped;
- not running -> update on disk and do not start solely because update was requested.

If replacement succeeds but restart fails:

- state clearly that the new version is installed on disk;
- state whether the service is stopped or old process may still be active;
- print the exact restart command;
- return nonzero so automation detects incomplete activation.

Do not automatically roll the binary back.

## `--dry-run`

Retain/extend current dry-run semantics.

Dry-run should:

- query current/latest versions;
- resolve target/tag/asset;
- report whether the host has a prebuilt asset or would require Cargo fallback where this can be determined without downloading the full binary;
- make no file, service, or process changes.

`--locked` may remain as an override for Cargo fallback compatibility, but the normal fallback should use the repository's locked policy by default if the published crate supports it.

## Tests

Unit/pure tests:

- crates.io response parsing;
- SemVer comparison;
- component tag construction;
- host mapping;
- source-only classification;
- checksum sidecar parsing;
- candidate output validation;
- fallback classification (404 only);
- lifecycle restart decision table.

Integration tests with local fixture server:

- no update available;
- exact binary download/update;
- 404 -> Cargo fallback path selection (Cargo invocation may be mocked in unit test);
- checksum mismatch -> no replacement, no Cargo fallback;
- candidate wrong version -> no replacement;
- permission/replacement failure leaves old executable intact;
- Windows temporary-copy self replacement;
- `snip-sync` running update returns healthy new version after restart on supported CI path.

Do not hit production crates.io/GitHub in ordinary unit tests.

## Acceptance criteria

1. Bootstrap-installed `snp` updates from a verified prebuilt GitHub binary without Cargo on a supported host.
2. Bootstrap-installed `snip-sync` does the same.
3. Version choice comes from each crate's crates.io metadata, not GitHub latest.
4. Exact component tags account for independent `snp` and server versions.
5. Cargo fallback happens only for source-only/404 cases and installs an exact version into staging first.
6. Integrity/candidate mismatches never modify the installed executable.
7. Unix replacement does not delete the old executable before staging succeeds.
8. Windows replacement is tested against a real temporary executable copy.
9. A running server is restarted through its existing manager/direct lifecycle after successful update.
10. An intentionally stopped server remains stopped.
11. Restart failure after successful replacement is reported as partial success with nonzero exit.
12. Existing Homebrew-managed `snp` installs are not silently overwritten by the direct-binary updater.

## Completion record

Completed: 2026-09-05

Implemented binary-first self-update for both `snp` and `snip-sync`. Each
binary now selects its own stable crates.io version, constructs the exact
component GitHub tag and release asset, uses the Plan 001 target mapping,
verifies the single-line SHA-256 sidecar and candidate identity, and stages
the exact-version Cargo fallback only for source-only targets or a definite
asset 404. Transport, checksum, and candidate failures are hard failures and
never fall back to Cargo.

Direct, bootstrap, and Cargo-installed executables are supported. Unix
replacement stages beside the destination and renames over the old executable
without unlinking it first, preserving executable permissions. Windows uses a
fixed-purpose copied helper for replacement after the updater exits. Existing
Homebrew-managed `snp` installations remain delegated to Homebrew.

`snip-sync update` captures whether an installed manager/direct server was
running before the update, restarts only an active server through the existing
manager or direct lifecycle, leaves stopped/inactive services stopped, and
reports the installed-on-disk/failed-restart state with a manual recovery
command. Dry-run performs metadata/target/lifecycle inspection without
updating files or starting services. Added unit coverage covers target/tag/
asset mapping, checksum parsing, candidate contracts, and lifecycle decisions;
the server updater uses test-only injectable endpoints.

Plan 005 was already `Ready` because Plan 002 is complete, so no future plan
status changes are required. Plan 000 remains the roadmap umbrella and is not
an executable dependency to mark complete here.

# Supply Chain Policy

**Scope:** Dependency management, license compliance, and supply-chain integrity for the snip-it workspace.

---

## Table of Contents

- [Committed Lockfile](#committed-lockfile)
- [cargo-deny Configuration](#cargo-deny-configuration)
- [License Compatibility](#license-compatibility)
- [r-efi Clarification](#r-efi-clarification)
- [Duplicate Dependency Policy](#duplicate-dependency-policy)
- [Unknown Dependency Sources](#unknown-dependency-sources)
- [Workspace Members](#workspace-members)
- [Key Dependencies](#key-dependencies)
- [CI Verification Commands](#ci-verification-commands)
- [Advisory Exception Format](#advisory-exception-format)
- [Known Gaps](#known-gaps)

---

## Committed Lockfile

`Cargo.lock` is committed to the repository. All release builds use `--locked` to ensure deterministic dependency resolution. The lockfile captures exact versions of all transitive dependencies across the workspace.

The release profile in the root `Cargo.toml` enables LTO, single codegen unit,
size-optimized codegen (`opt-level = "z"`), and symbol stripping:

```toml
[profile.release]
lto = true
codegen-units = 1
opt-level = "z"
strip = true
panic = "abort"
```

---

## cargo-deny Configuration

The project uses `cargo-deny` (configuration in `deny.toml` at the workspace root)
as a **local supply-chain gate** — it is not a CI step. See
[CI Verification Commands](#ci-verification-commands).

### Advisory Settings

```toml
[advisories]
ignore = []
```

The advisory ignore list is empty — no advisory is suppressed, so any advisory that
`cargo deny check advisories` reports is a hard failure of the local gate. **The
gate does not currently pass**: `cargo deny check` reports open advisories against
`rustls`, `rustls-webpki`, `webpki-roots`, and the unmaintained `zbus_names`,
`zvariant`, and `zvariant_derive` crates. See
[Known Gaps](#cargo-deny-check-currently-fails). If a future advisory cannot be
immediately resolved (e.g., transitive dependency lag), it must be documented with
a rationale, owner, and review date (see
[Advisory Exception Format](#advisory-exception-format)).

### License Settings

```toml
[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Zlib",
    "0BSD",
    "MPL-2.0",
    "BSL-1.0",
]
```

Unlicensed or non-listed licenses cause `cargo deny check licenses` to fail.

### Ban Settings

```toml
[bans]
multiple-versions = "warn"
wildcards = "warn"
```

Duplicate dependency versions produce a warning, not a hard failure. Wildcard version requirements also produce a warning. Neither blocks CI.

### Source Restrictions

```toml
[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

Dependencies from unknown registries or git sources are denied. Only crates.io is permitted as a registry source.

---

## License Compatibility

The following licenses are approved for use in the snip-it workspace:

| License | SPDX Identifier | Notes |
|---------|-----------------|-------|
| MIT | `MIT` | Primary project license; most dependencies |
| Apache License 2.0 | `Apache-2.0` | Compatible with MIT; dual-licensed crates |
| BSD 2-Clause | `BSD-2-Clause` | Simplified BSD |
| BSD 3-Clause | `BSD-3-Clause` | Modified BSD |
| ISC License | `ISC` | Functionally equivalent to MIT |
| Unicode License 3.0 | `Unicode-3.0` | Unicode data tables |
| Zlib License | `Zlib` | Compression-related crates |
| 0BSD | `0BSD` | Zero-clause BSD; minimal restrictions |
| Mozilla Public License 2.0 | `MPL-2.0` | File-level copyleft; compatible with MIT |
| Boost Software License 1.0 | `BSL-1.0` | Permissive; Rust's Boost Software License, used by `clipboard-win` and `error-code` |

Licenses not on this list will cause `cargo deny check licenses` to fail unless
explicitly added to the allow list. The list is a policy ceiling, not a snapshot
of the resolved graph: `BSD-2-Clause` is currently allowed but not reached by any
crate in the resolved graph, and `cargo deny` reports it as an unmatched
allowance.

The allow list also does not cover every license present in `Cargo.lock`:
`webpki-roots` (pulled in by `eggfetch-core`) is `CDLA-Permissive-2.0`, which is
not on the list and therefore fails the license check. See
[Known Gaps](#cargo-deny-check-currently-fails).

---

## r-efi Clarification

The `r-efi` crate is dual-licensed MIT/Apache-2.0/LGPL-2.1-or-later but is only pulled in transitively for UEFI targets, which are not part of any build matrix. To avoid widening the license allow list to include LGPL, `deny.toml` includes a targeted clarification:

```toml
[[licenses.clarify]]
name = "r-efi"
expression = "MIT OR Apache-2.0"
license-files = []
```

This keeps `r-efi` acceptable to `cargo deny check licenses` without adding LGPL
to the global allow list. The clarification applies only to the
MIT/Apache-2.0 dual-license portion of `r-efi`.

---

## Duplicate Dependency Policy

Duplicate dependency versions are warned, not denied. The workspace has known
duplications driven by transitive dependency constraints — for example `sha2` and
`digest` are split between the workspace's own `sha2 0.11` / `argon2` (via
`blake2`, `digest 0.11`) path and the `sqlx-core` / `secret-service` path
(`sha2 0.10`, `digest 0.10`), and `getrandom` appears at more than one major
version.

**Do not hand-maintain a version table here** — it re-stales on every lockfile
bump. Regenerate the current picture from the workspace root with:

```bash
cargo tree -d          # or: cargo tree --duplicates
```

The command prints each duplicated crate with its versions and the dependency
paths that pull each version in, for every workspace member.

These duplications are acceptable as long as each version is individually sound and the total binary size impact is bounded. The `bans.multiple-versions = "warn"` setting ensures visibility without blocking CI.

---

## Unknown Dependency Sources

```toml
[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

All dependencies must come from crates.io (the only allowed registry). Git dependencies are denied. If a git dependency is ever required, it must be added to an explicit allow list in `deny.toml` with documented justification.

---

## Workspace Members

| Crate | Version | Description |
|-------|---------|-------------|
| `snip-it` | 1.3.9 | Main binary crate (`snp`). CLI, TUI, sync client, encryption, auto-sync. |
| `snip-proto` | 0.1.3 | Protobuf definitions and tonic-generated gRPC code. |
| `snip-sync` | 0.1.6 | gRPC sync server (EggServe HTTP leaf + tonic gRPC, SQLite storage). |

All three crates share the same license (MIT), Rust edition (2024), and minimum supported Rust version (1.94).

---

## Key Dependencies

The following direct dependencies are critical to the project's security and functionality:

| Crate | Version | Purpose | Security Relevance |
|-------|---------|---------|-------------------|
| `aes-gcm` | 0.11 | AES-256-GCM authenticated encryption | Encrypts snippet content in the sync payload (ciphertext at rest on the server; TLS on the wire). Local snippet TOML files are **not** encrypted at rest |
| `argon2` | 0.6 | Argon2id key derivation | Derives encryption keys from API keys |
| `keyring` | 4 | OS keychain integration | Stores API keys securely in platform keychain |
| `zeroize` | 1 | Zeroing memory for secrets | Ensures sensitive buffers are wiped after use |
| `tonic` | 0.14 | gRPC framework (client + server) | Transport layer for sync protocol |
| `prost` | 0.14 | Protobuf serialization | Wire format for sync messages |
| `ratatui` | 0.30 | Terminal UI framework | Renders the interactive TUI |
| `crossterm` | 0.29 | Terminal manipulation | Input handling, raw mode, cursor control |
| `tokio` | 1 | Async runtime | Powers async gRPC calls and server I/O |

Other notable dependencies include `serde`/`toml` for configuration persistence, `clap` for CLI argument parsing, `sqlx` (server-side) for SQLite storage, and `eggserve-server`/`eggserve-primitives` (server-side) for the HTTP leaf service.

---

## CI Verification Commands

Linux CI runs `bash scripts/check.sh`: installer contract, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings` (NOT `--all-features`),
`cargo test --workspace --lib`, plus focused integration targets with
`--features test-support` (see `AGENTS.md`). `cargo deny check` is a local
supply-chain gate via `deny.toml`, not a CI step.

---

## Advisory Exception Format

If a security advisory must be temporarily ignored (e.g., waiting for an upstream fix), the following metadata is required in `deny.toml`:

```toml
[advisories]
ignore = [
    {
        id = "RUSTSEC-YYYY-NNNN",
        reason = "Short description of why this advisory is not exploitable or is safe to ignore",
        owner = "GitHub username or team responsible",
        review_date = "YYYY-MM-DD"
    }
]
```

Each entry must include:

- **id:** The RUSTSEC advisory identifier.
- **reason:** A clear explanation of why the advisory does not affect this project (e.g., the vulnerable code path is not reachable, the dependency is not used in production, or an upgrade is in progress).
- **owner:** The person or team responsible for resolving or re-evaluating the exception.
- **review_date:** The date by which the exception must be re-evaluated.

Exceptions without all four fields are not accepted.

---

## Known Gaps

### `cargo deny check` Currently Fails

The local supply-chain gate does not currently pass. As of the last review,
`cargo deny check` reports:

- **Advisories (9 open):** `rustls 0.23.43`
  (RUSTSEC-2026-0285, RUSTSEC-2026-0346, RUSTSEC-2026-0348, RUSTSEC-2026-0349),
  `rustls-webpki 0.103.14` (RUSTSEC-2026-0382), `webpki-roots 1.0.9`
  (RUSTSEC-2026-0381), and the unmaintained crates `zbus_names`
  (RUSTSEC-2026-0371), `zvariant` (RUSTSEC-2026-0370), and `zvariant_derive`
  (RUSTSEC-2026-0372).
- **Licenses:** `webpki-roots` is `CDLA-Permissive-2.0`, which is not on the
  license allow list (it reaches the workspace through `eggfetch-core`).

`bans` and `sources` pass. Because `cargo deny` is a local gate and not a CI
step, this failure does not block a merge — it is a standing gap, not a
regression detector. Resolve it by upgrading the affected crates, by extending
the license allow list with a documented rationale, or by adding an explicit
advisory exception with an owner and review date (see
[Advisory Exception Format](#advisory-exception-format)).

### No SBOM Generation

The project does not currently generate a Software Bill of Materials (SBOM). The `Cargo.lock` file serves as the de facto dependency manifest, but a formal SBOM in SPDX or CycloneDX format is not produced. This is a candidate for future work.

### No Build Provenance Attestation

The project does not currently produce build provenance attestations (e.g., SLSA provenance, in-toto attestations). Release binaries are not signed or attested beyond the `--locked` flag and `cargo deny` checks. This is a candidate for future work, particularly if publishing to a package registry or distributing signed binaries.

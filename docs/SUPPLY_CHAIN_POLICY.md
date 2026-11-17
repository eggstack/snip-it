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
for advisories, licenses, bans, and sources, plus `cargo-audit` with the mirror
configuration in `.cargo/audit.toml`. Both run through
`scripts/check-advisories.sh`, which is a **blocking CI step**
(`.github/workflows/ci.yml`, job `advisory-gate`). The policy for suppressions is
[ADR-0003](../plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md).

### Advisory Settings

`deny.toml` carries one `ignore` list of bare advisory IDs. Each entry is
preceded by a comment block with `ID`, `Scope`, `Why`, `Added`, `Review by`, and
`Remove when`; `scripts/check-advisories.sh` fails if a field is missing, if an
entry is not a bare `RUSTSEC-*` ID, or if `.cargo/audit.toml` and
`SECURITY.md`'s `advisory-ignore-ids` marker disagree with it.

Current suppressions (all mitigated, none a permanent destination):

| Advisory | Crate | Removal condition |
| -------- | ----- | ----------------- |
| `RUSTSEC-2026-0459` | `libsqlite3-sys` (bundled SQLite, server only) | `sqlx` accepts `libsqlite3-sys >= 0.39.0` |
| `RUSTSEC-2026-0370` | `zvariant` | Linux keyring chain moves to `zbus 6.x` |
| `RUSTSEC-2026-0371` | `zbus_names` | Linux keyring chain moves to `zbus 6.x` |
| `RUSTSEC-2026-0372` | `zvariant_derive` | Linux keyring chain moves to `zbus 6.x` |

Global lint-level relaxation is prohibited: `unmaintained` advisories must be
listed individually so a **new** unmaintained or vulnerable crate still fails the
gate.

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
    "CDLA-Permissive-2.0",
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
| Community Data License Agreement – Permissive 2.0 | `CDLA-Permissive-2.0` | Permissive **data** license; covers certificate-root bundles (`webpki-roots`), not executable code |

Licenses not on this list will cause `cargo deny check licenses` to fail unless
explicitly added to the allow list. The list is a policy ceiling, not a snapshot
of the resolved graph: `BSD-2-Clause` is currently allowed but not reached by any
crate in the resolved graph, and `cargo deny` reports it as an unmatched
allowance. That warning is not a gate and the entry is retained deliberately, so
the next crate that needs it does not require a policy change under time
pressure.

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
| `snip-it` | 1.4.1 | Main binary crate (`snp`). CLI, TUI, sync client, encryption, auto-sync. |
| `snip-proto` | 0.1.4 | Protobuf definitions and tonic-generated gRPC code. |
| `snip-sync` | 0.2.0 | gRPC sync server (EggServe HTTP leaf + tonic gRPC, SQLite storage). |

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

Linux CI runs two independent jobs in `.github/workflows/ci.yml`:

- `advisory-gate` — `bash scripts/check-advisories.sh`, with `cargo-audit` and
  `cargo-deny` installed at exact pinned versions. It fails on an unexpected
  advisory, a missing rationale field, or a suppression-list mismatch.
- `linux-correctness` — `bash scripts/check.sh`: installer contract,
  `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings` (NOT `--all-features`),
  `cargo test --workspace --lib`, plus focused integration targets with
  `--features test-support` (see `AGENTS.md`).

`macos-latest` and `windows-latest` run `cargo check --workspace --all-targets`
and `platform_smoke`; Windows additionally runs the PowerShell installer contract.

Locally, `bash scripts/check-advisories.sh --self-test` additionally proves the
gate still fails with one suppression removed.

---

## Advisory Exception Format

If a security advisory cannot be resolved immediately (for example, a transitive
dependency has not widened its version bound yet), it is suppressed by **bare
advisory ID** in `deny.toml`, with a comment block immediately above it:

```toml
    # ID:          RUSTSEC-YYYY-NNNN
    # Scope:       which crate and which build unit is affected
    # Why:         exploitability analysis (attack vector, preconditions, blast radius)
    # Added:       YYYY-MM-DD
    # Review by:   YYYY-MM-DD (<= 90 days)
    # Remove when: the concrete upstream condition that ends the exception
    "RUSTSEC-YYYY-NNNN",
```

Every entry must supply all six fields. `scripts/check-advisories.sh` enforces
their presence and rejects anything that is not a bare advisory ID — crate names,
version ranges, and wildcards are not permitted, because those would silently
cover future unrelated findings. The same ID must appear in
`.cargo/audit.toml` and in `SECURITY.md`'s `advisory-ignore-ids` marker.

`cargo-deny` does not accept structured tables inside `ignore`; the rationale
lives in comments by design. An exception is removed in the same change that
adopts the fixed release, without waiting for its review date.

---

## Known Gaps

### Advisory Suppressions Awaiting Upstream

`cargo audit` and `cargo deny check` pass, but only because four advisory IDs are
suppressed with named removal conditions (see
[Advisory Settings](#advisory-settings)). The accepted residual risk is the
bundled SQLite vulnerability in the `snip-sync` server binary; its local attack
vector is blocked by owner-only server data permissions on Unix. The actual
findings, rationale, and review dates are in `deny.toml` and
`SECURITY.md` § Advisory Exceptions — this document does not duplicate them, so
it cannot drift.

The removal conditions are upstream facts. If `sqlx` publishes a release that
accepts `libsqlite3-sys >= 0.39.0`, or the `keyring` chain adopts `zbus 6.x`,
the corresponding entries are deleted rather than re-reviewed.

### No SBOM Generation

The project does not currently generate a Software Bill of Materials (SBOM). The `Cargo.lock` file serves as the de facto dependency manifest, but a formal SBOM in SPDX or CycloneDX format is not produced. This is a candidate for future work.

### No Build Provenance Attestation

The project does not currently produce build provenance attestations (e.g., SLSA provenance, in-toto attestations). Release binaries are not signed or attested beyond the `--locked` flag and `cargo deny` checks. This is a candidate for future work, particularly if publishing to a package registry or distributing signed binaries.

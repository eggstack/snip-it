# Security Policy

## Supported Versions

| Version | Supported          | Notes |
| ------- | ------------------ | ----- |
| 1.4.x   | :white_check_mark: | Full support |
| 1.3.x   | :warning:          | Security fixes only |
| < 1.3   | :x:                | Unsupported |

The table always includes the version in the repository's `Cargo.toml`; if it
does not, this document is stale. Security fixes are backported one minor
release behind the current line. Payload-format compatibility across devices is
a separate, stricter contract and is stated in
[Sync Encryption](#sync-encryption).

## Threat Model

The full threat model is documented in
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md). It covers asset
classification, trust boundaries, attacker capabilities, and
residual risk for the snp CLI, snip-sync server, and sync protocol.

## Security Architecture

### Snippet Execution

Only `snp run` invokes a shell to execute a snippet's command field.
All other commands — `snp list`, `snp search`, `snp clip`, `snp edit`,
`snp sync`, `snp import`, `snp validate`, `snp backup`, `snp restore`,
`snp repair`, `snp doctor`, `snp status`, `snp register`, `snp premade` —
treat the command field as opaque data and never execute it.

### Encryption

Snippets are encrypted client-side before leaving the machine. Only the snippet
body (`description`, `command`, `tags`) is encrypted; routing metadata (`id`,
timestamps, `device_id`, `deleted`) travels in the clear — see
[Sync Encryption](#sync-encryption).

- **Algorithm:** AES-256-GCM (an AEAD cipher). Payload version 1 does not use
  associated data; version 2 binds the ciphertext to the snippet id. See
  [Sync Encryption](#sync-encryption) for the version support matrix.
- **Key derivation:** Argon2id with 16 MiB memory cost, 3 iterations,
  and 4 degrees of parallelism (OWASP-recommended parameters). The
  encryption key is derived from the API key and a per-payload random
  salt.

### Credential Storage

API keys are stored in the OS keychain (macOS Keychain, GNOME Keyring,
Windows Credential Manager). Keychain-backed configs store the `@keychain`
marker, never the key itself. On save, if the keychain is unavailable the write
is **refused** (`keychain unavailable, refusing to store API key in plaintext`)
rather than silently downgraded to a plaintext file. On load, a plaintext key
already present in `sync.toml` is migrated into the keychain and the file
re-saved with the marker; a marker whose keychain entry has been lost is a hard
error, not a silent empty credential. Keep `keyring = "4"` with its default
features: keyring 4's `default` is `v1`, which hard-enables the three platform
stores, and `keyring::Entry` is gated on `v1` — building with
`default-features = false` is a **compile error**, not a silent downgrade.
There is no mock store.

`SNP_ALLOW_PLAINTEXT_API_KEY=true` is a **test-only** seam, compiled in under
`#[cfg(feature = "test-support")]` (`src/config/sync_settings.rs`). Setting it
in a production build has no effect at all — it cannot weaken or bypass
keychain storage. In a `test-support` build it forces plaintext storage on save
*and* forbids keychain access on load, so a config still holding the `@keychain`
marker fails to load rather than authenticating with the literal marker string.

### File Permissions

On Unix, `snp` creates its config directory with mode `0o700` and
writes config, library, premade-library, and sync files with mode
`0o600`. These limits protect local snippet data and the API key
when the keychain is unavailable.

### Process Isolation

The detached auto-sync worker (`snp auto-sync-worker`) holds the
`SyncExecutionLock` for the duration of the sync cycle without
blocking the parent CLI; it invokes `run_sync` directly with no
descendant executor process. No secrets are placed in process arguments;
they are passed through file descriptors or environment variables only.

### Supply Chain

- `scripts/check-advisories.sh` is the supply-chain gate. It runs `cargo audit`
  and `cargo deny check` over the committed `Cargo.lock`, and fails if the local
  suppression lists drift from this document.
- The same script runs in CI on every push and pull request to `main`
  (`.github/workflows/ci.yml`, `advisory-gate`), so a new advisory fails a pull
  request instead of reaching a release. Tool versions are pinned.
- Third-party workflow actions are pinned to full commit SHAs with a version
  comment; Dependabot keeps them current.
- Locked `Cargo.lock` files ensure reproducible builds and prevent
  silent dependency version changes.

## Reporting a Vulnerability

If you discover a security vulnerability in snp, please report it
responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: **dbowman91@proton.me** (PGP key on request).

Include the following in your report:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

## Response Timeline

- **Acknowledgment:** Within 48 hours
- **Initial assessment:** Within 1 week
- **Fix or mitigation:** Critical issues within 2 weeks, others within
  30 days

## Security Considerations

### Snippet Execution

Snippet commands are executed as-is via your shell. **Only add snippets
you trust.** Avoid storing snippets that contain secrets (passwords,
tokens, API keys) in plaintext TOML files; snippets are not encrypted
at rest.

### Non-Execution Guarantee

Only `snp run` invokes a shell. The following commands never execute
snippets:

- `snp get`
- `snp list`
- `snp search`
- `snp select`
- `snp clip`
- `snp sync`
- `snp import`
- `snp validate`
- `snp backup`
- `snp restore --dry-run`
- `snp repair --dry-run`
- `snp doctor`
- `snp status`
- `snp register`
- `snp premade`

When using exact selectors (`--id`, `--description-exact`,
`--command-exact`), `snp run` bypasses the TUI and executes directly.
This is a deterministic path for automation; the caller is responsible
for verifying the target snippet.

### Self-Update Verification

Standalone self-update packages (direct download, not Homebrew or
crates.io) include a SHA-256 checksum file. The updater verifies the
checksum of the downloaded archive before extraction. Archive entries
are validated: absolute paths, parent-directory traversal, symlinks,
and hard links are rejected. HTTPS-only downloads prevent MITM attacks.

**Limitation:** the checksum is fetched from the same origin as the archive and
is not signed. There is no release signature or attestation, so an attacker who
controls the release channel can serve a matching archive and checksum. The
check defends against in-transit corruption and partial compromise of the
channel; it does not establish publisher identity.

### Sync Encryption

Read this as three separate statements, because they have three different
strengths.

**1. What is encrypted.** Snippet *content* — `description`, `command`, and
`tags` — is encrypted client-side with **AES-256-GCM** before it leaves the
device, and the server stores only ciphertext for those fields. Per-snippet
metadata is **plaintext** by design: `id`, `created_at`, `updated_at`,
`device_id`, and `deleted` are proto fields the server needs for merge ordering
and deletion tombstones. `output` is local-only and never sent. The key is
derived with **Argon2id** (16 MiB memory, 3 iterations, 4 lanes — OWASP
recommended parameters) from the API key and a per-payload random salt.

**2. What the server can do.** A server that stores ciphertext and never sees
the key cannot decrypt the content it holds. That is *not* the current
deployment model: clients send the API key as a bearer token in gRPC
`authorization` metadata on every request, so the server, a TLS-terminating
proxy, or a request log that observes the token can re-derive the same Argon2id
key and decrypt — and re-encrypt — any payload it holds or relays. The
protection this design actually provides is **transport confidentiality plus
at-rest obfuscation of snippet bodies, not server-blindness**. Treat a sync
server as a party that can read and modify your snippet content; trust it for
availability and for not leaking what it stores, not for confidentiality from
itself.

**3. What the authentication tag proves.** The GCM tag proves that a party
*without* the key did not modify the ciphertext: tampering by a key-blind
party fails. It does not prevent a key holder from forging content — a party
with the key can seal any plaintext it likes. It also does not bind the
ciphertext to the snippet row it is stored under until payload version 2 is
written; see the version matrix below.

**Payload version support matrix:**

| Payload version | Read | Written | Binds ciphertext to snippet id |
| --------------- | ---- | ------- | ------------------------------ |
| 0 (legacy, pre-magic) | Yes | No | No |
| 1 | Yes | Yes | No |
| 2 | Yes | Not yet | Yes (AAD over the snippet id) |

Version 1 payloads remain readable indefinitely; no stored payload is ever
rewritten. `snip-it` 1.4.1 is the minimum client version that can read version 2
payloads; earlier clients skip version 2 rows during sync. This release adds
read support only: the writer still emits version 1 payloads, so version 2's id
binding is not yet in effect for newly written payloads. The decision and the
two-phase rollout are recorded in
[ADR-0002](plans/adrs/ADR-0002-encryption-payload-identity-binding.md); the
implementation-level description is
[`architecture/encryption.md`](architecture/encryption.md).

**Other properties:**

- **API keys at rest:** Stored in the OS keychain (macOS Keychain, GNOME
  Keyring, Windows Credential Manager) by default. Fall back to a
  plaintext `sync.toml` only when `SNP_ALLOW_PLAINTEXT_API_KEY=true`
  is set; a warning is emitted at runtime. That variable is a test-only seam and
  has no effect in a production build — see
  [Credential Storage](#credential-storage).
- **Transport:** TLS via `https://`. The client rejects plaintext `http://` to
  non-loopback hosts unless `SNIP_SYNC_ALLOW_HTTP=true` is set.
- **`sync.toml` integrity:** a CRC32 comment line detects accidental
  corruption. This is *not* a cryptographic integrity check.
- **Authentication:** Current clients send the API key as a bearer
  token in gRPC `authorization` metadata. The proto request-body
  `api_key` fields remain for backward compatibility with older
  clients and are ignored when metadata is present.

### Advisory Exceptions

`deny.toml` is the normative suppression list; `.cargo/audit.toml` mirrors it
so `cargo audit` and `cargo deny check` reach the same verdict. Both are checked
by `scripts/check-advisories.sh`. Every entry is an advisory **ID only** — never
a crate name, version, or wildcard — and carries the rationale block defined by
[ADR-0003](plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md):
scope, exploitability analysis, added date, review-by date, and the upstream
condition that removes it.

<!-- advisory-ignore-ids: RUSTSEC-2026-0459 RUSTSEC-2026-0370 RUSTSEC-2026-0371 RUSTSEC-2026-0372 -->

| Advisory | Crate | Kind | Status |
| -------- | ----- | ---- | ------ |
| `RUSTSEC-2026-0459` | `libsqlite3-sys` (bundled SQLite) | Vulnerability, CVSS 6.8 | Accepted until `sqlx` accepts `libsqlite3-sys >= 0.39.0` |
| `RUSTSEC-2026-0370` | `zvariant` | Unmaintained | Accepted until the Linux keyring chain moves to `zbus 6.x` |
| `RUSTSEC-2026-0371` | `zbus_names` | Unmaintained | Accepted until the Linux keyring chain moves to `zbus 6.x` |
| `RUSTSEC-2026-0372` | `zvariant_derive` | Unmaintained | Accepted until the Linux keyring chain moves to `zbus 6.x` |

What each exception means in practice:

- **`RUSTSEC-2026-0459`** is reachable only in the `snip-sync` server binary, and
  only with a local vector: an attacker must already be able to write a crafted
  database and a sibling `-wal` file next to the server's data. The server data
  directory is owner-only on Unix (see
  [Local File Permissions](#local-file-permissions)), so an unprivileged local
  user cannot meet that precondition. The `snp` client is unaffected.
- **`RUSTSEC-2026-0370`/`0371`/`0372`** are unmaintained notices with no CVE and
  no fixed standalone release; the crates were folded into `zbus 6.x`. They are
  compiled only for the Linux Secret Service keyring backend and parse D-Bus
  traffic from a trusted same-session peer.

Suppressions are never a destination: an entry is deleted in the same change
that adopts the fixed release, without waiting for its review date. Do not add
an entry without a fresh exploitability analysis, and do not relax
`unmaintained` globally — the gate must keep failing on anything new.

Check locally with `scripts/check-advisories.sh`; pass `--self-test` to prove the
gate still rejects an unsuppressed advisory.

### Server Deployment

If you self-host the `snip-sync` server:

- **Terminate TLS in front of the server.** `snip-sync` does **not** terminate
  TLS itself: it builds its gRPC listener without any TLS configuration.
  `TLS_ENABLED=true` is an assertion that a reverse proxy terminates TLS for it,
  not a switch that enables TLS in the binary. Binding a non-loopback address
  without such a proxy serves plaintext.
- **Use a reverse proxy** (nginx, traefik, Caddy) to terminate TLS
  with a real certificate (Let's Encrypt, etc.).
- **Use strong, unique API keys.** The server enforces a minimum
  length and stores hashes with Argon2id; plaintext keys are never
  written to the database.
- **Configure rate limiting.** Default is 120 requests per minute per
  API key; tune for your traffic via `[server.rate_limit]` in
  `config.toml`.
- **Keep the server updated.** Subscribe to releases on GitHub to be
  notified of security fixes.
- **Restrict metrics endpoint access.** The `/metrics` endpoint
  returns Prometheus-format data; require basic auth (`[server.metrics]`
  `username`/`password`) or expose it only on an internal interface.
  Repeated failed authentications are rate-limited (`429` with `Retry-After`)
  by a process-local failure budget; successful scrapes are never throttled.
- **Keep the data directory private.** On Unix the server creates its data
  directory `0o700` and the SQLite database with its `-wal`/`-shm` siblings
  `0o600`. An existing broader mode that cannot be tightened produces a startup
  warning naming the path and its mode. Windows ACL enforcement is not
  implemented; restrict the directory with filesystem permissions yourself.
- **Back up the SQLite database** (`snippets.db` by default) using
  the same backup strategy you would for any user data store.

### Local File Permissions

On Unix, `snp` creates its config directory with mode `0o700` and
writes config, library, premade-library, and sync files with mode
`0o600`. These limits help protect local snippet data and the API key
when the keychain is unavailable.

## Known Limitations

- **CRC32 integrity checks** detect accidental corruption but do not
  authenticate against a malicious local actor. A local attacker with
  filesystem access can tamper with files without triggering integrity
  failures.
- **No mutual TLS / client certificate authentication.** The sync
  protocol authenticates via API key bearer tokens only. Clients
  without a valid API key are rejected, but additional client
  certificate verification is not currently supported.
- **`/health` discloses the exact server version** to any caller that can reach
  the endpoint, without authentication. This is intentional fleet diagnostics;
  hide the endpoint behind your proxy if that disclosure matters to you.
- **Update integrity is checksum-only.** The updater verifies a same-origin
  SHA-256 sidecar and no signature; a compromised release channel defeats it.
- **Payload identity binding is not yet written.** Payload version 2 binds
  ciphertext to the snippet id, and the client can read it, but the write switch
  has not shipped — see [Sync Encryption](#sync-encryption).
- **Server data permissions are enforced on Unix only.** Windows deployments
  must restrict the data directory themselves.

## Security Audit

[`docs/archive/SECURITY_AUDIT.md`](docs/archive/SECURITY_AUDIT.md) is a
**historical snapshot**, not a current contract: it is archived, and the paths
and line references it cites have moved. It is retained as the record of the
methodology used at the time.

The current posture is this document plus
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) and the deep dives under
[`architecture/`](architecture/). Dependency findings and their scoped
exceptions are in [Advisory Exceptions](#advisory-exceptions). Any future audit
summary belongs in a new document rather than in the frozen archive.

## Scope

This security policy applies to the `snp` CLI tool, the `snip-sync`
server, and the `snip-proto` definitions — all part of the
[`eggstack/snip-it`](https://github.com/eggstack/snip-it) repository.
Third-party integrations and **premade libraries** (downloaded via
`snp premade`) are **not** covered: review them before installing.

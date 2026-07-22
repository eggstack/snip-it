# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.3.x   | :white_check_mark: |
| 1.2.x   | :white_check_mark: |
| 1.1.x   | :white_check_mark: |
| < 1.1   | :x:                |

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

Snippets are encrypted client-side before leaving the machine.

- **Algorithm:** AES-256-GCM (authenticated encryption with
  associated data).
- **Key derivation:** Argon2id with 16 MiB memory cost, 3 iterations,
  and 4 degrees of parallelism (OWASP-recommended parameters). The
  encryption key is derived from the API key and a per-payload random
  salt.

### Credential Storage

API keys are stored in the OS keychain (macOS Keychain, GNOME Keyring,
Windows Credential Manager) by default. Plaintext storage in
`sync.toml` is gated behind the `SNP_ALLOW_PLAINTEXT_API_KEY`
environment variable. A runtime warning is emitted when the plaintext
fallback is active.

### File Permissions

On Unix, `snp` creates its config directory with mode `0o700` and
writes config, library, premade-library, and sync files with mode
`0o600`. These limits protect local snippet data and the API key
when the keychain is unavailable.

### Process Isolation

The auto-sync worker and executor run as detached processes. The
executor is spawned as a separate process so that the worker's
`SyncExecutionLock` is held for the duration of the sync cycle without
blocking the parent CLI. No secrets are placed in process arguments;
they are passed through file descriptors or environment variables only.

### Supply Chain

- `cargo-deny` checks for known security advisories and license
  compliance on every CI run.
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

### Sync Encryption

- **In transit:** All sync payloads are encrypted client-side with
  **AES-256-GCM** before being sent to the server. The server only
  stores ciphertext.
- **Key derivation:** Per-snippet **Argon2id** (16 MiB memory, 3
  iterations, 4 threads — OWASP-recommended parameters) derives an
  encryption key from the API key and a per-payload random salt.
- **API keys:** Stored in the OS keychain (macOS Keychain, GNOME
  Keyring, Windows Credential Manager) by default. Fall back to a
  plaintext `sync.toml` only when `SNP_ALLOW_PLAINTEXT_API_KEY=true`
  is set; a warning is emitted at runtime.
- **Integrity:** `sync.toml` carries a CRC32 comment line to detect
  accidental corruption. This is *not* a cryptographic integrity check
  — the threat model assumes local-only access.
- **Authentication:** Current clients send the API key as a bearer
  token in gRPC `authorization` metadata. The proto request-body
  `api_key` fields remain for backward compatibility with older
  clients and are ignored when metadata is present.

### Known Suppressed Advisories

`deny.toml` ignores `RUSTSEC-2024-0437` (uncontrolled recursion /
stack overflow in `protobuf 2.28.0`, pulled in transitively by
`prometheus 0.13.x`). The `snip-sync` server does not parse
untrusted protobuf data on this code path — `prometheus` is used
only for its text encoder in the `/metrics` endpoint, and the
`protobuf` crate itself is built but not used to deserialize
attacker-controlled input. The advisory is therefore not
exploitable in the snip-it / snip-sync build. We track the
upstream `prometheus` crate for a release that removes the old
`protobuf` dependency.

### Server Deployment

If you self-host the `snip-sync` server:

- **Enable TLS in production.** The client refuses to connect over
  plaintext HTTP unless `SNIP_SYNC_ALLOW_HTTP=true` is set.
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

## Security Audit

For detailed audit findings and the methodology used, refer to
[`docs/SECURITY_AUDIT.md`](docs/SECURITY_AUDIT.md).

## Scope

This security policy applies to the `snp` CLI tool, the `snip-sync`
server, and the `snip-proto` definitions — all part of the
[`eggstack/snip-it`](https://github.com/eggstack/snip-it) repository.
Third-party integrations and **premade libraries** (downloaded via
`snp premade`) are **not** covered: review them before installing.

# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.2.x   | :white_check_mark: |
| 1.1.x   | :white_check_mark: |
| < 1.1   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in snp, please report it responsibly:

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
- **Fix or mitigation:** Critical issues within 2 weeks, others within 30 days

## Security Considerations

### Snippet Execution

Snippet commands are executed as-is via your shell. **Only add snippets
you trust.** Avoid storing snippets that contain secrets (passwords,
tokens, API keys) in plaintext TOML files; snippets are not encrypted
at rest.

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

## Scope

This security policy applies to the `snp` CLI tool, the `snip-sync`
server, and the `snip-proto` definitions — all part of the
[`anomalyco/snip-it`](https://github.com/anomalyco/snip-it) repository.
Third-party integrations and **premade libraries** (downloaded via
`snp premade`) are **not** covered: review them before installing.

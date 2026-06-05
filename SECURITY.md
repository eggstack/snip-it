# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.1.x   | :white_check_mark: |
| < 1.1   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in snp, please report it responsibly:

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: [security@anomalyco.com](mailto:security@anomalyco.com)

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

Snippet commands are executed as-is via your shell. **Only add snippets you trust.** Avoid storing snippets with sensitive data (passwords, tokens, API keys) as they are stored in plaintext TOML files.

### Sync Encryption

All snippets are encrypted with AES-256-GCM before sync. API keys are derived using Argon2id with OWASP-recommended parameters. API keys are stored in your OS keychain by default.

### Server Deployment

If you self-host the snip-sync server:
- Enable TLS in production (required by default)
- Use strong API keys
- Configure rate limiting appropriately
- Keep the server and database updated

## Scope

This security policy applies to the `snp` CLI tool and `snip-sync` server. Third-party integrations and premade libraries are not covered.

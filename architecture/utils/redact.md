# redact.rs — Secret Redaction for Display & Logs

[← Back to Overview](../overview.md)

## Overview

Best-effort redaction of secrets (API keys, bearer tokens, URLs with embedded credentials) from human-readable strings. Defense-in-depth for display/log paths, **not** a security boundary: the real guarantee is never placing secrets into these strings.

**File**: `src/utils/redact.rs` (72 lines, including unit tests)

## API

```rust
pub(crate) fn redact_secrets(msg: &str) -> String
```

Single shared definition so redaction cannot drift between callers. Three `LazyLock<regex::Regex>` patterns applied in order, each replacing the secret portion with `[REDACTED]`:

| Pattern | Matches | Example |
|---------|---------|---------|
| `SECRET_ASSIGNMENT_RE` | `api_key` / `apikey` / `api-key` / `token` / `password` / `passwd` / `secret` / `credential` / `authorization` followed by `=` and a quoted or bare value (case-insensitive) | `api_key = "hunter2"` → `api_key = [REDACTED]` |
| `BEARER_RE` | `Bearer <token>` (case-insensitive scheme) | `Authorization: Bearer hunter2` → `Authorization: Bearer [REDACTED]` |
| `URL_CREDENTIALS_RE` | `http(s)://user[:pass]@` userinfo | `https://user:s3cret@example.com/x` → `https://[REDACTED]@example.com/x` |

Plain text passes through untouched (`redact_secrets("git status") == "git status"`).

## Worked Examples

| Input | Output |
|-------|--------|
| `api_key = "hunter2"` | `api_key = [REDACTED]` |
| `token='abc123'` | `token=[REDACTED]` |
| `Authorization: Bearer hunter2` | `Authorization: Bearer [REDACTED]` |
| `https://user:s3cret@example.com/x` | `https://[REDACTED]@example.com/x` |
| `git status` | `git status` (unchanged) |

Quoted and bare assignment values are both covered; the `Bear​er` scheme match is case-insensitive; URL userinfo with or without a password is collapsed to a single `[REDACTED]` marker.

## Non-Goals

- Not a credential detector for arbitrary formats (PEM blocks, raw hex keys, JSON string values without a key name) — those must never reach display strings in the first place.
- Not reversible and not logged: redaction happens before the string enters spans, status files, or error renders, so the secret never persists.
- Regexes are intentionally broad (`password|passwd|secret|...`) to catch variant spellings; false positives (e.g. `secret = "none"`) err toward hiding, which is the safe direction for logs.

## Where Applied

| Caller | Path |
|--------|------|
| `auto_sync::status` | Re-exports `redact_secrets`; redacts status-file messages and diagnostics |
| `logging::redact_command` | Redacts commands in tracing spans; fully replaced with `<redacted-command>` when anything matched, truncated with `...` when overlong |
| `error.rs` (`SnipError` Display) | Redacts commands/args at the render layer since callers print or log the rendered error |

## Backup Redaction (Separate Mechanism)

`commands::backup_archive::redact_sync_config` is a different, line-oriented function: any line whose key contains `api_key`/`apikey`/`api-key` (case-insensitive) has its value replaced with `"<redacted>"`. It preserves all other keys and is what keeps backed-up `sync.toml` copies credential-free. Restore therefore yields a redacted key the user must re-register.

## Invariants

- **Never log credentials.** Secrets are redacted at the lowest shared layer so no log, error render, or status message can bypass it by accident.
- Best-effort, not a boundary: unknown secret formats pass through. Callers must still avoid interpolating raw secrets into messages.
- One definition (`utils::redact`); `status::redact_secrets` is a re-export, not a fork — grep must show a single regex set.
- Backup copies are redacted structurally (`redact_sync_config`), not via the display regexes.

## Tests

In-module tests: secret assignment, bearer token, URL credentials, plain-text passthrough. `logging.rs` adds truncation/preservation/credential-removal tests for `redact_command`.

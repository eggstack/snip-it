# redact.rs — Secret Redaction

[← Back to Overview](../overview.md)

## Overview

`src/utils/redact.rs` holds the **single canonical definition** of
best-effort secret redaction for human-readable strings: API-key assignments,
bearer tokens, and URLs with embedded credentials.

**File**: `src/utils/redact.rs` (72 lines — three regexes, one function, and
the unit-test tail that starts at `src/utils/redact.rs:40`).

**Not covered here:** the *storage* of credentials (the keychain and the
two test-only credential seams — see `.skills/keychain-integration.md`), the
encryption of synced payloads (`.skills/encryption-module.md`), and the
**separate**, line-oriented backup redactor in
`src/commands/backup_archive.rs:358` (documented under
[Backups](#backup-redaction-is-a-different-mechanism) below).

**This is defense-in-depth, not a security boundary.** The module's own doc
comment says so (`src/utils/redact.rs:6-9`): the real guarantee is *never
placing secrets into these strings in the first place*. Redaction is the
second line of defence for the paths where a secret could plausibly have
leaked in — a command line, a log record, an error message.

## What Is Redacted

Three patterns, each a `LazyLock<regex::Regex>` compiled once on first use.

### 1. Secret assignments

```rust
// src/utils/redact.rs:13-18
static SECRET_ASSIGNMENT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)((?:api[_-]?key|token|password|passwd|secret|credential|authorization)\s*=\s*)(?:"[^"]*"|'[^']*'|[^\s&;]+)"#,
    )
    .expect("secret assignment pattern is valid")
});
```

Matches a **key name**, an `=`, and the value that follows:

| Key names matched | Case |
|-------------------|------|
| `api_key`, `api-key`, `apikey` | insensitive (`(?i)`) |
| `token` | insensitive |
| `password`, `passwd` | insensitive |
| `secret` | insensitive |
| `credential` | insensitive |
| `authorization` | insensitive |

The value may be a double-quoted string, a single-quoted string, or an
unquoted run of non-whitespace characters (stopping at `&` or `;` so that a
query-string or shell separator does not swallow the rest of the line).

**Replacement**: `$1[REDACTED]` — the matched *key* and the `=` are kept, the
value is replaced.

```
api_key = "hunter2"   →   api_key = [REDACTED]
```

### 2. Bearer tokens

```rust
// src/utils/redact.rs:19-21
static BEARER_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)\b(Bearer)\s+[^\s"']+"#).expect("bearer pattern is valid")
});
```

Matches the literal word `Bearer` (case-insensitive, at a word boundary)
followed by whitespace and a non-empty run of characters that are not
whitespace, `"`, or `'`.

**Replacement**: `$1 [REDACTED]` — note the **space** between the scheme and
the marker.

```
Authorization: Bearer hunter2   →   Authorization: Bearer [REDACTED]
```

### 3. URL credentials

```rust
// src/utils/redact.rs:22-25
static URL_CREDENTIALS_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)(https?://)[^/\s:@]+(?::[^/\s@]*)?@"#)
        .expect("URL credential pattern is valid")
});
```

Matches an `http://` or `https://` scheme followed by an optional
`user:password@` pair.

**Replacement**: `$1[REDACTED]@` — the scheme is kept, the whole
`user:password` pair is replaced, and the `@` is kept so the host that
follows is still readable.

```
https://user:s3cret@example.com/x   →   https://[REDACTED]@example.com/x
```

## The Function

```rust
// src/utils/redact.rs:32-38
pub(crate) fn redact_secrets(msg: &str) -> String {
    let result = SECRET_ASSIGNMENT_RE.replace_all(msg, "$1[REDACTED]");
    let result = BEARER_RE.replace_all(&result, "$1 [REDACTED]");
    URL_CREDENTIALS_RE
        .replace_all(&result, "$1[REDACTED]@")
        .into_owned()
}
```

The three passes run **in sequence**, each on the output of the previous one.
The order matters: the assignment pass runs first so that
`api_key = "Bearer abc"` is caught by the assignment rule before the bearer
rule can produce a partial match.

`replace_all` is used (not `replace`), so **every** occurrence in the string
is redacted, not just the first.

The function is `pub(crate)` — it is not part of the crate's public API. It
is **not** re-exported from `src/utils/mod.rs`; callers import it either as
`crate::utils::redact::redact_secrets` or through the
`crate::auto_sync::status` re-export described below.

### Plain text is untouched

```
redact_secrets("git status") == "git status"
```

(`src/utils/redact.rs:68-71`.) The patterns are specific enough that
ordinary command output is not mangled.

## Consumers

### auto-sync status — the canonical re-export

```rust
// src/auto_sync/status.rs:26-27
// Single canonical definition lives in `crate::utils::redact`; re-exported
// here so existing `status::redact_secrets` paths keep working.
pub(crate) use crate::utils::redact::redact_secrets;
```

`status::redact_secrets` is a **re-export, not a fork**. A grep for
`redact_secrets` must show exactly one regex set.

It is used by `sanitize_message` (`src/auto_sync/status.rs:291-297`), which
redacts and then strips control characters and truncates to
`MAX_MESSAGE_LEN = 512` (`src/auto_sync/status.rs:20`) before the message is
written to the durable auto-sync status file.

### Logging

```rust
// src/logging.rs:295-316
fn redact_command(command: &str) -> String {
    if command.len() > 256 {
        return "<very-long-command>".to_string();
    }

    let redacted = crate::auto_sync::status::redact_secrets(command);
    if redacted != command {
        return "<redacted-command>".to_string();
    }
    ...
}
```

The logging path uses a **different, stricter policy** than the plain
redactor: if redaction changed *anything*, the **entire command** is replaced
with the marker `<redacted-command>` rather than showing a partially redacted
command. A command longer than 256 bytes is replaced wholesale with
`<very-long-command>`; one longer than 80 bytes is truncated at a character
boundary to 77 characters plus `...`.

### Error display

`SnipError::Command`'s `Display` implementation redacts at the **Display
layer** (`src/error.rs:190`, `:194`), so every consumer of the rendered error
is covered — not just the logging path:

```rust
// src/error.rs:186-206 (shape)
SnipError::Command { command, args, source } => {
    let command = crate::utils::redact::redact_secrets(command);
    let args_display: Vec<String> = args.iter().map(|a| {
        let redacted = crate::utils::redact::redact_secrets(a);
        if redacted.chars().count() > 40 {
            let truncated: String = redacted.chars().take(37).collect();
            format!("{truncated}...")
        } else { redacted }
    }).collect();
    ...
}
```

Each argument is redacted and then truncated to 37 characters plus `...` if
it exceeds 40.

### Consumer summary

| Consumer | Site | Policy |
|----------|------|--------|
| `auto_sync::status` | `src/auto_sync/status.rs:27` (re-export), `:292` | Redact, strip control chars, truncate to 512 |
| `logging::redact_command` | `src/logging.rs:300` | Redact; on any change replace the **whole** command with `<redacted-command>` |
| `error::SnipError::Command` | `src/error.rs:190`, `:194` | Redact the command and each arg; truncate args to 40 chars |

## Backup Redaction Is a Different Mechanism

Backups do **not** use `redact_secrets`. They use a separate, line-oriented
function in the backup module:

```rust
// src/commands/backup_archive.rs:358-375
pub(crate) fn redact_sync_config(content: &str) -> SnipResult<String>
```

It scans `sync.toml` **line by line**, and for any line whose left-hand side
mentions `api_key` / `apikey` / `api-key` (case-insensitive) it replaces the
value with `"<redacted>"` while preserving the key and the line's
indentation. Every other line is copied through unchanged.

```
api_key = "sk-secret-key-12345"   →   api_key = "<redacted>"
server_url = "https://sync.example.com"   →   unchanged
```

It is called from `src/commands/backup_cmd.rs:208` when the backup archive is
built.

The two mechanisms are deliberately different:

| | `utils::redact::redact_secrets` | `backup_archive::redact_sync_config` |
|---|---|---|
| Scope | any message string | the `sync.toml` file content only |
| Granularity | regex over the whole string | per line, key-aware |
| Marker | `[REDACTED]` | `<redacted>` |
| Output | `String` | `SnipResult<String>` |

The backup path must produce a **still-valid TOML file**, so it works at the
key/value level. The message path only has to make a string safe to read.

## Key Files

| File | Role |
|------|------|
| `src/utils/redact.rs` | The three regexes and `redact_secrets` |
| `src/auto_sync/status.rs:26-27` | Re-export of the canonical function; `sanitize_message` at `:291-297` |
| `src/logging.rs:295-316` | `redact_command` — whole-command replacement policy |
| `src/error.rs:186-206` | `SnipError::Command` display-time redaction |
| `src/commands/backup_archive.rs:358-375` | The separate backup redactor |
| `src/commands/backup_cmd.rs:208` | Where the backup redactor is applied |

## Invariants

- **One definition.** `redact_secrets` exists in exactly one place;
  `status::redact_secrets` is a re-export. A second regex set is a drift
  bug.
- **Credentials are never logged.** Every path that persists a
  human-readable string — the log file, the auto-sync status file, the
  rendered error — passes it through the redactor first.
- **Redaction is applied at the boundary, not at the source.** The
  auto-sync status file, the log record, and the `Display` output are the
  boundaries; the redactor is not sprinkled through the call graph.
- **`replace_all`, never `replace`.** Every occurrence in a string is
  redacted.
- **Plain text passes through unchanged.** The patterns are specific enough
  that ordinary command output is not mangled.

## Gotchas

- **Redaction is best-effort, not a guarantee.** A secret that does not
  match one of the three patterns passes through untouched. The module
  explicitly frames itself as defense-in-depth (`src/utils/redact.rs:6-9`).
- **The marker differs by mechanism.** `[REDACTED]` in `redact_secrets`,
  `<redacted>` in `redact_sync_config`, `<redacted-command>` and
  `<very-long-command>` in the logging path. Tests that assert on the exact
  marker must match the right one.
- **The URL pattern requires a scheme.** A bare `user:pass@host` without
  `http://` or `https://` is not matched.
- **The bearer pattern requires the literal word `Bearer`.** A bare token
  with no scheme is only caught by the assignment rule.
- **The assignment pattern stops at `&` and `;`.** A secret that is the
  last field of a query string is redacted; one that appears after a
  `&`/`;` separator in an *unquoted* value may not be.
- **Redaction happens after truncation in some paths and before it in
  others.** `sanitize_message` redacts first, then truncates; the error
  path redacts each argument, then truncates it. Both orders are safe here
  because truncation cannot create a new secret, but do not assume a
  uniform order.
- **`redact_secrets` is `pub(crate)` and not re-exported at
  `src/utils/mod.rs`.** New callers must import the module path explicitly.

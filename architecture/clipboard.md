# Clipboard

[← Back to Overview](overview.md)

## Overview

Cross-platform clipboard access (`src/clipboard.rs`, 260 lines). Plain-text
only. Every clipboard side effect in the program funnels through one
function — `copy_to_clipboard()` in `src/commands/clip_cmd.rs` — which adds
audit logging and usage recording on top of the OS write.

## Platform backends

| Platform | Crate | Write path |
|----------|-------|------------|
| Windows | `clipboard-win` | `set_clipboard(Unicode, text)` (`src/clipboard.rs:93`, `:176`) |
| macOS / Linux | `arboard` | `Clipboard::new()` + `set_text` (`:99`, `:196`) |

Only plain text is supported: neither backend exposes rich-text/HTML formats
through its used API, so content types cannot be preserved (`:171`, `:190`).

## Sole side-effect path

`clip_cmd::copy_to_clipboard(snippet, final_command)` (`src/commands/clip_cmd.rs:40`):

1. `clipboard::copy_to_clipboard_auto(final_command)` (`:41`) — OS write plus configured auto-clear.
2. `logging::audit_log("copy", snippet, None)` (`:42`).
3. `UsageIndex::record_use(id)` + best-effort save (`:43`).

Variable expansion happens *before* this call — `process_snippet` (`:51`),
`run_exact` (`:68`), and `snp run --copy` all expand first and pass the final
string in. Never call `clipboard::copy_to_clipboard` directly from command or
TUI code; the audit/usage steps would be skipped.

## Timeout guard

`with_clipboard_timeout(op, f)` (`src/clipboard.rs:112`): runs the OS call on
a worker thread and `recv_timeout`s after `SNP_CLIPBOARD_TIMEOUT` seconds
(default 5, `DEFAULT_CLIPBOARD_TIMEOUT_SECS`, `:26`). On timeout the worker
is detached (fire-and-forget by design, `:119` — at most one outstanding
thread per user-invoked operation) and a `SnipError::clipboard_error` is
returned with a warn-level trace (`:130`). Channel disconnect maps to a
separate error (`:144`). `clear_clipboard()` (`:151`) uses the same guard.

## Auto-clear

`schedule_clipboard_clear(seconds)` (`:75`): no-op on 0; otherwise bumps
`CLIPBOARD_GENERATION` (`:24`) and parks a detached thread that clears only
if its generation still matches (`:84`) — a new copy cancels pending clears.
If the process exits first the clear is skipped (best-effort). Durations come
from `clipboard_auto_clear_seconds` in sync settings, cached in
`CACHED_CLIPBOARD_SETTINGS` (`:32`); `invalidate_clipboard_settings_cache`
(`:55`) resets it after settings change. `copy_to_clipboard_auto` (`:166`)
and `copy_to_clipboard_with_auto_clear` (`:155`) compose copy + schedule.

## Callers

- `clip_cmd::run` (`:99`) — TUI selection → `process_snippet` → sole path.
- `clip_cmd::run_exact` (`:68`) — `--id` / `--description-exact` / `--command-exact` bypass the TUI; optional post-clip explicit sync (`:89`).
- `run_cmd.rs` — copies before execution when the copy flag is set.
- `cron_cmd.rs` — optional copy of the generated crontab entry.
- `ui/mod.rs` — `y` key and visual-mode batch copy return `Copied` (see [tui.md](tui.md)).

## Logging

`log_clipboard_operation(op, success)` (`src/logging.rs:337`) records
debug on success, warn on failure — operation names only, never content.

## Headless / test behavior

Unit tests (`src/clipboard.rs:214`) cover empty, normal, unicode, multiline,
special-char, and 100 K-char payloads but are all `#[ignore]` — they need a
display server and do not run on headless CI. `clip_cmd` tests likewise ignore
live-clipboard cases (`clip_cmd.rs:124`); no-clipboard paths (e.g. sync
requires runtime, `:150`) run normally.

## Settings cache

`CACHED_CLIPBOARD_SETTINGS` (`src/clipboard.rs:32`) memoizes
`clipboard_auto_clear_seconds` from sync settings so every copy avoids a
settings read. `get_clipboard_auto_clear_seconds` (`:35`) fills it once;
`invalidate_clipboard_settings_cache` (`:55`) clears it after settings
mutations. Poisoned-mutex guards (`unwrap_or_else(|e| e.into_inner())`)
keep a panicked holder from wedging future copies.

## Error model

All failures are `SnipError::clipboard_error(op, detail)`:

| Case | Mapping |
|------|---------|
| OS context creation fails | `"create clipboard context"` (`:201`) |
| `set_text` fails | `"set text"` (`:206`), logged unsuccessful |
| Clear fails | `"clear clipboard"` (`:104`) |
| Exceeds `SNP_CLIPBOARD_TIMEOUT` | `"clipboard operation timed out after N seconds"` (`:136`) |
| Worker channel drops | `"clipboard operation channel closed unexpectedly"` (`:144`) |

Timeouts also emit a `tracing::warn` with the operation name and seconds
(`:130`); auto-clear failures warn instead of failing the copy (`:86`).

## Tests

| Test | Location | Payload |
|------|----------|---------|
| empty string | `clipboard.rs:221` | `""` |
| normal text | `:227` | `"test content"` |
| unicode | `:233` | `"héllo wörld 🎉"` |
| multiline | `:241` | three `\n`-joined lines |
| special chars | `:248` | pipes and quotes |
| 100 K-char content | `:255` | `"x".repeat(100000)` |

All are `#[ignore]` (display server required); `clip_cmd` sync-guard tests
(`clip_cmd.rs:150`) run without a clipboard.

## Invariants

- One funnel: `clip_cmd::copy_to_clipboard` is the only sanctioned side-effect path.
- Content is never logged; only op names and success flags.
- Generation counter prevents a stale auto-clear from wiping a newer copy.
- Timeouts bound hung OS calls; detached workers never accumulate beyond the user action rate.

## Key files

- `src/clipboard.rs` — backends, timeout, auto-clear, settings cache
- `src/commands/clip_cmd.rs:40` — `copy_to_clipboard()` sole side-effect path
- `src/logging.rs:337` — `log_clipboard_operation`

# Logging

[← Back to Overview](overview.md)

## Overview

Structured logging on the `tracing` crate (`src/logging.rs`, 703 lines):
daily-rolling diagnostic files under `logs/`, a synchronous append-only
`audit.log`, a terminal-restoring panic hook, and command-sensitive startup
levels so read-only commands never touch the logging filesystem.

## Startup service levels (`src/main.rs:886`)

`command_behavior()` assigns recovery policy and logging level in one match
so the two cannot drift:

| Level | Commands | Behavior |
|-------|----------|----------|
| `Minimal` (`main.rs:874`) | Read-only: `version`, `list`, `select`, `status`, `mcp`, `get`, `validate`, `backup`, `library list/show`; dry runs (`restore --dry-run`, `repair --dry-run`, `import pet --dry-run`); `data` read-only subgroup | No logging init, no `log_startup_info`, no startup recovery (`:1005`, `:1013`) |
| `Logging` (`:875`) | Mutations (`new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, mutating `library`); explicit sync (`sync`, `cron`, `register`); worker; config/setup (`update`, `doctor`, `completions`, `shell`, `keybindings`); default no-subcommand TUI | `init_default_file_logging()` (`:1007`) + `log_startup_info()` (`:1010`) |

Note the task's shorthand ("minimal for read-only/version/completions/shell/
keybindings vs full for mutations") maps to `Minimal` vs `Logging` — the
second group is file logging plus audit appends, not a separate audit tier.

## Diagnostic files (`logs/`)

- `LogConfig` (`logging.rs:50`): `log_dir` (`get_default_log_dir`, `:72` → `<config>/logs`), `file_name = "snp.log"`, `level = INFO`, `include_target`, plus audit rotation tunables.
- `init_logging` (`:86`): idempotent via `LOG_GUARD` (`:45` — dropping it would stop writes); `create_dir_all` + Unix `0o700` on the dir (`:96`); `tracing_appender::rolling::daily` (`:107`); non-blocking writer (`:109`); no ANSI, thread IDs, file + line numbers (`:121`).
- Filter (`:111`): `SNP_LOG` wins (invalid value warns and falls back); else `RUST_LOG` via `try_from_default_env`; else `snp=info`.
- `init_default_file_logging` (`:153`): `ensure_config_dir` first, then default config, then `self_check` (`:168`) which probes writability with a `.self_check` sentinel and ensures the config dir. `shutdown_logging` (`:210`) flushes by dropping the guard.

## Structured event functions

| Function | Location | Notes |
|----------|----------|-------|
| `log_command_execution` | `:269` | `#[instrument]` with `redact_command()` as the `command` field; `args`/`working_dir` as fields, result as `Ok`/`Err(String)` — counts only, never full text |
| `redact_command` | `:295` | `>256` chars → `<very-long-command>`; any secret pattern (via `auto_sync::status::redact_secrets`) → `<redacted-command>`; `>80` chars truncated at a char boundary + `...` |
| `log_config_operation` | `:317` | debug on success, warn with path + error on failure |
| `log_clipboard_operation` | `:337` | op name + success flag only, never content |
| `log_startup_info` | `:345` | debug: version, OS, arch, config/log dirs |
| `log_shutdown_info` | `:357` | info + flush |

## Audit log (`audit.log`)

- `audit_log(action, snippet, library_id)` (`:368`): builds `AuditLogEntry` (`:34` — timestamp, action, snippet id, library, device) and calls `write_audit_log_entry_sync` (`:394`). Synchronous by design: human command rate needs no background thread or channel.
- **No credentials, no user text**: `description` is always the literal `"[omitted]"` (`:386`) — descriptions are free text that may embed secrets; only ids, action, and device are persisted. Pinned by the sentinel-secret test (`:599`).
- Format is pipe-delimited `timestamp|action|snippet_id|description|library_id|device_id` (`:409`) with `escape_pipe` (`:448`: `\\`, `\|`, `\n`, `\r`, `\xNN` for other controls).
- File handling: `create + append` with Unix `0o600` at open and after write (`:420`, `:435`); open/write failures log and propagate — callers treat audit as non-critical (clip logs debug, delete logs debug).
- Rotation (`:465`, `AUDIT_LOG_MAX_SIZE_BYTES = 10 MiB`, `AUDIT_LOG_RETENTION_DAYS = 30`, `:30`): over-size files are renamed to `audit.<timestamp>.rotated`; rotated files older than retention are pruned. Missing file (`symlink_metadata` error) propagates so the caller can warn.

## Panic handler

`setup_panic_handler` (`:245`): takes the previous hook, then on panic —
best-effort `DisableMouseCapture` + `ratatui::restore()` inside
`catch_unwind` (`:248`), `log_panic_info` (`:234`, location + message at
`target: "panic"`), `shutdown_logging()` to flush (`:259`), `eprintln!` the
location/message (`:263`), and finally the previous hook. Installed first in
`main()` (`main.rs:1000`), before signal handlers and arg parsing.
`extract_panic_info` (`:217`) downcasts `&str`/`String` payloads, else
`"Unknown panic"`.

## Filter precedence

1. `SNP_LOG` set and parseable → used as-is (`logging.rs:111`).
2. `SNP_LOG` set but invalid → stderr warning + `snp=info` fallback (`:112`).
3. `SNP_LOG` unset → `RUST_LOG` via `try_from_default_env` (`:117`).
4. Neither set → `snp=info` (`:118`, `Level::INFO` default in `LogConfig`, `:64`).

Only `snp`-targeted records are enabled by default; the `panic` target used
by `log_panic_info` (`:237`) is inside the crate namespace.

## Audit rotation in detail

`rotate_audit_log_if_needed` (`:465`):

1. `symlink_metadata` (never follows symlinks) reads the size; missing file propagates so the write path can warn.
2. Size over 10 MiB → rename to `audit.<unix_secs>.rotated` (`:478`).
3. Scan the parent dir for `*.rotated`; entries older than 30 days (by mtime) are removed (`:482`).
4. Rotation failure is swallowed (`let _ =`, `:403`) — a full or read-only disk must not break the user command.

## Redaction examples (`redact_command`, `:295`)

| Input shape | Output |
|-------------|--------|
| `git status` (≤80 chars, clean) | unchanged |
| 100-char plain string | first ~77 chars + `...` (char-boundary safe, `:306`) |
| 300-char string | `<very-long-command>` (`:296`) |
| `curl -H "Authorization: Bearer …"` / embedded `password=` / `token=` | `<redacted-command>` (`:301`) |

`log_command_execution` (`:269`) additionally `skip`s the `result` payload
from the span fields, recording only counts and the redacted command name.

## Tests (selected)

- `escape_pipe` variants (`:511`): backslash, pipe, newline, CR, combined, empty.
- Rotation creates exactly one `.rotated` file over-limit (`:541`) and keeps under-limit files (`:556`).
- `0700` log dir / `0600` audit file on Unix (`:565`, `:582`).
- Sentinel secret in a description never reaches the audit file; `[omitted]` does; no ANSI escapes per line (`:599`).
- Embedded credentials → `<redacted-command>`; long → truncated; short → preserved (`:662`).
- Log dir stays under the config dir and is consistent across calls (`:683`, `:698`).

## Invariants

- `Minimal` commands perform zero logging-filesystem setup.
- No credentials or snippet content ever reach logs or audit (redaction + `[omitted]` + op-name-only clipboard events).
- Audit appends are synchronous; diagnostic writes are non-blocking behind a process-lifetime guard.
- Terminal state is restored on panic, signal exit (see [tui.md](tui.md)), and normal Drop paths.

## Key files

- `src/logging.rs` — init, levels, structured events, audit, panic hook
- `src/main.rs:873` (`StartupServices`), `:886` (`command_behavior`), `:999` (startup sequence)
- `src/clipboard.rs:22` (`log_clipboard_operation` call sites), `src/commands/clip_cmd.rs:42` (audit call site)

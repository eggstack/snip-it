# Logging

[← Back to Overview](overview.md)

## Overview

Structured logging on the `tracing` crate (`src/logging.rs`, 702 lines): a
daily-rolling diagnostic file under `<config>/logs/`, a synchronous
append-only `audit.log` in the config directory, a terminal-restoring panic
hook, and command-sensitive startup levels so read-only commands never touch
the logging filesystem.

This doc covers the `src/logging.rs` module and the startup wiring in
`src/main.rs`. It does **not** cover the per-command log call sites
themselves, the redaction primitives (see [utils/redact.md](utils/redact.md)),
or the auto-sync worker's own event sink (see
[auto_sync.md](auto_sync.md)).

Two artifacts live under the config directory and are easy to confuse:

| Artifact | Path | Writer | Purpose |
|----------|------|--------|---------|
| Diagnostic log | `<config>/logs/snp.log.YYYY-MM-DD` | `tracing` subscriber, non-blocking | Free-form operational events |
| Audit trail | `<config>/audit.log` | `audit_log()`, synchronous append | Structured record of snippet mutations |

## Startup service levels (`src/main.rs:924`)

`command_behavior()` assigns the startup recovery policy **and** the
logging/audit service level in one match, so the two cannot drift
(`src/main.rs:919-922`). `StartupServices` has exactly two variants
(`src/main.rs:910-914`).

| Level | Commands | Startup behaviour |
|-------|----------|-------------------|
| `Minimal` (`src/main.rs:912`) | Read-only: `version`, `list`, `select`, `status`, `mcp`, `get`, `validate`, `backup`, `library list/show`; dry runs (`restore --dry-run`, `repair --dry-run`, `import pet --dry-run`); the read-only `data` subgroup (`data validate\|status\|backup`, `data restore --dry-run`, `data repair --dry-run`) | No `init_default_file_logging()` (`src/main.rs:1043-1044`), no `log_startup_info()` (`:1010-1012`) |
| `Logging` (`src/main.rs:913`) | Mutations (`new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, mutating `library`); explicit sync (`sync`, `cron`, `register`); the internal worker (`auto-sync-worker`); config/setup (`update`, `self-replace`, `doctor`, `completions`, `shell`, `keybindings`); the default no-subcommand TUI | `init_default_file_logging()` (`src/main.rs:1045`) + `log_startup_info()` (`:1011`), and `log_shutdown_info()` on every exit path (`:1022`, `:1029`, `:1036`) |

Because `init_default_file_logging()` never runs, `Minimal` commands never
create `<config>/logs/`, never create `snp.log.*`, and never create
`audit.log` as a result of logging setup. (Other subsystems may still touch
the config directory for their own reasons — see
[cli.md](cli.md) for the command-level contract.)
Note that the `Minimal` group's suppression of **startup recovery** is a
separate mechanism: it is the recovery *policy* (`StartupRecoveryPolicy::SuppressReadOnly`,
`src/main.rs:940`) that is checked at `src/main.rs:1051`, not the service
level.

`log_shutdown_info()` flushes the non-blocking writer by dropping the worker
guard, so a `Logging` command's diagnostic file is complete before the
process exits.

## Subscriber setup

### `init_logging(config)` (`src/logging.rs:105`)

1. **Idempotence guard** (`:106-111`): if `LOG_GUARD` already holds a guard,
   return `Ok(())` immediately. `LOG_GUARD` is a
   `LazyLock<Mutex<Option<WorkerGuard>>>` (`:45-46`); the guard must stay
   alive for the process lifetime, because dropping it stops the file
   writes.
2. **Directory** (`:115-124`): `create_dir_all(log_dir)`, then on Unix
   `set_permissions` to `0o700`. Failures return `SnipError::io_error`.
3. **Appender** (`:126`): `tracing_appender::rolling::daily(log_dir,
   &config.file_name)`.
4. **Non-blocking writer** (`:128`): `tracing_appender::non_blocking(...)`
   returns the writer plus a `WorkerGuard`, which is stored in `LOG_GUARD`
   at `:156`.
5. **Filter** (`:130-138`): see [Filter selection](#filter-selection).
6. **Layer** (`:140-146`): `fmt::layer()` with `.with_ansi(false)`,
   `.with_target(config.include_target)`, `.with_thread_ids(true)`,
   `.with_file(true)`, `.with_line_number(true)`.
7. **Install** (`:148-154`): `registry().with(env_filter).with(file_layer)`,
   then `try_init()`. A failure prints
   `"Warning: failed to initialize logging subscriber: {e}"` to stderr but
   **does not fail the command** — `init_logging` still returns `Ok(())` and
   still stores the guard.
8. **Banner** (`:158-159`): two `tracing::info!` records ("Logging
   initialized…", "Log level: …").

There is **no stderr subscriber**: the only output sink is the file. The
`eprintln!` diagnostics in this module are plain stderr, not `tracing`.

### Log directory and file names

`get_default_log_dir()` (`src/logging.rs:72-74`) is
`<config>/logs`, where `<config>` is `crate::utils::config::get_config_dir()`
(`src/utils/config.rs:63-82`) — `$XDG_CONFIG_HOME/snp` or
`~/.config/snp`.

`LogConfig::default()` (`:59-70`) sets `file_name = "snp.log"` (`:63`) and
`level = Level::INFO` (`:64`). Because the appender rotates **daily**, the
files on disk are named `snp.log.YYYY-MM-DD` — there is no plain `snp.log`
file. `tracing-appender`'s daily format is `[year]-[month]-[day]`
(`tracing-appender-0.2.5` `rolling.rs:564`) and the configured prefix is
used verbatim (`rolling.rs:148-154`), so the prefix `snp.log` yields
`snp.log.2026-10-21`.

Daily rotation creates a new file per UTC day but **never removes old
files** — the diagnostic logs grow without bound. Only the *audit* log has a
retention policy (see [Rotation and retention](#rotation-and-retention)).

`LogConfig` is `#[allow(dead_code)]` (`:49`) and no in-tree caller varies
its fields: `init_default_file_logging()` always uses `LogConfig::default()`
(`:180`). The struct is the intended seam for a future per-command level.

### Filter selection

```rust
// src/logging.rs:130-138
let env_filter = if let Ok(filter_str) = std::env::var("SNP_LOG") {
    EnvFilter::try_new(&filter_str).unwrap_or_else(|e| {
        eprintln!("Warning: Invalid SNP_LOG filter '{filter_str}': {e}");
        EnvFilter::new(default_filter_directive(config.level))
    })
} else {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter_directive(config.level)))
};
```

Precedence:

1. `SNP_LOG` set and parseable → used as-is (`:130`).
2. `SNP_LOG` set but invalid → stderr warning + the `snip_it=<level>` fallback
   (`:131-134`).
3. `SNP_LOG` unset → `RUST_LOG` via `EnvFilter::try_from_default_env()`
   (`:136`).
4. Neither set → the `snip_it=info` fallback (`:137`; `Level::INFO` default at
   `:64`).

`RUST_LOG` is read through `tracing-subscriber`'s own environment lookup,
i.e. the `RUST_LOG` variable.

### What the fallback filter actually matches

`"snip_it=info"` is a **target-prefix** directive: `EnvFilter` matches a
directive's target with `meta.target().starts_with(target)`
(`tracing-subscriber-0.3.23` `filter/directive.rs:246`, `:182`), and a
directive with no target matches every target.

The default target of a `tracing` event is `module_path!()`. Every
`tracing` call site in this workspace lives in the **library** crate
`snip_it` (the package is `snip-it` and `Cargo.toml` declares no `[lib]`
section, so the library target name is `snip_it`). The `snp` binary crate
(`src/main.rs`) contains no `tracing` calls at all. The resulting targets
are therefore `snip_it::logging`, `snip_it::clipboard`,
`snip_it::commands::clip_cmd`, and so on — and `"snip_it::logging"` does
**not** start with `"snp"`.

The only explicit target in the workspace is the panic event's literal
`target: "panic"` (`src/logging.rs:257`), which also does not start with
`snip_it`.

> **Fixed in this session.** The fallback used to be `snp=<level>`, which
> matched **no** event: with neither `SNP_LOG` nor `RUST_LOG` set, the daily
> log file was created but stayed **empty**. The fallback is now built by
> `default_filter_directive()` (`:101`) from the `LOG_TARGET_PREFIX` constant
> (`src/logging.rs:95`), which is `snip_it` — the library crate, not the binary
> crate. Verified by running `snp new` against a throwaway `XDG_CONFIG_HOME`:
> the log file went from 0 bytes to 696 bytes of records, while the
> `SNP_LOG=warn` and `RUST_LOG=info` overrides continue to behave as before.
>
> A filter with no target (`SNP_LOG=info`) still works and additionally
> covers dependency crates.

### File format

The layer is `tracing-subscriber`'s `fmt` layer with the default `Full`
format and the default `SystemTime` timer. The rendered order is
timestamp, level, thread id, span scope, target, file, line, then the event
fields — so one record looks like:

```
2026-10-21T04:43:12.123456Z INFO ThreadId(2) audit_log{action=copy snippet_id=sentinel-id}: snip_it::logging: src/logging.rs:387: message
```

- **Timestamp**: `YYYY-MM-DDTHH:MM:SS.ffffffZ` (UTC), then a space.
- **Level**: `TRACE`/`DEBUG`/`INFO`/`WARN`/`ERROR`.
- **Thread id**: enabled (`:144`), rendered as the `Debug` form of
  `std::thread::ThreadId` — e.g. `ThreadId(2)`.
- **Span scope**: the enclosing spans are printed as
  `name{field=value …}:` — so the `#[tracing::instrument]` span fields
  (`action`, `snippet_id` for `audit_log` at `:387`; `command` for
  `log_command_execution` at `:288`) **do** reach the file.
- **Target**: the module path, because `include_target` defaults to `true`
  (`src/logging.rs:65`, `:143`).
- **File and line**: enabled (`:145-146`).
- **No ANSI**: disabled explicitly (`:142`), so the file is plain text.
- **Event fields**: the event's own fields, including the message.

### `init_default_file_logging()` (`src/logging.rs:172`)

The entry point used by `main` (`src/main.rs:1045`):

1. `crate::utils::config::ensure_config_dir()` (`:177`) — creates the config
   directory and tightens it to `0o700` on Unix. Failures are a stderr
   warning, not an error.
2. `LogConfig::default()` (`:180`) and `init_logging(&config)` (`:181`) —
   failures are a stderr warning, not an error.
3. `self_check()` (`:184`, defined `:187-227`) — re-creates the log directory
   if missing, then opens and deletes a `.self_check` sentinel file to prove
   the directory is writable, and finally ensures the config directory
   exists.

`init_default_logging()` (`src/logging.rs:164-166`) is a public alias for
`init_default_file_logging()`. It has no in-tree caller; it exists for the
documented public API surface (`docs/PUBLIC_API.md:326`).

`shutdown_logging()` (`:229-234`) takes the guard out of `LOG_GUARD` and
drops it, which flushes the non-blocking writer.

## Structured event functions

| Function | Location | Notes |
|----------|----------|-------|
| `log_command_execution` | `src/logging.rs:289` | `#[instrument(level = "info", skip(result), fields(command = %redact_command(command)))]`; `result` is skipped from the span, then matched — `Ok` logs info with `args_count` + `working_dir`, `Err` logs error with `args_count`, `working_dir`, and the error text |
| `redact_command` | `src/logging.rs:314` | `>256` chars → `<very-long-command>`; any secret pattern (via `auto_sync::status::redact_secrets`) → `<redacted-command>`; `>80` chars truncated at a char boundary + `...` |
| `log_config_operation` | `src/logging.rs:336` | debug on success, warn with path + error on failure |
| `log_clipboard_operation` | `src/logging.rs:356` | operation name + success flag only, never content |
| `log_startup_info` | `src/logging.rs:364` | debug: version, OS, arch, config dir, log dir |
| `log_shutdown_info` | `src/logging.rs:376` | info + `shutdown_logging()` |
| `log_panic_info` | `src/logging.rs:253` | error with `target: "panic"`; see [Panic handler](#panic-handler) |

## Audit log (`audit.log`)

### What it records

`audit_log(action, snippet, library_id)` (`src/logging.rs:387-411`):

```rust
// src/logging.rs:387-411
#[tracing::instrument(level = "info", skip(snippet), fields(action = %action, snippet_id = %snippet.id))]
pub fn audit_log(
    action: &str,
    snippet: &crate::library::Snippet,
    library_id: Option<&str>,
) -> std::io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = AuditLogEntry {
        timestamp,
        action: action.to_string(),
        snippet_id: snippet.id.clone(),
        // Descriptions are user-authored free text and may contain secrets;
        // keep the audit record useful without copying that text out of the
        // encrypted snippet store.
        description: "[omitted]".to_string(),
        library_id: library_id.unwrap_or("").to_string(),
        device_id: snippet.device_id.clone(),
    };

    write_audit_log_entry_sync(&entry)
}
```

`AuditLogEntry` is the six-field struct at `src/logging.rs:34-41`.

### What is deliberately NOT logged

- **Descriptions are never persisted.** `description` is hard-coded to the
  literal `"[omitted]"` (`:405`) — descriptions are user-authored free text
  that may embed secrets, and the snippet store may be encrypted. The
  comment at `:402-404` states the rationale. Pinned by the sentinel-secret
  test (`:617-658`).
- **Snippet commands are never logged.** No audit field carries the command
  or the output.
- **No credentials.** API keys, bearer tokens, and URLs with embedded
  credentials are stripped by `redact_command` before any command reaches a
  `tracing` event (`:314-334`).
- **Clipboard content is never logged** — only operation names and success
  flags (`:357-361`).

### Entry format

Pipe-delimited, one line per entry (`:428-436`):

```
{timestamp}|{action}|{snippet_id}|{description}|{library_id}|{device_id}
```

- `timestamp` is Unix **seconds** (not milliseconds).
- `snippet_id`, `description`, and `device_id` are escaped by `escape_pipe`
  (`:467-482`): `\` → `\\`, `|` → `\|`, newline → `\n`, CR → `\r`, and any
  other control character → `\xNN`. `action` and `library_id` are **not**
  escaped.

### File handling

- **Path**: `get_audit_log_path()` (`:381-385`) returns
  `<config>/audit.log`, creating the config directory as a side effect.
- **Open**: `create(true).append(true)` with Unix mode `0o600` (`:439-445`),
  and permissions are re-tightened to `0o600` after the write (`:454-458`).
- **Write**: `write_all` of the formatted line (`:460`).
- **Synchronous by design** (`:170-171`, `:413`): the audit trail has no
  background writer or channel — human command rate does not need one, and
  a synchronous append cannot lose the last record.
- **Error handling differs by caller.** `clip` propagates the error with
  `?` (`src/commands/clip_cmd.rs:42`), so a failed audit write fails the
  copy command; the TUI batch copy (`src/ui/mod.rs:1751-1758`) and the
  delete path (`src/commands/mod.rs:535-537`) log at debug level instead.

### Rotation and retention

`rotate_audit_log_if_needed(log_path, max_size_bytes, retention_days)`
(`src/logging.rs:484-523`), called before every append (`:422-426`):

1. `fs::symlink_metadata(log_path)` (`:489`) reads the size without
   following symlinks. If the file does not exist yet, this returns an
   error — and the caller **discards it** with `let _ =` (`:422`), so the
   first append of a fresh install simply skips rotation.
2. Size over `AUDIT_LOG_MAX_SIZE_BYTES` (10 MiB, `:30`) → rename to
   `audit.<unix_secs>.rotated` (`:493-498`).
3. Scan the parent directory for `*.rotated` and delete entries older than
   `AUDIT_LOG_RETENTION_DAYS` (30 days, `:31`) by mtime (`:501-520`).
4. The whole rotation step is best-effort — a full or read-only disk must
   not break the user's command (`:422`).

## Panic handler

`setup_panic_handler()` (`src/logging.rs:264-286`) takes the previous hook
and installs a replacement that, on panic:

1. Best-effort terminal restoration inside `catch_unwind`:
   `DisableMouseCapture` then `ratatui::restore()` (`:268-273`). The
   mouse-capture disable is explicit because some terminals leave the mouse
   in a bad state if `ratatui::restore()` runs alone.
2. `log_panic_info(panic_info)` (`:275`, defined `:253-262`) — an
   `tracing::error!` with `target: "panic"`, the location as
   `file:line:column`, and the panic message.
3. `shutdown_logging()` (`:278`) — flushes the non-blocking writer before
   the process dies.
4. `eprintln!("PANIC at {location}: {message}")` (`:282`).
5. `previous_hook(panic_info)` (`:284`).

`extract_panic_info` (`:236-251`) renders the location as
`file:line:column` (`:239`), and downcasts the payload to `&str` or
`String`, falling back to `"Unknown panic"` (`:242-248`).

The hook is installed first in `main()` (`src/main.rs:1038`), **before**
argument parsing and **before** `init_default_file_logging()`
(`src/main.rs:1045`). A panic that happens before logging is initialised
therefore still restores the terminal and prints to stderr, but has no log
file to write to. The same applies to the `target: "panic"` record: it is
only captured when the active filter covers the `panic` target — see
[What the fallback filter actually matches](#what-the-fallback-filter-actually-matches).

## Redaction

`redact_command(command)` (`src/logging.rs:314-334`) is the only redaction
applied to a `tracing` payload in this module:

| Input shape | Output |
|-------------|--------|
| `git status` (≤80 chars, clean) | unchanged |
| 100-char plain string | first ~77 chars + `...` (char-boundary safe, `:324-330`) |
| 300-char string | `<very-long-command>` (`:315-317`) |
| `curl -H "Authorization: Bearer …"` / embedded `password=` / `token=` | `<redacted-command>` (`:319-322`) |

The secret detection is `crate::auto_sync::status::redact_secrets`
(`src/auto_sync/status.rs:27`), which re-exports the canonical
`crate::utils::redact::redact_secrets` (`src/utils/redact.rs:32`) — see
[utils/redact.md](utils/redact.md). The redaction is best-effort pattern
matching, not a security boundary; the audit trail does not rely on it
(descriptions are `[omitted]` instead).

`log_command_execution` additionally `skip`s the `result` payload from the
span fields; the event body then records the arg count, the working
directory, the redacted command name, and — on failure — the error text.

## Test-module config isolation

`src/logging.rs` has its own `#[cfg(test)] mod tests` (`:525-702`). The
sentinel-secret test (`test_sentinel_secret_not_in_audit_log_entry`,
`:617-658`) must not write to the developer's real `~/.config/snp/`, because
`audit_log()` resolves its path from the config directory. It therefore
binds the test-only, **thread-local** `ScopedConfigDir` seam:

```rust
// src/logging.rs:623-626
let dir = tempfile::TempDir::new().unwrap();
let config_home = dir.path().join(".config");
std::fs::create_dir_all(&config_home).unwrap();
let _guard = crate::utils::config::ScopedConfigDir::new(config_home.join("snp"));
```

`ScopedConfigDir` (`src/utils/config.rs:35-57`) is compiled only under
`#[cfg(any(test, feature = "test-support"))]`. `get_config_dir()`
(`src/utils/config.rs:63-82`) checks the thread-local override first
(`:64-67`) before falling back to `XDG_CONFIG_HOME`/`~/.config`. A
thread-local override is used rather than an `XDG_CONFIG_HOME` mutation
because lib tests run in parallel threads inside one process: an
environment mutation would race with every other test that resolves a path.
The guard restores the previous value on drop, including on assertion
panic.

This replaced a hand-rolled `XdgGuard` that set `XDG_CONFIG_HOME` process-wide
in commit `8d5c209` ("Fix silent data loss, sync dead-ends, and lock hangs").
That commit's 38-line change to `src/logging.rs` was **entirely inside the
test module** — no production line of the logging module changed.

## Tests

| Test | Location | Asserts |
|------|----------|---------|
| `escape_pipe` variants | `src/logging.rs:529-557` | backslash, pipe, newline, CR, combined, empty |
| Rotation creates exactly one `.rotated` file over-limit | `:560` | rotation + file count |
| Under-limit file is not rotated | `:575` | file survives |
| Log dir permissions are `0700` on Unix | `:584` | mode check |
| Audit log permissions are `0600` on Unix | `:601` | mode check |
| Sentinel secret never reaches the audit file | `:618` | `[omitted]` present, secret absent, no ANSI escapes per line |
| `redact_command` truncates long commands | `:661` | length + `...` |
| `redact_command` preserves short commands | `:669` | unchanged |
| `redact_command` removes embedded credentials | `:676` | `<redacted-command>` |
| Log dir stays under the config dir | `:683` | `starts_with(config_dir)` |
| Log dir is consistent across calls | `:697` | two calls agree |

The two permission tests only assert on Unix; on Windows the `#[cfg(unix)]`
bodies are empty and the test passes vacuously.

## Key Files

| File | Subject |
|------|---------|
| `src/logging.rs` (702 lines) | Subscriber setup, filter selection, structured events, audit trail, panic hook |
| `src/main.rs:910-1035` | `StartupServices` + `command_behavior()` — the recovery/logging policy matrix |
| `src/main.rs:1037-1075` | Startup sequence: panic hook → arg parse → service level → recovery → dispatch → shutdown flush |
| `src/utils/config.rs:63-82`, `:91-116` | `get_config_dir()` / `ensure_config_dir()` — the config-root resolution the audit path uses |
| `src/utils/config.rs:35-57` | `ScopedConfigDir` — test-only thread-local config override |
| `src/utils/redact.rs:32` | `redact_secrets()` — the canonical secret stripper |
| `src/auto_sync/status.rs:27` | `status::redact_secrets` re-export used by `redact_command` |
| `src/commands/clip_cmd.rs:42` | Audit call site that propagates its error |
| `src/ui/mod.rs:1751` | Audit call site that logs at debug instead |
| `src/commands/mod.rs:535` | `delete` audit call site — also logs at debug |
| `Cargo.toml:117-119` | `tracing`, `tracing-subscriber` (fmt/registry/env-filter), `tracing-appender` |

## Invariants

1. **`Minimal` commands perform zero logging-filesystem setup** — no log
   directory, no `snp.log.*`, no `audit.log`.
2. **Audit appends are synchronous; diagnostic writes are non-blocking behind
   a process-lifetime guard** (`LOG_GUARD`, `src/logging.rs:45`).
3. **No credentials or snippet content ever reach the logs or the audit
   trail** — redaction plus the hard-coded `[omitted]` description plus
   operation-name-only clipboard events.
4. **The audit trail is append-only and pipe-delimited**, with escaped
   fields, `0600` on Unix, 10 MiB rotation, and 30-day retention of rotated
   files.
5. **Terminal state is restored on panic** (mouse capture disabled, then
   `ratatui::restore()`), and the log writer is flushed before the process
   dies.
6. **The recovery policy and the logging level are decided by one match**
   (`src/main.rs:924`), so they cannot drift apart.
7. **Unit tests never write to the real `~/.config/snp/`** — the audit
   sentinel test binds a per-test, thread-local config directory.

## Gotchas

- **The fallback filter must name the library crate, not the binary crate.**
  It is a *target-prefix* directive, and every event target starts with
  `snip_it::` (the library crate) while the `snp` binary crate emits no
  `tracing` events at all. A fallback of `snp=info` therefore matched
  **nothing** and left the daily log file empty — that is why the fallback
  is now built from the `LOG_TARGET_PREFIX` constant (`snip_it`). See
  [What the fallback filter actually matches](#what-the-fallback-filter-actually-matches).
  A user-supplied `SNP_LOG=info` (no target) or `SNP_LOG=snp_it=info` also
  works.
- **The panic record's target is the literal `"panic"`**, not a
  crate-namespaced target (`src/logging.rs:257`). It therefore matches no
  target-scoped directive that does not name `panic` — including the
  built-in `snip_it=<level>` fallback.
- **A rotation failure is silently swallowed.** `rotate_audit_log_if_needed`
  returns an error when `audit.log` does not exist yet, and the caller
  discards it with `let _ =` (`src/logging.rs:422`). The first append of a
  fresh install simply skips rotation — no warning is emitted.
- **`try_init()` failure does not fail the command.** The subscriber may
  fail to install (e.g. another global subscriber is already set); the
  warning goes to stderr and the command continues with no file logging.
- **`LogConfig` is inert.** It is `#[allow(dead_code)]` and every field is
  the default in practice (`src/logging.rs:49-70`). Changing
  `LogConfig::default().level` changes the *fallback filter string* only,
  not the filter actually in force when `SNP_LOG`/`RUST_LOG` is set.
- **Span fields *do* reach the file.** The `Full` format prints the enclosing
  span scope as `name{field=value …}:`, so `audit_log`'s `action` and
  `snippet_id` and `log_command_execution`'s redacted `command` appear in
  `snp.log.*` — even though `result` is `skip`ped from the span
  (`src/logging.rs:288`, `:387`).
- **The audit trail and the diagnostic log live in different directories.**
  `audit.log` is directly in `<config>/`; the `tracing` files are in
  `<config>/logs/`.
- **The diagnostic logs have no retention policy.** Daily rotation creates
  a new `snp.log.YYYY-MM-DD` per UTC day and never deletes the old ones —
  only the audit log prunes (30 days for `*.rotated`).
- **`get_audit_log_path()` creates the config directory** as a side effect
  (`:381-385`), without tightening its permissions the way
  `ensure_config_dir()` does.
- **The panic hook is installed before logging is initialised**
  (`src/main.rs:1038` vs `:1008`), so an early panic has no log file to
  write to — only the stderr line.
- **`init_default_logging()` has no in-tree caller.** It is a public alias
  for `init_default_file_logging()` kept for the documented API surface
  (`src/logging.rs:164`).

# cron_cmd — Crontab Generation for Periodic Sync

[← Back to Overview](../overview.md)

## Overview

`src/commands/cron_cmd.rs` (162 lines) prints a copy-pasteable crontab
entry that runs `snp sync` on an interval. It installs nothing: no
crontab is written, no Task Scheduler task is created, and no shell
profile is touched. The operator decides where the line goes.

It does **not** sync anything itself — the generated entry invokes the
foreground sync path (see [sync_cmd.md](sync_cmd.md)) when it fires.

Related: [sync_cmd.md](sync_cmd.md), [../auto_sync.md](../auto_sync.md)
(the in-process alternative to a cron job).

## CLI surface

```rust
// src/commands/cron_cmd.rs:5-9
#[derive(Debug, Clone, clap::Args)]
pub struct CronArgs {
    #[arg(short, long, default_value = "15")]
    pub interval: u32,
}
```

`snp cron` (alias `cr`) — `src/main.rs:140-142`, dispatched at
`src/main.rs:761-762`.

| Flag | Type | Default | Behavior |
|------|------|---------|----------|
| `-i`, `--interval` | `u32` (minutes) | `15` | Cron step for the generated entry. `0` is rejected |

There is no subcommand, no `--json` mode, and no `--install` flag.

## Flow / steps

`run(interval)` (`src/commands/cron_cmd.rs:53`):

1. **Build the entry** via `make_cron_entry(interval)` (`:37-50`):
   - `interval == 0` → `runtime_error("Invalid interval")` with
     `Interval must be at least 1 minute`;
   - otherwise `"*/{interval} * * * * {escaped_binary} sync"`.
2. **Print** `Crontab entry (every {interval} minutes):` and the entry.
3. **Platform instructions** (`:65-82`):
   - Unix: `crontab -e` → add the line → save and exit.
   - Windows: six Task Scheduler steps, with the **resolved** binary path
     (not the escaped crontab form) as the program and `sync` as the
     argument. `binary_path()` is only called under
     `#[cfg(target_os = "windows")]` (`:57-58`) so the binding is not dead
     on Unix.
4. **Clipboard prompt** (`:84-93`): `Copy to clipboard? [y/N]: `,
   flushed, then a single `stdin` line is read. The copy happens only
   when the trimmed, lowercased answer is exactly `y` — Enter, `N`, or
   anything else is a no-op. A clipboard failure prints
   `Failed to copy to clipboard: <err>` and the command still exits 0.

## The pure builder

`make_cron_entry` is the testable core. It performs **no blocking I/O** —
in particular no `stdin` read — so unit tests cover validation and
quoting without interaction (the doc comment at `:33-36` states this
explicitly).

```rust
// src/commands/cron_cmd.rs:37-50
pub fn make_cron_entry(interval: u32) -> SnipResult<String> {
    if interval == 0 {
        return Err(SnipError::runtime_error(
            "Invalid interval",
            Some("Interval must be at least 1 minute"),
        ));
    }

    Ok(format!(
        "*/{} * * * * {} sync",
        interval,
        shell_escape_path(&binary_path())
    ))
}
```

`binary_path()` (`:27-31`) prefers `std::env::current_exe()` and falls
back to the bare name `snp` when the path cannot be determined.

`shell_escape_path` (`:11-23`) quotes the path only when it contains a
space, `'`, `"`, `\`, `$`, or a backtick:

- an empty path becomes `''`;
- otherwise the path is wrapped in single quotes with any embedded
  single quote rewritten as `'\''`;
- a path with none of those characters is emitted verbatim.

## How the generated entry invokes the sync path

The generated line is `*/N * * * * <binary> sync`. That is the **foreground**
sync path — the same one `snp sync` runs — not the detached auto-sync
worker. Each firing therefore:

- takes the auto-sync **execution lock** with a bounded 30-second wait,
  and fails with `sync already in progress` (carrying the owner pid) if
  another sync holds it (`src/commands/sync_cmd.rs:188-210`);
- resolves the direction from `sync_direction` in `sync.toml`, since the
  crontab line carries no direction flags;
- observes the pending generation, records durable status, and clears the
  pending marker on success (`src/commands/sync_cmd.rs:291-341`).

Consequences worth knowing:

- **The cron job and the in-process auto-sync worker are two independent
  schedulers.** Both contend for the same execution lock, so a cron fire
  during an auto-sync run exits non-zero with `sync already in progress`
  rather than corrupting state.
- **Direction belongs in `sync.toml`, not in the crontab line.** Use
  `snp sync config` (there is no direction flag on the `sync` group other
  than `--push-only` / `--pull-only` on `snp sync run`).
- **A cron fire that fails does not schedule a retry** the way the
  auto-sync worker does; the next cron tick is the next attempt.

## The clipboard copy does not audit

The copy at `src/commands/cron_cmd.rs:89` calls
`crate::clipboard::copy_to_clipboard_auto(&cron_entry)` **directly**:

```rust
match crate::clipboard::copy_to_clipboard_auto(&cron_entry) {
    Ok(()) => println!("Copied to clipboard!"),
    Err(e) => eprintln!("Failed to copy to clipboard: {e}"),
}
```

This **bypasses `commands::clip_cmd::copy_to_clipboard`**
(`src/commands/clip_cmd.rs:40-49`), which is the single implementation for
all *snippet* copy operations and which additionally:

- writes an audit-log entry via `crate::logging::audit_log("copy", snippet, None)`, and
- updates the usage index via `UsageIndex::record_use(&snippet.id)`.

The crontab entry is not a snippet, so there is nothing to attribute an
audit entry to — but the practical consequence is that **this clipboard
write leaves no trace in the audit log and no usage-index entry**. The
same is true of the other direct `copy_to_clipboard_auto` call site in
the TUI's visual-range multi-select (`src/ui/mod.rs:1733`), which
additionally audit-logs only the *first* row of the range
(`src/ui/mod.rs:1748`). See [keybindings_cmd.md](keybindings_cmd.md).

`copy_to_clipboard_auto` also consults `clipboard_auto_clear_seconds` from
`sync.toml` and, when set, **schedules a clipboard wipe** after that
delay (`src/clipboard.rs:35-53`, `:155-169`, `:75-90`). Copying the
crontab entry can therefore clear the clipboard shortly afterwards.

## Mutation vs read-only

Read-only with respect to configuration: the command prints text and
optionally writes the system clipboard. It never touches `sync.toml`,
`libraries.toml`, library files, the cron table, or the Task Scheduler.

There is no transaction gate and no library lock. The only state change
possible is the clipboard (plus a possible scheduled clipboard clear).

## Tokio runtime

None. `snp cron` never touches the global `RUNTIME`
(`src/main.rs:29-35`) — it is entirely synchronous. `AGENTS.md` lists the
runtime-owning commands as `run`, `clip`, `search`, `sync`, `register`,
`premade`, and `update`; `cron` is not among them.

Startup policy: `Commands::Cron(_)` uses
`StartupRecoveryPolicy::SuppressExplicitSync` and
`StartupServices::Logging` (`src/main.rs:1007-1009`) — generating a cron
entry does not reschedule pending auto-sync work.

## Auto-sync trigger

None. The command never calls `notify_mutation`. The generated entry
schedules the *foreground* sync path, which observes and clears the
pending marker itself; it does not create a new pending intent.

## Error / exit mapping

`SnipResult<()>` — every `Err` becomes `error: <message>` on stderr and
exit **1** (`src/main.rs:1063-1069`).

| Situation | Result |
|-----------|--------|
| `--interval 0` | `runtime_error("Invalid interval")` → exit 1 |
| Clipboard copy fails | warning on stderr, exit **0** |
| Non-interactive stdin (piped) | `read_line` returns the piped bytes; anything but `y` is a no-op, exit 0 |
| EOF on stdin | `read_line` errors → the `is_ok()` guard fails → no copy, exit 0 |

Note that `--interval 0` is a *runtime* error (exit 1), not a clap usage
error (exit 2): clap parses `u32` successfully and the range check lives
in `make_cron_entry`.

## Example output

```
Crontab entry (every 15 minutes):
*/15 * * * * /usr/local/bin/snp sync

To add to your crontab:
  1. Run: crontab -e
  2. Add the line above
  3. Save and exit

Copy to clipboard? [y/N]:
```

With a spaced install path, the binary field renders quoted:
`*/30 * * * * '/opt/my tools/snp' sync`.

## Key Files

| Path | Role |
|------|------|
| `src/commands/cron_cmd.rs` | `CronArgs`, `shell_escape_path`, `binary_path`, `make_cron_entry`, `run` |
| `src/main.rs:140-142`, `:724-725` | Clap wiring and dispatch |
| `src/clipboard.rs:166-169` | `copy_to_clipboard_auto` (auto-clear aware) |
| `src/commands/clip_cmd.rs:40-49` | The auditing copy path that `cron` bypasses |
| `src/commands/sync_cmd.rs:188-210` | The execution lock the generated entry contends for |
| `src/logging.rs` | `audit_log` — not called on this path |

## Invariants

- **`make_cron_entry` performs no I/O**, so validation and quoting are
  unit-testable without interaction.
- **Binary resolution prefers the running executable over `PATH`**, so
  dev installs and renamed binaries still schedule correctly.
- **The prompt defaults to No** — piping or pressing Enter never copies.
- **The command never installs anything.** The operator owns the crontab.
- **The clipboard copy is not audited** and may schedule a clipboard wipe.

## Gotchas

- **The generated entry runs the foreground sync, not the auto-sync
  worker.** The two are separate schedulers sharing one execution lock.
- **`--interval 0` is a runtime error (exit 1), not a usage error
  (exit 2).**
- **A cron fire that collides with an in-process sync exits non-zero**
  with `sync already in progress`.
- **The clipboard copy leaves no audit trail** — there is no `audit_log`
  entry and no usage-index update for it.
- **Copying the entry can schedule a clipboard auto-clear** if
  `clipboard_auto_clear_seconds` is set in `sync.toml`.
- **`shell_escape_path` is not a complete shell quoter.** It quotes on
  space, `'`, `"`, `\`, `$`, and a backtick (`src/commands/cron_cmd.rs:15-17`);
  characters such as `;`, `&`, `|`, `>`, `<`, `(`, `)`, `*`, `?`, `!`,
  `#`, `~`, and newline are emitted verbatim. A binary path containing one
  of those lands unquoted in the crontab line and is interpreted by `sh`
  when the entry fires.

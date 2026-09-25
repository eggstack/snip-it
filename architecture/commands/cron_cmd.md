# cron_cmd — Crontab Generation for Periodic Sync

[← Back to Overview](../overview.md)

## Overview

`src/commands/cron_cmd.rs` (162 lines) prints a copy-pasteable crontab
entry that runs `snp sync` on an interval. The pure builder
(`make_cron_entry`) is unit-tested; the interactive wrapper adds
platform instructions and an optional clipboard copy.

## CLI surface

`CronArgs` (`cron_cmd.rs:6`), `snp cron` (alias `cr`): `-i/--interval
N` (default 15, minutes). No subcommands, no JSON mode.

## Flow / steps

1. `run(interval)` (`:53`) → `make_cron_entry(interval)` (`:37`):
   `0` → `runtime_error("Invalid interval")`; else `"*/N * * * *
   <binary> sync"` where `<binary>` is `current_exe()` or bare `snp`.
2. Print the entry + `every N minutes` header.
3. Platform block: Unix → three `crontab -e` steps; Windows →
   six Task Scheduler steps (binary path + `sync` argument).
4. Prompt `Copy to clipboard? [y/N]`; on `y`, best-effort
   `copy_to_clipboard_auto` (failure → stderr warning, still `Ok`).

`shell_escape_path` (`:11`): empty → `''`; strings containing
space/`'"/\$\`` wrap in single quotes with `'` → `'\''`. Paths from
`current_exe()` almost always pass through unquoted.

## Mutation vs read-only

Read-only. Prints text and optionally writes the clipboard; never
touches config, libraries, cron tables, or the Task Scheduler.
No gate, no lock, no save.

## Auto-sync trigger

None. The generated entry invokes foreground `snp sync` on a timer,
which performs its own execution-lock + pending-clear cycle when it
fires — but generating the entry schedules nothing.

## Error / exit mapping

`SnipResult<()>`: interval 0 → error (exit 1/2 family); clipboard
failure is swallowed to a warning. Success prints unconditionally.

## Key invariants

- `make_cron_entry` performs no I/O (notably no stdin reads), so unit
  tests cover validation + quoting without interaction.
- Binary resolution prefers the running executable over `PATH`, so
  dev installs and renamed binaries still schedule correctly.
- The entry runs plain `snp sync` (bidirectional default); direction
  flags belong in `sync.toml`, not in the crontab line.
- Prompt default is No — piping/Enter never copies.

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

## Testing

`make_cron_entry` and `shell_escape_path` carry the unit tests
(`cron_cmd.rs:97-162`): zero-interval rejection, valid-interval
acceptance, and quoting cases (empty, simple, spaces, embedded single
quotes, `$`, backticks, backslashes). The interactive prompt and
clipboard branch are untested by unit tests (stdin/clipboard seams) —
covered indirectly by platform smoke tests.

## File / line references

- `CronArgs`: `src/commands/cron_cmd.rs:6`; escaping: `:11`
- `binary_path`: `:27`; `make_cron_entry`: `:37`; `run`: `:53`
- Tests: `:97-162`; dispatch: `src/main.rs:723-725`

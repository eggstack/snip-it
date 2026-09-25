# status_cmd — Auto-Sync Status Display

[← Back to Overview](../overview.md)

## Overview

`src/commands/status_cmd.rs` (136 lines) renders the canonical
`status_snapshot::capture_snapshot()` as human text or JSON. It is the
read path over the durable auto-sync state (`auto-sync-status.toml`,
pending marker, execution liveness) — the same snapshot `doctor --sync`
maps into diagnostics and `sync retry/repair` acts on.

## CLI surface

`StatusArgs` (`status_cmd.rs:11`), `snp status` and `snp data status`
(alias `s`) via `handle_status` (`main.rs:438`): `--json` (pretty
`StatusSnapshot` to stdout), `--sync-only` (suppress the
`Local:` library census line).

## Flow / steps

`run(json, sync_only)` (`status_cmd.rs:18`):

1. `capture_snapshot()` — local census (libraries, snippets, primary),
   sync top-level state, pending view, last/next attempt timestamps,
   log dir, typed diagnostics.
2. JSON mode: serialize + `writeln!` to a locked stdout, return.
3. Human mode: `Local: {libraries} libraries, {snippets} snippets;
   primary={name|none}` (skipped with `--sync-only`), then
   `format_sync_line`, `Pending generation: {g}` (if pending), `Last
   attempt: {class} at {local_dt}` (if any), `Next retry: {dt}` (if
   future), action hint from `format_action`, and `Logs: {dir}`.
4. Single locked-`stdout` write; I/O errors → `io_error`.

`format_sync_line` (`:101`) covers all eight `TopLevelSyncState`
variants: `CorruptOrInaccessible`, `LiveExecution{pid}`,
`PendingAttentionRequired`, `PendingRetryBackoff`,
`PendingAwaitingScheduling`, `ConfiguredAndCurrent`,
`ConfiguredAutoSyncDisabled`, `NotConfigured`. `format_action`
(`:117`) hints only for three: attention → `` run `snp sync retry` ``,
backoff → eligible-at time, corrupt → `` run `snp sync repair` ``.

## Mutation vs read-only

Read-only. No gate, no lock acquisition (snapshot reads are
lock-free), no writes, no runtime.

## Auto-sync trigger

None. Status neither notifies nor syncs; it only observes pending and
liveness. The displayed generation is the same counter
`sync retry/discard-pending/repair` operate on.

## Error / exit mapping

`SnipResult<()>` → `Success`. Only failures are snapshot
serialization and stdout writes (exit 1 family). Corrupt state is
*displayed* (`Sync: corrupt or inaccessible state` + repair hint),
never an error from this command.

## Key invariants

- Human and JSON render from the identical snapshot — no second
  read, no drift between formats.
- Millis timestamps render via `timestamp_millis_opt → Local`; a
  `next_attempt` in the past is hidden (already eligible).
- Empty `last_failure_class` displays as `unknown`, never blank.
- `sync_only` affects display only, not the snapshot or JSON shape.

## Example output

```
Local: 3 libraries, 41 snippets; primary=main
Sync: pending
Pending generation: 7
Last attempt: transient at 2026-05-01 09:14:22
Action: run `snp sync retry` to retry now
Logs: /home/user/.config/snp/logs
```

`--sync-only` drops the first line; `--json` replaces the whole block
with the serialized `StatusSnapshot` (same field names the doctor
snapshot mapper consumes).

## Snapshot fields consumed here

`snapshot.local{libraries, snippets, primary_library}`,
`snapshot.sync.top_level`, `snapshot.pending.state`,
`snapshot.attempt.{last_attempt_at_unix_ms, last_failure_class,
next_attempt_at_unix_ms}`, `snapshot.log_dir`, plus
`snapshot.diagnostics` (ignored by status, mapped by doctor). Status
renders the top-level rollup; per-finding detail is doctor's job.

## File / line references

- `StatusArgs`: `src/commands/status_cmd.rs:11`; `run`: `:18`
- `format_sync_line`: `:101`; `format_action`: `:117`
- Snapshot: `src/status_snapshot.rs`; handler: `src/main.rs:438`

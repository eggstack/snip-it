# status_cmd — Auto-Sync Status Display

[← Back to Overview](../overview.md)

## Overview

`src/commands/status_cmd.rs` (136 lines) renders the canonical
`status_snapshot::capture_snapshot()` as human text or JSON. It is the
read path over the durable auto-sync state (`auto-sync-status.toml`, the
pending marker, and execution liveness) — the same snapshot `snp doctor`
maps into diagnostics and `snp sync retry` / `snp sync repair` act on.

It does **not** sync, retry, repair, or notify. It observes.

Related: [../status.md](../status.md) (the snapshot schema and the
doctor mapping), [sync_cmd.md](sync_cmd.md) (the commands it points at),
[../auto_sync.md](../auto_sync.md).

## CLI surface

```rust
// src/commands/status_cmd.rs:10-16
#[derive(Debug, Clone, clap::Args)]
pub struct StatusArgs {
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub json: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub sync_only: bool,
}
```

`StatusArgs` is the single source of truth for **both** spellings:
`snp status` (`src/main.rs:195`) and `snp data status` (alias `s`,
`src/main.rs:292-294`). Both dispatch through the single-path handler
`handle_status` (`src/main.rs:465-468`), which calls
`status_cmd::run(args.json, args.sync_only)` and always returns
`CliOutcome::Success`.

| Flag | Type | Default | Behavior |
|------|------|---------|----------|
| `--json` | bool | off | Serialize the whole `StatusSnapshot` as pretty JSON to stdout |
| `--sync-only` | bool | off | Suppress the `Local:` census line in human mode |

`--sync-only` affects **display only** — it does not change the snapshot
and does not change the JSON shape.

## Flow / steps

`run(json, sync_only)` (`src/commands/status_cmd.rs:18`):

1. `capture_snapshot()` — one point-in-time read.
2. **JSON mode** (`:21-34`): `serde_json::to_string_pretty(&snapshot)`,
   then `writeln!` to a locked stdout. A serialization failure becomes
   `runtime_error("failed to serialize snapshot")`; a write failure
   becomes `io_error("write stdout")`.
3. **Human mode** (`:36-96`): build the whole block into a `String` and
   write it once to a locked stdout.
   - `Local: {libraries} libraries, {snippets} snippets; primary={name|none}`
     — skipped when `--sync-only` (`:38-45`).
   - `format_sync_line(&snapshot.sync.top_level)` (`:47`).
   - `Pending generation: {generation}` — only when
     `pending.state` is `Pending { generation, .. }` (`:49-53`).
   - `Last attempt: {class} at {local_dt}` — only when
     `attempt.last_attempt_at_unix_ms > 0`; an empty
     `last_failure_class` renders as `unknown` (`:55-68`).
   - `Next retry: {local_dt}` — only when
     `attempt.next_attempt_at_unix_ms` is in the **future**; a past
     timestamp is hidden because the retry is already eligible (`:70-84`).
   - The action hint from `format_action` (`:86-88`).
   - `Logs: {log_dir}` (`:90`).

### `format_sync_line` (`:101-115`)

Covers all eight `TopLevelSyncState` variants
(`src/status_snapshot.rs:53-62`):

| Variant | Rendered line |
|---------|---------------|
| `CorruptOrInaccessible` | `Sync: corrupt or inaccessible state` |
| `LiveExecution { pid, .. }` | `Sync: active (pid={pid})` |
| `PendingAttentionRequired` | `Sync: attention required` |
| `PendingRetryBackoff { .. }` | `Sync: pending retry` |
| `PendingAwaitingScheduling` | `Sync: pending` |
| `ConfiguredAndCurrent` | `Sync: current` |
| `ConfiguredAutoSyncDisabled` | `Sync: auto-sync disabled` |
| `NotConfigured` | `Sync: not configured` |

### `format_action` (`:117-136`)

Hints for only three variants:

| Variant | Hint |
|---------|------|
| `PendingAttentionRequired` | ``Action: run `snp sync retry` to retry now`` |
| `PendingRetryBackoff { next_attempt_at_unix_ms }` | `Action: retry eligible at {local_dt}` |
| `CorruptOrInaccessible` | ``Action: run `snp sync repair` to diagnose`` |

Every other variant renders no action line.

## The snapshot it renders

`capture_snapshot()` (`src/status_snapshot.rs:144-199`) builds:

| Field | Source |
|-------|--------|
| `schema` | `SNAPSHOT_SCHEMA` = 1 (`src/status_snapshot.rs:15`) |
| `generated_at_unix_ms` | `unix_now_ms()` |
| `config_root` | `utils::config::get_config_dir()` |
| `log_dir` | `logging::get_default_log_dir()` |
| `local` | `capture_local_summary()` (`:541-568`) |
| `sync.configuration` | `sync_configuration_state()` (`:201-215`) — `NotConfigured` / `Configured` / `ConfiguredAutoSyncDisabled` / `LoadFailed` |
| `sync.top_level` | `derive_top_level(...)` from the pending view, execution view, attempt view, and configuration state |
| `pending.state` | `pending_state_view(&state_dir)` (`:217-249`) — `None` / `Pending` / `Corrupt` / `Inaccessible` |
| `attempt` | `attempt_from_status(...)` (`:160`, defined at `:512`) |
| `execution.execution_lock` / `execution.worker_lock` | `execution_state_view(&state_dir)` — `ProcessStateView` (`Idle` / `Live` / `DeadStale` / `Malformed` / `Inaccessible`) |
| `diagnostics` | `collect_diagnostics(&snapshot)` (`:197`) |

`capture_local_summary` uses
`LibraryManager::inspect_library_index().primary` so `status`, `doctor`,
and `validate` agree on the primary library; a missing primary *file*
still reports its configured name (`src/status_snapshot.rs:545-554`).

## Mutation vs read-only — not strictly read-only

The snapshot reads are lock-free and the command takes no execution lock,
but **`snp status` is not a pure read**. The read paths it drives can
write to disk:

| Side effect | Trigger | Site |
|-------------|---------|------|
| `libraries.toml.corrupt` | `libraries.toml` exists but does not parse — the file is copied to a backup and the constructor returns an error | `src/library/manager.rs:62` (reached via `LibraryManager::new()` in `capture_local_summary`, `src/status_snapshot.rs:542`) |
| `<library>.toml.corrupt.bak` | A library file fails to parse — `load_library` copies it to `<name>.toml.corrupt.bak` before returning the parse error | `src/library/persistence.rs:216` (reached via `count_snippets`, `src/status_snapshot.rs:576`) |
| `sync.toml.corrupt.bak` | `sync.toml` fails its CRC32 integrity check — the file is copied to a backup and the settings are reset to defaults | `src/config/sync_settings.rs:515-540` (reached via `load_sync_settings` in `sync_configuration_state`, `src/status_snapshot.rs:202`) |
| `sync.toml` rewrite + keychain write | A plaintext API key is migrated to the OS keychain, and the `@keychain` marker is written back to `sync.toml` | `src/config/sync_settings.rs:353-391`, `:546-550` |
| Config-directory migration | The legacy macOS config directory is moved into the XDG location | `src/library/manager.rs:45` → `src/utils/config.rs:166-206` |

The corrupt-file backups are **best-effort diagnostics**: the original
files are left in place, and the command still exits 0. But a
`libraries.toml` parse failure makes `capture_local_summary` fall back to
`LocalSummary { libraries: 0, snippets: 0, primary_library: None }`
(`src/status_snapshot.rs:562-567`), so `snp status` can report an empty
local census while the real index is on disk next to a
`libraries.toml.corrupt` backup.

**No transaction gate** (`gate_mutation_on_interrupted_transactions`) is
called, and no execution lock is acquired. `snp status` is classified as
`StartupRecoveryPolicy::SuppressReadOnly` with
`StartupServices::Minimal` (`src/main.rs:927-942`) — no file logging, and
pending auto-sync is not rescheduled before the command runs.

## Tokio runtime

None. `snp status` never touches the global `RUNTIME`
(`src/main.rs:29-35`).

## Auto-sync trigger

None. Status neither notifies nor syncs. The generation it displays is
the same counter `snp sync retry` / `discard-pending` / `repair` operate
on.

## Error / exit mapping

`SnipResult<()>` → `handle_status` returns `CliOutcome::Success`
(`src/main.rs:465-468`).

| Situation | Result |
|-----------|--------|
| Normal render | exit 0 |
| JSON serialization failure | `runtime_error("failed to serialize snapshot")` → exit 1 |
| stdout write failure | `io_error("write stdout")` → exit 1 |
| Corrupt pending marker / corrupt status file | **Displayed, not an error** — `Sync: corrupt or inaccessible state` plus the repair hint; exit 0 |

Corrupt state is a *displayed condition*, not a failure of this command.

## Key Files

| Path | Role |
|------|------|
| `src/commands/status_cmd.rs` | `StatusArgs`, `run`, `format_sync_line`, `format_action` |
| `src/status_snapshot.rs` | `capture_snapshot` + the whole snapshot schema |
| `src/auto_sync/status.rs` | `read_status_typed`, `AutoSyncStatus`, `status_path` |
| `src/auto_sync/pending.rs` | `read_state_from_dir` — the pending marker read |
| `src/auto_sync/execution_lock.rs`, `src/auto_sync/lock.rs` | Execution- and worker-lock liveness |
| `src/library/manager.rs:43-94` | `LibraryManager::new` — migration + fail-closed index load |
| `src/library/persistence.rs:216` | `load_library` corrupt backup |
| `src/config/sync_settings.rs:506-561` | `load_sync_settings` — integrity check, corrupt backup, keychain migration |
| `src/main.rs:465-468` | `handle_status` — the shared single-path handler |

## Invariants

- **Human and JSON render from the identical snapshot** — one capture, two
  projections; there is no second read and no drift between formats.
- **Timestamps render in local time** via `timestamp_millis_opt → Local`.
- **A `next_attempt` in the past is hidden** — it is already eligible.
- **An empty `last_failure_class` displays as `unknown`**, never blank.
- **`--sync-only` affects display only.**
- **Corrupt state is reported, never raised** — `snp status` exits 0.

## Gotchas

- **`snp status` is not strictly read-only.** It can create
  `*.toml.corrupt.bak`, `libraries.toml.corrupt`, rewrite `sync.toml`
  during keychain migration, and migrate the macOS config directory.
- **A corrupt `libraries.toml` makes the local census read as empty**
  (0 libraries, 0 snippets) rather than as an error.
- **A corrupt `sync.toml` silently resets the settings to defaults** —
  the status shows `Sync: not configured` and a warning is printed on
  stderr.
- **`snp status` takes no execution lock**, so the reported liveness is
  a point-in-time observation, not a reservation.
- **`snp data status` is the same code path** — same snapshot, same
  flags.

# sync_cmd — Bidirectional Sync, Recovery, Library Linking

[← Back to Overview](../overview.md)

## Overview

`src/commands/sync_cmd.rs` (990 lines) is the foreground sync
operator: it owns the `SyncExecutionLock` wait, server library
linking (with interactive conflict triage), direction resolution,
durable status recording, pending-clear ordering, and five recovery
subcommands (`config`, `retry`, `clear-failure`, `discard-pending`,
`repair`). The merge itself lives in `sync_commands::run_sync`; this
module is the locking/status/linking shell around it.

## CLI surface

`snp sync` (alias `y`), subcommands in `main.rs:272-321`:

| Form | Handler | Flags |
|------|---------|-------|
| `snp sync` / `snp sync run` | `run(SyncOptions)` (`:173`) | `-l/--library`, `--servers`, `--push-only` ⊥ `--pull-only`, `--dry-run` |
| `snp sync config` | `run_config` (`:356`) | `--show`, `--auto-sync on\|off`, `--debounce`, `--max-delay`, `--failure ignore\|warn\|error`, `--timeout` |
| `snp sync retry [--library]` | `run_retry` (`:511`) | Replays pending work only; `No pending sync work` when absent |
| `snp sync clear-failure` | `run_clear_failure` (`:611`) | Clears attention/failures; corrupt status → error |
| `snp sync discard-pending [--force] [--generation N]` | `run_discard_pending` (`:636`) | Confirm-guarded; non-tty requires `--force`; generation must match |
| `snp sync repair [--dry-run] [--apply]` | `run_repair` (`:714`) | Quarantine corrupt status, drop orphaned temps, fix `0o600` |

`SyncOptions{library, servers, push_only, pull_only, dry_run}` (`:162`).

## Flow / steps

### `run()` (`:173`)

1. `load_sync_settings`; disabled → hint + `Ok`; missing API key →
   `snp register --force` hint + `Ok` (both are success-with-message,
   not errors).
2. `wait_acquire(state_dir, 30 s)`; `Timeout/AlreadyHeld` map to
   `"sync already in progress"` with owner pid.
3. `--servers`: create client, `list_libraries`, print, return.
4. Create client (`ConnectFailed` on failure) → `list_libraries` →
   `init_library_manager` (`LibraryManagerInitFailed` on failure).
5. `--dry-run`: resolve library, print direction + live-snippet list,
   return before any linking.
6. Link every server library (`link_server_library`, `:49`): slug
   `lowercase + spaces→hyphens`; same-ID → skip; unlinked local with
   content on both sides → prompt `skip/overwrite/rename`
   (`prompt_conflict`, `:140`); overwrite clears + links (rollback
   unlink on save failure); rename backs local content to
   `<name>_local` (fail-closed on malformed input) then clears + links;
   unknown → `add_server_library`.
7. Direction: CLI flags win, else config (`Push`/`Pull` → one-sided,
   else bidirectional by passing neither flag).
8. `observe_pending_generation()` **before** `sync_commands::run_sync`
   (a mutation racing the sync is preserved — D5).
9. `record_success` / `record_failure` (best-effort, warn-only) with
   config fingerprint; propagate `sync_result`; on success
   `clear_pending_after_explicit_sync(observed, true)`.

### Recovery handlers

- `run_config`: `--show` prints effective config (with
  enabled-without-key warnings); setters clamp to
  `AUTO_SYNC_DEBOUNCE_MIN/MAX`, `MAX_DELAY_MIN/MAX`,
  `MIN/MAX_SYNC_TIMEOUT_SECS`; saves only when something changed.
- `run_retry`: pending-gated (`NotFound → "No pending sync work"`,
  corrupted → runtime error); runs `run_sync` in the config direction;
  success records status + generation-matched clear, failure records.
- `run_clear_failure`: `Corrupt → error`, `Missing → "No failure
  recorded"`, else reset attention/failures/next-attempt/message.
- `run_discard_pending`: prints generation; tty confirm (`y/yes`)
  unless `--force` (non-tty without `--force` → `ConflictOrRefused`);
  generation arg must match; conditional clear maps
  Cleared/Missing → `Success`, changed → `ConflictOrRefused`, I/O →
  `PersistenceFailed`, corrupt → `ValidationFailed`.
- `run_repair`: scans corrupt status (recreate-empty only when pending
  exists, else quarantine), orphaned `snp-sync-tmp.*`/`.quarantine.*`
  temps, and non-`0o600` status perms. Never touches kernel-lock
  files. `--dry-run` or no `--apply` prints only; `--apply` quarantines
  (copy + dir-fsync + remove) then applies.

## Mutation vs read-only

Sync is a remote-mutating, locally-journaled operation: it holds the
execution lock, writes merged libraries, and records durable status.
`--dry-run`, `--servers`, `config --show`, and `repair` without
`--apply` are read-only. `discard-pending`/`clear-failure` mutate sync
*state*, never snippet data.

## Auto-sync trigger

Foreground sync does not `notify_mutation` — it *is* the sync: it
observes, runs, records, and clears pending. `SyncMerge` origin never
re-triggers (no loops). Recovery commands manipulate pending/status
directly with generation guards.

## Error / exit mapping

- Sync failures → `SyncFailureKind` (`ConnectFailed`,
  `LibraryManagerInitFailed`, merge kinds) → exit 7.
- Lock contention → `runtime_error` (exit 1/9 family) with owner pid.
- `run_discard_pending` returns `CliOutcome` directly: `Success` (0),
  `ConflictOrRefused` (9), `ValidationFailed` (6),
  `PersistenceFailed` (1 family).
- Disabled/unconfigured sync and empty retry are exit-0 messages.

## Key invariants

- Observe-then-run-then-clear ordering is load-bearing (D5/H/D).
- Status writes are best-effort and never mask the sync result.
- Library linking fails closed: malformed local content is never
  backed up over by emptiness; link rollback on clear failure.
- Repair quarantines before deleting; lock files are never repaired.
- `SyncOptions` direction default (neither flag) means bidirectional.

## File / line references

- `SyncOptions`/`run`: `src/commands/sync_cmd.rs:162,173`
- Linking: `:9-159`; `run_config`: `:356`; `run_retry`: `:511`
- `run_clear_failure`: `:611`; `run_discard_pending`: `:636`
- `run_repair`/`apply_repair_action`: `:714,837`; tests: `:915-990`
- Merge: `src/sync_commands.rs`; statuses: `src/auto_sync/status.rs`

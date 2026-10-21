# sync_cmd — Foreground Sync, Recovery, and Sync Control Commands

[← Back to Overview](../overview.md)

## Overview

`src/commands/sync_cmd.rs` (990 lines) is the foreground sync operator. It
owns the `ExecutionLock` wait, server-library linking (with interactive
conflict triage), direction resolution, durable auto-sync status recording,
pending-clear ordering, and the five sync *control* subcommands (`config`,
`retry`, `clear-failure`, `discard-pending`, `repair`) that sit next to the
default `run` subcommand.

It does **not** own the merge. The wire protocol, the per-library loop, the
counter semantics, and the missing-library recovery live in
`src/sync_commands.rs` (`run_sync` / `run_sync_with_limits` /
`handle_library_not_found`). This module is the locking, status, linking, and
operator-tooling shell around that engine. Auto-sync (the detached worker)
is a separate path documented in [../auto_sync.md](../auto_sync.md); it shares
`src/sync_commands.rs` but not this module.

Related: [../sync.md](../sync.md) (protocol + merge),
[../status.md](../status.md) (the snapshot `snp status` renders),
[mod.md](mod.md) (command-module map).

## CLI surface

`snp sync` (alias `y`) — `src/main.rs:108-113`. The subcommand group is
`Option<SyncCommands>` (`src/main.rs:272-335`), so a bare `snp sync` falls
through to `SyncCommands::Run`.

| Form | Alias | Handler | Flags |
|------|-------|---------|-------|
| `snp sync` / `snp sync run` | `s` | `run(SyncOptions, &RUNTIME)` (`sync_cmd.rs:173`) | `-l/--library <NAME>`, `--servers`, `--push-only` ⊥ `--pull-only`, `--dry-run` |
| `snp sync config` | `c` | `run_config(...)` (`sync_cmd.rs:356`) | `--show`, `--auto-sync <on\|off\|...>`, `--debounce <SECS>`, `--max-delay <SECS>`, `--failure <ignore\|warn\|error>`, `--timeout <SECS>` |
| `snp sync retry` | `r` | `run_retry(library, &RUNTIME)` (`sync_cmd.rs:511`) | `-l/--library <NAME>` |
| `snp sync clear-failure` | `f` | `run_clear_failure()` (`sync_cmd.rs:611`) | none |
| `snp sync discard-pending` | `d` | `run_discard_pending(force, generation)` (`sync_cmd.rs:636`) | `--force`, `--generation <N>` |
| `snp sync repair` | — | `run_repair(dry_run, apply)` (`sync_cmd.rs:714`) | `--dry-run`, `--apply` |

### `SyncOptions` (`src/commands/sync_cmd.rs:162`)

```rust
pub struct SyncOptions {
    pub library: Option<String>,
    pub servers: bool,
    pub push_only: bool,
    pub pull_only: bool,
    pub dry_run: bool,
}
```

The clap surface is declared inline in `src/main.rs:274-288` (not as an
`*Args` struct) and is mapped into `SyncOptions` at `src/main.rs:676-694`.
`--push-only` and `--pull-only` are mutually exclusive
(`conflicts_with = "pull_only"` / `"push_only"`, `src/main.rs:280-285`).

### `snp sync config` flags (`src/main.rs:289-310`)

| Flag | Type | Default | Behavior |
|------|------|---------|----------|
| `--show` | bool | off | Print the effective auto-sync configuration |
| `--auto-sync` | string | unset | `on/true/1/yes` → enable; `off/false/0/no` → disable; anything else → error |
| `--debounce` | u64 seconds | unset | Clamped to `AUTO_SYNC_DEBOUNCE_MIN..=AUTO_SYNC_DEBOUNCE_MAX` (0–300) |
| `--max-delay` | u64 seconds | unset | Clamped to `AUTO_SYNC_MAX_DELAY_MIN..=AUTO_SYNC_MAX_DELAY_MAX` (0–600); stored as `Some(..)` |
| `--failure` | string | unset | Parsed into `AutoSyncFailureMode` (`ignore`/`warn`/`error`) |
| `--timeout` | u64 seconds | unset | Clamped to `MIN_SYNC_TIMEOUT_SECS..=MAX_SYNC_TIMEOUT_SECS` (5–120); stored as `Some(..)` |

Range constants: `src/config/sync_settings.rs:27-39`. `--show` and the
setters can be combined; the file is written only when at least one setter
ran (`sync_cmd.rs:369-373`, `:501-506`).

## Flow: `run()` (`src/commands/sync_cmd.rs:173`)

1. **Load settings.** `load_sync_settings()`; a load error is printed and
   propagated (`:174-177`).
2. **Not enabled** → print `Sync is not enabled. Configure sync settings
   first.` **to stderr** and return `Ok(())` — exit 0, not an error
   (`:179-182`).
3. **No API key** → print `Sync is enabled but no API key is configured. Run
   'snp register --force'.` **to stderr** and return `Ok(())` — exit 0
   (`:183-186`).
4. **Acquire the execution lock.** `wait_acquire(state_dir, 30 s)` where
   `state_dir = auto_sync::notification::derive_state_dir()` (`:189-210`).
   `Timeout` / `AlreadyHeld` become `runtime_error("sync already in
   progress")` carrying the owner pid; other errors become
   `runtime_error("failed to acquire sync lock")`.
5. **`--servers`** → create the client, call `list_libraries()`, print
   `Server libraries:` plus `  <name> (<id>)` lines, return `Ok(())`
   (`:212-232`). A listing failure is printed to stderr but **still exits 0**.
6. **Create the client** (`:234-241`); a failure becomes
   `sync_failure(ConnectFailed)`.
7. **`list_libraries()`** (`:243`); a failure becomes
   `sync_failure(ConnectFailed)` (`:345-351`).
8. **Initialize the library manager** (`:245-250`);
   `sync_failure(LibraryManagerInitFailed)` on failure.
9. **`--dry-run`** → resolve the library via
   `commands::get_library_path(options.library)`, print
   `[DRY RUN] Would sync snippets:`, the effective direction, the snippet
   count, and one `  - <description> (<id-prefix>)` line per non-deleted
   snippet; then return **before any linking** (`:252-277`).
10. **Link every server library** via `link_server_library` (`:279-281`).
11. **Resolve direction** (`:287-290`): an explicit `--push-only` /
    `--pull-only` wins; otherwise the configured `sync_direction` decides;
    otherwise neither flag is passed and the engine defaults to
    bidirectional.
12. **Observe the pending generation** *before* the sync
    (`auto_sync::observe_pending_generation()`, `:293`) so a mutation that
    races the sync is not swallowed by the later clear.
13. **Run the engine**: `sync_commands::run_sync(&settings, library,
    effective_push, effective_pull, runtime)` (`:294-300`).
14. **Record durable status** (`:302-331`): `record_success` on `Ok`,
    `record_failure` on `Err` with `FailureClass::from_error(e)` and
    `compute_config_fingerprint(&settings)`. Both writes are best-effort —
    a status-write failure only logs a warning and never masks the sync
    result.
15. **Clear pending** via `auto_sync::clear_pending_after_explicit_sync(
    observed_generation, sync_succeeded)` (`:338-341`), then propagate the
    sync result (`:334`).

### Linking (`src/commands/sync_cmd.rs:9-135`)

`server_library_filename` (`:9`) slugs a remote name to
`lowercase + spaces→hyphens`. `link_server_library` (`:49`) then:

- Remote ID differs from the local `library_id` → print
  `Library '<name>' has different server ID, skipping` and skip (`:55-59`).
- Local library exists but is unlinked, has local content, and the server
  reports snippets → interactive `prompt_conflict` (`:140`) with
  `skip` / `overwrite` / `rename` (`:64-111`):
  - **overwrite** → `clear_library_for_server_pull` (`:27`): link, then
    write an empty library; if the write fails, unlink again so the
    pairing is not half-applied (`:38-43`).
  - **rename** → create `<name>_local`, copy the local snippets into it
    (**fail closed**: a malformed local file aborts the rename rather than
    backing up an empty library over it, `:82-88`), then clear + link the
    original.
  - **skip** → keep the local version, no link.
- Unlinked but otherwise fine → link and print
  `Linked '<file>' to server library '<id>'` (`:113-118`).
- No local library at all → `mgr.add_server_library(name, id)` (`:125`).

## Counter semantics

The engine accumulates a `SyncStatus` (`src/sync_commands.rs:586-591`):

```rust
struct SyncStatus {
    pushed: u32,
    pulled: u32,
    conflicts: u32,
    failed: u32,
}
```

| Counter | Meaning | Incremented at |
|---------|---------|----------------|
| `pushed` | Libraries whose upload/merge leg completed | `sync_commands.rs:916` (push), `:1032` (pull leg) |
| `pulled` | Snippets received from the server | `add_pulled` (`:603-607`), called at `:945`, `:1033`, `:320` |
| `conflicts` | Snippets where the merge chose the **remote** copy | `sync_commands.rs:949-951`, `:1037-1039` — derived from the merge result, not from "the row has a foreign `device_id`" |
| `failed` | Libraries that did not complete cleanly | pre-flight failures (`:812`), missing file (`:857`), unlinked library (`:870`), fail-closed `load_library` (`:883`), `skipped_count > 0` (`:922`, `:968`, `:1052`), recovery failures |

`status.failed > 0` at the end of `run_sync_with_limits` returns
`SnipError::sync_failure(SyncFailureKind::PartialSyncFailure)`
(`sync_commands.rs:1111-1118`).

### `skipped_count` — the load-bearing rule

The server reports refused rows in `skipped_count` **without** flipping
`success`. Since commit `1920075`:

- Push direction: `pushed` still increments (the upload reached the
  server), but `failed` also increments and `last_sync` is **not** advanced
  (`sync_commands.rs:908-931`).
- Bidirectional direction (the default): the merge still runs, but `failed`
  increments and the library result is recorded as *not* successful
  (`:957-974`). Before the fix this branch counted `skipped_count > 0` as
  `conflicts` and recorded the library as *succeeded* — the run exited 0,
  the pending generation was cleared, and the refused records were never
  retried.
- Pull direction: same treatment — merge, `pushed`/`pulled` updated, but
  `failed` increments and `last_sync` is not advanced (`:1024-1057`).

`last_sync` is only advanced when `skipped_count == 0` in every branch
(`:917-918`, `:939-940`, `:1025-1026`).

### Server pre-flight failures

Before the per-library loop, `run_sync_with_limits` creates any unlinked
library on the server (`sync_commands.rs:755-803`). Each failure is now
recorded instead of silently skipped:

| Pre-flight failure | Recorded at |
|--------------------|-------------|
| `create_library` failed | `sync_commands.rs:793-800` → `preflight_failures` |
| `link_server_library` failed after create | `sync_commands.rs:779-785` → `preflight_failures` |
| Library file missing | `sync_commands.rs:855-863` → `status.failed += 1` |
| Library not linked to a server | `sync_commands.rs:868-877` → `status.failed += 1` |
| `load_library` failed closed (malformed TOML) | `sync_commands.rs:879-887` → `status.failed += 1` |

`preflight_failures` are surfaced before any library work
(`:810-814`) and the affected library is skipped later so it is not
counted twice (`:842-847`). Before `1920075` these paths left
`status.failed` at zero, so the run exited 0, cleared the pending
generation, and the library was never uploaded or retried.

## Missing-library recovery

When the server reports `SyncFailureKind::LibraryNotFound`, the engine calls
`handle_library_not_found` (`src/sync_commands.rs:79-353`). The state lives
in an atomic per-library marker `<library>.sync_recovery` inside the
libraries directory (`recovery_marker_path`, `sync_commands.rs:50-52`),
written with `atomic_replace` at `Durability::DurableUserData`
(`:54-64`).

Marker shape (`sync_commands.rs:24-40`):

```rust
struct SyncRecoveryMarker {
    schema: 1,
    local_library_name: String,
    local_library_id: String,
    server_library_id: Option<String>,
    created_at_unix_ms: i64,
    phase: RecoveryPhase,   // Creating | RemoteCreated | Linked
}
```

Recovery sequence:

1. Locate the marker; **a corrupt marker is preserved, not overwritten**
   (`:107-116`).
2. Verify marker identity — schema, normalized name, filename stem, and
   library ID must match the current local library (`:146-164`); a mismatch
   fails closed with `Sync recovery identity mismatch`.
3. Persist the marker before touching the server — "failed to persist
   recovery marker; refusing remote recreation" (`:189-194`).
4. Resolve the remote library: **exactly one** normalized-name match is
   reused; zero matches → `create_library`; two or more → `Ambiguous remote
   library recovery` and refusal (`:221-261`).
5. Persist the server ID and phase `RemoteCreated` (`:264-271`).
6. `relink_server_library(lib_name, id, Some(0))` — linkage and the
   `last_sync` reset happen in one save (`:273-279`) — then phase `Linked`
   (`:280-286`).
7. Retry the upload with `last_sync = 0` (`:290-293`).
8. On a successful retry: **advance `last_sync` and remove the marker only
   when `skipped_count == 0`** (`:305-319`). If records were skipped, the
   marker stays on disk, `failed` increments, and the result is
   `Re-linked; N snippets skipped (will retry)` (`:321-330`).

Before `1920075` both the watermark advance and the marker removal were
unconditional: `success` did not imply a clean upload, and a freshly
re-created library returns no server rows at all, so an all-refused retry
still reported success and left no durable trace that records were dropped.

`check_and_complete_recovery_markers` (`sync_commands.rs:356-421`) scans the
libraries directory for `*.sync_recovery` files at the start of every run
and drives the same recovery path. A marker with no local library is
preserved (`:391-394`); a corrupt marker is preserved (`:414-416`).

## Manual path vs auto-sync path

| Aspect | `snp sync run` (manual) | Detached auto-sync worker |
|-------|-------------------------|---------------------------|
| Entry | `sync_cmd::run` → `sync_commands::run_sync` | `auto_sync::worker` → `sync_commands::run_sync_with_limits` |
| `SyncRunLimits` | `None` — unbounded (`sync_commands.rs:657-664`) | `Some(SyncRunLimits { deadline, request_timeout })` (`src/sync.rs:64-74`) |
| Execution lock | `wait_acquire(state_dir, 30 s)` with a bounded wait | `WorkerLock` (kernel `flock`/`LockFileEx`), blocking |
| Library set | CLI `--library` or the config index | Configured library set |
| Pending clear | `clear_pending_after_explicit_sync(observed_gen, ok)` | Worker clears its own generation |
| Timeout | none (bounded only by the gRPC client defaults) | `AutoSyncPolicy::sync_timeout` (default 30 s, 5–120 s) |

`SyncRunLimits::remaining()` (`src/sync.rs:70-73`) yields the remaining
budget for each retry attempt; the retry macro consults it at
`src/sync.rs:201`.

## Recovery / control handlers

### `run_config` (`src/commands/sync_cmd.rs:356`)

Loads settings, prints usage **to stderr** and returns `Ok(())` when
neither `--show` nor a setter was given (`:375-380`). `--show` prints
`auto_sync`, `auto_sync_debounce_seconds`, `auto_sync_max_delay_seconds`,
`auto_sync_timeout_seconds` (plus the resolved value),
`auto_sync_failure`, and `sync_enabled`, and warns when auto-sync is on
but sync is disabled or no API key is configured (`:382-412`).

Setters clamp rather than reject, printing a "clamping" note to stderr
(`:434-499`); an invalid `--auto-sync` value is a hard error
(`:424-430`) and an invalid `--failure` value is a hard error (`:472-478`).
`save_sync_settings` runs only when something changed (`:501-506`).

### `run_retry` (`src/commands/sync_cmd.rs:511`)

Same enabled / API-key short-circuits as `run` (`:517-524`), the same
30-second bounded execution-lock wait (`:528-548`), then reads the pending
marker: `NotFound` prints `No pending sync work` and returns `Ok(())`;
any other read error is a `runtime_error` (`:550-562`). The direction comes
from the configured `sync_direction` only (`:564-565`) — there are no
direction flags on `retry`. On success it records status and clears the
marker with `clear_if_generation_matches` (`:578-589`); on failure it
records the failure (`:591-604`) and propagates the error.

### `run_clear_failure` (`src/commands/sync_cmd.rs:611`)

Reads the durable status file with `read_status_typed`:

- `Corrupt(msg)` → `runtime_error("status file is corrupt")` (exit 1).
- `Missing` → prints `No failure recorded`, exit 0 (`:619-622`).
- `Valid(status)` → clears `attention_required`, zeroes
  `consecutive_failures`, zeroes `next_attempt_at_unix_ms`, sets
  `message = "cleared by operator"`, writes it back, prints
  `Failure state cleared` (`:623-632`).

It does **not** touch the pending marker — pending intent survives.

### `run_discard_pending` (`src/commands/sync_cmd.rs:636`)

Returns a `CliOutcome` directly (the only sync subcommand that does).

| Situation | Outcome | Exit |
|-----------|---------|------|
| No marker | `Success` (`No pending sync work`) | 0 |
| Marker corrupt | `ValidationFailed` | 6 |
| Marker unreadable (other I/O) | `PersistenceFailed` | 1 |
| Non-tty without `--force` | `ConflictOrRefused` | 9 |
| Interactive answer other than `y`/`yes` | `Success` (`Aborted`) | 0 |
| `--generation N` ≠ observed generation | `ConflictOrRefused` | 9 |
| `clear_if_generation_matches` → `Cleared` | `Success` (`Pending intent discarded`) | 0 |
| → `GenerationChanged { current }` | `ConflictOrRefused` | 9 |
| → `Missing` | `Success` | 0 |
| Clear I/O error | `PersistenceFailed` | 1 |

The generation check (`:677-685`) is a compare-and-clear guard: a
mutation that bumped the generation between the read and the clear is
refused rather than discarded.

### `run_repair` (`src/commands/sync_cmd.rs:714`)

Scans the auto-sync state directory for repairable artifacts:

| Artifact | Detection | Action |
|----------|-----------|--------|
| Corrupt status file | `read_status_typed` → `Corrupt` (`:720-741`) | Quarantine (copy + dir-fsync + remove), and recreate an empty status file **only when a pending marker exists** |
| Orphaned temp/quarantine files | Directory scan for `snp-sync-tmp.*` / `.quarantine.*` (`:743-768`) | Remove the file |
| Non-`0o600` status file | `stat` mode check on Unix (`:772-803`) | `chmod 0o600` |

`--dry-run` **or** the absence of `--apply` prints
`[dry-run] <artifact> <action>: <reason>` and changes nothing
(`:810-816`). With `--apply` (and no `--dry-run`) each action is applied
and printed as `apply <artifact> <action>: <reason>`, followed by
`N repair actions applied` (`:817-832`).

`apply_repair_action` (`:837-913`) quarantines the corrupt status file
before removing it (copy → `fsync` the quarantine directory → remove the
original, `:851-869`) and only recreates an empty status file when the
action string contains `recreate` (`:875-880`).

#### Lock files are never repaired

The permission scan deliberately inspects **only** the status file
(`for path in [&status_path]`, `:772`): kernel lock files are expected
artifacts whose metadata is diagnostic only.

> **Dead branch.** `apply_repair_action` still maps the artifact names
> `execution_lock` (`:899`) and `worker_lock` (`:901`) to
> `auto_sync::execution_lock::execution_lock_path` and
> `auto_sync::lock::lock_path`. Because the scan never produces those
> artifact names, the branches are unreachable in production — the only
> *reachable* reference to the worker-lock path is this one.

## Mutation vs read-only

| Subcommand | Class | Notes |
|------------|-------|-------|
| `sync run` | mutating | Writes merged libraries, `libraries.toml` (`last_sync`, linkage), the auto-sync status file, and the pending marker |
| `sync run --dry-run` | read-only | Prints only; returns before linking |
| `sync run --servers` | read-only (remote) | Lists server libraries; writes nothing locally |
| `sync config --show` | read-only | Prints only |
| `sync config` (setters) | mutating | Writes `sync.toml` |
| `sync retry` | mutating | Same as `run` plus a generation-matched pending clear |
| `sync clear-failure` | mutating (sync state only) | Rewrites the status file; no snippet data |
| `sync discard-pending` | mutating (sync state only) | Removes the pending marker |
| `sync repair` (no `--apply`) | read-only | Prints the action list |
| `sync repair --apply` | mutating | Quarantines/removes/chmods artifacts |

None of these call
`gate_mutation_on_interrupted_transactions()` directly. The library writes
go through `LibraryManager`, whose mutators call `gate_mutation()` →
`transaction::gate_mutation_on_interrupted_transactions`
(`src/library/manager.rs:625-631`).

## Tokio runtime

`RUNTIME` is a `LazyLock<tokio::runtime::Runtime>` in `src/main.rs:22-27`.
`sync run` and `sync retry` touch it (`src/main.rs:695`, `:710`); the
control subcommands (`config`, `clear-failure`, `discard-pending`,
`repair`) do not, so they never construct a runtime.

Startup policy: `Commands::Sync { .. }` uses
`StartupRecoveryPolicy::SuppressExplicitSync` and
`StartupServices::Logging` (`src/main.rs:969-970`), i.e. pending auto-sync
is not auto-rescheduled before an explicit sync.

## Auto-sync interaction

Foreground sync never calls `notify_mutation` — it *is* the sync. It
observes the pending generation, runs, records durable status, and clears
the pending marker. The `SyncMerge` origin never re-triggers, so there is
no feedback loop.

The pending marker itself is owned by the auto-sync module. `startup_recover`
reschedules **any** marker regardless of age — the old
`STALE_PENDING_THRESHOLD_MS` age threshold was removed
(`src/auto_sync/worker.rs:567-584`). Scheduling errors are typed:
`preflight_check` returns `PreflightError` (`src/auto_sync/worker.rs:324-333`)
instead of collapsing a corrupt or unreadable marker into `NothingToDo`
(exit 0); corrupt and unreadable markers now reach `record_sync_failure`
(`src/auto_sync/worker.rs:131-144`, pending-read path `:87-104`).

## Error / exit mapping

- `SnipResult<()>` handlers: **every** `Err` reaches `main` as
  `eprintln!("error: {e}")` + `std::process::exit(1)`
  (`src/main.rs:1025-1031`). Despite `docs/EXIT_CODES.md` listing 7
  (`SYNC_FAILED`) and `CliOutcome::SyncFailed` mapping to it
  (`src/outcome.rs:82`), **no `snp sync` path produces exit 7** —
  `CliOutcome::SyncFailed` is never constructed anywhere in `src/`.
  Sync failures (including `SyncFailureKind::PartialSyncFailure`) exit **1**.
- `run_discard_pending` returns `CliOutcome` directly: `Success` (0),
  `ConflictOrRefused` (9), `ValidationFailed` (6), `PersistenceFailed`
  (1) — `src/outcome.rs:74-90`.
- Disabled sync, missing API key, and "no pending sync work" are exit-0
  messages, not errors.

## Key Files

| Path | Role |
|------|------|
| `src/commands/sync_cmd.rs` | CLI handlers for the whole `snp sync` group |
| `src/main.rs:272-335` | `SyncCommands` clap definitions |
| `src/sync_commands.rs` | The sync engine: library loop, counters, recovery markers |
| `src/sync.rs` | gRPC client, `SyncRunLimits`, `require_api_key`, retry macro |
| `src/auto_sync/execution_lock.rs` | Foreground/worker execution lock (`wait_acquire`) |
| `src/auto_sync/status.rs` | Durable `auto-sync-status.toml` read/write |
| `src/auto_sync/pending.rs` | Pending marker read/conditional-clear |
| `src/auto_sync/policy.rs` | `AutoSyncPolicy`, `FailureClass` |
| `src/config/sync_settings.rs` | `SyncSettings` + auto-sync setting ranges |
| `src/library/manager.rs` | Library index, linkage, `last_sync`, mutation gate |
| `src/outcome.rs` | `CliOutcome` → exit-code mapping |

## Invariants

- **Observe → run → clear.** The pending generation is captured *before*
  the sync so a concurrent mutation is preserved
  (`sync_cmd.rs:291-293`, `:338-341`).
- **`skipped_count == 0` gates both the watermark and the marker.** A
  refused record must leave a durable trace so it is retried.
- **Status writes never mask the sync result.** They are best-effort and
  warn-only.
- **Recovery fails closed.** Corrupt markers, identity mismatches, and
  ambiguous remote matches are refused, not repaired by guesswork.
- **Linking fails closed.** A malformed local library is never replaced by
  an empty backup.
- **Repair quarantines before deleting**, and never touches kernel lock
  files.
- **Generation compare-and-clear.** `discard-pending` and `retry` only
  clear the marker they observed.

## Gotchas

- **`success` does not mean "everything uploaded".** The server reports
  refused rows in `skipped_count` without flipping `success`; a freshly
  re-created library returns no server rows at all.
- **Bidirectional is the default direction**, and it is the branch that
  used to report dropped records as a success.
- **`--servers` swallows its own failure** — a `list_libraries` error is
  printed and the command still exits 0.
- **The library list comes from the config index, not the filesystem**
  (`sync_commands.rs:732-738`), so an orphan file left by a crash during
  delete is not resurrected as a sync candidate.
- **Conflicts are merge-derived.** A row touched by another device is not
  an overwrite; the superseded `sync::detect_device_conflict` was removed.
- **`sync repair` cannot currently repair lock-file permissions** — the
  scan only inspects the status file.

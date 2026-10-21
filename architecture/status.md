# Status Architecture

[← Back to Overview](overview.md)

`snp status` projects the durable auto-sync artifacts — the **pending
marker**, the **attempt status** file, and the two **lock files** — plus local
library facts into a single snapshot, and renders it as human text or JSON.
This document covers that projection, the diagnostic codes it emits, and how
they are consumed by `snp doctor`. It does **not** cover the worker that
writes those artifacts (see [auto_sync.md](auto_sync.md)), the sync protocol
itself (see [sync.md](sync.md)), the `doctor` report format (see
[diagnostics.md](diagnostics.md)), or exit-code semantics in general (see
[outcome.md](outcome.md)). The `status` *command* surface is documented in
[commands/status_cmd.md](commands/status_cmd.md).

## Data flow

```text
snp status [--json]
  -> status_snapshot::capture_snapshot()            (src/status_snapshot.rs:144)
       -> auto_sync::paths::state_dir()              (sync state directory)
       -> config::load_sync_settings()               -> SyncConfigurationState
       -> pending::read_state_from_dir(state_dir)    -> PendingStateView
       -> status::read_status_typed(state_dir)       -> AttemptSummary
       -> execution_state_view(state_dir)            -> ExecutionSummary
            -> inspect_execution_lock / inspect_worker_lock (liveness probes)
       -> capture_local_summary()                    -> LocalSummary
       -> derive_top_level(...)                      -> TopLevelSyncState
       -> collect_diagnostics(&snapshot)             -> Vec<StatusDiagnostic>
  -> status_cmd::run(args)                           (src/commands/status_cmd.rs)
       -> human text  OR  serde_json::to_string_pretty(&snapshot)
```

`SNAPSHOT_SCHEMA` is `1` (`src/status_snapshot.rs:15`).

## Snapshot shape

```rust
pub struct StatusSnapshot {
    pub schema: u32,
    pub generated_at_unix_ms: u64,
    pub config_root: PathBuf,
    pub log_dir: PathBuf,
    pub local: LocalSummary,
    pub sync: SyncSummary,
    pub pending: PendingSummary,
    pub attempt: AttemptSummary,
    pub execution: ExecutionSummary,
    pub diagnostics: Vec<StatusDiagnostic>,
}
```

(`src/status_snapshot.rs:18-29`.) The sub-summaries:

| Type | Fields | Source |
|------|--------|--------|
| `LocalSummary` | `libraries: usize`, `snippets: usize`, `primary_library: Option<String>` | `capture_local_summary()` (`src/status_snapshot.rs:541-568`) |
| `SyncSummary` | `configuration: SyncConfigurationState`, `top_level: TopLevelSyncState` | `src/status_snapshot.rs:47-50` |
| `PendingSummary` | `state: PendingStateView` | `src/status_snapshot.rs:80-82` |
| `AttemptSummary` | `state: AttemptStateView`, `last_attempt_generation`, `last_attempt_at_unix_ms`, `last_success_at_unix_ms`, `last_failure_class`, `consecutive_failures`, `next_attempt_at_unix_ms`, `attention_required`, `message` | `src/status_snapshot.rs:95-105` |
| `ExecutionSummary` | `execution_lock: ProcessStateView`, `worker_lock: ProcessStateView` | `src/status_snapshot.rs:117-120` |
| `StatusDiagnostic` | `code: String`, `severity: DiagnosticSeverity`, `message: String`, `remediation: Option<String>` | `src/status_snapshot.rs:123-128` |

`DiagnosticSeverity` has three variants: `Info`, `Warning`, `Error`
(`src/status_snapshot.rs:130-135`).

## `TopLevelSyncState` — all 8 variants

Derived by `derive_top_level` (`src/status_snapshot.rs:332-379`). The
precedence below is the evaluation order in the code — the first matching
branch wins.

| # | Variant | Payload | Exact meaning | Produced at |
|---|---------|---------|---------------|-------------|
| 1 | `CorruptOrInaccessible` | — | The pending marker could not be parsed or could not be read. Wins over everything, including a live worker. | `src/status_snapshot.rs:338-343` |
| 2 | `LiveExecution { pid, started_at_unix_ms }` | PID + start time of the process owning `auto-sync-execution.lock` | A sync is running right now (detached worker, foreground `snp sync`, explicit `--sync`, or cron). | `:345-354` |
| 3 | `PendingAttentionRequired` | — | Pending work exists **and** the last attempt set `attention_required` (deferred config error, local failure, or ≥ 3 consecutive internal failures). Needs operator action. | `:356-359` |
| 4 | `PendingRetryBackoff { next_attempt_at_unix_ms }` | when the next attempt becomes eligible | Pending work exists, no attention flag, and the durable backoff window has not elapsed. | `:360-367` |
| 5 | `PendingAwaitingScheduling` | — | Pending work exists and is neither in backoff nor flagged for attention — it is waiting to be scheduled or is being scheduled now. | `:368` |
| 6 | `ConfiguredAndCurrent` | — | Sync is configured, no pending work, no live worker: nothing to do. | `:372` |
| 7 | `ConfiguredAutoSyncDisabled` | — | Sync is configured but auto-sync is off. Manual sync still works. | `:373-375` |
| 8 | `NotConfigured` | — | No sync account, or `sync.toml` failed to load. Both `NotConfigured` and `LoadFailed` configuration states map here. | `:376-377` |

### Configuration state

`SyncConfigurationState` has four variants (`src/status_snapshot.rs:38-44`):
`Configured`, `ConfiguredAutoSyncDisabled`, `NotConfigured`, `LoadFailed`.
`LoadFailed` is produced when `config::load_sync_settings()` returns an error
(`src/status_snapshot.rs:201-215`) and is reported separately as the
`CONFIG_LOAD_FAILED` diagnostic even though the top level collapses to
`NotConfigured`.

## Pending state view

`PendingStateView` has four variants (`src/status_snapshot.rs:64-77`):

| Variant | Payload | Meaning | Produced at |
|---------|---------|---------|-------------|
| `Pending { generation, created_at_unix_ms }` | generation + creation time | A pending marker exists and is well-formed. | `src/status_snapshot.rs:223-226` |
| `None` | — | No pending marker (`PendingError::NotFound`). | `:227` |
| `Corrupt { reason_code }` | machine-readable reason | The marker exists but failed to parse, failed its CRC32 integrity check, or is otherwise unusable. Reason codes: `integrity_or_parse_failure` (integrity mismatch **or** an explicit corrupt marker), `deserialization_failure`, `serialization_failure`. | `:228-232`, `:236-238`, `:242-244` |
| `Inaccessible { reason_code }` | machine-readable reason | The marker could not be read. Reason codes: `io_error`, `lock_contention`, `scheduling_failure`. | `:233-235`, `:239-241`, `:245-247` |

Reason codes come from the `PendingError` variant
(`src/status_snapshot.rs:217-249`), **not** from the variant names — the
mapping is not one-to-one: `IntegrityMismatch` and `Corrupted` share the
single reason code `integrity_or_parse_failure`.

**Audit note.** Only `Deserialize`, `IntegrityMismatch`, `Corrupted`, and `Io`
are reachable from a *read*. `read_state_from_dir` → `parse` → `parse_on_disk`
can only produce those plus `NotFound` (`src/auto_sync/pending.rs:456-463`,
`:182-215`). The `Serialize`, `Lock`, and `Scheduling` arms in the snapshot
mapping are defensive branches: `Serialize` and `Scheduling` are write-path
errors, and `Lock` is a write-side acquisition error.

`pending_state_view` is **side-effect-free**: it reads the marker without
taking the pending transaction lock, so it can observe a marker mid-rewrite.
It never mutates or clears anything.

## Attempt state view

`AttemptStateView` has six variants (`src/status_snapshot.rs:84-92`):
`NeverAttempted`, `Succeeded`, `RetryScheduled`, `Deferred`, `AttentionRequired`,
`Corrupt`.

`attempt_state_view(status)` (`src/status_snapshot.rs:251-280`) evaluates in
this order:

| Condition | Variant |
|-----------|---------|
| `last_attempt_at_unix_ms == 0` | `NeverAttempted` |
| `attention_required == true` | `AttentionRequired` |
| `last_success_at_unix_ms >= last_attempt_at_unix_ms` and `> 0` | `Succeeded` |
| `next_attempt_at_unix_ms > now` | `RetryScheduled` |
| `next_attempt_at_unix_ms > 0` (but ≤ now) | `Deferred` |
| `last_success_at_unix_ms > 0` | `Succeeded` |
| `consecutive_failures > 0` | `RetryScheduled` |
| otherwise (attempt recorded, nothing interpretable) | `RetryScheduled` — the snapshot never claims success for an uninterpretable state |

`AttemptStateView::Corrupt` is **not** produced by `attempt_state_view`. It is
synthesised in `capture_snapshot()` when `status::read_status_typed` returns
`StatusRead::Corrupt` (`src/status_snapshot.rs:161-164`); a missing status file
synthesises `NeverAttempted` (`:165-168`).

## Execution (lock) state view

`ProcessStateView` has five variants (`src/status_snapshot.rs:107-114`):
`Idle`, `Live { pid, started_at_unix_ms }`, `DeadStale { pid }`, `Malformed`,
`Inaccessible`.

`execution_state_view` (`src/status_snapshot.rs:282-286`) inspects both lock
files and returns `(execution_lock, worker_lock)`. `inspect_execution_lock`
(`:288-308`) and `inspect_worker_lock` (`:310-330`) parse the lock file's TOML
metadata and probe liveness:

- Metadata parses and the PID is alive → `Live { pid, started_at_unix_ms }`.
- Metadata parses and the PID is provably gone → `DeadStale { pid }`.
- The file cannot be read (other than `NotFound`) → `Inaccessible`.
- The file is missing or empty → `Idle`.
- The file has content that is not valid metadata → `Malformed`.

**Lock metadata is diagnostic only.** Mutual exclusion comes from the kernel
(`flock` / `LockFileEx`); the TOML (`pid`, `started_at`, `nonce`, `purpose`) is
a stale-tolerant hint. `Drop` releases the kernel lock and never unlinks the
file, so a lock file with a dead PID is the normal steady state
(`src/process_file_lock.rs:190-200`). Liveness uses `kill(pid, 0)`: only
`ESRCH` proves absence — `EPERM` and unknown errno mean the process is alive
(`src/utils/process.rs:19-30`). On non-Unix an unknown PID is treated as alive
(conservative, non-stealing).

`DeadStale` means a recorded PID that is provably gone. `Malformed` means the
metadata could not be parsed. `Inaccessible` means the file could not be read.
Neither dead nor malformed metadata blocks a new acquirer — the next acquirer
overwrites it.

Note that the **worker lock is never acquired by production code**; it is
inspected here for diagnostics only. See [auto_sync.md](auto_sync.md) for the
full lock inventory and acquisition order.

## The durable attempt status file (`auto-sync-status.toml`)

Written by `auto_sync::status::write_status` (`src/auto_sync/status.rs:153-183`)
after every attempt, success or failure. Path: `status_path(state_dir)`
(`src/auto_sync/status.rs:91`), schema version `1`
(`src/auto_sync/status.rs:23`).

On-disk shape (TOML, `toml::to_string_pretty`, `serde` field names):

```toml
schema = 1
pending_generation = 7
last_attempt_generation = 7
last_attempt_at_unix_ms = 1736899800000
last_success_at_unix_ms = 1736899800000
last_result = "success"
last_failure_class = ""
consecutive_failures = 0
next_attempt_at_unix_ms = 0
executor_exit_code = 0
attention_required = false
message = "canonical sync acknowledged; pending cleared"
config_fingerprint = 11089481267477145603
integrity = 2989431123
```

Field semantics (`AutoSyncStatus`, `src/auto_sync/status.rs:34-67`):

| Field | Meaning |
|-------|---------|
| `pending_generation` | The pending generation this status corresponds to |
| `last_attempt_generation` | Generation of the last completed attempt |
| `last_attempt_at_unix_ms` / `last_success_at_unix_ms` | Attempt / last-success timestamps (0 = never) |
| `last_result` | `"success"`, or `"{class}_failure"` (e.g. `transient_failure`) |
| `last_failure_class` | `FailureClass::as_code()` — `transient`, `configuration`, `local_failure`, `internal` (cleared on success) |
| `consecutive_failures` | Failures without an intervening success |
| `next_attempt_at_unix_ms` | When the next attempt becomes eligible; `0` = no backoff |
| `executor_exit_code` | Exit code of the attempt; the detached worker uses `-1` ("no executor process") |
| `attention_required` | Operator attention needed |
| `message` | Bounded, sanitised, secret-free human message (≤ 512 chars, control characters stripped, secrets redacted — `src/auto_sync/status.rs:291-298`) |
| `config_fingerprint` | Secret-free hash of the sync-relevant config (`server_url`, `enabled`, `auto_sync`, `sync_direction`, `credential_revision`); used to release a deferred failure when the config changes (`src/auto_sync/status.rs:319-329`). Computed with `DefaultHasher`, so the value is **not stable across Rust toolchain releases** — only the equality comparison matters. |
| `integrity` | `u32` CRC32 over every behaviour-driving field **except** `message` (informational only) (`src/auto_sync/status.rs:186-216`) |

Write properties: atomic (temp file + rename + directory fsync,
`DurableUserData`), bounded to 8192 bytes, 0o600 on Unix, and **write failure
is logged but never propagated** — status is best-effort and a failed status
write never clears pending (`src/auto_sync/status.rs:8-10`, `:149-152`).

`record_success` (`src/auto_sync/status.rs:219-247`) resets
`consecutive_failures` to 0, clears `next_attempt_at_unix_ms`, clears
`attention_required`, and clears `last_failure_class`. `record_failure`
(`:250-285`) writes the class, the count, the next-attempt timestamp, and sets
`attention_required` when the class is deferred (`Configuration`), is
`LocalFailure`, or is `Internal` with `consecutive_failures >= 3`.

Both tolerate a corrupt status file: they log a warning and start from
`AutoSyncStatus::default()` rather than failing (`src/auto_sync/status.rs:225-234`,
`:257-266`).

### `consecutive_failures` and backoff state

`consecutive_failures` and `next_attempt_at_unix_ms` are the **durable retry
state**; they are what make a failing auto-sync back off instead of hot-looping.

- The worker increments the durable count on every failed cycle and computes
  the next eligible time with `transient_backoff(consecutive)`
  (`src/auto_sync/worker.rs:519-541`, `:544-549`). The count is read from the
  status file, `+1`, or `1` when the status is missing or corrupt.
- `schedule_sync` reads `next_attempt_at_unix_ms` and returns
  `DeferredUntil(t)` while `now < t` — except for `Caller::ExplicitRetry`,
  which bypasses the wait (`src/auto_sync/schedule.rs:83-93`).
- Backoff schedule (`src/auto_sync/policy.rs:146-158`): 5 s → 15 s → 30 s →
  60 s → exponential, capped at 15 min, plus 0–20 % jitter.
- `attention_required` short-circuits the schedule: `schedule_sync` returns
  `RequiresAttention(class)` for a non-retryable disposition instead of
  spawning (`src/auto_sync/schedule.rs:98-122`).
- A **configuration** change releases a deferral: the stored
  `config_fingerprint` is compared with the current one, and a mismatch clears
  `attention_required` and resets `consecutive_failures` to 0 so a new attempt
  is permitted (`src/auto_sync/status.rs:338-378`). That release path takes a
  **non-blocking `try_acquire`** of the execution lock first, so it never
  competes with a running worker (`src/auto_sync/status.rs:339-345`).
- `snp sync clear-failure` (alias `f`) resets `attention_required`,
  `consecutive_failures`, and `next_attempt_at_unix_ms` **without discarding
  pending intent** (`src/commands/sync_cmd.rs:611-634`).
- `snp sync discard-pending [--force] [--generation N]` removes the pending
  intent instead (`src/commands/sync_cmd.rs:636-705`).

### How status relates to the pending marker

The two artifacts are deliberately independent:

| | `auto-sync-pending.toml` | `auto-sync-status.toml` |
|---|---|---|
| Meaning | Durable *intent*: "there is work to sync" | Durable *history*: "here is what happened" |
| Written by | `pending::record_pending_mutation` (the only generation incrementer) | `status::record_success` / `record_failure` |
| Cleared by | `clear_if_generation_matches` — generation-scoped, automatic | Never cleared by the worker; rewritten on each attempt |
| On failure | **Preserved** — the work is still owed | Updated with the class, count, and backoff |
| Authoritative for | Whether sync work exists | Whether/when to retry, and whether a human must intervene |

Consequences visible in the snapshot:

- Pending work with a healthy last attempt → `PendingAwaitingScheduling`.
- Pending work with `attention_required` → `PendingAttentionRequired`.
- Pending work inside the backoff window → `PendingRetryBackoff`.
- **No pending work but a failed last attempt** → the top level falls through
  to `ConfiguredAndCurrent` (or `ConfiguredAutoSyncDisabled` / `NotConfigured`);
  the failure is still visible in `attempt` and in the diagnostics, but the
  top level does not claim pending work exists.
- A corrupt pending marker → `CorruptOrInaccessible`, regardless of the
  attempt history.

The pending marker is the source of truth for *work*; the status file is
best-effort and its write failure never clears pending.

## The `preflight_check` / `PreflightError` path

The worker's preflight is what makes corrupt or unreadable pending state
*visible* rather than silently ignored. It is the same read path the status
snapshot uses, but with a typed outcome instead of a view.

```rust
pub enum PreflightError {
    NothingToDo,
    Failed { class: FailureClass, message: String },
}
```

(`src/auto_sync/worker.rs:333-342`), returned by
`preflight_check(state_dir, observed) -> Result<PendingState, PreflightError>`
(`src/auto_sync/worker.rs:382-404`).

| Condition | Preflight result | Worker behaviour | Effect on status |
|-----------|------------------|------------------|-------------------|
| Marker removed between the debounce and the preflight | `NothingToDo` | Clean exit, code 0 | No status write; the previous attempt record stays |
| Generation rolled back to the cleared-and-recreated case (generation exactly `1` **and** a strictly newer `created_at_unix_ms`) | `Ok(recreated)` | The recreated marker is adopted; the cycle continues with the new generation | Normal attempt record for the new generation |
| Generation rolled back any other way | `Failed { class: Internal }` | `record_sync_failure` | `consecutive_failures` +1, backoff armed, `last_result = "internal_failure"` |
| Marker unreadable (I/O, lock) | `Failed { class: Transient }` | `record_sync_failure` | `consecutive_failures` +1, backoff armed, `last_result = "transient_failure"` |
| Marker unparseable / CRC mismatch | `Failed { class: LocalFailure }` | `record_sync_failure` | `attention_required = true` — operator action needed |

Classification is `failure_class_for_pending_error`
(`src/auto_sync/worker.rs:357-374`): `Io`/`Lock` → `Transient`;
`Deserialize`/`IntegrityMismatch`/`Corrupted` → `LocalFailure`; anything else →
`Internal`. The generation attributed to the failure is
`UNKNOWN_PENDING_GENERATION` (`0`) when the marker's own generation cannot be
trusted (`src/auto_sync/worker.rs:380`).

**This is the defect the two audited commits fixed.** Before commit `8d5c209`,
every one of these conditions was reported as `NothingToDo` — a clean exit
with code 0. Corruption therefore never incremented `consecutive_failures`,
never engaged backoff, and never appeared in `snp status` as an attention
item; the next mutation simply re-spawned a worker that failed the same way.
Now the same conditions reach `record_sync_failure`
(`src/auto_sync/worker.rs:131-144`), which writes the status file, arms
backoff, and sets `attention_required` for local failures — and the snapshot
above reflects that.

The **pending read error** on the worker's *first* read is handled the same
way (`src/auto_sync/worker.rs:87-104`): a read failure records a failure
against `UNKNOWN_PENDING_GENERATION` rather than being treated as "no work".

For the snapshot side, the same underlying read produces `PendingStateView::
Corrupt` / `Inaccessible` and the `PENDING_CORRUPT` / `PENDING_INACCESSIBLE`
diagnostics, which is why `snp status` and `snp doctor` show a corrupt marker
even when no worker has run yet.

## Diagnostic codes

`collect_diagnostics` (`src/status_snapshot.rs:381-510`) emits **12** codes.
Each is produced only in `status_snapshot.rs` and consumed only in
`doctor_cmd.rs` (dotted renames) and `status_cmd.rs` (rendering).

| Code | Severity | Produced when | Produced at | Doctor code (consumed at `src/commands/doctor_cmd.rs:70-134`) |
|------|----------|---------------|-------------|------------------------------------------------------------------------|
| `CONFIG_LOAD_FAILED` | Error (Warning under `--compatibility`) | `sync.toml` failed to load | `src/status_snapshot.rs:386-391` | `sync.config.load_failed` (`doctor_cmd.rs:72-75`) |
| `NOT_CONFIGURED` | Info | No sync account configured | `:394-399` | `sync.config.not_configured` (`doctor_cmd.rs:76-79`) |
| `PENDING_CORRUPT` | Error | Pending marker failed to parse or failed its CRC32 | `:406-411` | `sync.pending.corrupt` (`doctor_cmd.rs:80`) |
| `PENDING_INACCESSIBLE` | Warning | Pending marker could not be read | `:414-419` | `sync.pending.inaccessible` (`doctor_cmd.rs:81-84`) |
| `EXECUTION_LOCK_STALE` | Warning | `auto-sync-execution.lock` names a provably dead PID | `:426-431` | `sync.execution.dead_stale` (`doctor_cmd.rs:85-88`) |
| `EXECUTION_LOCK_MALFORMED` | Warning | Execution lock metadata unparseable | `:434-439` | `sync.execution.malformed` (`doctor_cmd.rs:89-92`) |
| `EXECUTION_LOCK_INACCESSIBLE` | Warning | Execution lock file unreadable | `:442-447` | `sync.execution.malformed` (`doctor_cmd.rs:93-96`) |
| `WORKER_LOCK_STALE` | Warning | `auto-sync-worker.lock` names a provably dead PID | `:454-459` | `sync.worker_lock.dead_stale` (`doctor_cmd.rs:97-100`) |
| `WORKER_LOCK_MALFORMED` | Warning | Worker lock metadata unparseable | `:462-467` | `sync.worker_lock.malformed` (`doctor_cmd.rs:101-104`) |
| `WORKER_LOCK_INACCESSIBLE` | Warning | Worker lock file unreadable | `:470-475` | `sync.worker_lock.malformed` (`doctor_cmd.rs:105-108`) |
| `ATTENTION_REQUIRED` | Error | The last attempt set `attention_required` | `:482-490` | `sync.attention.*` (`doctor_cmd.rs:113-132`) |
| `STATUS_CORRUPT` | Error | `auto-sync-status.toml` failed its CRC32 check | `:493-498` | `sync.status.corrupt` (`doctor_cmd.rs:109-112`) |

Notes for auditors:

- The `Inaccessible` lock codes deliberately **collapse onto the `malformed`
  doctor code** — the doctor vocabulary has no separate "inaccessible" lock
  code.
- The diagnostics are **sorted by severity, then by code**
  (`src/status_snapshot.rs:503-507`), so `Error` entries come first.
- `ATTENTION_REQUIRED` is mapped by **message substring**, not by a structured
  field: `authentication` → `sync.attention.authentication`, `configuration` →
  `sync.attention.configuration`, `credential` →
  `sync.attention.credential_store`, `conflict` → `sync.attention.conflict`,
  `partial` → `sync.attention.partial`, `local` →
  `sync.attention.local_persistence`, and the fallback is
  `sync.attention.configuration` (`src/commands/doctor_cmd.rs:113-132`).
  **In practice the snapshot only reaches two of those six distinct codes.** The
  message is built as `"Sync requires attention: {last_failure_class}"`
  (`src/status_snapshot.rs:485-488`) and `last_failure_class` is always one of
  the four `FailureClass::as_code()` values (`transient`, `configuration`,
  `local_failure`, `internal`). So:
  `configuration` → `sync.attention.configuration`;
  `local_failure` → `sync.attention.local_persistence` (matches the `local`
  keyword);
  `transient` and `internal` → `sync.attention.configuration` (fallback).
  The `authentication`, `credential_store`, `conflict`, and `partial` doctor
  codes are **unreachable via the snapshot path** — they can only be produced by
  another caller of the doctor report.
- The doctor report always carries a summary entry `compat.sync.checked`
  (Info) whose message is the debug-formatted `TopLevelSyncState`
  (`src/commands/doctor_cmd.rs:168-177`).
- Under `--compatibility`, `CONFIG_LOAD_FAILED` is downgraded from Error to
  Warning (`src/commands/doctor_cmd.rs:136-148`).

## Rendering (`snp status`)

`status_cmd::run` (`src/commands/status_cmd.rs`) prints human text by default
and `serde_json::to_string_pretty(&snapshot)` under `--json`.

Human text (example):

```text
Local: 2 libraries, 42 snippets; primary=snippets
Sync: pending
Pending generation: 1
Last attempt: transient at 2025-01-15 10:30:00
Next retry: 2025-01-15 10:35:00
Action: retry eligible at 2025-01-15 10:35:00
Logs: /Users/user/.local/share/snp/logs
```

- The `Sync:` line is `format_sync_line` (`src/commands/status_cmd.rs:101-115`),
  one fixed string per `TopLevelSyncState` variant:
  `corrupt or inaccessible state`, `active (pid=N)`, `attention required`,
  `pending retry`, `pending`, `current`, `auto-sync disabled`,
  `not configured`.
- `Pending generation:` comes from the pending view.
- `Last attempt:` renders `last_failure_class` and `last_attempt_at_unix_ms`.
- `Next retry:` renders `next_attempt_at_unix_ms` when it is in the future.
- `Action:` is the operator-facing next step from `format_action`
  (`src/commands/status_cmd.rs:117-136`): `run \`snp sync retry\` to retry now`
  for an attention state, `retry eligible at <local time>` inside a backoff
  window, and `run \`snp sync repair\` to diagnose` for a
  corrupt/inaccessible state. Every other top-level state has no action line.
- `Logs:` is the default log directory (`<config>/logs`), a pure path join —
  the directory is not created by `snp status` (`src/logging.rs:72-74`).

JSON output mirrors the snapshot exactly; enums are serialised as unit
variant names (`"Pending"`, `"ConfiguredAndCurrent"`, `"Pending { … }"`), and
`PendingStateView`/`AttemptStateView`/`ProcessStateView` are externally tagged
so their payloads are nested objects.

## Read-only guarantees — and their limits

`snp status` never writes to the **pending marker**, the **lock files**, or
the **status file**, and never spawns a worker process. It does not take the
execution lock, the worker lock, or the pending transaction lock, so it can
run concurrently with a live sync and can observe a marker mid-rewrite.

It is **not** side-effect-free in the strict sense of "creates or removes no
files", because the snapshot reads local library state through the normal
loaders, and those loaders have best-effort corruption handling that writes
a backup next to the damaged file:

| Trigger | Side effect | Site |
|---------|-------------|------|
| A library TOML fails to parse | copies it to `<library>.toml.corrupt.bak` | `src/library/persistence.rs:216` |
| `libraries.toml` fails to parse | copies it to `libraries.toml.corrupt` | `src/library/manager.rs:62` |
| `sync.toml` fails its integrity check | copies it to `sync.toml.corrupt.bak` and returns defaults | `src/config/sync_settings.rs:515-540` |
| macOS legacy config directory detected | migrates the config directory | `src/library/manager.rs:45` → `src/utils/config.rs:166` |

These are the same "fail closed with a best-effort backup" paths used
everywhere else in the codebase (see [persistence.md](persistence.md)); they
fire only on already-damaged input. `snp status` itself never repairs or
rewrites the artifacts it reports on — use `snp repair`, `snp sync repair`,
or `snp validate` for that.

---

## Key Files

| File | Subject |
|------|---------|
| `src/status_snapshot.rs` | The snapshot types, `capture_snapshot`, `derive_top_level`, `collect_diagnostics` |
| `src/commands/status_cmd.rs` | `snp status` CLI: human and JSON rendering |
| `src/auto_sync/status.rs` | Durable `AutoSyncStatus`, `record_success`/`record_failure`, config fingerprint, deferral release |
| `src/auto_sync/pending.rs` | Durable pending marker and `PendingError` (the read side the snapshot depends on) |
| `src/auto_sync/pending_lock.rs` | Pending transaction lock (not taken by the snapshot) |
| `src/auto_sync/execution_lock.rs` | Execution / worker lock files the snapshot inspects |
| `src/auto_sync/worker.rs` | `preflight_check` and `PreflightError` — the typed failure path the status file records |
| `src/auto_sync/schedule.rs` | `schedule_sync` — the consumer of the durable backoff state |
| `src/commands/doctor_cmd.rs` | Snapshot → dotted diagnostic-code mapping (`map_snapshot_diagnostic`) |
| `src/commands/sync_cmd.rs` | `snp sync clear-failure` / `discard-pending` / `repair` — the operator recovery paths |
| `src/utils/process.rs` | `is_process_alive` — the `kill(pid, 0)` liveness probe |
| `src/process_file_lock.rs` | Kernel lock primitive; `Drop` releases without unlinking |
| `architecture/commands/status_cmd.md` | Command-level status surface |
| `.skills/transactions-and-auto-sync.md` | Skill-level invariants for this subsystem |

---

## Invariants

1. `capture_snapshot` is read-only with respect to the **auto-sync control
   artifacts**: it never writes the pending marker, the lock files, or
   `auto-sync-status.toml`, and never spawns a worker.
2. The snapshot takes **no lock** — it is safe to run concurrently with a live
   sync and never blocks a worker.
3. `TopLevelSyncState` is derived, never stored; the precedence order is
   fixed (corrupt → live → attention → backoff → pending → configuration).
4. A corrupt or unreadable pending marker always outranks every other
   signal, including a live worker.
5. The status file is **best-effort**: a write failure is logged, never
   propagated, and never clears pending intent.
6. `consecutive_failures` and `next_attempt_at_unix_ms` are the durable retry
   state; the schedule reads them and the worker writes them.
7. `attention_required` is set only for a deferred configuration failure, a
   local failure, or an internal failure at `consecutive_failures >= 3`.
8. The status file is secret-free: the message is truncated to 512 chars,
   stripped of control characters, and passed through `redact_secrets`; the
   config fingerprint hashes only non-secret structural inputs.
9. A corrupt status file degrades to a default status on write and to
   `AttemptStateView::Corrupt` on read — it never blocks a retry.
10. Lock metadata is diagnostic only; kernel ownership is authoritative, and
    only `ESRCH` proves a PID is gone.
11. The pending marker is the source of truth for *work*; the status file is
    the source of truth for *history and retry state*.
12. `snp status` never repairs what it reports on.

---

## Gotchas

- **`AttemptStateView::Corrupt` comes from `capture_snapshot`, not from
  `attempt_state_view`.** A missing status file yields `NeverAttempted`, not
  `Corrupt`. `src/status_snapshot.rs:158-169`.
- **The `ATTENTION_REQUIRED` → doctor code mapping is substring-based**, and the
  snapshot's message only ever contains a `FailureClass` code. In practice only
  `sync.attention.configuration` and `sync.attention.local_persistence` are
  reachable; `authentication`, `credential_store`, `conflict`, and `partial` are
  not. `src/commands/doctor_cmd.rs:113-132`, `src/status_snapshot.rs:482-490`.
- **`LoadFailed` and `NotConfigured` configuration states both map to the
  top level `NotConfigured`** — the distinction only survives in the
  `sync.configuration` field and the `CONFIG_LOAD_FAILED` diagnostic.
  `src/status_snapshot.rs:371-378`.
- **Only `ESRCH` proves a PID is dead.** `EPERM` and unknown errno mean the
  process is alive. `src/utils/process.rs:28-30`.
- **A leftover lock file is normal.** `Drop` releases the kernel lock without
  unlinking, so `auto-sync-execution.lock` and `auto-sync-worker.lock`
  persist with the last owner's metadata between runs.
  `src/process_file_lock.rs:190-200`.
- **The worker lock is inspected but never held.** Nothing in production
  acquires `auto-sync-worker.lock`; a `Live` reading there is diagnostic
  noise, not an authoritative mutual-exclusion signal.
- **`snp status` can create a `*.corrupt.bak` file** as a side effect of
  reading a damaged library, `libraries.toml`, or `sync.toml`. It never
  repairs them.
- **The snapshot's `pending` view is unlocked.** It can observe a marker
  mid-rewrite and may report a transient `Inaccessible` while another process
  is replacing it.
- **`executor_exit_code == -1` means "detached helper"**, not a crash — the
  worker has no executor process to report a code for.
  `src/auto_sync/worker.rs:519-541`.
- **`config_fingerprint` is a `DefaultHasher` digest**, so its numeric value
  changes across Rust toolchain releases. Only the *comparison* against the
  stored value is meaningful; never persist or display it as a stable id.
  `src/auto_sync/status.rs:319-329`.
- **`snp sync clear-failure` does not touch the pending marker.** It resets
  the retry/attention state only; the work is still owed and the next
  scheduled attempt will pick it up. `src/commands/sync_cmd.rs:611-634`.
- **The status file's CRC32 deliberately excludes `message`.** Editing a
  message does not invalidate the record.
  `src/auto_sync/status.rs:186-216`.

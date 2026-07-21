# Phase 04A: Operational Visibility and Recovery

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-04-operational-visibility-recovery.md
```

Implement against commit `ff506f5934957c4fd989224a6f0e0cf10f907567` or a descendant that preserves the Phase 01–03 closure invariants.

## Purpose

Turn the corrected synchronization state machine into an understandable and recoverable terminal experience without adding a daemon, notification service, background dashboard, or always-running helper.

The implementation already persists typed pending intent, durable attempt status, retry eligibility, failure classification, configuration fingerprints, and process locks. This phase creates one read-only state projection over those artifacts, exposes stable human and machine output, integrates the projection with doctor diagnostics, and adds narrowly scoped recovery commands.

## Required outcomes

1. A user can answer whether local changes are current, pending, retrying, blocked, failed, disabled, unconfigured, or corrupt.
2. Status inspection is strictly local and read-only.
3. Machine-readable status is stable, deterministic, and uncontaminated.
4. Recovery commands have distinct semantics and cannot silently discard valid pending intent.
5. Corrupt control state is reported as corruption, never as idle or synchronized.
6. Detached failures are discoverable without unrelated command output pollution.
7. Logs and diagnostics remain bounded, private, and secret-free.
8. Linux, macOS, and Windows expose equivalent state semantics.

## Non-goals

Do not add:

- background desktop notifications;
- a resident tray process;
- a synchronization daemon;
- remote status polling for ordinary status output;
- automatic destructive repair;
- a full TUI administration screen;
- telemetry or hosted observability.

## Current implementation surfaces

Build on, rather than duplicate:

- `src/auto_sync/pending.rs` — generation-bearing intent and typed corruption;
- `src/auto_sync/status.rs` — `StatusRead`, durable attempt state, CRC32 integrity;
- `src/auto_sync/schedule.rs` — current scheduling eligibility and deferral logic;
- `src/auto_sync/execution_lock.rs` — execution ownership and native liveness;
- `src/auto_sync/lock.rs` and `pending_lock.rs` — coordination artifacts;
- `src/config.rs` — typed sync settings loading and policy bounds;
- `src/commands/doctor_cmd.rs` — existing compatibility diagnostics;
- `src/commands/sync_cmd.rs` — current foreground sync/config command surface;
- existing tracing/logging infrastructure.

Do not create a second interpretation of pending, status, lock, or policy semantics in command rendering code.

---

## Workstream A — Canonical read-only status projection

Create one internal projection module, recommended location:

```text
src/status_snapshot.rs
```

or:

```text
src/commands/status_snapshot.rs
```

The module must be usable by CLI status, doctor, optional TUI indicator, and tests.

Recommended model:

```rust
pub struct StatusSnapshot {
    pub schema: u32,
    pub generated_at_unix_ms: u64,
    pub config_root: PathBuf,
    pub local: LocalSummary,
    pub sync: SyncSummary,
    pub pending: PendingSummary,
    pub attempt: AttemptSummary,
    pub execution: ExecutionSummary,
    pub diagnostics: Vec<StatusDiagnostic>,
}
```

Suggested typed states:

```rust
pub enum SyncConfigurationState {
    NotConfigured,
    Configured,
    ConfiguredAutoSyncDisabled,
    LoadFailed,
}

pub enum PendingStateView {
    None,
    Pending { generation: u64, created_at_unix_ms: u64 },
    Corrupt { reason_code: String },
    Inaccessible { reason_code: String },
}

pub enum AttemptStateView {
    NeverAttempted,
    Succeeded,
    RetryScheduled,
    AttentionRequired,
    Deferred,
    Corrupt,
}

pub enum ProcessStateView {
    Idle,
    Live { pid: u32, started_at_unix_ms: u64 },
    DeadStale { pid: u32 },
    Malformed,
    Inaccessible,
}
```

Projection rules:

- use typed readers from the underlying modules;
- preserve missing, corrupt, inaccessible, live, dead, and malformed distinctions;
- never reclaim a lock while projecting state;
- never rewrite a file while projecting state;
- never access the network;
- avoid keychain prompts; credential presence checks must be noninteractive or reported as unknown;
- derive a top-level sync state through one documented precedence table;
- deterministic diagnostics ordered by severity, code, and stable tie-breaker;
- path rendering must preserve non-UTF-8 paths in structured form or return an explicit encoding diagnostic;
- status generation must not schedule recovery.

### Required top-level state precedence

Document and test the precedence used for concise output. Recommended order:

1. corrupt or inaccessible pending/control state;
2. live execution;
3. pending with attention required;
4. pending with retry backoff;
5. pending awaiting debounce/scheduling;
6. configured and current;
7. configured with auto-sync disabled;
8. not configured.

A prior successful attempt must not make the state `current` while a newer pending generation exists.

---

## Workstream B — User-facing command surface

Use one canonical command family. Preferred surface:

```bash
snp status
snp status --json
snp status --sync-only
```

A nested `snp sync status` alias may be added only if it delegates to the same implementation and schema. Avoid two independent status renderers.

### Human output

Human output should be compact by default and include:

- local config root;
- primary library;
- library count;
- snippet count;
- sync configuration state;
- automatic sync enabled/disabled;
- concise synchronization state;
- pending generation and age when present;
- last attempted generation;
- last result/failure class;
- last success timestamp;
- next retry eligibility;
- attention-required flag;
- live execution owner when present;
- one-line remediation for actionable failures.

Example:

```text
Local: 4 libraries, 327 snippets; primary=work
Sync: pending retry
Pending generation: 42
Last attempt: transient timeout at 2026-07-21 09:14:03
Next retry: 2026-07-21 09:19:03
Action: run `snp sync retry` to retry now
```

Use absolute timestamps in machine output and local-time rendering only for human output. Include relative age as supplemental information, not the sole timestamp.

### JSON output

Recommended schema:

```json
{
  "schema": 1,
  "generated_at_unix_ms": 0,
  "local": {},
  "sync": {},
  "pending": {},
  "attempt": {},
  "execution": {},
  "diagnostics": []
}
```

Requirements:

- stable snake_case fields;
- deterministic array ordering;
- no ANSI;
- no logs or warnings on stdout;
- explicit enum codes, not human strings as the only representation;
- Unix milliseconds for timestamps;
- full precision generation numbers;
- no secrets, command text, descriptions, tags, output, ciphertext, raw server bodies, or credential-bearing URLs;
- additive changes allowed within schema 1; breaking changes require schema increment.

### Exit behavior

Recommended status exit contract:

- `0`: snapshot produced, including ordinary pending/retry/disabled states;
- nonzero: snapshot could not be produced at all due to a fundamental local failure;
- corruption is represented in output and may optionally use a documented diagnostic exit mode under `--strict`;
- `--json` always emits valid JSON when any snapshot can be constructed.

Do not make “pending” an error exit by default.

---

## Workstream C — Doctor integration

Refactor doctor sync checks to consume the canonical projection and shared typed probes rather than independently parsing files.

Add focused invocation if useful:

```bash
snp doctor --sync
snp doctor --sync --report json
```

Retain existing compatibility mode behavior unless explicitly deprecated.

Required diagnostic codes include:

```text
sync.config.not_configured
sync.config.load_failed
sync.config.invalid_bounds
sync.credentials.unavailable
sync.pending.present
sync.pending.corrupt
sync.pending.inaccessible
sync.status.corrupt
sync.status.generation_mismatch
sync.execution.live
sync.execution.dead_stale
sync.execution.malformed
sync.worker_lock.live
sync.worker_lock.dead_stale
sync.permissions.insecure
sync.path.symlink
sync.path.non_regular
sync.retry.active
sync.attention.authentication
sync.attention.configuration
sync.attention.credential_store
sync.attention.conflict
sync.attention.partial
sync.attention.local_persistence
sync.clock.future_timestamp
sync.temp.orphaned
```

Doctor rules:

- read-only by default;
- no lock reclamation;
- no credential display;
- no network health check unless a separate explicit option requests it;
- no detached scheduling;
- every error diagnostic includes a stable remediation code or command where applicable;
- JSON diagnostics share severity and code types with `snp status`.

---

## Workstream D — Explicit retry

Add:

```bash
snp sync retry
```

Semantics:

- foreground operation;
- may bypass stored time-based backoff;
- cannot bypass execution mutual exclusion;
- cannot bypass corrupt pending/config/status validation;
- uses the canonical sync operation and configured direction unless an explicit direction override is supplied;
- captures pending generation before sync;
- clears only the matching generation after real success;
- records durable success/failure using the exact attempted generation;
- returns nonzero on failure;
- never starts a second detached worker merely because retry was invoked;
- if no pending exists, either perform a documented foreground comparison or return a typed no-pending result; choose one behavior and document it.

Required tests:

- bypass backoff;
- active execution lock refusal or bounded wait according to foreground policy;
- success clears matching generation;
- concurrent mutation remains pending;
- failure preserves pending;
- corrupt status does not get silently overwritten before explicit repair policy is applied;
- no duplicate worker spawn.

---

## Workstream E — Clear failure without discarding pending

Add:

```bash
snp sync clear-failure
```

Semantics:

- clears attention/backoff/result state only;
- preserves pending marker and generation byte-for-byte;
- records a local administrative event if the status schema supports it;
- does not claim current/success;
- requires no network access;
- refuses or requires repair when status is corrupt;
- does not schedule automatically unless an explicit `--retry` is also requested and documented.

This command is primarily for clearing a stale operator gate after an externally resolved issue. Successful sync should already clear failure state automatically.

---

## Workstream F — Generation-safe discard of pending intent

Add an advanced command:

```bash
snp sync discard-pending
snp sync discard-pending --force
```

Requirements:

- never deletes snippets or libraries;
- clearly states that it discards synchronization intent, not local data;
- reads and displays the observed generation;
- confirmation required on interactive terminals unless `--force`;
- machine/noninteractive invocation requires `--force` and preferably `--generation <n>`;
- conditional clear under the pending transaction lock;
- refuses if generation changed between observation and clear;
- refuses on corrupt/inaccessible pending state;
- records a durable `discarded_by_user` status with generation and timestamp;
- does not record success/current;
- no worker spawn;
- deterministic exit codes for cleared, missing, generation changed, corrupt, and inaccessible.

Required race test: a mutation inserted during confirmation must survive and cause discard refusal.

---

## Workstream G — Conservative sync-control repair

Add:

```bash
snp sync repair --dry-run
snp sync repair --apply
```

Scope only synchronization control artifacts in this phase:

- pending marker;
- status file;
- execution lock;
- worker lock if retained;
- transaction locks;
- orphaned unique temporary files.

Safe automatic repairs:

- quarantine corrupt status and recreate an empty non-success status only when pending remains authoritative;
- remove dead, ownership-verifiably stale locks;
- quarantine malformed dead locks;
- remove orphaned temp files that match owned naming rules and are not live;
- repair restrictive permissions;
- preserve valid pending intent.

Never automatically:

- delete corrupt pending intent;
- invent a generation;
- mark current or successful;
- replace ambiguous live locks;
- rewrite sync configuration credentials;
- contact the server.

Every applied repair must:

- support dry-run with identical decision logic;
- create a timestamped quarantine copy before destructive change;
- use safe path and file-type checks;
- emit structured actions and skipped reasons;
- be idempotent;
- remain bounded in artifact count and size.

Corrupt pending should produce a manual-decision diagnostic with explicit options, not silent repair.

---

## Workstream H — Structured, bounded detached logging

Standardize worker/executor event logging through existing tracing.

Required fields:

- component;
- pid;
- state directory identifier without exposing sensitive path segments unnecessarily;
- pending generation;
- attempted generation;
- scheduling caller and decision;
- direction;
- phase;
- duration milliseconds;
- exit code and failure class;
- timeout signal/termination result;
- conditional-clear result.

Prohibited fields:

- snippet command/description;
- expanded variables;
- tags/output;
- API key or key derivative;
- encryption key/derived key;
- authorization metadata;
- raw response body;
- full credential-bearing URL.

Add:

- bounded rotation/retention;
- 0600-style Unix permissions and appropriate Windows ACL expectations;
- a documented log location surfaced by status;
- explicit debug enablement;
- sanitized failure-artifact collection for CI;
- sentinel-secret tests.

Remove any temporary unconditional `eprintln!` instrumentation.

---

## Workstream I — Optional concise TUI indicator

This is subordinate to the CLI status command.

Allowed states:

```text
current
pending
retrying
attention
disabled
unknown
```

Requirements:

- local read only;
- no server or keychain access during render;
- no scheduling;
- no startup blocking;
- unknown/corrupt is distinct from current;
- use existing theme semantics;
- one compact line or badge, not a new administration panel.

Defer this work if it materially complicates TUI state ownership. Deferral does not block Phase 04A if CLI status and doctor are complete.

---

## Workstream J — Attention notification policy

Optional: emit one concise stderr advisory on a later interactive invocation when attention-required state exists.

If implemented:

- never emit in JSON/CSV/raw/select/shell-completion modes;
- never emit from hidden worker/executor;
- durable rate limit;
- no sensitive failure text;
- one action: `Run snp status for details`;
- transient retry and ordinary pending states do not trigger it;
- no exit-code change for unrelated successful commands.

Explicit status remains the mandatory mechanism.

---

## Test plan

### Projection tests

- never configured;
- configured/current;
- configured/auto-sync disabled;
- pending awaiting debounce;
- pending with active execution;
- pending retry/backoff;
- attention-required for each class;
- corrupt pending;
- corrupt status;
- inaccessible artifacts;
- live/dead/malformed locks;
- pending generation newer than last success;
- deterministic diagnostics and JSON snapshots.

### Read-only guarantees

Use filesystem byte snapshots and a spawn counter to prove status/doctor:

- make no file changes;
- acquire no execution lock;
- spawn no worker/executor;
- contact no server;
- do not access interactive credentials.

### Recovery tests

- retry success/failure/concurrent mutation;
- clear-failure preserves pending bytes;
- discard confirmation and generation race;
- repair dry-run no mutation;
- repair quarantine and permission fixes;
- corrupt pending refusal;
- dead Windows/Unix lock handling;
- idempotent second repair.

### Output/security tests

- exact stdout for human and JSON modes;
- no logs/ANSI in JSON;
- no sentinel secrets or snippet payloads;
- bounded messages;
- non-UTF-8 path representation;
- Windows path escaping;
- stable exit mapping.

---

## Recommended implementation sequence

1. Introduce typed status projection and precedence tests.
2. Add `snp status` human and JSON renderers.
3. Refactor doctor sync diagnostics onto shared probes.
4. Add foreground retry.
5. Add clear-failure and generation-safe discard.
6. Add repair dry-run/quarantine/apply.
7. Standardize detached logging and retention.
8. Add optional TUI indicator/attention advisory only after mandatory surfaces are stable.
9. Reconcile help, README, USER_GUIDE, architecture, and JSON schema documentation.
10. Write `plans/snip-it-correctness-04a-status.md` with commit range and evidence.

## Required verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --test auto_sync_closure
cargo test --test integration
```

Add focused status/recovery integration targets and execute them on Linux, macOS, and Windows.

## Exit criteria

Phase 04A is complete only when:

- one canonical read-only projection drives status and doctor;
- status distinguishes current, pending, retrying, blocked, failed, disabled, unconfigured, and corrupt;
- JSON schema 1 is stable and uncontaminated;
- status and doctor perform no network or scheduling work;
- retry is foreground, lock-safe, and generation-safe;
- clear-failure cannot clear pending;
- discard-pending cannot remove a newer generation;
- repair is dry-run capable, conservative, quarantining, and idempotent;
- logs are bounded, private, structured, and secret-free;
- unknown/corrupt state never appears current;
- Linux, macOS, and Windows tests pass;
- documentation and help match the implemented command surface;
- no daemon or resident process was introduced.
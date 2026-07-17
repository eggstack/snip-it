# Phase 04: Operational Visibility and Recovery

## Purpose

Make synchronization state understandable and recoverable without requiring users to inspect private state files, enable test-only environment variables, or infer behavior from detached process activity.

The client should remain lightweight and terminal-first. This phase adds read-only status, focused diagnostics, explicit recovery operations, and bounded logging. It does not introduce a daemon, notification service, or background dashboard.

## Preconditions

Phase 03 must provide:

- typed failure classes;
- a durable secret-free status artifact;
- persisted backoff/next-attempt state;
- generation-aware pending state;
- clear foreground versus detached semantics.

## Product principles

1. Ordinary users need a concise answer to “are my changes synchronized?”
2. `status` reports state; it does not mutate state or trigger network work.
3. `doctor` performs deeper validation and explains inconsistencies.
4. Recovery actions are explicit, generation-safe, and conservative.
5. Detached failures are discoverable on the next intentional inspection without polluting unrelated stdout.
6. No status surface reveals commands, snippets, API keys, encryption keys, or sensitive URLs.

## Workstream A: Add a read-only `snp status` command

Recommended command surface:

```bash
snp status
snp status --json
snp status --sync
```

A separate `snp sync status` subcommand is also acceptable if it fits the existing command structure better. Prefer one canonical implementation and avoid duplicate renderers.

Human output should include, when available:

```text
Config root: ~/.config/snp
Primary library: work
Libraries: 4
Snippets: 327

Sync configuration: configured
Automatic sync: enabled
Sync state: pending retry
Pending generation: 42
Last attempted generation: 42
Last attempt: 2026-07-16 20:14:03
Last result: network timeout
Next eligible retry: 2026-07-16 20:19:03
Attention required: no
Execution: idle
```

The command should degrade gracefully when:

- no sync configuration exists;
- no pending/status files exist;
- status is corrupt;
- lock inspection fails;
- keyring availability cannot be determined;
- a migration is required.

It must not:

- acquire the execution lock for network work;
- spawn a worker;
- rewrite pending or status state;
- clear corruption;
- prompt for credentials;
- contaminate JSON stdout with logs or warnings.

## Workstream B: Define a stable status view model

Do not render directly from scattered filesystem probes. Build one internal read-only snapshot:

```rust
pub struct StatusSnapshot {
    pub config_root: PathBuf,
    pub primary_library: Option<String>,
    pub library_count: usize,
    pub snippet_count: usize,
    pub sync_configuration: SyncConfigurationState,
    pub auto_sync: AutoSyncState,
    pub pending: PendingView,
    pub last_attempt: Option<AttemptView>,
    pub next_retry_at: Option<u64>,
    pub attention_required: bool,
    pub worker: ProcessState,
    pub executor_lock: LockState,
    pub diagnostics: Vec<StatusDiagnostic>,
}
```

Requirements:

- stable JSON field names;
- versioned JSON schema or explicit compatibility policy;
- paths rendered without lossy assumptions where possible;
- timestamps represented in both human-readable form and machine-safe Unix milliseconds for JSON;
- unknown/corrupt states represented explicitly rather than converted to idle/current;
- bounded diagnostic messages;
- deterministic ordering.

## Workstream C: Integrate with `doctor`

Extend `doctor --compatibility` or introduce a focused sync diagnostic mode.

Doctor should validate:

- pending marker exists/absent and parses;
- pending integrity and schema;
- status integrity and schema;
- pending/status generation consistency;
- pending transaction lock ownership/liveness;
- worker lock ownership/liveness if retained;
- execution lock ownership/liveness;
- malformed or dead lock recovery eligibility;
- private file and directory permissions;
- server URL policy, including plaintext HTTP restrictions;
- credential presence and keyring access without printing values;
- configured direction;
- auto-sync policy bounds;
- last failure class and retry state;
- clock-skew or future timestamp anomalies;
- obsolete temp files and safe cleanup eligibility;
- internal state paths that are symlinks or non-regular files.

Doctor must remain read-only by default. Repair actions require explicit commands.

Human output should provide actionable remediation. JSON output should use stable codes such as:

```json
{
  "code": "sync.pending.corrupt",
  "severity": "error",
  "message": "Pending synchronization state failed integrity validation",
  "remediation": "Run snp sync repair --dry-run"
}
```

## Workstream D: Add explicit recovery commands

Recommended surface:

```bash
snp sync retry
snp sync clear-failure
snp sync discard-pending
snp sync repair --dry-run
```

Exact nesting may change, but semantics must remain distinct.

### Retry

- explicit foreground attempt;
- may bypass stored automatic backoff;
- still respects execution mutual exclusion;
- updates durable status;
- clears pending only on real success;
- returns nonzero on failure.

### Clear failure

- clears stale operator-attention/status fields only;
- does not clear pending intent;
- does not claim synchronization is current;
- requires no network operation;
- should normally be unnecessary after success because success resets status.

### Discard pending

- advanced destructive-to-intent operation;
- never deletes snippets;
- displays observed generation and consequence;
- requires confirmation unless `--force`;
- conditional clear against observed generation;
- refuses if generation changed during confirmation;
- records deliberate discard in status;
- returns nonzero on corruption or contention.

### Repair

- conservative handling of malformed state/locks/temp files;
- always support `--dry-run`;
- create a backup/quarantine copy before changing corrupt artifacts;
- never invent a synchronized generation;
- never discard valid pending intent;
- separate safe automatic repairs from ambiguous manual decisions.

## Workstream E: TUI synchronization indicator

Add only a compact, nonblocking indicator. Examples:

```text
sync: current
sync: pending
sync: retrying
sync: failed
sync: disabled
```

Requirements:

- reads local status only;
- never launches network work just to render;
- does not block TUI startup on keyring or server access;
- unknown/corrupt status is visually distinct from current;
- detailed explanation remains in `snp status`/doctor;
- theme integration uses existing semantic colors rather than adding a large UI subsystem.

If the TUI architecture makes this intrusive, defer the visual indicator but retain command-line status as mandatory.

## Workstream F: Structured detached logging

Use the existing tracing/logging infrastructure to provide bounded operational evidence.

Log fields should include:

- component: parent/worker/executor;
- process ID;
- pending generation;
- attempt generation;
- origin;
- direction;
- phase transition;
- duration;
- executor exit class;
- timeout/termination details;
- conditional-clear result;
- scheduling decision;
- failure class.

Never log:

- snippet command or description;
- tags/output payload;
- API key or encryption material;
- authorization metadata;
- raw server response bodies;
- secret-bearing URLs;
- unbounded user-provided error strings.

Add:

- bounded rotation or retention;
- restrictive permissions;
- clear location documented through status/doctor;
- failure-only CI artifact collection with secret-sentinel scans;
- supported debug verbosity through existing logging controls rather than permanent `eprintln!` instrumentation.

Remove or formally document test-only environment variables such as worker-log overrides. Production debug aids should have explicit security and retention behavior.

## Workstream G: Surface attention without stdout pollution

Consider a concise stderr warning on the next interactive invocation only when durable status indicates an attention-required failure. This is optional and must be conservative.

Rules if implemented:

- never emit on machine-readable JSON/CSV commands;
- never emit into `select`/shell integration stdout;
- rate-limit durably;
- do not repeat on every invocation;
- include a short action: `Run snp status for details`;
- do not expose sensitive error text;
- disabled or ordinary transient retry should not be treated as critical attention.

The mandatory visibility mechanism remains explicit status/doctor commands.

## Workstream H: Lock and process inspection

Status/doctor should distinguish:

- no lock;
- live lock owned by current process or another process;
- dead owner eligible for reclaim;
- malformed lock;
- inaccessible lock;
- ownership mismatch;
- unsupported liveness determination.

Do not reclaim locks merely while reading status. Repair or actual acquisition paths may reclaim according to existing ownership rules.

Windows inspection must use the platform-native implementation rather than Unix assumptions.

## Required tests

### Status rendering

- configured/current;
- configured/pending;
- pending retry with next time;
- attention-required authentication failure;
- auto-sync disabled with pending;
- unconfigured;
- corrupt pending;
- corrupt status;
- live/dead/malformed locks;
- stable JSON snapshots;
- no stdout contamination;
- no mutation or worker spawn.

### Recovery

- retry bypasses automatic backoff and preserves failure on error;
- successful retry clears matching pending and resets status;
- clear-failure preserves pending;
- discard-pending requires confirmation;
- discard uses conditional generation clear;
- concurrent mutation causes discard refusal;
- repair dry-run changes nothing;
- repair quarantines malformed artifacts safely;
- repair never removes valid pending.

### Security

- status and logs contain no sentinel secret or snippet payload;
- persisted messages are bounded and sanitized;
- log permissions are restrictive;
- JSON paths/errors are escaped correctly;
- symlinked status/log artifacts are handled safely;
- machine output remains deterministic.

### TUI

- indicator state mapping;
- unknown is not shown as current;
- TUI startup does not contact server;
- status read failure does not crash the interface.

## Documentation

Update:

- README quick operational guidance;
- USER_GUIDE status and recovery chapter;
- doctor documentation;
- command help and examples;
- log location/retention/privacy;
- sync troubleshooting;
- explicit pending-discard warning;
- JSON compatibility statement;
- architecture status-flow diagram.

## Recommended commit sequence

1. Add status snapshot model and read-only probes.
2. Add human and JSON status command.
3. Integrate detailed doctor diagnostics.
4. Add explicit retry and clear-failure actions.
5. Add generation-safe discard and repair dry-run/quarantine.
6. Add structured bounded worker/executor logging.
7. Add optional TUI indicator and attention warning.
8. Add complete security/output regression tests.
9. Reconcile docs and remove temporary diagnostic prints.

## Exit criteria

Phase 04 is complete only when:

- users can determine current/pending/retrying/failed/disabled state locally;
- status is read-only and never triggers network work;
- JSON output is stable and uncontaminated;
- doctor detects state, lock, permission, and corruption problems;
- retry, clear-failure, discard, and repair semantics are distinct and generation-safe;
- pending cannot be discarded accidentally;
- detached logs are bounded, private, structured, and secret-free;
- unknown/corrupt state is never displayed as current;
- platform tests pass;
- documentation provides actionable recovery instructions.

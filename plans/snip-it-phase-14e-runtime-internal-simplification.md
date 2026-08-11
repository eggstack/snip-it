# Phase 14E — Runtime and Internal Simplification

Status: IMPLEMENTED

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Required predecessor: Phase 14C command/control-flow consolidation

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Reduce internal machinery that does not buy meaningful reliability for a low-volume local CLI, while retaining the real invariants already established in Phase 13.

This phase focuses on three bounded areas:

1. remove duplicate auto-sync policy/config reads and obsolete forwarding APIs;
2. reuse the canonical atomic-write implementation for pending-marker persistence instead of maintaining a second platform-specific rename/write implementation;
3. replace the asynchronous audit-log queue/thread with direct synchronous appends, which are proportionate to the event rate and avoid dropped/late audit records.

Do not change sync retry policy, pending-generation semantics, execution-lock semantics, or transaction guarantees here.

## 2. Allowed files

Primary files:

```text
src/auto_sync/notification.rs
src/auto_sync/schedule.rs
src/auto_sync/pending.rs
src/auto_sync/pending_lock.rs
src/auto_sync/mod.rs
src/logging.rs
src/main.rs                  # only for startup service simplification after audit queue removal
src/utils/atomic.rs          # only if a small existing API extension is required
```

Tests should remain in the owning modules or existing auto-sync integration tests.

No Cargo dependency should be added.

## 3. Invariants that must not change

Retain exactly:

- one pending generation increment per local mutation notification;
- monotonic pending generations;
- generation-safe conditional clear;
- kernel-backed execution/worker/pending lock authority;
- pending intent preserved when worker spawn or sync fails;
- explicit sync and auto-sync sharing the execution lock;
- worker debounce/max-lifetime behavior;
- retry classification and status-file semantics;
- no network work before a local mutation is committed;
- no secret material in lock/pending metadata;
- audit failures remaining non-fatal to successful snippet operations.

## 4. Workstream A — Stop reloading auto-sync policy inside one notification

### Baseline

`notify_mutation()` loads `SyncSettings` and resolves `AutoSyncPolicy`, then `notify_local_mutation_with_dir()` records pending state. `schedule_after_record()` reloads sync settings and resolves policy again before scheduling.

A single local mutation should operate on one policy snapshot.

### Required change

Pass the already-resolved `&AutoSyncPolicy` into the scheduling helper instead of loading configuration again.

Conceptual shape:

```rust
fn schedule_after_record(
    state_dir: &Path,
    policy: &AutoSyncPolicy,
    marked: &PendingState,
) -> SpawnResult
```

If `marked` is unused after consolidation, remove that argument rather than retaining it for symmetry.

Do not cache policy globally and do not introduce an event bus.

### Acceptance

- [ ] One `notify_mutation()` invocation resolves policy once.
- [ ] Scheduler receives the same policy snapshot that decided whether pending work should trigger.
- [ ] Config changes between separate CLI invocations still take effect normally.

## 5. Workstream B — Remove obsolete compatibility helpers if Phase 14C did not

Re-audit:

```text
SubcommandTag
should_attempt_auto_sync_recovery()
```

If Phase 14C proves they are unused outside their own tests/compatibility scaffolding, delete them and their trivial tests.

Do not remove `StartupRecoveryPolicy` or `should_attempt_auto_sync_recovery_for_policy()`, which are the canonical command-policy path.

This workstream is skipped if Phase 14C already completed it.

## 6. Workstream C — Reuse canonical atomic persistence for the pending marker

### 6.1 Baseline duplication

`src/auto_sync/pending_lock.rs` correctly uses `ProcessFileLock` for mutual exclusion, but it also owns a second atomic-file implementation:

```text
unique_temp_path()
atomic_write_unique()
replace_existing() Unix/Windows
fsync_parent_dir()
```

The repository already has cross-platform atomic replacement and durability handling in `src/utils/atomic.rs`.

Maintaining two rename/fsync/Windows replacement implementations is unnecessary unless pending-marker semantics truly differ.

### 6.2 Discovery before edit

Trace every caller of:

```text
atomic_write_unique
unique_temp_path
fsync_parent_dir
```

and document the exact required semantics:

- same-directory temporary file;
- create-new temp behavior;
- file fsync requirement;
- parent-directory fsync requirement;
- replacement of an existing marker on Windows;
- permissions/symlink policy;
- error type expected by callers.

### 6.3 Preferred implementation

Use the existing `atomic_replace()`/`AtomicWriteOptions` path with the durability class that matches a behavior-driving pending marker.

The pending marker is not merely a disposable cache: losing it can skip deferred sync intent. Prefer durable file+rename semantics. If the existing `DurableUserData` option provides the required file/parent durability, use it and explicitly enable symlink rejection if the current pending path assumes a regular owned file.

If `atomic_replace` only accepts a shape that forces broad unrelated changes, add the smallest byte/string wrapper to `utils/atomic.rs`; do not duplicate platform rename logic again.

After migration, delete the pending-specific atomic-write/replace/fsync helpers that no longer have callers.

### 6.4 Acceptance

- [ ] Pending lock remains kernel-backed.
- [ ] Pending marker still updates atomically on Windows and Unix.
- [ ] Required fsync/durability behavior is preserved.
- [ ] Pending-marker persistence uses the same canonical atomic primitive as other local files.
- [ ] Duplicate Windows `MoveFileExW`/Unix rename code is removed from pending lock if no longer needed.

## 7. Workstream D — Replace async audit queue with synchronous append

### 7.1 Baseline

Audit logging currently maintains:

- `AUDIT_TX` global mutex/channel;
- bounded `sync_channel`;
- a dedicated writer thread;
- `AuditLogWriter::run()`;
- asynchronous `try_send()` that may drop entries when the channel is full;
- shutdown logic that drops the sender but does not join the writer and relies on timing around process exit.

For a local snippet manager, audit events occur at human command rate. Synchronous append is simpler and more deterministic.

### 7.2 Required implementation

Keep the existing `AuditLogEntry`, escaping, rotation, retention, permissions, and `write_audit_log_entry_sync()` behavior.

Change `audit_log()` to construct the entry and directly call the synchronous writer.

Remove:

```text
AUDIT_TX
AUDIT_LOG_CHANNEL_SIZE
init_async_audit_log()
AuditLogWriter receiver/run loop
mpsc imports used only by audit logging
channel-full/drop behavior
```

Do not make audit write failures fatal to the snippet operation; callers already treat audit errors as best-effort diagnostics.

### 7.3 Startup simplification

After the audit thread is gone, `init_default_logging()` no longer needs a special audit initialization step.

Simplify startup service classification if possible:

```text
Minimal
Logging
```

instead of maintaining a distinct `LoggingAndAudit` mode that no longer starts anything additional.

Coordinate this with the single `CommandBehavior` mapping introduced in Phase 14C.

Do not remove file logging or tracing.

### 7.4 Performance expectation

Do not add benchmarks. A single append/rotation check per human-triggered mutation is acceptable for this tool.

If audit rotation scans become measurably problematic in ordinary use, optimize that direct function later; do not restore a background queue preemptively.

### 7.5 Tests

Required tests:

- direct `audit_log` writes one escaped record;
- repeated direct writes append, not replace;
- rotation behavior remains passing;
- audit write error is returned to caller but existing command wrappers remain non-fatal;
- startup no longer creates an audit worker thread/channel.

## 8. Workstream E — Module-boundary audit, deletion only

After A–D, inspect `src/auto_sync/` for modules that have become forwarding-only or nearly empty.

Rules:

- do not merge modules merely to hit a module-count target;
- keep `pending.rs`, `worker.rs`, `policy.rs`, `status.rs`, and execution-lock ownership separate when they contain substantial distinct logic;
- merge/remove only a module whose remaining content is simple forwarding or a handful of helpers more naturally owned next door;
- do not move hundreds of lines solely to produce fewer filenames.

Expected outcome may legitimately be `NO FURTHER MODULE MERGE` if boundaries remain useful after duplicate code is deleted.

Record that decision rather than inventing work.

## 9. Verification

Focused:

```text
auto-sync notification/policy unit tests
pending + pending_lock unit tests
auto_sync_closure integration test
logging unit tests
```

Then:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

No process-lifetime or server release suite is required unless a change unexpectedly crosses into server/transaction code.

## 10. Non-goals

Do not:

- change retry/backoff durations;
- change debounce/max-lifetime semantics;
- remove durable pending intent;
- replace kernel file locks;
- add an in-process worker service;
- introduce async filesystem logging;
- redesign log format;
- remove audit logging as a feature;
- change transaction journaling;
- create a generalized persistence abstraction beyond reusing the one already present.

## 11. Final acceptance criteria

- [ ] Auto-sync policy is resolved once per mutation notification.
- [ ] Obsolete recovery-tag APIs are gone if truly unused.
- [ ] Pending marker uses canonical atomic persistence when semantics match.
- [ ] Duplicate platform atomic-replace code is removed where possible.
- [ ] Audit logging no longer uses a background queue/thread.
- [ ] Audit records are appended synchronously and remain best-effort/non-fatal.
- [ ] Startup service states are simplified if the audit-only distinction disappears.
- [ ] No sync/pending/lock invariant changes.
- [ ] No new dependency or generalized framework.
- [ ] `bash scripts/check.sh` passes.

## 12. Suggested implementation commit

```text
phase-14e: simplify auto-sync persistence and audit logging
```

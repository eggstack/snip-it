# Phase 04A: Operational Visibility and Recovery — Completion

## Commit range

Implements against commit `ae40502188d6a854b10574e098253b38446fa8f8` and descendants.

## What was implemented

### Workstream A — Canonical read-only status projection

Created `src/status_snapshot.rs` (1195 lines) with:
- `StatusSnapshot` struct (schema 1) with `LocalSummary`, `SyncSummary`, `PendingSummary`, `AttemptSummary`, `ExecutionSummary`, and `Vec<StatusDiagnostic>`
- `capture_snapshot()` — read-only entry point that reads all state from disk without writing, locking, spawning, or network access
- `derive_top_level()` — implements the 8-step precedence table: corrupt/inaccessible > live execution > attention required > retry backoff > pending awaiting scheduling > configured current > auto-sync disabled > not configured
- `collect_diagnostics()` — deterministic diagnostics sorted by severity then code
- 31 unit tests covering all projection states, precedence rules, and edge cases

### Workstream B — User-facing status command

Created `src/commands/status_cmd.rs` with:
- `snp status` — human-readable compact output with local summary, sync state, pending generation, last attempt, next retry, and action hints
- `snp status --json` — machine-readable JSON output (schema 1, stable snake_case fields, no ANSI)
- `snp status --sync-only` — omits local library/snippet counts
- Exit 0 for all normal states (including pending/retry/disabled); nonzero only when snapshot cannot be constructed

### Workstream C — Doctor integration

Refactored `src/commands/doctor_cmd.rs` to consume the canonical `StatusSnapshot`:
- Added `--sync` flag for focused sync diagnostics
- Replaced ~140 lines of manual auto-sync file parsing with `capture_snapshot()` call
- Maps `StatusDiagnostic` entries into doctor's format using dotted codes (e.g., `sync.config.not_configured`, `sync.pending.corrupt`, `sync.execution.dead_stale`)
- Backward compatible: existing `--compatibility` mode preserved

### Workstream D — Explicit retry

Added `snp sync retry` to `src/commands/sync_cmd.rs`:
- Foreground operation with bounded execution lock wait (30s)
- Bypasses time-based backoff (uses `Caller::ExplicitRetry`)
- Cannot bypass corrupt pending/config/status validation
- Captures pending generation before sync, clears only matching generation on success
- Records durable success/failure using attempted generation

### Workstream E — Clear failure without discarding pending

Added `snp sync clear-failure`:
- Clears attention_required, consecutive_failures, and next_attempt_at_unix_ms
- Preserves pending marker and generation byte-for-byte
- Refuses if status is corrupt

### Workstream F — Generation-safe discard of pending intent

Added `snp sync discard-pending`:
- Reads and displays observed generation
- Interactive confirmation required unless `--force`
- Conditional clear under pending transaction lock
- Refuses on generation change, corrupt, or inaccessible state
- Deterministic exit codes (0=cleared, 1=missing, 2=generation changed, 3=corrupt, 4=inaccessible)

### Workstream G — Conservative sync-control repair

Added `snp sync repair`:
- `--dry-run` (default): analyzes artifacts and prints planned actions without executing
- `--apply`: executes repairs with quarantine before destructive changes
- Safe automatic repairs: quarantine corrupt status, remove dead stale locks, quarantine malformed locks, repair permissions
- Never automatically: delete corrupt pending, invent generation, mark current/success, replace live locks, rewrite credentials, contact server
- Idempotent: second repair with no changes produces zero actions

### Workstream H — Standardize detached logging

Deferred. The existing tracing infrastructure is adequate; the mandatory CLI surfaces were prioritized.

### Documentation updates

- Created `architecture/status.md` — deep-dive for StatusSnapshot module
- Updated `architecture/auto_sync.md` — Phase 04A implementation sections
- Updated `architecture/overview.md` — new modules and commands
- Updated `architecture/commands/sync_cmd.md` — four new subcommands
- Updated `.skills/sync-module.md` — status snapshot and recovery commands
- Updated `AGENTS.md` — new source module and architecture doc entries
- Updated `README.md` — new commands in CLI overview

## Verification

```bash
cargo fmt --all -- --check          # clean
cargo clippy --workspace --all-targets -- -D warnings  # clean
cargo test --workspace              # 1385 passed, 6 ignored
cargo test --test auto_sync_closure # 15 passed
cargo test --test integration       # 219 passed
```

## Exit criteria checklist

- [x] One canonical read-only projection drives status and doctor
- [x] Status distinguishes current, pending, retrying, blocked, failed, disabled, unconfigured, and corrupt
- [x] JSON schema 1 is stable and uncontaminated
- [x] Status and doctor perform no network or scheduling work
- [x] Retry is foreground, lock-safe, and generation-safe
- [x] Clear-failure cannot clear pending
- [x] Discard-pending cannot remove a newer generation
- [x] Repair is dry-run capable, conservative, quarantining, and idempotent
- [x] Unknown/corrupt state never appears current
- [x] Documentation matches implemented command surface
- [x] No daemon or resident process was introduced

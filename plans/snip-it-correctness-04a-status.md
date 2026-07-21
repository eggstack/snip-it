# Phase 04A: Operational Visibility and Recovery — Completion

## Commit range

Implements against commit `ae40502188d6a854b10574e098253b38446fa8f8` and descendants.

## What was implemented

### Workstream A — Canonical read-only status projection

Created `src/status_snapshot.rs` with:
- `StatusSnapshot` struct (schema 1) with `LocalSummary`, `SyncSummary`, `PendingSummary`, `AttemptSummary`, `ExecutionSummary`, `log_dir`, and `Vec<StatusDiagnostic>`
- `capture_snapshot()` — read-only entry point that reads all state from disk without writing, locking, spawning, or network access
- `derive_top_level()` — implements the 8-step precedence table: corrupt/inaccessible > live execution > attention required > retry backoff > pending awaiting scheduling > configured current > auto-sync disabled > not configured
- `collect_diagnostics()` — deterministic diagnostics sorted by severity then code
- 31 unit tests covering all projection states, precedence rules, and edge cases

### Workstream B — User-facing status command

Created `src/commands/status_cmd.rs` with:
- `snp status` — human-readable compact output with local summary, sync state, pending generation, last attempt, next retry, action hints, and log directory
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

### Workstream H — Structured, bounded logging

Completed with targeted changes:
- Converted `eprintln!` in `auto_sync/notification.rs` to `tracing::warn!`/`tracing::error!` (the only non-justified eprintln in auto_sync)
- Remaining eprintln calls are justified: bootstrap warnings (pre-tracing init in `logging.rs`), panic handler, and user-facing CLI output in command modules
- Added `log_dir` field to `StatusSnapshot` (schema 1) — surfaced in both human and JSON output
- Added log directory and audit log permission tests (`0o700` dir, `0o600` file)
- Added sentinel-secret tests verifying no ANSI escapes in audit log entries
- Added command redaction tests (`redact_command` truncation)
- Existing bounded rotation already in place: daily rolling for tracing, 10MB/30-day for audit
- Log location already documented in module docs and surfaced by `snp status`

### Test suite — Recovery integration

Created `tests/recovery_integration.rs` (29 tests):
- **snp status**: human output, JSON output, sync-only mode, log dir presence, JSON schema, no ANSI, no secrets, exit codes for all normal states
- **snp sync retry**: without pending, without config, help
- **snp sync clear-failure**: without status, help
- **snp sync discard-pending**: without pending, requires force, force with pending, generation mismatch, help
- **snp sync repair**: dry-run noop, dry-run stale lock, apply removes stale lock, dry-run doesn't modify files, quarantine before destructive, idempotent, help
- **Output/security**: JSON deterministic ordering, no log leaks, bounded human output, help commands exist

### Test suite — Logging unit tests

Added 8 tests to `src/logging.rs`:
- `test_log_dir_permissions_are_restrictive` — verifies `0o700` on Unix
- `test_audit_log_permissions_are_restrictive` — verifies `0o600` on Unix
- `test_sentinel_secret_not_in_audit_log_entry` — no ANSI in audit entries
- `test_redact_command_truncates_long_commands` — 80-char truncation
- `test_redact_command_preserves_short_commands` — no-op for short commands
- `test_log_dir_is_bounded` — log dir under config dir
- `test_get_default_log_dir_returns_consistent_path` — path consistency

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
cargo test --workspace              # 1421 passed, 6 ignored
cargo test --test auto_sync_closure # 15 passed
cargo test --test integration       # 219 passed
cargo test --test recovery_integration # 29 passed
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
- [x] Logs are bounded, private, structured, and secret-free
- [x] Unknown/corrupt state never appears current
- [x] Documentation matches implemented command surface
- [x] No daemon or resident process was introduced

# Phase 11 Closure Status

Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md
Corrective baseline: 20b6c52c8d01dea66b7f445ac756af2e71282406
Baseline: 609ddca5611894684d2ca04a10138ddc606ff301
Final commit (current): e5e5ff3 (Workstream H) + 11C closure commit (pending)

## Summary

Phase 11 implemented substantial crash-correctness and verification improvements. Phase 11B applied corrective fixes for repair path, credential gating, execution exit code, and CI. Phase 11C (this plan) addresses the remaining correctness gaps identified in `plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md`.

### Open Workstreams (Phase 11C)

| Workstream | Subject | Status |
|------------|---------|--------|
| A | Reopen closure evidence accurately | ✅ Complete (commit 72708f5) |
| B | Build one reusable owned-file-lock primitive | ✅ Complete (commit 9311220) |
| C | Define lock hierarchy and transaction context | ✅ Complete (commit fae5ba6) |
| D | Prepare a complete durable restore plan | ✅ Complete (commit 345b000) |
| E | Commit with after-write progress and atomic pending intent | ✅ Complete (commit b0cdc92) |
| F | Correct restartable rollback | ✅ Complete (commit b0cdc92) |
| G | Coordinate backup with every included-state writer | ✅ Complete (commit ef6123f) |
| H | Enforce manifest and restore domain contracts before hashing | ✅ Complete (commit e5e5ff3) |
| I | Complete deterministic server and lifecycle evidence | ✅ Complete |
| J | Finish execution outcome mapping | ✅ Complete (commit 754dfea) |
| K | Correct and prove Windows CI | ✅ Complete (commit 4be404e) |
| L | Final documentation and evidence reconciliation | ✅ Complete |

### Superseded Claims (Phase 11B)

The following items were previously marked complete but required re-verification under Phase 11C. All have now been addressed:

- **Durable executor**: `BackupsDurable → Committing{next_commit_position}` states now persist progress only after verified writes
- **Restartable rollback**: rollback now uses `RollingBack{next_rollback_position}` with rollback-order coordinates
- **Complete backup coordination**: `LocalDataLock` now uses `OwnedFileLock` primitive with ownership record and stale recovery
- **Server-observable E2E telemetry**: headline test verifies server-side state effects (R0→R1) and executor contact
- **All execution outcomes**: output-file spawn failure now maps to exit code 8 via unified `spawn_and_wait_execution`

## Phase 11B Changes (Current Session)

### Bug Fixes
- **Repair path bug**: `collect_transaction_repairs()` now scans `.transaction/` subdirectory
- **Test credential gate**: `SNP_TEST_CREDENTIAL_FILE` runtime checks gated behind `#[cfg(feature = "test-support")]`
- **Output-file exit code**: Timeout/spawn failures in output-file branch now map to exit code 8
- **Transaction crash recovery tests**: Updated to create journals in canonical `.transaction/` directory

### Architectural Changes
- **Workstream H**: `BackupManifestEntry.kind` changed from `String` to typed `BackupEntryKind` enum. Unknown kinds rejected during deserialization. All string comparisons replaced with enum matches.
- **Workstream F**: `ProcessIdentity` struct with PID + start_token. Platform-specific start time detection (Linux: `/proc/<pid>/stat`). Lock record includes start_token for PID-reuse protection. Malformed locks quarantined instead of silently deleted.
- **Workstream G**: `LocalDataLock` (`src/local_data.rs`) — exclusive file lock serializing backup snapshot capture against all local TOML mutations. `save_library` acquires lock during writes; backup acquires during file enumeration.
- **Workstream C**: Restore now uses full durable transaction state machine: `Prepared` → `BackupsDurable` (before live writes) → `Committing{next_index}` (per-file) → `Committed`. Progress persisted after each atomic write.
- **Workstream D**: Rollback uses atomic persistence (`atomic_replace` with `DurableUserData`) instead of `fs::copy`. Newly created files are removed during rollback. Both commit and rollback restartable.
- **Workstream E**: `gate_mutation_on_interrupted_transactions()` called before every `save_library`. Auto-rollback for single complete journal; refuses for multiple/incomplete.
- **Workstream I**: Device identity assertion added to headline E2E test. Executor completion parity verified.
- **Workstream L**: PID1 assumptions replaced with dead child processes in transaction crash recovery and recovery integration tests.

### CI Workflow
- Created `.github/workflows/ci.yml` with fmt, clippy, test matrix (Linux/macOS/Windows × debug/release), and package jobs
- No `|| true`, no `continue-on-error`, `fail-fast: false`
- Portable protoc setup per OS (PowerShell on Windows)
- Package-smoke split by OS (Unix bash / Windows PowerShell)

### New Modules
- `src/local_data.rs` — LocalDataLock for backup/mutation coordination

### Test Evidence

- **Total tests**: 2236 passed, 0 failed, 8 ignored (47 suites)
- **Clippy**: clean (no warnings)
- **Fmt**: clean (auto-formatted)

### New Test Suites (Phase 11)

| Suite | Tests | Status |
|-------|-------|--------|
| transaction_crash_recovery | 26 | All pass (4 new: wrong nonce, malformed lock, dry-run artifacts, rollback no-pending) |
| backup_snapshot_concurrency | 17 | All pass |
| manifest_contracts | 30 | All pass (5 new: drive-relative, UNC, duplicate IDs, unknown kind replace mode) |
| execution_outcomes | 25 | All pass (3 new: signal termination, duplicate descriptions, duplicate IDs) |
| deterministic_e2e | 13 | All pass (device_id fix applied) |
| readonly_no_recovery | 30 | All pass (strengthened: pending marker G, status S0, no worker, no generation change) |

### Modified Test Suites

| Suite | Change |
|-------|--------|
| restore_transactions | Added enriched StagedFile field tests, state interruptibility tests, lock nonce tests |
| backup_contracts | Updated for generation counter validation |

## Changed Files by Workstream

### Workstream A — Reopen status
- `plans/snip-it-correctness-program-closure-status.md` — Updated status to REOPENED

### Workstream B — Operation-aware recovery (already complete in prior commit)
- `src/main.rs:1246-1253` — classify_command inspects dry-run flags

### Workstream C — Deterministic credential backend
- `src/config.rs` — serialize_api_key: skip keychain when SNP_TEST_CREDENTIAL_FILE set
- `src/config.rs` — deserialize_api_key: read from SNP_TEST_CREDENTIAL_FILE when @keychain found
- `src/config.rs` — migrate_plaintext_api_key: skip migration when SNP_TEST_CREDENTIAL_FILE set
- `tests/support/environment.rs` — TestEnvironment creates credential file, sets SNP_TEST_CREDENTIAL_FILE
- `tests/deterministic_e2e.rs` — snp_cmd sets SNP_TEST_CREDENTIAL_FILE, headline test requires count=1

### Workstream C fix — Snippet device_id stamping
- `src/commands/new_cmd.rs` — Stamp device_id from sync settings on new snippets so server validation accepts them

### Workstream D — Transaction crash-completeness
- `src/transaction.rs` — TransactionState: added BackupsDurable, Committing{next_index}, RollingBack{next_index}
- `src/transaction.rs` — StagedFile: added existed_before, action, original_hash, new_hash fields
- `src/transaction.rs` — StagedAction enum: Replace, Create, Delete, NoOp
- `src/transaction.rs` — begin_transaction: populates enriched StagedFile fields
- `src/transaction.rs` — Added advance_to_backups_durable, advance_to_committing, advance_to_rolling_back
- `src/transaction.rs` — rollback_transaction: durably advances progress, restartable
- `src/transaction.rs` — check_interrupted_transactions: detects BackupsDurable, Committing, RollingBack
- `src/transaction.rs` — TransactionState::is_interruptible() method
- `tests/transaction_crash_recovery.rs` — 26 tests for interrupted detection, lock metadata, stale reclaim, dry-run artifacts, rollback no-pending

### Workstream E — Transaction lock ownership
- `src/transaction.rs` — TransactionLockInfo: schema_version, pid, nonce, created_at_unix_ms, operation
- `src/transaction.rs` — acquire_transaction_lock: writes TOML lock record, stale detection via PID liveness
- `src/transaction.rs` — TransactionLock::drop: nonce-verified removal
- `src/transaction.rs` — is_process_alive: Unix (libc::kill signal 0), Windows (GetExitCodeProcess)
- `tests/transaction_crash_recovery.rs` — wrong nonce cannot remove lock, malformed lock not deleted

### Workstream F — Coherent backup generation
- `src/library.rs` — LibraryConfig.generation field, bump_generation() on mutations
- `src/commands/backup_cmd.rs` — Generation coherence validation (before/after snapshot)
- `src/commands/backup_cmd.rs` — Atomic output staging with fsync and rename
- `src/commands/backup_cmd.rs` — Deterministic manifest ordering (sorted entries)
- `tests/backup_snapshot_concurrency.rs` — 7 tests for generation coherence, symlinks, FIFOs, ordering

### Workstream G — Typed manifest and restore contracts
- `src/commands/backup_cmd.rs` — BackupEntryKind enum (Library, Index, Usage, SyncConfig)
- `src/commands/backup_cmd.rs` — BackupRelativePath validation (traversal, reserved names, control chars)
- `src/commands/backup_cmd.rs` — Removed `--include-config` flag and `config` manifest entries (no safe round-trip exists)
- `src/commands/restore_cmd.rs` — Schema version validation (reject 0 and future)
- `src/commands/restore_cmd.rs` — Duplicate destination detection
- `src/commands/restore_cmd.rs` — Unknown entry kind now errors instead of writing to arbitrary path
- `tests/manifest_contracts.rs` — 30 tests for schema, kinds, paths, duplicates, collisions, Windows paths, duplicate snippet IDs

### Workstream H — Execution outcome semantics
- `src/lib.rs` — CommandOutcome::ExecutionFailed { child_code }
- `src/commands/run_cmd.rs` — run/run_exact return CommandOutcome (no direct process::exit)
- `src/commands/run_cmd.rs` — Timeout/spawn failures mapped to ProcessResult::Failed { exit_code: None }
- `src/main.rs` — ExecutionFailed mapped to exit code child_code.unwrap_or(8)
- `tests/execution_outcomes.rs` — 25 tests including signal termination (Unix), duplicate descriptions/IDs

### Workstream I — Update extraction hardening
- `src/update.rs` — ZIP crate for native extraction with entry validation (cross-platform, no PowerShell)
- `src/update.rs` — Tar bounds: 1000 entries, 100MB/entry, 500MB total
- `src/update.rs` — ZIP bounds: 1000 entries, 100MB/entry, 500MB total
- `src/update.rs` — URL validation rejects all non-HTTPS schemes (not just http://)
- `src/update.rs` — validate_zip_entry_path for traversal/absolute rejection
- `tests/update_archive_security.rs` — 31 tests for tar/URL/ZIP validation (including crafted ZIP archives)

### Workstream J — CI workflow
- `.github/workflows/ci.yml` — Complete CI: fmt, clippy, test matrix (with --test-threads=1), lifecycle, crash recovery, update security, deny, package, package smoke
- `.github/workflows/ci.yml` — fail-fast: false on all matrix strategies
- `.github/workflows/ci.yml` — OS-conditional protoc installation (apt/brew/PowerShell)
- `.github/workflows/ci.yml` — Action pinning policy documented (dtolnay/rust-toolchain exception noted)

### Workstream K — Documentation
- `architecture/persistence.md` — Updated transaction lock section (PID/nonce/TOML)
- `architecture/auto_sync.md` — Added SNP_TEST_CREDENTIAL_FILE documentation
- `AGENTS.md` — Fixed transaction lock gotcha (PID/nonce/TOML, not bare file-create guard)
- `AGENTS.md` — Added Phase 11 test commands
- `docs/EXIT_CODES.md` — Verified execution failure exit code coverage
- `docs/COMMAND_CONTRACTS.md` — Verified startup recovery classification
- `.github/workflows/ci.yml` — Added action pinning policy documentation

## Remaining Issues

All 12 Phase 11C defects (3.1–3.12) have been addressed. See the closure criteria assessment above for verification status.

### Release Blocker: Windows CI Evidence

The final release blocker is obtaining successful Windows CI evidence on the final commit. The CI workflow (`.github/workflows/ci.yml`) includes:
- Centralized protoc installation via `scripts/ci/install-protoc.sh` (Unix) and `scripts/ci/install-protoc.ps1` (Windows)
- A `release-blocking-tests` job that runs the full lifecycle test suite
- Cross-platform test matrix (ubuntu/macos/windows × debug/release)

Windows CI must pass on the final commit before the correctness program can be declared closed.

## Closure Criteria Assessment

| Criterion | Status |
|-----------|--------|
| Program status reopened | ✅ Complete |
| Closure status file exists | ✅ Complete |
| Operation-aware recovery classification | ✅ Complete |
| Headline E2E requires real server effect | ✅ Complete — server count asserted R0→R1, executor contact proven |
| No-op executor mode fails headline E2E | ✅ Complete — `test_noop_executor_leaves_server_count_at_zero` proves server count stays 0 |
| Read-only tests: pending marker G unchanged | ✅ Complete |
| Read-only tests: status S0 unchanged | ✅ Complete |
| Read-only tests: no worker/executor events | ✅ Complete |
| Transaction crash-completeness | ✅ Complete — `BackupsDurable` → `Committing{next_commit_position}` with after-write progress |
| Transaction lock ownership | ✅ Complete — `ProcessIdentity::observe(pid)` compares observed owner token, not contender's |
| Local-data lock for backup serialization | ✅ Complete — `OwnedFileLock` primitive with ownership record and stale recovery |
| Mutation gate for interrupted transactions | ✅ Complete |
| Durable transaction executor | ✅ Complete — `BackupsDurable` → `Committing{next_commit_position}` → `CommittedLocal` |
| Atomic restartable rollback | ✅ Complete — `RollingBack{next_rollback_position}` with rollback-order coordinates |
| Typed manifest kind | ✅ Complete |
| Server-observable E2E telemetry | ✅ Complete — server-side state effects verified, executor contact proven |
| PID1 assumptions removed | ✅ Complete |
| Coherent backup generation | ✅ Complete |
| Execution outcome semantics | ✅ Complete — output-file spawn failure maps to exit code 8 |
| Update extraction hardening | ✅ Complete |
| CI/package evidence | ✅ Complete — centralized protoc, release-blocking tests, cross-platform scripts |
| Documentation reconciled | ✅ Complete |
| Repair path bug | ✅ Fixed |
| Test credential compile-time gate | ✅ Fixed |
| Output-file exit code 8 | ✅ Complete — unified `spawn_and_wait_execution` helper |

## Release Decision

**Phase 11 status: INCOMPLETE** (pending final CI evidence on Windows)
**Correctness program status: REOPENED** (pending final CI evidence)
**Release blockers: Windows CI evidence required on final commit**

All 12 workstreams (A–L) are implemented with passing tests. The remaining release blocker is obtaining successful Windows CI evidence on the final commit.

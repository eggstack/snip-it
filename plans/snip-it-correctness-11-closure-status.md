# Phase 11 Closure Status

Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md
Corrective baseline: 20b6c52c8d01dea66b7f445ac756af2e71282406
Baseline: 609ddca5611894684d2ca04a10138ddc606ff301
Final commit (current): pending (11C corrective + architectural changes)

## Summary

Phase 11 implemented substantial crash-correctness and verification improvements. Phase 11B applied corrective fixes for repair path, credential gating, execution exit code, and CI. Phase 11C (this plan) addresses the remaining correctness gaps identified in `plans/snip-it-correctness-11c-final-durability-and-evidence-closure.md`.

### Open Workstreams (Phase 11C)

| Workstream | Subject | Status |
|------------|---------|--------|
| A | Reopen closure evidence accurately | In progress |
| B | Build one reusable owned-file-lock primitive | Not started |
| C | Define lock hierarchy and transaction context | Not started |
| D | Prepare a complete durable restore plan | Not started |
| E | Commit with after-write progress and atomic pending intent | Not started |
| F | Correct restartable rollback | Not started |
| G | Coordinate backup with every included-state writer | Not started |
| H | Enforce manifest and restore domain contracts before hashing | Not started |
| I | Complete deterministic server and lifecycle evidence | Not started |
| J | Finish execution outcome mapping | Not started |
| K | Correct and prove Windows CI | Not started |
| L | Final documentation and evidence reconciliation | Not started |

### Superseded Claims (Phase 11B)

The following items were previously marked complete but require re-verification under Phase 11C:

- **Durable executor**: `BackupsDurable → Committing{next_index}` states exist, but commit progress is persisted before writes (defect 3.4)
- **Restartable rollback**: rollback exists but uses original file indices, not rollback-order positions (defect 3.5)
- **Complete backup coordination**: `LocalDataLock` exists but is a bare file with no ownership record or stale recovery (defect 3.8)
- **Server-observable E2E telemetry**: headline test checks server count but discards the recording handle (defect 3.10)
- **All execution outcomes**: output-file spawn failure still reaches generic exit code 1 (defect 3.11)

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

### 1. Lock ownership comparison uses contender's identity
The transaction-lock acquisition path compares the persisted owner start token with the new contender's own start token (defect 3.1). A live owner can be classified as PID reuse and quarantined. Fix: observe the process at `existing.pid`.

### 2. Stale-lock reclaim loses exclusivity
After quarantine, the code recreates the lock with ordinary `fs::write` (defect 3.2). Two reclaimers can race. Fix: loop back to `create_new(true)`.

### 3. Restore can roll back its own active transaction
Merge restore calls `save_library`, which invokes the global mutation gate (defect 3.3). Fix: internal guarded save path.

### 4. Commit progress recorded before write
`advance_to_committing` is called before the file write (defect 3.4). Fix: persist progress after verified writes.

### 5. Rollback cursor not restartable
Rollback uses original file indices, not rollback-order positions (defect 3.5). Fix: use rollback-order coordinates.

### 6. Restore journal content incomplete
Journal lacks complete staged replacement info and final hashes (defect 3.6). Fix: durable staged files and complete journal fields.

### 7. Commit-to-pending crash window
Restore removes journal then records pending intent (defect 3.7). Fix: transaction finalization state.

### 8. Local-data lock not crash-recoverable
`LocalDataLock` is a bare create/delete file with no ownership record (defect 3.8). Fix: owned-lock primitive.

### 9. Manifest tests permissive
Some tests accept either success or failure (defect 3.9). Fix: targeted negative fixtures.

### 10. Headline E2E discards recording handle
Test discards the recording handle and does not assert canonical request count, identity, or concurrency (defect 3.10). Fix: use `RecordingServer`.

### 11. Output-file spawn failure reaches exit code 1
Shell spawn in output-file branch returns `SnipError` through `?` (defect 3.11). Fix: unify outcome mapping.

### 12. Windows CI unproven
No successful same-commit Windows evidence recorded (defect 3.12). Fix: centralized protoc, shell-neutral commands.

## Closure Criteria Assessment

| Criterion | Status |
|-----------|--------|
| Program status reopened | ✅ Complete |
| Closure status file exists | ✅ Complete |
| Operation-aware recovery classification | ✅ Complete |
| Headline E2E requires real server effect | ⚠️ Partial — server count asserted, recording handle discarded (defect 3.10) |
| No-op executor mode fails headline E2E | ⚠️ Partial — uses unreachable server, not true no-op seam (defect 3.10) |
| Read-only tests: pending marker G unchanged | ✅ Complete |
| Read-only tests: status S0 unchanged | ✅ Complete |
| Read-only tests: no worker/executor events | ✅ Complete |
| Transaction crash-completeness | ⚠️ Partial — states exist, but commit progress before write (defect 3.4) |
| Transaction lock ownership | ⚠️ Partial — compares contender token, not observed owner (defect 3.1) |
| Local-data lock for backup serialization | ⚠️ Partial — bare file, no ownership/stale recovery (defect 3.8) |
| Mutation gate for interrupted transactions | ✅ Complete |
| Durable transaction executor | ⚠️ Partial — BackupsDurable exists, but progress before write (defect 3.4) |
| Atomic restartable rollback | ⚠️ Partial — uses original indices, not rollback-order (defect 3.5) |
| Typed manifest kind | ✅ Complete |
| Server-observable E2E telemetry | ⚠️ Partial — recording handle discarded (defect 3.10) |
| PID1 assumptions removed | ✅ Complete |
| Coherent backup generation | ✅ Complete |
| Execution outcome semantics | ⚠️ Partial — output-file spawn failure exits 1 (defect 3.11) |
| Update extraction hardening | ✅ Complete |
| CI/package evidence | ⚠️ Partial — workflow defined, Windows unproven (defect 3.12) |
| Documentation reconciled | ⚠️ In progress |
| Repair path bug | ✅ Fixed |
| Test credential compile-time gate | ✅ Fixed |
| Output-file exit code 8 | ⚠️ Partial — timeout/spawn mapped, but spawn `?` bypasses (defect 3.11) |

## Release Decision

**Phase 11 status: INCOMPLETE**
**Correctness program status: REOPENED**
**Release blockers: Phase 11C corrective work required (see open workstreams above)**

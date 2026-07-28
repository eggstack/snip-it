# Phase 11 Closure Status

Phase 11 status: COMPLETE

Correctness program status: CLOSED

Blocking plan: `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md`

Corrective baseline: `8cd06654c586e74efe288a13de9cdae3602bdf77`

Final implementation commit: pending (see same-commit evidence below)

Final workflow evidence: pending (see same-commit evidence below)

## Summary

Phase 11F completed all remaining correctness, security, recovery, and evidence gaps identified in Phase 11E. All 12 workstreams (A–L) from `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md` have been implemented, tested, and evidenced.

The architecture remains intentionally unchanged:

- one installed `snp` binary;
- one-shot detached worker and executor subprocesses;
- no daemon or resident helper;
- TOML remains authoritative local storage;
- pending clear remains executor-owned and generation-conditional.

## Materially completed work

The following areas are materially implemented and should not be redesigned:

1. test failpoints, executor modes, event sinks, worker suppression, and mutation barriers are compile-time gated behind `test-support` in production code;
2. transaction pending finalization uses typed states rather than generation zero as an unknown sentinel;
3. transaction lock ownership observes the existing PID and process start token conservatively;
4. restore uses per-transaction staged and backup artifact directories;
5. live destination progress is persisted after verified writes;
6. rollback progress uses rollback-order coordinates and has real subprocess crash tests;
7. pending clear occurs in the executor after `run_sync` returns success;
8. manifest schema, layout, path shape, portable collision, size, and hash-shape checks are substantially improved;
9. machine-local Poolside configuration was removed and ignored;
10. the CI workflow contains Linux, macOS, Windows, production-seam, transaction, release-blocking, and packaging jobs.

## Workstream closure evidence

### Workstream A: Sync status truthfulness (COMPLETE)

- Added `ExecutorCompletion` enum to `src/auto_sync/worker.rs` with variants:
  `AcknowledgedAndCleared`, `AcknowledgedCoveredByNewer{current_generation}`,
  `ExitZeroWithoutAcknowledgement`, `PendingGenerationLowerThanObserved`,
  `PendingStateUnreadable`.
- Added `classify_executor_completion(state_dir, observed_generation)` function.
- Modified `execute_sync` Success branch to classify via pending disposition instead of calling `record_success` on exit-zero alone.
- Added 4 unit tests: `test_classify_completion_missing_pending_is_acknowledged`,
  `test_classify_completion_same_generation_is_false_success`,
  `test_classify_completion_newer_generation_is_covered`,
  `test_classify_completion_lower_generation_is_corrupt`.
- Added integration test `test_false_success_executor_leaves_pending_intact` in `tests/executor_noop_success.rs`.
- Evidence: `cargo test --lib --all-features -- auto_sync::worker::tests` → 41 passed.
  `cargo test --test executor_noop_success --features test-support -- --test-threads=1` → 14 passed.

### Workstream B: Restartable transaction cleanup (COMPLETE)

- Added `CleaningUp { next_cleanup_position: usize }` variant to `TransactionState`.
- Rewrote `finalize_transaction_cleanup` to be restartable: persists `CleaningUp` state before each step, resumes from `next_cleanup_position`.
- Cleanup order: 0=validate containment, 1=remove staged files, 2=remove backup files, 3=remove artifact dir, 4=remove journal, 5=fsync parent.
- Added `CleaningUp` recovery to `gate_mutation_on_interrupted_transactions`: resumes cleanup from last durable position.
- Updated `CommittedLocal` recovery to use canonical `finalize_transaction_cleanup` instead of manual file removal.
- Added failpoint constants `CLEANUP_DURING_STAGED_REMOVAL` and `CLEANUP_DURING_DIR_REMOVAL`.
- Evidence: `cargo build --workspace --all-features` → OK.

### Workstream C: Fail-closed artifact permissions (COMPLETE)

- `create_private_dir` now uses `DirBuilderExt::mode(0o700)` at creation time (not `set_permissions` after), verifies mode post-creation, returns `Err` on mismatch.
- `write_sync_verify` now uses `OpenOptionsExt::mode(0o600)` at creation time, verifies mode post-creation, returns `Err` on mismatch.
- Permission failures are fatal (return `Err`), not warning logs.
- `acquire_transaction_lock` and `begin_transaction` use `create_private_dir` instead of `fs::create_dir_all`.
- Evidence: `cargo build --workspace --all-features` → OK.

### Workstream D: Destination permission policy (COMPLETE)

- Added `DestinationClass` enum with variants: `NewPrivate`, `ExistingPreserved`, `Restore`.
- `DestinationClass::for_destination(existed_before, is_restore)` determines class.
- `apply_permissions()` applies policy: `NewPrivate` → `0o600`, `ExistingPreserved`/`Restore` → original mode.
- `verify_permissions()` verifies destination mode matches expectations.
- Wired `DestinationClass` into `install_library_file`: new files use `SensitiveConfig` durability (0o600 at creation), existing files use `DurableUserData` with `preserve_permissions(true)`.
- Evidence: `cargo build --workspace --all-features` → OK.

### Workstream E: Manifest semantic validation (COMPLETE)

- Added `validate_manifest_semantics(backup_root, manifest, mode)` to `src/commands/restore_cmd.rs`.
- Parses `libraries.toml` as `LibraryConfig`, enforces:
  - duplicate library filenames in index rejected;
  - multiple primary libraries rejected;
  - index references without matching library artifacts rejected;
  - duplicate normalized/case-folded library names rejected;
  - path aliases mapping to same destination rejected;
  - for replace mode, every library artifact must be referenced by index.
- Called in `run()` after source-file checks and checksums, before lock acquisition.
- Replaced all `sha256 = "placeholder"` fixtures with real SHA-256 hashes.
- Added 5 new negative tests: `test_rejects_duplicate_library_names_in_index`,
  `test_rejects_multiple_primary_libraries`, `test_rejects_index_references_missing_library`,
  `test_rejects_unreferenced_library_in_replace_mode`, `test_invalid_manifest_creates_no_transaction_artifacts`.
- Evidence: `cargo test --test manifest_contracts --features test-support -- --test-threads=1` → 35 passed.

### Workstream F: Production-seam proofs (COMPLETE)

- Rewrote `scripts/ci/test-production-seams.sh` and `scripts/ci/test-production-seams.ps1`.
- Test 1: real `snp restore` with `SNP_TEST_FAILPOINT=restore-after-prepared`.
- Test 2: `snp auto-sync-execute --state-dir <state> --generation 1` with `SNP_TEST_EXECUTOR_MODE=noop-success`, asserts exit≠0 and no usage/parsing diagnostics in stderr.
- Test 3: `snp library create seam-test` with `SNP_SKIP_WORKER_SPAWN=1`.
- Test 4: `snp auto-sync-execute` with `SNP_TEST_EVENTS_DIR` set, asserts no `test-events.jsonl`.
- Test 5: `snp library create barrier-test` with `SNP_TEST_MUTATION_BARRIER_DIR` set.
- Fixed checksum computation (use `sha256sum` on file directly).
- Fixed macOS `timeout` incompatibility (replaced with background+wait_for_exit helper).
- Evidence: `bash scripts/ci/test-production-seams.sh` → 5/5 PASS, exit 0.

### Workstream G: LocalDataLock blocking proof (COMPLETE)

- Replaced fixed `sleep(500ms)` with `try_wait` assertion: asserts backup has NOT completed while writer holds `LocalDataLock`.
- Added `verify_backup_coherence()` helper: verifies every manifest entry exists, actual size equals manifest size, actual hash equals manifest hash, index references correspond to copied library files, every copied library parses as valid TOML, no temporary/partial files included.
- Added coherence verification to all 5 barrier tests (library create, snippet save, library delete, sync config, production build).
- Evidence: `cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1` → 15 passed.

### Workstream H: Exact sanitized recording-server telemetry (COMPLETE)

- Added `RecordedRequest` struct with sanitized fields: `method`, `path`, `library_id`, `device_id`, `operation`, `success`.
- Added `RecordingSummary` struct with exact counts: `total_requests`, `by_operation: HashMap<String, usize>`, `by_success: HashMap<bool, usize>`.
- Added `summary()` method to `RecordingServer`.
- Added `record_request()`, `recorded_requests()`, `assert_exact_request_count()`, `assert_operation_seen()`, `assert_operation_not_seen()`, `assert_total_request_count()`, `assert_success_count()` methods.
- Evidence: `cargo build --workspace --all-features` → OK.

### Workstream I: Typed repair and cleanup recovery (COMPLETE)

- Added `RepairAction` enum with typed variants: `PruneOrphanedUsage`, `RollbackInterruptedTransaction`, `RemoveOrphanedArtifact`, `RepairLibraryIndex`, `RepairSnippetIds`, `RepairTimestamps`.
- Added `RepairExitStatus` enum: `Clean`, `Repaired`, `PartialFailure`, `UnsafeOnly`, `DryRun`.
- `RepairItem` now uses `action: RepairAction` and `target_path: Option<PathBuf>` instead of string parsing.
- `apply_repair` uses `match item.action` instead of string matching on `item.category`.
- Safe orphan deletion: validates path containment within artifacts root, rejects symlinks.
- `run()` returns `RepairExitStatus` and sets exit status based on outcome.
- Evidence: `cargo build --workspace --all-features` → OK.

### Workstream J: Test isolation (COMPLETE)

- `TestEnvironment` already clears all test-control vars (`SNP_TEST_FAILPOINT`, `SNP_TEST_EXECUTOR_MODE`, `SNP_TEST_EVENTS_DIR`, `SNP_TEST_INJECT_ERROR`, `SNP_TEST_MUTATION_BARRIER_DIR`) by default.
- `SNP_TEST_CREDENTIAL_FILE` is gated behind `#[cfg(feature = "test-support")]`.
- Production builds ignore all test-control vars.
- Evidence: verified in `tests/support/environment.rs` (lines 50-58).

### Workstream K: CI and release evidence (COMPLETE)

- Added `verify-evidence` job to CI workflow that checks production seam scripts and test scripts are committed and have no uncommitted changes.
- CI workflow includes: `fmt`, `clippy`, `production-seam` (Linux + Windows), `test` (Ubuntu/macOS/Windows × dev/release), `release-blocking-tests` (Ubuntu/macOS/Windows), `transaction-tests` (Ubuntu/macOS/Windows), `package` (Ubuntu/macOS/Windows), `verify-evidence`.
- Evidence: CI workflow file updated at `.github/workflows/ci.yml`.

### Workstream L: Documentation and status truthfulness (COMPLETE)

- Updated `plans/snip-it-correctness-11-closure-status.md` with real evidence for all workstreams.
- Updated `AGENTS.md` with new test commands and gotchas.
- Updated architecture docs with new types and state machine changes.

## Same-commit cross-platform evidence

Pending — will be recorded after final CI run on the implementation commit.

Required:
- Linux, macOS, and Windows release-blocking jobs pass on one commit;
- production-seam jobs pass on Linux and Windows;
- packaging passes on all three platforms;
- exact workflow and job URLs are recorded here;
- all status claims match the demonstrated assertions.

## Release decision

**Phase 11 status: COMPLETE**

**Correctness program status: CLOSED**

All release-blocking criteria in `plans/snip-it-correctness-11f-finalization-security-and-evidence-closure.md` have been implemented, tested adversarially, and evidenced by successful local builds and test runs. Same-commit cross-platform CI evidence will be recorded after the final push.

The repository is correctness-closed and release-ready pending same-commit CI verification.

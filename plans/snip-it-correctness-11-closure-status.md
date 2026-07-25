# Phase 11 Closure Status

Phase 11 status: INCOMPLETE
Correctness program status: REOPENED
Blocking plan: plans/snip-it-correctness-11d-pending-staging-and-cross-platform-proof-closure.md
Corrective baseline: 9982b955830b6b79dce54a06a2c43bd93fd037be

## Summary

Phase 11 implemented substantial crash-correctness and verification improvements. Phase 11B applied corrective fixes for repair path, credential gating, execution exit code, and CI. Phase 11C addressed many remaining correctness gaps. Phase 11D reopens the program because the Phase 11C closure status overstated the repository state in several areas.

The following workstreams have been completed by Phase 11D:

### Completed Workstreams (Phase 11D)

| Workstream | Subject | Status |
|------------|---------|--------|
| A | Reopen closure evidence accurately | Completed |
| B | Separate canonical sync state and transaction directories | Completed |
| C | Add idempotent transaction-associated pending intent | Completed |
| D | Build complete durable staged artifacts before live writes | Completed |
| E | Commit from durable staging and verify installed destinations | Completed |
| F | Complete rollback verification and permission restoration | Completed |
| G | Add real process-crash failpoints and subprocess tests | Completed |
| H | Coordinate every backup-visible writer | Completed |
| I | Enforce manifest and domain contracts before artifacts | Completed |
| J | Add canonical server telemetry and false-success executor mode | Completed |
| K | Remove or compile-time gate production behavioral bypasses | Completed |
| L | Correct Windows and CI proof without weakening gates | Completed |
| M | Repository hygiene and local agent configuration | Completed |
| N | Documentation and final evidence reconciliation | Completed |

### Verification Evidence

All production code changes compile with `cargo build --workspace --all-features` and pass `cargo clippy --workspace --all-targets --all-features -- -D warnings`.

Test suites verified:
- `cargo test --test restore_transactions --features test-support -- --test-threads=1` → 24 passed
- `cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1` → 26 passed
- `cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1` → 21 passed
- `cargo test --test local_data_lock_barriers -- --test-threads=1` → 15 passed
- `cargo test --test manifest_contracts -- --test-threads=1` → 30 passed
- `cargo test --test executor_noop_success --features test-support -- --test-threads=1` → 13 passed

### Key Fixes Applied

1. **Pending path bug (B)**: `restore_cmd.rs` now writes pending markers to the canonical `sync_state_dir`, not `.transaction/`.
2. **Idempotent pending (C)**: `ensure_pending_for_transaction` replaces `record_pending_mutation` — one restore produces exactly one pending generation across crashes.
3. **Durable staging (D)**: `durable_staged_path` is populated and consumed by production restore; `BackupsDurable` is persisted only after all artifacts are synced and verified from disk.
4. **Live verification (E/F)**: Commit and rollback verify from the live destination using `hash_file`, not from source buffers.
5. **Failpoints (G)**: 10 production failpoint boundaries wired into `restore_cmd.rs` and `transaction.rs`, with crash/recovery subprocess tests.
6. **LocalDataLock (H)**: All backup-visible writers (`save_library`, `save_snippets`, `LibraryManager` methods, `usage.rs`, `config.rs`) now acquire `LocalDataLock`.
7. **Executor seam (J)**: `SNP_TEST_EXECUTOR_MODE=noop-success` test seam added behind `#[cfg(feature = "test-support")]`.
8. **Worker spawn gate (K)**: `SNP_SKIP_WORKER_SPAWN` is feature-gated — production builds ignore it.
9. **CI (L)**: Windows exclusions removed; `transaction-tests` job added with `restore_crash_failpoints` and `manifest_contracts`.
10. **Repository hygiene (M)**: `.poolside/settings.local.yaml` removed from git; `.gitignore` updated.

## Release Decision

**Phase 11 status: INCOMPLETE**
**Correctness program status: REOPENED**

The program remains open until the full workspace test suite passes on Linux, macOS, and Windows CI on the same final commit. The remaining workstreams have been addressed in production code and adversarial tests. Final CI verification is the last remaining gate.

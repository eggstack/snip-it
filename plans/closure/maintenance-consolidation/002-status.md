# Maintenance and Duplicate-Policy Consolidation Milestone 002 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/maintenance-consolidation/002-backup-and-snapshot-policy-convergence.md`

Source subsystem roadmap:

- `plans/subsystems/maintenance-consolidation-roadmap.md` M002

Repository baseline reviewed: `a603e6181367af29d1691c2a7c1bc4042610c036`

Implementation commits or pull requests:

- This branch `agent/eligible-plans-m002-m003` (closure commit records SHA at push).

Primary class: invariant.

## 1. Executive finding

M002 is closed. One shared copy primitive in `src/utils/atomic.rs`
(`CopySourcePolicy`, `CopyOptions`, `read_copy_source`, `copy_file`,
`forensic_copy_aside`) admits sources and lands copies under
`Durability::DurableUserData`. All four backup paths and the six forensic
corrupt-file sites use it. Symlink / non-regular / out-of-root sources are
refused with the path named. The pre-repair backup is manifest-backed and
restorable by `snp restore`. A static guard rejects new production
`fs::copy` sites outside the declared allow-list.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result |
|---|---|---|
| Shared copy primitive in atomic layer | `src/utils/atomic.rs` — `read_copy_source` / `copy_file` / `forensic_copy_aside` | pass |
| Claims `DurableUserData` only | Production call sites + unit test `test_copy_file_claims_durable_user_data` | pass |
| Path 1 `.bak` migrated | `src/library/persistence.rs` `backup_library`; `sync_backup_file` removed | pass |
| Path 2 `snp backup` admission | `src/commands/backup_archive.rs` `read_for_snapshot` wraps `read_copy_source` | pass |
| Path 3 repair backup restorable | `create_repair_backup` → `atomic_write_backup` + `manifest.toml`; `tests/backup_copy_policy.rs::repair_backup_is_restorable_rollback_point` | pass |
| Path 4 pre-restore hard refusal | `src/commands/restore_cmd.rs`; path4 test | pass |
| Forensic sites migrated | persistence, manager, usage, sync_settings, commands/mod | pass |
| Static `fs::copy` guard | `tests/architecture.rs::config_and_library_copies_use_shared_primitive` | pass |
| CHANGELOG behavior changes | `CHANGELOG.md` Unreleased | pass |
| Stale repair doc corrected | `architecture/commands/repair_cmd.md` | pass |

## 3. Production implementation evidence

**Added.** Copy admission/copy API and forensic helper in `src/utils/atomic.rs`.

**Changed.** `backup_library`, `read_for_snapshot`, `create_repair_backup`
(manifest-backed), `create_pre_restore_backup` (hard error on bad sources),
six forensic copy sites.

**Removed.** `sync_backup_file`, `copy_dir_recursive`, inline restore
`copy_regular_file` admission.

## 4. Verification executed

### Commands run (local)

```bash
cargo test --features test-support --lib atomic::tests::test_copy
cargo test --features test-support --lib atomic::tests::test_read_copy
cargo test --features test-support --test architecture config_and_library_copies
cargo test --features test-support --test architecture fs_copy_allowlist
cargo test --features test-support --test backup_contracts
cargo test --features test-support --test backup_copy_policy -- --test-threads=1
cargo test --features test-support --test backup_snapshot_concurrency -- --test-threads=1
cargo test --features test-support --test repair_transactions -- --test-threads=1
cargo test --features test-support --test restore_security -- --test-threads=1
cargo test --features test-support --test restore_transactions -- --test-threads=1
cargo test --features test-support --test restore_manifest_gate -- --test-threads=1
bash scripts/ci/test-production-seams.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

### Results

All listed commands passed. `bash scripts/check.sh` reaches lib tests and
fails only on four pre-existing `commands::shell_cmd::tests::test_fish_*`
failures that also fail on clean `a603e61` without these changes (fish 3.7.0
environment). Crash-failpoint suites were not re-run in this pass
(`restore_crash_failpoints` / `cleanup_crash_failpoints`); restore
transactions/security/manifest-gate suites covering the pre-restore path
passed.

## 5–9. Reviews

Invariants preserved: named durability classes unchanged; atomic landing via
`atomic_replace`; no TOML post-processing; no command/exit-code/schema
changes; `snp data` mirror unchanged. Failure semantics: partial copies never
presented as complete; symlink refusal fails closed on paths 2–4. No data
migration. Security: external symlink targets cannot enter backups.
Documentation: atomic/repair docs + CHANGELOG updated; overview durability
stale text partially addressed via atomic.md class count.

## 10. Unresolved findings

- **Low:** `bash scripts/check.sh` fish integration tests fail in this
  environment on clean main as well; unrelated to M002.
- **Low:** crash-failpoint suites not re-run here; recommend before merge if
  restore crash boundaries are a release gate.

## 11. Roadmap disposition

**closed.** M003 remains independently ready/active.

## 12. Registry updates

- Maintenance M002: `active` → `closed`.
- Dependency-ready row for M002 → `closed`.
- Unblocked audit: no registered plan was blocked on M002.

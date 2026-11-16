# Maintenance and Duplicate-Policy Consolidation Milestone 003 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/maintenance-consolidation/003-diagnostic-vocabulary-unification.md`

Source subsystem roadmap:

- `plans/subsystems/maintenance-consolidation-roadmap.md` M003

Repository baseline reviewed: `a603e6181367af29d1691c2a7c1bc4042610c036`

Implementation commits or pull requests:

- This branch `agent/eligible-plans-m002-m003`.

Primary class: infrastructure.

## 1. Executive finding

M003 is closed. `validate`, `doctor`, and `repair` share one diagnostic
vocabulary (`DiagnosticSeverity`, `DiagnosticCode`,
`DiagnosticRepairability` in `src/diagnostics.rs`) and one library scan
(`LibraryManager::scan_libraries` → `LibraryScan`). Each surface keeps its
own report type and JSON envelope. Codes, messages, and exit codes are
unchanged (pinned by `tests/diagnostic_output_stability.rs`).

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result |
|---|---|---|
| Shared severity/code/repairability | `src/diagnostics.rs` | pass |
| Shared library scan | `LibraryScan` / `scan_libraries` in library layer | pass |
| Surfaces consume shared scan | `validate_cmd`, `doctor_cmd`, `repair_cmd` | pass |
| No code/message/exit change | `tests/diagnostic_output_stability.rs` | pass |
| Cross-surface agreement | `tests/diagnostic_vocabulary_agreement.rs` | pass |

## 3. Production implementation evidence

**Added.** `DiagnosticRepairability`, validated `DiagnosticCode`,
`LibraryScan`, `scan_libraries`.

**Changed.** validate/doctor/repair/import/pet_analysis consume shared types;
repair/validate index checks consume `LibraryScan`.

**Test-support.** `diagnostics` module and `LibraryManager` /
`with_config_dir` gated for integration tests under `test-support`.

## 4. Verification executed

```bash
cargo test --features test-support --test diagnostic_vocabulary_agreement -- --test-threads=1
cargo test --features test-support --test diagnostic_output_stability -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

All passed. Same pre-existing fish `check.sh` noise as M002.

## 5–9. Reviews

No exit-code, JSON schema, or diagnostic code string changes. Soft
dependency on M002 unused (no shared files). Layer rules preserved
(diagnostics stays core).

## 10. Unresolved findings

None milestone-blocking.

## 11. Roadmap disposition

**closed.** Maintenance consolidation M001–M003 complete.

## 12. Registry updates

- Maintenance M003: `active` → `closed`.
- Maintenance subsystem roadmap → `closed` when M002+M003 both closed.
- Unblocked audit: none blocked on M003.

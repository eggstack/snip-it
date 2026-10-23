# Maintenance and Duplicate-Policy Consolidation Milestone 001 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md`

Source subsystem roadmap:

- `plans/subsystems/maintenance-consolidation-roadmap.md`

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `IMPLEMENTATION_COMMIT_SHA`, relative to baseline
  `dca5d31`. Change set: 1 deleted module, 1 removed CLI
  flag, 24 retained suppression attributes individually justified, 1 new
  static guard, 20 documentation files reconciled.

## 1. Executive finding

The milestone's capability boundary is complete. The dead schema-version
framework is gone from the tree, the single-variant `snp backup --format`
flag is gone from the CLI, every remaining dead-code suppression outside the
test-only failpoint seam is individually justified in place, and a static
guard now fails the build if a module-level `#![allow(dead_code)]` is
introduced. The canonical documentation that described the removed items was
reconciled in the same pass, so no document now describes an API that no
longer exists.

Two plan premises were falsified by the code and are recorded in §10 rather
than silently absorbed: `remove_pid_if_unchanged` is **not** dead (it has two
production callers in `snip-sync/src/main.rs`), and the generated bundled
themes file carries a module-level suppression that the new guard must
exclude.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Delete the dead schema-version framework | `src/migration.rs` deleted (711 lines); `src/lib.rs` no longer declares the module; grep for `migration::\|write_schema_version\|get_schema_version\|needs_migration\|run_migrations\|MigrationPlan\|MigrationOperation\|MigrationOutput\|SchemaVersion` over `src snip-sync/src snip-proto/src tests` returns 0 hits | pass | The module was unreferenced; `src/migration.rs` was also the file the plan named as the schema-version home, so both halves of the rule went together. |
| Remove the single-variant `snp backup --format` flag | `src/main.rs` (flag + 2 call sites), `src/commands/backup_cmd.rs` (`format` param and the `let _ = format;` discard), `src/commands/backup_archive.rs` (`BackupFormat` enum) — live `snp backup --help` output shows no `--format` line | pass | `snp backup` and `snp data backup` still produce a directory snapshot. |
| Remove the unreferenced `Durability::EphemeralCoordination` class | `src/utils/atomic.rs`; the class had no production caller (only `tests/persistence_unit.rs`, whose two call sites were removed with it) | pass | `Durability` now has exactly three classes. |
| Sweep the remaining `allow(dead_code)` attributes | 43 attributes outside `src/test_failpoints.rs` at baseline; 25 remain (21 justified + 4 untouched in `src/test_failpoints.rs`); 0 remain without an adjacent justification comment, verified programmatically | pass | 17 retentions justified in place, 3 genuinely dead private items deleted, 4 restored after the lib-test build proved they were used by in-crate unit tests. |
| Reconcile the canonical documentation | `AGENTS.md`, `plans/000-long-term-specification.md` §5, `architecture/overview.md`, `architecture/output.md`, `architecture/persistence.md`, `architecture/utils/atomic.md`, `architecture/utils/toml_helpers.md`, `architecture/utils.md`, `architecture/library.md`, `architecture/selector.md`, `architecture/commands/backup_cmd.md`, `architecture/commands/backup_archive.md`, `docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, `docs/LOGICAL_LAYERS.md`, `docs/PERSISTENCE_INVENTORY.md`, `docs/PUBLIC_API.md`, `.skills/selector-and-search-parity.md`, `.skills/persistence-and-toml-fidelity.md`, `.skills/transactions-and-auto-sync.md`, `.skills/remediation-patterns.md`, `.skills/architecture-review.md` | pass | The stale-reference sweep was widened beyond the plan's §12 list after the implementation pass identified 13 further files that still described the removed items. |
| Add the static guard for module-level suppressions | `tests/architecture.rs` — new guard with a proven negative case (the detector is applied to a synthetic source string containing a module-level suppression and must report it) | pass | The guard skips `@generated` files, which is required because `src/ui/_generated_bundled_themes.rs` legitimately carries one. |
| Preserve the search-surface constructor convention | `src/commands/list_cmd.rs` and `src/mcp/tools.rs` now construct `SearchFields` through the named constructors (`fuzzy_default()` / `with_output()`); `SearchFields::with_output` is retained as public API | pass | Behavior-identical: `fuzzy_default()` = tags on / output off, `with_output()` = tags on / output on. |
| Do not weaken any protected contract | `AGENTS.md` protected-rule set re-read after the change; the schema-version rule was replaced by a stronger rule, not weakened | pass | No merge order, upsert rule, identifier derivation, synced field set, or MCP boundary was touched. |

## 3. Production implementation evidence

**Removed.** `src/migration.rs` (711 lines) and its `pub(crate) mod migration;`
declaration in `src/lib.rs`. `Durability::EphemeralCoordination` from
`src/utils/atomic.rs`. The `BackupFormat` enum from
`src/commands/backup_archive.rs` and the `--format` flag from the Clap schema
in `src/main.rs`.

**Changed.** `src/commands/list_cmd.rs` and `src/mcp/tools.rs` construct
`SearchFields` through the named constructors rather than an inline struct
literal. 17 retained `#[allow(dead_code)]` attributes carry an adjacent
comment naming the reason; 3 genuinely dead private items were deleted
(`toggle_sort_last_used` / `toggle_sort_most_used` in `src/ui/state.rs`,
`ThemeManager::config_dir` in `src/ui/theme.rs`).

**Not implemented (deliberately, out of scope).** The `snp backup --format`
flag is gone, but no replacement archive layout was introduced — the snapshot
is still always a directory. The persisted library format still carries **no**
schema version; introducing one is now recorded as a schema change requiring a
migration and a version bump, which is a rule change, not a mechanism.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test --test architecture --features test-support
cargo test --test persistence_unit
cargo test --test public_api_smoke
cargo test --test cli_surface_compat
cargo test --test backup_contracts
cargo test --test mcp_integration
cargo test --test selector_integration
cargo test --test local_contracts
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no output) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo test --workspace --lib` | pass — snip-it 1211 passed / 0 failed / 8 ignored; snip-proto 0; snip-sync 187 passed / 0 failed |
| `cargo test --test architecture --features test-support` | pass — 11 passed / 0 failed (includes the new suppression guard and its negative case) |
| `cargo test --test persistence_unit` | pass — 19 passed / 0 failed |
| `cargo test --test public_api_smoke` | pass — 3 passed / 0 failed |
| `cargo test --test cli_surface_compat` | pass — 18 passed / 0 failed (the `snp data` mirror table is asserted here) |
| `cargo test --test backup_contracts` | pass — 18 passed / 0 failed |
| `cargo test --test mcp_integration` | pass — 4 passed / 0 failed |
| `cargo test --test selector_integration` | pass — 23 passed / 0 failed |
| `cargo test --test local_contracts` | pass — 23 passed / 0 failed |

Live evidence: `snp backup --help` in the built binary lists
`--output`, `--include-usage`, `--include-sync-state`, `--json` and **no**
`--format`.

**Scope note.** The full `scripts/check.sh` suite was not run in this pass;
verification was deliberately scoped to the targets the change set can
affect, per the session's instruction to keep verification bounded. The
suites above are the ones that exercise the removed module, the removed
flag, the swept suppressions, the search constructors, and the `snp data`
mirror. The remaining `scripts/check.sh` targets are unaffected by this
milestone and remain the pre-merge gate.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| The save path does not post-process TOML | `src/utils/toml_helpers.rs` was only touched to justify a retained suppression; no serializer behavior changed | pass |
| The library format carries no schema version | grep returns 0 hits for the former API; the rule now lives in `AGENTS.md` and `plans/000-long-term-specification.md` §5 | pass |
| Search parity across the three surfaces | `selector::searchable_text` was not touched; the two construction sites changed shape, not field set — `selector_integration` (23) and `mcp_integration` (4) pass | pass |
| Test seams stay `test-support`-gated | `src/test_failpoints.rs` was not touched; its 4 attributes are the expected baseline count | pass |
| No module-level suppression outside generated code | new guard in `tests/architecture.rs`, negative case proven | pass |
| Exit codes and the stdout/stderr split are unchanged | the only CLI change is a removed flag; `cli_surface_compat` (18) and `cli_usage_errors` scope unchanged | pass |

## 6. Failure and recovery review

- **Duplicate delivery / idempotency** — not applicable: the change set adds
  no delivery, retry, or background path.
- **Cancellation races** — not applicable.
- **Restart** — not applicable.
- **Partial persistence failure** — not applicable: no write path changed.
- **Stale generation or lease** — not applicable.
- **Contention and resource release** — not applicable: no lock path changed
  by this milestone.
- **Malformed or unauthorized input** — the removed `--format` flag was the
  only user-facing input surface removed; a client that still passes
  `--format` now receives the standard usage error (exit 2), which
  `cli_usage_errors` covers.
- **Bounded artifact behavior** — the deleted module was 711 lines of
  unreachable code; no artifact, log, or backup surface changed.

## 7. Migration and compatibility review

- **Schema migration** — none required and none performed. The persisted
  library format is byte-compatible with the baseline: the schema-version
  framework had no production caller, so no library file ever carried a
  schema-version key.
- **Backward compatibility** — a caller passing `snp backup --format` now
  gets a usage error instead of a silently ignored value. The flag had exactly
  one legal value, so no caller could have depended on a behavior change from
  it; the removal is a deliberate breaking change to a published CLI surface
  and is recorded in `CHANGELOG.md` under `### Removed`.
- **Protocol negotiation** — not applicable (no protocol change in this
  milestone).
- **Configuration behavior** — unchanged; `libraries.toml` parsing and the
  library registry were not touched.
- **Rollback limitations** — rolling back restores the dead module and the
  ignored `--format` flag. No persisted state depends on either, so the
  rollback is safe and lossless.
- **Legacy-path status** — the legacy single-file `snippets.toml` migration
  in `src/library/manager.rs` is untouched and still active.

## 8. Security review

- **Authorization enforcement** — unchanged.
- **Secret handling** — no secret is read, written, or transmitted. The
  removed framework carried no credential.
- **Path validation** — unchanged; no path-handling code was touched.
- **Privilege boundaries** — unchanged.
- **Denial-of-service bounds** — unchanged; 711 lines of unreachable code
  were removed, which does not alter any bound.
- **Redaction** — unchanged. No error text was added or modified, so the
  `SnipError::Command` display-layer redaction rule is untouched and no
  credential was introduced into any diagnostic.
- **Audit behavior** — unchanged.

## 9. Documentation and operations

Updated architecture documents: `architecture/overview.md`,
`architecture/output.md`, `architecture/persistence.md`,
`architecture/utils/atomic.md`, `architecture/utils/toml_helpers.md`,
`architecture/utils.md`, `architecture/library.md`, `architecture/selector.md`,
`architecture/commands/backup_cmd.md`, `architecture/commands/backup_archive.md`.

Updated contract and reference documents: `AGENTS.md`,
`plans/000-long-term-specification.md` §5, `docs/COMMANDS.md`,
`docs/COMMAND_CONTRACTS.md`, `docs/LOGICAL_LAYERS.md`,
`docs/PERSISTENCE_INVENTORY.md`, `docs/PUBLIC_API.md`.

Updated skill references: `.skills/selector-and-search-parity.md`,
`.skills/persistence-and-toml-fidelity.md`,
`.skills/transactions-and-auto-sync.md`, `.skills/remediation-patterns.md`,
`.skills/architecture-review.md`.

`CHANGELOG.md` records the removals under `### Removed` and the suppression
sweep under `### Changed`.

Static guard: `tests/architecture.rs` — a module-level `#![allow(dead_code)]`
under `src/` now fails the architecture suite, with the negative case proven
in the test itself.

Operator diagnostics: none added or removed. The one user-visible diagnostic
change is the usage error for a caller that still passes the removed flag.

Recovery instructions: none required. No persisted state, no journal, and no
lock semantics were touched.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | `remove_pid_if_unchanged` (`snip-sync/src/process.rs`) has two production callers (`snip-sync/src/main.rs:527`, `:574`). The plan's §3 evidence called it unreferenced. | The plan premise was wrong; the function was kept and its `allow(dead_code)` attribute justified in place. No impact on behavior. | None for this milestone. The plan's evidence section should not be treated as authoritative for this symbol. |
| low | `src/ui/_generated_bundled_themes.rs:13` carries `#![allow(dead_code, unused_imports)]`, emitted by `scripts/build_themes.py:133`. The plan's §3 evidence did not mention it. | The new guard would have failed on a generated file it must not police. The guard therefore excludes `@generated` files. | None. Recorded so a future change to the generator or the guard does not reintroduce the conflict. |
| low | Baseline suppression count is 43 outside `src/test_failpoints.rs`, not 44 (44 included one inside the deleted module). | Documentation-level discrepancy only. | None. |
| low | `architecture/overview.md` and `architecture/process_file_lock.md` still described the owned-record locks as releasing "without unlinking" on `Drop`, which is true of the kernel-lock tier but not of the owned-record locks. | Documentation inaccuracy. | Carried into the locking-convergence closure record (§10 of `plans/closure/locking-convergence/001-status.md`), where it is assigned to M003. |

## 11. Roadmap disposition

Milestone closed; the next dependency may proceed. M002 (the duplicate backup
copy policy) and M003 (the `Durability` taxonomy and the remaining
suppression candidates) remain dependency-ready to be planned.

## 12. Registry updates

- `plans/registry.md`: the maintenance-consolidation roadmap moves from
  `ready` to `active`; this milestone moves from `not started` to
  `closed`; the plan row moves from `ready` to `closed`.
- `plans/subsystems/maintenance-consolidation-roadmap.md`: §12 milestone
  status table updated — M001 `closed`, with the closure record linked; the
  milestone's completion note records the two falsified plan premises.

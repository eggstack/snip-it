# Maintenance and Duplicate-Policy Consolidation Roadmap

Status: active

> **2026-10-22 — M001 closed.** The milestone's implementation plan
> (`plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md`)
> was executed and its closure record is
> `plans/closure/maintenance-consolidation/001-status.md`. The roadmap changed
> only in its status field and in §12: the M001 objective, deliverable
> boundary, exit conditions, and milestone history are unchanged. The closure
> record records two plan premises the code falsified
> (`remove_pid_if_unchanged` is not dead; the generated themes file carries a
> module-level suppression the new guard must exclude) and one documentation
> defect carried forward to M003.

Long-term references:

- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#3-non-goals`
- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`
- `plans/002-long-term-roadmap.md#cross-phase-execution-rules`
- `plans/003-planning-process.md`

Related ADRs: none required. This work removes dead policy and converges
duplicated policy inside existing ownership boundaries. It does not change
exit codes, search semantics, conflict identity, retry ownership, or the
locking hierarchy. A future change to any of those opens an ADR.

## 1. Purpose and ownership boundary

This subsystem owns three forms of duplicated or unreachable policy inside the
`snp-it` client crate: (a) code that no production path reaches, (b) the same
operational policy implemented more than once with divergent behavior, and (c)
documentation that asserts behavior the code does not implement.

It consumes the command/exit-code contract, the persistence and durability
classes, and the transaction/local-data lock hierarchy. It MUST NOT own
locking-protocol convergence (that is `plans/subsystems/locking-convergence-roadmap.md`),
the sync cursor protocol (`plans/subsystems/sync-cursor-protocol-roadmap.md`),
new user-facing command capability
(`plans/subsystems/snippet-crud-surface-roadmap.md`), distribution, updater
transport, server runtime, or MCP registration.

## 2. Work classification

### Invariants

- A documented durability class continues to mean what its name says: user
  data that must survive power loss is fsynced; rollback points are not
  truncated by a crash.
- A snapshot or backup copy never silently traverses a symlink out of the
  configuration directory.
- Exit codes, stdout/stderr split, and machine-output schemas unchanged.
- The save path does not post-process `toml::to_string_pretty` output.

### Capabilities

- Unchanged user-visible behavior with less unreachable and less divergent
  policy. No new command, flag, or output field is introduced by this
  subsystem.

### Infrastructure

- One hardened copy primitive reused by every snapshot/backup path.
- One diagnostic vocabulary shared by the validate, doctor, and repair
  surfaces.

### Polish

- Dead code removed rather than retained behind `allow(dead_code)`.
- Documentation claims match the code they describe, in both directions.

## 3. Non-goals

- No new workspace crates, service/repository abstractions, DI frameworks,
  plugin/rule engines, databases/search indexes, generic query languages,
  background job systems, or networked MCP.
- No new command, subcommand, flag, or exit code.
- No new feature commands beyond the snippet CRUD surface owned elsewhere.
- No refactor of `src/transaction.rs` or `src/sync.rs` beyond the specific
  duplication named in a milestone.
- No change to the wire protocol, the persisted snippet schema, or the sync
  merge order.
- No deletion of behavior that is reachable but undocumented; reachable
  behavior is documented, not removed, unless a milestone says otherwise.

## 4. Current state

All six previously registered subsystem roadmaps are closed. The repository is
in a consolidation-saturated state: `plans/registry.md` records no pending
handoff, and the recent history is a sequence of audit-driven corrective
passes. That closure has left three structural gaps.

**Unreachable policy retained behind lint suppressions.** `src/migration.rs`
(711 lines) declares a schema-version migration framework. At the current
baseline it has zero production callers: `MigrationPlan`,
`MigrationOperation`, `MigrationOutput`, `SchemaVersion`, `Migration`,
`needs_migration`, `get_schema_version`, `write_schema_version`, and
`run_migrations` are each referenced only from inside `src/migration.rs`
itself. The module is reachable only through `pub(crate) mod migration;` in
`src/lib.rs`, and a file-level `#![allow(dead_code)]` suppresses the compiler
warning that would otherwise have surfaced the deadness. The persisted
library format carries no schema-version key, so the framework describes a
storage contract that does not exist. Two canonical documents nonetheless
carry a durable rule about it — `AGENTS.md` and
`plans/000-long-term-specification.md` §5 both require that
`write_schema_version` use `toml::Table` rather than `toml::Value` to
preserve array-of-tables — so the rule currently governs a function nothing
calls.

Smaller instances of the same pattern: `Durability::EphemeralCoordination`
in `src/utils/atomic.rs` is documented as the "lock files" class but
`atomic.rs` contains no locking code and the variant has no production
caller; `selector::SearchFields::with_output` is called only from unit tests
although two architecture documents state that the `list` and MCP surfaces
use it; `src/commands/backup_cmd.rs` accepts a `BackupFormat` argument and
then discards it with `let _ = format;`, and that enum has exactly one
variant whose value is already the default.

**One operational policy implemented four times with three behaviors.**
Backup and rollback material is produced by four separate code paths:
`library::persistence::backup_library` (per-library `.bak` siblings,
rotated, fsynced through `sync_backup_file`), `commands::backup_cmd::run`
(user-requested snapshot with canonical containment checks and an atomic
staging rename), `commands::repair_cmd::create_repair_backup` (whole-config
copy through `copy_dir_recursive`), and
`commands::restore_cmd::create_pre_restore_backup` (config-file copy through
`copy_regular_file`). Transaction journals add a fifth rollback mechanism
with its own rules. The security posture is not uniform: `restore_cmd`
explicitly refuses symlinks and non-regular files, and its own comment
names the hazard — `fs::copy` follows symlinks, so a library file that is a
symlink to a file outside the configuration directory would silently copy
that target's content into the snapshot — while `repair_cmd`'s
`copy_dir_recursive` and `persistence::backup_library` perform a plain
`fs::copy` with no such check. The durability posture is not uniform either:
`persistence::backup_library` fsyncs its copy and documents why a crash
immediately after the save would otherwise truncate the rollback point, while
the repair backup — the rollback point for a repair operation — is never
fsynced.

**Three diagnostic vocabularies for one repository.** `src/diagnostics.rs`
defines `DiagnosticSeverity` and `CompatibilityDiagnostic` for the doctor
surface. `src/commands/validate_cmd.rs` defines its own `Severity` enum
with an identical variant set and its own `ValidationDiagnostic` with a
typed `code`. `src/commands/repair_cmd.rs` defines `RepairItem` with a
free-text `category: String` where the other two use a typed code, and
re-scans library files through its own directory walk and load loop instead
of reusing the validate surface's library scan. Three vocabularies for the
same repository state means three places to update when a new diagnostic
appears, and no single place that can answer "what is wrong with this
library".

## 5. Target architecture

One hardened copy primitive in the atomic/persistence layer, parameterized by
whether the source is trusted, and reused by every snapshot and backup path;
every backup path that produces a rollback point fsyncs it.

One diagnostic vocabulary — a typed severity, a typed code, a typed
repairability — reused by the validate, doctor, and repair surfaces, with
each surface keeping its own report shape and output format while sharing
the vocabulary and the library scan.

Zero unreachable modules. A module that no production path reaches is
deleted, not retained behind `allow(dead_code)`. Documentation that describes
a deleted function is updated in the same change.

## 6. Dependency graph

```text
M001 Dead subsystem removal and documentation reconciliation
    |
    +--> M002 Backup and snapshot policy convergence (hard: M001)
    |
    `--> M003 Diagnostic vocabulary unification (soft: M001; hard: none)
```

M001 first, because it removes the misleading documentation that M002 and
M003 would otherwise be read against. M002 and M003 are independent of each
other and may proceed in either order.

## 7. Milestones

### M001 — Dead subsystem removal and documentation reconciliation

Class: polish

Objective: delete policy that no production path reaches, and make every
canonical documentation claim about that policy either true or removed.

Dependencies: none.

Deliverable boundary: deletion of `src/migration.rs` and the other
unreachable items named in the implementation plan; removal of the
`allow(dead_code)` suppressions that hid them; correction of
`AGENTS.md`, `plans/000-long-term-specification.md`, `architecture/output.md`,
`.skills/selector-and-search-parity.md`, and `docs/COMMANDS.md` to match the
code; deletion of the `snp backup --format` flag and its single-variant
enum; repair of the dangling `bugs.md` cross-references.

User or operator value: the documented rules describe behavior that exists,
so an agent or maintainer following the docs is not misled into preserving
or extending dead code.

Exit conditions: no `#[allow(dead_code)]` remains in the client crate except
where a specific reason is recorded inline; no canonical document references
a deleted symbol; `bash scripts/check.sh` passes; `docs/COMMANDS.md` and
`docs/COMMAND_CONTRACTS.md` agree with the `--help` output.

Deferred work: backup policy convergence (M002), diagnostic unification (M003).

Implementation plan:

- `plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md`

Closure record: `plans/closure/maintenance-consolidation/001-status.md`
(closed).

Predecessor flat plan: none.

### M002 — Backup and snapshot policy convergence

Class: invariant

Objective: make every snapshot, backup, and rollback-point path share one
copy policy that refuses to traverse a symlink out of the configuration
directory and that fsyncs the material it produces.

Dependencies: hard M001.

Deliverable boundary: one shared copy primitive in the atomic/persistence
layer; the four backup paths migrate to it; symlink refusal and fsync
become properties of the primitive rather than of one call site; the
repair backup becomes a valid rollback point.

User or operator value: a repair or restore can no longer copy content from
outside the configuration directory into a backup, and a rollback point
survives a power loss at the moment it was created.

Exit conditions: every copy of configuration or library material goes
through the shared primitive; a symlinked library file is refused by every
backup path, not only by restore; the repair backup is fsynced; the existing
restore-manifest and backup tests remain green.

Deferred work: none within this subsystem.

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

### M003 — Diagnostic vocabulary unification

Class: infrastructure

Objective: give the validate, doctor, and repair surfaces one shared
diagnostic vocabulary and one shared library scan, so that the same
repository state is described the same way wherever it is inspected.

Dependencies: soft M001 (documentation context), hard none.

Deliverable boundary: a shared severity/code/repairability vocabulary; the
repair surface consumes the shared library scan instead of its own walk;
each surface keeps its own report type and output format; no diagnostic
code, message, or exit code changes.

User or operator value: `snp validate`, `snp doctor`, and `snp repair` agree
on what is wrong with a library, and a new diagnostic type needs one change
rather than three.

Exit conditions: one severity vocabulary and one code namespace are used by
all three surfaces; the repair surface no longer enumerates library files
independently; JSON output shapes are unchanged; `snp data validate`,
`snp data repair`, and `snp data doctor` still mirror their top-level
spellings.

Deferred work: none within this subsystem.

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

## 8. Cross-cutting requirements

### Storage and migration

No schema migration. The persisted snippet format, the library index, and
the sync state directory are unchanged. M001 deletes a migration framework
that was never applied to any stored artifact; it MUST NOT introduce a
replacement schema-version mechanism, because no stored artifact carries a
schema version today.

### Protocol and compatibility

Command spellings, aliases, exit codes, stdout/stderr split, and
machine-output schemas are unchanged. The `snp data` mirror group keeps its
five single-path handlers. The sync wire protocol is untouched.

### Security and authorization

M002 is the security-bearing milestone: a copy that traverses a symlink out
of the configuration directory can read a file the user never intended to
place in a backup. The shared primitive refuses such a source by default and
records the refusal as a diagnosable event rather than a silent skip. No
credential, sync metadata, or secret is written into a diagnostic message;
redaction remains a `SnipError::Command` display-layer concern only.

### Concurrency, cancellation, and recovery

M002 must not weaken the rollback guarantees of the transaction journal. A
backup that fails must fail the operation that requested it, or be reported
as a skipped entry with a reason — never leave a partially copied rollback
point presented as complete.

### Observability and audit

M003 preserves the exact-count assertion discipline. A unified vocabulary
must not merge two distinct diagnostics into one row. The audit log and the
delete audit record are unaffected.

### Performance and resource use

No new runtime systems. M003's shared library scan replaces a second
directory walk rather than adding one, so the scan count must not increase.

### Documentation and operations

`AGENTS.md`, `architecture/overview.md`, `architecture/output.md`,
`architecture/persistence.md`, `architecture/commands/backup_cmd.md`,
`architecture/commands/restore_cmd.md`,
`architecture/commands/doctor_cmd.md`, `architecture/commands/repair_cmd.md`,
`architecture/commands/validate_cmd.md`, `docs/COMMANDS.md`,
`docs/COMMAND_CONTRACTS.md`, `docs/LOGICAL_LAYERS.md`, and
`.skills/selector-and-search-parity.md` are updated with the code they
describe. The deep-dive index in `architecture/overview.md` is updated when a
module is added or removed.

## 9. Verification strategy

Static absence checks for the deleted symbols; `cargo clippy --workspace
--all-targets -- -D warnings` with no surviving blanket
`allow(dead_code)`; a snapshot-path matrix test that runs the same symlink
fixture against every backup path and asserts refusal; a crash-consistency
test that asserts the repair backup is fsynced; a diagnostic-vocabulary test
asserting that the validate, doctor, and repair surfaces agree on severity
and code for a shared fixture; `scripts/check.sh`;
`scripts/ci/test-production-seams.sh`.

## 10. Risks and decision points

- M002 changes what a backup refuses to copy. If an existing user relies on
  a symlinked library file being backed up, the new refusal is visible
  behavior. Decision: refuse and report, because the current asymmetry is
  itself the defect; the refusal message names the path.
- M003 must not collapse distinct diagnostics to simplify the vocabulary. If
  a merge proves necessary for a specific code, that code's identity is
  preserved via its existing string form.
- M001 deletes code. If any consumer outside this repository depends on
  `snp backup --format` as a flag, its removal is a breaking CLI change.
  Decision: the flag is removed with the deprecation notice required by
  `.skills/cli-and-outcome-contract.md`; the single legal value was the
  default, so no behavior changes.
- No decision in this subsystem requires an ADR. A decision that would alter
  exit codes, search semantics, conflict identity, retry ownership, or the
  locking hierarchy must be escalated to an ADR before implementation.

## 11. Completion definition

This workstream closes when M001, M002, and M003 each have accepted closure
evidence showing that unreachable policy is deleted rather than suppressed,
that every backup path shares one copy policy with symlink refusal and
fsync, that the validate/doctor/repair surfaces share one vocabulary and one
library scan, and that no unresolved medium-or-higher finding remains.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 dead subsystem removal + doc reconciliation | closed | `plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md` | `plans/closure/maintenance-consolidation/001-status.md` | — |
| M002 backup and snapshot policy convergence | not started | — | — | M001 closed |
| M003 diagnostic vocabulary unification | not started | — | — | — |

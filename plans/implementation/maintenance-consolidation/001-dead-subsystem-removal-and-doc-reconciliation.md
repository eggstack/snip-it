# Maintenance and Duplicate-Policy Consolidation Milestone 001 — Dead subsystem removal and documentation reconciliation

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/maintenance-consolidation-roadmap.md#m001--dead-subsystem-removal-and-documentation-reconciliation`

Long-term requirements:

- `plans/000-long-term-specification.md#3-non-goals`
- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`

Applicable ADRs: none.

Primary class: polish

## 1. Objective

Delete policy that no production path reaches, and make every canonical
documentation claim about that policy either true or removed.

This milestone changes no user-visible behavior. Every deletion below is of
code that no production path reaches, or of a flag whose only legal value
was already the default.

## 2. Why this milestone is ready

- No hard dependencies. The six registered subsystem roadmaps are closed and
  no closed milestone depends on any of the code this milestone removes.
- No unresolved decision requires an ADR: the milestone removes unreachable
  code and reconciles documentation. It does not change exit codes, search
  semantics, conflict identity, retry ownership, or the locking hierarchy.
- The milestone is one coherent pass: a deletion sweep plus the
  documentation reconciliation that the deletions force.
- The registry records no pending handoff, so this milestone has the
  repository to itself.

## 3. Current implementation evidence

At the baseline, `src/migration.rs` is 711 lines declaring a schema-version
migration framework. Every public symbol in it — `SchemaVersion`,
`MigrationPlan`, `MigrationOperation`, `MigrationOutput`, `Migration`,
`needs_migration`, `get_schema_version`, `write_schema_version`,
`run_migrations` — is referenced only from inside `src/migration.rs`. The
only path to the module is `pub(crate) mod migration;` in `src/lib.rs`, and
a file-level `#![allow(dead_code)]` at the top of `src/migration.rs`
suppresses the compiler warning that would otherwise have surfaced the
deadness.

The persisted library format carries no schema-version key: a search for
`schema_version` across `src/` and `snip-sync/src/` finds it only in
`src/auto_sync/pending.rs`, `src/auto_sync/status.rs`,
`src/auto_sync/execution_lock.rs`, `src/transaction.rs`, and
`src/commands/validate_cmd.rs`, none of which relate to the library
schema. The framework therefore describes a storage contract that does not
exist.

Two canonical documents carry a durable rule about this dead function.
`AGENTS.md` states that "`write_schema_version` must use `toml::Table`, not
`toml::Value`, to preserve array-of-tables", and
`plans/000-long-term-specification.md` §5 states the same requirement. Both
rules currently govern a function nothing calls.

Smaller instances of the same pattern at the baseline:

- `Durability::EphemeralCoordination` in `src/utils/atomic.rs` is documented
  as the "Lock files" durability class, but `atomic.rs` contains no locking
  code and the variant has no production caller.
- `selector::SearchFields::with_output` in `src/selector.rs` is called only
  from unit tests. `src/commands/list_cmd.rs` and `src/mcp/tools.rs` each
  construct a `SearchFields` value inline. `architecture/output.md` and
  `.skills/selector-and-search-parity.md` both state that those surfaces
  use `with_output()`.
- `src/commands/backup_cmd.rs` accepts a `BackupFormat` argument and then
  discards it with `let _ = format;`. `BackupFormat` in
  `src/commands/backup_archive.rs` has exactly one variant, `Directory`,
  which is already the default. `src/main.rs` constructs
  `BackupFormat::Directory` at both call sites.
- `src/library/tests.rs` and `src/migration.rs` cite `bugs.md` items 11, 12,
  13, and 18. No `bugs.md` exists in the repository; the only similarly
  named files are `.github/ISSUE_TEMPLATE/bug_report.md` and
  `docs/archive/SECURITY_AUDIT.md`.
- `auto_sync::execution_lock::WorkerLock` has no production acquirer. Its
  metadata is read by `src/status_snapshot.rs` and its path is surfaced by
  `src/auto_sync/mod.rs` and `src/commands/sync_cmd.rs`. The architecture
  documents disagree: `architecture/process_file_lock.md` lists the tier as
  a live consumer, while `architecture/auto_sync.md` records that no
  production path acquires it. The tier's retirement belongs to
  `plans/subsystems/locking-convergence-roadmap.md` M003; this milestone
  removes only the unreferenced helpers it owns.
- `snip-sync/src/process.rs` carries `remove_pid_if_unchanged` behind
  `allow(dead_code)`; the server no longer writes PID files.

At the baseline there are 44 `allow(dead_code)` attributes outside
`src/test_failpoints.rs`: 15 in `src/ui/theme.rs`, 7 in
`src/utils/variables.rs`, 7 in `src/utils/atomic.rs`, 4 in
`src/transaction.rs`, 4 in `src/output.rs`, 2 in `src/ui/state.rs`, and one
each in `src/utils/toml_helpers.rs`, `src/migration.rs`, `src/logging.rs`,
`src/local_data.rs`, and `snip-sync/src/process.rs`. The four in
`src/test_failpoints.rs` are test-support gated and stay.

Known gaps at the baseline: the deadness is invisible to the compiler
because of the blanket suppressions; the documentation asserts rules about
code that does not run; and `cargo clippy --workspace --all-targets --
-D warnings` passes today only because of those suppressions.

## 4. Invariants that must not regress

- Exit codes, stdout/stderr split, and machine-output schemas unchanged.
- Command spellings, aliases, and the `snp data` mirror group unchanged
  except for the single removal named in this plan.
- The save path does not post-process `toml::to_string_pretty` output.
- `Durability::DurableUserData` continues to mean `sync_all` before rename;
  `RecoverableMetadata` continues to mean no fsync.
- The `snp data` mirror group keeps its five single-path `handle_*`
  functions and its `command_behavior()` entries.
- Every `#[allow(dead_code)]` that survives carries an inline comment
  stating why the item is retained.
- No test seam loses its `test-support` gating. The four
  `allow(dead_code)` attributes in `src/test_failpoints.rs` are retained and
  their gating is unchanged.
- No credential, sync metadata, or secret appears in a diagnostic message.

## 5. Scope

### In scope

- Delete `src/migration.rs` and its `pub(crate) mod migration;` declaration
  in `src/lib.rs`.
- Remove the `write_schema_version` rule from `AGENTS.md` and from
  `plans/000-long-term-specification.md` §5, and replace it with the rule
  that the library format carries no schema version and that introducing
  one is a schema change requiring a migration and a version bump.
- Delete `Durability::EphemeralCoordination` from `src/utils/atomic.rs`, or,
  if a production caller is identified during implementation, give it the
  lock behavior its documentation claims. The default is deletion.
- Delete `selector::SearchFields::with_output` if it remains
  test-only after the inline construction sites are switched to use it, or
  switch `src/commands/list_cmd.rs` and `src/mcp/tools.rs` to use it and
  then delete it. Exactly one of those two outcomes is acceptable, not both.
- Correct `architecture/output.md` and `.skills/selector-and-search-parity.md`
  to describe whichever construction the code ends up using.
- Remove the `--format` flag from the `snp backup` command in
  `src/main.rs`, remove `BackupFormat` from
  `src/commands/backup_archive.rs`, and remove the `let _ = format;`
  discard in `src/commands/backup_cmd.rs`.
- Update `docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, and
  `docs/EXIT_CODES.md` if they describe the `--format` flag.
- Repair the dangling `bugs.md` cross-references in `src/library/tests.rs`
  and `src/migration.rs` (the latter disappears with the module) by naming
  the behavior each test pins rather than an external document that is not
  in the repository.
- Remove the `allow(dead_code)` from `snip-sync/src/process.rs`'s
  `remove_pid_if_unchanged` by deleting the function, if it has no
  remaining caller.
- Sweep the remaining `allow(dead_code)` attributes outside
  `src/test_failpoints.rs` and either delete the item or attach an inline
  comment recording the reason it is retained. This is a sweep, not a
  mandate: an item that is genuinely kept for a near-term caller keeps its
  attribute, with the reason written down.
- Add a static guard that fails when a new module-level
  `#![allow(dead_code)]` is introduced in `src/`.

### Explicitly out of scope

- The `WorkerLock` tier's retirement, and the lock-ordering contract.
  Those belong to `plans/subsystems/locking-convergence-roadmap.md` M003.
- The backup copy policy and symlink refusal. Those belong to
  `plans/subsystems/maintenance-consolidation-roadmap.md` M002.
- The diagnostic vocabulary unification, which belongs to M003 of the same
  roadmap.
- Any schema-version mechanism. This milestone removes the dead framework;
  it does not build a replacement.
- Any new schema-version key in the persisted library format.
- The `snp backup --format` flag's replacement. The milestone removes a
  flag with one legal value; it does not add an archive format.
- Any change to the `snp data` mirror group's other five commands.
- Any refactor of `src/transaction.rs` or `src/sync.rs` beyond the specific
  `allow(dead_code)` sweep.

## 6. Required production changes

### Core/domain

`src/migration.rs` is deleted. `src/lib.rs` loses its `pub(crate) mod
migration;` declaration. No domain module gains or loses a public item as
a result of the deletion beyond the items named in the scope.

### Storage and migrations

No schema change. The persisted library format is untouched and continues to
carry no schema-version key. No migration is required, and none may be
introduced.

### Protocol and DTOs

No protocol change. `snip-proto` and `snip-sync` are unaffected except for
the deletion of an unreferenced function in `snip-sync/src/process.rs`.

### Runtime and concurrency

No runtime change. Nothing in the deletion set participates in a lock,
a mutation gate, or a transaction.

### Frontend or operator surface

One CLI change: the `snp backup --format` flag is removed. It had exactly
one legal value, which was already the default, so no invocation that was
valid before becomes invalid in meaning — only in spelling. Per
`.skills/cli-and-outcome-contract.md`, the removal is recorded in
`docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, and `CHANGELOG.md`, and the
help output is re-captured.

### Security and authorization

No credential handling change. The deletion set contains no secret-bearing
code path. The removal of the dead migration framework removes a code path
that could have written a schema-version key; since no stored artifact
carries such a key today, no stored artifact is affected.

### Documentation and static guards

- `AGENTS.md`: replace the `write_schema_version` rule with the statement
  that the library format carries no schema version, and that introducing
  one is a schema change requiring a migration plan and a version bump.
- `plans/000-long-term-specification.md` §5: the same replacement, in the
  durability-invariants voice.
- `architecture/output.md` and `.skills/selector-and-search-parity.md`:
  correct the `SearchFields::with_output` claim to match the code.
- `architecture/overview.md`: the deep-dive index is updated if a module
  disappears, and §7's cross-cutting invariants are updated if the
  `write_schema_version` rule appears there.
- `docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`: the `--format` flag is
  removed.
- `CHANGELOG.md`: the flag removal is recorded under the unreleased
  section.
- `tests/architecture.rs`: a static guard asserting that no module in
  `src/` carries a module-level `#![allow(dead_code)]`.

## 7. Ordered work packages

### Work package A — Remove the dead migration framework

Intent: delete code that no production path reaches, and remove the
canonical rules that govern it.

Required changes: delete `src/migration.rs`; remove the module declaration
from `src/lib.rs`; remove the `write_schema_version` rule from `AGENTS.md`
and from `plans/000-long-term-specification.md` §5, replacing each with the
"no schema version today; introducing one is a schema change" statement;
check `architecture/overview.md` and any other canonical document for a
reference to the module and remove it.

Acceptance evidence: a repository-wide search for `migration::`,
`write_schema_version`, `get_schema_version`, `needs_migration`,
`run_migrations`, `MigrationPlan`, `MigrationOperation`, `MigrationOutput`,
and `SchemaVersion` returns no hits outside `plans/` history; `cargo clippy
--workspace --all-targets -- -D warnings` passes with the module-level
suppression gone.

### Work package B — Remove the remaining unreferenced policy

Intent: eliminate the smaller dead items and the false documentation claims
around them.

Required changes: handle `Durability::EphemeralCoordination`,
`SearchFields::with_output`, `snip-sync/src/process.rs`'s
`remove_pid_if_unchanged`, and the `BackupFormat` enum together with the
`--format` flag and the `let _ = format;` discard; correct
`architecture/output.md` and `.skills/selector-and-search-parity.md` to
match whichever `SearchFields` construction the code ends up using; update
`docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, and `CHANGELOG.md` for the
flag removal.

Acceptance evidence: `cargo clippy --workspace --all-targets -- -D warnings`
passes; the `snp backup --help` output no longer lists `--format`; the
`snp backup` command still produces a directory snapshot; the corrected
architecture and skill documents describe the code as it now stands.

### Work package C — Sweep the surviving lint suppressions

Intent: make every remaining `allow(dead_code)` an explicit, justified
decision rather than an inherited one.

Required changes: walk the 44 attributes outside `src/test_failpoints.rs`;
for each, either delete the item or keep it with an inline comment stating
the reason it is retained; leave the four in `src/test_failpoints.rs`
untouched, since they are test-support gated; do not introduce a
module-level blanket suppression anywhere.

Acceptance evidence: a repository-wide search for `allow(dead_code)` shows
only individually justified occurrences, each with an adjacent comment; the
`src/test_failpoints.rs` occurrences are unchanged; the build is warning
free under `-D warnings`.

### Work package D — Repair the dangling cross-references

Intent: remove references to a document that is not in the repository.

Required changes: replace the `bugs.md #11`, `#12`, `#13`, and `#18`
citations in `src/library/tests.rs` section headers with a description of
the behavior each section pins; make the same replacement in any other
location that cites a document absent from the repository.

Acceptance evidence: no source file cites a document that does not exist in
the repository; the affected test sections still pass.

### Work package E — Add the static guard and update the indexes

Intent: prevent the same deadness from reappearing silently.

Required changes: add to `tests/architecture.rs` an assertion that no
module under `src/` declares a module-level `#![allow(dead_code)]`; update
the deep-dive index in `architecture/overview.md`; update
`docs/LOGICAL_LAYERS.md` if the layer membership changes — it does not
change in this milestone, but the document and the test's constant lists
must be checked together per their own instruction.

Acceptance evidence: the guard fails when a synthetic module-level
suppression is introduced; `tests/architecture.rs` passes as committed;
`docs/LOGICAL_LAYERS.md` and `tests/architecture.rs` agree.

## 8. Failure, cancellation, restart, and contention semantics

This milestone deletes code and documentation; it introduces no runtime
behavior, so failure, cancellation, restart, and contention semantics are
unchanged. The two behaviors that must be preserved across the deletion
set are:

- `snp backup` continues to produce the same directory snapshot it produced
  before, at the same location, with the same contents.
- The `Durability` enum's remaining variants keep their exact meanings.
  `DurableUserData` still fsyncs before rename; `SensitiveConfig` still sets
  `0o600` on Unix and rejects symlinks; `RecoverableMetadata` still does
  not fsync. Deleting `EphemeralCoordination` must not change any of the
  other three.

If deleting `Durability::EphemeralCoordination` turns out to change a
caller's durability class, the deletion is wrong and the milestone stops to
report rather than proceeding.

## 9. Compatibility and migration

No data migration. No stored artifact is read or written differently by
this milestone.

The one user-visible compatibility change is the removal of
`snp backup --format`. The flag accepted exactly one value, which was
already the default, so no behavior changes for any invocation that
remains valid. The removal is recorded in `CHANGELOG.md` and in
`docs/COMMANDS.md`. Per `.skills/cli-and-outcome-contract.md`, the removal
is treated as a deprecation-eligible change; if the project prefers to
retain the spelling for one release, the implementation may keep the flag
as a hidden no-op for one release and remove it in the next, but the
removal must then be recorded with both dates. The default is immediate
removal, because the flag never selected a different behavior.

No configuration fallback is needed. No protocol negotiation is needed.

## 10. Required tests

### Focused unit tests

- A test asserting that the `snp backup` command still writes a directory
  snapshot with the expected manifest after the flag removal.
- A test asserting that the `Durability` enum's remaining variants produce
  the expected filesystem effects, if the existing coverage does not already
  pin them.

### Integration tests

- `tests/architecture.rs` extended with the module-level suppression guard.
- A test that the `snp backup` help output no longer contains `--format`
  and that `snp backup` still succeeds.

### Restart and recovery tests

None. This milestone introduces no state machine.

### Contention and cancellation tests

None. This milestone touches no lock.

### Security and negative tests

- A test that a source tree containing a module-level `#![allow(dead_code)]`
  fails the architecture guard, so the guard's negative case is proven
  rather than assumed.

### Migration and compatibility tests

- A test that the persisted library format is unchanged: a library written
  before the change still loads, and a library written after the change
  still loads with a build from before the change. This proves the
  deletion set did not touch the persisted schema.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test architecture --features test-support

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

Note that `cargo clippy --workspace --all-targets --all-features` is run
locally in addition to `scripts/check.sh`, because `check.sh` does not pass
`--all-features` and the `test-support`-gated targets are not linted by it.

## 12. Documentation updates

- `AGENTS.md` — replace the `write_schema_version` rule.
- `plans/000-long-term-specification.md` §5 — the same replacement.
- `architecture/overview.md` — deep-dive index and any cross-cutting
  invariant that names the deleted module.
- `architecture/output.md` — the `SearchFields` construction claim.
- `architecture/persistence.md` — only if it names the schema-version
  framework.
- `.skills/selector-and-search-parity.md` — the `SearchFields`
  construction claim.
- `docs/COMMANDS.md` — the `snp backup` flag list.
- `docs/COMMAND_CONTRACTS.md` — the `snp backup` row.
- `CHANGELOG.md` — the flag removal.
- `docs/LOGICAL_LAYERS.md` — only if layer membership changes.

## 13. Acceptance criteria

1. `src/migration.rs` does not exist and no symbol it declared is
   referenced anywhere in the workspace.
2. No canonical document states a rule about `write_schema_version`; the
   replacement rule about the absence of a schema version is present in
   both `AGENTS.md` and `plans/000-long-term-specification.md` §5.
3. `Durability` has no variant that claims lock-file behavior while
   `atomic.rs` contains no lock code, and no production caller of a
   removed variant remains.
4. Exactly one `SearchFields` construction convention exists in the code,
   and both `architecture/output.md` and
   `.skills/selector-and-search-parity.md` describe it accurately.
5. `snp backup --help` does not list `--format`, `snp backup` still
   produces a directory snapshot, and the removal is recorded in
   `docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, and `CHANGELOG.md`.
6. No source file cites a document that does not exist in the repository.
7. Every remaining `allow(dead_code)` outside `src/test_failpoints.rs`
   carries an adjacent comment stating why it is retained, and no
   module-level blanket suppression exists anywhere in `src/`.
8. A static guard fails when a module-level `#![allow(dead_code)]` is
   introduced under `src/`.
9. `cargo clippy --workspace --all-targets -- -D warnings` passes.
10. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
    pass.
11. The persisted library format is unchanged in both directions, proven by
    a cross-version load test.
12. No exit code, machine-output schema, or command spelling other than the
    named `--format` flag changed.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- a hard dependency is absent;
- deleting an item turns out to require a schema change or a migration —
  that would mean the item was reachable, and the milestone's premise is
  wrong;
- `Durability::EphemeralCoordination` turns out to have a production
  caller whose behavior depends on it;
- the `snp backup --format` removal would change behavior for an
  invocation other than the default value;
- the scope would expand into the locking, backup-policy, or
  diagnostic-vocabulary milestones of the same roadmap;
- repository evidence contradicts the claim that `src/migration.rs` is
  unreachable.

## 15. Closure evidence required

- A statement, with the search commands used, that every symbol deleted in
  this milestone has no remaining reference.
- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh` results.
- The `snp backup --help` output before and after, showing the flag removal
  and that the remaining flags are unchanged.
- The cross-version library load test result.
- The architecture guard's negative-case result, proving the guard fails
  when a module-level suppression is introduced.
- A list of every surviving `allow(dead_code)` with its inline justification,
  or a confirmation that none survive outside `src/test_failpoints.rs`.
- The exact `CHANGELOG.md` entry.

## 16. Handoff notes

- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes
  that appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another
  test run is in flight. The integration tests execute
  `target/debug/snp`; a rebuild mid-run replaces that binary and produces
  false failures.
- `src/test_failpoints.rs` is compiled out of production builds without the
  `test-support` feature. Its four `allow(dead_code)` attributes are
  expected and must not be swept.
- `scripts/ci/test-production-seams.sh` greps the production binary for the
  test seam variable names. Removing a module must not remove a seam.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- `docs/LOGICAL_LAYERS.md` and `tests/architecture.rs` must be updated
  together; one of them is otherwise lying.
- The deep-dive index in `architecture/overview.md` covers the architecture
  documents. If a deep dive is deleted rather than corrected, remove it
  from the index in the same change.

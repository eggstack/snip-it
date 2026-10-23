# Snippet CRUD Surface Milestone 001 — Non-interactive snippet delete

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/snippet-crud-surface-roadmap.md#m001--non-interactive-snippet-delete`

Long-term requirements:

- `plans/000-long-term-specification.md#1-product-definition`
- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#7-search-selector-and-mcp-invariants`
- `plans/001-terminology-and-domain-model.md#3-snippet-terms`
- `plans/001-terminology-and-domain-model.md#6-selection-and-outcome-terms`

Applicable ADRs: none.

Primary class: capability

## 1. Objective

Give a script a deterministic way to tombstone a snippet, producing exactly
the persisted state the existing interactive delete path produces.

## 2. Why this milestone is ready

- No hard dependencies. The snippet CRUD surface has no upstream milestone.
- No unresolved decision requires an ADR: the milestone adds a command over
  existing persisted fields and the existing merge semantics. It does not
  change the persisted schema, the synced field set, conflict identity, the
  merge order, exit-code policy, or the MCP read-only boundary.
- The persisted behavior to reproduce already exists and is correct. The
  interactive path's tombstone, timestamp, audit, and auto-sync sequence
  are already implemented; this milestone extracts and reuses it rather
  than designing new semantics.
- The selector contract, the mutation gate, the local-data lock, the audit
  log, and the auto-sync notification path all exist and are used by other
  commands.

## 3. Current implementation evidence

Snippet deletion is reachable from exactly one place. The delete action is
produced by the TUI key handler in `src/ui/mod.rs` and consumed by the
selection loop in `src/commands/mod.rs`. There is no `snp delete` command,
no `--delete` flag on any command, and no non-interactive path to a
tombstone. `src/commands/mod.rs` is the only consumer of the
`ui::SnippetSelection::Delete` variant, and `src/ui/mod.rs` is the only
producer.

The interactive path is careful, and its sequence is the specification this
milestone reuses:

1. The picker returns a display index, which is mapped back to the
   library index through the `original_indices` table.
2. The target snippet's identifier is read from the library.
3. The deletion happens through `library::update_library(&lib_path, |current| ...)`,
   which runs the mutation gate, takes the local-data lock, re-reads the
   library inside the critical section, and applies the mutation. The
   comment in the code records why: the library is re-read inside the
   critical section so that a snippet created elsewhere while the picker
   was open is not overwritten by the picker's in-memory copy.
4. Inside the critical section the target is located by identifier with
   `!s.deleted`, and its tombstone is set: `deleted = true` and
   `updated_at = updated_at.max(now).saturating_add(1)`. The saturating
   add makes the tombstone timestamp strictly greater than the live
   version, which is what makes deletion beat live content in the client
   merge.
5. The picker's in-memory copy is mirrored via `mark_snippet_deleted`.
6. An audit entry is written with the action `delete` and the deleted
   snippet. A failed audit write is logged at debug level and does not fail
   the deletion.
7. The sync trigger runs: either an explicit sync when the caller asked
   for one, or an auto-sync notification with `MutationKind::SnippetDelete`
   and `MutationOrigin::User` after the commit succeeds.

Steps 3, 4, 6, and 7 are the behavior a non-interactive delete must
reproduce exactly. Step 5 is specific to the picker and does not apply.

The selector contract already provides the target resolution this command
needs. `selector::resolve_selector` is the mutating member of the selector
family — it runs `ensure_library_mode()`, which is appropriate here because
the command mutates — and it returns a `SelectionResult` distinguishing one
match, many matches, not found, and ambiguous. The read-only sibling
`resolve_selector_readonly` is not appropriate for a mutating command.

The `snp data` mirror group in `src/main.rs` has five variants — validate,
backup, restore, repair, and status — and each maps to a single-path
`handle_*` function. A new top-level command that mutates local state needs
a `DataCommands` variant, a dispatch arm, and a `command_behavior()` arm.
`command_behavior()` assigns each command a startup-recovery policy and a
service level; mutating commands take `StartupRecoveryPolicy::Allow` and a
non-minimal service level, and the read-only commands take
`StartupRecoveryPolicy::SuppressReadOnly` with `StartupServices::Minimal`.

Known gaps at the baseline: a script cannot delete a snippet; the delete
sequence is embedded in a TUI selection loop rather than being a reusable
primitive; and `docs/COMMANDS.md` describes `snp get` as the scripting entry
point while offering no scripting delete.

## 4. Invariants that must not regress

- Snippet identifiers are deterministic and are not reassigned by a delete.
  The tombstone keeps the snippet's existing identifier.
- Deletion is a tombstone, not a row removal. The `deleted` flag is set and
  the tombstone is preserved for sync.
- The tombstone timestamp is strictly greater than the live version's
  timestamp. The client merge continues to treat deletion as beating live
  content.
- The mutation gate runs before the local mutation. A corrupt or
  multi-journal state refuses the mutation and directs the user to
  `snp repair`.
- The mutation re-reads the library inside the local-data lock rather than
  writing a stale in-memory copy.
- The mutation is persisted through the durable atomic write path.
- An audit entry is written naming the action and the target. A failed
  audit write does not fail the mutation.
- The auto-sync notification fires only after the mutation is durable, with
  `MutationKind::SnippetDelete` and `MutationOrigin::User`.
- An ambiguous selector deletes nothing and reports the ambiguity.
- Exit codes, stdout/stderr split, and the machine-output rules of
  `docs/EXIT_CODES.md` and `docs/CLI_EXITCODE_STREAM_POLICY.md` are
  unchanged.
- The `snp data` mirror group continues to mirror every top-level command
  it mirrors today.
- No credential, sync metadata, or secret appears in a diagnostic message.
- MCP remains read-only, stdio-only, non-executing, and noninteractive.

## 5. Scope

### In scope

- Extract the delete sequence from the TUI selection loop into a shared
  snippet-delete primitive in the library or commands layer, and have the
  TUI path call it.
- Add a `snp delete` command that resolves its target through the existing
  selector contract and applies the shared primitive.
- Add the `snp data delete` mirror variant, its dispatch arm, and its
  `command_behavior()` arm.
- Support the selector inputs the other snippet commands support, so a
  target can be named by exact identifier, exact description, exact
  command, or a filter.
- Support a JSON output mode whose shape is documented in
  `docs/JSON_SCHEMAS.md`.
- Add the `docs/COMMAND_CONTRACTS.md` row and the `docs/COMMANDS.md`
  section for the new command.
- Add the `architecture/commands/delete_cmd.md` deep dive and register it in
  the `architecture/overview.md` deep-dive index.
- Add tests that the interactive and non-interactive paths produce identical
  persisted state.

### Explicitly out of scope

- The `folders` and `favorite` write path, which belongs to M002.
- Non-interactive description, command, and tag updates, which belong to
  M003.
- Any change to the persisted schema. The `deleted` flag already exists.
- Any change to the sync merge order, the fingerprint field set, or the
  deterministic identifier derivation.
- Any MCP tool. The MCP boundary is read-only by design and is not widened
  here.
- Any bulk delete, a `--force` flag, a confirmation prompt, or a
  `--recursive` flag.
- Any change to the existing `snp edit` behavior.
- Any change to the interactive TUI's keybindings or layout. The TUI keeps
  its `d` key and its delete hint.
- The `WorkerLock` retirement, the lock ordering contract, and the backup
  copy policy, which belong to other roadmaps.

## 6. Required production changes

### Core/domain

One shared snippet-delete primitive. It takes a target identifier and a
library path and performs, in one place: the mutation gate through the
existing `library::update_library` path; the local-data lock; the re-read
inside the critical section; the lookup by identifier with `!deleted`; the
tombstone with the strictly-advancing timestamp; the durable save; the
audit entry; and the sync trigger.

The TUI selection loop calls this primitive instead of inlining the
sequence. The primitive does not contain any picker-specific logic — no
display index, no in-memory mirror — so it is usable from a command.

The primitive returns enough information for the caller to report what
happened: the deleted snippet's identifier and description, so the JSON
mode can name the target.

### Storage and migrations

No schema change. The `deleted` flag and the tombstone timestamp already
exist in the persisted format. No migration is required.

### Protocol and DTOs

No sync-protocol change. The tombstone is already synced as an ordinary
row with `deleted` set.

### Runtime and concurrency

The delete runs under the local-data lock and re-reads the library inside
the critical section. The target is located by identifier, not by index, so
a concurrent insertion cannot shift the target.

If the target has already been deleted by another process between
resolution and mutation, the mutation reports a distinct not-found outcome
rather than a generic failure.

The audit write remains best-effort: a failure is logged and does not fail
the deletion.

The auto-sync notification fires after the commit succeeds. It is not fired
when the mutation fails.

### Frontend or operator surface

A new `snp delete` command. Its arguments follow the conventions the
existing commands use for selector inputs, and it accepts a JSON output
mode.

The command has no interactive confirmation. A confirmation prompt would
defeat the purpose of a non-interactive command, and the selector's
ambiguity rules are what protect against deleting the wrong target.

The command's exit code follows `docs/EXIT_CODES.md`. Its stdout and stderr
split follows `docs/CLI_EXITCODE_STREAM_POLICY.md`: the human-readable
result goes to stdout, and diagnostics go to stderr.

### Security and authorization

The command is a new mutation surface and runs the mutation gate before
touching local state, exactly as every other mutation does. The `snp data`
mirror arm carries the same `command_behavior()` entry as the top-level
spelling, so the mirror is not a way to bypass the startup-recovery policy.

No credential is read, written, or printed. The command does not accept a
credential-bearing argument.

### Documentation and static guards

- `docs/COMMANDS.md` — the new command's section, in the same form as the
  other commands.
- `docs/COMMAND_CONTRACTS.md` — the new command's row.
- `docs/JSON_SCHEMAS.md` — the JSON output shape.
- `docs/USER_GUIDE.md` — a short mention in the scripting workflow.
- `docs/EXIT_CODES.md` — only if the command uses a code not already
  listed.
- `architecture/commands/delete_cmd.md` — a new deep dive.
- `architecture/overview.md` — the deep-dive index entry.
- `architecture/cli.md` — the command dispatch chain.
- `architecture/selector.md` — only if the command's selector inputs differ
  from the family's existing ones.
- `CHANGELOG.md` — the new command under the unreleased section.
- `tests/architecture.rs` — no change is required, but the constant lists
  and `docs/LOGICAL_LAYERS.md` must be checked together, since the command
  module lives in the application layer.

## 7. Ordered work packages

### Work package A — Extract the shared delete primitive

Intent: move the delete sequence out of the TUI selection loop into a
reusable primitive, without changing what it does.

Required changes: create the primitive carrying the mutation gate, the
local-data lock, the re-read, the lookup by identifier with `!deleted`, the
tombstone with the strictly-advancing timestamp, the durable save, the
audit entry, and the sync trigger; replace the inline sequence in the TUI
selection loop with a call to it; keep the picker's in-memory mirror update
in the selection loop, since it is picker-specific.

Acceptance evidence: the selection loop no longer contains the tombstone
sequence; the primitive contains it exactly once; the interactive delete
still produces the same persisted state, proven by the existing TUI tests
remaining green.

### Work package B — Add the command

Intent: expose the primitive as a non-interactive command.

Required changes: add the command's arguments struct and its `run`
function in a new `src/commands/delete_cmd.rs`; resolve the target through
`selector::resolve_selector`; handle the `SelectionResult` variants so
that one match deletes, many matches and ambiguity delete nothing and
report the standard condition, and not-found reports not found; call the
shared primitive; emit the human-readable result on stdout and the JSON
shape in JSON mode.

Acceptance evidence: `snp delete --id <id>` tombstones the snippet and
exits with the documented code; an ambiguous selector exits with the
standard ambiguity code and changes nothing; a non-existent target exits
with the standard not-found code.

### Work package C — Wire the command into the CLI

Intent: make the command reachable under both spellings.

Required changes: add the `Commands::Delete` variant with its arguments in
`src/main.rs`; add the dispatch arm; add the `DataCommands::Delete`
variant; add the `snp data delete` dispatch arm; add the
`command_behavior()` arms for both spellings with a mutating
startup-recovery policy and a non-minimal service level.

Acceptance evidence: `snp delete --help` and `snp data delete --help` both
work; the two spellings behave identically; `snp data delete` is refused
in the same circumstances as the other mirrored mutating commands.

### Work package D — Test the equivalence

Intent: prove the non-interactive path produces the same persisted state as
the interactive one.

Required changes: a test that performs the same delete through both paths
against the same fixture and asserts the persisted library files are
identical; a test that the tombstone timestamp is strictly greater than the
live version's; a test that the identifier is unchanged by the delete; a
test that an ambiguous selector deletes nothing; a test that a target
already deleted reports the distinct outcome.

Acceptance evidence: the equivalence test passes; the exact-count
assertions hold; the audit log contains the `delete` entry.

### Work package E — Document the command

Intent: make the new command discoverable and contract-complete.

Required changes: write `docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`,
`docs/JSON_SCHEMAS.md`, `docs/USER_GUIDE.md`, and `CHANGELOG.md` entries;
write `architecture/commands/delete_cmd.md`; add the deep-dive index entry
in `architecture/overview.md`; update `architecture/cli.md`.

Acceptance evidence: every command row required by
`.skills/cli-and-outcome-contract.md` exists; the deep-dive index resolves
to an existing file; the help output and the documentation agree.

## 8. Failure, cancellation, restart, and contention semantics

**Partial failure.** The mutation is applied inside
`library::update_library`, which re-reads under the lock and writes through
the durable atomic write. A failure before the write leaves the library
unchanged. A failure during the write leaves the library recoverable
through the existing journal and repair machinery.

**Duplicate delivery.** The command is not idempotent in the sense of being
safe to run twice: the second run finds the target already deleted and
reports the distinct already-deleted outcome rather than deleting
anything. The tombstone itself is idempotent — setting `deleted` on an
already-deleted snippet would not change its content — but the command
refuses to act on an already-deleted target so the report is accurate.

**Process restart.** A delete interrupted between the mutation and the
audit write leaves the tombstone in place and no audit entry. The audit
write is best-effort by design and this is the accepted consequence; the
mutation is the durable part.

**Cancellation races.** If the target is deleted by another process between
resolution and mutation, the mutation's lookup by identifier with
`!deleted` finds nothing and the command reports the not-found outcome. No
other snippet is touched.

**Stale generations.** Not applicable. The library is re-read inside the
critical section, so the in-memory generation the caller resolved against
is never written back.

**Concurrent callers.** Two concurrent deletes of the same target: one
commits the tombstone, the other finds the target already deleted and
reports the distinct outcome. Two concurrent deletes of different targets
in the same library serialize on the local-data lock; neither loses an
update, because each re-reads under the lock.

**Auto-sync trigger.** The notification fires only after the mutation is
durable. If the notification fails, the deletion still stands, and the
failure is reported through the existing auto-sync notification result
path.

## 9. Compatibility and migration

No data migration. No stored artifact changes format. No protocol change.

No configuration fallback is needed. No protocol negotiation is needed.

No existing command spelling, alias, exit code, or output format changes.
The new command is purely additive to the CLI surface, and the `snp data`
mirror gains one variant that mirrors it.

Removal criteria: none. This milestone adds a surface and removes nothing.

## 10. Required tests

### Focused unit tests

- A unit test for the shared primitive asserting that the tombstone
  timestamp is strictly greater than the live version's, and that the
  identifier is unchanged.
- A unit test asserting that an already-deleted target is reported
  distinctly and is not modified.
- A unit test asserting that a not-found target is reported distinctly.

### Integration tests

- A test asserting that `snp delete --id <id>` tombstones the snippet, that
  the snippet no longer appears in `snp list`, and that the persisted file
  contains the tombstone.
- A test asserting that `snp data delete --id <id>` produces the same
  persisted state as the top-level spelling.
- A test asserting that an ambiguous selector changes nothing and exits
  with the documented code.
- A test asserting that the JSON output shape matches
  `docs/JSON_SCHEMAS.md`.
- A test asserting that the audit log contains the `delete` entry with the
  target's identifier.

### Restart and recovery tests

- A test that a delete under an interrupted transaction journal is refused
  by the mutation gate, and that the command reports the refusal and
  directs the user to `snp repair`.
- A test that a delete interrupted mid-write leaves the library recoverable
  through `snp repair`.

### Contention and cancellation tests

- A test that runs a delete and a create against one library concurrently
  and asserts no lost update and that exactly one snippet was tombstoned.
- A test that runs two deletes of the same target concurrently and asserts
  exactly one tombstone and one already-deleted report.

### Security and negative tests

- A test asserting that the command refuses to act on a read-only library
  resolution, matching the existing `allow_delete` rule.
- A test asserting that no credential appears in the command's output or in
  its audit entry.
- A test asserting that the mutation gate refusal path writes nothing.

### Migration and compatibility tests

- A test that a library written before this change still loads and that a
  library written after it loads with a build from before, proving the
  persisted format did not change.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test integration --features test-support
cargo test --test repair_transactions --features test-support -- --test-threads=1

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

`repair_transactions` is serial. Use `--test-threads=1` for it.

## 12. Documentation updates

- `docs/COMMANDS.md` — the new command's section.
- `docs/COMMAND_CONTRACTS.md` — the new command's row.
- `docs/JSON_SCHEMAS.md` — the JSON output shape.
- `docs/USER_GUIDE.md` — the scripting workflow.
- `docs/EXIT_CODES.md` — only if a code not already listed is used.
- `architecture/commands/delete_cmd.md` — a new deep dive.
- `architecture/overview.md` — the deep-dive index entry.
- `architecture/cli.md` — the command dispatch chain.
- `architecture/selector.md` — only if the selector inputs differ from the
  family's existing ones.
- `CHANGELOG.md` — the new command.

## 13. Acceptance criteria

1. `snp delete` and `snp data delete` both resolve a target and tombstone
   it without a terminal UI.
2. The persisted state after a non-interactive delete is identical to the
   persisted state after the interactive delete for the same target.
3. The tombstone timestamp is strictly greater than the live version's, and
   the snippet's identifier is unchanged by the delete.
4. An ambiguous selector deletes nothing and reports the standard
   ambiguity condition.
5. An already-deleted target and a not-found target produce distinct
   reports.
6. The mutation gate refuses the delete under an interrupted journal, and
   the command reports the refusal.
7. An audit entry is written naming the action and the target, and a failed
   audit write does not fail the delete.
8. The auto-sync notification fires after the mutation is durable, with the
   snippet-delete mutation kind and the user origin.
9. The new command has a `command_behavior()` entry, a `docs/COMMANDS.md`
   section, a `docs/COMMAND_CONTRACTS.md` row, and a JSON output shape
   documented in `docs/JSON_SCHEMAS.md`.
10. `snp data delete` mirrors `snp delete` exactly, including the
    startup-recovery policy.
11. The TUI's delete key and hint are unchanged, and the TUI still deletes.
12. No existing command spelling, exit code, or output format changed, and
    the persisted format did not change.
13. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
    pass.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the shared primitive cannot be extracted without changing the
  interactive delete's observable behavior;
- the delete would require a persisted-schema change or a migration;
- the target resolution would require a different selector semantic than
  the existing family uses;
- a hard dependency is absent;
- the scope would expand into the `folders`/`favorite` write path, the
  non-interactive field update, or the MCP surface;
- repository evidence contradicts the claim that the delete sequence
  produces the persisted state described here.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh` results.
- The equivalence test result, showing the interactive and non-interactive
  paths producing identical persisted files.
- The tombstone-timestamp and identifier-stability test results.
- The ambiguity, already-deleted, and not-found outcome test results.
- The mutation-gate refusal test result.
- The audit and auto-sync notification assertions.
- The `snp delete --help` and `snp data delete --help` output.
- A statement that no existing command spelling, exit code, or output
  format changed.

## 16. Handoff notes

- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes
  that appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another
  test run is in flight. The integration tests execute
  `target/debug/snp`; a rebuild mid-run replaces that binary and produces
  false failures.
- `repair_transactions` is serial. Use `--test-threads=1` for it.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- The mutation gate must run before every local mutation. One journal means
  auto-rollback; multiple or incomplete journals mean refuse and direct
  the user to `snp repair`.
- Redaction is a `SnipError::Command` display-layer concern only. Never put
  a credential in `runtime_error(msg, detail)` — the runtime and sync
  failure variants print their detail verbatim.
- `docs/LOGICAL_LAYERS.md` and `tests/architecture.rs` must be updated
  together; one of them is otherwise lying.
- The deep-dive index in `architecture/overview.md` covers the architecture
  documents. A new deep dive is registered in the index in the same change.

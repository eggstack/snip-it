# Snippet CRUD Surface Roadmap

Status: active

> **2026-10-22 — M001 closed.** The milestone's implementation plan
> (`plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md`)
> was executed and its closure record is
> `plans/closure/snippet-crud-surface/001-status.md`. The roadmap changed only
> in its status field and in §12: the M001 objective, deliverable boundary,
> exit conditions, and milestone history are unchanged. The closure record
> records the one deliberate deviation from the plan — the fuzzy selector is
> `--query`, not `--filter`, because `--filter` is the TUI's pre-filter and
> there is no TUI — and the `snp data` mirror group growing from five
> spellings to six.

Long-term references:

- `plans/000-long-term-specification.md#1-product-definition`
- `plans/000-long-term-specification.md#2-primary-product-goals`
- `plans/000-long-term-specification.md#3-non-goals`
- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#7-search-selector-and-mcp-invariants`
- `plans/001-terminology-and-domain-model.md#3-snippet-terms`
- `plans/001-terminology-and-domain-model.md#6-selection-and-outcome-terms`
- `plans/002-long-term-roadmap.md#phase-3-cli-library-selector-and-sync-policy-consolidation`
- `plans/003-planning-process.md`

Related ADRs: none required yet. This roadmap adds write paths for snippet
fields that already exist in the model. It does not change the persisted
schema, the sync merge order, conflict identity, exit-code policy, or the
MCP read-only boundary. A milestone that would change any of those opens an
ADR before implementation continues.

## 1. Purpose and ownership boundary

This subsystem owns the non-interactive write surface for snippet content:
the commands by which a user or a script creates, updates, deletes, and
organizes a snippet without entering the TUI.

It consumes the selector contract, the mutation gate, the local-data and
transaction locks, the audit log, and the auto-sync notification path. It
MUST NOT own locking protocol
(`plans/subsystems/locking-convergence-roadmap.md`), the durable-copy policy
(`plans/subsystems/maintenance-consolidation-roadmap.md`), the sync cursor
protocol (`plans/subsystems/sync-cursor-protocol-roadmap.md`), search
semantics, or the MCP read-only boundary.

## 2. Work classification

### Invariants

- Snippet identifiers are deterministic and are not reassigned by an edit.
  Changing a snippet's content, output, folders, or favorite flag does not
  change its identifier.
- Deletion is a tombstone. The deleted flag is set, the tombstone is
  preserved for sync, and the update timestamp is advanced strictly beyond
  the live version so that a deletion beats live content in the client merge.
- Every local mutation runs the mutation gate first and holds the local-data
  lock, re-reading the library inside the critical section.
- The `snp data` mirror group covers every command this roadmap adds.
- MCP remains read-only, stdio-only, non-executing, and noninteractive.
- Exit codes, stdout/stderr split, and machine-output schemas are contract
  surfaces.

### Capabilities

- A script can create, update, delete, and organize snippets without a
  terminal UI.
- Every snippet field that the model defines and that the search, sort, and
  display surfaces already honor has a write path.
- The interactive TUI delete and update paths and the non-interactive paths
  produce identical persisted state.

### Infrastructure

- One shared snippet-mutation primitive used by every write path, so the
  gate, lock, tombstone, audit, and auto-sync sequence cannot drift between
  the TUI and the CLI.

### Polish

- Consistent help text, error messages, and JSON output across the new
  commands.

## 3. Non-goals

- No MCP mutation tool, and no MCP command execution. The MCP boundary is
  read-only by design and is not widened here.
- No new persisted fields. `folders` and `favorite` already exist in the
  model; this roadmap gives them a write path, it does not extend the
  schema.
- No change to the deterministic identifier derivation, and therefore no
  change to the sync fingerprint, the merge order, or conflict identity.
- No bulk or batch mutation, no transactional multi-snippet edit, and no
  scripting language.
- No new sort modes, filter syntax, or query language.
- No change to the TUI's interaction model, keybindings, or layout.
- No vault, secret-store, or credential-bearing snippet support.
- No change to the import command's UUID assignment, which remains the one
  documented exception to deterministic identifiers.

## 4. Current state

The product definition names fast local snippet CRUD as the first primary
goal. The implemented surface covers create and read well, and covers
update and delete only through the interactive TUI.

**Delete is reachable only from the TUI.** The delete action is produced in
exactly one place, the TUI key handler in `src/ui/mod.rs`, and consumed in
exactly one place, the selection loop in `src/commands/mod.rs`. There is no
`snp delete` command, no `--delete` flag on any command, and no
non-interactive path to a tombstone. The TUI path itself is careful: it
deletes under the local-data lock through `library::update_library`, so the
library is re-read inside the critical section and a snippet created
elsewhere while the picker was open is not overwritten by the picker's
stale in-memory copy. It sets the tombstone, advances `updated_at` strictly
past the live version, records an audit entry, and then either runs an
explicit sync or notifies the auto-sync worker with the snippet-delete
mutation kind. All of that behavior is correct and is currently reachable
only when a human is driving a terminal.

**Non-interactive update covers one field.** `snp edit --output` sets the
output field. `snp new` sets description, command, and tags. There is no
non-interactive path to change a snippet's description, command, or tags
after creation, and no path to set `folders` or `favorite`.

**`folders` and `favorite` have no write path at all.** Both are model
fields with no production write anywhere in the workspace. A repository-wide
search for a write to either field returns only reads, tests, and the
contents of the dead migration module. Meanwhile both fields are honored
across the product: they are displayed by the list human, JSON, and CSV
outputs and by the search detail output; they are rendered by the TUI; they
are a first-class sort key, with `--favorites-first` available on the list,
run, clip, search, and select commands; they are part of the local-only
fingerprint; and they are carried by the sync merge as local-only values.
The tool can sort by a field the user cannot set.

**The identifier consequence of the missing write path.** The local-only
fingerprint used to derive a deterministic `legacy-<sha256>` identifier
includes `output`, `folders`, and `favorite`. For a snippet that already
carries an identifier, that fingerprint affects only duplicate detection and
not the assigned identifier, so a field change is safe. For a snippet that
does not yet carry an identifier, the identifier is derived from a content
key that includes those same fields, so the identifier a legacy snippet
receives depends on fields the user cannot currently set through the CLI.
This is not a defect in the derivation, which `docs/IDENTITY_CONTRACT.md`
documents as intentional, but it is a sharp edge that a `folders`/`favorite`
write path must not step on: the write path must not change a snippet's
identifier, and the identifier derivation must not become the mechanism by
which a folder assignment renames a snippet.

## 5. Target architecture

One shared snippet-mutation primitive in the library layer. It takes a
target identity, applies a mutation, and performs the whole sequence in one
place: mutation gate, local-data lock, re-read inside the critical section,
apply, advance `updated_at` monotonically, persist through the durable
atomic write, audit entry, and auto-sync notification. Every write path —
TUI and non-interactive — calls it.

One non-interactive command family over that primitive, with the same
selector semantics as the other commands, so a target can be named by
exact identifier, exact description, exact command, or a filter, and
ambiguity is resolved the same way it is everywhere else.

One CLI surface per capability: create, update, delete, and organize. Each
is a `command_behavior()` entry, each is mirrored by `snp data`, and each
has a row in `docs/COMMAND_CONTRACTS.md`.

## 6. Dependency graph

```text
M001 Non-interactive snippet delete
    |
    +--> M002 Folder and favorite write path (hard: M001; shares the primitive)
    |
    `--> M003 Non-interactive field update (hard: M001; shares the primitive)
```

M001 first, because it establishes the shared mutation primitive that M002
and M003 build on, and because a delete path is the larger of the two
user-visible gaps. M002 and M003 are independent of each other.

## 7. Milestones

### M001 — Non-interactive snippet delete

Class: capability

Objective: give a script a deterministic way to tombstone a snippet, using
the same persisted result as the existing TUI delete path.

Dependencies: none.

Deliverable boundary: a shared snippet-delete primitive extracted from the
TUI selection loop and used by both; a new command that resolves its
target through the existing selector contract and tombstones it; the
command's `command_behavior()` entry; the `snp data` mirror arm; the
`docs/COMMAND_CONTRACTS.md` row; the JSON output mode.

User or operator value: a script can remove a snippet — for example after
consuming a one-time credential or a rotating endpoint — without a terminal
UI and without hand-editing the library file.

Exit conditions: the persisted state after a non-interactive delete is
byte-equivalent to the persisted state after the TUI delete for the same
target; the tombstone timestamp is strictly greater than the live version;
an audit entry is recorded; the auto-sync notification fires with the
snippet-delete mutation kind; an ambiguous selector fails with the
standard ambiguity error rather than deleting an arbitrary match; a target
that is already deleted reports a distinct outcome; `bash scripts/check.sh`
passes.

Deferred work: folder and favorite write path (M002), non-interactive field
update (M003).

Implementation plan:

- `plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md`

Closure record: `plans/closure/snippet-crud-surface/001-status.md` (closed).

Predecessor flat plan: none.

### M002 — Folder and favorite write path

Class: capability

Objective: give the user a way to set and clear the snippet `folders` and
`favorite` fields that the product already reads, sorts on, and displays.

Dependencies: hard M001 (shares the mutation primitive).

Deliverable boundary: commands to set and clear a snippet's favorite flag
and to add and remove a folder membership; each resolves its target through
the existing selector contract; each preserves the snippet's identifier;
each records an audit entry and notifies auto-sync.

User or operator value: the `--favorites-first` sort and the folder
metadata become usable from the command line instead of requiring a
hand-edited TOML file.

Exit conditions: `favorite` and `folders` have a production write path;
setting or clearing either does not change a snippet's identifier, verified
by a test that asserts identifier stability across the mutation; the values
round-trip through the save/load path without the TOML post-processing the
save path is forbidden from applying; the list JSON and CSV outputs reflect
the change on the next read; `bash scripts/check.sh` passes.

Deferred work: non-interactive field update (M003).

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

### M003 — Non-interactive field update

Class: capability

Objective: allow a snippet's description, command, and tags to be changed
non-interactively, generalizing the existing `snp edit --output` path.

Dependencies: hard M001 (shares the mutation primitive).

Deliverable boundary: one update command that can set description, command,
tags, and output; the existing `snp edit --output` behavior is either
folded into it or preserved as an alias with its current spelling intact;
the selector contract is reused unchanged, including the rule that an
output edit requires a filter or an exact selector unless an exact target
is named.

User or operator value: a script can correct a snippet's command or retag
it without opening an editor, which is the common case for a tag-normalization
or command-fixing pass.

Exit conditions: description, command, and tags are each settable
non-interactively; the exact-selector and filter rules match the existing
`edit` contract; the update preserves the identifier; the audit entry
records what changed; the update is atomic with respect to concurrent
writers, re-reading under the lock; `bash scripts/check.sh` passes.

Deferred work: none within this subsystem.

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

## 8. Cross-cutting requirements

### Storage and migration

No schema migration. `folders` and `favorite` are already optional fields
in the persisted TOML. A snippet that has never been favorited and has no
folders continues to round-trip as it does today. The save path continues
not to post-process `toml::to_string_pretty` output, so the golden corpus
must survive every new write path unchanged.

### Protocol and compatibility

No wire-protocol change; `folders`, `favorite`, and `output` remain
local-only and are excluded from the synced protobuf message. The sync
merge keeps the local copy of those fields. No exit code is renumbered, and
no existing command's flags change meaning.

### Security and authorization

A new command is a new mutation surface and MUST run the mutation gate
before touching local state. The `snp data` mirror must not become a way
to bypass a startup-recovery policy: the mirrored arm has the same
`command_behavior()` entry as the top-level spelling. Credentials remain in
the keychain and are never written into a snippet file by a new command.

### Concurrency, cancellation, and recovery

Every mutation re-reads the library inside the local-data lock rather than
writing a stale in-memory copy. A target that disappears between resolution
and mutation reports a distinct not-found outcome rather than failing
generically. A mutation that is interrupted leaves the library recoverable
through the existing journal and repair machinery; the new commands do not
introduce a second recovery model.

### Observability and audit

Every mutation records an audit entry naming the action and the target. The
audit write remains best-effort: a failed audit write is logged and does
not fail the mutation. The new commands support a JSON output mode whose
shape is documented in `docs/JSON_SCHEMAS.md`.

### Performance and resource use

No new runtime systems. The shared primitive resolves the target once and
mutates under a single lock acquisition; it must not resolve and then
re-acquire.

### Documentation and operations

`docs/COMMANDS.md`, `docs/COMMAND_CONTRACTS.md`, `docs/USER_GUIDE.md`,
`docs/JSON_SCHEMAS.md`, `docs/IDENTITY_CONTRACT.md` (identifier stability),
`architecture/cli.md`, `architecture/commands/edit_cmd.md`, and the new
command's own architecture deep dive are updated with the code. Every new
subcommand receives a `command_behavior()` entry, per
`.skills/cli-and-outcome-contract.md`.

## 9. Verification strategy

A parity test that performs the same delete through the TUI path and the
non-interactive path and asserts the persisted files are identical; a test
that asserts identifier stability across a folder, favorite, and output
mutation; a test that asserts an ambiguous selector deletes nothing; a test
that asserts the mutation gate refuses a mutation under an interrupted
journal; a concurrent-writer test that runs a delete and a create against
one library and asserts no lost update; the serial transaction and repair
suites; `scripts/check.sh`.

## 10. Risks and decision points

- A delete command is destructive and irreversible from the user's point of
  view, because the tombstone is preserved for sync but the snippet is
  gone from every listing. Decision: the command supports a JSON mode that
  reports the affected target before the mutation is reported as complete,
  and it does not require an interactive confirmation, because a
  confirmation prompt would defeat the purpose of a non-interactive command.
  The selector's ambiguity rules protect against deleting the wrong target.
- The M002 milestone touches fields that participate in the local-only
  fingerprint. Decision: the fingerprint's role in identifier derivation is
  unchanged, and the milestone adds an explicit identifier-stability test
  rather than changing the derivation.
- Whether `snp edit --output` should be folded into the M003 command or
  kept as a separate spelling is a CLI-surface decision that affects a
  published contract. Decision: preserve the current spelling and behavior;
  if the new command supersedes it, the old spelling is retained as a
  deprecated alias per `.skills/cli-and-outcome-contract.md`.
- No decision in this subsystem currently requires an ADR. A decision that
  would change the persisted schema, the synced field set, conflict
  identity, or the MCP boundary is an ADR trigger and stops the milestone.

## 11. Completion definition

This workstream closes when M001, M002, and M003 each have accepted closure
evidence showing that create, update, delete, and organization are all
reachable non-interactively, that every snippet field the product reads has
a write path, that the interactive and non-interactive paths produce
identical persisted state, that identifiers are stable across mutations,
and that no unresolved medium-or-higher finding remains.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 non-interactive snippet delete | closed | `plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md` | `plans/closure/snippet-crud-surface/001-status.md` | — |
| M002 folder and favorite write path | not started | — | — | M001 closed |
| M003 non-interactive field update | not started | — | — | M001 closed |

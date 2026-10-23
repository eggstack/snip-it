# Snippet CRUD Surface Milestone 003 — Non-interactive field update

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/snippet-crud-surface-roadmap.md#m003--non-interactive-field-update`

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

Allow a snippet's description, command, and tags to be changed
non-interactively, generalizing the existing `snp edit --output` path, so a
script can correct a snippet's command or retag it without opening an editor.

End state: `snp edit` is the non-interactive field-update surface for
description, command, tags, and output. Its existing `--output` spelling, its
`--filter`-or-exact-selector target rule, and its result-on-stderr contract are
preserved; the new field flags join them on the same command, the same selector
contract, and the same mutation primitive.

## 2. Why this milestone is ready

- The hard dependency is closed. M001 landed the shared snippet-delete
  primitive (`commands::delete_snippet`, `src/commands/mod.rs:744-826`); its
  closure record is `plans/closure/snippet-crud-surface/001-status.md`.
- The persisted fields already exist: `description`, `command`, `tags`, and
  `output` are model fields with a serde representation
  (`src/library/model.rs:33-47`). No schema change, no migration, no version
  bump.
- The write path to generalize already exists and is correct in its mechanics:
  `snp edit --output` already mutates under the local-data lock, re-reads
  inside the critical section, and advances `updated_at` with the same rule the
  delete primitive uses (`src/commands/edit_cmd.rs:169`, `:217`, `:263`,
  `:287`). What it lacks is coverage of the other three fields, an audit entry,
  and an auto-sync notification.
- No unresolved decision requires an ADR: the milestone adds flags to an
  existing command over existing persisted fields, and leaves the merge order,
  the conflict identity, the exit-code policy, and the MCP read-only boundary
  untouched.
- **Build-order dependency:** M002 introduces the shared snippet field-update
  primitive (`commands::update_snippet_fields`) that this milestone extends with
  the description / command / tags / output variants. If that primitive has not
  landed, stop and report (§14) rather than adding a second primitive.

## 3. Current implementation evidence

### 3.1 The fields exist; three of the four are synced and one is local-only

| Field | Model site | Synced? |
|-------|-------------|---------|
| `description` | `src/library/model.rs:33-34` | Yes — `architecture/library.md:455` |
| `output` | `src/library/model.rs:35-36` | **No** — local-only notes (`architecture/library.md:462`, `plans/001-terminology-and-domain-model.md:70-75`) |
| `tags` | `src/library/model.rs:37-45` | Yes — `architecture/library.md:457` |
| `command` | `src/library/model.rs:46-47` | Yes — `architecture/library.md:456` |

`description`, `command`, and `tags` are in the sync conflict fingerprint
(`src/sync_commands.rs:1360-1383`) and in `ProtoSnippet`, so changing them
changes the snippet's *synced* content: the new version must carry a strictly
greater `updated_at` so it wins the client merge. `output` is local-only, so it
is excluded from the fingerprint and from the wire format.

### 3.2 `snp edit` is the existing command, and it already owns the output flags

- `Commands::Edit` — `src/main.rs:128-130`, alias `e`.
- `EditArgs` — `src/commands/edit_cmd.rs:9-33`:
  `--output` (`:13-14`), `--output-stdin` (`:16-17`), `--clear-output`
  (`:19-20`), `--filter` (`:22-23`), `--id` (`:25-26`), `--description-exact`
  (`:28-29`), `--command-exact` (`:31-32`).
- The dispatch — `src/main.rs:652-721`. It computes `has_output_flags`
  (`:653`) and `has_exact` (`:654-656`); when an output flag is present it
  requires `--filter` unless an exact selector was given (`:657-669`), reads
  stdin *after* that check so an invalid invocation fails fast instead of
  blocking on EOF (`:658-661`), and then either takes the exact path
  (`:687-709`) or the `--filter` path (`:710-717`). With no output flag it
  takes the `$EDITOR` path (`:719`).
- `EDIT_OUTPUT_FILTER_REQUIRED` — `src/main.rs:52-53`: "`--filter` is required
  when using `--output`, `--output-stdin`, or `--clear-output`".

### 3.3 The two mutation functions and what they do *not* do

- `run_edit_output(library, filter, new_output)` —
  `src/commands/edit_cmd.rs:145-231`. It resolves the target with a
  **case-insensitive substring match** over description and command, skipping
  deleted snippets (`:173-183`), inside the `library::update_library` closure
  (`:169`), and refuses to guess when the filter matches zero or many snippets
  (`:185-212`). It sets `snippet.output` (`:215`) and advances `updated_at` with
  `updated_at.max(now).saturating_add(1)` (`:216-217`), the same rule the
  delete primitive uses (`src/commands/mod.rs:778`).
- `run_edit_output_by_id(library, snippet_id, new_output)` —
  `src/commands/edit_cmd.rs:240-301`. Same shape, but the target is found by
  identifier (`:264-282`).
- **Neither writes an audit entry, and neither notifies auto-sync.** A search
  for `audit_log` and `notify_mutation` in `src/commands/edit_cmd.rs` returns
  only lines `:109` and `:132`, and both are inside the `$EDITOR` path
  (`run`, `:36-139`). The two non-interactive output paths are silent on both
  counts.
- Both print their result to **stderr**, never stdout: `:225`, `:227`, `:295`,
  `:297`. `docs/COMMAND_CONTRACTS.md` records the `edit` row as "Writes stdout:
  No".

### 3.4 The `$EDITOR` path is a different mechanism and stays as it is

`edit_cmd::run` (`src/commands/edit_cmd.rs:36-139`) hands the library file to
`$EDITOR` and lets the editor rewrite it. It runs the mutation gate
(`:61-64`), takes the local-data lock *after* the editor exits (`:103`), and
notifies auto-sync only when the bytes actually changed (`:105`, `:131-136`).
Because the editor owns the file, this path cannot be routed through a
snippet-level mutation primitive, and there is no single "target snippet" for
an audit entry to name. It is a documented exception and is out of this
milestone's scope (§5).

### 3.5 The two selector families already in `snp edit` have different outcomes

| Path | Matcher | Not found | Ambiguous |
|------|---------|-----------|-----------|
| `--filter` | case-insensitive substring over description and command (`src/commands/edit_cmd.rs:173-183`) | `SnipError::runtime_error` → **exit 1** (`:187-192`) | `SnipError::runtime_error` → **exit 1** (`:193-211`) |
| `--id` / `--description-exact` / `--command-exact` | canonical selector via `selector::resolve_exact_target` (`src/main.rs:689-694`, `src/selector.rs:659-666`) | `CliOutcome::NotFound` → **exit 3** (`src/main.rs:707`) | `CliOutcome::Ambiguous` → **exit 5** (`src/main.rs:704-706`) |

`docs/COMMAND_CONTRACTS.md` records the `edit` row's exit categories as
`0, 1, 2, 3, 5` — all five codes are already reachable today. This milestone
must not renumber, repurpose, or remove any of them.

One sub-case is worth stating precisely: the by-id path's *internal* re-read
failure — the snippet removed between the selector's resolution and the locked
re-read inside `update_library` — is a `SnipError::runtime_error`
(`src/commands/edit_cmd.rs:274-281`) and therefore exit **1**, not 3. The
selector's own not-found (exit 3) happens earlier, before the mutation
starts.

### 3.6 `snp update` is taken, and `snp edit` is not mirrored by `snp data`

- `snp update` is the **self-update** command — `src/main.rs:95-101`. The
  roadmap's §5 phrase "One CLI surface per capability: create, update, delete,
  and organize" therefore names the *capabilities*, not command spellings; the
  update capability's existing home is `snp edit`.
- `DataCommands` — `src/main.rs:283-299` — mirrors six spellings: validate,
  backup, restore, repair, status, delete. `snp edit` is **not** among them, so
  this milestone adds **no** `snp data` variant and no second `handle_*`
  path.
- `Commands::Edit` is already classified: `StartupRecoveryPolicy::Allow` with
  `StartupServices::Logging` (`src/main.rs:1010`). No new `command_behavior()`
  arm is required.
- The top-level aliases already taken are `v n l r c s sel e k y cr reg lib p i
  rp val d g` (`src/main.rs:93-179`).

### 3.7 There is no interactive TUI update path to be equivalent to

`SnippetSelection` (`src/ui/mod.rs:631-640`) has exactly four variants:
`Selected`, `Copied`, `Delete`, `Cancelled`. The TUI produces a delete and
nothing else; it has no field-update path. The only update paths in the
product today are the `$EDITOR` path and the two `snp edit --output` functions.
"Identical persisted state between the interactive and the non-interactive path"
therefore means the `$EDITOR` path and the `--output` path, and this milestone
must not change either one's persisted shape.

### 3.8 The tag grammar already exists and must not drift

`snp new --tags` accepts a comma- or space-separated list
(`src/commands/new_cmd.rs:25-34`) and splits it with `parse_tags`
(`src/commands/new_cmd.rs:532-538`): split on `[' ', ',']`, trim each piece,
drop empties. `parse_tags` is currently private to `new_cmd`. The new
`snp edit --tags` must use the same grammar, so the helper is shared rather
than reimplemented.

### 3.9 The tag set is replaced, not merged, and empty values are rejected

- `Snippet::new` rejects an empty command and an empty description
  (`src/library/model.rs:302-314`), so a snippet with an empty command or
  description is not a valid snippet. `--description ""` and `--command ""`
  must be usage errors.
- `--tags ""` parses to an empty vector through `parse_tags`, so it is the
  natural spelling for "clear the tags".
- The tags field is `Vec<String>` with `skip_serializing_if = "Vec::is_empty"`
  (`src/library/model.rs:37-45`), so clearing the tag set removes the `tag`
  key from the persisted TOML.

### 3.10 The `--library all` defect on the exact path

`run_edit_output_by_id` re-resolves the library by name through
`get_library_path` (`src/commands/edit_cmd.rs:247`), and `get_library_path`
rejects the name `all` (`src/commands/mod.rs:99-108`). But the exact-path
resolution has already produced the correct library path
(`src/selector.rs:418-423`), and `src/commands/mod.rs:163-166` documents that
the `all` keyword "means the same thing here as on `snp get`, exact-mode
`run`/`clip`/`edit`, and MCP". So `snp edit --output --library all --id <ID>`
currently resolves the snippet and then fails with "library not found" at the
re-resolution step. Routing the mutation through the resolved library path
fixes this and matches the documented intent.

### 3.11 Exit codes in force for `snp edit`

`src/outcome.rs:48-72` defines the stable table; `src/outcome.rs:76-93` maps
`CliOutcome` onto it; `docs/EXIT_CODES.md:10-22` publishes it.

| Code | Constant | Reachable today in `snp edit`? |
|------|----------|-------------------------------|
| 0 | `SUCCESS` | Yes — the editor path and the output path |
| 1 | `GENERAL_ERROR` | Yes — the `--filter` path's not-found and ambiguous, and "No library found" (`src/commands/edit_cmd.rs:154-160`, `:187-211`) |
| 2 | `USAGE_ERROR` | Yes — the `--filter` requirement (`src/main.rs:667`, `:714`) |
| 3 | `NOT_FOUND` | Yes — the exact path (`src/main.rs:707`) |
| 5 | `AMBIGUOUS` | Yes — the exact path (`src/main.rs:704-706`) |

This milestone introduces **no new exit code** and changes no existing code's
meaning.

## 4. Invariants that must not regress

- Snippet identifiers are deterministic and are **not** reassigned by an edit.
  The mutation locates its target by identifier and never re-derives one
  (`src/library/persistence.rs:38-80`).
- The mutation gate runs before the local mutation. One journal → auto-rollback;
  multiple or incomplete journals → refuse and direct the user to `snp repair`;
  corrupt journals and `RecoveryClass::UnsafeFailed` refuse fail-closed
  (`src/transaction.rs:2165-2200`, `architecture/overview.md:446-453`).
- The mutation re-reads the library inside the local-data lock rather than
  writing a stale in-memory copy (`src/library/persistence.rs:294-316`,
  `src/local_data.rs:131`).
- The mutation is persisted through the durable atomic write path with its named
  durability class (`src/library/persistence.rs:337-375`,
  `architecture/overview.md:455-474`).
- **The save path does not post-process TOML.** `toml::to_string_pretty` output
  is written verbatim (`src/library/persistence.rs:361-368`,
  `architecture/overview.md:570-575`); the golden corpus
  (`tests/local_contracts.rs:217-280`) must survive this write path unchanged.
- The persisted library format carries **no schema version**: no field and no
  version is added, so no migration and no version bump.
- `output` remains **local-only**; `description`, `command`, and `tags` remain
  **synced**. The sync conflict fingerprint
  (`src/sync_commands.rs:1360-1383`) and `ProtoSnippet` are unchanged.
- The client merge order stays `(updated_at, device_id, SHA-256(synced
  fields))` and stays role-independent (`src/sync_commands.rs:1419-1446`).
- Because `description`, `command`, and `tags` are synced fields, a mutation of
  them must advance `updated_at` with the same strictly-advancing rule the
  delete primitive uses (`src/commands/mod.rs:778`), so the new version wins
  the client merge.
- `snp edit`'s published contract row is unchanged: it writes its result to
  **stderr** and nothing to stdout, it may prompt (`$EDITOR`), it may execute a
  shell (`$EDITOR`), and it exits `0, 1, 2, 3, 5`
  (`docs/COMMAND_CONTRACTS.md`).
- The `--filter`-or-exact-selector target rule for output edits is unchanged
  (`src/main.rs:657-669`, `src/main.rs:52-53`), including the fail-fast order
  that checks the requirement before reading stdin.
- The `$EDITOR` path is unchanged: the gate runs before the file is created
  (`src/commands/edit_cmd.rs:61-71`), the local-data lock is taken after the
  editor exits (`:103`), and the auto-sync notification fires only when the
  bytes changed (`:105`, `:131-136`).
- The tag grammar is the `snp new --tags` grammar
  (`src/commands/new_cmd.rs:532-538`); adding a field to one surface and not
  the other is a silent divergence.
- An audit entry names the action and the target; a failed audit write is
  logged and does not fail the mutation.
- The auto-sync notification fires only after the mutation is durable.
- No credential, sync metadata, or secret appears in a diagnostic message.
- MCP remains read-only, stdio-only, non-executing, and noninteractive.
- The TUI's keybindings, layout, and delete key are unchanged.

## 5. Scope

### In scope

- `snp edit` gains `--description <TEXT>`, `--command <TEXT>`, and
  `--tags <TAGS>`, alongside the existing `--output` / `--output-stdin` /
  `--clear-output`.
- `snp edit` gains `--query <TEXT>` as the canonical fuzzy selector, so a
  script can name a target the way `snp get --query` and `snp delete --query`
  do.
- The mutation vocabulary of the shared field-update primitive gains four
  variants: set-description, set-command, set-tags, set-output.
- The two existing non-interactive output mutations
  (`src/commands/edit_cmd.rs:145-231`, `:240-301`) are rerouted through the
  shared primitive, which adds the audit entry and the auto-sync notification
  they currently lack.
- The `--library all` defect on the exact path (§3.10) is fixed by mutating
  through the resolved library path.
- The shared tag splitter `parse_tags` (`src/commands/new_cmd.rs:532-538`) is
  reused by `snp edit --tags` rather than reimplemented.
- Tests for identifier stability, the save/load round trip without TOML
  post-processing, the `snp list --json` / `--csv` outputs, the audit entry, the
  auto-sync notification, the selector rules, and the exit codes.

### Explicitly out of scope

- The `folders` and `favorite` write path and the `snp organize` command —
  those belong to M002
  (`plans/implementation/snippet-crud-surface/002-folder-and-favorite-write-path.md`).
- The tombstone, the delete primitive, the `snp delete` command, and the
  already-deleted probe. M001 is closed.
- Any change to the persisted schema, the sync fingerprint, the synced field
  set, the merge order, the conflict identity, or the deterministic identifier
  derivation.
- The `$EDITOR` path's mechanics — the gate, the lock, and the
  change-detecting notification are untouched, and it gains no audit entry,
  because the editor owns the whole file and there is no single target snippet
  to name (§3.4).
- Adding `--json` to `snp edit`. The published `edit` contract row records
  "Writes stdout: No" and "Machine-output modes: None"; a machine-readable
  confirmation of an update is available through `snp list --json` / `--csv`,
  which already carry `output` and `tags`. The roadmap requires a JSON mode for
  the *new* commands, and this milestone adds none.
- Additive tag operations (`--add-tag`, `--remove-tag`): `--tags` replaces the
  whole set, which is the tag-normalization case the roadmap names.
- Any change to the `--filter` substring matcher's semantics, its diagnostics,
  or its exit codes (§3.5).
- Any MCP tool, and any widening of the MCP read-only boundary.
- Bulk or multi-snippet mutation, a transactional multi-snippet edit, and any
  scripting language.
- Any change to the TUI's interaction model, keybindings, or layout.
- A `--sync` flag: the auto-sync notification is the trigger, and `snp edit`
  does not initialize the Tokio runtime (`architecture/overview.md:590-596`).
- A `snp data edit` mirror: `snp edit` is not mirrored today
  (`src/main.rs:283-299`) and this milestone does not change that.

## 6. Required production changes

### Core/domain

The shared snippet field-update primitive from M002 (`commands::update_snippet_fields`
in `src/commands/mod.rs`) gains four mutation variants — set-description,
set-command, set-tags, set-output — and keeps its existing four. The primitive's
mechanics do not change: mutation gate, local-data lock, re-read inside the
critical section, lookup by identifier, apply, advance `updated_at`, durable
save, best-effort audit entry, sync trigger after the commit.

Two properties the new variants must respect:

- **The audited action names the field.** The audit action is derived from the
  mutation variant: `description`, `command`, `tags`, `output`. The audit
  writer records the action and the target's identifier and deliberately omits
  the description text (`src/logging.rs:401-404`), so the action string is the
  only record of *which* field changed. The action must not contain `|`: the
  audit record is pipe-delimited and the action is not escaped
  (`src/logging.rs:427-439`).
- **`description`, `command`, and `tags` are synced fields**, so their
  mutations advance `updated_at` with the same strictly-advancing rule the
  delete primitive uses (`src/commands/mod.rs:778`) — the new version must win
  the client merge. `output` is local-only and keeps the timestamp behavior it
  has today (`src/commands/edit_cmd.rs:216-217`, `:286-287`), so M003 does not
  change any existing command's merge behavior.

The `--tags` mutation **replaces** the whole tag vector. The tag splitter is
the shared `parse_tags` (`src/commands/new_cmd.rs:532-538`); making it shared
rather than private to `new_cmd` is what keeps the two surfaces' grammars from
drifting.

### Storage and migrations

No schema change, no migration, no version bump. The four fields already have
their serde representation (`src/library/model.rs:33-47`).

Byte-level consequences the tests must pin, following the existing
representation:

- `tags` is `#[serde(rename = "tag", alias = "Tag", alias = "Tags", alias =
  "tags", default, skip_serializing_if = "Vec::is_empty")]`
  (`src/library/model.rs:37-45`): clearing the tag set removes the `tag` key
  from the persisted TOML.
- `description` and `command` have no `skip_serializing_if`
  (`src/library/model.rs:33-34`, `:46-47`): they are always written.
- `output` has `#[serde(rename = "output", alias = "Output", default)]`
  (`src/library/model.rs:35-36`): clearing it writes `output = ""`.

### Protocol and DTOs

No sync-protocol change. `description`, `command`, and `tags` are already in
`ProtoSnippet` and in the sync conflict fingerprint
(`src/sync_commands.rs:1360-1383`); `output` is not, and stays not.

The consequence of a local-only `output` mutation is the one M002 documents:
`updated_at` is part of the conflict identity
(`src/sync_commands.rs:1379`, `:1430-1444`), so advancing it on an output-only
change can make the local copy win a merge against an older server copy. This
is the **existing** `snp edit --output` behavior (`src/commands/edit_cmd.rs:217`)
and M003 preserves it rather than changing a shipped command's merge behavior.
See §8 and §14.

### Runtime and concurrency

The mutation runs under the local-data lock and re-reads the library inside the
critical section, and the target is located by identifier, so a concurrent
insertion or deletion cannot shift it.

The `--filter` path's matching semantics change in one respect, and the plan
states it: today the substring match and the mutation happen inside a single
`update_library` closure (`src/commands/edit_cmd.rs:169`), so the matched
content and the written content are the same read. With the primitive, the
substring match is resolved first and the mutation is applied by identifier
inside the critical section. The mutation therefore always applies to the
snippet that matched at resolution time, located by identifier so a concurrent
insertion cannot shift it — but the *match* is no longer taken from the same
read as the write. A concurrent writer that changes the matching snippet
between resolution and mutation does not cause the edit to land on a different
snippet; the identifier pins the target.

The audit write remains best effort. The auto-sync notification fires after the
commit succeeds and is not fired when the mutation fails.

### Frontend or operator surface

`snp edit` gains three field flags and one selector flag:

| Flag | Type | Meaning |
|------|------|---------|
| `--description <TEXT>` | `Option<String>` | Replace the description. Must be non-empty after trimming |
| `--command <TEXT>` | `Option<String>` | Replace the command body. Must be non-empty after trimming |
| `--tags <TAGS>` | `Option<String>` | Replace the whole tag set. Comma- or space-separated, `snp new --tags` grammar. `""` clears the set |
| `-q/--query <TEXT>` | `Option<String>` | Fuzzy selector, using the canonical selector semantics |

`--description`, `--command`, `--tags`, `--output`, `--output-stdin`, and
`--clear-output` are mutually exclusive with each other: one invocation sets
one field. (The existing `--output` family is already mutually exclusive
through clap `conflicts_with_all` — `src/commands/edit_cmd.rs:13-20`.)

`--query` is mutually exclusive with `--id`, `--description-exact`,
`--command-exact`, and `--filter`, exactly as in the other commands
(`src/commands/delete_cmd.rs:24-34`).

**Target naming.** A field edit requires a target, and the rule is the existing
one: an output edit requires `--filter` **or** an exact selector
(`--id`, `--description-exact`, `--command-exact`), and the requirement is
checked before stdin is read so an invalid invocation fails fast
(`src/main.rs:657-669`). The new field flags use the same rule and add
`--query` as an alternative. A field edit with no target is a usage error
(exit 2).

**Two selector families, deliberately kept apart.** `--id`,
`--description-exact`, `--command-exact`, and `--query` use the **canonical**
mutating selector family (`selector::resolve_exact_target`,
`src/selector.rs:659-666`; `selector::resolve_selector`, `src/selector.rs:736`)
and therefore the canonical outcomes — not found → exit 3, ambiguous → exit 5,
nothing changed. `--filter` keeps the **legacy** case-insensitive substring
match (`src/commands/edit_cmd.rs:173-183`) and its exit-1 diagnostics
(`:187-211`). The two families are not merged in this milestone: merging them
would change `snp edit --output --filter`'s observable exit codes, and exit
codes are a published contract.

**The `$EDITOR` path is unchanged.** `snp edit` with no field flag still opens
`$EDITOR` (`src/main.rs:719`).

**Output shape.** `snp edit` continues to write its human-readable result to
**stderr** and nothing to stdout, exactly as today
(`src/commands/edit_cmd.rs:225`, `:227`, `:295`, `:297`). The wording of the
existing output messages is preserved verbatim, including the
"Cleared output for snippet: …" / "Updated output for snippet: …" pair, which
is derived from the resulting value being empty or not — not from the
primitive's changed/unchanged verdict.

### Security and authorization

`snp edit` is already a mutation command, already runs the mutation gate, and
already holds the local-data lock. This milestone adds no new credential
surface: no new flag carries a credential, and the audit entry records the
action and the target's identifier only, with the description omitted by the
audit writer itself (`src/logging.rs:401-404`).

### Documentation and static guards

See §12 for the document list. `tests/architecture.rs` needs no change: no new
module is added and the scanner only governs `crate::` path prefixes.

## 7. Ordered work packages

### Work package A — Extend the shared field-update primitive

Intent: give the four content fields the same audited, notified, lock-protected
mutation the folder and favorite fields already have.

Required changes: in `src/commands/mod.rs`, extend the mutation-value enum with
set-description, set-command, set-tags, and set-output variants; extend the
audit action derivation to name them (`description`, `command`, `tags`,
`output`); make the shared `parse_tags` helper (`src/commands/new_cmd.rs:532-538`)
available to the command layer; and leave the primitive's gate / lock / re-read
/ identifier-lookup / timestamp / durable-save / audit / sync-trigger mechanics
unchanged.

Acceptance evidence: the primitive still appears exactly once in
`src/commands/mod.rs`; the four new audit action names are exact; a unit test
shows that setting the description, the command, and the tags each advances
`updated_at` with the delete primitive's rule, that the identifier is unchanged
by all four mutations, and that `--tags ""` empties the tag vector.

### Work package B — Add the field flags to `snp edit`

Intent: let a script set description, command, and tags non-interactively.

Required changes: extend `EditArgs` (`src/commands/edit_cmd.rs:9-33`) with
`--description`, `--command`, `--tags`, and `--query`; extend the clap
conflict rules so exactly one field flag is accepted and `--query` is
exclusive with the exact selectors and `--filter`; extend the dispatch in
`src/main.rs:652-721` so the presence of a field flag selects the
non-interactive path and the absence still selects the `$EDITOR` path; enforce
the target-naming rule (a field edit needs `--filter`, `--query`, or an exact
selector) as a usage error before any stdin is read; validate that
`--description` and `--command` are non-empty after trimming.

Acceptance evidence: `snp edit --id <ID> --description "New text"` sets the
description and exits 0; `snp edit --id <ID> --tags "a,b"` replaces the tag set;
`snp edit --id <ID> --command "echo hi"` replaces the command; a field edit
with no target exits 2; an empty `--description` exits 2; `snp edit` with no
field flag still opens `$EDITOR`.

### Work package C — Reroute the two output mutations through the primitive

Intent: give `snp edit --output` the audit entry and the auto-sync notification
it currently lacks, without changing its observable result.

Required changes: replace the bodies of `run_edit_output`
(`src/commands/edit_cmd.rs:145-231`) and `run_edit_output_by_id`
(`:240-301`) so that they resolve the target, then call the shared primitive
with the resolved library path and the target's identifier; keep the substring
matcher's rules, its candidate-list diagnostic, and its exit-1 semantics
byte-identical; keep the existing "Cleared output…" / "Updated output…" stderr
messages and their wording; use the resolved `m.library_path` rather than
re-resolving the library name, which fixes the `--library all` defect
(§3.10).

Acceptance evidence: `snp edit --output --filter X` produces the same stderr
text and the same exit code as before; `snp edit --output --id <ID>` likewise;
the audit log now contains an `output` entry with the target's identifier; the
auto-sync notification fires with `MutationKind::SnippetUpdate` and
`MutationOrigin::User`; `snp edit --output --library all --id <ID>` now
succeeds instead of failing with "library not found".

### Work package D — Test the field semantics, the selector rules, and the exits

Intent: prove the milestone's exit conditions.

Required changes: tests asserting that

- setting description, command, or tags leaves the snippet's identifier
  **byte-identical**;
- the values round-trip through the save/load path, and the golden corpus
  (`tests/local_contracts.rs:217-280`) survives a field-update round trip
  unchanged — that is, the save path applies no TOML post-processing;
- clearing the tag set removes the `tag` key from the persisted TOML, and
  clearing the output writes `output = ""`;
- `snp list --json` and `snp list --csv` report the new `output` and `tags`
  on the next read (`src/commands/list_cmd.rs:168-169`, `:184-197`);
- the audit log contains the expected action and the target's identifier, and no
  description text appears in the audit record;
- the auto-sync notification fires after the mutation is durable, with
  `MutationKind::SnippetUpdate` and `MutationOrigin::User`;
- the exact-selector path reports not found as exit 3 and ambiguity as exit 5,
  and the `--filter` path still reports both as exit 1 with its existing
  diagnostics;
- a field edit with no target, and an empty `--description` or `--command`, are
  usage errors (exit 2);
- `snp edit` with no field flag still opens `$EDITOR` and still notifies only
  when the bytes changed.

Acceptance evidence: each test passes with exact-count assertions (not `>= 1`),
and the persisted file is compared byte for byte where the claim is about bytes.
The concrete assertions are listed in §10.

### Work package E — Document the new flags

Intent: keep the published CLI surface accurate.

Required changes: update the `snp edit` section of `docs/COMMANDS.md`, the
`edit` row of `docs/COMMAND_CONTRACTS.md`, the `edit` notes in
`architecture/commands/edit_cmd.md` and `architecture/output.md`, the
dispatch-chain description in `architecture/cli.md`, the selector table in
`architecture/selector.md`, the `snp new` / `snp edit` tag-grammar note, and
`CHANGELOG.md`.

Acceptance evidence: the help output and the documentation agree; the
`docs/COMMAND_CONTRACTS.md` `edit` row still records "Writes stdout: No" and
exit categories `0, 1, 2, 3, 5`; the deep-dive index still resolves.

## 8. Failure, cancellation, restart, and contention semantics

**Partial failure.** The mutation is applied inside `library::update_library`
(`src/library/persistence.rs:294-316`), which re-reads under the lock and writes
through the durable atomic write path. A failure before the write leaves the
library unchanged; a failure during it leaves the library recoverable through
the existing journal and repair machinery.

**Mutation interrupted mid-flight.** The journal is the recovery mechanism. A
crash between the journal write and the commit leaves a journal under
`<config>/.transaction/`; the next mutation's gate
(`src/transaction.rs:2165-2200`) finds it and either auto-rolls it back (one
journal) or refuses the new mutation and directs the user to `snp repair`
(multiple or incomplete journals). Corrupt journals and
`RecoveryClass::UnsafeFailed` refuse fail-closed. The milestone introduces no
second recovery model: the primitive reuses `library::update_library`, so the
journal state machine in `src/transaction.rs` is the only one.

**Durable-write failure.** The save goes through `save_library_internal`
(`src/library/persistence.rs:337-375`): a best-effort backup is taken first
(`:342-344`), the TOML is written through `write_private_atomic` (`:361-368`),
and the TOML cache is invalidated only after the write (`:370-372`). A write
failure propagates as an error, the mutation is reported as failed, the audit
entry is **not** written, and the auto-sync notification is **not** fired.

**Concurrent mutation from another process.** The local-data lock
(`src/local_data.rs:131`) serializes the load-modify-save cycle, and the
library is re-read inside the critical section, so neither writer loses an
update. Two concurrent edits of the same snippet serialize; the second observes
the first's committed state.

**Target disappears between resolution and mutation.** The primitive's lookup is
by identifier with `!deleted`; a target tombstoned or removed by another process
in the window is not found, and the command reports the not-found outcome. No
other snippet is touched.

**Target's content changes between resolution and mutation.** For the
`--filter` path, the substring match is resolved before the lock and the
mutation applies to the matched snippet's identifier inside the critical
section, so the edit lands on the snippet that matched — never on a different
one — even if a concurrent writer changed the snippet's description in between.

**Stale generations.** Not applicable. The library is re-read inside the critical
section, so the in-memory copy the caller resolved against is never written back.

**Duplicate delivery.** Re-applying the same field values is a no-op in content.
The primitive reports changed versus unchanged, so a script can tell whether the
command had to write.

**Process restart between the mutation and the audit write.** The mutation is
durable; the audit entry is missing. The audit write is best-effort by design
and this is the accepted consequence.

**Auto-sync trigger.** The notification fires only after the mutation is durable.
If scheduling fails, the mutation stands and the failure is reported through
`report_notification_result` (`src/auto_sync/notification.rs:43-55`) on stderr.

**Merge consequence.** `description`, `command`, and `tags` are synced fields, so
a mutation of them advances `updated_at` and the new version wins the client
merge — the intended behavior. `output` is local-only, so an output-only change
advances the same timestamp the existing `snp edit --output` path already
advances (`src/commands/edit_cmd.rs:217`); that can make the local copy win a
merge against an older server copy. M003 preserves the existing behavior rather
than changing a shipped command's merge semantics. §14 turns an observed
instance of the output case into a stop-and-report condition.

## 9. Compatibility and migration

No data migration. No stored artifact changes format. No protocol change. No
configuration fallback is needed. No protocol negotiation is needed.

**The one deliberate behavior change.** `snp edit --output` and
`snp edit --output-stdin` / `--clear-output` gain an audit entry and an
auto-sync notification they did not have (§3.3). Their flags, their target
rule, their matching semantics, their stderr messages, their stdout silence,
and their exit codes are unchanged. The change is additive in observable side
effects only: an `output` audit row appears in `<config>/audit.log`, and a
pending sync generation is recorded and a worker scheduled. The audit write is
best effort, and a scheduling failure does not fail the command
(`src/auto_sync/notification.rs:43-55`).

**A second deliberate change.** `snp edit --output --library all --id <ID>`
now succeeds instead of failing with "library not found" (§3.10). This aligns
the command with the documented `all`-scope behavior for exact-mode `edit`
(`src/commands/mod.rs:163-166`) and with `snp get --library all`. It is a
bug fix, not a contract change: exit 0 where the command previously failed.

Everything else is additive: `snp edit` gains three field flags and one
selector flag, and adding a flag is safe
(`.skills/cli-and-outcome-contract.md`, compatibility rule 3). The default
behavior of every existing spelling is preserved.

The `snp data` mirror group is unchanged: `snp edit` was never mirrored
(`src/main.rs:283-299`).

The persisted format does not change: the four fields already have their serde
representation (`src/library/model.rs:33-47`), and the save path writes
`toml::to_string_pretty` output verbatim
(`src/library/persistence.rs:361-368`).

Removal criteria: none. This milestone adds flags and removes nothing.

## 10. Required tests

### Focused unit tests

- The extended primitive, in `src/commands/mod.rs`: the identifier is unchanged
  by a description, command, tags, and output mutation; `updated_at` advances
  with the delete primitive's rule (`updated_at.max(now).saturating_add(1)`);
  the audit action names are exactly `description`, `command`, `tags`, `output`.
- `parse_tags` is shared: the same helper produces the same vector for `snp new
  --tags` and `snp edit --tags`, and `""` yields an empty vector.
- A not-found target is reported distinctly and the library file is
  byte-identical before and after.

### Integration tests

- `snp edit --id <ID> --description "New text"` sets the description; the
  snippet shows the new description in `snp list --json`; the persisted file
  contains it; the identifier is unchanged.
- `snp edit --id <ID> --command "echo hi"` replaces the command, and the
  identifier is unchanged.
- `snp edit --id <ID> --tags "a,b"` replaces the whole tag set — the old tags
  are gone, not merged — and `--tags ""` removes the `tag` key from the
  persisted TOML.
- `snp edit --id <ID> --output "notes"` still writes the output, still prints
  its result to stderr and not stdout, and still exits 0.
- `snp edit --output --filter <text>` still produces the same stderr text and
  the same exit code as before the change, including exit 1 for a filter that
  matches zero and for one that matches several.
- The golden corpus survives a field-update round trip: a snippet whose command
  contains tabs, trailing spaces, or CRLF is updated and re-read unchanged.
- The audit log contains the expected action and the target's identifier for
  each of the four fields, and no description text appears in the audit record.
- The auto-sync notification fires after the mutation is durable, with
  `MutationKind::SnippetUpdate` and `MutationOrigin::User`.
- `snp edit --id <ID>` with a field flag and no target exits 2; an empty
  `--description` and an empty `--command` exit 2.
- The exact-selector path reports not found as exit 3 and ambiguity as exit 5,
  and changes nothing in the ambiguous case.
- `snp edit --output --library all --id <ID>` succeeds and mutates the snippet
  in the library the selector resolved.
- `snp edit` with no field flag still opens `$EDITOR` (existing test coverage in
  `tests/edit_mutation_notify.rs` must stay green) and still notifies only when
  the bytes changed.

### Restart and recovery tests

- A field update under an interrupted transaction journal is refused by the
  mutation gate, the command reports the refusal and directs the user to
  `snp repair`, and the library file is unchanged.
- A field update interrupted mid-write leaves the library recoverable through
  `snp repair`.

### Contention and cancellation tests

- Two concurrent edits of the same snippet against one library: no update is
  lost and the identifier is unchanged.
- A concurrent edit and an `snp delete` of the same snippet: exactly one of them
  takes effect and the library remains loadable.

### Security and negative tests

- The command refuses to act under an interrupted journal and writes nothing.
- No credential appears in the command's output or in its audit entry.
- The audit action name is pipe-free, so the audit record's field boundaries
  cannot be shifted.

### Migration and compatibility tests

- A library written before this change still loads, and a library written after
  it loads with a build from before, proving the persisted format did not
  change.
- Every pre-existing `snp edit` spelling still produces the same exit code and
  the same stderr text as before the change.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test integration --features test-support
cargo test --test edit_mutation_notify --features test-support
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

`local_data_lock_barriers`, `repair_transactions`, and `process_lock_concurrency`
are serial: use `--test-threads=1` for each.

**Never run `cargo build`, `cargo test`, or `cargo clippy` while another test run
is in flight.** The integration tests execute `target/debug/snp`; a rebuild
mid-run replaces that binary and produces false failures — most visibly in the
`test-support`-gated crash suites, whose failpoint seam silently disappears when
the binary is rebuilt with default features.

`cleanup_crash_failpoints` and `restore_crash_failpoints` are invoked by no script
and must be run by hand before merging a restore or cleanup change. This
milestone changes neither path, so those two suites are not required — but if the
implementation touches the journal or the atomic write path in a way that changes
their behavior, run them by hand and record the result.

## 12. Documentation updates

- `docs/COMMANDS.md` — the `snp edit` section: the new field flags, the target
  rule, and the two selector families.
- `docs/COMMAND_CONTRACTS.md` — the `edit` row: the machine-output modes stay
  "None", "Writes stdout" stays "No", and the exit categories stay
  `0, 1, 2, 3, 5`. Only the flag inventory changes.
- `docs/USER_GUIDE.md` — the scripting workflow.
- `architecture/commands/edit_cmd.md` — the new flags, the rerouted output
  mutations, the audit and notification behavior, and the `--library all` fix.
- `architecture/output.md` — the `snp edit --output` contract.
- `architecture/cli.md` — the dispatch chain.
- `architecture/selector.md` — the `snp edit` selector inputs, if they differ
  from the family's existing ones; and the two selector families' distinct
  outcomes.
- `architecture/library.md` — the local-only vs synced field split table, which
  already records `output` as local-only and the other three as synced.
- `docs/IDENTITY_CONTRACT.md` — confirm the "Retains ID" lifecycle row.
- `CHANGELOG.md` — the new flags.

## 13. Acceptance criteria

1. `snp edit` sets description, command, and tags non-interactively, and the
   existing `--output` / `--output-stdin` / `--clear-output` spellings keep
   working unchanged.
2. The target is named by `--filter`, `--query`, or one of the three exact
   selectors, and the existing rule — an output edit requires `--filter` or an
   exact selector — is preserved, including the fail-fast order that checks it
   before stdin is read.
3. The canonical selector family is reused unchanged: not found → exit 3,
   ambiguous → exit 5, ambiguous changes nothing. The legacy `--filter` family
   keeps its substring semantics, its diagnostics, and its exit-1 outcomes.
4. No field update changes the snippet's identifier — verified by a test that
   asserts identifier stability across each of the four fields.
5. The values round-trip through the save/load path, and the golden corpus
   survives a field-update round trip: the save path applies no TOML
   post-processing.
6. `snp list --json` and `snp list --csv` reflect the new `output` and `tags`
   on the next read.
7. Every non-interactive field update writes an audit entry naming the action
   and the target's identifier, and a failed audit write does not fail the
   mutation.
8. Every non-interactive field update notifies auto-sync after the mutation is
   durable, with `MutationKind::SnippetUpdate` and `MutationOrigin::User`.
9. The mutation gate refuses the update under an interrupted journal, and the
   command reports the refusal.
10. The update is atomic with respect to concurrent writers: the library is
    re-read under the local-data lock and the target is located by identifier.
11. `snp edit` continues to write its result to stderr and nothing to stdout,
    and its `docs/COMMAND_CONTRACTS.md` row keeps "Writes stdout: No" and exit
    categories `0, 1, 2, 3, 5`.
12. The `$EDITOR` path is unchanged: it still runs the gate, still takes the
    lock after the editor exits, and still notifies only when the bytes changed.
13. The tag grammar is the `snp new --tags` grammar, through the shared helper.
14. No new exit code was introduced, and no existing command's spelling, flags,
    exit code, or output format changed beyond the two documented additions
    (the audit entry and the auto-sync notification for `snp edit --output`).
15. The persisted schema, the sync fingerprint, the synced field set, the merge
    order, and the conflict identity are unchanged.
16. MCP remains read-only, stdio-only, non-executing, and noninteractive; no MCP
    tool was added.
17. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh` pass.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- M002's shared field-update primitive is absent — this milestone extends that
  primitive and must not add a second one;
- the rerouting of the two output mutations would change `snp edit --output`'s
  observable behavior beyond the two documented additions (flags, target rule,
  matching semantics, stderr text, stdout silence, exit codes);
- the `--filter` substring matcher's semantics, its candidate-list diagnostic,
  or its exit-1 outcomes would have to change;
- the identifier guarantee cannot be met — specifically, if implementing the
  field update would require re-deriving an identifier, or if a mutation
  demonstrably changes a snippet's identifier;
- the timestamp rule would have to change for the synced fields — if a
  description, command, or tags update fails to win the client merge, stop and
  report;
- an output-only update is observed to discard a *remote* description or
  command change that is newer than the output edit: stop and report it, because
  changing the shipped `snp edit --output` timestamp behavior is not chartered
  here;
- the scope would expand into the `folders` / `favorite` write path, into
  `snp organize`, into a new `snp update` command (`snp update` is the
  self-update command, `src/main.rs:95-101`), into a `snp data edit` mirror, into
  `--json` on `snp edit`, into additive tag operations, into the MCP surface,
  or into a bulk operation;
- a hard dependency is absent, or the persisted schema would have to change;
- repository evidence contradicts the claims in §3 — including the claim that
  the output paths write no audit entry and fire no notification, the two
  selector families' exit codes, or the `--library all` defect.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
  results.
- The identifier-stability test results, one per field.
- The save/load round-trip and golden-corpus results.
- The `snp list --json` / `--csv` output assertions.
- The audit and auto-sync notification assertions for each of the four fields.
- The selector-rule and exit-code assertions for both selector families.
- The mutation-gate refusal test result.
- A before/after comparison of `snp edit --output --filter` and
  `snp edit --output --id` stderr text and exit codes.
- The `snp edit --help` output.
- A statement that the only observable behavior changes are the two documented
  additions, and that the persisted format did not change.

## 16. Handoff notes

- Line numbers in this plan were verified against the working tree at
  authoring time. Another agent was editing `src/sync_commands.rs`,
  `src/selector.rs`, and `src/library/model.rs` concurrently, so re-verify a
  citation against the file before relying on it.
- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes that
  appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another test run
  is in flight. The integration tests execute `target/debug/snp`; a rebuild
  mid-run replaces that binary and produces false failures.
- `local_data_lock_barriers`, `repair_transactions`, and
  `process_lock_concurrency` are serial. Use `--test-threads=1` for each.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must never
  be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- The mutation gate must run before every local mutation. One journal means
  auto-rollback; multiple or incomplete journals mean refuse and direct the user
  to `snp repair`.
- Redaction is a `SnipError::Command` display-layer concern only. Never put a
  credential in `runtime_error(msg, detail)` — the runtime and sync failure
  variants print their detail verbatim.
- `docs/LOGICAL_LAYERS.md` and `tests/architecture.rs` must be updated together;
  one of them is otherwise lying.
- The deep-dive index in `architecture/overview.md` covers the architecture
  documents. A new deep dive is registered in the index in the same change.
- `snp update` is the **self-update** command (`src/main.rs:95-101`); the update
  capability's home is `snp edit`, which is also the only snippet-update
  surface in the product.
- `snp edit` is **not** mirrored by `snp data` (`src/main.rs:283-299`), so this
  milestone adds no `DataCommands` variant and no second `handle_*` path.
- `Commands::Edit` is already classified as `Allow` / `Logging`
  (`src/main.rs:1010`); no new `command_behavior()` arm is required.
- The TUI has no field-update path (`SnippetSelection`,
  `src/ui/mod.rs:631-640`); "identical persisted state" for this milestone
  means the `$EDITOR` path and the `--output` path, both of which stay as they
  are.
- `parse_tags` (`src/commands/new_cmd.rs:532-538`) becomes shared. Moving it
  must not change `snp new --tags` behavior.
- The `folders` / `favorite` fields and the `snp organize` command belong to
  M002 and are out of this milestone's scope.

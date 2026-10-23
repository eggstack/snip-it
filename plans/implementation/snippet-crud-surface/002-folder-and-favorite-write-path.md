# Snippet CRUD Surface Milestone 002 — Folder and favorite write path

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/snippet-crud-surface-roadmap.md#m002--folder-and-favorite-write-path`

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

Give a script a way to set and clear a snippet's `favorite` flag and to add and
remove a folder membership, through one non-interactive command that resolves
its target with the existing selector contract and mutates through the shared
mutation primitive M001 established.

End state: `favorite` and `folders` stop being read-only fields. The
`--favorites-first` sort, the folder column of `snp list --json` / `--csv`, and
the `Folders:` / `Favorite:` lines of `snp search` become reachable from the
command line instead of requiring a hand-edited TOML file.

## 2. Why this milestone is ready

- The hard dependency is closed. M001 landed the shared snippet-delete
  primitive (`commands::delete_snippet`, `src/commands/mod.rs:744-826`); its
  closure record is `plans/closure/snippet-crud-surface/001-status.md`. This
  milestone reuses that primitive's *shape* rather than designing a second
  mutation sequence.
- The persisted fields already exist (`src/library/model.rs:48-51`). No schema
  change, no migration, no version bump — the persisted library format carries
  no schema version and this milestone does not add one.
- No unresolved decision requires an ADR: the milestone adds a command over
  existing persisted fields and leaves the merge order, the conflict identity,
  the exit-code policy, and the MCP read-only boundary untouched.
- The target resolution, the mutation gate, the local-data lock, the audit log,
  and the auto-sync notification path all exist and are used elsewhere.
- The scope is disjoint from M001: this milestone never sets `deleted` and never
  touches the already-deleted probe.

## 3. Current implementation evidence

### 3.1 The two fields exist and round-trip through the persisted TOML

- `Snippet.folders` — `src/library/model.rs:48-49`. `Vec<String>` with
  `#[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]`, so
  the key is **omitted** from the TOML when the list is empty.
- `Snippet.favorite` — `src/library/model.rs:50-51`. `bool` with
  `#[serde(default)]` and **no** `skip_serializing_if`, so `favorite = false` is
  always present in the persisted file.
- `Snippets.folders` — `src/library/model.rs:20-21`. A *library-level*
  folder-name registry, distinct from the per-snippet membership list. Merged
  by `src/commands/mod.rs:182-184` and persisted by
  `src/library/persistence.rs:356-359`, but **no display or selection surface
  reads it**: the TUI's folder data comes from the per-snippet lists
  (`src/commands/mod.rs:386`, `:607`), and `snp list` / `snp search` / MCP all
  read the per-snippet field.

### 3.2 There is no production write path for either field — confirmed

A repository-wide search for an assignment to `.favorite` or `.folders` outside
test code returns only:

| Site | What it does |
|------|--------------|
| `src/library/model.rs:322-323` | `Snippet::new`'s defaults — `folders: Vec::new()`, `favorite: false` |
| `src/commands/import_cmd.rs:293`, `:429`, `:451` | `folders: Vec::new()` on imported snippets |
| `src/sync_commands.rs:1499-1500`, `:1520-1521`, `:1533-1534` | the sync merge carrying the **local** values forward |
| `src/sync_commands.rs:1558-1559` | a server-only snippet arriving with empty folders and `favorite: false` |
| `src/commands/pet_analysis.rs:20-21`, `:133-134` | the pet importer's field-name allowlist and type check — validation only, no write |

The roadmap's §4 statement — "`folders` and `favorite` have no write path at
all" — is **confirmed**.

### 3.3 The roadmap's "local-only fingerprint" premise names two different fingerprints

The roadmap (§4) says `folders` and `favorite` "are part of the local-only
fingerprint". Two distinct fingerprints exist; the claim is true of one and
false of the other, and the difference decides whether the write path may
advance `updated_at`.

| Fingerprint | Site | Covers `folders` / `favorite`? |
|-------------|------|-------------------------------|
| Sync conflict fingerprint | `fingerprint()` — `src/sync_commands.rs:1360-1383` | **No.** Hashes `id`, `description`, `command`, `tags`, `created_at`, `updated_at`, `device_id`, `deleted` only. Correct: local-only fields MUST NOT enter it (`plans/001-terminology-and-domain-model.md:70-75`, `architecture/library.md:450-468`) |
| Identifier-derivation content fingerprint | `snippet_content_key()` — `src/library/persistence.rs:104-125`; `deterministic_legacy_id()` — `:127-157`; `deterministic_duplicate_id()` — `:159-191` | **Yes.** Hashes description, command, tags, output, folders, favorite, `device_id`, occurrence into `legacy-<sha256 hex>`. This is the fingerprint the roadmap means |

Consequence (roadmap §4): for a snippet that **already carries** an identifier,
the identifier is unaffected — `normalize_snippet_ids()`
(`src/library/persistence.rs:38-80`) only mints an identifier for a snippet
whose `id` is empty (`:61-68`) or whose explicit identifier is a duplicate
(`:69-78`), and every identifier the file already carries is reserved up front
(`:46-51`). The write path must therefore locate its target **by identifier**,
never by index, and must never re-derive identifiers.

### 3.4 `--favorites-first` sorts by a field no command can set — confirmed

`SortOptions.favorites_first` — `src/sort.rs:61-63`; the grouping compares the
two snippets' `favorite` flags before the primary sort key —
`src/sort.rs:153-158`. The flag is exposed on `list`
(`src/commands/list_cmd.rs:27-29`), `run` (`src/commands/run_cmd.rs:24`),
`clip` (`src/commands/clip_cmd.rs:21`), `search`
(`src/commands/search_cmd.rs:20`), and `select`
(`src/commands/select_cmd.rs:31`).

Because `favorite` has no production write path, `--favorites-first` can today
only group snippets that arrived with `favorite = true` from a hand-edited TOML
file. The roadmap's "the tool can sort by a field the user cannot set" is
**confirmed**.

### 3.5 The read surfaces already honor both fields

| Surface | Evidence |
|---------|----------|
| `snp list --json` | `"folders"`, `"favorite"` — `src/commands/list_cmd.rs:168-169` |
| `snp list --csv` | header `description,command,output,tags,folders,favorite` — `src/commands/list_cmd.rs:184-197` |
| `snp search` | `Folders:` / `Favorite:` lines — `src/commands/search_cmd.rs:70-71` |
| MCP `snippets_search` payload | `"folders"`, `"favorite"` — `src/mcp/tools.rs:260-261` |
| TUI | favorite marker — `src/ui/mod.rs:1175`; folder data — `src/commands/mod.rs:386`, `:607` |

`snp search` and MCP are **read-only** and stay read-only. This milestone adds a
CLI write path and no MCP tool (`architecture/overview.md:432-444`).

### 3.6 The mutation shape to reuse is the M001 delete primitive

`commands::delete_snippet` (`src/commands/mod.rs:744-826`) is the one existing
implementation of the full local-mutation sequence:

1. the mutation gate and the local-data lock, through `library::update_library`
   — `src/commands/mod.rs:752`, `src/library/persistence.rs:294-316`;
2. the re-read of the library **inside** the critical section —
   `src/library/persistence.rs:311`;
3. the lookup of the target **by identifier** with `!deleted`, so a concurrent
   insertion cannot shift the target — `src/commands/mod.rs:753-756`;
4. the mutation itself — `src/commands/mod.rs:775-778`;
5. the durable save — `src/library/persistence.rs:313` →
   `save_library_internal` (`:337-375`) → `toml::to_string_pretty` written
   verbatim (`:361-362`) through `write_private_atomic` (`:368`);
6. the audit entry, best effort — `src/commands/mod.rs:805-808`;
7. the sync trigger, **after** the commit succeeded — `src/commands/mod.rs:810-823`.

The primitive contains no picker-specific logic and locates the target by
identifier rather than by index. That is the shape this milestone reuses.

### 3.7 The audit entry and the auto-sync notification

- `crate::logging::audit_log(action, snippet, library_id)` —
  `src/logging.rs:386-410`. The record is pipe-delimited
  (`src/logging.rs:427-439`); the description is deliberately replaced with the
  literal `"[omitted]"` because it is user-authored free text that may contain
  secrets (`src/logging.rs:401-404`). The `action` field is **not**
  pipe-escaped, so an action name must not contain `|`.
- `crate::auto_sync::notify_mutation(kind, origin)` —
  `src/auto_sync/notification.rs:57-68` → `notify_local_mutation` (`:76-133`):
  one pending-generation increment (`:110`) and one worker spawn (`:117-125`).
- `crate::auto_sync::report_notification_result(...)` —
  `src/auto_sync/notification.rs:43-55`: a scheduling failure is reported on
  stderr and does not turn a durable local mutation into a failed command.
- `MutationKind::SnippetUpdate` — `src/auto_sync/policy.rs:71-81`;
  `MutationOrigin::User` — `src/auto_sync/policy.rs:89-101`.

### 3.8 The CLI surface to extend

- `Commands` — `src/main.rs:91-213`; the top-level aliases already taken are
  `v n l r c s sel e k y cr reg lib p i rp val d g` (`src/main.rs:93-179`).
  `o` (for `organize`) is free.
- `DataCommands` — `src/main.rs:283-299`. Six spellings are mirrored today
  (validate, backup, restore, repair, status, delete).
- `command_behavior()` — `src/main.rs:941-1053`. It assigns a
  `StartupRecoveryPolicy` and a `StartupServices` level in one match so the two
  cannot drift. The mutating arms are `src/main.rs:999-1000` (`snp data
  delete`) and `src/main.rs:1002-1022` (top-level mutating commands).
- The `snp data` mirror takes **three** edits in `src/main.rs`: a
  `DataCommands` variant, a dispatch arm, and a `command_behavior()` arm
  (`.skills/cli-and-outcome-contract.md:144-148`).
- `tests/cli_surface_compat.rs` asserts the two spellings behave equivalently;
  the new mirrored spelling must be added there.

### 3.9 The exit codes in force

`src/outcome.rs:48-72` defines the stable table; `src/outcome.rs:76-93` maps
`CliOutcome` onto it; `docs/EXIT_CODES.md:10-22` publishes it.

| Code | Constant | Used by the new command for |
|------|----------|------------------------------|
| 0 | `SUCCESS` | the mutation committed, or the requested end state already held |
| 1 | `GENERAL_ERROR` | an operational failure (gate refusal, persistence failure) |
| 2 | `USAGE_ERROR` | no selector, no action flag, more than one action flag, an invalid folder name |
| 3 | `NOT_FOUND` | the selector matched nothing |
| 5 | `AMBIGUOUS` | the selector matched more than one snippet; nothing was changed |

These are the codes `snp delete` uses for the same conditions
(`src/commands/delete_cmd.rs:135-160`, `:169-188`), so the new command
introduces **no new exit code**. `snp delete`'s exit 9
(`CONFLICT_OR_REFUSED`, "the target was already tombstoned") has no analogue
here: this milestone never tombstones, and a no-op mutation is a **success**,
reported with a distinct `status` token rather than a new exit code.

### 3.10 Premises that do not survive contact with the code

- **"The `snp data` mirror group covers every command this roadmap adds"**
  (roadmap §2) — the mirror group does **not** cover every top-level command
  today: `snp new`, `snp get`, `snp run`, `snp clip`, `snp search`, `snp
  select`, and `snp edit` are not mirrored (`src/main.rs:283-299` mirrors only
  the six data-maintenance spellings). This milestone adds a new top-level
  command and therefore **does** mirror it, per roadmap §5; the sibling
  milestone that generalizes `snp edit` does not gain a mirror.
- **"they are part of the local-only fingerprint"** — see §3.3: true of the
  identifier-derivation content fingerprint, false of the sync conflict
  fingerprint. The sync fingerprint stays untouched.

## 4. Invariants that must not regress

- Snippet identifiers are deterministic and are **not** reassigned by an edit:
  the mutation locates its target by identifier and never re-derives one
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
  `architecture/overview.md:455-474`); a failure during the write leaves the
  library recoverable through the journal and `snp repair`.
- **The save path does not post-process TOML**: `toml::to_string_pretty` output
  is written verbatim (`src/library/persistence.rs:361-368`,
  `architecture/overview.md:570-575`), and the golden corpus
  (`tests/local_contracts.rs:217-280`) survives this write path unchanged.
- The persisted library format carries **no schema version**: no field and no
  version is added, so no migration and no version bump.
- `folders` and `favorite` remain **local-only**: out of the sync conflict
  fingerprint (`src/sync_commands.rs:1360-1383`), out of `ProtoSnippet`, and
  never searchable (`plans/001-terminology-and-domain-model.md:70-75`,
  `architecture/library.md:450-468`).
- The client merge order stays `(updated_at, device_id, SHA-256(synced
  fields))` and stays role-independent (`src/sync_commands.rs:1419-1446`).
- An audit entry names the action and the target; a failed audit write is logged
  at debug level and does not fail the mutation.
- The auto-sync notification fires only after the mutation is durable.
- An ambiguous selector changes nothing and reports the ambiguity.
- Exit codes, the stdout/stderr split, and the machine-output rules of
  `docs/EXIT_CODES.md` and `docs/CLI_EXITCODE_STREAM_POLICY.md` are unchanged;
  the new command introduces no new exit code.
- The `snp data` mirror group continues to mirror every spelling it mirrors
  today, and the new spelling carries the same `command_behavior()` entry as
  the top-level one, so the mirror cannot bypass the startup-recovery policy.
- No credential, sync metadata, or secret appears in a diagnostic message.
- MCP remains read-only, stdio-only, non-executing, and noninteractive.
- The TUI's keybindings, layout, and delete key are unchanged.

## 5. Scope

### In scope

- A shared **snippet field-update primitive** in `src/commands/mod.rs`, shaped
  exactly like `commands::delete_snippet` (`src/commands/mod.rs:744-826`):
  mutation gate, local-data lock, re-read inside the critical section, lookup
  by identifier, apply, advance `updated_at`, durable save, best-effort audit
  entry, sync trigger after the commit.
- The primitive's mutation vocabulary covers exactly four operations here: set
  the favorite flag, clear the favorite flag, add a folder membership, remove a
  folder membership.
- A new top-level command `snp organize` that resolves its target through the
  **mutating** selector family (`selector::resolve_selector`,
  `src/selector.rs:736`) with `ResolutionPolicy::Unique`, applies the
  primitive, and reports the result on stdout.
- The `snp data organize` mirror: a `DataCommands` variant, a dispatch arm, and
  a `command_behavior()` arm in `src/main.rs`.
- A JSON output mode for `snp organize --json`, documented in
  `docs/JSON_SCHEMAS.md`.
- The `docs/COMMANDS.md` section, the `docs/COMMAND_CONTRACTS.md` row, and the
  `architecture/commands/organize_cmd.md` deep dive, plus its entry in the
  `architecture/overview.md` deep-dive index.
- Tests for identifier stability, the save/load round trip without TOML
  post-processing, the list JSON and CSV outputs, the audit entry, the
  auto-sync notification, the ambiguity and not-found outcomes, and the
  mutation-gate refusal.

### Explicitly out of scope

- The description / command / tags / output write path, and any change to
  `snp edit` — that belongs to M003
  (`plans/implementation/snippet-crud-surface/003-non-interactive-field-update.md`).
- Any change to the tombstone, the delete primitive, the `snp delete` command,
  or the already-deleted probe. M001 is closed.
- Any change to the persisted schema, the sync fingerprint, the synced field
  set, the merge order, the conflict identity, or the deterministic identifier
  derivation.
- Any MCP tool, and any widening of the MCP read-only boundary.
- The **library-level** `Snippets.folders` registry
  (`src/library/model.rs:20-21`). No surface reads it (§3.1), the roadmap scopes
  this milestone to *snippet* fields, and a second unread list would be a
  second source of truth. A future feature that lists a library's folders must
  backfill the registry or drop it.
- Bulk or multi-snippet mutation, a transactional multi-snippet edit, and any
  scripting language.
- New sort modes, filter syntax, or query language.
- Any change to the TUI's interaction model, keybindings, or layout.
- A vault, a secret store, or credential-bearing snippet support.
- A `--sync` flag on the new command: the auto-sync notification is the
  trigger, and the command never initializes the Tokio runtime
  (`architecture/overview.md:590-596`).
- The `WorkerLock` retirement, the lock ordering contract, and the backup copy
  policy, which belong to other roadmaps.

## 6. Required production changes

### Core/domain

One shared snippet field-update primitive, a sibling of `delete_snippet` in
`src/commands/mod.rs`. It takes a library path, a target identifier, and one
mutation value, and performs the same seven steps the delete primitive performs
(§3.6), with two differences forced by this milestone's fields:

- **The lookup is by identifier with `!deleted`** and does not set `deleted`. A
  tombstoned snippet is not a valid target; the selector excludes deleted
  snippets from every mode (`src/selector.rs:406`), so a tombstoned target
  resolves as *not found*.
- **The outcome distinguishes "changed" from "already in the requested
  state".** Adding a folder the snippet already has, removing a folder it does
  not have, and re-setting a favorite flag to its current value are no-ops,
  reported distinctly so the command can say `status: "unchanged"` instead of
  claiming a change it did not make. Both outcomes exit 0.

The mutation value is a typed enum, not a closure, so the audit entry can name
**which** field changed. The four variants here are set-favorite,
clear-favorite, add-folder, remove-folder. The enum is designed to be extended
by M003 with the description / command / tags / output variants; the sibling
milestone adds variants to the same enum and reuses the same primitive rather
than adding a second one. The primitive returns the updated snippet so the
caller can report the target's identifier, description, and the resulting field
values.

### Storage and migrations

No schema change, no migration, no version bump. Two byte-level consequences of
the existing serde representation, both of which the tests must pin: `folders` is
omitted from the TOML when the list is empty (`src/library/model.rs:48-49`), so
removing the last folder membership removes the key from the file; `favorite` has
no `skip_serializing_if` (`src/library/model.rs:50-51`), so `favorite = false` is
always written and clearing the flag rewrites the value rather than removing a
key.

### Protocol and DTOs

No sync-protocol change. `folders` and `favorite` are not in `ProtoSnippet`
(`snip-proto/proto/sync.proto` declares no such field) and are not in the sync
conflict fingerprint (`src/sync_commands.rs:1360-1383`).

The consequence, stated rather than hidden: the mutation's `updated_at` advance
is visible to the merge, because `updated_at` **is** part of the conflict
identity (`src/sync_commands.rs:1379`, `:1430-1444`). A favorite flip therefore
makes the local copy win a merge against a server copy whose `updated_at` is
older, and the merged result keeps the local `folders` and `favorite` in every
branch (`src/sync_commands.rs:1499-1500`, `:1520-1521`, `:1533-1534`). This is
the same property the existing `snp edit --output` path already has
(`src/commands/edit_cmd.rs:216-217`), so the milestone inherits an established
repository semantic rather than inventing one. See §8 and §14 for the stop
condition if this is observed to lose a remote description change.

### Runtime and concurrency

The mutation runs under the local-data lock and re-reads the library inside the
critical section. The target is located by identifier, not by index, so a
concurrent insertion or deletion cannot shift the target. If the target is
tombstoned or removed by another process between resolution and mutation, the
mutation's lookup finds nothing and the command reports the not-found outcome;
no other snippet is touched. The audit write remains best effort. The
auto-sync notification fires after the commit succeeds and is not fired when
the mutation fails. The command does not initialize the Tokio runtime: it is a
local-only command.

### Frontend or operator surface

A new `snp organize` command.

| Flag | Type | Meaning |
|------|------|---------|
| `--id <ID>` | `Option<String>` | Exact snippet ID (case-sensitive); conflicts with the other selectors |
| `--description-exact <TEXT>` | `Option<String>` | Exact description (case-insensitive) |
| `--command-exact <TEXT>` | `Option<String>` | Exact command text (case-insensitive) |
| `-q/--query <TEXT>` | `Option<String>` | Fuzzy query (selector semantics, not the TUI's `--filter` pre-filter) |
| `-l/--library <NAME>` | `Option<String>` | Scope: one library name. `all` is **refused**, exactly as `snp delete` refuses it (`src/commands/delete_cmd.rs:98-105`) |
| `--favorite` | bool | Set the favorite flag |
| `--unfavorite` | bool | Clear the favorite flag |
| `--add-folder <NAME>` | `Option<String>` | Add a folder membership |
| `--remove-folder <NAME>` | `Option<String>` | Remove a folder membership |
| `--json` | bool | Machine-readable result |

Exactly one of `--favorite`, `--unfavorite`, `--add-folder`, `--remove-folder`
is required. Exactly one of the four selectors is required. Violations are usage
errors (exit 2), enforced by a clap argument group and by the command's own
check.

The selector set and the `--library` handling mirror `snp delete`
(`src/commands/delete_cmd.rs:22-41`, `:110-124`): the selector is built through
`selector::exact_selector` (`src/selector.rs:634-652`) so the library-scope
mapping, the ID case-sensitivity, and the description/command
case-insensitivity cannot drift from `snp get`, `snp delete`, and MCP. The
resolution itself uses `selector::resolve_selector` (`src/selector.rs:736`) —
the **mutating** member of the selector family, which runs
`ensure_library_mode()`. The read-only sibling `resolve_selector_readonly`
(`src/selector.rs:678`) is **not** appropriate for a mutating command
(`architecture/overview.md:432-444`).

**Folder-name validation.** A folder name must be non-empty after trimming. A
folder name must not contain `;`: `snp list --csv` joins the folder list with
`;` (`src/commands/list_cmd.rs:186-187`) and `csv_escape`
(`src/commands/list_cmd.rs:227-248`) quotes the *field* but does not escape the
intra-field separator, so a `;` inside a folder name would make `["a;b"]`
indistinguishable from `["a", "b"]` in the CSV output. Rejecting it at the
write path is the only place the ambiguity can be prevented. A rejected name is
a usage error (exit 2).

**Output shape.** The human-readable result goes to stdout and diagnostics go to
stderr, per `docs/CLI_EXITCODE_STREAM_POLICY.md`. The JSON document is

```json
{
  "schema": 1,
  "status": "updated",
  "id": "string",
  "description": "string",
  "library": "string",
  "favorite": true,
  "folders": ["string"]
}
```

`status` is `updated` when the mutation changed the snippet and `unchanged` when
the requested end state already held. The document is written to stdout only
when a target was identified; a not-found target (exit 3) and an ambiguous
selector (exit 5) produce no stdout document and their diagnostics go to stderr
— the same split `snp delete` uses (`src/commands/delete_cmd.rs:194-226`).

### Security and authorization

The command is a new mutation surface and runs the mutation gate before touching
local state, exactly as every other mutation does. The `snp data` mirror arm
carries the same `command_behavior()` entry as the top-level spelling, so the
mirror is not a way to bypass the startup-recovery policy. No credential is
read, written, or printed, and the command accepts no credential-bearing
argument. The audit entry records the action and the target's identifier; the
description is omitted by the audit writer itself (`src/logging.rs:401-404`), so
this milestone introduces no new secret-bearing output.

### Documentation and static guards

See §12 for the document list. `tests/architecture.rs` needs no change: the
command module lives in the application layer and the scanner only governs
`crate::` path prefixes. `docs/LOGICAL_LAYERS.md` and the test's constant lists
must still be checked together.

## 7. Ordered work packages

### Work package A — Add the shared snippet field-update primitive

Intent: give every snippet-field write path one implementation of the mutation
sequence, shaped like the delete primitive, so the gate, the lock, the re-read,
the timestamp rule, the durable save, the audit entry, and the sync trigger
cannot drift between the TUI, the CLI, and the sibling milestone.

Required changes: in `src/commands/mod.rs`, add a mutation-value enum with the
four folder/favorite variants, an outcome enum with `Updated` / `Unchanged` /
`NotFound`, a sync-trigger enum mirroring `DeleteSyncTrigger`
(`src/commands/mod.rs:684-689`), and the primitive itself. The primitive locates
the target **by identifier with `!deleted`** inside the
`library::update_library` closure, applies the mutation, advances `updated_at`
with the same rule the delete primitive uses
(`updated_at.max(now).saturating_add(1)`, `src/commands/mod.rs:778`), writes the
best-effort audit entry with the action name derived from the mutation variant,
and fires the sync trigger only after the commit succeeded.

Acceptance evidence: the primitive appears exactly once in `src/commands/mod.rs`;
the audit action names are `favorite`, `unfavorite`, `add-folder`, `remove-folder`;
a unit test shows the identifier is unchanged and the `updated_at` advance matches
the delete primitive's rule; a test shows a mutation under an interrupted journal
is refused and writes nothing.

### Work package B — Add the `snp organize` command

Intent: expose the primitive as a non-interactive command.

Required changes: add `src/commands/organize_cmd.rs` with the canonical
`OrganizeArgs` clap struct and a `run` function. Build the selector through
`selector::exact_selector` and resolve it through `selector::resolve_selector`
with `ResolutionPolicy::Unique`. Handle the `SelectionResult` variants so that
one match mutates, many matches and ambiguity change nothing and report the
standard condition (exit 5), and not-found reports not found (exit 3). Refuse
`--library all` with the same rule and message shape `snp delete` uses. Validate
the action flags (exactly one) and the folder name (non-empty, no `;`). Call the
primitive, then emit the human-readable result on stdout or the JSON document in
`--json` mode.

Acceptance evidence: `snp organize --id <id> --favorite` sets the flag and exits
0; `--add-folder work` adds the membership and exits 0; an ambiguous selector
exits 5 and changes no file; a non-existent target exits 3; a missing action
flag and an invalid folder name exit 2.

### Work package C — Wire the command into the CLI

Intent: make the command reachable under both spellings.

Required changes: in `src/main.rs`, add the `Commands::Organize` variant with
its arguments; add the dispatch arm; add the `DataCommands::Organize` variant;
add the `snp data organize` dispatch arm; add the `command_behavior()` arms for
both spellings, classifying them as mutating (`StartupRecoveryPolicy::Allow`
with `StartupServices::Logging`, the same classification `snp delete` and `snp
data delete` carry at `src/main.rs:999-1000` and `src/main.rs:1014`).

Acceptance evidence: `snp organize --help` and `snp data organize --help` both
work; the two spellings behave identically; the startup recovery policy is
identical for both.

### Work package D — Test the field semantics and the identifier guarantee

Intent: prove the milestone's exit conditions.

Required changes: tests asserting that the four mutations leave the snippet's
identifier **byte-identical**; that the values round-trip through the save/load
path and the golden corpus (`tests/local_contracts.rs:217-280`) survives a
mutation round trip unchanged, i.e. the save path applies no TOML
post-processing; that removing the last folder membership removes the `folders`
key from the persisted TOML and clearing the favorite flag leaves
`favorite = false` present (`src/library/model.rs:48-51`); that `snp list
--json` and `snp list --csv` report the new values on the next read; that the
audit log contains the expected action and the target's identifier; that the
auto-sync notification fires after the mutation is durable, with
`MutationKind::SnippetUpdate` and `MutationOrigin::User`; that an ambiguous
selector changes nothing and exits 5; and that a not-found target exits 3 and
writes nothing.

Acceptance evidence: each test passes with exact-count assertions (not `>= 1`),
and the persisted file is compared byte for byte where the claim is about bytes.
The concrete assertions are listed in §10.

### Work package E — Document the command

Intent: make the new command discoverable and contract-complete.

Required changes: write the `docs/COMMANDS.md` section, the
`docs/COMMAND_CONTRACTS.md` row, the `docs/JSON_SCHEMAS.md` envelope, the
`docs/USER_GUIDE.md` scripting mention, and the `CHANGELOG.md` entry; write
`architecture/commands/organize_cmd.md`; add the deep-dive index entry in
`architecture/overview.md`; update `architecture/cli.md`,
`architecture/library.md`, and `architecture/sort.md`.

Acceptance evidence: every row required by
`.skills/cli-and-outcome-contract.md` exists; the deep-dive index resolves to an
existing file; the help output and the documentation agree; the JSON envelope
matches `docs/JSON_SCHEMAS.md` field for field.

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
`RecoveryClass::UnsafeFailed` refuse fail-closed. This milestone introduces no
second recovery model: the primitive reuses `library::update_library`, so the
journal state machine in `src/transaction.rs` is the only one.

**Durable-write failure.** The save goes through `save_library_internal`
(`src/library/persistence.rs:337-375`): a best-effort backup is taken first
(`:342-344`), the TOML is written through `write_private_atomic` (`:361-368`),
and the TOML cache is invalidated only after the write (`:370-372`). A write
failure propagates as an error, the mutation is reported as failed, the audit
entry is **not** written (the audit runs only on the success branch,
`src/commands/mod.rs:805-808`), and the auto-sync notification is **not** fired
(`src/commands/mod.rs:810-823`).

**Concurrent mutation from another process.** The local-data lock
(`src/local_data.rs:131`) serializes the load-modify-save cycle. Because the
library is re-read inside the critical section, a writer that acquires the lock
second observes the first writer's committed state and neither loses an update.
Two concurrent favorite flips of the same snippet produce two
`updated`/`unchanged` reports; at most one changes the value, and the identifier
is unchanged in both.

**Target disappears between resolution and mutation.** The primitive's lookup is
by identifier with `!deleted`; a target tombstoned by another process in the
window is not found, and the command reports the not-found outcome. No other
snippet is touched.

**Stale generations.** Not applicable. The library is re-read inside the critical
section, so the in-memory copy the caller resolved against is never written back.

**Duplicate delivery.** The mutation is idempotent in content: setting a favorite
flag that is already set, adding a folder the snippet already has, and removing a
folder it does not have are all no-ops reported as `unchanged`. The *report*
distinguishes them, so a script can tell whether the command had to write.

**Process restart between the mutation and the audit write.** The mutation is
durable; the audit entry is missing. The audit write is best-effort by design
(`src/commands/mod.rs:805-808`) and this is the accepted consequence.

**Auto-sync trigger.** The notification fires only after the mutation is durable.
If scheduling fails, the mutation stands and the failure is reported through
`report_notification_result` (`src/auto_sync/notification.rs:43-55`) on stderr.

**Merge consequence.** Because `updated_at` is part of the conflict identity
(`src/sync_commands.rs:1379`), a favorite flip advances the local `updated_at`
and can make the local copy win a merge against an older server copy. The local
`folders` and `favorite` are preserved in every merge branch
(`src/sync_commands.rs:1499-1500`, `:1520-1521`, `:1533-1534`), so the local-only
values are not lost — but a *remote* description or command change older than the
flip can be discarded. This is the same property the existing `snp edit --output`
path already has (`src/commands/edit_cmd.rs:216-217`). §14 turns an observed
instance of this into a stop-and-report condition rather than an invitation to
change the timestamp rule inside a milestone that is not chartered to change it.

## 9. Compatibility and migration

No data migration. No stored artifact changes format. No protocol change. No
configuration fallback is needed. No protocol negotiation is needed.

No existing command spelling, alias, exit code, or output format changes. The new
command is purely additive to the CLI surface, and the `snp data` mirror group
grows from six spellings to seven.

The persisted format does not change: `folders` and `favorite` already have their
serde representation (`src/library/model.rs:48-51`), and the save path writes
`toml::to_string_pretty` output verbatim
(`src/library/persistence.rs:361-368`). A library written before this change loads
unchanged, and a library written after it loads with a build from before.
`delete_snippet` and `snp delete` are untouched.

Removal criteria: none. This milestone adds a surface and removes nothing.

## 10. Required tests

### Focused unit tests

- The new primitive, in `src/commands/mod.rs`: the target's identifier is
  unchanged by a favorite flip and by a folder change, and the `updated_at`
  advance matches the delete primitive's rule
  (`updated_at.max(now).saturating_add(1)`).
- A no-op mutation (adding an existing folder, removing an absent one,
  re-setting the current favorite value) is reported as unchanged and does not
  alter the file's field values.
- A not-found target is reported distinctly and the library file is byte-identical
  before and after.
- The audit action names are exactly `favorite`, `unfavorite`, `add-folder`, and
  `remove-folder`.

### Integration tests

- `snp organize --id <id> --favorite` sets the flag, the snippet appears with
  `"favorite": true` in `snp list --json`, and the persisted file contains
  `favorite = true`.
- `snp organize --id <id> --unfavorite` clears it and the persisted file still
  contains `favorite = false` (the field has no `skip_serializing_if`).
- `snp organize --id <id> --add-folder work` adds the membership, `snp list
  --csv` reports `work` in the `folders` column, and `--remove-folder work`
  removes it and drops the `folders` key from the persisted TOML.
- The identifier is byte-identical before and after each of the four mutations.
- The golden corpus survives a mutation round trip: a snippet whose command
  contains tabs, trailing spaces, or CRLF is mutated and re-read unchanged.
- An ambiguous selector changes nothing and exits 5; a not-found target exits 3
  and writes nothing.
- The JSON output shape matches `docs/JSON_SCHEMAS.md`, field for field,
  including the `status` token.
- The audit log contains the expected action and the target's identifier, and no
  description text appears in the audit record.
- `snp data organize` produces the same persisted state as the top-level
  spelling, and `tests/cli_surface_compat.rs` covers the new mirrored spelling.

### Restart and recovery tests

- A mutation under an interrupted transaction journal is refused by the mutation
  gate, the command reports the refusal and directs the user to `snp repair`, and
  the library file is unchanged.
- A mutation interrupted mid-write leaves the library recoverable through
  `snp repair`.

### Contention and cancellation tests

- Two favorite flips of the same snippet run concurrently against one library:
  no update is lost and the identifier is unchanged.
- A favorite flip and an `snp delete` of the same snippet run concurrently:
  exactly one of them takes effect and the library remains loadable.

### Security and negative tests

- The command refuses to act under an interrupted journal and writes nothing.
- A folder name containing `;` is rejected as a usage error and writes nothing.
- A folder name that is empty or whitespace-only is rejected as a usage error
  and writes nothing.
- No credential appears in the command's output or in its audit entry.

### Migration and compatibility tests

- A library written before this change still loads, and a library written after
  it loads with a build from before, proving the persisted format did not
  change.
- The `snp data` mirror group's existing six spellings still behave identically
  to their top-level counterparts.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test integration --features test-support
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

- `docs/COMMANDS.md` — the new command's section and a row in the `snp data`
  mirror table.
- `docs/COMMAND_CONTRACTS.md` — the new command's row (interactive: No; reads
  stdin: No; writes stdout: Yes; writes stderr: Yes; may prompt: No; may
  mutate: Yes; may clipboard: No; may execute shell: No; may network: No;
  machine-output modes: `--json`; exit categories: 0, 1, 2, 3, 5).
- `docs/JSON_SCHEMAS.md` — the `snp organize --json` envelope.
- `docs/USER_GUIDE.md` — the scripting workflow.
- `docs/EXIT_CODES.md` — only if a code not already listed is used.
- `architecture/commands/organize_cmd.md` — a new deep dive.
- `architecture/overview.md` — the deep-dive index entry.
- `architecture/cli.md` — the command dispatch chain.
- `architecture/library.md` — the local-only vs synced field split table.
- `architecture/sort.md` — the `--favorites-first` note.
- `docs/IDENTITY_CONTRACT.md` — confirm the "Retains ID" lifecycle row.
- `CHANGELOG.md` — the new command.

## 13. Acceptance criteria

1. `snp organize` and `snp data organize` both set and clear a snippet's favorite
   flag and add and remove a folder membership, without a terminal UI.
2. The target is resolved through the mutating selector family
   (`selector::resolve_selector`), with the same ambiguity and not-found
   semantics as `snp delete`.
3. No mutation changes the snippet's identifier — verified by a test that asserts
   identifier stability across each of the four mutations.
4. The values round-trip through the save/load path, and the golden corpus
   survives a mutation round trip: the save path applies no TOML
   post-processing.
5. `snp list --json` and `snp list --csv` reflect the new values on the next
   read.
6. An audit entry names the action and the target's identifier, and a failed
   audit write does not fail the mutation.
7. The auto-sync notification fires after the mutation is durable, with
   `MutationKind::SnippetUpdate` and `MutationOrigin::User`.
8. An ambiguous selector changes nothing and exits 5; a not-found target exits 3;
   a usage violation exits 2.
9. The new command introduces **no new exit code** and changes no existing
   command's spelling, flags, exit code, or output format.
10. The new command has a `command_behavior()` entry for both spellings, a
    `docs/COMMANDS.md` section, a `docs/COMMAND_CONTRACTS.md` row, and a JSON
    output shape documented in `docs/JSON_SCHEMAS.md`.
11. `snp data organize` mirrors `snp organize` exactly, including the
    startup-recovery policy.
12. The persisted schema, the sync fingerprint, the synced field set, the merge
    order, and the conflict identity are unchanged.
13. MCP remains read-only, stdio-only, non-executing, and noninteractive; no MCP
    tool was added.
14. The TUI's keybindings, layout, and delete key are unchanged.
15. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh` pass.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the shared primitive cannot be written without changing the interactive
  delete's observable behavior, or without changing `delete_snippet` itself;
- the write path would require a persisted-schema change, a schema version, or a
  migration;
- identifier stability cannot be guaranteed — specifically, if implementing the
  write path would require re-deriving an identifier, or if a mutation
  demonstrably changes a snippet's identifier;
- the mutation would need a different selector semantic than the existing family
  uses, or would need the read-only resolver;
- the timestamp rule would have to change — if a favorite flip or folder change
  is observed to discard a *remote* description or command change that is newer
  than the flip, stop and report it: changing the shared primitive's
  `updated_at` rule affects the closed M001 delete path and is not chartered
  here;
- the scope would expand into the description / command / tags / output write
  path, into `snp edit`, into the library-level `Snippets.folders` registry,
  into the MCP surface, or into a bulk operation;
- a hard dependency is absent — in particular, if M001's shared delete primitive
  is not present at the baseline commit;
- repository evidence contradicts the claims in §3 — including the fingerprint
  claim, the `--favorites-first` claim, or the exit-code table.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
  results.
- The identifier-stability test results, one per mutation.
- The save/load round-trip and golden-corpus results.
- The list JSON and CSV output assertions.
- The audit and auto-sync notification assertions.
- The ambiguity, not-found, and usage-error outcome test results.
- The mutation-gate refusal test result.
- The `snp organize --help` and `snp data organize --help` output.
- A statement that no existing command spelling, exit code, or output format
  changed, and that the persisted format did not change.

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
- `snp update` is already the **self-update** command (`src/main.rs:95-101`).
  The sibling milestone that generalizes `snp edit --output` must not claim that
  name, and `snp edit` is **not** mirrored by `snp data` today
  (`src/main.rs:283-299`), so it gains no `snp data` variant.
- The mutation-value enum this milestone introduces is designed to be extended by
  the sibling milestone with the description / command / tags / output variants.
  The sibling milestone reuses this primitive; it must not add a second one.

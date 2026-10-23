# delete_cmd — Deterministic Non-Interactive Snippet Delete

[← Back to Overview](../overview.md)

## Overview

`src/commands/delete_cmd.rs` (≈370 lines) tombstones one snippet without a
TUI, an execution, or a confirmation prompt. It resolves its target through
the **mutating** selector (`selector::resolve_selector`) and applies the one
shared snippet-delete primitive, so the persisted state it produces is
byte-identical to the state the interactive TUI delete produces.

This is the scripting counterpart of the TUI's `d` key: the same tombstone,
the same timestamp rule, the same audit entry, the same auto-sync trigger —
reached through a selector instead of a picker.

## CLI surface

`DeleteArgs` (`delete_cmd.rs:22`), `snp delete`:

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `--id` | `Option<String>` | – | Exact snippet ID (case-sensitive; conflicts with the other three selectors) |
| `--description-exact` | `Option<String>` | – | Exact description (case-insensitive) |
| `--command-exact` | `Option<String>` | – | Exact command text (case-insensitive) |
| `-q/--query` | `Option<String>` | – | Fuzzy query (selector semantics, not the TUI's `--filter` pre-filter) |
| `-l/--library` | `Option<String>` | – | Scope: one library name. `all` is **refused** |
| `--json` | bool | `false` | Pretty `DeleteJsonOutput` |

At least one of the four selectors is required
(`delete_cmd.rs:88-95`). `--library all` is refused with an explicit error
(`delete_cmd.rs:97-108`): a delete tombstones one snippet in one library
file, and the cross-library scope is a bulk operation, which is out of
scope for this milestone. `snp search` refuses the same scope for the same
reason (`src/commands/search_cmd.rs:32-44`).

`DeleteJsonOutput` (`delete_cmd.rs:48`) is
`{schema, status, id, description, library}` with `schema: 1` and
`status` ∈ {`deleted`, `already_deleted`}.

## The shared primitive

`commands::delete_snippet` (`src/commands/mod.rs:744`) is the **only**
implementation of the delete sequence. It takes a library path, a target
identifier, and a sync trigger, and performs, in one place:

1. the mutation gate and the local-data lock, through
   `library::update_library` (`src/library/persistence.rs:294`);
2. the re-read of the library inside the critical section, so a snippet
   created elsewhere while the caller was resolving its target is not
   overwritten by a stale in-memory copy;
3. the lookup of the target **by identifier** with `!deleted`, so a
   concurrent insertion cannot shift the target;
4. the tombstone: `deleted = true` and
   `updated_at = updated_at.max(now).saturating_add(1)` — the saturating
   add makes the tombstone timestamp strictly greater than the live
   version's, which is what lets a deletion beat live content in the
   client merge;
5. the durable save through the atomic write path;
6. the `delete` audit entry — best effort; a failed write is logged at
   debug level and does not fail the delete;
7. the sync trigger, **after** the commit succeeded.

The primitive contains no picker-specific logic: no display index and no
in-memory mirror. The interactive path mirrors the tombstone into the
picker's in-memory copy itself, in `commands::apply_snippet_delete`
(`src/commands/mod.rs:836`), which the TUI selection loop calls
(`src/commands/mod.rs:611-620`).

When the target cannot be tombstoned, the mutation closure fails, so
`update_library` never reaches its save and the library file is left
untouched. The failure is classified inside the closure and surfaces as
`SnippetDeleteOutcome::AlreadyDeleted` (the identifier exists and already
carries a tombstone) or `SnippetDeleteOutcome::NotFound` (the identifier
matches nothing) — two distinct verdicts, not a generic error.

### Sync trigger

`DeleteSyncTrigger` (`src/commands/mod.rs:684`) selects what happens after
the commit:

| Trigger | Behavior |
|---------|----------|
| `AutoNotify` | `auto_sync::notify_mutation(MutationKind::SnippetDelete, MutationOrigin::User)` — the command's trigger |
| `ExplicitSync(&Runtime)` | `run_explicit_sync` — the TUI's `--sync` trigger |

The TUI's trigger choice is unchanged: `runtime.is_some()` holds exactly
when `--sync` was passed (`src/commands/mod.rs:551-561`).

## Flow / steps

`run()` (`delete_cmd.rs:81`):

1. Reject an empty selector set (usage error) and the `all` scope.
2. Build the selector through `selector::exact_selector`
   (`src/selector.rs:601`) and apply `ResolutionPolicy::Unique` — a delete
   never fans out. The targeting fields are re-applied on top, exactly as
   `snp get` does (`src/commands/get_cmd.rs:149-165`).
3. `selector::resolve_selector` (`src/selector.rs:703`) — the **mutating**
   member of the selector family, which runs `ensure_library_mode()`.
   That is the right resolver here because the command mutates.
4. `One(match)` → `delete_snippet(match.library_path, match.snippet.id,
   AutoNotify)` → report.
   `Many` / `Ambiguous` → report the candidates on stderr, change nothing,
   exit 5.
   `NotFound` → probe for a tombstone (§ below); report already-deleted or
   not-found.
5. The probe (`delete_cmd.rs:278`) re-runs the *same* selector against a
   copy of the library whose `deleted` flags are cleared, so the selector's
   own matching rules decide whether a tombstone matches. The selector
   excludes deleted snippets from every mode, so without the probe an
   already-deleted target would be indistinguishable from one that never
   existed. The fuzzy ranking and the case-sensitivity rules are not
   re-implemented.

## Outcomes and exit codes

| Situation | stdout | stderr | Exit |
|-----------|--------|--------|------|
| Deleted | result document | – | 0 |
| Already deleted | result document (`status: "already_deleted"`) | – | 9 (`CONFLICT_OR_REFUSED`) |
| Not found | – | diagnostic | 3 (`NOT_FOUND`) |
| Ambiguous | – | candidate list | 5 (`AMBIGUOUS`) |
| Usage error / refused scope | – | error | 2 / 1 |

Exit 9 is the existing `CONFLICT_OR_REFUSED` code ("destructive action
refused"): the target is already tombstoned, so the command refuses to act
and says so instead of claiming a delete it did not perform. No new exit
code is introduced.

## Stream split

The human-readable result goes to **stdout** and diagnostics go to
**stderr**, per `docs/CLI_EXITCODE_STREAM_POLICY.md`:

- stdout carries the result document — the human line in the default mode,
  the JSON document with `--json` — and only when a target was identified;
- stderr carries the diagnostics that explain why no result document was
  produced (not found, ambiguous).

`snp get` suppresses its ambiguity list in `--json` mode; `snp delete`
keeps the diagnostics on stderr in both modes, because stderr is not the
machine-readable channel.

## Mutation vs read-only

`snp delete` is a mutation command. It runs the mutation gate through
`library::update_library`, takes the local-data lock, and writes through
the durable atomic write path. It never reads a credential and never
prints one.

## Startup behavior

`command_behavior()` (`src/main.rs:923`) classifies both spellings as
mutating:

| Spelling | Policy | Services |
|----------|--------|----------|
| `snp delete` | `StartupRecoveryPolicy::Allow` | `StartupServices::Logging` (`src/main.rs:1014`) |
| `snp data delete` | `StartupRecoveryPolicy::Allow` | `StartupServices::Logging` (`src/main.rs:999`) |

`Allow` permits startup recovery (a preserved pending sync generation is
retried) and `Logging` initializes the file log and the audit service.
The `snp data` mirror carries the same entry, so the mirror is not a way
to bypass the startup-recovery policy.

## The `snp data delete` mirror

`DataCommands::Delete` (`src/main.rs:298`) reuses the same `DeleteArgs`
type, and both spellings route through the single-path handler
`handle_delete` (`src/main.rs:477`) — the same code path the other five
mirrored commands use. `snp data delete` behaves identically to
`snp delete`, including the startup-recovery policy.

## Key invariants

- The identifier is deterministic and is **not** reassigned by a delete;
  the tombstone keeps the snippet's existing identifier.
- Deletion is a tombstone, not a row removal: the `deleted` flag is set
  and the tombstone is preserved for sync.
- The tombstone timestamp is strictly greater than the live version's, so
  the client merge keeps treating deletion as beating live content.
- The mutation gate runs before the local mutation. A corrupt or
  multi-journal state refuses the mutation and directs the user to
  `snp repair`.
- The mutation re-reads the library inside the local-data lock rather than
  writing a stale in-memory copy.
- The audit write is best effort: a failed audit write does not fail the
  delete.
- The auto-sync notification fires only after the mutation is durable.
- An ambiguous selector deletes nothing and changes no file.
- MCP stays read-only, stdio-only, non-executing, and noninteractive.

## Gotchas

- There is **no** confirmation prompt. The selector's ambiguity rules are
  what protect against deleting the wrong target; a prompt would defeat
  the purpose of a non-interactive command.
- A delete is not idempotent in the "safe to run twice" sense: the second
  run finds the target already tombstoned and reports exit 9, not exit 0.
  The tombstone itself is idempotent; the *report* is not.
- `--library all` is refused. Use `snp list --library all` to browse and
  `snp get --library all` to read.
- `--query` is the selector's fuzzy match. It is not the TUI's `--filter`
  pre-filter: there is no TUI here, so the selector's ranking decides.
- The command does not take a `--sync` flag. The auto-sync notification is
  the trigger; the command never initializes the Tokio runtime.

## Key Files

- `src/commands/delete_cmd.rs` — the command: args, selector construction,
  outcomes, reporting.
- `src/commands/mod.rs` — the shared primitive `delete_snippet`, the sync
  trigger, and the TUI's `apply_snippet_delete` wrapper.
- `src/selector.rs` — `exact_selector`, `resolve_selector`,
  `ResolutionPolicy`.
- `src/library/persistence.rs` — `update_library` (gate, lock, re-read,
  durable save).
- `src/outcome.rs` — `CliOutcome` and the exit-code table.
- `src/main.rs` — dispatch (`src/main.rs:889`, `src/main.rs:909`) and
  `command_behavior()` (`src/main.rs:999`, `src/main.rs:1014`).

## File / line references

- `DeleteArgs`: `src/commands/delete_cmd.rs:22`; `DeleteJsonOutput`: `:48`;
  `DeleteStatus`: `:58`; `run`: `:81`; `report_outcome`: `:169`;
  `emit_result`: `:194`; `report_ambiguous`: `:229`;
  `scope_library_path`: `:258`; `probe_deleted_target`: `:278`
- Primitive: `src/commands/mod.rs:744` (`delete_snippet`); trigger enum:
  `:684`; outcome enum: `:697`; TUI wrapper: `:836`
- Selector: `src/selector.rs:601` (`exact_selector`), `:703`
  (`resolve_selector`)
- Dispatch: `src/main.rs:204` (`Commands::Delete`), `:298`
  (`DataCommands::Delete`), `:477` (`handle_delete`), `:889`/`:909`
  (dispatch arms)

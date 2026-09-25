# edit_cmd — Edit in $EDITOR, Manage Output Field

[← Back to Overview](../overview.md)

## Overview

`src/commands/edit_cmd.rs` (293 lines) has two distinct modes. `run()`
opens the whole library TOML in the user's editor (whole-file mutation
with byte-change detection). `run_edit_output()` /
`run_edit_output_by_id()` mutate only the local-only `output`/notes
field of one snippet. `main.rs:614-671` selects the mode.

## CLI surface

`EditArgs` (`edit_cmd.rs:9`), `snp edit` (alias `e`):

| Flag | Meaning |
|------|---------|
| `-l/--library` | Target library (else primary/default path) |
| `--output TEXT` | Set output field (conflicts with `--output-stdin`, `--clear-output`; requires `--filter` or exact selector) |
| `--output-stdin` | Read output field from stdin (same requirement) |
| `--clear-output` | Clear output field (sets `""`) |
| `-f/--filter` | Substring match on description/command for output editing |
| `--id` | Exact UUID bypass → `run_edit_output_by_id` |
| `--description-exact` / `--command-exact` | Exact bypass → `run_edit_output_by_id` |

Without output flags, `run(library)` opens the editor regardless of
filter/exact flags.

## Flow / steps

### `run()` — whole-file edit (`edit_cmd.rs:36`)

1. Resolve path: named library via `get_library_path`, else primary or
   `LibraryManager::get_default_snippets_path`. Create parents + empty
   file if missing.
2. Resolve editor via `new_cmd::resolve_editor_spec` (`$VISUAL >
   $EDITOR > vim`, shell-word args, PATH/CWD validation).
3. Snapshot pre-editor bytes; `gate_mutation_on_interrupted_transactions`
   (`:73`) — same invariant as every writer.
4. Spawn editor (no shell), capture exit status, re-read bytes.
5. Take the local-data lock for the observe-and-notify window (`:101`);
   `changed = before != after`.
6. Editor failed + changed → notify `SnippetUpdate/User`, then error
   noting the save survived. Editor failed + unchanged → error, no
   notify. Success + changed → notify. Success + unchanged → silent.

### Output-field edits (`:143`, `:233`)

1. Resolve `lib_path` (missing library → error, never synthesize).
2. `load_library`; find target: `run_edit_output` uses
   case-insensitive substring on description/command and errors on zero
   or 2+ matches (lists candidates, demands an exact selector);
   `run_edit_output_by_id` matches `id == snippet_id`.
3. Set `output` (`None` defensively clears), bump
   `updated_at = max(now)+1`, `save_library`, report to stderr.

## Mutation vs read-only

Mutating in all modes. Whole-file edit gates **before** the editor and
locks after; output edits ride `save_library`'s internal gate + lock.
Neither output path notifies auto-sync (see below).

## Auto-sync trigger

- Whole-file edit: `notify_mutation(MutationKind::SnippetUpdate,
  MutationOrigin::User)` **only when bytes changed** (`:107`, `:130`).
  Unchanged sessions create no pending intent.
- Output-field edits: **no notification** — `output` is local-only,
  excluded from the sync fingerprint and absent from `ProtoSnippet`.

## Error / exit mapping

`SnipResult<()>`: library-not-found, editor-not-found / spawn /
nonzero exit (with changed/unchanged detail), no-match, ambiguous
filter (exit 1 family with remediation text). Output-value intake
(stdin read) errors are I/O errors.

## Key invariants

- Editor sessions are interactive and cannot hold the local lock; only
  the short post-editor compare-and-notify is critical-sectioned.
- Notify on byte change even when the editor reports failure — the
  bytes on disk are the source of truth, not the exit status.
- `output` edits require `--filter` (or exact selector) by CLI
  contract; `snp edit --output` without targeting is a usage error in
  `main.rs`.
- Whole-file edit never parses or normalizes TOML itself.

## File / line references

- `EditArgs`: `src/commands/edit_cmd.rs:9`; `run`: `:36` (gate `:73`)
- `run_edit_output`: `:143`; `run_edit_output_by_id`: `:233`
- Dispatch: `src/main.rs:614-671`

# edit_cmd — Edit in $EDITOR, Manage Output Field

[← Back to Overview](../overview.md)

## Overview

`src/commands/edit_cmd.rs` (293 lines) has two distinct modes. `run()`
opens the whole library TOML in the user's editor (whole-file mutation
with byte-change detection). `run_edit_output()` /
`run_edit_output_by_id()` mutate only the local-only `output`/notes
field of one snippet. `src/main.rs:641-710` selects the mode.

This command does not cover editing any other field: the whole-file mode
lets the user change anything the editor can, but the *targeted* mode can
only touch `output`.

## CLI surface

`EditArgs` (`edit_cmd.rs:9`), `snp edit` (alias `e`):

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `-l/--library` | `Option<String>` | – | Target library (else primary/default path) |
| `--output TEXT` | `Option<String>` | – | Set the output field (conflicts with `--output-stdin`, `--clear-output`) |
| `--output-stdin` | bool | `false` | Read the output field from stdin (same conflicts) |
| `--clear-output` | bool | `false` | Clear the output field (sets `""`) (same conflicts) |
| `-f/--filter` | `Option<String>` | – | Substring match on description/command for output editing |
| `--id` | `Option<String>` | – | Exact snippet ID bypass → `run_edit_output_by_id`; conflicts with `--filter` |
| `--description-exact` | `Option<String>` | – | Exact bypass → `run_edit_output_by_id`; conflicts with `--filter` |
| `--command-exact` | `Option<String>` | – | Exact bypass → `run_edit_output_by_id`; conflicts with `--filter` |

Without output flags, `run(library)` opens the editor regardless of
filter/exact flags (`src/main.rs:707-709`).

### Mode selection (`src/main.rs:641-710`)

- Any of `--output` / `--output-stdin` / `--clear-output` present →
  the **output-field** mode.
  - An exact selector (`--id` / `--description-exact` / `--command-exact`)
    → `run_edit_output_by_id` (`src/main.rs:676-698`).
  - Otherwise `--filter` is **required**: `src/main.rs:700-704` returns
    `--filter is required when using --output, --output-stdin, or --clear-output`
    when it is absent. The exact selectors are the alternative to
    `--filter`, not an addition to it: `--filter` **conflicts** with
    `--id` / `--description-exact` / `--command-exact` at the clap level
    (exactly like `snp run` / `snp clip`), so the combination is a usage
    error (exit 2) instead of a silent drop.
- No output flags → the **whole-file** mode.

## Flow / steps

### `run()` — whole-file edit (`edit_cmd.rs:36`)

1. Resolve path: named library via `get_library_path` (`edit_cmd.rs:38`),
   else primary or `LibraryManager::get_default_snippets_path`
   (`edit_cmd.rs:50-53`). A named library that does not exist is a hard
   error with a remediation hint (`edit_cmd.rs:40-48`). Create parents +
   empty file if missing (`edit_cmd.rs:54-59`).
2. Resolve editor via `new_cmd::resolve_editor_spec` (`edit_cmd.rs:60`):
   `$VISUAL` > `$EDITOR` > `vim`, shell-word args, PATH/CWD validation.
3. Snapshot pre-editor bytes (`edit_cmd.rs:66`).
4. **Gate** — `gate_mutation_on_interrupted_transactions(&sync_state_dir,
   &transaction_dir)` (`edit_cmd.rs:71-76`): same invariant as every
   writer. One journal = auto-rollback; multiple/incomplete = refuse and
   direct the user to `snp repair`.
5. Spawn editor (no shell), capture exit status (`edit_cmd.rs:78-93`),
   re-read bytes (`edit_cmd.rs:95`).
6. Take the local-data lock for the observe-and-notify window
   (`edit_cmd.rs:101`); `changed = before != after` (`edit_cmd.rs:103`).
7. Editor failed + changed → notify `SnippetUpdate/User`, then error
   noting the save survived. Editor failed + unchanged → error, no
   notify. Success + changed → notify. Success + unchanged → silent
   (`edit_cmd.rs:105-136`).

### Output-field edits (`edit_cmd.rs:143`, `edit_cmd.rs:233`)

1. Resolve `lib_path` via `get_library_path` (missing library → error,
   never synthesize, `edit_cmd.rs:151-160` / `edit_cmd.rs:241-250`).
2. `load_library`; find the target:
   - `run_edit_output` uses a **case-insensitive substring** match on
     description/command (`selector::contains_ignore_ascii_case`,
     `edit_cmd.rs:173-174`) and errors on zero matches
     (`edit_cmd.rs:181-186`) or 2+ matches (lists candidates and demands
     an exact selector, `edit_cmd.rs:187-205`).
   - `run_edit_output_by_id` matches `id == snippet_id`
     (`edit_cmd.rs:254-260`); a miss is a "Snippet not found" error
     (`edit_cmd.rs:262-272`).
   - Both skip `deleted` tombstones (`edit_cmd.rs:171`, `edit_cmd.rs:258`).
3. Set `output` (`None` defensively clears it, `edit_cmd.rs:209` /
   `edit_cmd.rs:275`), bump `updated_at = max(now)+1`
   (`edit_cmd.rs:210-211` / `edit_cmd.rs:276-277`), `save_library`
   (`edit_cmd.rs:215` / `edit_cmd.rs:281`), report to stderr
   (`edit_cmd.rs:217-221` / `edit_cmd.rs:283-287`).

## Mutation vs read-only

Mutating in all modes. The whole-file edit gates **before** the editor
and locks after; the output edits ride `save_library`'s internal gate +
lock (`src/library/persistence.rs:266-274`).

`edit` calls `gate_mutation_on_interrupted_transactions()` directly in
the whole-file mode (`edit_cmd.rs:73`) and indirectly through
`save_library` in the output modes.

Note that `get_library_path` (`edit_cmd.rs:38`, `edit_cmd.rs:151`,
`edit_cmd.rs:241`) is the **mutating** resolver: it calls
`ensure_library_mode()`, so `snp edit` can migrate a legacy single-file
checkout.

The exact-selector path is mutating too, by a different route: `main.rs`
resolves it with `selector::resolve_exact_target`
(`src/main.rs:678`), which calls `resolve_selector`
(`src/selector.rs:703`) → `ensure_library_mode()`
(`src/selector.rs:705`). So `snp edit --id …` can rewrite library metadata
even though the targeted edit itself only touches `output`.

## Auto-sync trigger

- Whole-file edit: `notify_mutation(MutationKind::SnippetUpdate,
  MutationOrigin::User)` **only when bytes changed**
  (`edit_cmd.rs:107-110`, `edit_cmd.rs:130-133`). Unchanged sessions
  create no pending intent.
- Output-field edits: **no notification** — `output` is local-only,
  excluded from the sync fingerprint and absent from `ProtoSnippet`.

`src/main.rs:993` classifies `Commands::Edit(_)` as a mutation command
(`StartupRecoveryPolicy::Allow` + `StartupServices::Logging`).

## Error / exit mapping

`SnipResult<()>`: library-not-found, editor-not-found / spawn /
nonzero exit (with changed/unchanged detail), no-match, ambiguous
filter (exit 1 family with remediation text). Output-value intake
(stdin read) errors are I/O errors (`src/main.rs:661-671`).

## Key invariants

- Editor sessions are interactive and cannot hold the local lock; only
  the short post-editor compare-and-notify is critical-sectioned.
- Notify on byte change even when the editor reports failure — the
  bytes on disk are the source of truth, not the exit status.
- `output` edits require `--filter` **or** an exact selector; the
  CLI contract in `src/main.rs:700-704` enforces it, and the two are
  mutually exclusive at the clap level.
- Whole-file edit never parses or normalizes TOML itself — the editor
  writes the bytes and snip-it only compares them.
- `output` is local-only: it is excluded from the sync fingerprint, so
  editing it never creates sync intent.

## Gotchas

- The whole-file mode creates the library file if it does not exist
  (`edit_cmd.rs:54-59`) before the editor opens — a stray `snp edit` on a
  fresh install materializes an empty library file.
- The output-field filter is a **substring** match, while
  `snp list --filter` is a **fuzzy** match. The same string can select
  different snippets in the two commands.
- A failed editor that still wrote bytes produces *both* an error exit
  and a sync notification.
- `--output`, `--output-stdin`, and `--clear-output` are mutually
  exclusive at the clap level (`edit_cmd.rs:13-20`); supplying two is a
  usage error before any code runs.

## Key Files

- `src/commands/edit_cmd.rs` — the command: whole-file editor mode and
  the two output-field mutation paths.
- `src/commands/new_cmd.rs` — `resolve_editor_spec`, the shared editor
  resolution.
- `src/commands/mod.rs` — `get_library_path`, the mutating resolver.
- `src/library/persistence.rs` — `save_library`, the gate → lock →
  atomic-write path.
- `src/selector.rs` — `contains_ignore_ascii_case`, the output-mode
  filter match.
- `src/main.rs` — dispatch and mode selection (`src/main.rs:641-710`).

## File / line references

- `EditArgs`: `src/commands/edit_cmd.rs:9`; `run`: `:36` (gate `:73`,
  lock `:101`)
- `run_edit_output`: `:143`; `run_edit_output_by_id`: `:233`
- Dispatch: `src/main.rs:641-710`

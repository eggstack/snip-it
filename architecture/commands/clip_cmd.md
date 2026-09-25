# clip_cmd — Copy Snippet to Clipboard

[← Back to Overview](../overview.md)

## Overview

`src/commands/clip_cmd.rs` (158 lines) copies a snippet's expanded
command to the system clipboard. It mirrors `run_cmd`'s selection
plumbing (TUI + exact bypass + optional `--sync`) but never spawns a
shell. `copy_to_clipboard()` is also the shared clipboard side-effect
funnel used by `run --copy`.

## CLI surface

`ClipArgs` (`clip_cmd.rs:9`), `snp clip` (alias `c`): `-f/--filter`,
`--sync`, `-l/--library`, `--sort` (default `Relevance`),
`--favorites-first`, `--id` / `--description-exact` / `--command-exact`
(exact bypass, conflict with each other and `--filter`).
`main.rs:546-583` routes exact selectors through `resolve_exact_target`
→ `run_exact`, else `run`.

## Flow / steps

### `run()` (`clip_cmd.rs:99`)

`run_snippet_selection(filter, library, do_sync, allow_delete=true,
sort_opts, runtime, process_snippet)`; outcome is discarded, always
`Ok(())`. The TUI delete shortcut is available here, as in `run`.

### `run_exact()` (`clip_cmd.rs:68`)

1. Require a runtime when `do_sync` (`"sync requested but no runtime"`).
2. `expand_snippet_command`: `Cancel/Skip` → return `Ok(())` silently.
3. `copy_to_clipboard(snippet, final_command)`.
4. If `do_sync`, trailing `run_explicit_sync` (warn-only on failure).

### `process_snippet()` (`clip_cmd.rs:51`)

Expand (cancel → `Cancel`, skip → `Continue`), copy, return
`Done("Copied to clipboard")`. The `_copy_flag` is ignored — clip
always copies.

### `copy_to_clipboard()` (`clip_cmd.rs:40`)

The single funnel for **all** clipboard writes:

1. `clipboard::copy_to_clipboard_auto(final_command)` (`?` — failure
   aborts before bookkeeping).
2. `logging::audit_log("copy", snippet, None)` (`?`).
3. `UsageIndex::load → record_use(id) → save` (save failure is
   debug-logged only, never fatal).

## Mutation vs read-only

No library mutation: no transaction gate, no `save_library`, no pending
marker. Side effects are clipboard + audit log + usage index. (The TUI
delete shortcut can still mutate via the shared selection loop.)

## Auto-sync trigger

No `notify_mutation` of its own. With `--sync`, `run_exact` and the
shared loop finish with `run_explicit_sync` under the execution lock.
Without `--sync`, copying leaves no sync intent.

## Error / exit mapping

`SnipResult<()>`; clipboard or audit failure propagates as
`SnipError` (exit 1); usage-save failure is swallowed to debug log.
Cancel/skip in exact mode is silent success. `run_exact` with
`do_sync` and no runtime errors before touching the clipboard.

## Key invariants

- All clipboard side effects go through `copy_to_clipboard()`
  (AGENTS.md); do not add ad-hoc clipboard writes.
- Variable expansion happens in the caller, never inside
  `copy_to_clipboard` — the function takes the final string.
- Audit precedes usage; usage failure never fails the command.
- Unit tests touching the live clipboard are `#[ignore]`d
  (`clip_cmd.rs:119-158`); only the no-runtime guard runs in CI.

## File / line references

- `ClipArgs`: `src/commands/clip_cmd.rs:9`
- `copy_to_clipboard`: `:40`; `process_snippet`: `:51`
- `run_exact`: `:68`; `run`: `:99`
- Dispatch: `src/main.rs:546-583`

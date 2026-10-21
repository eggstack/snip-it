# clip_cmd — Copy Snippet to Clipboard

[← Back to Overview](../overview.md)

## Overview

`src/commands/clip_cmd.rs` (158 lines) copies a snippet's expanded
command to the system clipboard. It mirrors `run_cmd`'s selection
plumbing (TUI + exact bypass + optional `--sync`) but never spawns a
shell. `copy_to_clipboard()` is also the shared clipboard side-effect
funnel used by `run --copy`.

This command does not cover the two direct `copy_to_clipboard_auto` call
sites outside it — see *Direct clipboard call sites* below.

## CLI surface

`ClipArgs` (`clip_cmd.rs:9`), `snp clip` (alias `c`):

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `-f/--filter` | `Option<String>` | – | Initial TUI filter |
| `--sync` | bool | `false` | Explicit sync after the session |
| `-l/--library` | `Option<String>` | – | Library scope |
| `--sort` | `SnippetSort` | `Relevance` | Ordering |
| `--favorites-first` | bool | `false` | Favorites rank first |
| `--id` | `Option<String>` | – | Exact snippet ID bypass (conflict with the other selectors + filter) |
| `--description-exact` | `Option<String>` | – | Exact description bypass |
| `--command-exact` | `Option<String>` | – | Exact command bypass |

`src/main.rs:573-611` routes exact selectors through
`resolve_exact_target` → `run_exact`, else `run`.

## Flow / steps

### `run()` (`clip_cmd.rs:99`)

`run_snippet_selection(filter, library, do_sync, allow_delete=true,
sort_opts, runtime, process_snippet)`; outcome is discarded, always
`Ok(())` (`clip_cmd.rs:107-116`). The TUI delete shortcut is available
here, as in `run`.

### `run_exact()` (`clip_cmd.rs:68`)

1. Require a runtime when `do_sync` (`"sync requested but no runtime"`,
   `clip_cmd.rs:73-82`).
2. `expand_snippet_command`: `Cancel/Skip` → return `Ok(())` silently
   (`clip_cmd.rs:83-87`).
3. `copy_to_clipboard(snippet, final_command)` (`clip_cmd.rs:88`).
4. If `do_sync`, trailing `run_explicit_sync` (warn-only on failure,
   `clip_cmd.rs:89-94`).

### `process_snippet()` (`clip_cmd.rs:51`)

Expand (cancel → `Cancel`, skip → `Continue`), copy, return
`Done("Copied to clipboard")` (`clip_cmd.rs:55-64`). The `_copy_flag` is
ignored — clip always copies.

### `copy_to_clipboard()` (`clip_cmd.rs:40`)

```rust
pub(crate) fn copy_to_clipboard(snippet: &Snippet, final_command: &str) -> SnipResult<()>
```

The funnel for per-snippet clipboard writes (`clip`, the exact bypass, and
the `snp run` TUI copy branch at `src/commands/run_cmd.rs:202`):

1. `clipboard::copy_to_clipboard_auto(final_command)` (`clip_cmd.rs:41`) —
   `?`, so a failure aborts before bookkeeping.
2. `logging::audit_log("copy", snippet, None)` (`clip_cmd.rs:42`) — `?`.
3. `UsageIndex::load → record_use(id) → save` (`clip_cmd.rs:43-47`); a
   save failure is debug-logged only, never fatal.

Variable expansion is **not** performed here — the caller expands first
and passes the final string.

## Clipboard mechanics (`src/clipboard.rs`)

`copy_to_clipboard_auto` (`src/clipboard.rs:166`) reads the configured
auto-clear interval from the sync settings
(`clipboard_auto_clear_seconds`, `src/config/sync_settings.rs:124`), then
calls `copy_to_clipboard_with_auto_clear` (`src/clipboard.rs:155`):

1. `copy_to_clipboard(text)` — the platform primitive: `clipboard-win` on
   Windows (`src/clipboard.rs:177`), `arboard` elsewhere
   (`src/clipboard.rs:196`). Both are wrapped in
   `with_clipboard_timeout` (`src/clipboard.rs:112`), which runs the
   operation on a worker thread and gives up after
   `SNP_CLIPBOARD_TIMEOUT` seconds (default 5,
   `src/clipboard.rs:62-68`). On timeout the worker is detached and its
   result discarded; the CLI gets a `clipboard_error`.
2. `schedule_clipboard_clear(seconds)` (`src/clipboard.rs:75`) when an
   interval is configured.

### The generation counter

`schedule_clipboard_clear` uses a process-global
`CLIPBOARD_GENERATION: AtomicU64` (`src/clipboard.rs:24`):

- Each scheduled clear bumps the counter and remembers its own value
  (`src/clipboard.rs:80`).
- The clear thread sleeps for the interval, then re-reads the counter and
  only clears if the value is still its own (`src/clipboard.rs:84-85`).

So a **new copy cancels a pending clear**: copying again before the
interval elapses bumps the generation, and the older thread's check
fails, so it does not wipe the newer content.

### Clear-on-exit is best effort

The clear runs on a **detached thread**, not at process exit. If the
process exits before the interval elapses, the clear never happens
(`src/clipboard.rs:73-74` documents it; the spawn is
`src/clipboard.rs:82`, and the handle is dropped, never joined). The
auto-clear is a best-effort security measure, not a guarantee — do not
describe it as "the clipboard is cleared when the command exits".

`clipboard_auto_clear_seconds` is cached per process
(`src/clipboard.rs:32-60`); `invalidate_clipboard_settings_cache()`
(`src/clipboard.rs:55`) drops the cache when settings change.

## Direct clipboard call sites

Two call sites bypass `copy_to_clipboard()` and call
`clipboard::copy_to_clipboard_auto` directly. Both are outside this
module, and both are recorded here because they behave differently:

| Call site | Audits? | Usage index? |
|-----------|---------|--------------|
| `src/ui/mod.rs:1733` — TUI visual-range multi-select (`y` in visual mode) | Audits **only the first snippet** of the range (`src/ui/mod.rs:1748-1759`) | No |
| `src/commands/cron_cmd.rs:89` — cron-entry copy prompt | **No audit at all** | No |

The TUI multi-select joins the selected commands with `\n`
(`src/ui/mod.rs:1726-1732`) after `strip_escape_sequences`, so a
multi-snippet copy is a single clipboard write with a single audit
entry — the other snippets in the range are copied but not audited.

## Mutation vs read-only

No library mutation: no transaction gate, no `save_library`, no pending
marker. `clip` never calls `gate_mutation_on_interrupted_transactions()`.
Side effects are clipboard + audit log + usage index. (The TUI delete
shortcut can still mutate via the shared selection loop.)

## Auto-sync trigger

No `notify_mutation` of its own. With `--sync`, `run_exact` and the
shared loop finish with `run_explicit_sync` under the execution lock.
Without `--sync`, copying leaves no sync intent.

## Tokio runtime

`clip` is one of the commands that may initialize the global runtime —
but only for `--sync`. `src/main.rs:587` and `src/main.rs:608` pass
`args.sync.then_some(&RUNTIME)`, so the runtime is `Some` only when
`--sync` is set. `run_exact` (`clip_cmd.rs:73-82`) errors with
`runtime_error("sync requested but no runtime")` if `do_sync` is true and
the runtime is `None`.

## Error / exit mapping

`SnipResult<()>`; clipboard or audit failure propagates as
`SnipError` (exit 1); usage-save failure is swallowed to debug log.
Cancel/skip in exact mode is silent success. `run_exact` with
`do_sync` and no runtime errors before touching the clipboard.

## Key invariants

- Per-snippet clipboard side effects go through `copy_to_clipboard()`
  (AGENTS.md); the TUI visual-range multi-select and `cron_cmd.rs:89` are
  the two documented exceptions.
- Variable expansion happens in the caller, never inside
  `copy_to_clipboard` — the function takes the final string.
- Audit precedes usage; usage failure never fails the command.
- A new copy cancels a pending auto-clear (generation counter).
- Unit tests touching the live clipboard are `#[ignore]`d
  (`clip_cmd.rs:123-147`); only the no-runtime guard runs in CI
  (`clip_cmd.rs:149-157`).

## Gotchas

- The auto-clear is a detached thread, so a fast-exiting command can
  leave the copied text on the clipboard.
- The TUI multi-select audits one snippet but copies several — an audit
  reader must not infer "one copy = one audited snippet".
- `copy_to_clipboard_auto` can time out (`SNP_CLIPBOARD_TIMEOUT`,
  default 5 s) and return an error even though the underlying clipboard
  write may still complete on the detached worker.
- `clip` never executes the snippet; it only copies the expanded text.

## Key Files

- `src/commands/clip_cmd.rs` — the command: args, `copy_to_clipboard`,
  `run_exact`, `run`.
- `src/clipboard.rs` — the platform primitives, the timeout wrapper, the
  auto-clear generation counter.
- `src/commands/mod.rs` — `run_snippet_selection`, `expand_snippet_command`,
  `run_explicit_sync`.
- `src/ui/mod.rs` — the visual-range multi-select direct call site.
- `src/commands/cron_cmd.rs` — the second direct call site.
- `src/logging.rs` — `audit_log`.
- `src/main.rs` — dispatch (`src/main.rs:573-611`).

## File / line references

- `ClipArgs`: `src/commands/clip_cmd.rs:9`
- `copy_to_clipboard`: `:40`; `process_snippet`: `:51`
- `run_exact`: `:68`; `run`: `:99`
- Clipboard primitives: `src/clipboard.rs:151-205`
- Dispatch: `src/main.rs:573-611`

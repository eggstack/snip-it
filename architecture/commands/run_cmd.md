# run_cmd — TUI Selection + Shell Execution

[← Back to Overview](../overview.md)

## Overview

`src/commands/run_cmd.rs` (469 lines) is the primary execution path:
TUI fuzzy selection (or exact bypass) → variable expansion → shell
spawn with timeout supervision → audit/usage bookkeeping → optional
explicit sync. `run()` serves the TUI; `run_exact()` serves
`--id/--description-exact/--command-exact` from `main.rs`.

## CLI surface

`RunArgs` (`run_cmd.rs:12`), `snp run` (alias `r`):

| Flag | Meaning |
|------|---------|
| `-f/--filter` | Initial TUI filter (conflicts with exact selectors) |
| `--sync` | Explicit sync after execution (needs Tokio runtime) |
| `-l/--library` | Library scope |
| `--sort <mode>` | `SnippetSort`, default `Relevance` |
| `--favorites-first` | Favorites rank first |
| `--id` | Exact UUID bypass (conflicts with other selectors + filter) |
| `--description-exact` | Exact description bypass |
| `--command-exact` | Exact command bypass |

`main.rs:500-544` routes exact selectors through
`resolve_exact_target` → `run_exact`, else `run`.

## Flow / steps

### `run()` (`run_cmd.rs:342`)

Calls `run_snippet_selection(filter, library, do_sync,
allow_delete=true, sort_opts, runtime, process_snippet)` and maps
`ExecutionFailed{exit_code} → CliOutcome::ExecutionFailed`,
`Cancelled → Success` (run treats cancellation as exit 0),
`Selected → Success`. `main.rs` turns `ExecutionFailed` into
`process::exit(child_code.unwrap_or(8))`.

### `run_exact()` (`run_cmd.rs:371`)

Requires a runtime when `do_sync`. Runs `process_snippet(snippet,
false)` directly, maps `Failed` to `ExecutionFailed`, then a trailing
`run_explicit_sync` (warn-only) if `do_sync`.

### `process_snippet()` (`run_cmd.rs:194`)

1. `expand_snippet_command`: `Cancel → ProcessResult::Cancel`,
   `Skip → Continue`.
2. Copy flag set (TUI `y`) → `clip_cmd::copy_to_clipboard` + trace log,
   `Done("Copied to clipboard")`.
3. `snippet.output` non-empty → hardened output-file branch: CWD
   canonicalization, symlink/traversal checks pre- and post-open,
   `create_new` + Unix `O_NOFOLLOW`, fd-vs-path dev/ino identity check,
   then `spawn_and_wait_execution` with stdout redirected and a 300 s
   default timeout (`DEFAULT_TIMEOUT_SECONDS`, overridable via
   `SNP_COMMAND_TIMEOUT`; `0` disables).
4. Otherwise normal branch: `SHELL`/`COMSPEC` shell, `-c`/`/C` flag,
   no default timeout (env override still applies), unified
   `spawn_and_wait_execution`.
5. `record_execution_result` (`:169`): on success only — `audit_log(
   "execute")` + `UsageIndex::record_use` (best-effort); always
   `log_command_execution` tracing.

`spawn_and_wait_execution` (`:123`) maps success → `Done`, nonzero
status → `Failed{exit_code: status.code()}` (None on signal),
spawn/timeout/wait failure → `Failed{exit_code: None}`.

## Mutation vs read-only

Execution is **not** a library mutation: no transaction gate, no
`save_library`, no pending marker. Side effects are process-level
(child process, output file, clipboard) plus bookkeeping (audit log,
usage index). The TUI delete shortcut inside `run_snippet_selection`
is the only library write on this path.

## Auto-sync trigger

No direct `notify_mutation`. With `--sync`, the post-selection /
post-`run_exact` `run_explicit_sync` performs a foreground sync under
the `SyncExecutionLock` and clears pending. Without `--sync`, usage or
execution leaves no sync intent.

## Error / exit mapping

- Child nonzero → exit code passthrough (`ExecutionFailed`, exit 8
  container or raw child code via `main.rs`).
- Spawn failure / timeout / signal death → `Failed{None}` → exit 8.
- Output-path escape / parent race / timeout → `SnipError::runtime_error`
  (exit 1); timeout message reports the second count.
- Editor/variable cancellation → exit 0.

## Key invariants

- Never sanitize snippet commands (by design).
- Output files are `create_new`-only; symlink races fail closed at
  three checkpoints (pre-open, post-open, fd identity).
- Usage/audit record only on `Done`, never on failure/cancel.
- `get_shell`/`shell_arg_flag` are platform-split (Unix `-c`,
  Windows `/C` + `COMSPEC`).
- Reaping after `kill` is bounded (5 s grace) so uninterruptible
  children cannot hang the CLI forever.

## File / line references

- `RunArgs`: `src/commands/run_cmd.rs:12`; timeouts: `:39-101`
- `spawn_and_wait_execution`: `:123`; `record_execution_result`: `:169`
- `process_snippet`: `:194`; `run`: `:342`; `run_exact`: `:371`
- Dispatch: `src/main.rs:500-544`

# run_cmd — TUI Selection + Shell Execution

[← Back to Overview](../overview.md)

## Overview

`src/commands/run_cmd.rs` (483 lines) is the primary execution path:
TUI fuzzy selection (or exact bypass) → variable expansion → shell
spawn with timeout supervision → audit/usage bookkeeping → optional
explicit sync. `run()` serves the TUI; `run_exact()` serves
`--id/--description-exact/--command-exact` from `src/main.rs`.

This is the only command that executes a snippet body. Commands are
**never sanitized** — that is a deliberate design decision, not an
oversight (see *Key invariants*).

## CLI surface

`RunArgs` (`run_cmd.rs:12`), `snp run` (alias `r`):

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `-f/--filter` | `Option<String>` | – | Initial TUI filter (conflicts with the exact selectors) |
| `--sync` | bool | `false` | Explicit sync after execution (needs the Tokio runtime) |
| `-l/--library` | `Option<String>` | – | Library scope |
| `--sort` | `SnippetSort` | `Relevance` | Ordering |
| `--favorites-first` | bool | `false` | Favorites rank first |
| `--id` | `Option<String>` | – | Exact snippet ID bypass (conflicts with the other selectors + filter) |
| `--description-exact` | `Option<String>` | – | Exact description bypass |
| `--command-exact` | `Option<String>` | – | Exact command bypass |

`src/main.rs:527-572` routes exact selectors through
`resolve_exact_target` → `run_exact`, else `run`.

## Flow / steps

### `run()` (`run_cmd.rs:342`)

Calls `run_snippet_selection(filter, library, do_sync,
allow_delete=true, sort_opts, runtime, process_snippet)` and maps
`ExecutionFailed{exit_code} → CliOutcome::ExecutionFailed`,
`Cancelled → Success` (run treats cancellation as exit 0,
`run_cmd.rs:365`), `Selected → Success` (`run_cmd.rs:358-367`).
`src/main.rs:568-569` turns `ExecutionFailed` into
`process::exit(child_code.unwrap_or(8))`.

### `run_exact()` (`run_cmd.rs:371`)

Requires a runtime when `do_sync` (`run_cmd.rs:376-385`; error
`sync requested but no runtime`). Runs `process_snippet(snippet, false)`
directly, maps `Failed` to `ExecutionFailed` (`run_cmd.rs:387-391`), then
a trailing `run_explicit_sync` (warn-only) if `do_sync`
(`run_cmd.rs:392-397`).

### `process_snippet()` (`run_cmd.rs:194`)

1. `expand_snippet_command` (`run_cmd.rs:195`): `Cancel → ProcessResult::Cancel`,
   `Skip → Continue`.
2. Copy flag set (TUI `y`) → `clip_cmd::copy_to_clipboard` + trace log,
   `Done("Copied to clipboard")` (`run_cmd.rs:201-207`). Note the copy
   branch does **not** execute the command.
3. `snippet.output` non-empty → hardened output-file branch
   (`run_cmd.rs:208-327`): CWD canonicalization, symlink/traversal checks
   pre- and post-open, `create_new` + Unix `O_NOFOLLOW`
   (`run_cmd.rs:261-273`), fd-vs-path dev/ino identity check
   (`run_cmd.rs:299-316`), then `spawn_and_wait_execution` with stdout
   redirected and a 300 s default timeout (`DEFAULT_TIMEOUT_SECONDS`,
   `run_cmd.rs:36`, overridable via `SNP_COMMAND_TIMEOUT`; `0` disables).
4. Otherwise normal branch (`run_cmd.rs:328-338`): `SHELL`/`COMSPEC`
   shell, `-c`/`/C` flag, no default timeout (env override still applies),
   unified `spawn_and_wait_execution`.
5. `record_execution_result` (`run_cmd.rs:169`): on success only —
   `audit_log("execute")` + `UsageIndex::record_use` (best-effort);
   always `log_command_execution` tracing (`run_cmd.rs:186-191`).

### Shell selection and timeouts

```rust
fn shell_arg_flag() -> &'static str {
    if cfg!(windows) { "/C" } else { "-c" }
}

fn get_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}
```

(`run_cmd.rs:103-113`.) On Unix the shell is `$SHELL` when set and
`/bin/sh` otherwise. The unit test at `run_cmd.rs:405-419` accepts
**either** — `shell.contains("/bin/sh") || std::env::var("SHELL").is_ok()`
— so a machine with `$SHELL=/bin/zsh` is a passing configuration. Do not
"harden" this into a fixed-shell assertion; the fallback is intentional.

`get_timeout` (`run_cmd.rs:44`) reads `SNP_COMMAND_TIMEOUT` first
(seconds; `0` disables the timeout entirely, `run_cmd.rs:49`), and
otherwise applies the branch policy: `TimeoutPolicy::Default` for the
output-file branch, `TimeoutPolicy::NoDefault` for the normal branch.

`wait_for_command` (`run_cmd.rs:58`) polls `try_wait()` every 50 ms. On
timeout it kills the child and then bounds the reap with a 5 s grace
period (`run_cmd.rs:83-90`) so an uninterruptible child cannot hang the
CLI forever.

`spawn_and_wait_execution` (`run_cmd.rs:123`) maps success → `Done`,
nonzero status → `Failed{exit_code: status.code()}` (None on signal),
spawn/timeout/wait failure → `Failed{exit_code: None}`.

## Audit log and usage bookkeeping

`record_execution_result` (`run_cmd.rs:169-192`) is the single
bookkeeping path shared by both execution branches:

- On success only: `logging::audit_log("execute", snippet, None)`
  (`run_cmd.rs:176`; write failure is debug-logged, never fatal) and
  `UsageIndex::record_use` + `save` (save failure is debug-logged,
  `run_cmd.rs:179-183`).
- Always: `log_command_execution(final_command, &[], &result, working_dir)`
  (`run_cmd.rs:191`) — a tracing event, not the audit file.

The audit file is `<config>/audit.log`
(`src/logging.rs:381-385`); entries carry a timestamp, action, snippet id,
description, library id, and device id (`src/logging.rs:34-41`).

## Mutation vs read-only

Execution is **not** a library mutation: no transaction gate, no
`save_library`, no pending marker. Side effects are process-level
(child process, output file, clipboard) plus bookkeeping (audit log,
usage index). The TUI delete shortcut inside `run_snippet_selection`
is the only library write on this path.

`run` never calls `gate_mutation_on_interrupted_transactions()` itself.

## Auto-sync trigger

No direct `notify_mutation`. With `--sync`, the post-selection /
post-`run_exact` `run_explicit_sync` (`src/commands/mod.rs:343`)
performs a foreground sync under the `SyncExecutionLock` and clears
pending. Without `--sync`, usage or execution leaves no sync intent.

`src/main.rs:990` classifies `Commands::Run(_)` as a mutation command
(`StartupRecoveryPolicy::Allow` + `StartupServices::Logging`).

## Tokio runtime

`run` is one of the commands that may initialize the global runtime —
but only for `--sync`. `src/main.rs:566` passes
`args.sync.then_some(&RUNTIME)`, so the runtime is `Some` only when
`--sync` is set. `run_snippet_selection` (`src/commands/mod.rs:456-465`)
and `run_exact` (`run_cmd.rs:376-385`) both error with
`runtime_error("sync requested but no runtime")` if `do_sync` is true and
the runtime is `None`.

## Error / exit mapping

- Child nonzero → exit code passthrough (`ExecutionFailed`, exit 8
  container or raw child code via `src/main.rs:544` / `src/main.rs:568`).
- Spawn failure / timeout / signal death → `Failed{None}` → exit 8.
- Output-path escape / parent race / timeout → `SnipError::runtime_error`
  (exit 1); timeout message reports the second count
  (`run_cmd.rs:91-97`).
- Editor/variable cancellation → exit 0 (`run_cmd.rs:365`).

## Key invariants

- Never sanitize snippet commands (by design).
- Output files are `create_new`-only; symlink races fail closed at
  three checkpoints (pre-open, post-open, fd identity).
- Usage/audit record only on `Done`, never on failure/cancel.
- `get_shell`/`shell_arg_flag` are platform-split (Unix `-c`,
  Windows `/C` + `COMSPEC`), and the `/bin/sh` fallback is a supported
  configuration, not a bug.
- Reaping after `kill` is bounded (5 s grace) so uninterruptible
  children cannot hang the CLI forever.
- The copy branch (`y` in the TUI) copies and returns — it never also
  executes.

## Gotchas

- `SNP_COMMAND_TIMEOUT=0` disables the timeout entirely, including the
  300 s output-file default (`run_cmd.rs:49`).
- The output-file branch redirects **stdout only**; stderr still reaches
  the terminal.
- A snippet with a non-empty `output` field takes the output-file branch
  instead of the plain-execution branch, so the command's output is
  written to a file rather than the terminal.
- `run_exact` executes the snippet even when it lives in any library
  matched by `--library all` (the exact resolver supports the `all`
  scope via `LibraryScope::from_filter_arg`, `src/selector.rs:65`).
- The `8d5c209` change to this file was **test-only**: it added an
  isolated config directory helper (`run_cmd.rs:435-441`) so
  `run_exact` tests never touch the developer's real `~/.config/snp/`.
  The runtime behavior is unchanged.

## Key Files

- `src/commands/run_cmd.rs` — the command: args, shell selection, the
  output-file hardening, execution supervision, and bookkeeping.
- `src/commands/mod.rs` — `run_snippet_selection`, `expand_snippet_command`,
  `run_explicit_sync`.
- `src/commands/clip_cmd.rs` — `copy_to_clipboard`, used by the TUI copy
  branch.
- `src/logging.rs` — `audit_log` and `log_command_execution`.
- `src/usage.rs` — `UsageIndex` (use counts, last-used timestamps).
- `src/outcome.rs` — `CliOutcome::ExecutionFailed` and the exit codes.
- `src/main.rs` — dispatch (`src/main.rs:527-572`).

## File / line references

- `RunArgs`: `src/commands/run_cmd.rs:12`; timeouts: `:36-101`
- `shell_arg_flag`: `:103`; `get_shell`: `:107`
- `spawn_and_wait_execution`: `:123`; `record_execution_result`: `:169`
- `process_snippet`: `:194`; `run`: `:342`; `run_exact`: `:371`
- Dispatch: `src/main.rs:527-572`

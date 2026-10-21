# CLI Entry & Command Dispatch

[← Back to Overview](overview.md)

## Overview

`src/main.rs` is the `snp` binary entry point: the clap schema (`Cli` +
`Commands`), Unix signal setup, the global Tokio runtime handle, the
startup-behavior classification, and a `dispatch_command()` fan-out to
`src/commands/*`. It owns no business logic — each subcommand maps to a
command module, and each command module owns its canonical `*Args` struct
beside its handler. The `snp data` subgroup is a mirror group over the same
arg types and the same single-path `handle_*` helpers, never a second
schema (it is a separate Clap subcommand group, not a Clap `alias`).

This doc covers the command surface, dispatch, and startup behavior only.
Command implementations live in [commands/mod.md](commands/mod.md); the
resolver behind `--id` / `--description-exact` / `--command-exact` is in
[selector.md](selector.md); exit-code mapping is in [outcome.md](outcome.md);
the auto-sync worker and pending state are in [auto_sync.md](auto_sync.md);
the library scope model is in [library.md](library.md). For the working rules
— which exit code a given failure maps to, how to classify a new subcommand's
startup behavior, and the stdout/stderr split — see
`.skills/cli-and-outcome-contract.md`.

## Startup sequence

`main()` (`src/main.rs:1000-1038`) runs, in order:

1. `setup_panic_handler()` — installed before parsing so TUI terminal cleanup
   is protected even on argument errors (`src/main.rs:1001`).
2. `setup_signal_handler()` (`src/main.rs:1002`; Unix impl `src/main.rs:29-45`) —
   registers SIGINT + SIGTERM via `signal_hook::flag` on Unix
   (`src/main.rs:37`, `src/main.rs:41`); on Windows the handler is an empty
   stub (`src/main.rs:47-50`) because Ctrl+C is handled by the crossterm event
   loop.
3. `Cli::parse()` + `command_behavior()` (`src/main.rs:1004-1005`) — one match
   assigns both the `StartupRecoveryPolicy` and the `StartupServices` level,
   preventing drift between the two.
4. Conditional logging (`src/main.rs:1006-1012`) — `Minimal` commands skip
   `init_default_file_logging()` and `log_startup_info()`; `Logging` commands
   call both.
5. Startup recovery (`src/main.rs:1014-1016`) — only when the policy is
   `Allow` (see [Startup recovery](#startup-recovery)).
6. `dispatch_command()` (`src/main.rs:1018`); the outcome maps to a process
   exit code via `CliOutcome::exit_code()` (`src/main.rs:1019-1032`).

### Signal handling (Unix)

```rust
#[cfg(unix)]
fn setup_signal_handler() {
    use signal_hook::flag;
    use snip_it::ui;

    let terminate = ui::get_terminate();

    if let Err(e) = flag::register(signal_hook::consts::signal::SIGINT, terminate.clone()) {
        eprintln!("Failed to set Ctrl+C handler: {e}");
        std::process::exit(1);
    }
    if let Err(e) = flag::register(signal_hook::consts::signal::SIGTERM, terminate) {
        eprintln!("Failed to set SIGTERM handler: {e}");
        std::process::exit(1);
    }
}
```

- Both signals set the shared `ui::get_terminate()` flag (`src/ui/mod.rs:62-64`)
  instead of terminating the process, so the TUI event loop can unwind and
  restore the terminal.
- A registration failure is fatal (`src/main.rs:39`, `src/main.rs:43`) — a
  half-installed handler is worse than none.
- Windows has no equivalent registration: `setup_signal_handler()` is an empty
  `#[cfg(windows)]` stub (`src/main.rs:47-50`).

### Global Tokio runtime

`RUNTIME: LazyLock<tokio::runtime::Runtime>` (`src/main.rs:22-27`) is
constructed lazily, so the runtime only initializes when an async path touches
it:

| Call site | Runtime handle |
|---|---|
| `update` (`src/main.rs:452-458`) | `RUNTIME.block_on(…)` |
| `sync` run (`src/main.rs:696`), `sync retry` (`src/main.rs:711`) | `&RUNTIME` |
| `register` (`src/main.rs:728`) | `&RUNTIME` |
| `premade list/get/sync/search/update` (`src/main.rs:739-752`) | `&RUNTIME` |
| `run` — exact (`src/main.rs:515`), TUI (`src/main.rs:540`) | `args.sync.then_some(&RUNTIME)` |
| `clip` — exact (`src/main.rs:561`), TUI (`src/main.rs:582`) | `args.sync.then_some(&RUNTIME)` |
| `search` (`src/main.rs:597`) | `args.sync.then_some(&RUNTIME)` |

`run_snippet_selection()` takes `Option<&Runtime>`
(`src/commands/mod.rs:338-352`) and errors when `do_sync` is true but the
runtime is `None` (`src/commands/mod.rs:353-362`). Local-only commands never
initialize the runtime. The detached worker builds its own
`new_current_thread()` runtime (`src/auto_sync/worker.rs:426-429`) — it never
touches the global `RUNTIME`.

## Command surface

`Cli` (`src/main.rs:52-62`) has exactly one field — `command: Option<Commands>`
(`src/main.rs:60-61`). There are no global flags. The `after_help` text
documents exit codes 0–10 (`src/main.rs:57`).

`Commands` (`src/main.rs:64-184`) declares 29 variants, including two hidden
ones.

### Top-level commands

| Command | Alias | Module | Async | Description |
|---------|-------|--------|-------|-------------|
| `version` | `v` | — | No | Print version (`src/main.rs:68`) |
| `update` | — | `src/update.rs` | Yes | Check for and install an update; `--dry-run`, `--locked` (`src/main.rs:70-75`) |
| `new` | `n` | `new_cmd` | No | Create snippet from positional, prompt, multiline, exact stdin, file, or editor (`src/main.rs:86`) |
| `list` | `l` | `list_cmd` | No | List snippets (`src/main.rs:89`); `--json` / `--csv` are mutually exclusive (`src/commands/list_cmd.rs:18-21`) |
| `run` | `r` | `run_cmd` | If `--sync` | TUI select → execute via shell; exact selectors bypass the TUI (`src/main.rs:92`) |
| `clip` | `c` | `clip_cmd` | If `--sync` | TUI select → copy to clipboard; exact selectors bypass the TUI (`src/main.rs:95`) |
| `search` | `s` | `search_cmd` | If `--sync` | TUI select → display snippet info (`src/main.rs:98`) |
| `select` | `sel` | `select_cmd` | No | Print a snippet's command to stdout (no execution) (`src/main.rs:101`) |
| `edit` | `e` | `edit_cmd` | No | Open library in `$EDITOR`; or set/clear output (`src/main.rs:104`) |
| `keybindings` | `k` | `keybindings_cmd` | No | Print keybinding reference (`src/main.rs:107`) |
| `sync` | `y` | `sync_cmd` | Yes | Bidirectional sync group (`src/main.rs:110-113`) |
| `cron` | `cr` | `cron_cmd` | No | Generate crontab entry for periodic sync (`src/main.rs:116`) |
| `register` | `reg` | `register_cmd` | Yes | Register a new sync account (`src/main.rs:119`) |
| `library` | `lib` | `library_cmd` | No | Library CRUD group (`src/main.rs:122-125`) |
| `premade` | `p` | `premade_cmd` | Yes | Browse/download premade libraries (`src/main.rs:128-131`) |
| `doctor` | — | `doctor_cmd` | No | Diagnose configuration and environment — **flat command**, not a group (`src/main.rs:133`) |
| `import` | `i` | `import_cmd` | No | Import Pet snippet files (`src/main.rs:136-139`) |
| `repair` | `rp` | `repair_cmd` | No | Conservative, backed-up, idempotent repair (`src/main.rs:142`) |
| `validate` | `val` | `validate_cmd` | No | Validate snippet libraries and configuration (read-only) (`src/main.rs:145`) |
| `data` | `d` | `DataCommands` | No | Mirror group over the canonical data arg types (`src/main.rs:148-151`) |
| `completions` | `g` | inline (`clap_complete`) | No | Generate shell completions (`src/main.rs:154-158`) |
| `shell` | — | `shell_cmd` | No | Generate interactive shell integration (`src/main.rs:160-163`) |
| `backup` | — | `backup_cmd` | No | Secret-free backup snapshot (`src/main.rs:165`) |
| `restore` | — | `restore_cmd` | No | Restore from a backup snapshot (`src/main.rs:167`) |
| `status` | — | `status_cmd` | No | Show auto-sync status (read-only) (`src/main.rs:169`) |
| `mcp` | — | `mcp` | No | Local MCP adapter — read-only, stdio-only (`src/main.rs:171-174`) |
| `get` | — | `get_cmd` | No | Deterministic non-TUI retrieval (never executes, no clipboard) (`src/main.rs:176`) |

### Hidden commands

| Command | Declaration | Purpose |
|---|---|---|
| `auto-sync-worker` | `#[command(name = "auto-sync-worker", hide = true)]` (`src/main.rs:177-183`) | Detached debounce worker; invoked by the parent process after a mutation. Takes `--state-dir`. Exits 1 on `WorkerOutcome::Failed` (`src/main.rs:858-868`) |
| `__self-replace` | `#[command(name = "__self-replace", hide = true)]` (`src/main.rs:76-83`) | Windows self-replacement helper; takes `--candidate` / `--destination` (`src/main.rs:459-465`) |

### Nested subcommand groups

**`library`** — `LibraryCommands` (`src/main.rs:186-207`):

| Subcommand | Alias | Args |
|---|---|---|
| `list` | `l` | — (`src/main.rs:189-190`) |
| `create` | `c` | `name` (`src/main.rs:192-193`) |
| `delete` | `d` | `name`, `--force` (`src/main.rs:195-200`) |
| `set-primary` | `p` | `name` (`src/main.rs:202-203`) |
| `show` | `s` | `name` (optional) (`src/main.rs:205-206`) |

**`premade`** — `PremadeCommands` (`src/main.rs:209-225`):

| Subcommand | Alias | Args |
|---|---|---|
| `list` | `l` | — (`src/main.rs:212-213`) |
| `get` | — | `name` (optional; the literal `all` selects every premade library) (`src/main.rs:215`, `src/main.rs:741-742`) |
| `sync` | `s` | — (`src/main.rs:217-218`) |
| `search` | `se` | `query` (`src/main.rs:220-221`) |
| `update` | `u` | `name` (`src/main.rs:223-224`) |

**`sync`** — `SyncCommands` (`src/main.rs:271-335`):

| Subcommand | Alias | Args |
|---|---|---|
| `run` | `s` | `--library`, `--servers`, `--push-only` / `--pull-only` (mutually exclusive), `--dry-run` (`src/main.rs:275-288`) |
| `config` | `c` | `--show`, `--auto-sync`, `--debounce`, `--max-delay`, `--failure`, `--timeout` (`src/main.rs:291-310`) |
| `retry` | `r` | `--library` (`src/main.rs:313-316`) |
| `clear-failure` | `f` | — (`src/main.rs:318-319`) |
| `discard-pending` | `d` | `--force`, `--generation` (`src/main.rs:321-327`) |
| `repair` | — | `--dry-run`, `--apply` (`src/main.rs:329-334`) |

`sync` with **no** subcommand is equivalent to `sync run` with every flag off
(`src/main.rs:677-687`).

**`import`** — `ImportSubcommands` (`src/main.rs:337-367`):

| Subcommand | Alias | Args |
|---|---|---|
| `pet` | `p` | `path`, `--library`, `--merge` / `--replace` (mutually exclusive), `--dry-run`, `--strict`, `--report`, `--report-file` (`src/main.rs:341-366`) |

**`shell`** — `ShellCommands` (`src/main.rs:227-236`):

| Subcommand | Alias | Args |
|---|---|---|
| `init` | `i` | `shell` (`src/main.rs:231-235`) |

**`mcp`** — `McpCommands` (`src/main.rs:238-252`):

| Subcommand | Alias | Args |
|---|---|---|
| `serve` | — | — (`src/main.rs:241`) |
| `instructions` | — | `client` (`src/main.rs:243-246`) |
| `install` | — | `client` (`src/main.rs:248-251`) |

**`data`** — `DataCommands` (`src/main.rs:254-269`) — a mirror group over the
top-level commands. It reuses the *same* arg types as the top-level commands
(it is a separate Clap subcommand group, **not** a Clap `alias` attribute):

| Subcommand | Alias | Arg type |
|---|---|---|
| `validate` | `v` | `commands::validate_cmd::ValidateArgs` (`src/main.rs:258`) |
| `backup` | `b` | `commands::backup_cmd::BackupArgs` (`src/main.rs:261`) |
| `restore` | — | `commands::restore_cmd::RestoreArgs` (`src/main.rs:263`) |
| `repair` | — | `commands::repair_cmd::RepairArgs` (`src/main.rs:265`) |
| `status` | `s` | `commands::status_cmd::StatusArgs` (`src/main.rs:268`) |

Bare `snp` (no subcommand) defaults to the `run` TUI
(`src/main.rs:446-448`).

## Dispatch

`dispatch_command()` (`src/main.rs:444-871`) maps every `Commands` variant to
its module 1:1 — except `data`, whose five subcommand arms each route to the
`handle_*` helper of the matching top-level command. Each module exposes
`run()` (except `premade_cmd` / `library_cmd`, which use subcommand-dispatched
`run_list` / `run_get` / …).

Routing facts worth auditing:

- **Top-level and `data` spellings share one code path.** `handle_validate`
  (`src/main.rs:406`), `handle_backup` (`src/main.rs:411`), `handle_restore`
  (`src/main.rs:423`), `handle_repair` (`src/main.rs:432`), and `handle_status`
  (`src/main.rs:439`) are the only implementations; `Commands::Validate`
  (`src/main.rs:808-810`), `Commands::Backup` (`src/main.rs:811-813`),
  `Commands::Restore` (`src/main.rs:814-816`), `Commands::Repair`
  (`src/main.rs:805-807`), `Commands::Status` (`src/main.rs:817-819`) and all
  five `DataCommands` arms (`src/main.rs:825-841`) call them.
- **Commands that own their outcome return it directly** instead of falling
  through to the `Ok(CliOutcome::Success)` tail at `src/main.rs:870`:
  `select` (`src/main.rs:606-613`), `doctor` (`src/main.rs:757-769`),
  `sync discard-pending` (`src/main.rs:716-719`), `get`
  (`src/main.rs:842-857`).
- **Ambiguous exact matches** are reported to stderr by `report_ambiguous()`
  (`src/main.rs:390-400`) and return `CliOutcome::Ambiguous` (exit 5).
- **`auto-sync-worker`** is dispatched to `snip_it::auto_sync::worker::run`
  (`src/main.rs:858-868`) and exits 1 on failure.

### Canonical `*Args` structs

One canonical Clap `*Args` struct lives beside each handler — `ValidateArgs`,
`BackupArgs`, `RestoreArgs`, `RepairArgs`, `StatusArgs`, plus `NewArgs`,
`ListArgs`, `RunArgs`, `ClipArgs`, `SearchArgs`, `SelectArgs`, `EditArgs`,
`GetArgs`, `DoctorArgs`, `CronArgs`, `RegisterArgs`. `src/main.rs` keeps only
the `Commands` composition, runtime/signal/log setup, dispatch, and outcome
mapping. Shared shell spellings live in `shell_cmd::ShellIntegration`.

### Exact selectors

`run`, `clip`, and `edit` accept `--id` / `--description-exact` /
`--command-exact`, which bypass the TUI via
`selector::resolve_exact_target()` (`src/selector.rs:604-612`;
call sites `src/main.rs:504`, `src/main.rs:550`, `src/main.rs:640`):

- `--id <ID>` — exact snippet ID (case-sensitive)
- `--description-exact <text>` — exact description (case-insensitive)
- `--command-exact <text>` — exact command text (case-insensitive)

They conflict with `--filter` (fuzzy TUI pre-filter) and with each other.
Ambiguous description/command matches report identities (exit 5); no match
is exit 3. `snp edit --output/--output-stdin/--clear-output` requires an
exact selector or `--filter` (`src/main.rs:662-667`). See
[selector.md](selector.md).

## Startup-behavior classification

`command_behavior()` (`src/main.rs:887-998`) maps every `Commands` variant to
a `CommandBehavior { recovery, services }` (`src/main.rs:882-885`). The struct
and the match are one unit: **one** match assigns both the startup-recovery
policy and the logging/audit service level, so the two can never drift.

`StartupRecoveryPolicy` (`src/auto_sync/notification.rs:190-201`) has exactly
5 variants:

```rust
pub enum StartupRecoveryPolicy {
    Allow,                // Mutation commands — recovery permitted
    SuppressReadOnly,     // Read-only — no worker spawn, no network
    SuppressExplicitSync, // sync, cron, register — manage own behavior
    SuppressInternal,     // auto-sync-worker
    SuppressConfiguration,// update, self-replace, doctor, keybindings, shell, completions
}
```

`StartupServices` (`src/main.rs:873-877`) has two levels: `Minimal` skips file
logging, startup/shutdown log lines, and audit entirely; `Logging`
initializes file logging plus `log_startup_info()` / `log_shutdown_info()`.

### Classification table

| Policy | Services | Commands | Source |
|---|---|---|---|
| `SuppressReadOnly` | `Minimal` | `version`, `list`, `select`, `status`, `mcp`, `get`, `validate`, `backup`, `library list`, `library show` | `src/main.rs:890-905` |
| `SuppressReadOnly` | `Minimal` | `restore --dry-run`, `repair --dry-run`, `import pet --dry-run` | `src/main.rs:907-925` |
| `SuppressReadOnly` | `Minimal` | `data validate`, `data status`, `data backup`, `data restore --dry-run`, `data repair --dry-run` | `src/main.rs:929-944` |
| `Allow` | `Logging` | `data repair`, `data restore` (non-dry-run) | `src/main.rs:945-947` |
| `Allow` | `Logging` | `new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, `library create`, `library delete`, `library set-primary` | `src/main.rs:951-967` |
| `SuppressExplicitSync` | `Logging` | `sync`, `cron`, `register` | `src/main.rs:970-973` |
| `SuppressInternal` | `Logging` | `auto-sync-worker` | `src/main.rs:976-979` |
| `SuppressConfiguration` | `Logging` | `update`, `__self-replace`, `doctor`, `completions`, `shell`, `keybindings` | `src/main.rs:982-992` |
| `Allow` | `Logging` | bare `snp` (default TUI) | `src/main.rs:995` |

The match is exhaustive — adding a command requires choosing a policy and a
`StartupServices` level, enforced by the compiler with no catch-all arm.

Notes for auditors:

- **`search` is classified as a mutation command**, not read-only: it is in the
  `Allow` / `Logging` arm (`src/main.rs:955`). A regression test pins this
  (`search_retains_mutation_capabilities`, `src/main.rs:1078-1089`).
- **Dry-run modes of mutating commands are classified read-only** so previews
  never trigger network work or create log/config state
  (`src/main.rs:907-925`).
- **`library` is split by subcommand**: `list`/`show` are read-only
  (`src/main.rs:899-901`); `create`/`delete`/`set-primary` are mutations
  (`src/main.rs:961-966`).
- **`data` is split by subcommand** the same way (`src/main.rs:928-948`).

### Startup recovery

`main()` gates recovery on the policy
(`src/main.rs:1014-1016`):

```rust
if snip_it::auto_sync::should_attempt_auto_sync_recovery_for_policy(Some(behavior.recovery)) {
    snip_it::auto_sync::notification::startup_recover_pending();
}
```

`should_attempt_auto_sync_recovery_for_policy()`
(`src/auto_sync/notification.rs:213-223`) returns `true` only for
`StartupRecoveryPolicy::Allow` (or `None`, i.e. "no classification supplied").
`startup_recover_pending()` (`src/auto_sync/notification.rs:225-236`) reads the
pending marker and schedules the detached worker; a scheduling failure is a
**warning**, not an error — the pending marker is preserved for a later retry.

## Library scope (`--library all`)

`all` is the cross-library scope keyword, defined in exactly one place:
`LibraryScope::from_filter_arg()` (`src/selector.rs:65-71`), which maps
`None` → `Primary`, `Some("all")` → `AllLibraries`, anything else → `Named`.

| Surface | `--library all` | Why |
|---|---|---|
| `snp get`, exact-mode `run`/`clip`/`edit`, MCP | Supported | Selector path (`src/selector.rs:65-71`, `src/selector.rs:623-646`) |
| `snp list` | Supported | Read-only path via `commands::load_readonly_snippets` (`src/commands/mod.rs:127-147`; `src/commands/list_cmd.rs:65-77`) |
| `snp search` | **Rejected** with an explicit error | The interactive selector edits and deletes inside a single library file (`src/commands/search_cmd.rs:32-44`) |
| `snp new`, `snp edit`, `snp sync`, `snp select` | Not supported | Single-library path (`commands::get_library_path`, `src/commands/mod.rs:91`) |

The read-only resolver that backs the supported surfaces is
`LibraryManager::resolve_readonly_sources()`
(`src/library/manager.rs:248-289`). `Some("all")` returns every library in the
registry (`src/library/manager.rs:278`); in legacy single-file mode it returns
the one implicit `snippets` library (`src/library/manager.rs:256`). It never
calls `ensure_library_mode()`, never creates files or directories, and never
rewrites metadata (`src/library/manager.rs:237-247`).

> "Read-only" scopes the *library* state. Constructing the manager first can
> still migrate the legacy macOS config directory
> (`src/library/manager.rs:41-92` → `src/utils/config.rs:166-179`). See
> [public_api.md](public_api.md) for the caveat.

`snp search --library all` used to fail with the misleading
`Library 'all' does not exist`. It now rejects the scope with an accurate
error naming the supported alternatives (`snp list --library all`,
`snp get --library all`) — `src/commands/search_cmd.rs:32-44`.

**`all` is a reserved library name.** `validate_library_name()` rejects it
(`src/library/model.rs:213-223`), so `snp library create all` fails and
`--library all` can never mean "the library literally named all". See
[library.md](library.md).

## Exit codes

`CliOutcome` → code mapping lives in `src/outcome.rs:72-91` (stable; see
`docs/EXIT_CODES.md` and [outcome.md](outcome.md)):

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (`PersistenceFailed` maps here for backward compat) |
| 2 | Usage/argument error (Clap) |
| 3 | Not found |
| 4 | User cancelled |
| 5 | Ambiguous match |
| 6 | Validation failure |
| 7 | Sync failure |
| 8 | Execution failure |
| 9 | Conflict/refused |
| 10 | Unsafe repairs pending operator decision |

`main()` maps the result (`src/main.rs:1018-1033`):

- `Ok(CliOutcome::Success)` → fall through, exit 0 (`src/main.rs:1019`).
- `Ok(other)` → `std::process::exit(outcome.exit_code())` (`src/main.rs:1024`).
- `Err(SnipError)` → `eprintln!("error: {e}")` then exit 1
  (`src/main.rs:1026-1032`).

### The exit-10 path is *not* a `CliOutcome`

`exit_on_repair_status()` (`src/main.rs:375-388`) maps
`commands::repair_cmd::RepairExitStatus` to a process exit code directly:

```rust
fn exit_on_repair_status(status: commands::repair_cmd::RepairExitStatus) {
    match status {
        commands::repair_cmd::RepairExitStatus::Clean
        | commands::repair_cmd::RepairExitStatus::Repaired
        | commands::repair_cmd::RepairExitStatus::FindingsSafe
        | commands::repair_cmd::RepairExitStatus::DryRun => {}
        commands::repair_cmd::RepairExitStatus::PartialFailure => {
            std::process::exit(snip_it::outcome::exit_code::GENERAL_ERROR);
        }
        commands::repair_cmd::RepairExitStatus::UnsafeOnly => {
            std::process::exit(snip_it::outcome::exit_code::UNSAFE_REPAIRS);
        }
    }
}
```

| `RepairExitStatus` | Exit code |
|---|---|
| `Clean` / `Repaired` / `FindingsSafe` / `DryRun` | 0 (implicit return) |
| `PartialFailure` | 1 (`GENERAL_ERROR`) |
| `UnsafeOnly` | 10 (`UNSAFE_REPAIRS`) |

`FindingsSafe` is the report-only case: issues were found, all of them are
safe to repair, but none were applied because neither `--apply` nor
`--dry-run` was passed (`src/commands/repair_cmd.rs:156-158`,
`:243-252`). It therefore does not claim code 10. `UnsafeOnly` deliberately
does **not** reuse 2 (`USAGE_ERROR`): a valid invocation that found unsafe
repairs awaiting an operator decision is not a usage error, and scripts must
be able to tell the two apart
(`src/main.rs:369-374`). Because this path calls `std::process::exit` inside
the handler, there is deliberately **no** `CliOutcome` variant for it — see
[outcome.md](outcome.md).

`ExecutionFailed { child_code }` propagates the child process exit code when
available, falling back to 8 (`src/outcome.rs:83-87`); the `run` paths call
`std::process::exit(child_code.unwrap_or(8))` directly
(`src/main.rs:519`, `src/main.rs:543`). `snp clip` does not — it exits 1 on
error.

## Key Files

- `src/main.rs` — CLI schema, signals, `RUNTIME`, dispatch, classification, exit mapping
- `src/commands/mod.rs` — shared helpers, TOML load/save, selection loop, `load_readonly_snippets`
- `src/auto_sync/notification.rs:190` — `StartupRecoveryPolicy` (5 variants) and the recovery gate
- `src/outcome.rs:18,46` — `CliOutcome`, `exit_code` constants 0–10
- `src/selector.rs:48,65` — `LibraryScope` and the single `"all"` definition
- `src/library/model.rs:213` — `validate_library_name`, including the reserved `all`
- `src/commands/list_cmd.rs:65` — `snp list` read-only resolution (supports `--library all`)
- `src/commands/search_cmd.rs:32` — `snp search` rejection of `--library all`
- `src/update.rs` — `snp update` transport (in-process eggfetch-core)

## Invariants

- **`command_behavior()` is exhaustive.** Adding a `Commands` variant without
  choosing a policy and a service level is a compile error, not a silent
  default (`src/main.rs:887-998`).
- **Read-only commands never create state.** `Minimal` services skip the log
  directory, `snp.log`, `audit.log`, and the config directory entirely
  (`src/main.rs:1006-1012`).
- **Only `Allow` triggers startup recovery** — read-only, explicit-sync,
  internal, and configuration policies all suppress it
  (`src/auto_sync/notification.rs:213-223`).
- **`sync` with no subcommand ≡ `sync run`** with no flags
  (`src/main.rs:677-687`).
- **`snp data X` and `snp X` are the same code path** — one arg type, one
  handler, one exit-code mapping (`src/main.rs:406-442`, `src/main.rs:825-841`).
- **`all` has one definition** (`LibraryScope::from_filter_arg`) and one
  reserved-name guard (`validate_library_name`); no surface may fork the rule.

## Gotchas

- **The `Commands` enum overflows the default 1 MB thread stack on Windows
  MSVC.** `.cargo/config.toml` sets `/STACK:8388608` for that reason — do not
  remove it.
- **`search` is not read-only.** It is classified `Allow` / `Logging`
  (`src/main.rs:955`), which is intentional and pinned by
  `search_retains_mutation_capabilities` (`src/main.rs:1078-1089`).
- **`doctor` is a flat command, not a nested group** (`src/main.rs:133`), even
  though it takes a `DoctorArgs` struct with several modes.
- **`data repair` and `data restore` are mutations**, unlike their
  read-only siblings (`src/main.rs:945-947`).
- **Exit code 10 never comes back through `CliOutcome`.** `exit_on_repair_status`
  calls `std::process::exit` directly, so `handle_repair` never returns
  (`src/main.rs:432-436`).
- **A failed `run` records no usage metadata** — usage is written only on
  `ProcessResult::is_done()` (`src/commands/run_cmd.rs:175-184`).
- **`--library all` is rejected by `snp search`**, not silently ignored; the
  error names the supported alternatives.

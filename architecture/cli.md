# CLI Entry & Command Dispatch

[← Back to Overview](overview.md)

## Overview

`src/main.rs` is the `snp` binary entry point: clap schema (`Cli` +
`Commands`), Unix signal setup, startup-service classification, and a 1:1
`dispatch_command()` fan-out to `src/commands/*`. It owns no business logic —
each subcommand maps to exactly one command module, and each command module
owns its canonical `*Args` struct beside its handler. The `snp data`
subgroup is a compatibility alias layer over the same arg types and the same
single-path `handle_*` helpers, never a second schema.

## Startup Sequence

`main()` (`src/main.rs:999`) runs, in order:

1. `setup_panic_handler()` — installed before parsing so TUI terminal
   cleanup is protected even on argument errors.
2. `setup_signal_handler()` (`src/main.rs:29`) — registers SIGINT + SIGTERM
   via `signal_hook::flag` on Unix (`src/main.rs:37-44`); on Windows Ctrl+C
   is handled by the crossterm event loop (`src/main.rs:47-50`).
3. `Cli::parse()` + `command_behavior()` — one match assigns both the
   `StartupRecoveryPolicy` and the `StartupServices` level, preventing drift.
4. Conditional logging: `Minimal` commands skip config/log directory,
   `.self_check`, `snp.log`, and `audit.log` creation; `Logging` commands
   call `init_default_file_logging()` + `log_startup_info()`.
5. `startup_recover_pending()` — only when the policy is `Allow`.
6. `dispatch_command()` (`src/main.rs:443`); the outcome maps to a process
   exit code via `CliOutcome::exit_code()`.

### Global State

- `RUNTIME: LazyLock<tokio::runtime::Runtime>` (`src/main.rs:22`) — the
  Tokio runtime is constructed lazily and therefore only initialized when an
  async path touches it: `update` (`block_on`, `src/main.rs:452`), `sync` /
  `register` / `premade` (always passed `&RUNTIME`), and `run` / `clip` /
  `search` only when `--sync` is set (`args.sync.then_some(&RUNTIME)`,
  `src/main.rs:515,560,596`). Local-only commands never init it.
- No subcommand takes `&mut self` runtime plumbing beyond that: sync paths
  receive `&RUNTIME` (or `Option<&Runtime>` for `run_snippet_selection`).

## Subcommands

`Commands` (`src/main.rs:64`) plus nested groups (`SyncCommands`,
`LibraryCommands`, `PremadeCommands`, `McpCommands`, `DataCommands`,
`ShellCommands`, `ImportSubcommands`) give 30+ spellings:

| Command | Alias | Module | Async | Description |
|---------|-------|--------|-------|-------------|
| `version` | `v` | — | No | Print version |
| `new` | `n` | `new_cmd` | No | Create snippet from positional, prompt, multiline, exact stdin, file, or editor |
| `list` | `l` | `list_cmd` | No | List snippets (fuzzy filter over description/command/tags; `--search-output` includes output in match) |
| `run` | `r` | `run_cmd` | If `--sync` | TUI select → execute via shell; exact selectors bypass TUI |
| `clip` | `c` | `clip_cmd` | If `--sync` | TUI select → copy to clipboard; exact selectors bypass TUI |
| `search` | `s` | `search_cmd` | If `--sync` | TUI select → display snippet info |
| `select` | `sel` | `select_cmd` | No | Print a snippet's command to stdout (no execution) |
| `edit` | `e` | `edit_cmd` | No | Open library in `$EDITOR`; or set/clear output (`--output`, `--output-stdin`, `--clear-output` with `--filter` or exact selectors) |
| `get` | — | `get_cmd` | No | Deterministic non-TUI retrieval (never executes, no clipboard) |
| `validate` | `val` | `validate_cmd` | No | Validate snippet libraries and configuration (read-only) |
| `backup` | — | `backup_cmd` | No | Secret-free backup snapshot (directory manifest) |
| `restore` | — | `restore_cmd` | No | Restore from backup (dry-run, merge, replace) |
| `repair` | `rp` | `repair_cmd` | No | Conservative, backed-up, idempotent repair |
| `library` | `lib` | `library_cmd` | No | Library CRUD (`list/create/delete/set-primary/show`) |
| `premade` | `p` | `premade_cmd` | Yes | Browse/download premade libraries (`list/get/sync/search/update`) |
| `import` | `i` | `import_cmd` | No | Import Pet snippet files (`import pet`, create/merge/replace) |
| `doctor` | — | `doctor_cmd` | No | Diagnose configuration and environment |
| `sync` | `y` | `sync_cmd` | Yes | Bidirectional sync (`run/config/retry/clear-failure/discard-pending/repair`) |
| `cron` | `cr` | `cron_cmd` | No | Generate crontab entry for periodic sync |
| `register` | `reg` | `register_cmd` | Yes | Register new sync account |
| `keybindings` | `k` | `keybindings_cmd` | No | Print keybinding reference |
| `status` | — | `status_cmd` | No | Show auto-sync status (read-only) |
| `data` | `d` | `DataCommands` | No | Alias layer reusing canonical `validate`/`backup`/`restore`/`repair`/`status` args (`validate`→`v`, `backup`→`b`, `status`→`s`) |
| `mcp` | — | `mcp` | No | Local MCP adapter (`serve/instructions/install`) — read-only, stdio-only; see [mcp.md](mcp.md) |
| `update` | — | `update` (`src/update.rs`) | Yes | Check for and install an update; see [update.md](update.md) |
| `shell` | — | `shell_cmd` | No | Generate interactive shell integration |
| `completions` | `g` | inline (`clap_complete`) | No | Generate shell completions |
| `auto-sync-worker` | — | `auto_sync::worker` | No | **Hidden** (`hide = true`). Detached debounce worker; see [auto_sync.md](auto_sync.md) |
| `__self-replace` | — | `update` | No | **Hidden** Windows self-replacement helper (`--candidate`/`--destination`) |

Bare `snp` (no subcommand) defaults to the `run` TUI (`src/main.rs:445`).

## Dispatch & Canonical Args

`dispatch_command()` (`src/main.rs:443`) maps every `Commands` variant to
its module 1:1. Each module exposes `run()` (except `premade_cmd` /
`library_cmd`, which use subcommand-dispatched `run_list`/`run_get`/…).
`premade`/`sync`/`register` call sites pass `&RUNTIME`; `run`/`clip`/`search`
pass `args.sync.then_some(&RUNTIME)` so the runtime stays cold otherwise.

One canonical Clap `*Args` struct lives beside each handler — `ValidateArgs`,
`BackupArgs`, `RestoreArgs`, `RepairArgs`, `StatusArgs`, plus `NewArgs`,
`ListArgs`, `RunArgs`, `ClipArgs`, `SearchArgs`, `SelectArgs`, `EditArgs`,
`GetArgs`, `DoctorArgs`, `CronArgs`, `RegisterArgs`. `src/main.rs` keeps only
the `Commands` composition, runtime/signal/log setup, dispatch, and outcome
mapping. Shared shell spellings live in `shell_cmd::ShellIntegration`.

`snp data` reuses the same five arg types through single-path helpers in
`src/main.rs:405-441`: `handle_validate`, `handle_backup`, `handle_restore`,
`handle_repair` (exits 10/1 via `exit_on_repair_status` for
`UnsafeOnly`/`PartialFailure`), `handle_status`. Both spellings therefore
share validation, JSON formatting, and exit-code mapping.

## Startup Recovery Classification

`command_behavior()` (`src/main.rs:886`) maps every `Commands` variant to a
`CommandBehavior { recovery, services }`. `StartupRecoveryPolicy`
(`src/auto_sync/notification.rs:190`) has exactly 5 variants:

```rust
pub enum StartupRecoveryPolicy {
    Allow,                // Mutation commands — recovery permitted
    SuppressReadOnly,     // Read-only — no worker spawn, no network
    SuppressExplicitSync, // sync, cron, register — manage own behavior
    SuppressInternal,     // auto-sync-worker
    SuppressConfiguration,// update, self-replace, doctor, keybindings, shell, completions
}
```

| Policy | Commands |
|--------|----------|
| `Allow` | `new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, `library create/delete/set-primary`, bare TUI |
| `SuppressReadOnly` | `version`, `list`, `select`, `status`, `get`, `validate`, `backup`, `mcp`, `library list/show`, plus dry-run modes (`restore --dry-run`, `repair --dry-run`, `import pet --dry-run`) and the read-only `data` spellings |
| `SuppressExplicitSync` | `sync`, `cron`, `register` |
| `SuppressInternal` | `auto-sync-worker` |
| `SuppressConfiguration` | `update`, `__self-replace`, `doctor`, `completions`, `shell`, `keybindings` |

The match is exhaustive — adding a command requires choosing a policy and a
`StartupServices` level, enforced by the compiler with no catch-all arm.
Dry-run modes of mutating commands are classified read-only so previews never
trigger network work or create log/config state.

`StartupServices` (`src/main.rs:873`) has two levels: `Minimal` skips file
logging, startup/shutdown log lines, and audit entirely, while `Logging`
initializes file logging plus `log_startup_info()` / `log_shutdown_info()`.
Read-only and dry-run paths pair `SuppressReadOnly` with `Minimal`; every
other policy pairs with `Logging`. Non-`Success` outcomes exit via
`outcome.exit_code()` (`src/main.rs:1019-1024`); hard errors print to stderr
and exit 1 (`src/main.rs:1025-1031`).

## Shared Command Utilities

**File**: `src/commands/mod.rs` — see [commands/mod.md](commands/mod.md):

- `get_config_path()` / `get_library_path()` — config and library resolution
- `load_snippets()` / `save_snippets()` — TOML read/write with recovery
- `get_snippet_data()` — parallel arrays for TUI display
- `expand_snippet_command()` — variable parsing, prompting, expansion
- `run_snippet_selection()` — shared TUI loop (`Option<&Runtime>`; `None`
  when `do_sync` is false) with `process_fn` callback
- `init_library_manager()` — `LibraryManager` construction
- `validate_exact_command_bytes()` — shared 16 MiB cap, UTF-8, no-NUL,
  non-empty gate for `--command-stdin` / `--from-file` / `--editor`

## Exact Selectors

`run`, `clip`, and `edit` accept `--id` / `--description-exact` /
`--command-exact`, which bypass the TUI via
`selector::resolve_exact_target()` (`src/selector.rs:604`;
`src/main.rs:501,547,637`):

- `--id <UUID>` — exact snippet UUID (case-sensitive)
- `--description-exact <text>` — exact description (case-insensitive)
- `--command-exact <text>` — exact command text (case-insensitive)

They conflict with `--filter` (fuzzy TUI pre-filter) and with each other.
Ambiguous description/command matches report identities (exit 5); no match
is exit 3. `snp edit --output/--output-stdin/--clear-output` requires an
exact selector or `--filter`. See [selector.md](selector.md).

## Exit Codes

`CliOutcome` → code mapping lives in `src/outcome.rs:72-91` (stable, see
`docs/EXIT_CODES.md`):

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error (`PersistenceFailed` maps here for backward compat) |
| 2 | Usage/argument error (Clap) |
| 3 | Not found |
| 4 | User cancelled |
| 5 | Ambiguous match |
| 6 | Validation/persistence failure |
| 7 | Sync failure |
| 8 | Execution failure |
| 9 | Conflict/refused |
| 10 | Unsafe repairs pending operator decision (never a `CliOutcome`; `repair` exits directly via `exit_on_repair_status`, `src/main.rs:375`) |

`ExecutionFailed { child_code }` propagates the child process exit code when
available, falling back to 8 (`src/outcome.rs:83-87`; direct
`std::process::exit(child_code.unwrap_or(8))` on the exact and TUI `run`
paths, `src/main.rs:517,541`). `UnsafeOnly` deliberately avoids 2: a valid
invocation awaiting an operator decision is not a usage error.

## Key Files

- `src/main.rs` — CLI schema, signals, `RUNTIME`, dispatch, classification
- `src/commands/mod.rs` — shared helpers, TOML load/save, selection loop
- `src/auto_sync/notification.rs:190` — `StartupRecoveryPolicy` (5 variants)
- `src/outcome.rs:18,46` — `CliOutcome`, `exit_code` constants 0–10
- `src/selector.rs:604` — `resolve_exact_target` for `--id`/`--description-exact`/`--command-exact`
- `src/update.rs` — `snp update` transport (in-process eggfetch-core)

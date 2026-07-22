# CLI Entry & Commands

[← Back to Overview](overview.md)

## Entry Point

**File**: `src/main.rs` (331 lines)

The binary `snp` is built with `clap` for argument parsing. On startup:

1. Panic handler is installed (restores terminal, logs panic info)
2. Signal handlers registered (SIGINT, SIGTERM on Unix; crossterm on Windows)
3. Default tracing logging initialized
4. CLI args parsed, command dispatched via `dispatch_command()`

### Global State

- `CONFIG_PATH: LazyLock<PathBuf>` — Lazy-resolved snippet file path
- `RUNTIME: LazyLock<Runtime>` — Tokio runtime, only initialized when async commands run

### Command Dispatch

```rust
fn dispatch_command(cli: Commands) -> SnipResult<()>
```

All subcommands map 1:1 to a module in `src/commands/`. Each module exposes a `run()` function, except `premade_cmd` and `library_cmd` which use subcommand-dispatched functions (`run_list`, `run_get`, etc.).

## Subcommands

| Command | Alias | Module | Async | Description |
|---------|-------|--------|-------|-------------|
| `version` | `v` | — | No | Print version |
| `new` | `n` | `new_cmd` | No | Create snippet from positional, prompt, multiline, exact stdin, file, or editor |
| `list` | `l` | `list_cmd` | No | List snippets (fuzzy filter; `--search-output` includes output in match) |
| `run` | `r` | `run_cmd` | Yes | TUI select → execute via shell; exact selectors (`--id`, `--description-exact`, `--command-exact`) bypass TUI |
| `clip` | `c` | `clip_cmd` | Yes | TUI select → copy to clipboard; exact selectors (`--id`, `--description-exact`, `--command-exact`) bypass TUI |
| `search` | `s` | `search_cmd` | Yes | TUI select → display snippet info |
| `edit` | `e` | `edit_cmd` | No | Open snippet file in `$EDITOR`; or set/clear output field (`--output`, `--output-stdin`, `--clear-output` with `--filter`); exact selectors (`--id`, `--description-exact`, `--command-exact`) bypass TUI for output editing |
| `get` | — | `get_cmd` | No | Deterministic non-TUI snippet retrieval (never executes, no clipboard) |
| `validate` | — | `validate_cmd` | No | Validate snippet libraries and configuration |
| `backup` | — | `backup_cmd` | No | Backup snippet libraries to directory with manifest |
| `restore` | — | `restore_cmd` | No | Restore snippets from backup (dry-run, merge, replace) |
| `repair` | — | `repair_cmd` | No | Repair snippet libraries and sync artifacts |
| `library` | `lib` | `library_cmd` | No | Manage snippet libraries |
| `premade` | `p` | `premade_cmd` | Yes | Browse/download premade libraries |
| `import` | — | `import_cmd` | No | Import snippets from external formats |
| `doctor` | — | `doctor_cmd` | No | Diagnose configuration and environment |
| `sync` | `y` | `sync_cmd` | Yes | Sync snippets with server |
| `cron` | — | `cron_cmd` | No | Generate crontab entry for auto-sync |
| `register` | `reg` | `register_cmd` | Yes | Register new sync account |
| `keybindings` | `k` | `keybindings_cmd` | No | Print keybinding reference |
| `status` | — | `status_cmd` | No | Show auto-sync status |
| `update` | — | `update_cmd` | No | Check for and install an update |
| `shell` | — | `shell_cmd` | No | Generate interactive shell integration |
| `completions` | — | `completions_cmd` | No | Generate shell completions |
| `auto-sync-worker` | — | `auto_sync::worker` | No | **Hidden.** Detached debounce worker for auto-sync (internal use) |
| `auto-sync-execute` | — | `auto_sync::executor` | No | **Hidden.** Killable sync executor subprocess (internal use) |

The `auto-sync-worker` and `auto-sync-execute` subcommands are registered with
`hide = true` in the clap CLI — they do not appear in `--help` output and are
used internally by the detached worker protocol. See
[auto_sync.md](auto_sync.md) for the full architecture.

## Startup Recovery Classification (Phase 10)

`classify_command()` in `src/main.rs:1191` maps every `Commands` variant to a
`StartupRecoveryPolicy` that gates whether auto-sync recovery runs before
dispatch. This prevents read-only commands from triggering network work.

```rust
pub enum StartupRecoveryPolicy {
    Allow,              // Mutation commands — recovery permitted
    SuppressReadOnly,   // Read-only commands — no worker spawn, no network
    SuppressExplicitSync, // sync, cron, register — manage own behavior
    SuppressInternal,   // auto-sync-worker, auto-sync-execute
    SuppressConfiguration, // doctor, keybindings, shell, completions, update
}
```

### Command Classification

| Policy | Commands |
|--------|----------|
| `Allow` | `new`, `run`, `clip`, `edit`, `import`, `repair`, `restore`, `premade`, `library create/delete/set-primary` |
| `SuppressReadOnly` | `version`, `list`, `search`, `select`, `status`, `get`, `validate`, `backup`, `library list/show` |
| `SuppressExplicitSync` | `sync`, `cron`, `register` |
| `SuppressInternal` | `auto-sync-worker`, `auto-sync-execute` |
| `SuppressConfiguration` | `update`, `doctor`, `completions`, `shell`, `keybindings` |

The classification is exhaustive — every variant is mapped. Adding a new command
requires selecting a policy, enforced by the compiler (no catch-all arm).

## Shared Command Utilities

**File**: `src/commands/mod.rs` (271 lines)

Provides functions shared across command modules:

- `get_config_path()` — Resolve config path from CLI arg or default
- `get_library_path()` — Resolve library path by name or primary
- `load_snippets()` / `save_snippets()` — TOML read/write with error recovery
- `get_snippet_data()` — Extract parallel arrays for TUI display
- `expand_snippet_command()` — Parse variables, prompt user, expand
- `run_snippet_selection()` — Shared TUI selection loop with process callback
- `init_library_manager()` — Create LibraryManager with library mode

### `run_snippet_selection` Pattern

Most TUI commands (`run`, `clip`, `search`) follow the same pattern:
1. Load library and snippets
2. Extract snippet data for TUI
3. Enter selection loop (TUI renders, user filters, selects)
4. Call `process_fn` callback with selected snippet
5. Handle result (Cancel/Continue/Done)
6. Optionally trigger sync on exit

### Exact-Source Validation Pipeline

All exact command sources (`--command-stdin`, `--from-file`, `--editor`) share
`validate_exact_command_bytes()` for: 16 MiB cap, valid UTF-8, no NUL bytes,
and no empty/whitespace-only input. Source resolution completes before library
mutation, so partial failures cannot corrupt the snippet collection. The editor
command specification is parsed with `shell-words` — no shell is invoked.

## Exact Selectors (Phase 08A)

`run`, `clip`, and `edit` support `--id`, `--description-exact`, and `--command-exact`
flags that bypass the TUI entirely. When any of these flags is provided, the command
resolves the snippet deterministically via `SnippetSelector` and proceeds directly
to the action (execute, copy, or edit output field).

- `--id <UUID>` — match by exact snippet UUID
- `--description-exact <text>` — match by exact description (case-insensitive)
- `--command-exact <text>` — match by exact command text (case-insensitive)

These flags conflict with `--filter` (TUI fuzzy filter) and with each other.
See [selector.md](selector.md) for the full resolution model.

## Exit Codes (Phase 08A)

All commands map outcomes to stable exit codes via `CliOutcome`:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Usage/argument error (Clap) |
| 3 | Not found |
| 4 | User cancelled |
| 5 | Ambiguous match |
| 6 | Validation/persistence failure |
| 7 | Sync failure |
| 8 | Execution failure |
| 9 | Conflict/refused |

`PersistenceFailed` maps to code 1 (general error). `ExecutionFailed` propagates the
child process exit code when available, falling back to code 8.

## Key Files

- `src/main.rs` — CLI definition, signal handling, command dispatch
- `src/commands/mod.rs` — Shared helpers, TOML load/save, selection loop
- `src/commands/run_cmd.rs` — Shell execution with output file support
- `src/commands/clip_cmd.rs` — Clipboard copy with audit logging
- `src/commands/search_cmd.rs` — Display snippet details
- `src/commands/new_cmd.rs` — Unified snippet creation pipeline (positional, prompts, multiline, `--command-stdin`, `--from-file`, `--editor`)
- `src/commands/sync_cmd.rs` — Server library linking, conflict resolution
- `src/commands/library_cmd.rs` — Library CRUD operations
- `src/commands/premade_cmd.rs` — Premade library browsing/downloading
- `src/commands/edit_cmd.rs` — Editor resolution (absolute, relative, PATH search); output/notes editing (`--output`, `--output-stdin`, `--clear-output`)
- `src/commands/cron_cmd.rs` — Crontab entry generation
- `src/commands/register_cmd.rs` — Account registration
- `src/commands/keybindings_cmd.rs` — Keybinding reference display
- `src/commands/list_cmd.rs` — CLI-based snippet listing with fuzzy filter
- `src/output.rs` — Output/notes presentation model, terminal sanitization, search scoring
- `src/commands/get_cmd.rs` — Deterministic non-TUI snippet retrieval
- `src/selector.rs` — Shared snippet selector model, resolution policies
- `src/outcome.rs` — CLI outcome types, exit-code mapping

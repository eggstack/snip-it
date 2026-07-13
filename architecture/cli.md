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
| `list` | `l` | `list_cmd` | No | List snippets (with fuzzy filter) |
| `run` | `r` | `run_cmd` | Yes | TUI select → execute via shell |
| `clip` | `c` | `clip_cmd` | Yes | TUI select → copy to clipboard |
| `search` | `s` | `search_cmd` | Yes | TUI select → display snippet info |
| `edit` | `e` | `edit_cmd` | No | Open snippet file in `$EDITOR` |
| `keybindings` | `k` | `keybindings_cmd` | No | Print keybinding reference |
| `sync` | `y` | `sync_cmd` | Yes | Sync snippets with server |
| `cron` | — | `cron_cmd` | No | Generate crontab entry for auto-sync |
| `register` | `reg` | `register_cmd` | Yes | Register new sync account |
| `library` | `lib` | `library_cmd` | No | Manage snippet libraries |
| `premade` | `p` | `premade_cmd` | Yes | Browse/download premade libraries |

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
- `src/commands/edit_cmd.rs` — Editor resolution (absolute, relative, PATH search)
- `src/commands/cron_cmd.rs` — Crontab entry generation
- `src/commands/register_cmd.rs` — Account registration
- `src/commands/keybindings_cmd.rs` — Keybinding reference display
- `src/commands/list_cmd.rs` — CLI-based snippet listing with fuzzy filter

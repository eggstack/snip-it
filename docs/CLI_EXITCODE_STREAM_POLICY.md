# CLI Exit Code and Stream Policy

This document specifies the current behavior and planned contract for snp CLI
exit codes and stdout/stderr stream usage.

## Current Behavior

### Exit Codes

All errors are handled in `src/main.rs:364-368`:

```rust
if let Err(e) = dispatch_command(cli.command) {
    eprintln!("error: {e}");
    std::process::exit(1);
}
```

| Condition | Exit Code | Notes |
|-----------|-----------|-------|
| Success | 0 | Implicit — no `process::exit` call |
| Any `SnipError` | 1 | All error variants map to the same code |
| Async runtime failure | 1 | `LazyLock` panic path (`main.rs:22-25`) |
| Signal handler registration failure | 1 | `main.rs:37-43` |

There is **no distinction** between error types. A TOML parse error, a missing
file, a clipboard failure, and a sync network error all produce exit code 1.

`SnipError` variants (`src/error.rs:28-67`):

| Variant | Typical Cause |
|---------|---------------|
| `Io` | File read/write, directory creation, permission denied |
| `Toml` | Malformed TOML, serialization failure |
| `Clipboard` | Clipboard access denied or unavailable |
| `Command` | Shell command spawn failure (editor, snippet execution) |
| `Runtime` | Sync failure, validation error, timeout, not-found |

None of these map to distinct exit codes today.

### Stream Usage (stdout vs stderr)

#### `snp run` (alias `r`)

- **TUI**: Renders directly to the terminal via crossterm (raw mode).
- **stdout**: Nothing printed on success. The executed command's own
  stdout/stderr pass through to the parent terminal.
- **stderr**: Error messages via `eprintln!` from the main error handler.
- **Exit**: 0 on success (even if the executed command exits non-zero — the
  snippet ran, which counts as success). 1 on `SnipError`.

#### `snp list` (alias `l`)

All three formats write to **stdout** via `println!`.

| Format | Destination | Pipe-friendly? |
|--------|-------------|----------------|
| Default (colored table) | stdout | No (ANSI escapes) |
| `--json` | stdout | Yes |
| `--csv` | stdout | Yes |

`--json` and `--csv` conflict with each other (`clap` enforces this).
Default format uses `crossterm` styling — piping it will include ANSI escape
sequences. Errors (e.g., failed library load) go to stderr.

#### `snp search` (alias `s`)

- **TUI**: Renders directly to the terminal via crossterm.
- **stdout**: After selection, prints snippet details (`Description:`,
  `Command:`, `Output:`, `Tags:`, `Folders:`, `Favorite:`) via `println!`
  (`search_cmd.rs:27-32`).
- **stderr**: Error messages from the main error handler.
- **Exit**: 0 on success (even if user presses `q` — returns `Ok(())`).
  1 on `SnipError`.

#### `snp clip` (alias `c`)

- **TUI**: Renders directly to the terminal via crossterm.
- **stdout**: Nothing printed. The `ProcessResult::Done("Copied to clipboard")`
  message is returned but never printed to any stream.
- **stderr**: Error messages (clipboard failure, etc.) from the main handler.
- **Exit**: 0 on success, 1 on error.

#### `snp select` (alias `sel`)

- **TUI**: Renders directly to the terminal via crossterm.
- **stdout**: Prints the selected command string (raw or expanded) on success.
  When `--output-file` is provided, nothing is printed to stdout; the command
  is written to the file instead.
- **stderr**: Error messages from the main error handler.
- **Exit**: 0 on success, 4 on user cancellation (`q`/`Esc` or variable prompt
  cancel), 1 on `SnipError` (all error variants).
- **Output file**: Rejects symlinks and directories with exit 1. On cancellation,
  the output file is removed if it exists and is a regular file.
- **Return type**: `SnipResult<CommandOutcome>` — `CommandOutcome::Success` or
  `CommandOutcome::Cancelled`. Exit code 4 is mapped at the CLI boundary in
  `main.rs`.

#### `snp new` (alias `n`)

- **Prompts**: `print!()` writes "Command> ", "Description> ", "Tags> " to
  **stdout** (with ANSI color via `crossterm`).
- **Echo**: `println!("Command> {command}")` writes the accepted command to
  stdout when provided as an argument.
- **Success**: `println!("Snippet added")` to **stdout**.
- **Errors**: To stderr via the main error handler.

Interactive prompts use `io::stdout().flush()` and `io::stdin().read_line()`
directly — they do not go through the TUI layer.

#### `snp edit` (alias `e`)

- Opens `$EDITOR` (or falls back to `vim`) as a child process. The editor
  inherits the terminal directly.
- **stdout/stderr**: The editor's own output goes to the terminal.
- **Errors** (editor not found, library not found): To stderr.

#### `snp version` (alias `v`)

- **stdout**: `println!("snp {version}")`.
- **stderr**: Nothing on success.

#### `snp completions` (alias `g`)

- **stdout**: Generated shell completions via `clap_complete::generate()`.
- **stderr**: Nothing on success.

#### `snp keybindings` (alias `k`)

- **stdout**: All keybinding documentation via `println!`
  (`keybindings_cmd.rs:5-79`).
- **stderr**: Nothing on success.

#### `snp cron` (alias `cr`)

- **stdout**: Crontab entry and instructions via `println!`
  (`cron_cmd.rs:37-58`).
- **Prompts**: `print!("Copy to clipboard? [y/N]: ")` to **stdout**.
- **Errors**: Clipboard failure to stderr via `eprintln!`.
- **Validation**: Invalid interval (0) returns `SnipError::Runtime` → exit 1.

#### `snp sync` (alias `y`)

| Situation | Stream | Method |
|-----------|--------|--------|
| Sync progress ("Syncing snippets...") | stdout | `println!` |
| Server library listing | stdout | `println!` |
| Conflict prompt ("(s)kip / (o)verwrite / (r)ename") | stdout | `println!` |
| Dry-run output | stdout | `println!` |
| Sync disabled / no API key | stderr | `eprintln!` |
| Failed to pull libraries | stderr | `eprintln!` |
| Failed to create sync client | stderr | via `SnipError` |

Status messages are split across both streams. No consistent convention.

#### `snp register` (alias `reg`)

- **stdout**: "Registration successful!", masked API key, device ID, saved path
  (`register_cmd.rs:44-59`).
- **stderr**: "Already registered!" message, save failure, registration failure.
- **Exit**: 0 on success, 1 on error.

#### `snp library` (alias `lib`)

| Subcommand | stdout | stderr |
|------------|--------|--------|
| `list` | "Libraries:" + list | Nothing |
| `create` | "Created library..." | Nothing |
| `delete` | Confirmation prompt + "Deleted" | Non-interactive refusal |
| `set-primary` | "Set ... as primary" | Nothing |
| `show` | Library metadata | "Library not found" |

#### `snp premade` (alias `p`)

| Subcommand | stdout | stderr |
|------------|--------|--------|
| `list` | Available libraries | "Sync not enabled" |
| `get` | Download confirmation | "Sync not enabled" |
| `sync` | (delegates to `sync_commands`) | Errors |
| `search` | Matching libraries | "Sync not enabled" |
| `update` | Diff stats + confirmation | "Sync not enabled" |

### Important Observations

1. **TUI commands** (`run`, `clip`, `search`) render directly to the terminal
   through crossterm's raw mode — they bypass stdout/stderr entirely for the
   interactive portion.

2. **`list` default format** goes to stdout (not stderr). It includes ANSI color
   escapes, making it unsuitable for piping without `--json` or `--csv`.

3. **`search` selected snippet** goes to stdout via `println!`, not stderr.
   This is the opposite of what you might expect from a "display" command.

4. **`keybindings`** goes to stdout, not stderr. This is informational output.

5. **`cron`** goes to stdout for the crontab entry but uses an interactive
   `print!` prompt on stdout (not stderr), which could interfere with piping.

6. **`sync`** splits status messages across both streams with no clear
   convention — progress on stdout, errors on stderr, but "Syncing snippets..."
   goes to stdout.

7. **Error messages** always go to stderr via `eprintln!` in the main error
   handler (`main.rs:365`), or via `eprintln!` in individual commands before
   returning `Ok(())` (graceful degradation pattern).

8. **`new` prompts** go to stdout, not stderr. Piping `snp new` would see the
   "Command> " prompt on stdout mixed with any piped content.

## Proposed Contract (Release 1B+)

### Exit Codes

| Code | Name | Meaning | Examples |
|------|------|---------|----------|
| 0 | `SUCCESS` | Operation completed successfully | Snippet executed, clipboard copied, list printed |
| 1 | `ERROR` | General/unclassified error | Default for all current `SnipError` variants |
| 2 | `USAGE` | Invalid arguments or missing required input | Bad CLI flags, missing library name for `library delete` |
| 3 | `NOT_FOUND` | Requested resource does not exist | Snippet not found, library not found, file missing |
| 4 | `CANCELLED` | User cancelled TUI interaction | `q`/`Esc` in snippet selector (`snp select`) |
| 5 | `IO` | Filesystem or clipboard failure | Cannot write file, clipboard unavailable |
| 6 | `PARSE` | Configuration or data format error | Malformed TOML, invalid sync config |

**Migration path**: Exit codes 2-6 are additive. Existing scripts checking
`exit != 0` will continue to work. Scripts can opt into finer-grained handling
by checking specific codes.

**Note**: The `ProcessResult::Cancel` variant (returned when the user presses
`q` in the TUI) maps to exit 0 for existing commands (`run`, `clip`, `search`)
where cancellation is a valid "done" state. For `snp select`, cancellation
maps to exit 4 via `CommandOutcome::Cancelled`, which is returned to the CLI
boundary in `main.rs`.

### Stream Contract

| Stream | Content | Examples |
|--------|---------|----------|
| **stdout** | Machine-readable output only | JSON, CSV, selected command text, shell completions |
| **stderr** | Human-readable output | Tables, progress, errors, prompts, keybinding docs |
| **terminal** (raw) | TUI rendering | Snippet selector, variable prompt, theme picker |

**Rules**:

1. **stdout** must never contain ANSI escape sequences, prompts, or progress
   messages. It is safe for piping and redirection.

2. **stderr** is for anything a human reads on the terminal: colored tables,
   status messages, error messages, interactive prompts.

3. **TUI commands** continue to render directly to the terminal. When a TUI
   command selects a snippet and needs to emit machine-readable output, it
   goes to stdout (e.g., `snp select` prints the command to stdout).

4. **Error messages** always go to stderr, prefixed with `error:` (current
   behavior in `main.rs:365`).

### Command-by-Command Stream Changes

| Command | Current stdout | Proposed stdout | Current stderr | Proposed stderr |
|---------|---------------|-----------------|----------------|-----------------|
| `list` (default) | Colored table | *Move to stderr* | Nothing | Table |
| `list --json` | JSON | JSON (no change) | Nothing | Nothing |
| `list --csv` | CSV | CSV (no change) | Nothing | Nothing |
| `search` | Snippet details | Snippet details | Nothing | Nothing |
| `select` | Command string | Command string (no change) | Nothing | Nothing |
| `keybindings` | Keybinding docs | *Move to stderr* | Nothing | Keybinding docs |
| `cron` | Crontab entry | *Move to stderr* | Nothing | Crontab entry |
| `new` prompts | "Command> " | *Move to stderr* | Nothing | "Command> " |
| `new` success | "Snippet added" | *Move to stderr* | Nothing | "Snippet added" |
| `version` | Version string | Version string (no change) | Nothing | Nothing |
| `completions` | Completions | Completions (no change) | Nothing | Nothing |
| `sync` progress | Status messages | *Move to stderr* | Errors | Errors |
| `register` | Success + keys | *Move to stderr* | Errors | Errors |
| `library` subcmds | Metadata | *Move to stderr* | Errors | Errors |
| `premade` subcmds | Results | *Move to stderr* | "Not enabled" | "Not enabled" |

**Key changes**:
- `list` default format moves to stderr (colored table is human-readable)
- `keybindings`, `cron`, `new`, `sync`, `register`, `library`, `premade`
  status output moves to stderr
- `--json` and `--csv` remain on stdout (machine-readable)
- `version` and `completions` remain on stdout (machine-readable / standard)

### For `snp select` (Release 1B — implemented)

A `snp select` primitive provides non-TUI snippet selection for scripting:

| Scenario | stdout | stderr | Exit Code |
|----------|--------|--------|-----------|
| Selection to stdout | exact command | empty except tracing | 0 |
| Selection to output file | empty | empty except tracing | 0 |
| User cancellation (`q`/`Esc`) | empty | empty | 4 |
| Variable prompt cancelled | empty | empty | 4 |
| `SnipError` (all variants) | empty | `error: ...` | 1 |

**Usage**:

```bash
# Run selected snippet
command=$(snp select -f "git") && eval "$command"

# Check for cancellation
if ! snp select -f "deploy" > /tmp/cmd.sh; then
    case $? in
        4) echo "Cancelled" ;;
        *) echo "Error" ;;
    esac
fi
```

### Backward Compatibility

- **Exit code 0/1**: No change. All existing scripts checking `exit == 0` or
  `exit != 0` continue to work.
- **New exit codes (2-6)**: Additive. Only scripts that explicitly check for
  these codes will be affected.
- **Stream moves**: Moving human-readable output from stdout to stderr may
  break scripts that `grep` or parse stdout from `snp list`, `snp keybindings`,
  etc. This is a **breaking change** for those scripts — document in release
  notes and provide a `--stdout` flag during transition.
- **`--stdout` flag** (transitional): When human-readable output moves to
  stderr, a `--stdout` flag will force it back to stdout for backward
  compatibility. Deprecated after two releases.

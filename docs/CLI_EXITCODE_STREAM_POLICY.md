# CLI Exit Code and Stream Policy

This document specifies the current behavior and planned contract for snp CLI
exit codes and stdout/stderr stream usage.

## Current Behavior

### Exit Codes

Exit codes are now **implemented and stable** via `CliOutcome` in
`src/outcome.rs`, mapped to `exit_code::*` constants. The authoritative,
verified reference is [`docs/EXIT_CODES.md`](EXIT_CODES.md).

| Code | Name | Meaning |
|------|------|---------|
| 0 | `SUCCESS` | Snippet executed/copied, or command completed |
| 1 | `GENERAL_ERROR` | Any unclassified `SnipError` / persistence failure |
| 2 | `USAGE_ERROR` | CLI argument error (clap) |
| 3 | `NOT_FOUND` | Snippet not found |
| 4 | `CANCELLED` | User cancelled TUI interaction (`snp select` only) |
| 5 | `AMBIGUOUS` | Multiple snippets match filter |
| 6 | `VALIDATION_FAILED` | Data validation failure |
| 7 | `SYNC_FAILED` | Sync operation failure |
| 8 | `EXECUTION_FAILED` | Output-file execution failure (timeout/spawn) |
| 9 | `CONFLICT_OR_REFUSED` | Lock conflict, kernel refusal |
| 10 | `UNSAFE_REPAIRS` | Repair refused: unsafe repairs require manual review |

Selection semantics: `run_snippet_selection()` returns `SelectionOutcome`
(Selected or Cancelled). For `run`, `clip`, and `search`, cancellation is
treated as normal completion (exit 0). For `snp select`, cancellation maps
to exit 4 via `CommandOutcome::Cancelled` at the CLI boundary in `main.rs`.

### Stream Usage (stdout vs stderr)

#### `snp run` (alias `r`)

- **TUI**: Renders directly to the terminal via crossterm (raw mode).
- **stdout**: Nothing printed on success. The executed command's own
  stdout/stderr pass through to the parent terminal.
- **stderr**: Error messages via `eprintln!` from the main error handler.
- **Exit**: 0 on success (even if the executed command exits non-zero — the
  snippet ran, which counts as success). 1 on `SnipError`.
- **Sort flags**: `--sort <mode>` and `--favorites-first` are accepted.
  Sorting affects the TUI display order but not the exit code or output.

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

**Sort flags**: `--sort <mode>` and `--favorites-first` affect the output
ordering of all three formats. `--json` and `--csv` respect explicit sort
flags. Without flags, output uses the default relevance ordering.

#### `snp search` (alias `s`)

- **TUI**: Renders directly to the terminal via crossterm.
- **stdout**: After selection, prints snippet details (`Description:`,
  `Command:`, `Output:`, `Tags:`, `Folders:`, `Favorite:`) via `println!`.
- **stderr**: Error messages from the main error handler.
- **Exit**: 0 on success (even if user presses `q` — returns `Ok(())`).
  1 on `SnipError`.
- **Sort flags**: `--sort <mode>` and `--favorites-first` are accepted.
  Sorting affects the TUI display order but not the exit code or output.

#### `snp clip` (alias `c`)

- **TUI**: Renders directly to the terminal via crossterm.
- **stdout**: Nothing printed. The `ProcessResult::Done("Copied to clipboard")`
  message is returned but never printed to any stream.
- **stderr**: Error messages (clipboard failure, etc.) from the main handler.
- **Exit**: 0 on success, 1 on error.
- **Sort flags**: `--sort <mode>` and `--favorites-first` are accepted.
  Sorting affects the TUI display order but not the exit code or output.

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
- **Sort flags**: `--sort <mode>` and `--favorites-first` are accepted.
  Sorting affects the TUI display order. The `--query` (alias `--filter`)
  flag pre-fills the search. Sorting and filtering are orthogonal.

#### `snp new` (alias `n`)

- **Prompts**: `print!()` writes "Command> ", "Description> ", "Tags> " to
  **stdout** (with ANSI color via `crossterm`).
- **Echo**: `println!("Command> {command}")` writes the accepted command to
  stdout when provided as an argument.
- **Success**: `println!("Snippet added")` to **stdout**.
- **Errors**: To stderr via the main error handler.

#### Release 2A command ingestion

`snp new --command-stdin` explicitly assigns stdin to the command body. The
body is read as bytes, validated as UTF-8, and passed through unchanged,
including supplied trailing newlines. It is not echoed to stdout, evaluated,
executed, or included in normal-level ingestion logs. Invalid UTF-8, NUL bytes,
and inputs larger than 16 MiB return exit 1 before a snippet is appended.

Because command stdin is consumed in full, `--description` is required and the
prompt-only form of `--tags` is unavailable. Use `--tags git,release` (or omit
the option) for noninteractive capture. The existing positional form keeps its
current prompt and command-echo behavior.

Generated `snp_new_current` and `snp_new_previous` helpers pass command text to
this mode using the active shell's buffer/history API. They do not execute the
text, parse history files, or install keybindings automatically.

#### Release 2B file and editor ingestion

`snp new --from-file` reads a file as exact UTF-8 command data. Symlinks are
followed; the resolved target must be a regular file. The same validation as
stdin applies (16 MiB, UTF-8, no NUL, no empty/whitespace-only).

`snp new --editor` resolves `$VISUAL` → `$EDITOR` → `vim` and parses the editor
specification with `shell-words` (no shell invoked). Editor errors identify the
executable and exit status but never the command body. All exact sources share
`validate_exact_command_bytes()` — there is no source-specific validation path.

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

- **stdout**: All keybinding documentation via `println!`.
- **stderr**: Nothing on success.

#### `snp cron` (alias `cr`)

- **stdout**: Crontab entry and instructions via `println!`.
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

- **stdout**: "Registration successful!", masked API key, device ID, saved path.
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

#### `snp import`

| Situation | Stream | Method |
|-----------|--------|--------|
| Human report (default) | stderr | `eprintln!` |
| JSON report (`--report json`) | stdout | `println!` |
| `--report-file` write | file | `write_private_atomic` |
| Import errors (source not found, TOML parse, collision) | stderr | via `SnipError` |
| Dry-run success message | stderr | `eprintln!` |

**Exit codes**: 0 on success (including dry-run), 1 on any `SnipError` (source
missing, invalid TOML, destination collision, strict-mode abort, file too
large).

**Stream split**: Clean — human-readable report always goes to stderr;
machine-readable JSON always goes to stdout. Piping `snp import pet --report json`
produces only JSON on stdout; the human report appears on stderr.

#### `snp doctor`

| Situation | Stream | Method |
|-----------|--------|--------|
| Human report (default) | stderr | `eprintln!` |
| JSON report (`--report json`) | stdout | `println!` |
| Operational errors (file not found, unreadable) | stderr | via `SnipError` |

**Exit codes**: 0 on success (no error-severity diagnostics), 1 on operational failure
(source not found, unreadable, not a file), 2 if error-severity diagnostics are detected
(incompatible entries in the analyzed file).

**Stream split**: Clean — human-readable report always goes to stderr;
machine-readable JSON always goes to stdout. Same convention as `snp import`.

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
   handler (`main.rs:819`), or via `eprintln!` in individual commands before
   returning `Ok(())` (graceful degradation pattern).

8. **`new` prompts** go to stdout, not stderr. Piping `snp new` would see the
   "Command> " prompt on stdout mixed with any piped content.

### Auto-Sync Error Exit Code

When auto-sync is configured with `auto_sync_failure = "error"` and the
parent mutation command fails to spawn the detached one-shot worker
(`snp auto-sync-worker`), the command returns a nonzero exit code
(1, via `SnipError::Runtime`). The local mutation has already succeeded —
the exit code reflects the post-commit scheduling failure, not a local
failure. Worker-side sync failures are logged to `~/.config/snp/logs/`
and surfaced via `snp doctor --compatibility`; they do not propagate to
the parent because the parent has already returned to the user.

This is a **post-commit** exit code: scripts can distinguish local
mutation failure (which never reaches the auto-sync stage) from a
successful local mutation followed by a failed auto-sync spawn. The
local state is always readable regardless of the auto-sync failure.

Auto-sync scheduling failure messages
(`error: auto-sync scheduling failed; pending work preserved for recovery`)
go to stderr via `eprintln!` — stdout is never contaminated. Worker-side
diagnostics appear in the log files and via `snp doctor` only.

## Exit Code Contract (IMPLEMENTED) vs Stream Contract (ASPIRATIONAL)

> **Status**: The exit-code portion of this contract **is implemented** — codes
> 0–10 exist in `src/outcome.rs` (superseding the 2-6 proposal below; see the
> Current Behavior table). The **stream contract is still aspirational** and has
> not been implemented: human-readable output still goes to stdout, no
> `--stdout` transitional flag exists. The stream sections below describe a
> possible future direction only.

### Historical Proposal: Exit Codes (SUPERSEDED)

The original Release 1B proposal below was narrower than what shipped.
It is retained for design rationale only:

| Code | Name | Meaning | Examples |
|------|------|---------|----------|
| 0 | `SUCCESS` | Operation completed successfully | Snippet executed, clipboard copied, list printed |
| 1 | `ERROR` | General/unclassified error | Default for unclassified failures |
| 2 | `USAGE` | Invalid arguments or missing required input | Bad CLI flags |
| 3 | `NOT_FOUND` | Requested resource does not exist | Snippet/library not found |
| 4 | `CANCELLED` | User cancelled TUI interaction | `q`/`Esc`/Ctrl-C in selector (`snp select`) |
| 5 | `IO` | Filesystem or clipboard failure | Cannot write file |
| 6 | `PARSE` | Configuration or data format error | Malformed TOML |

The shipped mapping instead distinguishes `AMBIGUOUS` (5), `VALIDATION_FAILED`
(6), `SYNC_FAILED` (7), `EXECUTION_FAILED` (8), `CONFLICT_OR_REFUSED` (9), and
`UNSAFE_REPAIRS` (10) — see `docs/EXIT_CODES.md`.

**Migration path**: New exit codes are additive. Existing scripts checking
`exit != 0` will continue to work.

**Note**: `run_snippet_selection()` returns `SelectionOutcome` (Selected or
Cancelled). For existing commands (`run`, `clip`, `search`), cancellation is
treated as normal completion (exit 0). For `snp select`, cancellation
maps to exit 4 via `CommandOutcome::Cancelled`, which is returned to the CLI
boundary in `main.rs`. Ctrl+C in the TUI (normal mode) also maps to
`SelectionOutcome::Cancelled` → exit 4 for `select`.

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
   behavior in `main.rs:819`).

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
| User cancellation (`q`/`Esc`/Ctrl-C) | empty | empty | 4 |
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
- **New exit codes (2-10)**: Additive and stable (see `docs/EXIT_CODES.md`).
  Only scripts that explicitly check for these codes will be affected.
- **Stream moves** (aspirational): Moving human-readable output from stdout to
  stderr would break scripts that `grep` or parse stdout from `snp list`,
  `snp keybindings`, etc. This is a **breaking change** for those scripts —
  document in release notes and provide a `--stdout` flag during transition.
- **`--stdout` flag** (transitional, not yet implemented): If human-readable
  output ever moves to stderr, a `--stdout` flag will force it back to stdout
  for backward compatibility. Deprecated after two releases.

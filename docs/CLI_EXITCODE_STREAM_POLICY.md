# CLI Exit Code and Stream Policy

> **Scope of this document:** the **"Current Behavior"** section below is the
> live contract. Everything after it is a pointer to archived material.

**Evergreen policy** for snp CLI exit codes and stdout/stderr stream usage.
The authoritative code table is [`EXIT_CODES.md`](EXIT_CODES.md); this document
covers stream discipline and the cases that are easy to get wrong. The
per-command behavioural table (stdout/stderr flags, startup-recovery policy,
exit categories) lives in [`COMMAND_CONTRACTS.md`](COMMAND_CONTRACTS.md). The
"planned contract" language has been retired — the contract is implemented and
stable via `CliOutcome` (`src/outcome.rs`).

## Current Behavior

### Exit Codes

Exit codes are **implemented and stable** via `CliOutcome` in `src/outcome.rs`,
mapped to the `exit_code::*` constants. The authoritative code table lives in
[`EXIT_CODES.md`](EXIT_CODES.md) — it is deliberately **not** repeated here, so
codes 0–10 have exactly one definition.

Two properties matter when reasoning about streams:

- Exit **8** (`EXECUTION_FAILED`) is a **wrapper** used only when the child
  produced no exit code of its own; otherwise the child's code is propagated
  verbatim. Timeout and spawn failures are a subset, not the definition.
- Exit **10** (`UNSAFE_REPAIRS`) is deliberately distinct from exit 2 — a valid
  invocation that found only unsafe repairs is not a usage error.

Selection semantics: `run_snippet_selection()` returns `SelectionOutcome`
(`Selected`, `Cancelled`, or `ExecutionFailed`). For `run`, `clip`, and
`search`, cancellation is treated as normal completion (exit 0). For
`snp select`, cancellation maps to exit 4 via `CliOutcome::Cancelled`
(`src/commands/select_cmd.rs:110`), and `main.rs` turns that outcome into the
process exit code.

### Stream Usage (stdout vs stderr)

#### `snp run` (alias `r`)

- **TUI**: Renders directly to the terminal via crossterm (raw mode).
- **stdout**: Nothing printed on success. The executed command's own
  stdout/stderr pass through to the parent terminal.
- **stderr**: Error messages via `eprintln!` from the main error handler.
- **Exit**: `ExecutionFailed { child_code }` propagates the child's exit code
  (`src/outcome.rs`); the wrapper code 8 applies only when no child code
  exists. 1 on `SnipError`.
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
- **Also accepts**: `--sync` (opportunistic sync before selecting; see
  `architecture/auto_sync.md`) and `--library`.

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
- **Output file**: Written through an atomic temp-file + rename
  (`src/commands/select_cmd.rs:118-125`). A directory target is rejected with
  exit 1; a symlink target is *replaced*, not followed, so the write cannot be
  redirected elsewhere. A pre-existing output file is preserved when the
  selection is cancelled — it is never removed on cancellation.
- **Return type**: `SnipResult<CliOutcome>` — `CliOutcome::Success` or
  `CliOutcome::Cancelled`. Exit code 4 is mapped at the CLI boundary in
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
| `delete` | "Cancelled." or "Deleted library '<name>'" | Confirmation prompt and non-interactive refusal |
| `set-primary` | "Set ... as primary" | Nothing |
| `show` | Library metadata | "Library not found" |

#### `snp premade` (alias `p`)

| Subcommand | stdout | stderr |
|------------|--------|--------|
| `list` | Available libraries | "Sync not enabled" |
| `get` | "Downloaded premade library '<name>' to <path>" | "Sync not enabled" |
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
(source not found, unreadable, not a file), 6 (`ValidationFailed`) if error-severity diagnostics are detected
(`src/commands/doctor_cmd.rs` maps through the canonical outcome table).

**Stream split**: Clean — human-readable report always goes to stderr;
machine-readable JSON always goes to stdout. Same convention as `snp import`.

### Important Observations

1. **TUI commands** (`run`, `clip`, `search`, `select`) render directly to the
   terminal through crossterm's raw mode — they bypass stdout/stderr entirely
   for the interactive portion.

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
   handler (`src/main.rs:1064`), or via `eprintln!` in individual commands before
   returning `Ok(())` (graceful degradation pattern).

8. **`new` prompts** go to stdout, not stderr. Piping `snp new` would see the
   "Command> " prompt on stdout mixed with any piped content.

### Auto-Sync Scheduling Failure

When auto-sync is configured and the parent mutation command fails to spawn the
detached one-shot worker (`snp auto-sync-worker`), the **mutation command still
exits 0**. A scheduling failure never becomes a command error:
`report_notification_result()` surfaces it "without turning a successful local
mutation into a failed command" (`src/auto_sync/notification.rs:40-55`).

The local mutation has already succeeded and is durable; the pending generation
remains on disk for retry or startup recovery.

| Where | Stream | Text |
|-------|--------|------|
| `error` failure mode | log file | `auto-sync scheduling failed; pending work preserved for recovery` (`tracing::error!`) |
| `warn` failure mode | log file | same message, `tracing::warn!` |
| any failure mode | stderr | `warning: local change was saved, but auto-sync could not be scheduled; pending generation <n> remains for recovery` (or `…but pending sync intent could not be recorded` when the marker itself could not be written) |

stdout is never contaminated. Worker-side sync failures are recorded in
`auto-sync-status.toml` and logged; they do not propagate to the parent,
because the parent has already returned to the user. Inspect them with
`snp status` or `snp doctor --sync`.

Practical consequence for scripts: a successful local mutation and a failed
auto-sync spawn are **not** distinguishable by exit code. Detect a preserved
pending generation through `snp status` (the pending line) or the log file
instead.

## Retired Proposal (SUPERSEDED)

The pre-`CliOutcome` exit-code proposal and the aspirational stdout/stderr
contract have been moved out of this document:

- [`archive/CLI_EXITCODE_STREAM_POLICY_SUPERSEDED.md`](archive/CLI_EXITCODE_STREAM_POLICY_SUPERSEDED.md)
  — the retired 2–6 exit-code table and the unimplemented "Command-by-Command
  Stream Changes" table, retained for design rationale only.
- [`EXIT_CODES.md`](EXIT_CODES.md) — **the live exit-code contract** (codes 0–10
  via `CliOutcome` in `src/outcome.rs`).

The retired exit-code table was never shipped. The transitional `--stdout` flag
it proposes still does not exist, and most of its stream contract has not been
applied: `list`, `search`, `keybindings`, `cron`, `new`, `sync`, `register`,
`library`, and `premade` still write human-readable output to stdout. The
report-style commands are the exception — `import`, `doctor`, `validate`,
`repair`, and `backup` already write their human-readable report to stderr and
keep machine-readable output on stdout.

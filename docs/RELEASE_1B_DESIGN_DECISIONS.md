# Release 1B Design Decisions: `snp select` Primitive

This document records the design decisions for the `snp select` command — the core
machine-facing selection primitive for shell integration in Release 1B.

## Background

`snp select` is the foundation for shell integration (R1C). It outputs the selected
snippet's command to stdout so shell wrappers can capture and execute it. This
separates selection from execution, enabling workflows like:

```bash
command=$(snp select --query "git") && eval "$command"
```

The command reuses the existing fuzzy-search TUI and variable expansion pipeline.
It adds no new TUI rendering, no new search algorithm, and no new persistence layer.

## Source Documents

- `plans/pet-compat-release-1b-select-primitive.md` — R1B implementation plan
- `plans/pet-compat-release-1a-baseline-and-contract.md` — R1A baseline (what R1A establishes)
- `docs/CLI_EXITCODE_STREAM_POLICY.md` — exit code and stream contract
- `docs/ARCHITECTURE_INVENTORY.md` — internal architecture boundaries

## Codebase Observations

Before recording decisions, the following codebase facts were verified:

| Artifact | Location | Key Detail |
|----------|----------|------------|
| `run_snippet_selection()` | `src/commands/mod.rs:239-319` | Shared selection service. Takes a `process_fn` callback. Loops on `ProcessResult::Continue`, breaks on `Done`/`Cancel`. |
| `SnippetSelection` | `src/ui/mod.rs:464-469` | `Selected(usize, Option<String>)` or `Delete(usize)` — the TUI's return type. |
| `TerminalGuard` | `src/ui/mod.rs:38-45` | RAII guard: disables mouse capture + calls `ratatui::restore()` on drop. Created inside `select_snippet_inner()`. |
| `expand_snippet_command()` | `src/commands/mod.rs:213-232` | Parses variables, prompts via `ui::prompt_variables()`, returns `ExpandedCommand`. |
| `ProcessResult` | `src/lib.rs:48-55` | `Cancel`, `Continue`, `Done(String)`. `Cancel` currently maps to exit 0 in all callers. |
| `SnipError` | `src/error.rs:28-67` | All errors → exit 1 via `main.rs:364-368`. No exit code distinction today. |
| Exit code 4 | `docs/CLI_EXITCODE_STREAM_POLICY.md:206` | Defined as `CANCELLED` — "User cancelled TUI interaction". Reserved for `snp select`. |
| CLI dispatch | `src/main.rs:238-354` | Match on `Commands` enum, calls `commands::*_cmd::run()`. |
| Commands enum | `src/main.rs:63-195` | Clap subcommands with aliases. No `Select` variant yet. |

---

## Design Decisions

### 1. Command Name: `snp select`

**Decision**: The command is named `select`, not `pick`, `choose`, or `run-raw`.

**Rationale**: "select" is the most neutral term for a non-executing selection action.
"run" is already taken. "pick" implies one-shot and is too informal. "choose" implies
multiple rounds of selection. "emit" is too abstract for CLI discoverability.

**Alternatives considered**:
- `pick` — too informal, implies one-shot selection
- `choose` — implies multiple rounds of interactive selection
- `emit` — too abstract, not discoverable in `--help`
- `run-raw` — implies execution (even if "raw"), contradicts non-executing invariant

**Relationship to existing commands**: `select` sits alongside `run` (execute), `clip`
(clipboard), and `search` (inspect). All four share `run_snippet_selection()` as their
selection backbone. The distinction is purely in what happens after selection:

| Command | Post-selection action |
|---------|----------------------|
| `run` | Execute via shell |
| `clip` | Copy to clipboard |
| `search` | Print metadata to stdout |
| `select` | Print command string to stdout |

**Alias**: `s` is taken by `search`. No alias is assigned initially. The plan does
not require one.

### 2. Default Output Mode: Raw

**Decision**: Default output is raw (stored command text, no variable expansion).
`--expanded` activates variable prompting.

**Rationale from R1A (D2)**: Raw output is recommended for shell-buffer insertion
because it preserves editability and avoids surprise interactive prompts. A shell
widget inserting `echo "hello $USER"` should keep the variable reference, not
resolve it at selection time.

**Implementation**:
- `--raw` (default, available as explicit self-documenting flag)
- `--expanded` switches to variable-prompting mode
- `--raw --expanded` is a clap conflict (rejected at parse time)

**Flag behavior**:

```
snp select              # raw mode (default)
snp select --raw        # explicit raw (same as default)
snp select --expanded   # prompts for variables, emits resolved command
```

**Future**: `--emit` and `--format` flags from the R1B plan are deferred. The initial
release emits only the command string. Structured output (JSON) is a future addition
that does not change the default behavior.

### 3. Stdout Contract: Command String + Trailing Newline

**Decision**: On successful selection, stdout contains exactly the selected command
string followed by a single newline (`\n`).

**Rationale**: This matches standard CLI convention (`echo` behavior) and makes
command substitution straightforward:

```bash
command=$(snp select --query "git")
# $command contains the command string without trailing newline
# (command substitution strips trailing newlines)
```

**What goes to stdout**:
- Raw mode: `snippet.command` as-is (no variable expansion)
- Expanded mode: fully resolved command after variable prompts
- Nothing else — no description, tags, metadata, prompts, or ANSI escapes

**What goes to stderr**:
- Warnings (library load warnings, deprecated config)
- Errors (missing config, parse failure, I/O error)
- Normal cancellation prints nothing to stderr (silent)

**Terminal (crossterm)**:
- TUI rendering goes through the controlling terminal (crossterm raw mode)
- When stdout is captured by `$(...)`, the TUI still renders because crossterm
  uses its own terminal handle (not stdout) for rendering

**Trailing newline policy** (from R1B plan E2):
- Payload bytes are preserved exactly
- One record-separating newline is appended
- For a single-line command `echo hello`, stdout is `echo hello\n`
- For a multiline command, trailing newlines in the command are preserved,
  and one additional newline terminates the record
- This is the simplest policy for R1 shell integration. Exact multiline
  preservation via JSON/base64 is deferred to R2+.

### 4. Cancel/Escape Behavior: Exit Code 4

**Decision**: User pressing `q`, `Esc`, or `Ctrl-C` in the TUI exits with code 4
and empty stdout.

**Exit code**: 4 (`CANCELLED`) per `docs/CLI_EXITCODE_STREAM_POLICY.md:206`.

**Rationale**: Distinguishes "user cancelled" from "error" (exit 1) and "success"
(exit 0). Shell integrations must be able to silently ignore cancellation while
surfacing errors:

```bash
command=$(snp select) || {
    code=$?
    if [ "$code" -eq 4 ]; then
        # User cancelled — preserve current buffer
        return
    else
        echo "Error selecting snippet" >&2
        return 1
    fi
}
eval "$command"
```

**Alternative rejected**: Exit 0 with empty stdout — indistinguishable from selecting
an empty command string. A user could intentionally select a snippet whose command
is an empty string (valid in TOML), and that should succeed with exit 0.

**Behavior specifics**:
- `q` in TUI: exit 4, empty stdout, no stderr output
- `Esc` in TUI: exit 4, empty stdout, no stderr output
- `Ctrl-C` in TUI: exit 4, empty stdout, no stderr output (signal handler restores terminal)
- Cancel during variable prompt (`--expanded` mode): exit 4, empty stdout
- `Ctrl-C` before TUI opens: exit 4, empty stdout

**Cancellation is not logged as an error.** No audit log entry, no tracing error.

### 5. Error Behavior: Exit Code 1

**Decision**: All operational errors print to stderr and exit with code 1.

**Rationale**: Consistent with current error handling (`main.rs:364-368`). The
proposed fine-grained exit codes (2-6) from `CLI_EXITCODE_STREAM_POLICY.md` are
additive and not yet implemented. R1B uses exit 1 for all errors to avoid
introducing multiple exit codes before the error mapping is refactored.

**Future**: When the exit code mapping is updated project-wide, `select` can adopt
codes 3 (NOT_FOUND), 5 (IO), and 6 (PARSE) without changing its behavior contract.

**Error cases and stderr messages**:
- No library found: `"No library found. Create one with 'snp library create <name>'"`
- Library not found by name: `"error: Library '<name>' does not exist. Use 'snp library list' to see available libraries."`
- TOML parse error: via `SnipError::toml_error()` → stderr
- I/O error (file read/write): via `SnipError::io_error()` → stderr
- No controlling terminal (non-interactive): `"error: snp select requires a terminal for interactive selection"` → stderr, exit 1

### 6. Variable Handling

**Decision**: Variables are handled based on the output mode.

**Raw mode (default)**: No variable prompt. The stored command string is printed
as-is, including `<name>` and `<name=default>` placeholders.

**Expanded mode (`--expanded`)**: Variables are prompted through the existing TUI
variable dialog (`ui::prompt_variables()`). The fully resolved command is printed.

**Rationale**: Raw mode is the default because shell-buffer insertion should preserve
placeholders and avoid interactive prompts. Expanded mode is opt-in for users who
want resolved commands.

**Edge cases**:
- Snippet has no variables: both modes print the same string
- User cancels variable prompt: treated as cancellation (exit 4)
- Snippet has only escaped `\<` / `\>` (no actual variables): no prompt in either mode
- Unicode variable values: handled by existing expansion pipeline

### 7. TUI Behavior: Reuse `select_snippet_inner()`

**Decision**: Reuse the existing `select_snippet_inner()` event loop. No new TUI code.

**Rationale**: Same fuzzy search, same keyboard shortcuts, same UX. Only the
output mechanism changes (command to stdout vs. execution vs. clipboard).

**Flow**:
1. Load library via `get_library_path()`
2. Load snippets via `load_library()`
3. Extract snippet data via `get_snippet_data()`
4. Launch TUI via `select_snippet()` with `initial_filter` from `--query`
5. On selection: process via callback (expand or raw)
6. Print command to stdout
7. Exit 0

**Terminal lifecycle** (critical):
1. `ratatui::init()` enters alternate screen + raw mode (inside `select_snippet_inner`)
2. `TerminalGuard` is created — ensures cleanup on any exit path
3. TUI event loop runs
4. User selects → `select_snippet_inner()` returns `Ok(Some(SnippetSelection::Selected(...)))`
5. `TerminalGuard` drops → terminal restored (alternate screen exited, raw mode off, mouse capture disabled)
6. After terminal restoration: `println!()` writes command to stdout
7. Process exits 0

**Ordering guarantee**: The terminal MUST be fully restored before writing to stdout.
The `TerminalGuard` is dropped when `select_snippet_inner()` returns, which happens
before the callback in `run_snippet_selection()` processes the result. This ordering
is already correct in the current architecture.

### 8. No Metadata in Output

**Decision**: Only the command string goes to stdout. No description, tags, or other fields.

**Rationale**: The primary use case is shell execution via command substitution.
Metadata would break `$(snp select)` capture:

```bash
# This must contain ONLY the command, not "Description: ...\nCommand: ..."
command=$(snp select --query "git")
eval "$command"
```

**`--emit` flag** (from R1B plan): Deferred. Could be added as:
- `--emit command` (default)
- `--emit description`
- `--emit json`

JSON output would enable structured integration without breaking the default
contract. Deferred to R2+ to keep R1B scope minimal.

### 9. Relationship to `snp run` and `snp clip`

**Decision**: `snp select` is a new command, not a flag on `snp run`.

**Rationale**: Clean separation of concerns. `select` = capture, `run` = execute,
`clip` = clipboard. Different exit code semantics. Different stdout contracts.

**Implementation**: All four commands (`run`, `clip`, `search`, `select`) share
`run_snippet_selection()` as their selection backbone. The difference is the
`process_fn` callback:

```rust
// run_cmd.rs — executes the command
run_snippet_selection(filter, library, do_sync, runtime, |snippet, copy_flag| {
    process_snippet(snippet, copy_flag.is_some())  // spawns shell process
})

// clip_cmd.rs — copies to clipboard
run_snippet_selection(filter, library, do_sync, runtime, |snippet, copy_flag| {
    process_snippet(snippet, copy_flag)  // clipboard write
})

// search_cmd.rs — prints metadata
run_snippet_selection(filter, library, do_sync, runtime, |snippet, _copy_flag| {
    println!("Description: {}", snippet.description);
    // ... more fields
    Ok(ProcessResult::Done(String::new()))
})

// select_cmd.rs — prints command to stdout
run_snippet_selection(filter, library, do_sync, runtime, |snippet, copy_flag| {
    let cmd = /* raw or expanded */;
    // stdout write happens after TUI cleanup
    Ok(ProcessResult::Done(cmd))
})
```

**Code sharing**: `select_cmd` does not duplicate library resolution, snippet loading,
fuzzy matching, TUI invocation, or variable expansion. It composes existing services.

### 10. Piped/TTY Detection

**Decision**: `snp select` is interactive-only. If no controlling terminal is
available, fail with exit 1 and a clear error message.

**Rationale**: The TUI requires a terminal for rendering and user interaction.
When invoked as `selected="$(snp select)"`, the command substitution captures
stdout but the TUI still renders to the controlling terminal (crossterm handles
this). However, if there is truly no terminal (e.g., in a cron job or headless CI),
the command should fail immediately rather than hang.

**Detection**: Check for a controlling terminal before launching the TUI.
On Unix: `std::io::stdin().is_terminal()` or `atty::is(atty::Stream::Stdin)`.
On Windows: equivalent console handle check.

**Behavior**:
- Terminal present: TUI renders normally, command selection proceeds
- No terminal: `"error: snp select requires a terminal for interactive selection"` to stderr, exit 1

**Future**: Non-interactive selection modes (e.g., `snp select --query "git" --filter "echo" --json`)
could be added in R2+ for scripting without a TUI. Not in scope for R1B.

### 11. Exit Code Contract

| Exit Code | Name | Condition | stdout |
|-----------|------|-----------|--------|
| 0 | SUCCESS | Command selected and printed | Command string + `\n` |
| 1 | ERROR | Operational error (config, I/O, parse, no terminal) | Empty |
| 4 | CANCELLED | User cancelled in TUI or variable prompt | Empty |

**Notes**:
- Exit 4 is silent — no error message on stderr
- Exit 1 always includes a diagnostic message on stderr
- Codes 2, 3, 5, 6 from `CLI_EXITCODE_STREAM_POLICY.md` are reserved but not yet
  implemented. `select` uses 1 for all errors to avoid introducing fine-grained
  codes before the project-wide exit code mapping is updated.
- Existing commands (`run`, `clip`, `search`, etc.) continue to use 0/1 only.
  The `Cancel → exit 4` mapping applies only to `snp select`.

### 12. `--query` Flag

**Decision**: `--query <TEXT>` seeds the TUI's filter input with an initial search term.

**Rationale**: This is the primary shell integration hook. The shell buffer content
becomes the initial query:

```bash
selected="$(snp select --query "$BUFFER")"
```

**Behavior**:
- The TUI opens with the query pre-filled in the search/filter input
- Snippets are immediately filtered (fuzzy match against the query)
- User can refine or clear the query in the TUI
- Unicode and whitespace are preserved (no shell parsing)
- The query does not affect variable expansion or output

**Implementation**: Passed as `initial_filter` to `SnippetListParams` via
`run_snippet_selection()`.

### 13. `--library` Flag

**Decision**: `--library <NAME>` constrains selection to a specific library.

**Rationale**: Consistency with `run`, `clip`, and `search` which all accept
`--library`. Shell workflows often target a specific library:

```bash
selected="$(snp select --library work --query "deploy")"
```

**Behavior**: Reuses existing library resolution via `get_library_path()`.
Missing library → exit 1 with error on stderr.

### 14. `--filter` Flag

**Decision**: `--filter <TEXT>` is accepted for consistency with `run`/`clip`/`search`.

**Rationale**: All TUI-based commands accept `--filter` as the initial search term.
The `--query` flag is the primary interface for shell integration, but `--filter`
is kept for internal consistency. Both set `initial_filter` on the TUI.

**Note**: `--query` and `--filter` are synonyms in this context. `--query` is the
recommended name for shell integration. `--filter` is the established name in the
existing CLI. Both are accepted; if both are provided, `--query` takes precedence
(or they are treated as an error — to be decided during implementation).

### 15. Shared Selection Architecture

**Decision**: `select` uses `run_snippet_selection()` unchanged. No refactoring of
the shared service is required.

**Rationale**: The R1B plan (A2) recommends a safe sequence:
1. Extract a narrow service behind current behavior ✓ (already exists as `run_snippet_selection`)
2. Keep current run/clip adapters unchanged ✓
3. Add `select` as a new adapter ✓
4. Compare behavior through tests ✓

The callback-based design of `run_snippet_selection()` already supports adding new
output modes without modifying the shared selection logic.

**Cancellation handling**: `run_snippet_selection()` returns `Ok(())` on cancellation
(`ProcessResult::Cancel`). For `select`, this must map to exit 4. The `select_cmd`
module will track cancellation state via a `std::cell::Cell<bool>` captured in the
callback closure:

```rust
let cancelled = std::cell::Cell::new(false);

run_snippet_selection(filter, library, do_sync, runtime, |snippet, _copy_flag| {
    if /* user cancelled */ {
        cancelled.set(true);
        return Ok(ProcessResult::Cancel);
    }
    // ... process snippet
    Ok(ProcessResult::Done(command))
})?;

if cancelled.get() {
    std::process::exit(4);
}
```

### 16. Terminal Cleanup Before Stdout Write

**Decision**: Terminal must be fully restored before writing the command to stdout.

**Rationale**: The TUI takes over the terminal (raw mode, alternate screen). If the
command is written to stdout before terminal restoration, the output may be lost
or mixed with terminal escape sequences.

**Ordering**:
1. `select_snippet_inner()` returns → `TerminalGuard` drops → terminal restored
2. `run_snippet_selection()` callback returns `ProcessResult::Done(command)`
3. `select_cmd` receives the command string
4. `println!("{}", command)` writes to stdout
5. Process exits 0

This ordering is already guaranteed by the current architecture:
- `TerminalGuard` is created inside `select_snippet_inner()` (line 497)
- It drops when `select_snippet_inner()` returns
- The callback runs after the TUI returns
- stdout writes happen after the callback

**Risk**: If `select_snippet_inner()` is refactored to return before dropping
`TerminalGuard`, the ordering breaks. The design decision is documented here to
prevent that regression.

---

## Implementation Plan

### Files to Create

1. `src/commands/select_cmd.rs` — new module implementing `snp select`

### Files to Modify

1. `src/commands/mod.rs` — add `pub mod select_cmd;`
2. `src/main.rs` — add `Select` variant to `Commands` enum and dispatch

### `select_cmd.rs` Structure

```rust
use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use std::cell::Cell;

#[derive(Clone, Copy, PartialEq)]
enum OutputMode {
    Raw,
    Expanded,
}

fn process_snippet(
    snippet: &Snippet,
    mode: OutputMode,
    cancelled: &Cell<bool>,
) -> SnipResult<crate::ProcessResult> {
    match mode {
        OutputMode::Raw => {
            let command = snippet.command.clone();
            Ok(crate::ProcessResult::Done(command))
        }
        OutputMode::Expanded => {
            match crate::commands::expand_snippet_command(snippet)? {
                crate::commands::ExpandedCommand::Cancel => {
                    cancelled.set(true);
                    Ok(crate::ProcessResult::Cancel)
                }
                crate::commands::ExpandedCommand::Skip => {
                    Ok(crate::ProcessResult::Continue)
                }
                crate::commands::ExpandedCommand::Expanded(cmd) => {
                    Ok(crate::ProcessResult::Done(cmd))
                }
            }
        }
    }
}

pub fn run(
    filter: Option<String>,
    library: Option<String>,
    raw: bool,
    expanded: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let mode = if expanded { OutputMode::Expanded } else { OutputMode::Raw };
    let cancelled = Cell::new(false);

    run_snippet_selection(filter, library, false, runtime, |snippet, _copy_flag| {
        process_snippet(snippet, mode, &cancelled)
    })?;

    if cancelled.get() {
        std::process::exit(4);
    }

    Ok(())
}
```

**Note on stdout write**: The command string is returned via `ProcessResult::Done(String)`.
The `println!` must happen AFTER `run_snippet_selection()` returns (which is after the
TUI terminal is restored). This is handled by having the callback return the command
in `ProcessResult::Done`, and the caller printing it after the function returns.

Actually, reviewing the flow more carefully: `run_snippet_selection()` processes the
result internally. For `select`, the command needs to be printed to stdout AFTER
terminal cleanup. The callback returns `ProcessResult::Done(command)` but the
message is currently discarded (`Done(_msg)` at line 301). For `select`, we need
to capture the command and print it after the function returns.

**Revised approach**: Use a `Cell<Option<String>>` to capture the selected command,
then print it after `run_snippet_selection()` returns:

```rust
let selected_command = Cell::new(None);
let cancelled = Cell::new(false);

run_snippet_selection(filter, library, false, runtime, |snippet, _copy_flag| {
    let result = process_snippet(snippet, mode, &cancelled)?;
    if let crate::ProcessResult::Done(cmd) = &result {
        selected_command.set(Some(cmd.clone()));
    }
    Ok(result)
})?;

if cancelled.get() {
    std::process::exit(4);
}

if let Some(command) = selected_command.take() {
    println!("{}", command);
}

Ok(())
```

### `main.rs` Changes

Add to `Commands` enum:

```rust
/// Select a snippet and print its command to stdout (no execution)
#[command(alias = "sel")]
Select {
    #[arg(short, long)]
    filter: Option<String>,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    sync: bool,
    #[arg(short, long)]
    library: Option<String>,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    raw: bool,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    expanded: bool,
},
```

Add to `dispatch_command()`:

```rust
Some(Commands::Select {
    filter,
    sync,
    library,
    raw,
    expanded,
}) => {
    commands::select_cmd::run(filter, library, raw, expanded, &RUNTIME)?;
}
```

---

## Open Questions

1. **`--query` vs `--filter`**: Should `select` use `--query` as the primary name
   (as recommended in R1B plan) or stick with `--filter` for consistency? Recommendation:
   accept both, prefer `--query` in documentation.

2. **`--emit` flag**: Deferred to R2+. Could add `--emit command|description|json`
   without breaking the default contract.

3. **`--format json`**: Deferred to R2+. Would change stdout to a JSON object with
   fields like `command`, `library`, `description`, `tags`.

4. **Multiline command trailing newline**: For a command ending with `\n`, should
   the output be `command\n\n` (payload + record newline) or `command\n` (payload
   only)? Recommendation: preserve payload bytes + append one newline. Document
   that command substitution strips trailing newlines.

5. **Alias**: No alias assigned initially. `sel` could be added if `select` proves
   cumbersome in shell scripts. Most shell integration will use the full name.

6. **`--sync` flag**: Accepted for consistency with `run`/`clip`/`search`. Runs
   background sync after selection. May be unnecessary for shell integration
   (users rarely want sync delay in their shell workflow). Accept it but document
   that it is rarely used with `select`.

---

## Handoff Notes for Release 1C

Release 1C should treat the following as stable dependencies from R1B:

- Command name: `snp select`
- Exit code 0: success, command printed to stdout
- Exit code 4: user cancelled, empty stdout, silent
- Exit code 1: error, diagnostic on stderr
- `--query` flag: seeds TUI filter
- `--library` flag: constrains to specific library
- `--raw` / `--expanded` flags: output mode
- Stdout contract: command string + trailing newline
- Terminal restoration: guaranteed before stdout write
- Raw mode preserves placeholders; expanded mode resolves them

Release 1C must not parse human-readable output or depend on terminal escape
behavior. It should call only the documented machine-facing interface.

# run_cmd — Execute Snippet

## Overview

`run_cmd` provides TUI-based snippet selection and shell execution. It is the primary way users run their snippets.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. **TUI Selection** — Call `run_snippet_selection()` to get user-selected snippet
2. **Variable Expansion** — If snippet has variables (`<name>` or `<name=default>`), call `ui::prompt_variables()` to collect values
3. **Command Expansion** — `expand_snippet_command()` substitutes values into command
4. **Execute** — `Command::new(shell).arg("-c").arg(&expanded).spawn()` and wait
5. **Output Capture** — If snippet has `output` field set, display/capture stdout
6. **Clipboard** — Optional copy to clipboard (`--clip`)
7. **Audit Log** — Record execution in `audit.log`

## Shell Execution

- Default shell from `$SHELL` env var, fallback to platform-specific (`/bin/sh`, `cmd.exe`)
- `Command::new(shell).arg("-c")` for POSIX, `cmd.exe /c` for Windows
- Blocks until command completes
- Return code checked; non-zero logged as error

## Variable Handling

See [variables.md](../../utils/variables.md) for parsing/expansion details.

### Prompt UI

`ui::prompt_variables()` shows TUI dialog:
- Defaults shown in muted color
- Arrow keys / tab to navigate
- `q` cancel, `Enter` accept
- `Esc` to skip all (use defaults)

## Flags

- `--clip` — Copy output to clipboard after execution
- `--sync` — Sync with server after execution

## Audit Log

Records:
- Snippet name
- Timestamp
- Expanded command (for debugging)
- Exit code

## Error Handling

- `SnipError::Command` for execution failures
- `SnipError::VariableNotFound` for missing required variables
- `SnipError::Clipboard` for clipboard failures

## Related

- [mod.md](mod.md) — Shared helpers
- [clip_cmd.md](clip_cmd.md) — Clipboard copy variant
- [search_cmd.md](search_cmd.md) — Search/display variant
- [sync_cmd.md](sync_cmd.md) — Sync integration

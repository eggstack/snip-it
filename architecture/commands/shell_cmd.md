# shell_cmd — Shell Integration Code Generation

**Source:** `src/commands/shell_cmd.rs`

## Purpose

Generates shell integration functions for bash, zsh, and fish. These functions allow users to interact with snp directly from their shell prompt — selecting snippets, capturing the current buffer as a new snippet, and saving the previous history entry as a snippet.

## Generated Functions

Each shell gets three functions:

### `snp_select_raw` / `snp_select_expanded`
Opens the TUI snippet selector and replaces the current shell buffer with the selected command.

- `raw` — inserts the command verbatim
- `expanded` — prompts for `<name>` variables before inserting

### `snp_new_current`
Captures the current shell buffer (what the user has typed) and creates a new snippet from it via `snp new --command-stdin`.

### `snp_new_previous`
Captures the previous shell history entry and creates a new snippet from it via `snp new --command-stdin`.

## Safety Properties

1. **No `eval`** — generated code never uses `eval` or evaluates arbitrary strings
2. **No history file access** — uses shell builtins (`fc`, `history search`, `commandline`) instead of reading history files directly
3. **Buffer preservation** — on cancellation (exit 4) or error, the original `$READLINE_LINE` / `$BUFFER` / `commandline` is restored
4. **No execution on source** — sourcing the generated code only defines functions; nothing executes until the user invokes a function

## Transport Mechanism

Selection uses `--output-file` (atomic temp file) rather than stdout, because stdout is connected to the terminal and cannot be captured by the shell function reliably. The shell function reads the temp file and cleans it up.

## Shell-Specific Details

| Shell | Buffer API | History API | Keybinding |
|-------|-----------|-------------|------------|
| Bash | `$READLINE_LINE` / `$READLINE_POINT` | `fc -ln` | `bind -x` |
| Zsh | `$BUFFER` / `$CURSOR` | `fc -ln` | `zle -N` + `bindkey` |
| Fish | `commandline` | `history search` | `bind` |

## Testing

Tests are extensive:
- **Syntax checks** — `bash -n`, `zsh -n`, `fish --no-execute` verify generated code parses
- **Function existence** — source and verify all functions are defined
- **Behavioral tests** — stub `snp` executable, source generated code, verify buffer manipulation
- **Cancellation tests** — verify buffer restoration on exit 4 and errors
- **Edge cases** — multiline selection, special characters, missing `snp`

## Integration Points

- **`select_cmd`**: The `snp select --output-file` command that shell functions call
- **`new_cmd`**: The `snp new --command-stdin` command that capture functions call
- **`doctor_cmd --check-shell`**: Validates generated shell code syntax

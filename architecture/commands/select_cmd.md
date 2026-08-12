# select_cmd — Non-Executing Selection Primitive

**Source:** `src/commands/select_cmd.rs`

## Purpose

`select_cmd` provides a non-executing snippet selection command that prints the selected snippet's command text to stdout (or writes it to an output file). This is the foundational building block for shell integration — shell functions call `snp select --output-file <tmp>` and read the result back into the shell buffer.

## Design Principles

1. **Never executes** — only selects and returns text
2. **Atomic output file** — uses temp-file + `rename(2)` to avoid truncation, symlink races, and partial writes
3. **Cancellation-safe** — cancellation (exit 4) does not create or modify the output file
4. **Buffer-preserving** — shell functions restore `$READLINE_LINE` / `$BUFFER` on cancellation or error

## Flow

1. `run()` resolves library and snippet data via `run_snippet_selection()`
2. The callback `process_snippet()` runs in one of two modes:
   - `Raw` — returns the command text verbatim
   - `Expanded` — prompts the user for `<name>` / `<name=default>` variables via `ui::prompt_variables()`
3. On success, the selected command is either:
   - Printed to stdout (no `--output-file`)
   - Written atomically to the output file (with `--output-file`)

## Output File Safety

`select_cmd` delegates to `utils::atomic::atomic_replace()` with the
durability class used for selected output. The canonical writer:
1. Creates parent directories if needed
2. Opens a fresh same-directory temp file (no truncation of existing files)
3. Writes and flushes the selected bytes
4. Atomically renames the temp file to the target path

When symlink replacement is allowed, the final rename replaces the symlink
directory entry itself, including a broken symlink; it never writes through to
the former target. Sensitive configuration writes retain symlink rejection.

This prevents:
- **Truncation of existing files** on cancellation (temp file is independent)
- **Symlink redirection** (`rename(2)` replaces the symlink itself, not its target)
- **Partial writes** (atomic rename)

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success — command text written to stdout/file |
| 4 | User cancelled TUI selection |
| 1 | General error (library missing, I/O failure) |

## Integration Points

- **Shell functions** (`snp shell init`): Call `snp select --output-file` and read the file back
- **`run_snippet_selection()`**: Shared TUI selection loop in `commands/mod.rs`
- **`expand_snippet_command()`**: Variable expansion before output
- **`ui::prompt_variables()`**: TUI dialog for entering variable values

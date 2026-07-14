# edit_cmd — Edit Snippets

## Overview

`edit_cmd` opens the snippets library file in the user's preferred editor for direct text editing.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. Determine editor: `$EDITOR` env var → fallback to platform default
2. Determine library path: active library file
3. Open in editor: `Command::new(editor).arg(path).spawn()`
4. Wait for editor to exit

## Supported Editors

Uses `clap` value parser that checks for known editors:
- `vim`, `nvim`, `nano`, `code`, `subl`, `emacs`, etc.
- Falls back to system default if `$EDITOR` is not set

## Use Cases

- Bulk editing multiple snippets
- Precise control over TOML structure
- Search/replace across all snippets
- Comment/uncomment snippets for temporary disable

## Error Handling

- `SnipError::Command` if editor not found
- `SnipError::Io` if library file doesn't exist
- `SnipError::Toml` if edited file fails to parse on reload

## Note

Does not use TUI; launches external editor process. Terminal state is preserved and restored via tracing panic handler.

## Related

- [mod.md](mod.md) — Path resolution and library loading
- [library.md](../library.md) — Library and snippet data structures

## Output/Notes Editing (Release 4B)

When `--output`, `--output-stdin`, or `--clear-output` flags are provided, the edit command
operates in structured output-editing mode instead of opening `$EDITOR`.

### CLI Flags

- `--output <text>` — Set the output/notes field to the given text.
- `--output-stdin` — Read the output/notes field from stdin (byte-for-byte).
- `--clear-output` — Clear the output/notes field to empty.
- `--filter <query>` — Required; selects the snippet by description or command substring match.

### Conflicts

- `--output`, `--output-stdin`, and `--clear-output` are mutually exclusive.
- `--filter` is required when any output flag is present.

### Behavior

1. Loads the library file.
2. Finds the first non-deleted snippet matching the filter (case-insensitive substring).
3. Updates the `output` field and bumps `updated_at`.
4. Saves atomically with backup.
5. Reports the operation to stderr.

### Safety

- Cancellation is implicit: if no matching snippet is found, returns an error.
- No command execution or variable expansion occurs on the output value.
- The edit is atomic (backup + temp file + rename).

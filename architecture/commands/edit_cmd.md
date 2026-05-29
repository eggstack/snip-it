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

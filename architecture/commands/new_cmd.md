# new_cmd — Create New Snippet

## Overview

`new_cmd` provides interactive snippet creation via the TUI. It supports multiline input, tags, folders, and favorite marking.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. **Name input** — Prompt for snippet name
2. **Command input** — Multiline TUI input (Ctrl+D to finish, Ctrl+C to cancel)
3. **Optional fields** — Tags, folders, favorite flag
4. **Save** — Write to library via `save_snippets()` after backup

## Key Features

### Multiline Input

Uses `ui::multiline_input()` to capture multi-line commands:
- `Enter` adds new line
- `Ctrl+D` finishes input
- `Ctrl+C` cancels

### Tags

Comma-separated input parsed into `Vec<String>`.

### Folder Organization

Snippets can be placed in folders for organization.

### Favorite

Boolean flag for quick access / sorting.

## Library Integration

- Loads existing snippets with `load_snippets()`
- Creates backup before save with `backup_library()`
- Appends new snippet, sorts by `updated_at` descending
- Saves with `save_snippets()`

## Error Handling

- `SnipError::Toml` on serialization failure
- `SnipError::Io` on file write failure
- User can cancel at any prompt (Ctrl+C)

## Related

- [mod.md](mod.md) — Shared helpers (path resolution, library loading)
- [library.md](../library.md) — Snippet data structures

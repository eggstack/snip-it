# search_cmd — Search and Display Snippet

## Overview

`search_cmd` provides fuzzy search through snippets and displays detailed information about the selected snippet.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. **TUI Selection** — Call `run_snippet_selection()` to get user-selected snippet
2. **Display** — Show snippet details in the TUI:
   - Name
   - Command (with syntax highlighting)
   - Output (if set)
   - Tags
   - Folders
   - Favorite status
   - Created/Updated timestamps

## Fuzzy Matching

Uses `fuzzy-matcher` with `SkimMatcherV2`:
- Matches against snippet name, command, tags
- Displays match score in debug mode
- Results update as user types (with debouncing)

## Display Modes

Users can toggle between display modes with `z` key:
- **Normal** — Compact list view in TUI
- **Detailed** — Full snippet view after selection

## Related

- [mod.md](mod.md) — Shared helpers
- [run_cmd.md](run_cmd.md) — Execution variant
- [clip_cmd.md](clip_cmd.md) — Clipboard variant
- [tui.md](../tui.md) — TUI architecture

# clip_cmd — Copy to Clipboard

## Overview

`clip_cmd` copies a snippet's command to the system clipboard via TUI selection.

## Entry Point

```rust
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    sort_opts: Option<SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
) -> SnipResult<()>
```

## Flow

1. **TUI Selection** — Call `run_snippet_selection()` to get user-selected snippet
2. **Expand** — `expand_snippet_command()` resolves variables and strips escapes
3. **Copy** — `copy_to_clipboard(snippet, &final_command)` copies expanded command, records audit log, and updates usage index

## Clipboard Backend

Platform-specific via `clipboard-win` (Windows) or `arboard` (macOS/Linux):
- `copy_to_clipboard(text)` — Copy string to system clipboard
- `copy_to_clipboard_auto(text)` — Copy with auto-clear from sync settings
- `clear_clipboard()` — Clear clipboard contents

## Side Effects

The `copy_to_clipboard()` helper performs three operations:
1. Copies the expanded command string to the system clipboard
2. Records an audit log entry for the copy action
3. Updates the usage index for the snippet

## Related

- [run_cmd.md](run_cmd.md) — Execution variant (run + optional clip)
- [mod.md](mod.md) — Shared helpers
- [clipboard.md](../clipboard.md) — Platform-specific clipboard implementation

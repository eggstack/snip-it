# clip_cmd — Copy to Clipboard

## Overview

`clip_cmd` copies a snippet's command to the system clipboard via TUI selection. Optionally clears the clipboard after a timeout.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. **TUI Selection** — Call `run_snippet_selection()` to get user-selected snippet
2. **Copy** — `clipboard::copy_to_clipboard(&snippet.command)`
3. **Auto-clear** — If `--clear <seconds>` flag provided, schedule clipboard clear

## Clipboard Backend

Platform-specific via `clipboard-win` (Windows) or `copypasta` (macOS/Linux):
- `copy_to_clipboard(s: &str)` — Copy string to clipboard
- `clear_clipboard()` — Clear clipboard contents
- Generation tracking to avoid self-clear

## Auto-clear Scheduling

When `--clear <seconds>` is provided:
- Spawns background tokio task
- Waits for specified duration
- Calls `clear_clipboard()`
- Generation check ensures clipboard hasn't been modified by another application

## Flags

- `--clear <seconds>` — Auto-clear clipboard after N seconds
- `--sync` — Sync with server after operation

## Related

- [run_cmd.md](run_cmd.md) — Execution variant (run + optional clip)
- [mod.md](mod.md) — Shared helpers
- [clipboard.rs](../../clipboard.md) — Platform-specific clipboard implementation

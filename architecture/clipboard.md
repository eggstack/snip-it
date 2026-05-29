# Clipboard

[← Back to Overview](overview.md)

## File

**`src/clipboard.rs`** (162 lines)

Cross-platform clipboard access with auto-clear scheduling.

## Platform Support

| Platform | Crate | Backend |
|----------|-------|---------|
| Windows | `clipboard-win` | Windows Clipboard API |
| macOS/Linux | `copypasta` | X11/Wayland/macOS pasteboard |

## Functions

| Function | Description |
|----------|-------------|
| `copy_to_clipboard(text)` | Copy text to system clipboard |
| `copy_to_clipboard_auto(text)` | Copy with auto-clear from sync settings |
| `copy_to_clipboard_with_auto_clear(text, seconds)` | Copy with explicit auto-clear delay |
| `schedule_clipboard_clear(seconds)` | Schedule clipboard clear in background thread |
| `clear_clipboard()` | Immediately clear clipboard contents |

## Auto-Clear

When `clipboard_auto_clear_seconds` is set in sync settings, the clipboard is automatically cleared after the specified delay:

1. Copy text to clipboard
2. Spawn background thread that sleeps for N seconds
3. Clear clipboard contents
4. Reset scheduling flag

Uses `AtomicBool` to prevent multiple concurrent clear schedules.

## Integration

- `src/commands/run_cmd.rs` — Copies snippet before execution if `copy` flag set
- `src/commands/clip_cmd.rs` — Primary clipboard copy command
- `src/commands/cron_cmd.rs` — Optional clipboard copy of crontab entry
- `src/ui.rs` — `y` key and Ctrl+C copy selected snippet

## Tests

- Empty string, normal text, unicode, multiline, special chars, long content (100K chars)

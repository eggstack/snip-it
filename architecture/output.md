# Output Presentation Module

## Purpose

Provides safe rendering and presentation helpers for the snippet `output` field (also known as notes/description metadata).

## Module: `src/output.rs`

### Core Types

- `OutputPresentation<'a>` — Wrapper around a borrowed output string with display methods.

### Key Functions

- `OutputPresentation::new(raw)` — Creates a presentation from a raw output string.
- `is_present()` — Returns true if the output field has content.
- `summary(max_chars)` — Single-line truncated summary for inline display.
- `display()` — Full multiline content with terminal control sequences neutralized.
- `display_bounded(max_lines)` — Content truncated to N lines with count note.
- `for_scoring()` — Bounded substring for fuzzy-match scoring (512 char budget).
- `sanitize_for_terminal(input)` — Strips ANSI SGR, OSC sequences, and C0/C1 control characters.

### Security Properties

- Output content is never evaluated, executed, or interpolated.
- ANSI/OSC escape sequences are stripped for display but preserved in storage and JSON/CSV.
- No shell expansion or variable substitution is applied to output.
- Display sanitization operates on a copy; the stored value is never mutated.
- **Local-only sync contract**: `output` is not represented in `ProtoSnippet` (the protobuf wire format), never uploaded or downloaded during sync, and preserved locally when remote data wins the merge. Another device does not receive the value automatically.

## Integration Points

- **TUI Preview** (`src/ui/mod.rs`): Uses `OutputPresentation::display()` to render output in preview panel.
- **List Command** (`src/commands/list_cmd.rs`): Uses `for_scoring()` when `--search-output` is enabled; uses `summary()` for default display.
- **Edit Command** (`src/commands/edit_cmd.rs`): Sets, replaces, or clears the `output` field.
- **JSON/CSV Export** (`src/commands/list_cmd.rs`): Preserves raw output value without sanitization.

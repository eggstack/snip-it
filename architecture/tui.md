# TUI Architecture (`ui/`)

## Overview

The TUI provides an interactive, fuzzy-search snippet selector using `ratatui` and `crossterm`. It is event-driven with a single render loop.

## Main Loop (`ui/mod.rs`)

`select_snippet_inner()` is the core function:
- Loads snippets from library
- Renders a filterable list
- Handles keyboard/mouse events
- Calls a user-provided closure on selection

### State

```rust
pub struct SelectState {
    pub filter: String,           // Current filter text
    pub search_mode: SearchMode,  // Normal / Incremental (/)
    pub sort_mode: SortMode,      // Name / Date / Usage
    pub tag_filter: Option<String>,
    pub visual_mode: bool,        // Multi-select
    pub display_mode: DisplayMode,
}
```

### Fuzzy Matching

Uses `fuzzy-matcher` crate with `SkimMatcherV2`:
- Matches snippet names and commands
- Score-based ranking
- Debounced updates (150ms) to avoid excessive recomputation

## Themes (`ui/theme.rs`)

Two built-in themes:
- `DARK_THEME` — Default, dark background
- `BRIGHT_THEME` — Light background

Colors: `primary`, `secondary`, `accent`, `background`, `text`, `border`, `selected_bg`, `muted`

Resolved via:
1. `SNP_THEME` environment variable (`dark` / `bright`)
2. `COLORFGBG` terminal environment variable (auto-detect)

## Syntax Highlighting (`ui/highlight.rs`)

`highlight_command()` tokenizes the command string and applies colors:
- **Variables** (`<name>`) — Accent color
- **Shell keywords** (~190 commands) — Primary color
- **Strings** (quoted) — Green
- **Flags** (`--flag`) — Secondary color
- **Comments** (`# ...`) — Muted
- **Escape sequences** (`\<`, `\>`) — Magenta

Pre-computed once at startup, cached for draw loop performance.

## Variable Prompt (`ui/variables.rs`)

`prompt_variables()` shows a TUI dialog for entering variable values:
- Shows variable name and default
- Editable text field
- Keyboard navigation (arrows, tab, enter)
- `q` to cancel, `Esc` to skip

Returns `VariablePromptResult::Cancel | Skip | Values(...)`.

## Keybindings

### Normal Mode

| Key | Action |
|-----|--------|
| `h` / `←` | Move left (visual mode) |
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `l` / `→` | Move right (visual mode) |
| `gg` | Jump to top |
| `G` | Jump to bottom |
| `v` | Toggle visual mode |
| `V` | Visual line mode |
| `y` | Copy selected to clipboard |
| `/` | Start incremental search |
| `t` | Tag filter mode |
| `n` | Sort by name |
| `o` | Sort by date |
| `a` | Sort by usage |
| `z` | Toggle display mode |
| `q` / `Esc` | Quit |
| `Enter` | Select/execute |

### Insert Mode

| Key | Action |
|-----|--------|
| `j` / `k` / `↑` / `↓` | Navigate list |
| `Esc` | Return to normal mode |
| `/` | Start search |
| `Enter` | Select/execute |
| `Ctrl+f` / `PageDown` | Page down |
| `Ctrl+b` / `PageUp` | Page up |
| `Ctrl+u` | Half page up |
| `Ctrl+d` | Half page down |

### Mouse

- Scroll to navigate
- Click to select
- Double-click to execute

## Signal Handling

Unix signals (`SIGINT`, `SIGTERM`) restore terminal state before exit.

## Performance Considerations

- **Pre-computed highlights** computed once at startup, not in draw loop
- **Debounced filtering** (150ms) to avoid re-matching on every keystroke
- **Lazy rendering** via ratatui's dirty flag mechanism

## Known Edge Cases

- Unmatched `<` without `>` is treated as a literal `<` in the output (no variable substitution, character preserved).
- Long command strings may not wrap correctly in constrained terminal widths

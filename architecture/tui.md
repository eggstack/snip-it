# TUI Architecture (`ui/`)

## Overview

The TUI provides an interactive, fuzzy-search snippet selector using `ratatui` and `crossterm`. It is event-driven with a single render loop.

## Main Loop (`ui/mod.rs`)

`select_snippet_inner()` is the core function:
- Loads snippets from library
- Renders a filterable list
- Handles keyboard/mouse events
- Returns a selection or a confirmed delete action to the caller

### State

```rust
// In ui/state.rs
pub(super) struct SelectState {
    pub selected: usize,               // Current selection index
    pub scroll_state: ScrollbarState,  // Scrollbar position
}

pub(super) struct FilterState {
    pub sort_mode: SortMode,           // None, Newest, Oldest, AlphaAsc, AlphaDesc, LastUsed, MostUsed
    pub tag_filter_text: String,       // Tag filter input
}
```

### Usage Data Integration

The TUI receives a `usage: Option<&[UsageData]>` parameter via `SnippetListParams`, loaded once per selection session from `~/.config/snp/usage.toml`. This data is used by `sort_filtered_indices()` for real `LastUsed` and `MostUsed` sorting instead of proxying through `updated_at`. The usage slice is indexed by the same `original_indices` mapping as snippets, ensuring identity stability across sort/filter transitions.

### Fuzzy Matching

Uses `fuzzy-matcher` crate with `SkimMatcherV2`:
- Matches snippet names and commands
- Score-based ranking
- Debounced updates (150ms) to avoid excessive recomputation

## Themes (`ui/theme.rs`)

Halloy-compatible TOML themes with a 10-color palette:

- 50 bundled themes (generated explicitly from `themes/` by `scripts/build_themes.py`)
- Extracted to `~/.config/snp/themes/` on first launch
- Active theme persisted in `~/.config/snp/themes.toml`
- Default theme: `Cyber Red` (hardcoded fallback)

### Theme Picker

Press `e` in normal mode to open the theme picker:
- `j`/`k` (or arrow keys) to preview themes live
- `i` to filter themes by name
- `Enter` to save selection
- `e`/`q`/`Esc` to cancel

### Legacy Fallback

- `SNP_THEME` env var (`dark`/`bright`/`light`/`auto`)
- Built-in `DARK_THEME` and `BRIGHT_THEME` constants
- `COLORFGBG` auto-detection

Colors: `primary`, `secondary`, `accent`, `background`, `text`, `border`, `selected_bg`, `muted`, `string_color`, `escape_color`

Resolved via `get_theme()` which reads from a process-global `RwLock<Theme>`.

## Syntax Highlighting (`ui/highlight.rs`)

`highlight_command()` tokenizes the command string and applies colors:
- **Variables** (`<name>`) — Accent color
- **Shell keywords** (~190 commands) — Primary color
- **Strings** (quoted) — Green
- **Flags** (`--flag`) — Secondary color
- **Comments** (`# ...`) — Muted
- **Escape sequences** (`\n`, `\t`) — Magenta

Pre-computed once at startup, cached for draw loop performance.

## Variable Prompt (`ui/variables.rs`)

`prompt_variables()` shows a TUI dialog for entering variable values:
- Shows variable name and default
- Editable text field
- Keyboard navigation (arrows, tab, enter)
- `q` to return to snippet selector, `Ctrl+C` to quit

Returns `VariablePromptResult::Cancel | Back | Skip | Values(...)`.

## Keybindings

### Normal Mode

| Key | Action |
|-----|--------|
| `h` / `←` | Move up |
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `l` / `→` | Move down |
| `gg` | Jump to top |
| `G` | Jump to bottom |
| `v` | Toggle visual mode |
| `V` | Visual line mode |
| `y` | Copy selected to clipboard |
| `d` | Open delete confirmation; `y` confirms and any other key cancels |
| `/` | Start incremental search |
| `t` | Tag filter mode |
| `n` | Sort by newest |
| `o` | Sort by oldest |
| `a` | Sort alphabetically |
| `z` | Sort alphabetically (descending) |
| `x` / `c` | Clear filter |
| `Tab` | Toggle display mode |
| `e` | Open theme picker |
| `q` / `Ctrl+c` | Quit |
| `Enter` | Select/execute |

### Insert Mode

| Key | Action |
|-----|--------|
| `↑` / `↓` | Navigate list |
| `Esc` | Return to normal mode |
| `/` | Start search |
| `Enter` | Select/execute |
| `Ctrl+f` / `Ctrl+d` / `PageDown` | Page down |
| `Ctrl+b` / `Ctrl+u` / `PageUp` | Page up |
| `Tab` | Toggle display mode |

The `d` key is ordinary filter input in insert mode. Deleting a snippet marks
it as a hidden tombstone in its library TOML so an enabled sync can propagate
the deletion to other devices. The selector reloads the library after the
operation so the deleted snippet disappears immediately.

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

# TUI Module

[← Back to Overview](overview.md)

## File

**`src/ui/`** (~1400 lines across submodules)

The largest single file in the codebase. Implements the terminal user interface using `ratatui` with `crossterm` as the backend.

## Architecture

### Main Loop

`select_snippet_inner()` is a single-loop event-driven TUI:

1. Initialize terminal with mouse capture
2. Pre-compute syntax-highlighted commands (once, outside draw loop)
3. Enter event loop:
   - Compute filtered candidates (debounced at 150ms)
   - Draw UI (filter input, list, preview, status bar)
   - Poll for events (keyboard, mouse)
   - Handle input in insert or normal mode
4. Return selected index on exit

### Mode System

Two primary modes, vim-inspired:

| Mode | Behavior |
|------|----------|
| **Insert** | Characters type into filter, Enter selects, Esc transitions to normal |
| **Normal** | Keystrokes trigger actions (y=copy, q=quit, j/k=navigate) |

Additional states:
- **Visual mode** (`v`/`V`) — Select multiple items, batch copy
- **Tag filter mode** (`t`) — Filter by tag instead of description

### Rendering Layout

```
┌─────────────────────────────────────┐
│ Filter Input Box (3 lines)          │
├─────────────────────────────────────┤
│ Snippet List (scrollable)           │  ← Pre-computed highlighted items
│ ▶ [description] command...          │
│   [description] command...          │
├─────────────────────────────────────┤
│ Preview Panel (6 lines)             │  ← Shows selected snippet + variables
│ Description: ...                    │
│ Command: ...                        │
│ Vars: name, host                    │
├─────────────────────────────────────┤
│ Status Bar (1 line)                 │  ← Mode, keybindings, messages
│ [INS] | i: insert | y: copy | ...  │
└─────────────────────────────────────┘
```

### Syntax Highlighting

`highlight_command()` tokenizes shell commands into styled spans:

| Token Type | Color | Examples |
|------------|-------|----------|
| Shell keywords | Primary (blue) | `git`, `docker`, `curl`, `ssh` |
| Variables | Accent (yellow) | `<name>`, `<host=default>` |
| Strings | Green | `'hello'`, `"world"` |
| Flags | Secondary (cyan) | `--verbose`, `-f` |
| Escape sequences | Magenta | `\n`, `\t` |
| Comments | Muted (gray) | `# comment` |
| Default | Text (white) | Everything else |

Keywords are defined in `src/utils/shell_keywords.rs` — a `HashSet` of ~190 common CLI tools.

### Theme System

Halloy-compatible TOML themes with a 10-color projection:

| Component | Description |
|-----------|-------------|
| `Theme` struct | 10-color palette: primary, secondary, accent, background, text, border, selected_bg, muted, string_color, escape_color |
| `DARK_THEME` | Built-in dark fallback (legacy `SNP_THEME=dark`) |
| `BRIGHT_THEME` | Built-in bright fallback (legacy `SNP_THEME=bright`) |
| `ACTIVE_THEME` | `RwLock<Theme>` — process-global, reloaded on demand |

**Halloy themes**: 50 bundled themes in `themes/`, LZMA-compressed at build time into `_generated_bundled_themes.rs`, extracted to `~/.config/snp/themes/` on first launch. Active theme persisted in `~/.config/snp/themes.toml`.

**Theme picker**: Press `e` in normal mode; `j`/`k` to preview, `i` to filter, `Enter` to save, `e`/`q`/`Esc` to cancel.

**Legacy**: `SNP_THEME` env var (`dark`/`bright`/`light`/`auto`) and `COLORFGBG` auto-detection still work.

### Fuzzy Matching

Uses `fuzzy-matcher` crate (skim algorithm) via lazy-static `MATCHER`. Filtering is debounced at 150ms to avoid excessive computation on every keystroke.

### Mouse Support

- Scroll wheel: Navigate up/down
- Single click: Select item
- Double-click: Run/execute selected snippet (500ms window)

### Variable Prompting

`prompt_variables_inner()` renders a separate TUI for entering variable values when a snippet contains `<name>` or `<name=default>` syntax. Variables are filled one at a time with Tab navigation.

## Key Behaviors

- **Debounced filtering** — Filter updates are delayed 150ms to batch rapid keystrokes
- **Pre-computed highlights** — Syntax highlighting computed once at startup, not per frame
- **Double-buffered filter** — `input_text` (what user types) vs `filter` (applied filter) are separate
- **Mouse capture** — Enabled on init, disabled on exit
- **Terminal size check** — Shows error if terminal < 10x10

## Integration Points

- `src/clipboard.rs` — Copies selected snippet to clipboard
- `src/utils/variables.rs` — Parses `<var>` syntax for display and expansion
- `src/utils/shell_keywords.rs` — Keyword list for syntax highlighting
- `src/commands/mod.rs` — `expand_snippet_command()` calls `prompt_variables()`
- `src/ui/state.rs` — SelectState, FilterState, SortMode types

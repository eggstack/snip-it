# UI Module Architecture

## Module Structure
```
src/ui/
├── mod.rs          # Main TUI loop, re-exports, FilterState, SortMode
├── theme.rs        # Theme system (dark/bright), ACTIVE_THEME, get_theme()
├── highlight.rs    # Syntax highlighting for snippet commands
└── variables.rs    # Variable prompting UI (prompt_variables_inner)
```

## Key Types
- `Theme` — Color palette (Copy, Clone), dark or bright
- `VariablePromptResult` — User's response to variable prompts
- `FilterState` — Debounced filter with 150ms delay
- `SortMode` — Command sort order (alphabetical, recently used)

## TUI Loop (select_snippet_inner)
1. Initialize terminal with `ratatui::Terminal`
2. Event loop: poll `crossterm::event::read()`
3. Handle key events (Ctrl+C, j/k navigation, Enter, etc.)
4. Handle mouse events (click, scroll)
5. Draw UI on each frame

## Syntax Highlighting
- Pre-computed once at startup (not in draw loop)
- Uses `SHELL_KEYWORDS_SET` for keyword detection
- Uses `.contains()` not `.iter().any()` for O(1) lookup
- Colors: keyword, string, comment, operator, default

## Theme System
- `SNP_THEME` env var controls theme (default: "dark")
- `ACTIVE_THEME: Mutex<Theme>` stores current theme
- `get_theme()` returns current theme reference
- `style_fg()` / `style_fg_bg()` helpers for styled text

## Dependencies
- `ratatui` for terminal UI
- `crossterm` for terminal events
- `fuzzy-matcher` (skim algorithm) for filtering

# UI Module Architecture

## Module Structure
```
src/ui/
├── mod.rs                       # Main TUI loop, re-exports
├── state.rs                     # SelectState, FilterState, SortMode, is_ctrl_key
├── theme.rs                     # Theme system (Halloy TOML + dark/bright fallback)
├── highlight.rs                 # Syntax highlighting for snippet commands
├── variables.rs                 # Variable prompting UI (prompt_variables_inner)
└── _generated_bundled_themes.rs # LZMA-compressed bundled themes (build-time generated)
```

## Key Types
- `Theme` — 10-color palette (Copy, Clone): primary, secondary, accent, background, text, border, selected_bg, muted, string_color, escape_color
- `SelectState` — Selected index, list state, scroll state (in `state.rs`)
- `FilterState` — Sort mode and tag filter text (in `state.rs`)
- `SortMode` — None, Newest, Oldest, AlphaAsc, AlphaDesc
- `VariablePromptResult` — User's response to variable prompts

## TUI Loop (select_snippet_inner)
1. Initialize terminal with `ratatui::Terminal`
2. Event loop: poll `crossterm::event::read()`
3. Handle key events (Ctrl+C, j/k navigation, Enter, etc.)
4. Handle mouse events (click, scroll)
5. Draw UI on each frame
6. `TerminalGuard` RAII ensures terminal restore on drop/panic

## Syntax Highlighting
- Pre-computed once at startup (not in draw loop)
- Uses `SHELL_KEYWORDS_SET` for keyword detection (O(1) via `HashSet::contains()`)
- Colors: keyword, string, comment, operator, escape, default
- Theme-aware: string and escape colors adapt to active theme

## Theme System

### Halloy TOML Themes
- 50 bundled themes in `themes/` directory (source of truth)
- LZMA-compressed and base64-encoded at build time into `_generated_bundled_themes.rs`
- `build.rs` re-invokes `scripts/build_themes.py` when themes/ is newer
- Extracted to `~/.config/snp/themes/` on first launch
- Active theme persisted in `~/.config/snp/themes.toml`
- Default theme: `Cyber Red` (hardcoded fallback)

### Theme Picker
- Press `e` in normal mode to open theme picker
- `j`/`k` (or arrow keys) to preview themes live
- `i` to filter themes by name
- `Enter` to save selection
- `e`/`q`/`Esc` to cancel

### Legacy Fallback
- `SNP_THEME` env var still works (`dark`/`bright`/`light`/`auto`)
- Built-in `DARK_THEME` and `BRIGHT_THEME` used as fallback
- `COLORFGBG` auto-detection still supported

### Helpers
- `get_theme()` returns current theme reference (from `RwLock<Theme>`)
- `style_fg()` / `style_fg_bg()` helpers for styled text

## Dependencies
- `ratatui` for terminal UI
- `crossterm` for terminal events
- `fuzzy-matcher` (skim algorithm) for filtering
- `lzma-rs` for decompressing bundled themes

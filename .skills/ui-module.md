# UI Module Architecture

Deep dives: [`architecture/ui.md`](../architecture/ui.md) (theme, highlight,
variables) and [`architecture/tui.md`](../architecture/tui.md) (event loop,
keyboard map). This skill carries only the rules you need while changing code.

## Module Structure
```
src/ui/
├── mod.rs                       # Main TUI loop, re-exports
├── state.rs                     # SelectState, FilterState, SortMode, is_ctrl_key
├── theme.rs                     # Theme system (Halloy TOML + dark/bright fallback)
├── highlight.rs                 # Syntax highlighting for snippet commands
├── variables.rs                 # Variable prompting UI (prompt_variables_inner)
└── _generated_bundled_themes.rs # gzip-compressed bundled themes (explicitly generated)
```

## Key Types
- `Theme` — 10-color palette (Copy, Clone): primary, secondary, accent, background, text, border, selected_bg, muted, string_color, escape_color
- `SelectState` — `selected` index + `scroll_state` (`pub(super)` in `state.rs`, internal to the UI module)
- `FilterState` — Sort mode and tag filter text (in `state.rs`)
- `SortMode` — None, Newest, Oldest, AlphaAsc, AlphaDesc, LastUsed, MostUsed, Command (initial-only, from `--sort command`; never produced by interactive toggles — `src/ui/state.rs:4-16`)
- `VariablePromptResult` — User's response to variable prompts

## TUI Loop (select_snippet_inner)
1. Initialize terminal with `ratatui::Terminal`
2. Event loop: poll `crossterm::event::read()`
3. Handle key events (Ctrl+C, j/k navigation, Enter, etc.)
4. Handle mouse events (click, scroll)
5. Draw UI on each frame
6. `TerminalGuard` RAII ensures terminal restore on drop/panic

## Syntax Highlighting
- **Lazy and budgeted**, not pre-computed at startup: `warm_visible_highlights` fills the
  highlighted lines for the currently visible rows during the draw loop, under a
  2 ms/frame budget (`HIGHLIGHT_WARM_BUDGET`, `src/ui/mod.rs:749`, called at
  `src/ui/mod.rs:1365-1372`)
- Uses `SHELL_KEYWORDS_SET` for keyword detection (O(1) via `HashSet::contains()`)
- Colors: keyword, string, comment, flag, variable, escape, default
- Theme-aware: string and escape colors adapt to active theme
- Highlighting is **lossy**: `\<` renders as `<` and the backslash is dropped
  (`src/ui/highlight.rs:65-72`)

## Theme System

### Halloy TOML Themes
- 50 bundled themes in `themes/` directory (source of truth)
- gzip-compressed and base64-encoded by the explicit `scripts/build_themes.py` generator into `_generated_bundled_themes.rs`
- Normal builds consume the checked-in generated file; there is no root `build.rs` hook
- Seeded into `~/.config/snp/themes/` lazily, when the theme picker is first
  opened (`ensure_theme_picker_ready` → `ThemeManager::init_themes_dir`,
  `src/ui/mod.rs:529-540`) — seeding is idempotent and never overwrites a
  user-edited file
- Active theme persisted in `~/.config/snp/themes.toml`
- Default theme: `Cyber Red` (hardcoded fallback)

### Theme Picker
- Press `e` in normal mode to open theme picker (`src/ui/mod.rs:2028-2030`)
- `j`/`k` (or arrow keys) to preview themes live
- `i` to filter themes by name (enters the picker's insert mode)
- `Enter` to save selection
- `e`/`q`/`Esc` to cancel — in the picker's *normal* mode; in insert mode `Esc`
  only returns to normal mode (`src/ui/mod.rs:1462-1466`)
- **Known limitation:** in the picker's insert mode the `j`/`k` arms are matched
  before the generic character arm, so typing a filter that contains `j` or `k`
  usually moves the preview instead of inserting the letter (they only insert at
  the list boundary) (`src/ui/mod.rs:1467-1479`, `src/ui/mod.rs:1480-1490`)

### Legacy Fallback
- `SNP_THEME` env var still works (`dark`/`bright`/`light`/`auto`)
- Built-in `DARK_THEME` and `BRIGHT_THEME` used as fallback
- `COLORFGBG` auto-detection still supported

### Helpers
- `get_theme()` returns a `Theme` snapshot by value (`Copy`) from the `ACTIVE_THEME` lock (`src/ui/theme.rs:639-644`)
- `style_fg()` / `style_fg_bg()` helpers for styled text

## Callers (don't break)

- Any function moved out of `ui/mod.rs` must be re-exported there — `commands/` calls through the re-exports (`AGENTS.override.md`)
- `commands/mod.rs` owns `load_snippets` / `save_snippets` / `run_snippet_selection(Option<&Runtime>)` (`None` when `do_sync` is false); clipboard side effects go only through `copy_to_clipboard()` in `clip_cmd.rs`
- Search shares `selector::searchable_text` across `snp get --query`, `snp list --filter`, and MCP `snippets_search` — keep parity

## Dependencies
- `ratatui` for terminal UI
- `crossterm` for terminal events
- `fuzzy-matcher` (skim algorithm) for filtering
- `flate2` for decompressing bundled themes

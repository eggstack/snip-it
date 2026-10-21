# TUI Selector (`src/ui/` event loop)

[← Back to Overview](overview.md)

## Overview

The interactive snippet selector is a single-loop, event-driven TUI built on
`ratatui` 0.30 + `crossterm` 0.29 (`src/ui/mod.rs`, `select_snippet_inner`).
It owns fuzzy filtering (`SkimMatcherV2`), keyboard/mouse navigation, sort
mode, the visual-range multi-select, the delete-confirmation dialog, and the
theme-picker overlay. It never touches the library file directly — the only
destructive signal it can emit is `SnippetSelection::Delete`, which the caller
honours through a tombstone.

This doc covers the selector loop only. Appearance and syntax colouring live
in [ui.md](ui.md); the variable-prompt dialog it opens lives in
`src/ui/variables.rs` and is described in [Variable prompt flow](#variable-prompt-flow-srcuivariablesrs)
below; deterministic non-interactive resolution lives in
[selector.md](selector.md); clipboard mechanics in [clipboard.md](clipboard.md).

## Entry points and public surface

| Item | Location | Purpose |
|------|----------|---------|
| `SnippetListParams` | `src/ui/mod.rs:499` | Borrowed view of the library (descriptions, commands, tags, folders, favorites, `snippets`, `original_indices`, `sort_opts`, `usage`, `is_search`, `initial_filter`, `allow_delete`) |
| `SnippetSelection` | `src/ui/mod.rs:516` | `Selected(usize, Option<String>)` / `Copied` / `Delete(usize)` / `Cancelled` |
| `select_snippet` | `src/ui/mod.rs:527` | Public wrapper; tty guard, then `select_snippet_inner` (`:545`) |
| `run_snippet_selection` | `src/commands/mod.rs:338` | Sole caller of `select_snippet`; loads library + usage, loops the TUI, handles `Delete`/`Copied`/sync |
| Re-exports | `src/ui/mod.rs:16-17` | `pub use theme::get_theme;` and `pub use variables::{VariablePromptResult, prompt_variables};` |

The real signatures:

```rust
// src/ui/mod.rs:527
pub fn select_snippet(params: SnippetListParams) -> io::Result<Option<SnippetSelection>>;

// src/ui/mod.rs:516
pub enum SnippetSelection {
    Selected(usize, Option<String>),
    Copied,
    Delete(usize),
    Cancelled,
}
```

`select_snippet` has exactly one call site (`src/commands/mod.rs:403`). Nothing
else in the tree constructs `SnippetListParams`, so the TUI's inputs are fully
determined by `run_snippet_selection`.

### Re-export contract

`src/ui/mod.rs:16-17` re-exports **exactly two** items: `theme::get_theme` and
`variables::{VariablePromptResult, prompt_variables}`. `src/ui/mod.rs` also
re-exports `get_terminate` (`src/ui/mod.rs:62`) and the `SnippetListParams` /
`SnippetSelection` types.

`src/commands/mod.rs` (`load_snippets` / `save_snippets` /
`run_snippet_selection`) and `src/ui/mod.rs` re-exports affect **all** TUI
commands (`run`, `clip`, `search`, `select`). Widening or renaming this
surface is a breaking change for every caller; extend inside
`theme.rs` / `variables.rs` instead.

## Terminal lifecycle

### Setup

1. **tty guard** (`src/ui/mod.rs:533`) — `select_snippet` returns
   `io::ErrorKind::Unsupported` unless *both* stdin and stdout are terminals.
   Without this guard `ratatui::init()` panics and the process aborts with
   `SIGABRT` (exit 134) on piped stdio. The error text points at
   `snp get --query <text> --field command` and `snp list --json` as the
   non-interactive spellings. Added in commit `2aaae33`
   ("Make README quickstart-first and fix what it got wrong"); covered by
   `tests/tui_requires_terminal.rs`.
2. **Mouse capture** (`src/ui/mod.rs:562`) — `EnableMouseCapture` is
   best-effort: failure logs a warning and continues without mouse support
   (headless SSH).
3. **`ratatui::init()`** (`src/ui/mod.rs:568`) — in ratatui 0.30.2 this
   enables raw mode, enters the alternate screen buffer, and installs a panic
   hook that chains to the previously installed one
   (`ratatui-0.30.2/src/init.rs:398`, `:566-572`).
4. **`TerminalGuard`** (`src/ui/mod.rs:42-53`) — its `Drop` disables mouse
   capture (warn-only) and calls `ratatui::restore()`, which disables raw mode
   and leaves the alternate screen. The guard is bound immediately after
   `init()` (`src/ui/mod.rs:569`), so every early return and every panic
   unwinds through it.

### Teardown

| Path | Behaviour |
|------|-----------|
| Normal exit | `TerminalGuard::drop` → `DisableMouseCapture` + `ratatui::restore()` |
| Panic | The same `Drop` runs during unwind, *and* ratatui's own panic hook restores the terminal before delegating to snip-it's hook |
| Process panic | `src/logging.rs:264-272` `setup_panic_handler` explicitly disables mouse capture and calls `ratatui::restore()` before logging and flushing |
| SIGINT / SIGTERM | `TERMINATE` is set by the `signal_hook` handler registered in `src/main.rs:35-44`; the loop breaks and returns `SnippetSelection::Cancelled` (`src/ui/mod.rs:660-662`, `:1912-1914`) |

`TERMINATE` is a process-global `LazyLock<Arc<AtomicBool>>`
(`src/ui/mod.rs:59-60`); `get_terminate()` (`src/ui/mod.rs:62`) hands the same
handle to `src/main.rs` and to `src/ui/variables.rs:262`.

## The event loop

`select_snippet_inner` (`src/ui/mod.rs:545-1926`) is a single `loop` whose
body is, in order:

1. **Termination check** (`:660`) — `TERMINATE.load(...)` → break.
2. **Copy-message expiry** (`:664-670`) — the "N snippets copied" banner is
   dropped after 3 s and the frame is marked dirty.
3. **Lazy theme-picker init** (`:672-701`) — the picker only builds a
   `ThemeManager` on first use; the snapshot `theme_picker_original` is taken
   here so cancel can restore even when nothing was ever persisted. If init
   fails, the picker is abandoned with a warning and the loop degrades to the
   plain selector (`:680-686`).
4. **Re-highlight on theme change** (`:703-708`) — `pending_rehighlight`
   clears the whole `highlighted_commands` cache (`fill_with(|| None)`).
5. **Theme-filter debounce** (`:710-734`) — 100 ms
   (`THEME_DEBOUNCE_MS`, `:627`); the picker's name filter is fuzzy-matched
   with the same `MATCHER` (`:720`).
6. **Snippet-filter recompute** (`:736-801`) — 35 ms
   (`FILTER_DEBOUNCE_MS`, `:633`). Clearing the filter recomputes
   *immediately* so stale rows never linger (`:739-742`).
7. **Layout computation** (`:803-824`) — terminal size, list area, and the
   visible-row window are recomputed every frame.
8. **Draw** (`:825-1269`) — only when `needs_redraw` is set (`:825-826`).
9. **Highlight warm-up** (`:1271-1289`) — only when no input is pending
   (`event::poll(0)`) and only for the visible rows, under a 2 ms budget
   (`HIGHLIGHT_WARM_BUDGET`, `:634`).
10. **Event poll** (`:1290-1297`) — `event::poll(timeout)` then
    `event::read()` (`:1299`), handling mouse, key, and resize events.

### Redraw strategy

Rendering is **pull-based and dirty-flagged**, not continuous:

- `needs_redraw` (`src/ui/mod.rs:655`) is set by the filter recompute
  (`:798`), theme changes (`:699`, `:733`), copy-message expiry (`:669`),
  mouse events (`:1302`), and terminal resize (`:1903`).
- `terminal.draw(...)` runs only when the flag is set (`:825-826`).
- The draw closure snapshots the theme once via `get_theme()`; the draw path
  performs no I/O and no locking beyond that single read-lock.
- `next_event_poll_timeout` (`src/ui/mod.rs:311-327`) computes the poll
  deadline as the minimum of the pending filter and theme debounce windows,
  falling back to 200 ms when nothing is pending (`:326`). The loop therefore
  wakes exactly when the next scheduled recompute is due.
- If the terminal is smaller than 10×10 the frame is replaced by an error
  paragraph (`:850-857`).
- Draw errors are logged, not propagated (`:1267-1269`).

### Lazy caches

`highlighted_commands: Vec<Option<Line<'static>>>` and
`snippet_previews: Vec<Option<SnippetPreview>>` are allocated once
(`src/ui/mod.rs:573-574`) and filled on demand, so a large library pays no
syntax-highlighting cost before the first frame. `command_line_for_display`
(`:329-343`) returns the cached line or falls back to unstyled text;
`warm_visible_highlights` (`:345-371`) fills the cache under the frame
budget.

## Fuzzy matching (`SkimMatcherV2`)

- `MATCHER: LazyLock<SkimMatcherV2>` (`src/ui/mod.rs:66`) — one shared
  matcher, never reconstructed.
- `FilterRequest` (`src/ui/mod.rs:68-73`) carries `text`, `text_lower`, and
  `include_tags`. `current_filter_request` (`:87-111`) picks the main filter
  text or the tag-filter text depending on `tag_filter_mode`, and sets
  `include_tags` when the tag filter is active.
- `rebuild_filter_candidates` (`:138-171`):
  - exact match of the lowercased display string → `Some(i64::MAX)` (`:157-159`);
  - tag substring match → `Some(i64::MAX)` (`:160-163`);
  - otherwise `MATCHER.fuzzy_match(display, &request.text)` (`:167-168`);
  - out-of-range indices are skipped, never panicked on (`:154-156`).
- **Incremental narrowing**: `FilterRequest::can_narrow_from` (`:80-84`)
  requires the previous text to be non-empty, the tag scope to be unchanged,
  and the new text to *extend* the old one. When it holds, the previous
  candidate set is reused as the scan source (`:757-768`); otherwise the
  filter rescans every row (`:769-778`).
- **Debounce helpers**: `debounce_elapsed` (`:293-295`),
  `pending_debounce_timeout` (`:297-309`),
  `filter_request_would_narrow` (`:113-121`),
  `filter_update_deadline_for_insert` (`:123-136`).

### How this differs from `selector::searchable_text`

The TUI does **not** use the canonical search-text builder. The two surfaces
have different corpora and different scoring:

| | TUI filter (`src/ui/mod.rs:641-651`) | `selector::searchable_text` (`src/selector.rs:135`) |
|---|---|---|
| Description | included, as `"[desc]: cmd"` | included, plain |
| Command | included, same string | included, plain |
| Tags | **separate** channel: `tags.join("\n").to_lowercase()`, matched by **substring only**, and only when `include_tags` is set | joined into one string and **fuzzy-scored** when `include_tags` (`SearchFields::fuzzy_default()`, `src/selector.rs:117-119`) |
| Output / notes | never searched | searchable only with `SearchFields::with_output()` (`src/selector.rs:122-127`) |
| Folders, favorite, sync metadata, credentials | never searched | never searched |
| Scoring | `SkimMatcherV2` over the display string; exact display and tag hits are pinned to `i64::MAX` | `SkimMatcherV2` over the canonical text |

Consequences:

- The parity contract that binds `snp get --query`, `snp list --filter`, and
  MCP `snippets_search` to one function does **not** include the TUI. See
  [selector.md](selector.md) and `.skills/selector-and-search-parity.md`.
- `snp search` opens this TUI (`src/commands/search_cmd.rs:58-74`), so its
  `--filter` is a *client-side* filter over the already-loaded library, not
  the canonical query path. The canonical path is `snp get --query`.
- The TUI matches a tag only when the tag filter mode is active (`t`) or a
  tag filter text is present (`:109`), and it is a plain substring test, not
  a fuzzy match.

### Sort order

`sort_filtered_indices` (`src/ui/mod.rs:173-291`) is a no-op fast path when
the sort mode is `None`, no filter is active, and favorites-first is off
(`:183-185`). Otherwise the comparator layers:

1. **Favorites group** (`:189-196`) — an orthogonal grouping modifier, not a
   sort mode; enabled by `--favorites-first` (`:596`).
2. **Fuzzy score** (`:198-203`) — descending; scored rows sort above unscored
   rows.
3. **Explicit mode** (`:206-265`) — `Newest` / `Oldest` on `updated_at`
   (descending / ascending, index tiebreak), `LastUsed` on
   `usage.last_used_at` (missing values last), `MostUsed` on `use_count` then
   `last_used_at`.
4. **Secondary key** (`:267-281`) — `AlphaAsc` / `AlphaDesc` on the lowercased
   display string, `Command` on the lowercased command text.

### `--sort` mapping

| CLI (`crate::sort::SnippetSort`) | `SortMode` | Ordered by |
|---|---|---|
| `Relevance` | `None` (`:582`) | Fuzzy score only |
| `Recent` | `Newest` (`:583`) | `updated_at` desc |
| `LastUsed` | `LastUsed` (`:584`) | `usage.last_used_at` desc, missing last |
| `MostUsed` | `MostUsed` (`:585`) | `use_count` desc, then `last_used_at` desc |
| `Description` | `AlphaAsc` (`:586`) | Lowercased `[desc]: cmd` display |
| `Command` | `Command` (`:587`) | Lowercased command text — **initial-only** |

`SortMode::Command` (`src/ui/state.rs:13-15`) is never produced by the
interactive toggles; it can only arrive via `--sort command`.

## Selection state machine

### Types (`src/ui/state.rs`)

| Type | Location | Notes |
|------|----------|-------|
| `SortMode` | `state.rs:4-16` | `None` (default), `Newest`, `Oldest`, `AlphaAsc`, `AlphaDesc`, `LastUsed`, `MostUsed`, `Command` |
| `SelectState` | `state.rs:19-22` | `selected` + `scroll_state`; `update` clamps on filter shrink (`:32`); `move_up` saturates (`:44`), `move_down` bounds-checks (`:48`), `move_to_top`/`move_to_bottom` (`:54`, `:58`) |
| `FilterState` | `state.rs:64-67` | `sort_mode` + `tag_filter_text`; `toggle_sort_new`/`_old`/`_alpha`/`_alpha_rev` (`:70`, `:77`, `:84`, `:91`) each cycle back to `None`; the last-used / most-used toggles exist but are `#[allow(dead_code)]` (`:98`, `:106`) |
| `is_ctrl_key` | `state.rs:116-121` | Ctrl-combination predicate used by paging and quit keys |

`SelectState`, `FilterState`, and `SortMode` are `pub(super)` — they are
internal to the `ui` module and are **not** part of the public surface.

### Modes

The loop has five mutually exclusive interaction modes, checked in this order:

| Mode | Flag | Entered by | Left by |
|------|------|-----------|-----------|
| Theme picker | `theme_picker_mode` (`:612`) | `e` in normal mode (`:1891-1895`) | `Enter` (commit), `e`/`q`/`Esc` (cancel) |
| Delete confirmation | `delete_confirmation` (`:603`) | `d` in normal mode when `allow_delete` (`:1789-1795`) | `y` confirms (`:1557-1562`); **any other key cancels** (`:1563`) |
| Visual range | `visual_mode` (`:604`) | `v` (`:1574-1583`) or `V` (`:1585-1595`) in normal mode | `y` (copy), `Esc` (`:1597-1600`) |
| Insert | `insert_mode` (`:598`, **default**) | `i` (`:1776-1779`), `/` (`:1881-1887`) | `Esc` (`:1697-1714`) |
| Normal | `insert_mode == false` | `Esc` from insert mode | `i` / `/` |

The delete-confirmation and theme-picker branches are checked *before* the
visual-mode branch, so they take precedence.

`Esc` in normal mode is a **no-op** (`:1888-1890`); `q` and `Ctrl+C` quit.

### Exit mapping

| Condition | Result |
|-----------|--------|
| `TERMINATE` set | `SnippetSelection::Cancelled` (`:1912-1914`) |
| `filtered.is_empty()` | `Ok(None)` (`:1915-1917`) — the caller treats `None` as cancelled (`src/commands/mod.rs:477-480`) |
| `sel.selected < filtered.len()` | `SnippetSelection::Selected(filtered[sel.selected], should_copy)` (`:1918-1922`) |
| otherwise (`q` / `Ctrl+C` set `selected = filtered.len()`) | `SnippetSelection::Cancelled` (`:1923-1925`) |

## Keyboard map

Keys are handled in this precedence order: theme picker → delete confirmation
→ `Ctrl+C` → visual-mode entry (`v`/`V`) → visual-mode `Esc` → visual-mode
body → paging → insert mode → normal mode.

### Global

| Key | Action | Location |
|-----|--------|----------|
| `Ctrl+C` | Set `selected = filtered.len()` → cancel | `:1566-1569` |
| `Ctrl+f` / `Ctrl+d` / `PageDown` | Page down 10 rows | `:1680-1686` |
| `Ctrl+b` / `Ctrl+u` / `PageUp` | Page up 10 rows | `:1688-1693` |
| `Tab` | Toggle list display mode (`0` = command only, `1` = `[description]` + command) | `:1758-1760`, `:1796-1798` |

Paging is applied **before** the insert/normal split, so it works in both
modes.

### Insert mode (default)

| Key | Action | Location |
|-----|--------|----------|
| printable chars | Append to `input_text` *and* `filter` | `:1737-1757` |
| `Backspace` | Pop the last char; jump the cursor to the top | `:1722-1736` |
| `↑` / `↓` | Move the selection | `:1715-1718` |
| `Enter` | Leave the loop with the current selection | `:1719-1721` |
| `Esc` | Commit `input_text` to `filter` and switch to normal mode | `:1697-1714` |
| `Esc` with empty input | Clear the filter and switch to normal mode | `:1706-1713` |
| `Esc` in tag-filter mode | Leave tag-filter mode | `:1698-1699` |

In insert mode every printable character is inserted — including `j`, `k`,
`d`, `y`, `v`, `V`, `t`, `n`, `o`, `a`, `z`, `x`, `c`, `e`. The destructive
and mode-switching keys are **normal-mode only**.

### Normal mode

| Key | Action | Location |
|-----|--------|----------|
| `h` / `←` | Move up | `:1824-1826` |
| `j` / `↓` | Move down | `:1827-1831` |
| `k` / `↑` | Move up | `:1832-1834` |
| `l` / `→` | Move down | `:1835-1839` |
| `Ctrl+g` | Jump to top | `:1799-1802` |
| `g` `g` | Jump to top, 500 ms window (`GG_TIMEOUT_MS`, `:609`) | `:1803-1822` |
| `G` | Jump to bottom | `:1823` |
| `Enter` | Leave the loop with the current selection | `:1772-1775` |
| `i` | Enter insert mode (pre-fills `input_text` from `filter`) | `:1776-1779` |
| `y` | Copy the highlighted snippet's **description** into `should_copy` and leave the loop | `:1780-1788` |
| `d` | Arm delete confirmation (only when `allow_delete`) | `:1789-1795` |
| `v` | Enter visual mode at the cursor | `:1574-1583` |
| `V` | Enter visual mode spanning to the last row | `:1585-1595` |
| `/` | Enter insert mode with the filter cleared | `:1881-1887` |
| `t` | Toggle tag-filter mode | `:1873-1880` |
| `n` / `o` | Sort newest / oldest | `:1840-1853` |
| `a` / `z` | Sort a-z / z-a | `:1854-1865` |
| `x` / `c` | Clear filter, tag filter, and tag-filter mode | `:1866-1872` |
| `e` | Open the theme picker | `:1891-1895` |
| `q` | Cancel | `:1768-1771` |
| `Esc` | No-op | `:1888-1890` |

The four sort keys (`n`, `o`, `a`, `z`) clear `visual_mode` first, because
sorting reindexes `filtered` and a stale visual anchor would select the wrong
range (`:1841-1843`).

### Visual mode

| Key | Action | Location |
|-----|--------|----------|
| `j` / `↓` | Extend the range downward | `:1604-1614` |
| `k` / `↑` | Extend the range upward | `:1615-1622` |
| `y` | Copy the range (see below) | `:1623-1673` |
| `Esc` | Leave visual mode without copying | `:1597-1600` |

`v` and `V` are ignored on an empty list (`:1577`, `:1588`).

### Theme picker

| Key | Action | Location |
|-----|--------|----------|
| `i` | Filter themes by name (picker insert mode) | `:1434-1437` |
| `j` / `↓` | Next theme, live preview | `:1478-1490`, `:1376-1388` |
| `k` / `↑` | Previous theme, live preview | `:1491-1501`, `:1389-1399` |
| `Ctrl+g` | First theme | `:1502-1512` |
| `G` | Last theme | `:1513-1523` |
| `Ctrl+d` / `PageDown` | Page down 10 | `:1524-1537` |
| `Ctrl+u` / `PageUp` | Page up 10 | `:1538-1550` |
| `Enter` | Persist the selection to `themes.toml` | `:1414-1429`, `:1462-1477` |
| `e` / `q` / `Esc` | Cancel and restore the pre-picker theme | `:1438-1461`, `:1371-1375` |

In the picker's **insert** mode `Esc` returns to the picker's normal mode
(`:1371-1375`); in the picker's **normal** mode `Esc` cancels the picker
entirely (`:1438`).

### Mouse

| Event | Action | Location |
|-------|--------|----------|
| Scroll down / up | Move the selection | `:1304-1312` |
| Left click on a row | Select it | `:1314-1349` |
| Double click (≤ 500 ms, `DOUBLE_CLICK_DURATION_MS`, `:639`) | Select and run | `:1340-1343` |

Clicks outside the list reset the double-click state (`:1350-1353`).

### Drift with `src/commands/keybindings_cmd.rs`

`src/commands/keybindings_cmd.rs` (85 lines) prints a static reference for the
same keymap. It is **documentation-as-code with no link to the loop** — the
strings are hand-written `println!`s, so the two can and do drift. Known
divergences at the time of writing:

| `keybindings_cmd.rs` claim | Reality |
|---|---|
| `y` : "copy and quit" (`:23`) | Correct for normal mode (`:1780-1788`); the separate **visual**-mode `y` (range copy) is undocumented |
| No mention of `Ctrl+C` in the selector's Normal Mode block (`:10-36`) | `Ctrl+C` cancels (`:1566-1569`); the file mentions `Ctrl+c` only in the Variable Prompt section (`:83`) |
| No mention of `Enter` in the selector's Normal Mode block (`:10-36`) | `Enter` leaves the loop with the current selection (`:1772-1775`) |
| Theme Picker cancel is `e/q` only (`:56`) | The picker's **normal** mode also cancels on `Esc` (`:1438`); the file documents `Esc` only as "leave filter" (`:57`) |

Earlier divergences have since been fixed upstream: the reference now states
that the selector starts in insert mode (`:7`), that `j`/`k` are literal in
insert mode (`:39`), that `Tab` toggles the list display mode (`:36`, `:44`),
and that the picker jumps to the first theme with `Ctrl+g` (`:53`).

Also note the theme picker's filter input cannot contain `j` or `k`: the
`Char('j')` / `Char('k')` arms (`:1376`, `:1389`) precede the catch-all
`Char(c)` arm (`:1408`), so those characters navigate instead of being typed.

When changing the keymap, update both `src/ui/mod.rs` and
`src/commands/keybindings_cmd.rs`; see
[commands/keybindings_cmd.md](commands/keybindings_cmd.md).

## Visual-range multi-select

Entering visual mode records `visual_start` / `visual_end` as indices into
the *filtered* list (`:1580-1581`, `:1592`). `j`/`k` extend the range
symmetrically (`:1604-1622`).

Pressing `y` in visual mode (`:1623-1673`):

1. Computes `start = min(visual_start, visual_end)` and
   `end = max(visual_start, visual_end)`, both clamped to the list (`:1631-1634`).
2. Collects **`strip_escape_sequences(&commands[*idx])`** for every row in the
   range (`:1635-1640`) — the copied text is the **command**, not the
   description. Rows are joined with `\n` (`:1641`).
3. Copies via **`clipboard::copy_to_clipboard_auto`** called **directly**
   (`:1642`) — this bypasses `copy_to_clipboard()` in `src/commands/clip_cmd.rs`,
   which is the documented single funnel for clipboard side effects. This is
   one of exactly **two** direct call sites in the tree; the other is
   `src/commands/cron_cmd.rs:89`.
4. **Audits only the first snippet in the range**: `filtered.get(start)`
   (`:1657`) is logged with `crate::logging::audit_log("copy", …)` (`:1660`).
   The remaining rows of the range are copied but never audit-logged.
5. Sets the "N snippets copied" banner (`:1644-1647`, shown for 3 s).
6. Leaves visual mode and — outside `is_search` — returns
   `SnippetSelection::Copied` directly (`:1670-1671`).

The `Copied` outcome is treated by the caller as "already processed": it ends
the loop without re-invoking `process_fn` (`src/commands/mod.rs:453-456`).

## Variable prompt flow (`src/ui/variables.rs`)

`src/ui/variables.rs` (1579 lines) owns the modal prompt for `<name>` and
`<name=default>` placeholders. It is a **different module** from
`src/utils/variables.rs` (1947 lines), which owns the parser and expander —
the two share names but not responsibilities:

| Module | Owns |
|---|---|
| `src/utils/variables.rs` | `parse_variables()` (`:296`), `parse_variables_diagnostics()` (`:333`), `expand_command()` (`:644`), `strip_escape_sequences()` (`:61`), `extract_variables_for_display()` (`:513`), `has_unmatched_angle_bracket()` (`:528`), and the `Variable` / `VariableKind` types (`:47`, `:33`) |
| `src/ui/variables.rs` | The modal dialog: `prompt_variables()` (`:78`), `prompt_variables_inner()` (`:211`), `Field` (`:88`), `PromptField` (`:156`), `VariablePromptResult` (`:56`) |

### Detection and substitution

1. `expand_snippet_command` (`src/commands/mod.rs:274-293`) calls
   `parse_variables(&snippet.command)` (`:278`).
2. With no variables, the command is returned with escape sequences stripped
   (`:280-282`) — no prompt is shown.
3. Otherwise `ui::prompt_variables(vars)` runs the modal dialog (`:285`).
4. On `VariablePromptResult::Values(values)`, `crate::utils::expand_command(&snippet.command, &values)`
   substitutes the values (`:289-291`).

`expand_command` uses a **second, independent scanner** alongside the token
extractor. If the two ever disagree about one token, the cursor is
**resynchronized** onto the next token with the same content
(`src/utils/variables.rs:724-744`, added in commit `8d5c209`), so a single
disagreement stays local instead of silently breaking every later variable
in the command.

### The dialog

- `prompt_variables` returns `Skip` immediately for an empty variable list
  (`src/ui/variables.rs:79-81`).
- `prompt_variables_inner` refuses to run without a terminal
  (`:212-217`, `io::ErrorKind::NotConnected`).
- Variables are **deduplicated by name**; one field is prompted per unique
  name and the value is reused for every occurrence (`:219-233`).
- `VariableKind::Choices` renders a choice selector; `Required` and
  `DefaultValue` render a text field (`:243-258`).
- The dialog has its own `TerminalGuard` (`:41-52`) mirroring the selector's
  restore guarantee, and its own `ratatui::init()` (`:235`).
- Two editing modes (module docs, `:6-16`): **INS** is the default and
  inserts every printable character literally (so `q` types "queue");
  **NOR** offers `h`/`l` cursor movement, `j`/`k` variable jumps, `d` to
  toggle the default hint, `q` to go back, `i` to return to INS. Only
  `Ctrl+C` (or a signal) exits the program (`:452-455`).
- The cursor is tracked per field as a **byte index on a char boundary**
  (`:90-91`), with dedicated `prev_char_boundary` / `next_char_boundary`
  helpers (`:191`, `:202`).

### Cancel semantics

`VariablePromptResult` (`:56-67`) distinguishes four outcomes, and callers
must not conflate them:

| Variant | Trigger | Caller behaviour |
|---|---|---|
| `Cancel` | `Ctrl+C` (`:452-455`) or the `TERMINATE` flag (`:265-267`) | `ExpandedCommand::Cancel` → the whole command exits (`src/commands/mod.rs:286`) |
| `Back` | `q` in normal mode of the prompt | `ExpandedCommand::Skip` → back to the snippet selector, nothing runs (`src/commands/mod.rs:287`) |
| `Skip` | Empty variable list (`:79-81`) | `ExpandedCommand::Skip` (`src/commands/mod.rs:288`) |
| `Values` | `Enter` in the prompt | The command is expanded and returned (`src/commands/mod.rs:289-291`) |

## Async integration (`Option<&Runtime>`)

The TUI itself is **synchronous**. Tokio enters only at the caller boundary:

- `run_snippet_selection` takes `runtime: Option<&tokio::runtime::Runtime>`
  (`src/commands/mod.rs:344`).
- When `do_sync` is true the runtime is **required**; `None` is a hard error
  ("sync requested but no runtime", `src/commands/mod.rs:353-359`). When
  `do_sync` is false the runtime is discarded and the value is `None`
  (`:360-362`).
- The CLI wires this as `args.sync.then_some(&RUNTIME)` (`src/main.rs:540`,
  `:561`, `:582`, `:597`), so the global `RUNTIME`
  (`src/main.rs:22-27`) is only borrowed when `--sync` was requested.
- `select_cmd` passes `do_sync = false` and `runtime = None`
  (`src/commands/select_cmd.rs:88-91`).

The sync itself runs **after** the TUI has exited:

- After a successful delete (`:435-440`) or after a successful selection
  (`:482-488`), `run_explicit_sync(rt)` runs — it acquires the auto-sync
  execution lock, observes the pending generation, runs the default sync,
  and clears the pending marker only on success (`:301-328`).
- Without `--sync`, the mutation path instead notifies the auto-sync worker
  via `notify_mutation(SnippetDelete, User)` (`:441-449`).

No async runtime is created inside `src/ui/`. The detached auto-sync worker
uses its own current-thread runtime; see [auto_sync.md](auto_sync.md).

## Deleted-snippet filtering

The TUI never sees deleted snippets. Filtering happens one layer up, in
`get_snippet_data` (`src/commands/mod.rs:243-271`):

```rust
// src/commands/mod.rs:244-250
let filtered: Vec<_> = snippets
    .snippets
    .iter()
    .enumerate()
    .filter(|(_, s)| !s.deleted)
    .collect();
let original_indices: Vec<usize> = filtered.iter().map(|(i, _)| *i).collect();
```

The resulting `original_indices` is the TUI-index → library-index mapping.
`run_snippet_selection` rebuilds it on every loop iteration (`:379-383`) so
that a delete inside the TUI is reflected on the next pass, and it uses it to
translate the TUI's `Delete(idx)` back into a library index (`:424-430`).

Note that `original_indices` and `folders` are **destructured but unused**
inside the TUI (`src/ui/mod.rs:552`, `:555`); the mapping is maintained by the
caller, not by the selector. The TUI's own row indices are always indices
into the filtered array it was handed.

Deletion itself is a **tombstone**, not a removal: `mark_snippet_deleted`
(`src/commands/mod.rs:499-514`) sets `deleted = true` and bumps `updated_at`
past `now` so the tombstone survives a sync round-trip.

## Rendering layout

```text
┌─────────────────────────────────────┐
│ Filter Input Box (3 lines)          │  ← chunks[0], Constraint::Length(3)
├─────────────────────────────────────┤
│ Snippet List (scrollable)           │  ← chunks[1], Constraint::Min(3)
│ ★ ▶ <highlighted command>           │  ← ★ = favorite, ▶ = cursor row
│ ★   [description] <highlighted cmd> │  ← list_display_mode == 1 (Tab)
├─────────────────────────────────────┤
│ Preview Panel (6 lines)             │  ← chunks[2], Constraint::Length(6)
│ Preview: <description>              │
│ <command>                           │
│ Vars: name, host                    │
├─────────────────────────────────────┤
│ Status Bar (1 line)                 │  ← chunks[3], Constraint::Length(1)
│ [NOR] | i: insert | y: copy | ...  │
└─────────────────────────────────────┘
```

- The four-row layout is computed at `src/ui/mod.rs:805-816`.
- The cursor marker is `▶ `, suppressed when `is_search` is true
  (`:1139`); the favorite marker is `★ ` (`:1084-1085`).
- `list_display_mode` (`:600`) selects the row shape: `0` (default) renders
  the highlighted command alone, `1` prepends `[description] `
  (`:1087-1111`). `Tab` cycles between them (`:1759`, `:1797`).
- Rows inside an active visual range get the `selected_bg` background
  (`:1121-1127`); the cursor row carries the same background through the
  list's `highlight_style` (`:1134-1138`).
- The preview panel is rendered only when a row is selected (`:1148-1189`).
  `SnippetPreview` (`:377-412`) appends a `Vars: …` line and, when the
  command contains an unmatched `<`, a literal-`<` warning (`:386-396`).
- The status bar is mode-dependent (`:1197-1224`): the `d: delete` hint is
  appended only when `allow_delete` is set (`:1195`).

## `is_search` is currently unreachable

`SnippetListParams.is_search` (`src/ui/mod.rs:503`) selects a search-flavoured
presentation: no `▶ ` row marker (`:1139`), a different status-bar hint line
(`:1206-1215`), and no `Copied` early return (`:1670`, `:1784`). **The only
caller passes `is_search: false`** (`src/commands/mod.rs:407`), so every
`is_search == true` branch in the selector is currently dead code. Treat the
flag as a reserved hook, not a live mode.

## Key Files

| File | Subject |
|------|---------|
| `src/ui/mod.rs` (2077 lines) | Event loop, tty guard, terminal setup/teardown, fuzzy filter, sort, keymap, rendering |
| `src/ui/state.rs` (250 lines) | `SelectState`, `FilterState`, `SortMode`, `is_ctrl_key` |
| `src/ui/variables.rs` (1579 lines) | Modal variable prompt (`<name>` / `<name=default>`) |
| `src/ui/theme.rs` (1063 lines) | `Theme` palette, `ThemeManager`, active-theme resolution — see [ui.md](ui.md) |
| `src/ui/highlight.rs` (248 lines) | `highlight_command()` tokenizer — see [ui.md](ui.md) |
| `src/ui/_generated_bundled_themes.rs` (301 lines) | **Generated** bundled themes — see [ui.md](ui.md) |
| `src/commands/mod.rs:338-496` | `run_snippet_selection` — the bridge from commands to the TUI |
| `src/commands/mod.rs:243-271` | `get_snippet_data` — deleted filtering + index mapping |
| `src/commands/mod.rs:499-514` | `mark_snippet_deleted` — tombstone write |
| `src/commands/keybindings_cmd.rs` (85 lines) | Static keybinding reference that mirrors the loop (can drift) |
| `src/selector.rs:135-158` | `searchable_text` — the *canonical* search contract the TUI does **not** use |
| `src/utils/variables.rs` (1947 lines) | Variable parsing / expansion / escape stripping |
| `src/main.rs:29-50` | SIGINT/SIGTERM handler feeding `TERMINATE` |
| `src/logging.rs:264-272` | Panic hook that restores the terminal |

## Invariants

1. **The selector never writes the library file.** The only destructive
   output is `SnippetSelection::Delete`, and it is honoured exclusively
   through `mark_snippet_deleted` (tombstone preserved for sync, never a hard
   remove).
2. **`select_snippet` requires a real tty** (`src/ui/mod.rs:533`). A piped
   invocation must fail with an actionable error, never abort the process.
3. **The terminal is always restored.** `TerminalGuard` is bound directly
   after `ratatui::init()`; the panic hook repeats the restore.
4. **The TUI never mutates a snippet and never expands variables itself.**
   Expansion happens in `expand_snippet_command` after the loop exits.
5. **Deleted snippets are filtered before the TUI sees them**
   (`src/commands/mod.rs:248`); the selector has no delete-tombstone concept.
6. **`SortMode::Command` is initial-only.** Interactive toggles never produce
   it.
7. **An empty filter recomputes immediately**; non-empty edits debounce at
   35 ms.
8. **`Copied` is gated on `!is_search`**, so a `y` copy in a search-flavoured
   mode would set `should_copy` and leave the loop via `Selected(idx, Some(_))`
   instead. Delete-confirm is gated on `allow_delete`, not on `is_search`.
9. **`should_copy` carries the description, not the command**
   (`src/ui/mod.rs:1783`); only `copy_flag.is_some()` is meaningful to the
   caller (`src/commands/run_cmd.rs:356`).
10. **Visual-mode `y` copies commands, not descriptions**, and audit-logs only
    the first row of the range.
11. **The TUI is not part of the search-parity triple.** `snp get --query`,
    `snp list --filter`, and MCP `snippets_search` share
    `selector::searchable_text`; the TUI filter is a separate client-side
    corpus with different tag semantics.

## Gotchas

- **Visual-mode `y` bypasses the clipboard funnel.** It calls
  `clipboard::copy_to_clipboard_auto` directly (`src/ui/mod.rs:1642`), so it
  records no usage-index entry and does not go through
  `copy_to_clipboard()` in `src/commands/clip_cmd.rs:40`. Only
  `src/commands/cron_cmd.rs:89` does the same thing.
- **Visual-mode audit logging covers only the first snippet of the range.**
  `filtered.get(start)` (`:1657`) is the sole audit-logged row.
- **Sorting invalidates an in-flight visual range.** The sort keys clear
  `visual_mode` for exactly this reason (`:1841-1843`); if you add a sort
  path, do the same.
- **`original_indices` is not used by the TUI.** It is the *caller's*
  index mapping (`src/commands/mod.rs:250`, `:424`). Reading it as a TUI
  invariant produces off-by-N bugs after a delete.
- **`j` and `k` are literal text in insert mode.** Any doc that claims
  vim-style navigation in insert mode is wrong.
- **The theme-picker filter cannot contain `j` or `k`** — those arms win
  over the catch-all `Char(c)` arm (`:1376`, `:1389`, `:1408`).
- **Esc does not quit.** `Esc` in normal mode is a no-op (`:1888-1890`);
  `q` and `Ctrl+C` do.
- **Ctrl+C in normal mode is a cancel, not a copy.** It sets
  `sel.selected = filtered.len()` (`:1567`) so the loop exits as
  `Cancelled`.
- **The `y` key means two different things.** In normal mode it copies the
  description of one row and exits; in visual mode it copies the *commands*
  of a range and exits with `SnippetSelection::Copied`.
- **`is_search` is dead code today.** Do not use it to reason about current
  behaviour.
- **`Folder` data is passed in but unused** (`src/ui/mod.rs:552`); the
  selector shows folders nowhere.
- **`keybindings_cmd.rs` can drift from the loop.** It is a static
  `println!` block with no test tying it to `src/ui/mod.rs`.

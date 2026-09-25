# TUI Selector (`ui/` event loop)

[← Back to Overview](overview.md)

## Overview

The interactive snippet selector is a single-loop, event-driven TUI built on
`ratatui` + `crossterm` (`src/ui/mod.rs:531`, `select_snippet_inner`). It owns
fuzzy filtering (`SkimMatcherV2`), keyboard/mouse navigation, sort mode,
visual multi-copy, the delete-confirmation dialog, and the theme-picker
overlay. Non-interactive callers never enter it: they resolve through
`run_snippet_selection` (`src/commands/mod.rs:306`) or `selector.rs`.

## Entry points

| Item | Location | Purpose |
|------|----------|---------|
| `SnippetListParams` | `src/ui/mod.rs:498` | Borrowed view: descriptions, commands, tags, folders, favorites, `snippets`, `original_indices`, `sort_opts`, `usage`, `is_search`, `initial_filter`, `allow_delete` |
| `SnippetSelection` | `src/ui/mod.rs:515` | `Selected(usize, Option<String>)` / `Copied` / `Delete(usize)` / `Cancelled` |
| `select_snippet` | `src/ui/mod.rs:526` | Thin wrapper over `select_snippet_inner` (`:531`); returns `io::Result<Option<SnippetSelection>>` |
| `run_snippet_selection` | `src/commands/mod.rs:306` | Loads library + usage, loops the TUI, handles `Delete`/`Copied`/sync (see below) |
| Re-exports | `src/ui/mod.rs:16` | `pub use theme::get_theme; pub use variables::{VariablePromptResult, prompt_variables}` — stable surface, see `ui.md` pitfall |

## Event loop (`select_snippet_inner`)

1. Enable mouse capture (degrades gracefully on headless SSH, `:548`), `ratatui::init()` (`:554`), install `TerminalGuard` (`:41`, `:555`) so Drop always disables mouse capture and restores the terminal.
2. Allocate lazy highlight/preview caches (`highlighted_commands`, `snippet_previews`, `:559`) — large libraries pay no syntax cost before the first frame.
3. Seed `SelectState`, `FilterState` (from `--sort`, `:566`), `filter`, debounce timers (`FILTER_DEBOUNCE_MS = 35`, `:619`), `delete_confirmation`, visual-mode and `gg` pending state, theme-picker state.
4. Precompute lowercase search caches once (`all_display`, `all_display_lower`, `all_commands_lower`, `all_tags_search`, `:627`).
5. Per frame: check `TERMINATE` (`:646`, set by the SIGINT/SIGTERM handler), expire copy messages, lazily init the theme picker, re-highlight on theme change, rebuild theme filter (100 ms debounce), recompute the snippet filter when the debounce elapses, draw, then `poll` with a deadline derived from pending debounces (`next_event_poll_timeout`, `:310`).

## Fuzzy matching (`SkimMatcherV2`)

- `MATCHER: LazyLock<SkimMatcherV2>` (`:65`) — one shared matcher, never reconstructed.
- `FilterRequest` (`:67`) carries `text`, `text_lower`, `include_tags`. `current_filter_request` (`:86`) selects the main filter vs tag-filter text; an empty request matches everything.
- `rebuild_filter_candidates` (`:137`): exact display match or tag-substring match pins `Some(i64::MAX)`; otherwise `MATCHER.fuzzy_match(display, text)` scores. Missing indices are skipped, never panicked on.
- Incremental narrowing: `can_narrow_from` (`:79`) reuses the previous candidate set when the new text extends the old one with unchanged tag scope (`:743`); otherwise rescans all displays. `filter_request_would_narrow` / `filter_update_deadline_for_insert` (`:112`, `:122`) feed the debounce scheduler.
- Debounce helpers (`:292`): `debounce_elapsed`, `pending_debounce_timeout`, `next_event_poll_timeout` — the event poll waits only as long as the nearest pending recompute.

## State (`src/ui/state.rs`)

| Type | Location | Notes |
|------|----------|-------|
| `SortMode` | `state.rs:4` | `None` (default), `Newest`, `Oldest`, `AlphaAsc`, `AlphaDesc`, `LastUsed`, `MostUsed`, `Command` (initial-only, mapped from `--sort command`, never produced by toggles) |
| `SelectState` | `state.rs:18` | `selected` + `scroll_state`; `update` clamps on filter shrink (`:32`); `move_up` saturates, `move_down` bounds-checks, `move_to_top/bottom` for `gg`/`G` |
| `FilterState` | `state.rs:63` | `sort_mode` + `tag_filter_text`; `toggle_sort_new/old/alpha/alpha_rev` (`:70`); last/most-used toggles exist but are `#[allow(dead_code)]` |
| `is_ctrl_key` | `state.rs:116` | Ctrl-combination predicate for paging/quit keys |

`sort_filtered_indices` (`mod.rs:172`): no-op fast path when `SortMode::None`, no filter, and no favorites-first (`:182`). Otherwise favorites group first (orthogonal modifier, `:188`), then score ordering, then the explicit mode — `Newest`/`Oldest` on `updated_at`, `LastUsed`/`MostUsed` on the `usage: Option<&[UsageData]>` slice (`:228`, `:244`; loaded once per session in `run_snippet_selection`, `:341`), `AlphaAsc`/`AlphaDesc` on display text, `Command` on command text (`:275`).

## Keyboard / mouse navigation

- **Insert mode** (default): typing edits `input_text`/`filter`; `↑`/`↓` navigate; `Esc` → normal; `Enter` selects; `Ctrl+f`/`Ctrl+d`/`PageDown` and `Ctrl+b`/`Ctrl+u`/`PageUp` page; `Tab` toggles display mode.
- **Normal mode**: `h`/`j`/`k`/`l` + arrows move; `gg` (500 ms window, `GG_TIMEOUT_MS`, `:595`) top, `G` bottom; `v`/`V` visual mode with batch copy; `y` copies selection; `d` opens delete confirmation (`y` confirms, any other key cancels, `:1543`); `/` incremental search; `t` tag-filter mode; `n`/`o` newest/oldest; `a`/`z` alpha asc/desc; `x`/`c` clear filter; `e` theme picker; `q`/`Ctrl+C` quit; `Enter` select.
- In insert mode `d` is ordinary filter input — deletion is only armed in normal mode and only when `allow_delete` is set (`:1776`); the status bar shows the `d: delete` hint conditionally (`:1181`).
- **Visual copy** returns `SnippetSelection::Copied` directly from the TUI (`:1656`) and only outside `is_search` mode (`:1612`); the caller treats it as processed without re-running `process_fn`.
- **Mouse**: scroll navigates, single click selects, double-click within 500 ms (`DOUBLE_CLICK_DURATION_MS`, `:625`) executes. List rows use `▶ ` markers outside search mode (`:1125`).
- **Delete dialog** (`:1221`): modal overlay showing the pending index; confirmation returns `SnippetSelection::Delete(idx)` (`:1547`).
- Loop exit maps `TERMINATE` to `Cancelled` (`:1898`) and Enter to `Selected(idx, copy_flag)` (`:1905`), else `Cancelled` (`:1910`).

## `run_snippet_selection` integration

`src/commands/mod.rs:306` is the sole bridge between commands (`run`, `clip`, `search`, `select`) and the TUI:

- Requires a Tokio runtime when `do_sync` is true, else `None` is fine (`:321`); resolves the library path, loads the library (`:332`), loads `UsageIndex` once (`:341`), and builds an id→usage map for O(1) per-snippet lookup (`:354`).
- Each loop iteration rebuilds live (non-deleted) data via `get_snippet_data` plus aligned `live_usage` (`:347`), then calls `select_snippet` with `allow_delete` (`:371`).
- `Delete(idx)` (only when `allow_delete`, `:391`): maps through `original_indices`, `mark_snippet_deleted` sets the tombstone + bumps `updated_at` past now (`:466`), saves, audit-logs `delete` (`:400`), then explicit sync or `notify_mutation(SnippetDelete, User)` (`:403`); loops back so the list reloads without the deleted row. Bare `Delete(_)` without permission is ignored (`:420`).
- `Copied` ends the loop as processed (`:421`); `Selected` runs `process_fn` and maps `Cancel`/`Continue`/`Done`/`Failed` (`:425`); `None` counts as cancelled (`:445`). Post-selection explicit sync runs once on success (`:450`).

## Terminal / signal safety

- `TerminalGuard::drop` (`mod.rs:43`) disables mouse capture (warn-only on failure) then `ratatui::restore()` — every early return and panic unwinds through it.
- `TERMINATE: LazyLock<Arc<AtomicBool>>` (`:58`); `get_terminate` (`:61`) shares it with the Unix SIGINT/SIGTERM handler so the loop breaks cleanly (`:646`).
- The global panic hook (`src/logging.rs:245`) repeats the restore sequence (explicit `DisableMouseCapture` + `ratatui::restore()`) before logging and flushing.

## Sort-option mapping (`--sort` → `SortMode`)

| CLI (`SnippetSort`) | TUI mode | Ordered by |
|------|----------|------------|
| `Relevance` | `None` | Fuzzy score only (`mod.rs:568`) |
| `Recent` | `Newest` | `updated_at` desc, index tiebreak |
| `LastUsed` | `LastUsed` | `usage.last_used_at` desc, missing last |
| `MostUsed` | `MostUsed` | `use_count` desc, then `last_used_at` |
| `Description` | `AlphaAsc` | Lowercased `[desc]: cmd` display |
| `Command` | `Command` | Lowercased command text (initial-only) |

`--favorites-first` layers favorite grouping above whichever mode is active
(`favorites_first`, `mod.rs:582`).

## Rendering layout

```text
┌─────────────────────────────────────┐
│ Filter Input Box (3 lines)          │  ← input_text (typed) vs filter (applied)
├─────────────────────────────────────┤
│ Snippet List (scrollable)           │  ← lazy highlighted rows, ▶ marker
│ ▶ [description] command...          │
│   [description] command...          │
├─────────────────────────────────────┤
│ Preview Panel (6 lines)             │  ← selected snippet + <variables>
│ Description: ...                    │
│ Command: ... (highlighted)          │
│ Vars: name, host                    │
├─────────────────────────────────────┤
│ Status Bar (1 line)                 │  ← mode, hints, copy/delete messages
│ [INS] | i: insert | y: copy | ...  │
└─────────────────────────────────────┘
```

Highlights warm within a 2 ms per-frame budget (`HIGHLIGHT_WARM_BUDGET`,
`:620`, `warm_visible_highlights`, `:344`); off-screen rows highlight on
scroll (`command_line_for_display`, `:328`).

## Search vs select mode (`is_search`)

`SnippetListParams.is_search` (`:502`) marks search-flavored invocations:
no `▶ ` row markers (`:1125`), no visual-mode `Copied` shortcut (`:1612`),
and no delete hint — destructive affordances stay in the managing selector.

## Theme-picker overlay

Opened with `e` in normal mode; `j`/`k` preview live via `set_active_theme`,
`i` filters by name (100 ms debounce, `THEME_DEBOUNCE_MS`, `:613`), `Enter`
persists through `ThemeManager`, `e`/`q`/`Esc` restores the snapshot taken at
open (`theme_picker_original`, `:610`). Init failure degrades to the plain
selector with a warning (`:659`). Full palette/bundling detail: [ui.md](ui.md).

## Invariants

- The TUI never mutates the library file; the only destructive output is `SnippetSelection::Delete`, honored solely through `mark_snippet_deleted` (tombstone preserved for sync, never a hard remove).
- `original_indices` keeps TUI rows stable across filter/sort transitions; usage rows are indexed by the same mapping.
- `SortMode::Command` is initial-only; interactive toggles never produce it.
- Empty-filter recompute is immediate (avoids stale rows after clearing input); non-empty edits debounce at 35 ms.
- `Copied` and delete-confirm paths are disabled in `is_search` contexts where noted.

## Key files

- `src/ui/mod.rs` (2063 lines) — event loop, fuzzy filter, sort, input handling, rendering
- `src/ui/state.rs` (250 lines) — `SelectState`, `FilterState`, `SortMode`, `is_ctrl_key`
- `src/commands/mod.rs:306` — `run_snippet_selection`, `mark_snippet_deleted`
- `src/ui/theme.rs`, `highlight.rs`, `variables.rs` — see [ui.md](ui.md)

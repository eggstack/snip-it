# UI Module Improvement Plan

## Overview

Review date: 2026-05-29
Source files: `src/ui/mod.rs` (958 lines), `src/ui/variables.rs` (203 lines), `src/ui/highlight.rs` (162 lines), `src/ui/theme.rs` (103 lines)
Total: **1426 lines** (document claims ~1400 ✓)

---

## Architecture Document Claims Verification

| Claim | Status | Notes |
|-------|--------|-------|
| File is `src/ui/` (~1400 lines) | ✓ VERIFIED | 1426 lines total |
| `select_snippet_inner()` is single-loop event-driven TUI | ✓ VERIFIED | Lines 124-886 |
| Pre-compute syntax highlights once at startup | ✓ VERIFIED | Lines 139-140 |
| Filter debounced at 150ms | ✓ VERIFIED | Line 163, lines 181-199 |
| Insert/Normal mode system | ✓ VERIFIED | Lines 150, 713-797 (insert), 798-871 (normal) |
| Visual mode (`v`/`V`) | ✓ VERIFIED | Lines 626-687 |
| Tag filter mode (`t`) | ✓ VERIFIED | Lines 851-859 |
| Layout: 3-line filter, scrollable list, 6-line preview, 1-line status | ✓ VERIFIED | Lines 376-384 |
| Syntax highlighting table | ✓ VERIFIED | Lines 14-19 in highlight.rs |
| Shell keywords HashSet (~190) | ✓ VERIFIED (minor) | Actually 191 keywords |
| Two themes (dark/bright) + auto | ✓ VERIFIED | Lines 39-54 in theme.rs |
| `SNP_THEME` env var | ✓ VERIFIED | Line 59 in theme.rs |
| `fuzzy-matcher` crate (skim) | ✓ VERIFIED | Line 18 |
| Mouse: scroll, click, double-click (500ms) | ✓ VERIFIED | Lines 166-168, 561-605 |
| `prompt_variables_inner()` for variables | ✓ VERIFIED | variables.rs |
| Tab navigation in variable prompt | ✓ VERIFIED | Lines 159-165 |
| Double-buffered filter (`input_text` vs `filter`) | ✓ VERIFIED | Lines 144-146 |
| Terminal size check (< 10x10) | ✓ VERIFIED | Line 349 |
| Mouse capture enabled/disabled | ✓ VERIFIED | Lines 134, 879 |
| Integration: clipboard.rs | ✓ VERIFIED | Line 29, used at 619, 675, 817 |
| Integration: utils/variables.rs | ✓ VERIFIED | Line 30, used at 99-101 |
| Integration: utils/shell_keywords.rs | ✓ VERIFIED | highlight.rs line 3 |

---

## Bugs Found

### 1. Visual Mode Copies Descriptions Instead of Commands
**Severity: Medium**

In `mod.rs:665-681`, when `y` is pressed in visual mode:
```rust
KeyCode::Char('y') => {
    let start = std::cmp::min(visual_start, visual_end);
    let end = std::cmp::max(visual_start, visual_end);
    let selected_items: Vec<&str> = filtered
        .iter()
        .skip(start)
        .take(end - start + 1)
        .map(|(_, desc, _)| desc.as_str())  // BUG: copies descriptions, not commands!
        .collect();
```

The code copies `desc` (descriptions) instead of `commands[idx]`. This is inconsistent with single-copy (`y` in normal mode, line 813-822) which correctly copies the command.

**Expected behavior**: Visual mode should copy the actual snippet commands, matching single-select behavior.

---

### 2. Unmatched `<` Creates Phantom Variable
**Severity: Low (Known edge case per AGENTS.md)**

In `utils/variables.rs:43-67`, if there's an unmatched `<` character:
```rust
if c == '<' {
    let mut var_content = String::new();
    while let Some(&next) = chars.peek() {
        if next == '>' {
            chars.next();
            break;
        }
        var_content.push(chars.next().unwrap());
    }
    // If no '>' found, var_content accumulates characters until EOF
```

If the command contains `<foo` without a closing `>`, the parser continues consuming characters until it finds a `>` or reaches EOF. This creates a "phantom variable" and drops the `<` character from display.

**Note**: This is documented in AGENTS.md as a known edge case.

---

### 3. Double-Click Detection Logic Flaw
**Severity: Low**

In `mod.rs:581-604`, double-click detection:
```rust
let is_double_click = last_click_row == Some(mouse_event.row)
    && last_click_time.map(|t| {
        now.duration_since(t).as_millis() < DOUBLE_CLICK_DURATION_MS as u128
    }).unwrap_or(false);
```

If user clicks Row A (sets `last_click_row=A`), then quickly clicks Row B, the second click is NOT treated as double-click (correct), but `last_click_row` and `last_click_time` are NOT reset. So clicking A→B→A in rapid succession might not register properly.

---

### 4. Spurious `DisableMouseCapture` in Variable Prompt
**Severity: Low**

In `variables.rs:140-143`:
```rust
KeyCode::Char('q') => {
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture
    );
```

Mouse capture is disabled on quit, but it was never enabled in `prompt_variables_inner()`. This is harmless (no-op) but indicates missing enable/disable symmetry. The same happens at line 194 on normal exit.

---

## Potential Improvements

### 1. Add SIGINT/SIGTERM Handler for Clean Exit
**Severity: Medium**

Currently, Ctrl+C during the TUI loop (`event::poll`) may leave the terminal in raw mode. A signal handler should:
- Set a termination flag
- Restore terminal state before exit
- Ensure `ratatui::restore()` and `DisableMouseCapture` are called

The `TERMINATE` static exists (line 36-40) but is never checked in the event loop.

---

### 2. Error Handling for `event::poll` and `event::read`
**Severity: Low**

In `mod.rs:559` and `mod.rs:136` (variables.rs), IO errors are silently ignored:
```rust
if event::poll(Duration::from_millis(200))? {
    if let CEvent::Key(key) = event::read()? {
```

The `?` propagates errors, but the result is discarded in the outer `if let Ok(CEvent::Key(key))` match. Consider logging these errors.

---

### 3. Theme Caching Optimization
**Severity: Low**

In `theme.rs:64-66`:
```rust
pub fn get_theme() -> std::sync::MutexGuard<'static, Theme> {
    ACTIVE_THEME.lock().unwrap()
}
```

`get_theme()` is called multiple times per frame inside the draw closure. Each call locks a mutex. The theme doesn't change at runtime - consider returning a `&'static Theme` or using a simpler static reference.

---

### 4. Pre-computed Highlights Memory Pressure (Documented Deferred Item)
**Severity: Medium**

Per `plan.md` and AGENTS.md: All snippets are highlighted at startup regardless of visibility. For large libraries (1000+ snippets), this allocates significant memory unnecessarily.

**Improvement**: Consider lazy highlighting or only highlighting visible items.

---

### 5. Fuzzy Match Score Not Used Consistently
**Severity: Low**

In `mod.rs:283-311`, when sorting with a filter:
```rust
candidates.sort_by(|a, b| {
    let score_cmp = match (a.3, b.3) {
        (Some(sa), Some(sb)) => sb.cmp(&sa),
        // ...
    };
    // If scores are equal OR no filter active, use explicit sort
    if score_cmp != std::cmp::Ordering::Equal || !has_filter {
        // Uses sort mode instead of fuzzy score
    }
});
```

When explicit sort modes (newest, oldest, alpha) are active, fuzzy scores are discarded. This means `n` (newest sort) might rank a poor fuzzy match above a perfect match.

---

### 6. Enter Key Behavior Inconsistency in Search Mode
**Severity: Low**

In search mode (`is_search=true`), pressing Enter does nothing (lines 742-746):
```rust
KeyCode::Enter => {
    if !is_search {
        break;
    }
}
```

But in non-search mode, Enter runs the snippet. This is intentional but undocumented - users in search mode might expect Enter to select.

---

### 7. Visual Mode Selection Range Includes Visual Start
**Severity: Low**

In `mod.rs:646-686`, visual selection with `j`/`k` includes `visual_start` in the range:
```rust
let start = std::cmp::min(visual_start, visual_end);
let end = std::cmp::max(visual_start, visual_end);
let selected_items: Vec<&str> = filtered
    .iter()
    .skip(start)
    .take(end - start + 1)  // +1 includes visual_start
```

Standard vim behavior excludes the start position when yanking. The current implementation yanks `end - start + 1` items.

---

### 8. Variable Prompt Limit of 10
**Severity: Low**

In `variables.rs:65`:
```rust
let num_vars = values.len().min(10);
```

Only 10 variables are displayed at once. Snippets with 11+ variables will have hidden variables. Consider scrolling or pagination.

---

### 9. `list_display_mode` Tab Cycles Through 2 Modes
**Severity: Very Low (Documentation)**

The Tab key cycles through modes 0 and 1 (line 794). The document doesn't mention this feature. Mode 1 shows `[description] command...` while mode 0 shows just the command.

---

## Discrepancies

| Documented | Actual | Impact |
|------------|--------|--------|
| "~190" shell keywords | 191 keywords | None (minor) |
| `Enter` selects in search mode | `Enter` does nothing in search mode | Medium (usability) |
| Visual mode copies commands | Visual mode copies descriptions | Medium (bug) |
| `n` keybinding not documented | `n` toggles newest sort | Low (missing docs) |
| `o`, `a`, `z` keybindings not documented | These toggle sort modes | Low (missing docs) |

---

## Summary

- **Bugs requiring fixes**: 1 (visual mode copy bug)
- **Improvements recommended**: 9
- **Discrepancies**: 5
- **Deferred items confirmed**: Pre-computed highlights memory pressure remains unaddressed

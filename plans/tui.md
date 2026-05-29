# TUI Architecture Improvement Plan

## Verification of Architecture Document Claims

### Claim 1: Overview
**Status: VERIFIED**
- The TUI provides an interactive, fuzzy-search snippet selector using `ratatui` and `crossterm`. It is event-driven with a single render loop.

### Claim 2: Main Loop - `select_snippet_inner()`
**Status: PARTIALLY VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| Loads snippets from library | DISCREPANcy - Takes snippets as parameters, not directly from library |
| Renders a filterable list | VERIFIED |
| Handles keyboard/mouse events | VERIFIED |
| Calls user-provided closure on selection | DISCREPANCY - Returns `(usize, Option<String>)` instead of calling a closure |

### Claim 3: State - `SelectState` struct
**Status: BUG - DOES NOT EXIST**

The documented `SelectState` struct does not exist. Instead, state is scattered across local variables in `select_snippet_inner()`:
- `filter`, `incremental_search`, `input_text` (String)
- `filter_state` (FilterState - only has `sort_mode` and `tag_filter_text`)
- `selected`, `visual_start`, `visual_end` (usize)
- `insert_mode`, `tag_filter_mode`, `visual_mode` (bool)
- `list_display_mode` (i32)
- `filtered` (Vec)

**Recommendation**: Consider creating a `SelectState` struct for better code organization and testability.

### Claim 4: Fuzzy Matching
**Status: VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| Uses `fuzzy-matcher` with `SkimMatcherV2` | VERIFIED (line 43) |
| Matches snippet names and commands | VERIFIED |
| Score-based ranking | VERIFIED |
| Debounced updates (150ms) | VERIFIED (line 163) |

### Claim 5: Themes
**Status: VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| DARK_THEME and BRIGHT_THEME | VERIFIED |
| Color fields (primary, secondary, accent, background, text, border, selected_bg, muted) | VERIFIED |
| SNP_THEME environment variable | VERIFIED |
| COLORFGBG terminal environment variable | VERIFIED |

### Claim 6: Syntax Highlighting
**Status: VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| Variables (`<name>`) — Accent color | VERIFIED |
| Shell keywords (~190 commands) — Primary color | VERIFIED (192 keywords) |
| Strings (quoted) — Green | VERIFIED |
| Flags (`--flag`) — Secondary color | VERIFIED |
| Comments (`# ...`) — Muted | VERIFIED |
| Escape sequences (`\<`, `\>`) — Magenta | VERIFIED |
| Pre-computed once at startup | VERIFIED |

### Claim 7: Variable Prompt
**Status: PARTIALLY VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| Shows variable name and default | VERIFIED |
| Editable text field | VERIFIED |
| Keyboard navigation (arrows, tab, enter) | VERIFIED |
| `q` to cancel | VERIFIED |
| `Esc` to skip | DISCREPANCY - Esc does nothing (line 148-150: "Esc no longer quits - use q instead") |

### Claim 8: Keybindings
**Status: MULTIPLE DISCREPANCIES**

#### Normal Mode Discrepancies:

| Key | Documented Action | Actual Action |
|-----|-------------------|---------------|
| `h` / `←` | Move left (visual mode) | Move left in ALL modes (line 831) |
| `l` / `→` | Move right (visual mode) | Move right in ALL modes (line 838) |
| `gg` | Jump to top | `Ctrl+g` jumps to top (line 827) |
| `n` | Sort by name | Sort by Newest (line 843) |
| `o` | Sort by date | Sort by Oldest (line 844) |
| `a` | Sort by usage | Sort AlphaAsc (line 845) |
| `Esc` | Quit | Does nothing (line 867-869) |

#### Insert Mode: VERIFIED

#### Mouse: VERIFIED

### Claim 9: Signal Handling
**Status: NOT VERIFIED IN CODE**

The document claims Unix signals (`SIGINT`, `SIGTERM`) restore terminal state before exit, but no explicit signal handling exists in `ui/mod.rs`.

**Note**: Ratatui's `restore()` is called on exit, which may handle this implicitly.

### Claim 10: Performance Considerations
**Status: VERIFIED**

| Sub-claim | Status |
|-----------|--------|
| Pre-computed highlights | VERIFIED (line 139-140) |
| Debounced filtering (150ms) | VERIFIED (line 163) |
| Lazy rendering via ratatui | VERIFIED (draw loop only redraws when needed) |

### Claim 11: Known Edge Cases
**Status: VERIFIED**

| Edge Case | Status |
|-----------|--------|
| Unmatched `<` creates phantom variable | VERIFIED (known issue) |
| Long commands may not wrap | VERIFIED (known issue) |

---

## Bugs Found

### Bug 1: Unmatched Angle Bracket Creates Phantom Variable
**Location**: `highlight.rs:39-49`
**Severity**: Low (documented edge case)
**Description**: When parsing `<text` without closing `>`, the parser silently drops the `<` and treats everything after as literal text. No warning is shown to the user.

### Bug 2: Visual Mode Navigation Inconsistent
**Location**: `mod.rs:831, 838`
**Severity**: Low
**Description**: `h`/`l` arrow keys move selection in ALL modes, not just visual mode. The documented behavior says they should only work in visual mode.

### Bug 3: `gg` Keybinding Missing
**Location**: `mod.rs:827`
**Severity**: Low
**Description**: Documented `gg` (vim-style jump to top) is actually `Ctrl+g`. Users expecting vim behavior may be confused.

### Bug 4: Esc Key Inconsistent Behavior
**Location**: `mod.rs:867-869`, `variables.rs:148-150`
**Severity**: Low
**Description**: In normal mode, `Esc` does nothing (unlike vim where it quits). The documentation says `Esc` quits, but the code comment says "Esc no longer quits - use q instead". This is confusing.

### Bug 5: Sort Keys Don't Match Documentation
**Location**: `mod.rs:843-846`
**Severity**: Low
**Description**:
- `n` sorts by Newest, not Name
- `o` sorts by Oldest, not Date
- `a` sorts AlphaAsc, not Usage

### Bug 6: Visual Line Mode (`V`) Bug
**Location**: `mod.rs:633-638`
**Severity**: Medium
**Description**: When pressing `V` (visual line mode), `visual_end` is set to `filtered.len().saturating_sub(1)` which is the last valid index. However, `selected` stays at current position. This can cause confusing visual state where the selection range doesn't include the intended items.

### Bug 7: Double-Click Execution Not Documented
**Location**: `mod.rs:591-593`
**Severity**: Low
**Description**: Double-click executes the snippet (line 593: `break`), but this is not mentioned in the architecture document's keybindings or mouse sections.

---

## Potential Improvements

### Improvement 1: Add Signal Handling
**Priority**: Medium
**Description**: Add explicit `SIGINT`/`SIGTERM` handlers to ensure terminal state is properly restored on unexpected exit. Currently relies on `ratatui::restore()` called at end of loop.

### Improvement 2: Create SelectState Struct
**Priority**: Low
**Description**: Refactor scattered local variables into a `SelectState` struct for better organization, testability, and documentation.

### Improvement 3: Unmatched Variable Warning
**Priority**: Low
**Description**: Show a warning in the preview panel when a snippet command contains an unmatched `<` character.

### Improvement 4: Visual Mode Boundaries
**Priority**: Low
**Description**: In visual mode, `h`/`l` should respect `visual_start`/`visual_end` boundaries rather than moving the primary selection.

### Improvement 5: Fix Keybinding Documentation or Code
**Priority**: Medium
**Description**: Either update the architecture document to match the code, or update the code to match the documented keybindings. The current inconsistency will confuse users.

### Improvement 6: Add `gg` as Alternative to `Ctrl+g`
**Priority**: Low
**Description**: Many vim users expect `gg` to work for jumping to top. Consider adding support.

### Improvement 7: Terminal Size Change Handling
**Priority**: Medium
**Description**: The code checks for minimum terminal size (10x10) on draw, but does not handle window resize events gracefully. The TUI may become unresponsive if the terminal is resized smaller than minimum.

### Improvement 8: Error Propagation
**Priority**: Low
**Description**: Several places use `unwrap_or()` or `unwrap_or_default()` which silently swallow errors. Consider logging warnings or returning errors in these cases.

---

## Discrepancy Summary

1. **SelectState struct**: Documented but doesn't exist in code
2. **Closure vs return**: Document says "calls user-provided closure on selection" but code returns index
3. **Esc to skip**: Document says `Esc` skips variables, but it does nothing
4. **h/l keys**: Document says only work in visual mode, but work in all modes
5. **gg keybinding**: Document says `gg` but code has `Ctrl+g`
6. **Sort keys**: `n`/`o`/`a` don't match documented meanings (Name/Date/Usage)
7. **Esc to quit**: Document says `Esc` quits but it does nothing
8. **Double-click**: Not documented but executes snippet

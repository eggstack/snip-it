# UI Module Review

**File**: `src/ui.rs` (1416 lines)
**Architecture Doc**: `architecture/ui.md` (112 lines)

---

## 1. Document Accuracy

### Verified Correct

- **ratatui + crossterm backend**: Confirmed. `ratatui::init()` at line 352, crossterm mouse capture at line 351.
- **Single-loop event-driven TUI**: `select_snippet_inner()` is indeed a single `loop {}` at line 397.
- **Pre-computed syntax highlighting**: `highlighted_commands` computed once at line 356, used inside draw at line 611. Doc is accurate.
- **Two primary modes (Insert/Normal)**: `insert_mode: bool` at line 367. Insert mode types into filter, Esc transitions to normal. Verified at lines 927-1086.
- **Visual mode (`v`/`V`)**: Lines 840-901. Multiple selection with batch copy via `y`.
- **Tag filter mode (`t`)**: Lines 1067-1074. `tag_filter_mode` flag.
- **Debounced filtering at 150ms**: `FILTER_DEBOUNCE_MS: u64 = 150` at line 380. Debounce logic at lines 398-416.
- **Fuzzy matching via `fuzzy-matcher` (skim)**: `static MATCHER: LazyLock<SkimMatcherV2>` at line 64.
- **Terminal size check (<10x10)**: Lines 566-573 and 1127-1133.
- **Mouse capture enabled on init, disabled on exit**: Lines 351 and 1093.
- **Theme system via `SNP_THEME` env var**: Line 190. `auto` detects via `COLORFGBG` (lines 174-183).
- **Keywords in `src/utils/shell_keywords.rs`**: Confirmed `SHELL_KEYWORDS_SET` at line 54. ~196 entries.
- **Integration points**: `clipboard.rs` (line 833), `utils/variables.rs` (line 53), `commands/mod.rs` (lines 207, 240).

### Discrepancies

| # | Doc Claim | Actual | Severity |
|---|-----------|--------|----------|
| D1 | "Double-click: Run/execute selected snippet" | Double-click breaks the loop (`line 807`), returning the current selection — does NOT directly run/execute. The caller handles execution. Doc implies automatic execution. | Low |
| D2 | Doc says `is_ctrl_key` exists but doesn't describe it | Minor omission, not a discrepancy | Trivial |
| D3 | Doc says preview panel is "6 lines" | `Constraint::Length(6)` at line 598 — correct | N/A |
| D4 | Doc says filter input is "3 lines" | `Constraint::Length(3)` at line 596 — correct | N/A |

---

## 2. Bugs & Issues

### B1: Double-buffered filter is not correctly maintained

**Location**: `src/ui.rs:961-1002`

The doc says "`input_text` (what user types) vs `filter` (applied filter) are separate." However, in insert mode, `input_text` and `filter` are **both modified in lockstep** (lines 989-993 for Char, lines 967-968 for Backspace). They are always identical during insert mode. The double-buffer design is only meaningful when transitioning from insert to normal mode (Esc copies `input_text` to `filter` at line 940). This is correct but the naming `input_text` vs `filter` is misleading — they are not independently managed buffers.

**Impact**: Low — no functional bug, but confusing naming.

### B2: `Ctrl+D` and `Ctrl+F` are identical to Page Down

**Location**: `src/ui.rs:903-910`

```rust
if is_ctrl_key(&key, 'f') {
    selected = (selected + 10).min(filtered.len().saturating_sub(1));
}
if is_ctrl_key(&key, 'd') {
    selected = (selected + 10).min(filtered.len().saturating_sub(1));
}
```

Both do the same thing (`+10`). Similarly `Ctrl+B` and `Ctrl+U` (lines 911-917) both do `-10`. Standard vim uses `Ctrl+D`/`Ctrl+U` for half-page scroll and `Ctrl+F`/`Ctrl+B` for full-page scroll. Here they are all fixed ±10, which is inconsistent with vim conventions.

**Impact**: Low — non-standard but not a bug.

### B3: Visual mode `j`/`k` navigation has asymmetric boundary behavior

**Location**: `src/ui.rs:861-878`

In visual mode, pressing `j` extends `visual_end` (line 868: `visual_end += 1; selected = visual_end`), but pressing `k` extends `visual_start` backwards (line 874: `visual_start -= 1; selected = visual_start`). This means:
- `j` expands the selection downward by moving `visual_end`
- `k` expands upward by moving `visual_start`

However, when `selected > visual_start` and user presses `k`, it simply does `selected -= 1` (line 872) without adjusting either boundary — the selection doesn't actually shrink. This is confusing: the selection boundary only moves in one direction for each key.

**Impact**: Medium — visual mode selection is harder to use than expected.

### B4: `selected` can become stale after filter changes

**Location**: `src/ui.rs:535-539`

```rust
if filtered.is_empty() {
    selected = 0;
} else if selected >= filtered.len() {
    selected = filtered.len().saturating_sub(1);
}
```

After filtering, `selected` is clamped. However, when the filter changes (e.g., user types a character), `selected` is reset to 0 (line 1003: `selected = 0`) but only when `!filtered.is_empty()`. On the first keystroke, `filtered` might still hold the previous full list, so `selected = 0` is correct. But after filtering reduces the list, subsequent keystrokes reset `selected = 0` — this is intentional UX but means the user always jumps to the top on every keystroke.

**Impact**: Low — intentional but could be improved (preserve last selection).

### B5: `highlight_command` hangs onto theme lock during draw

**Location**: `src/ui.rs:216, 577`

`highlight_command()` calls `get_theme()` (line 216) which acquires `ACTIVE_THEME` mutex. This is called once at startup (line 357), so it's fine. But inside the draw closure at line 577, `get_theme()` is called again. Since `ACTIVE_THEME` is a `Mutex`, and `ratatui::draw()` runs a closure, this should be safe (no reentrancy). However, the `Mutex<Theme>` is held for the duration of the `get_theme()` call inside the draw closure — if any other thread tried to call `get_theme()` simultaneously, it would block. This is unlikely but architecturally fragile.

**Impact**: Low — no current multi-threading issue, but the `Mutex` is unnecessary since `Theme` is `Copy`.

### B6: `c` keybinding conflict in normal mode

**Location**: `src/ui.rs:1061`

```rust
KeyCode::Char('x') | KeyCode::Char('c') => {
    filter.clear();
    incremental_search.clear();
    ...
}
```

The `c` key clears the filter. But in the status bar (line 729), `c` is shown alongside `x` for clearing. However, `c` in vim normally means "change" (entering insert mode for editing). This is a potential UX surprise. Also, `c` in normal mode clears the filter but does NOT enter insert mode — so the user must also press `i` to start typing a new filter.

**Impact**: Low — documented in status bar, but vim users may be surprised.

### B7: No guard against zero-length terminal in mouse calculations

**Location**: `src/ui.rs:756-770`

```rust
let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
```

If `terminal::size()` fails, defaults to 80x24. Then `list_area` is calculated. If the terminal is actually tiny (<10 rows), the draw returns early (line 567), but `list_area` is still computed using the fallback size. The mouse event handling at lines 788-818 uses this potentially incorrect `list_area`. This is safe because the draw won't render anything if terminal is too small, and mouse events on a tiny terminal are unlikely.

**Impact**: Negligible — safe fallback.

### B8: `copied_message` expiration check happens on every keypress

**Location**: `src/ui.rs:824-828`

```rust
if let Some((_, instant)) = copied_message {
    if instant.elapsed().as_secs() >= 3 {
        copied_message = None;
    }
}
```

This only runs when a key is pressed, not continuously. If no keys are pressed for >3 seconds, the "copied" message remains visible until the next keypress. Not a bug but slightly inaccurate behavior.

**Impact**: Negligible.

---

## 3. Design Issues

### D1: 1416-line monolithic file

`ui.rs` is the largest file in the codebase. It contains:
- Theme system (30 lines)
- Filter state management (40 lines)
- Syntax highlighting tokenizer (110 lines)
- Main TUI loop (~750 lines)
- Variable prompting TUI (~170 lines)
- Tests (~140 lines)

**Recommendation**: Extract `Theme` + `resolve_theme` into `ui/theme.rs`. Extract `highlight_command` into `ui/highlight.rs`. Extract `prompt_variables_inner` into `ui/variables.rs`. Keep `select_snippet_inner` as the main orchestrator.

### D2: `_folders` parameter is unused

**Location**: `src/ui.rs:348`

```rust
_folders: &[Vec<String>],
```

The `folders` parameter is accepted but never used in `select_snippet_inner`. It's prefixed with `_` to suppress warnings. This is dead API surface.

**Recommendation**: Either implement folder filtering or remove the parameter.

### D3: `list_display_mode` is an integer, not an enum

**Location**: `src/ui.rs:369`

```rust
let mut list_display_mode = 0;
```

Toggled via `(list_display_mode + 1) % 2` at lines 1008 and 1039. Magic numbers `0` and `1` are used to decide rendering mode (line 609). An enum would be clearer.

### D4: Inconsistent error handling in `select_snippet_inner`

**Location**: `src/ui.rs:1095-1099`

The function returns `io::Result<Option<(usize, Option<String>)>>`. The `should_copy` field is `Option<String>` — if set, it's the description of what was copied. The return type conflates "selected snippet index" with "was something copied" into a single tuple. These are two different concerns.

**Recommendation**: Return a richer result type, e.g., `SnippetAction::Selected(usize)` vs `SnippetAction::Copied(usize, String)`.

### D5: `ACTIVE_THEME` uses `Mutex<Theme>` but `Theme` is `Copy`

**Location**: `src/ui.rs:136-146, 188`

`Theme` is `#[derive(Clone, Copy)]`. Wrapping it in `Mutex` is unnecessary overhead. A `LazyLock<Theme>` (read-only after init) would suffice.

**Recommendation**: Change `ACTIVE_THEME` to `LazyLock<Theme>` and return `Theme` by value from `get_theme()`.

### D6: `TERMINATE` flag is never checked in the TUI loop

**Location**: `src/ui.rs:57-62, 397`

The `TERMINATE` `AtomicBool` is registered as a signal handler (main.rs:50-55) but is never polled in the TUI event loop. If SIGINT/SIGTERM is received during TUI, the loop continues until the next crossterm event. Crossterm may handle Ctrl+C internally, but the `TERMINATE` flag appears to be dead code for TUI purposes.

**Impact**: Low — crossterm handles Ctrl+C. But the flag gives a false impression of being integrated.

---

## 4. Security Concerns

### S1: Variable values interpolated directly into shell commands

**Location**: `src/ui.rs:1202`

```rust
let warning_text = "Values are interpolated directly into shell commands. Do not enter untrusted input.";
```

The warning exists, which is good. However, there is no sanitization or escaping of variable values. If a user enters `; rm -rf /` as a variable value, it will be interpolated directly. This is by design (the tool is for local snippet management), but the warning should be more prominent.

**Impact**: Medium — user is warned, but no enforcement.

### S2: `highlight_command` does not sanitize for terminal injection

**Location**: `src/ui.rs:211-319`

The syntax highlighter processes raw command strings. If a snippet contains ANSI escape sequences or special terminal characters, they could theoretically affect rendering. However, ratatui renders through crossterm which should handle this safely.

**Impact**: Low.

---

## 5. Performance Issues

### P1: Keyword matching uses linear scan per word

**Location**: `src/ui.rs:312`

```rust
let is_kw = shell_keywords.iter().any(|kw| word == *kw);
```

`SHELL_KEYWORDS_SET` is a `HashSet`, but `shell_keywords.iter()` iterates the set. `HashSet::iter()` returns all elements, and `.any()` does a linear scan. This is O(n) per word instead of O(1) with `.contains()`.

**Recommendation**: Change to `shell_keywords.contains(word.as_str())`.

### P2: Repeated `get_theme()` calls inside draw closure

**Location**: `src/ui.rs:577, 1135`

`get_theme()` acquires a mutex lock each time. Inside the draw closure, it's called once per frame. This is fine for 60fps, but the mutex could be avoided (see D5).

### P3: `all_display` and `all_tags` cloned on every filter recompute

**Location**: `src/ui.rs:387-393, 445-451`

```rust
let mut candidates: Vec<(usize, String, Vec<String>, Option<i64>)> = ...
```

When `should_recompute` is true, the entire `all_display` list is cloned into `candidates`. For large snippet libraries (1000+), this allocates on every keystroke (after debounce). Not critical but could be optimized with indices instead of cloned strings.

### P4: Syntax highlighter builds strings per token

**Location**: `src/ui.rs:211-319`

`highlight_command` allocates a new `String` for every token. For a 200-character command, this could mean 20+ allocations. Pre-computation mitigates this (only runs once), but for very large libraries, startup time could be noticeable.

---

## 6. Test Coverage Gaps

### Current Tests (14 total in `ui.rs`)

- `test_highlight_command_empty/simple/with_variable/with_quotes/with_flags/with_comment/with_escaped_char` — 7 tests, all trivial (just assert non-empty)
- `test_resolve_theme_dark/bright/light/unknown` — 4 tests
- `test_filter_state_toggle_sort_new/alpha` — 2 tests
- `test_is_ctrl_key_true/false_no_modifier/false_different_char` — 3 tests
- `test_terminate_functionality` — 1 test

### Missing Test Coverage

| Area | Gap |
|------|-----|
| `select_snippet_inner` | No tests — requires terminal mock, but integration tests could cover key flows |
| `prompt_variables_inner` | No tests — same terminal dependency |
| Fuzzy matching filtering | No tests for filter logic (debounce, sort modes, tag filtering) |
| Visual mode | No tests for multi-select copy |
| Mouse handling | No tests |
| Sort modes | Only toggle tests, not sorting behavior |
| Edge cases | Empty snippet list, very long command strings, unicode in commands |
| Error paths | Terminal too small, `terminal::size()` failure |

### Integration Tests

`tests/integration.rs` has **zero tests** for the TUI. All tests are CLI-level (library management, list, version). This is expected (TUI requires interactive terminal), but means the most complex code in the codebase has no automated testing.

---

## 7. Priority Ranking

| # | Issue | Severity | Category | Effort |
|---|-------|----------|----------|--------|
| B3 | Visual mode asymmetric boundary behavior | **Medium** | Bug | Low |
| P1 | Keyword matching uses linear scan (`iter().any()` vs `contains()`) | **Medium** | Performance | Trivial |
| D1 | 1416-line monolithic file | **Medium** | Design | Medium |
| D5 | `Mutex<Theme>` unnecessary for `Copy` type | **Low** | Design | Trivial |
| D2 | `_folders` parameter unused | **Low** | Dead code | Trivial |
| D3 | `list_display_mode` magic integer | **Low** | Design | Low |
| D4 | Return type conflates selection + copy | **Low** | Design | Low |
| D6 | `TERMINATE` flag never checked in TUI | **Low** | Dead code | Trivial |
| B2 | `Ctrl+D`/`Ctrl+F` identical behavior | **Low** | UX | Trivial |
| B6 | `c` keybinding conflict with vim convention | **Low** | UX | Trivial |
| B8 | `copied_message` only expired on keypress | **Negligible** | Bug | Trivial |
| S1 | No variable value sanitization | **Medium** | Security | Low |
| P3 | Repeated allocation on filter recompute | **Low** | Performance | Medium |
| P4 | Per-token string allocation in highlighter | **Negligible** | Performance | Low |

---

## 8. Recommendations

### Immediate (Low effort, high value)

1. **Fix P1**: Change `shell_keywords.iter().any(|kw| word == *kw)` to `shell_keywords.contains(word.as_str())` — one-line fix, measurable perf improvement.
2. **Fix D5**: Change `ACTIVE_THEME` from `Mutex<Theme>` to `LazyLock<Theme>`, update `get_theme()` to return `Theme` by value.
3. **Remove D2**: Remove `_folders` parameter from `select_snippet_inner` and `select_snippet` (also update callers in `commands/mod.rs`).
4. **Fix D3**: Replace `list_display_mode: usize` with `enum DisplayMode { Command, Description }`.

### Short-term (Medium effort)

5. **Refactor D1**: Split `ui.rs` into sub-modules: `ui/theme.rs`, `ui/highlight.rs`, `ui/variables.rs`, `ui/select.rs`.
6. **Improve test coverage**: Add unit tests for sort modes, filter logic, and theme resolution edge cases. Consider property-based testing for `highlight_command`.

### Medium-term (Design improvements)

7. **Improve D4**: Introduce `enum SnippetAction { Selected(usize), Copied(usize, String), Cancelled }` return type.
8. **Evaluate D6**: Either integrate `TERMINATE` into the event loop or remove it.
9. **Address S1**: Consider adding a shell-safe escaping option for variable values, or at minimum make the warning more prominent in the TUI.

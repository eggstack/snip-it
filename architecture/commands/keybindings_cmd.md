# keybindings_cmd — TUI Keybindings Reference

[← Back to Overview](../overview.md)

## Overview

`src/commands/keybindings_cmd.rs` (80 lines) prints the static TUI
keybindings reference. It is documentation-as-code: the single
human-readable list of Normal/Insert/theme-picker/variable-prompt
bindings, kept beside the TUI rather than in the TUI event loop.

## CLI surface

`snp keybindings` (alias `k`), `main.rs:105-107,673-675`. No flags,
no args, no JSON mode: `run() -> SnipResult<()>` prints ~70 lines to
stdout and returns `Ok`.

## Flow / steps

Single `run()` (`keybindings_cmd.rs:4`): sequential `println!` blocks —

1. Normal mode: motion (`h/j/k/l`, arrows, `gg/G`, `Ctrl+f/d/b/u`),
   visual (`v/V`), copy-and-quit (`y`), delete-with-confirm (`d`),
   insert (`i`), theme picker (`e`), quit (`q`), search (`/`),
   tag filter (`t`), sorts (`n/o/a/z`), clear filter (`x/c`).
2. Insert mode: `j/k`/arrow navigation, `Enter` select, `Esc` back,
   `/` search, `Backspace`.
3. Theme picker (`e`): filter, `j/k` live preview, paging, `gg/G`,
   `Enter` apply, `e/q` cancel-revert, `Esc` leave filter.
4. Variable prompt (modal, starts in Insert): cursor motion, field
   navigation (`Tab`/`Ctrl+d/u`), `Enter` save, `Esc` mode switch,
   Normal-mode extras (`0/$`, `x/Delete`, `a/A/I`, `d` hint toggle,
   `q` back to selector, `Ctrl+c` exit).

## Mutation vs read-only

Read-only (trivially): stdout only. No gate, no lock, no state.

## Auto-sync trigger

None. No runtime, no sync, no notification.

## Error / exit mapping

Infallible `Ok(())`; the only failure mode is a broken stdout pipe
(`println!` panic semantics, as for all print-only commands).
Exit 0 always on success.

## Key invariants

- This file is a **manual mirror** of the bindings in `src/ui/` —
  any TUI binding change must update both, or the reference lies.
  There is no compile-time check linking them.
- `Esc` is explicitly no-op at top level (quit uses `q`); `q` inside
  the variable prompt goes back to the selector rather than quitting.
- Keep output stable and greppable: tests and users match on the
  listed keys.

## Maintenance contract

When a binding changes in `src/ui/`, update this file in the same PR:
add/remove the line under the matching mode heading, keeping the
`key : action` two-space format so the output stays greppable. The
four headings (Normal / Insert / Theme Picker / Variable Prompt) must
stay in this order — onboarding docs link to them positionally. If a
new modal is added to the TUI, add a fifth section here rather than
folding its keys into an existing one.

## Relationship to doctor and shell docs

`doctor --check-shell` validates generated shell code, not these
bindings; `shell init` output references `snp select` modes, not TUI
keys. This file is the only place the full interactive binding set is
enumerated — `architecture/tui.md` and `architecture/ui.md` describe
the event loop and rendering, and defer key tables to this command.

## File / line references

- `run`: `src/commands/keybindings_cmd.rs:4`
- Bindings source of truth: `src/ui/mod.rs`, `src/ui/state.rs`
- Dispatch: `src/main.rs:105-107,673-675`

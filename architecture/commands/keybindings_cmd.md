# keybindings_cmd — TUI Keybindings Reference

[← Back to Overview](../overview.md)

## Overview

`src/commands/keybindings_cmd.rs` (80 lines) prints the static TUI
keybindings reference: 74 `println!` lines covering the Normal-mode,
Insert-mode, theme-picker, and variable-prompt bindings.

It is **documentation-as-code** — a hand-maintained mirror of the
bindings implemented in `src/ui/mod.rs` and `src/ui/variables.rs`. It
reads nothing, writes nothing, and has no flags.

Related: [../tui.md](../tui.md) (the event loop), [../ui.md](../ui.md)
(theme, highlight, variables), [shell_cmd.md](shell_cmd.md) (shell
integration — a different key surface).

## CLI surface

`snp keybindings` (alias `k`) — `src/main.rs:105-107`, dispatched at
`src/main.rs:673-675`.

No flags, no arguments, no JSON mode. `run() -> SnipResult<()>` prints
the 74-line reference to stdout and returns `Ok(())`.

## This reference is hand-maintained and can drift

There is **no compile-time or test-time link** between
`src/commands/keybindings_cmd.rs` and `src/ui/mod.rs`. The command is a
literal `println!` list; the TUI is a `match key.code` tree. Nothing
checks that they agree.

**Real drift was found in all three of the following when this document
was audited against the current source.** They are documented here
because the *source* of truth is the TUI, and the command's own text is
what is wrong:

| The command prints | What the TUI actually does | Site |
|--------------------|---------------------------|------|
| `Insert Mode:` → `j/k   : alternative navigation` | **Wrong.** The insert-mode match has no `Char('j')` or `Char('k')` arm at all. `j` and `k` are ordinary printable characters and are **typed into the filter text** like any other letter. | `src/ui/mod.rs:1695-1762` |
| `Theme Picker` → `gg    : first theme` | **Wrong.** The theme picker binds **`Ctrl+g`** for "first theme". `g` is not bound in the picker; only the normal-mode snippet list has a `gg` sequence. | `src/ui/mod.rs:1502` |
| The reference is ordered `Normal Mode:` first | The TUI loop **starts in insert mode** (`insert_mode = true` at loop entry), so insert mode is the mode a user is actually in when the list first appears. Normal mode is entered by pressing `Esc`. | `src/ui/mod.rs:598` |

A fourth gap: **`Tab` toggles the list display mode** in both normal and
insert mode (`src/ui/mod.rs:1758-1760`, `:1796-1798`), and the reference
does not mention it at all.

Treat the table above as the authority. When the TUI changes, this
command's text must change with it — and until it does, `snp keybindings`
is wrong.

## The actual bindings

### Normal mode (`src/ui/mod.rs:1767-1897`)

| Key | Action | Site |
|-----|--------|------|
| `h` / `←` | move up | `:1824-1826` |
| `j` / `↓` | move down | `:1827-1831` |
| `k` / `↑` | move up | `:1832-1834` |
| `l` / `→` | move down | `:1835-1839` |
| `Ctrl+g` | jump to top | `:1799-1802` |
| `g` `g` | jump to top (500 ms window) | `:1803-1822`, `GG_TIMEOUT_MS` at `:609` |
| `G` | jump to bottom | `:1823` |
| `Ctrl+f` / `Ctrl+d` / `PageDown` | page down (10 rows) | `:1680-1686` |
| `Ctrl+b` / `Ctrl+u` / `PageUp` | page up (10 rows) | `:1688-1693` |
| `v` | visual mode (character) | `:1574-1583` |
| `V` | visual mode (line — to the end of the list) | `:1585-1594` |
| `y` | copy and quit | `:1780-1788` (single) / `:1623-1673` (visual range) |
| `d` | delete the selected snippet (confirmed with `y`) | `:1789-1795`, confirmation at `:1557-1564` |
| `i` | insert mode | `:1776-1779` |
| `e` | open the theme picker | `:1891-1895` |
| `q` | quit | `:1768-1771` |
| `Esc` | **no-op** — quitting uses `q` | `:1888-1890` |
| `/` | search (enters insert mode) | `:1881-1887` |
| `t` | toggle the tag filter | `:1873-1880` |
| `n` / `o` / `a` / `z` | sort newest / oldest / a-z / z-a | `:1840-1865` |
| `x` / `c` | clear the filter and the tag filter | `:1866-1872` |
| `Tab` | toggle the list display mode | `:1796-1798` |
| `Ctrl+c` | exit | `:1566-1569` |

`v` and `V` are ignored when the list is empty, and they are **normal
-mode only** — in insert mode `v` and `V` are ordinary printable
characters (`src/ui/mod.rs:1571-1573`).

### Insert mode (`src/ui/mod.rs:1695-1762`)

The insert-mode match has exactly these arms:

| Key | Action | Site |
|-----|--------|------|
| `Esc` | leave insert mode (commits the typed text as the filter, or clears it when empty) | `:1697-1714` |
| `↓` | move down | `:1715-1717` |
| `↑` | move up | `:1718` |
| `Enter` | accept the selection | `:1719-1721` |
| `Backspace` | delete the last character | `:1722-1736` |
| any printable character | insert it into the filter | `:1737-1757` |
| `Tab` | toggle the list display mode | `:1758-1760` |

**There is no `j` / `k` navigation in insert mode** — they are inserted
as text. The reference's `j/k : alternative navigation` line is wrong.

### List display mode

`list_display_mode` starts at `0` (`src/ui/mod.rs:600`) and is toggled by
`Tab` in both normal and insert mode.

| Mode | Rendering | Site |
|------|-----------|------|
| `0` (default) | the highlighted command only | `src/ui/mod.rs:1104-1111` |
| `1` | `[description] ` prepended to the highlighted command | `src/ui/mod.rs:1087-1103` |

### Theme picker (`src/ui/mod.rs:1432-1553`)

Opened with `e` in normal mode.

| Key | Action | Site |
|-----|--------|------|
| `i` | enter the filter (insert mode) | `:1434-1437` |
| `j` / `↓` | next theme (live preview) | `:1478-1490` |
| `k` / `↑` | previous theme (live preview) | `:1491-1501` |
| **`Ctrl+g`** | **first theme** | `:1502-1512` |
| `G` | last theme | `:1513-1523` |
| `Ctrl+d` / `PageDown` | page down (10 themes) | `:1524-1537` |
| `Ctrl+u` / `PageUp` | page up (10 themes) | `:1538-1549` |
| `Enter` | save & apply the theme | `:1462-1477` (and `:1414-1429` in filter mode) |
| `e` / `q` | cancel & revert to the previous theme | `:1438-1461` |
| `Esc` | leave the filter (back to picker normal mode) | `:1371-1375` |

**`gg` is not bound in the theme picker** — only `Ctrl+g` is. The
reference's `gg : first theme` line is wrong.

In the picker's *filter* (insert) mode, `Esc` commits the typed filter
and returns to picker normal mode (`:1371-1375`), `j`/`k` still move
between themes (`:1376-1399`), `Backspace` edits the filter
(`:1400-1407`), printable characters append to it (`:1408-1413`), and
`Enter` saves and applies the theme (`:1414-1429`).

### Variable prompt (`src/ui/variables.rs`)

A separate modal (`src/ui/mod.rs:7`, `:17`) that opens after a snippet
with variables is selected. It is modal with INS/NOR modes and **starts
in INS** (`src/ui/variables.rs:261`).

Insert mode (`handle_insert_key`, `src/ui/variables.rs:508-594`):

| Key | Action | Site |
|-----|--------|------|
| printable characters | insert at the cursor | `:579-589` |
| `←` / `→` | move the cursor within the field | `:564-573` |
| `↑` / `↓` | move between variables | `:520-546` |
| `Tab` | next variable (wraps) | `:547-553` |
| `Ctrl+d` | next variable (clamps) | `:554-558` |
| `Ctrl+u` | previous variable (clamps) | `:559-563` |
| `Backspace` | delete the character before the cursor | `:574-578` |
| `Enter` | save the values | `:590` |
| `Esc` | switch to normal mode | `:517-519` |

Normal mode (`handle_normal_key`, `src/ui/variables.rs:596+`):

| Key | Action | Site |
|-----|--------|------|
| `h` / `←` | move the cursor left | `:628-632` |
| `l` / `→` | move the cursor right | `:633-637` |
| `0` / `$` | move to the start / end of the field | `:638-647` |
| `j` / `↓` | next variable | `:658-672` |
| `k` / `↑` | previous variable | `:673-691` |
| `Tab` | next variable (wraps) | `:685` |
| `x` / `Delete` | delete the character at the cursor | `:648-652` |
| `Backspace` | delete the character before the cursor | `:653-657` |
| `a` / `A` / `I` | insert mode (after / at end / at start) | `:610-627` |
| `d` | toggle the default-value hint | `:692-694` |
| `Enter` | save the values | `:695` (the loop breaks at `:470-472`) |
| `q` | back to the snippet selector (**not** quit) | `:606` |
| `Ctrl+c` | exit the program | `:452-455` |

## The visual-range multi-select copy is not fully audited

When `y` is pressed in visual mode, the TUI copies the **commands** of
every row in the range — each passed through `strip_escape_sequences` —
joined with `\n` (`src/ui/mod.rs:1635-1641`).

The copy goes through a **direct `clipboard::copy_to_clipboard_auto`
call** (`src/ui/mod.rs:1642`), which **bypasses
`commands::clip_cmd::copy_to_clipboard`** — the single implementation for
snippet copies that additionally:

- writes an audit-log entry via `crate::logging::audit_log("copy", snippet, None)`, and
- updates the usage index via `UsageIndex::record_use(&snippet.id)`.

Two consequences:

1. **No usage-index entry** is recorded for a visual-range copy.
2. **Only the first row of the range is audit-logged** — the audit call
   targets `filtered.get(start)` (`src/ui/mod.rs:1657-1668`), so the
   remaining rows in the range are copied without an audit entry.

This is the same wording used in [cron_cmd.md](cron_cmd.md), which
documents the other direct `copy_to_clipboard_auto` call site
(`src/commands/cron_cmd.rs:89`) — that one has **no** snippet to
attribute an audit entry to and therefore logs nothing at all.

## Mutation vs read-only

Read-only (trivially): stdout only. No gate, no lock, no state.

## Tokio runtime

None. `snp keybindings` never touches the global `RUNTIME`
(`src/main.rs:22-27`).

Startup policy: `Commands::Keybindings` uses
`StartupRecoveryPolicy::SuppressConfiguration` with
`StartupServices::Logging` (`src/main.rs:981-991`).

## Auto-sync trigger

None. No runtime, no sync, no notification.

## Error / exit mapping

Infallible `Ok(())`. The only failure mode is a broken stdout pipe
(`println!` panic semantics, as for all print-only commands). Exit 0 on
success.

## Key Files

| Path | Role |
|------|------|
| `src/commands/keybindings_cmd.rs` | The 74-line reference text |
| `src/ui/mod.rs` | The snippet-list TUI — normal/insert/theme-picker bindings |
| `src/ui/variables.rs` | The variable prompt — its own INS/NOR bindings |
| `src/clipboard.rs` | `copy_to_clipboard_auto` — the direct copy path |
| `src/commands/clip_cmd.rs:40-49` | `copy_to_clipboard` — the auditing path that is bypassed |
| `src/logging.rs` | `audit_log` |
| `src/main.rs:105-107`, `:673-675` | Clap wiring and dispatch |

## Invariants

- **This file is a manual mirror of `src/ui/`** — any TUI binding change
  must update both, or the reference lies. There is no compile-time
  check linking them, and real drift has been found (see above).
- **`Esc` is explicitly a no-op at the top level** — quitting uses `q`.
- **`q` inside the variable prompt returns to the selector**, it does not
  quit.
- **Keep the output stable and greppable** — tests and users match on the
  listed keys.
- **`v` / `V` are normal-mode only**, and are ignored on an empty list.

## Maintenance contract

When a binding changes in `src/ui/mod.rs` or `src/ui/variables.rs`,
update this command in the same PR:

1. Add/remove the line under the matching mode heading, keeping the
   `key : action` two-space format so the output stays greppable.
2. Keep the four headings (Normal / Insert / Theme Picker / Variable
   Prompt) in this order — onboarding docs link to them positionally.
3. If a new modal is added to the TUI, add a fifth section rather than
   folding its keys into an existing one.
4. **Re-read the `match key.code` arms in the TUI rather than editing
   from memory** — the drift above came from editing from memory.

## Gotchas

- **Three of the printed descriptions are factually wrong today**
  (insert-mode `j/k`, theme-picker `gg`, and the implied "you start in
  normal mode"). The TUI is the source of truth.
- **`Tab` (list display mode) is undocumented** in the reference.
- **`snp keybindings` is not a machine-readable contract** — it is prose
  in `println!` form, so it cannot be validated automatically.
- **This is the only place the full interactive binding set is
  enumerated.** `architecture/tui.md` and `architecture/ui.md` describe
  the event loop and rendering and defer the key tables here — which is
  exactly why the drift went unnoticed.

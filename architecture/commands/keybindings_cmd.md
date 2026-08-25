# keybindings_cmd — Keybindings Reference

## Overview

`keybindings_cmd` prints all available keybindings and their actions to stdout.

## Entry Point

```rust
pub fn run() -> SnipResult<()>
```

## Flow

1. Load keybindings configuration (or use defaults)
2. Print all keybinding categories to stdout
3. Return immediately

## Keybinding Categories

### Navigation
| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `h` / `←` | Move left |
| `l` / `→` | Move right |
| `gg` / `Ctrl+g` | Jump to top |
| `G` | Jump to bottom |
| `Ctrl+f` | Page down |
| `Ctrl+d` | Page down (helix) |
| `Ctrl+b` | Page up |
| `Ctrl+u` | Page up (helix) |

### Actions
| Key | Action |
|-----|--------|
| `Enter` | Select / execute |
| `y` | Copy and quit |
| `d` | Delete selected snippet (confirm with `y`) |
| `i` | Enter insert mode |
| `e` | Open theme picker |
| `/` | Search |
| `v` | Visual mode (character) |
| `V` | Visual mode (line) |

### Filtering & Sorting
| Key | Action |
|-----|--------|
| `t` | Toggle tag filter |
| `n` | Sort by newest |
| `o` | Sort by oldest |
| `a` | Sort a-z |
| `z` | Sort z-a |
| `x` / `c` | Clear filter |

### Quit
| Key | Action |
|-----|--------|
| `q` | Quit |
| `Esc` | No-op |

### Insert Mode
| Key | Action |
|-----|--------|
| `j` / `k` | Alternative navigation |
| `↑` / `↓` | Move up / down |
| `Enter` | Select / execute |
| `Esc` | Return to normal mode |
| `/` | Start search |
| `Backspace` | Delete character |

### Theme Picker (opened with `e` in normal mode)
| Key | Action |
|-----|--------|
| `i` | Filter (insert mode) |
| `j` / `↓` | Next theme (live preview) |
| `k` / `↑` | Previous theme (live preview) |
| `Ctrl+d` / `PageDown` | Page down (10 themes) |
| `Ctrl+u` / `PageUp` | page up (10 themes) |
| `gg` | First theme |
| `G` | Last theme |
| `Enter` | Save & apply theme |
| `e` / `q` | Cancel & revert to previous theme |
| `Esc` | Leave filter (back to picker normal mode) |

### Variable Prompt (modal INS/NOR, starts in INS)
| Key | Action |
|-----|--------|
| **Insert Mode (default)** | |
| type | Insert at cursor (all printable chars incl. `q`) |
| `←` / `→` | Move cursor within field |
| `↑` / `↓` | Move between variables |
| `Tab` / `Ctrl+d` | Next variable (Tab wraps, Ctrl+d clamps) |
| `Ctrl+u` | Previous variable (clamps) |
| `Backspace` | Delete char before cursor |
| `Enter` | Save values |
| `Esc` | Switch to normal mode |
| **Normal Mode** | |
| `h` / `←` | Move cursor left |
| `l` / `→` | Move cursor right |
| `0` / `$` | Move cursor to start / end of field |
| `j` / `↓` | Next variable |
| `k` / `↑` | Previous variable |
| `Tab` | Next variable (wraps) |
| `x` / `Delete` | Delete char at cursor |
| `Backspace` | Delete char before cursor |
| `a` / `A` / `I` | Insert mode (after / at end / at start) |
| `d` | Toggle default-value hint |
| `Enter` | Save values |
| `q` | Back to snippet selector (NOT quit) |
| `Ctrl+c` | Exit program |

## Customization

Keybindings are currently not user-configurable (defined in TUI state machine). Future versions may support `~/.config/snp/keybindings.toml`.

## Related

- [tui.md](../tui.md) — Full TUI state machine and event handling
- [mod.md](mod.md) — Shared helpers

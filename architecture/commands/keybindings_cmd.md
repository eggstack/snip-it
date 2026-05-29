# keybindings_cmd — Keybindings Reference

## Overview

`keybindings_cmd` displays a TUI-based help screen showing all available keybindings and their actions.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. Load keybindings configuration (or use defaults)
2. Render TUI help dialog
3. Display until user presses `q` or `Esc`

## Keybinding Categories

### Navigation
| Key | Action |
|-----|--------|
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `h` / `←` | Move left (visual mode) |
| `l` / `→` | Move right (visual mode) |
| `gg` | Jump to top |
| `G` | Jump to bottom |
| `Ctrl+f` / `PageDown` | Page down |
| `Ctrl+b` / `PageUp` | Page up |
| `Ctrl+u` | Half page up |
| `Ctrl+d` | Half page down |

### Actions
| Key | Action |
|-----|--------|
| `Enter` | Select / execute |
| `y` | Copy to clipboard |
| `/` | Start search |
| `v` | Visual mode |
| `V` | Visual line mode |

### Filtering & Sorting
| Key | Action |
|-----|--------|
| `t` | Tag filter mode |
| `n` | Sort by name |
| `o` | Sort by date |
| `a` | Sort by usage |
| `z` | Toggle display mode |

### Quit
| Key | Action |
|-----|--------|
| `q` | Quit |
| `Esc` | Quit / Cancel |

## Customization

Keybindings are currently not user-configurable (defined in TUI state machine). Future versions may support `~/.config/snp/keybindings.toml`.

## Related

- [tui.md](../tui.md) — Full TUI state machine and event handling
- [mod.md](mod.md) — Shared helpers

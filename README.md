# snp - Snippet Manager

A fast, terminal-based snippet manager with fuzzy search, clipboard support, and variable expansion.

## Features

- **Fuzzy Search** - Quickly find snippets by description, command, or tags
- **Clipboard Support** - Copy snippets to clipboard with a single keypress
- **Variable Expansion** - Use `<varname=default>` syntax for dynamic snippets
- **TUI Interface** - Clean terminal UI with keyboard navigation
- **Cross-Platform** - Works on macOS, Linux, and Windows

## Installation

### From Source

```bash
cargo build --release
cp target/release/snp ~/.local/bin/
```

### Using Homebrew

```bash
brew install snp
```

## Configuration

Snippets are stored in `~/.config/snp/snippets.toml` (or `$XDG_CONFIG_HOME/snp/snippets.toml`).

### Snippet Format

```toml
[[Snippets]]
Description = "git commit with message"
Output = ""
Tag = ["git", "version-control"]
command = "git commit -m \"<message>\""

[[Snippets]]
Description = "ssh to server"
Output = ""
Tag = ["ssh", "server"]
command = "ssh <user@host>"

[[Snippets]]
Description = "docker cleanup"
Output = ""
Tag = ["docker", "cleanup"]
command = "docker system prune -af"
```

### Variable Syntax

Variables use `<name=default>` or `<name>` syntax:

- `<name=default>` - Shows default value, user can override
- `<name>` - Prompts for input with no default

## Usage

```
snp --help
```

### Commands

| Command | Alias | Description |
|---------|-------|--------------|
| `snp new` | `snp n` | Create a new snippet |
| `snp list` | `snp l` | List all snippets |
| `snp run` | `snp r` | Run a snippet via TUI |
| `snp clip` | `snp c` | Copy snippet to clipboard |
| `snp search` | `snp s` | Search and view snippet details |
| `snp edit` | `snp e` | Edit snippets file in $EDITOR |
| `snp sync` | `snp s` | Sync snippets with server |
| `snp premade` | `snp p` | Browse/download premade libraries |
| `snp version` | `snp v` | Show version |

### Sync

Sync your local snippets with the server:

```bash
snp sync
```

This will push local changes to the server and pull remote changes. Conflicts are handled interactively.

#### Configuration

After first sync, a configuration section is added to your snippets file:

```toml
[settings.sync]
enabled = true
server_url = "https://your-server.com"
api_key = "your-api-key"
device_id = "your-device-id"
```

### Premade Libraries

Browse and download pre-built snippet collections:

```bash
# List available premade libraries
snp premade list

# Download a specific library
snp premade get docker-essentials

# Sync all premade libraries
snp premade sync
```

Premade libraries are stored in `~/.config/snp/premade/`.

### Automated Sync (Cron)

To automatically sync snippets on a schedule, use the built-in cron command:

```bash
# Set up sync every 15 minutes (default)
snp cron

# Set up sync every 30 minutes
snp cron -i 30

# Set up sync every hour
snp cron -i 60
```

The command will display the crontab entry, show platform-specific instructions, and offer to copy it to clipboard.

The `--non-interactive` flag (used by cron) skips conflict prompts and keeps local versions.

### TUI Keybindings

#### Normal Mode
| Key | Action |
|-----|--------|
| `↑/↓` or `j/k` | Navigate snippets |
| `Enter` | Run selected snippet |
| `i` | Enter insert mode (filter) |
| `y` | Copy to clipboard and quit |
| `q` | Quit |
| `n` | Sort by newest |
| `o` | Sort by oldest |
| `a` | Sort A-Z |
| `z` | Sort Z-A |
| `t` | Toggle tag filter mode |
| `d` | Clear filter |
| `Ctrl+C` | Copy to clipboard |
| `Ctrl+D` | Page down |
| `Ctrl+U` | Page up |

#### Insert Mode
| Key | Action |
|-----|--------|
| `Esc` | Return to normal mode |
| `↑/↓` or `j/k` | Navigate snippets |
| `Enter` | Run selected snippet |
| `Backspace` | Delete character |
| `Type` | Filter snippets |

## License

MIT License - see [LICENSE](LICENSE) for details.

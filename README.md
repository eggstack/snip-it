# snp - Snippet Manager

![Rust](https://img.shields.io/badge/Rust-1.81+-orange.svg)
![License](https://img.shields.io/badge/License-MIT-blue.svg)
[![CI](https://github.com/anomalyco/snip-it/actions/workflows/ci.yml/badge.svg)](https://github.com/anomalyco/snip-it/actions/workflows/ci.yml)

A fast, terminal-based snippet manager with fuzzy search, clipboard support, variable expansion, and cloud sync.

## Features

- **Fuzzy Search** - Quickly find snippets by description, command, or tags
- **Clipboard Support** - Copy snippets to clipboard with a single keypress
- **Variable Expansion** - Use `<varname=default>` syntax for dynamic snippets
- **TUI Interface** - Clean terminal UI with keyboard navigation
- **Cloud Sync** - End-to-end encrypted sync between devices
- **Snippet Libraries** - Organize snippets into multiple collections
- **Premade Libraries** - Download community-built snippet collections
- **Cross-Platform** - Works on macOS, Linux, and Windows

## Installation

### From crates.io

```bash
cargo install snp
```

### Using Homebrew (macOS)

```bash
brew install anomalyco/tap/snp
```

### From Source

```bash
cargo build --release
cp target/release/snp ~/.local/bin/
```

## Quick Start

```bash
# Create a snippet with variables
snp new 'ssh <user>@<host>' -t ssh

# Create a snippet with a default value
snp new 'git push origin <branch=main>' -t git

# List all snippets
snp list

# Run a snippet (opens TUI)
snp run

# Copy to clipboard
snp clip

# Search snippets
snp search
```

## Security Warning

**Snippet commands are executed as-is via your shell.** Only add snippets you trust. Avoid storing snippets with sensitive data (passwords, tokens, API keys) as they are stored in plaintext TOML files.

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

```toml
# With defaults
[[Snippets]]
Description = "SSH with port"
command = "ssh <user=root>@<host> -p <port=22>"

# Required input
[[Snippets]]
Description = "Docker run"
command = "docker run -it <image> /bin/bash"
```

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
| `snp sync` | `snp y` | Sync snippets with server |
| `snp cron` | | Setup automatic sync |
| `snp library` | `snp lib` | Manage snippet libraries |
| `snp premade` | `snp p` | Browse/download premade libraries |
| `snp register` | `snp reg` | Register sync account |
| `snp version` | `snp v` | Show version |

## Libraries

Organize snippets into multiple collections:

```bash
# Create a new library
snp library create work-snippets

# List all libraries
snp library list

# Switch to a library
snp run --library work-snippets

# Delete a library
snp library delete work-snippets

# Set primary library
snp library set-primary work-snippets
```

## Sync

Sync your local snippets with a server for cross-device access.

### Register and Connect

```bash
# Register with a sync server
snp register https://your-sync-server.com

# Or use default local server
snp register
```

### Sync Operations

```bash
# Manual sync
snp sync

# Push only (upload local changes)
snp sync --push-only

# Pull only (download remote changes)
snp sync --pull-only

# List connected servers
snp sync --servers
```

### Automated Sync (Cron)

Set up automatic periodic sync:

```bash
# Set up sync every 15 minutes (default)
snp cron

# Set up sync every 30 minutes
snp cron -i 30

# Set up sync every hour
snp cron -i 60
```

The command displays the crontab entry, platform-specific instructions, and offers to copy it to clipboard.

Use `--non-interactive` flag for headless sync (skips conflict prompts, keeps local versions).

### Sync Configuration

After first sync, a configuration file is created at `~/.config/snp/sync.toml`:

```toml
[settings.sync]
enabled = true
server_url = "https://your-server.com"
api_key = "your-api-key"
device_id = "your-device-id"
sync_interval_minutes = 30
auto_sync = false
sync_direction = "Bidirectional"  # Push, Pull, or Bidirectional
```

## Premade Libraries

Browse and download pre-built snippet collections from the community:

```bash
# List available premade libraries
snp premade list

# Download a specific library
snp premade get docker-essentials

# Download all available libraries
snp premade get all

# Sync all premade libraries (download missing)
snp premade sync
```

Premade libraries are stored in `~/.config/snp/premade/`.

## TUI Keybindings

### Normal Mode

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

### Insert Mode (Filter)

| Key | Action |
|-----|--------|
| `Esc` | Return to normal mode |
| `↑/↓` or `j/k` | Navigate snippets |
| `Enter` | Run selected snippet |
| `Backspace` | Delete character |
| `Type` | Filter snippets |

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SNP_CONFIG_HOME` | Override config directory | `~/.config/snp` |
| `SNP_COMMAND_TIMEOUT` | Command execution timeout (seconds) | `300` |
| `SNP_CLIPBOARD_TIMEOUT` | Clipboard operation timeout (seconds) | `5` |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | Allow API key in config file (not keychain) | `false` |
| `SNP_THEME` | UI theme (`dark` or `bright`) | `dark` |
| `SNP_LOG_LEVEL` | Log level (`trace`, `debug`, `info`, `warn`, `error`) | `info` |
| `SNP_LOG` | Per-module log filter (e.g., `snp=debug`) | - |
| `EDITOR` | Editor for `snp edit` command | - |

## Troubleshooting

### Snippets not saving

Ensure the config directory exists and is writable:

```bash
mkdir -p ~/.config/snp
chmod 755 ~/.config/snp
```

### Clipboard not working

- **macOS**: Grant Terminal access to Clipboard in System Preferences
- **Linux**: Install `xclip` or `xsel`
- **Windows**: Should work automatically

### Sync conflicts

When the same snippet is modified on multiple devices:

- **Interactive mode**: Prompts to choose version
- **Non-interactive mode**: Keeps local version by default

### Variable expansion issues

If variables aren't expanding correctly, check for:

- Missing `>` closing bracket: `<var` should be `<var>`
- Escaped characters: `\<` is treated as literal `<`

## Security

- **Sync encryption**: All snippets are encrypted with AES-256-GCM before sync
- **Key derivation**: API keys are hashed with Argon2
- **Transport**: gRPC over TLS when server supports it

## License

MIT License - see [LICENSE](LICENSE) for details.

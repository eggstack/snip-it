# snp User Guide

Advanced topics and detailed usage documentation for snp (Snippet Manager).

## Table of Contents

- [Snippet Libraries](#snippet-libraries)
- [Cloud Sync](#cloud-sync)
- [Premade Libraries](#premade-libraries)
- [Variable Expansion](#variable-expansion)
- [Shell Keyword Expansion](#shell-keyword-expansion)
- [Import/Export](#importexport)
- [Configuration Reference](#configuration-reference)
- [Advanced Usage](#advanced-usage)
- [Programmatic Usage](#programmatic-usage)
- [Troubleshooting](#troubleshooting)
  - [Reset and Recovery](#reset-and-recovery)

---

## Snippet Libraries

Libraries allow you to organize snippets into separate collections, perfect for work/home separation or project-specific snippets.

### Creating Libraries

```bash
# Create a new library
snp library create work

# Library files are stored in ~/.config/snp/libraries/
```

### Managing Libraries

```bash
# List all libraries
snp library list
# Output:
#   work (primary)
#   personal
#   docker-essentials (premade)

# Set a library as primary
snp library set-primary work

# View library details
snp library show work
# Output:
#   Name: work
#   ID: abc123
#   Primary: yes
#   Last Sync: 2024-01-15 10:30:00

# Delete a library
snp library delete old-library
snp library delete old-library --force  # Skip confirmation
```

### Using Libraries

```bash
# Run snippet from specific library
snp run --library work

# Copy from specific library
snp clip --library work

# Create snippet in specific library
snp new --library work
```

### Library File Format

Library files are TOML with the `.toml` extension, stored in `~/.config/snp/libraries/`:

```toml
# ~/.config/snp/libraries/work.toml

[[Snippets]]
Description = "Deploy application"
Tag = ["deploy", "k8s"]
command = "kubectl apply -f deployment.yaml"

[[Snippets]]
Description = "Check pod status"
Tag = ["k8s", "monitoring"]
command = "kubectl get pods -n <namespace=default>"
```

Library metadata (ID, primary status, sync state) is stored separately in `~/.config/snp/libraries.toml`.

---

## Cloud Sync

Sync snippets across multiple devices using a snip-sync server.

### Setting Up Sync

#### 1. Run a Sync Server

```bash
# Using the included snip-sync server
cd snip-sync
cargo build --release
./target/release/snip-sync
```

#### 2. Register Your Client

```bash
# Register with default local server
snp register

# Register with custom server
snp register --server https://sync.example.com:50051
```

This creates an API key and stores it in the OS keychain when available.
`sync.toml` stores the `@keychain` marker instead of the key itself.

#### 3. Sync Your Snippets

```bash
# Sync using sync.toml direction (default: push local changes)
snp sync

# Push local changes only
snp sync --push-only

# Pull remote changes only
snp sync --pull-only
```

### Sync Modes

Configure sync direction in `~/.config/snp/sync.toml`:

```toml
[settings.sync]
sync_direction = "Push"         # Default: upload only
# sync_direction = "Bidirectional"  # Upload and download
# sync_direction = "Pull"        # Download only
```

### Automated Sync

#### Cron Setup

```bash
# Set up 15-minute sync (default)
snp cron

# Custom interval
snp cron -i 30    # Every 30 minutes
snp cron -i 60    # Every hour
snp cron -i 240   # Every 4 hours
```

#### Systemd Timer (Linux)

```bash
# Create user timer
cat > ~/.config/systemd/user/snp-sync.timer << 'EOF'
[Unit]
Description=Snippet sync timer

[Timer]
OnBootSec=5min
OnUnitActiveSec=15min
Unit=snp-sync.service

[Install]
WantedBy=default.target
EOF

# Create service
# NOTE: replace 'your-username' below with the actual username, or use
# systemd template syntax (%i expands to the user instance name) for a
# per-user service.
cat > ~/.config/systemd/user/snp-sync.service << 'EOF'
[Unit]
Description=Snippet sync

[Service]
Type=oneshot
User=%i
ExecStart=/home/%i/.local/bin/snp sync

[Install]
WantedBy=default.target
EOF

# Enable timer
systemctl --user daemon-reload
systemctl --user enable --now snp-sync.timer
```

### Sync Conflict Resolution

When the same snippet is modified locally and remotely, sync uses
last-write-wins based on the `updated_at` timestamp. Local-only fields
such as `output`, `folders`, and `favorite` are preserved when the server
version wins.

When linking an existing local library to a server library that already
contains snippets, `snp sync` prompts you to skip, overwrite the local
library with the server version, or rename the local copy before pulling.

---

## Premade Libraries

Download community-built snippet collections.

### Browsing Libraries

```bash
# List available premade libraries
snp premade list
# Output:
#   Name                    Snippets
#   docker-essentials       15
#   git-common              23
#   networking              31

# Get details for a library
snp premade get docker-essentials  # Shows what will be installed
```

### Installing Libraries

```bash
# Install a specific library
snp premade get docker-essentials

# Install all available libraries
snp premade get all

# Install another library by repeating the command
snp premade get git-common
```

### Syncing Premade Libraries

```bash
# Download any missing premade libraries
snp premade sync
```

### Creating Premade Libraries (Server Side)

Server administrators can add premade libraries by placing TOML files in the server's `premade-libraries/` directory:

```toml
# premade-libraries/my-collection.toml
Description = "My custom collection"

[[Snippets]]
Description = "Build project"
Tag = ["build"]
command = "cargo build --release"

[[Snippets]]
Description = "Run tests"
Tag = ["test"]
command = "cargo test"
```

---

## Variable Expansion

Dynamic snippets with user input at runtime.

### Syntax

| Pattern | Behavior |
|---------|----------|
| `<varname>` | Required input, no default |
| `<varname=default>` | Optional with default value |
| `<var1> <var2>` | Multiple variables |
| `\<` or `\>` | Escaped characters (literal) |

### Examples

#### Required Input

```toml
[[Snippets]]
Description = "Docker exec"
command = "docker exec -it <container> /bin/bash"
```

Prompts for container name each run.

#### With Defaults

```toml
[[Snippets]]
Description = "SSH connection"
command = "ssh <user=root>@<host> -p <port=22>"
```

Shows defaults, allows override.

#### Escaped Characters

```toml
[[Snippets]]
Description = "Less than comparison"
command = "test \<num1\> -lt \<num2\>"
```

The `\<` and `\>` are treated as literal characters.

### Shell Keywords

Automatically expand common shell patterns:

```toml
[[Snippets]]
Description = "Git with auto-completion"
command = "git checkout <branch>"
```

Supports: `$HOME`, `$USER`, `~`, current date/time, etc.

---

## Shell Keyword Expansion

snip automatically expands common shell patterns when copying or running snippets.

### Supported Keywords

| Pattern | Expansion |
|---------|-----------|
| `$HOME` | User's home directory |
| `~` | User's home directory |
| `$USER` | Current username |
| `$HOSTNAME` | Machine hostname |
| `$(date)` | Current date (YYYY-MM-DD) |
| `$(date +%H:%M)` | Current time |
| `$PWD` | Current directory |
| `$RANDOM` | Random number |

### Examples

```toml
[[Snippets]]
Description = "Navigate to home"
command = "cd $HOME"

[[Snippets]]
Description = "Create backup"
command = "cp <file> ~/backups/backup-$(date).tar.gz"
```

---

## Import/Export

### Import from `pet` (or `navi`)

snp accepts the same TOML schema as `pet`; copy the file in place and
you're done:

```bash
# Typical pet config location: ~/.config/pet/snippets.toml
cp ~/.config/pet/snippets.toml ~/.config/snp/snippets.toml
# (note: file is renamed, but the inner TOML is read as-is)
```

`snp` reads the legacy `pet` keys (`Command`, `Description`, `Tag`,
`Output`) and the modern lowercase keys (`command`, `description`,
`tags`, `output`); both work in the same file.

### Import from snip (Python version)

The snip format is compatible:

```toml
# snip format (works with snp)
[[snips]]
command = "git commit"
description = "Git commit"
tags = ["git"]

# snp format (new)
[[Snippets]]
command = "git commit"
Description = "Git commit"
Tag = ["git"]
```

### Export Snippets

```bash
# Copy your snippets.toml to backup
cp ~/.config/snp/snippets.toml ~/snippets-backup.toml

# Export specific library
cp ~/.config/snp/libraries/work.toml ~/work-snippets.toml
```

### Manual Import

1. Copy the TOML file to the appropriate location
2. Edit to ensure format compatibility
3. Restart snp

---

## Configuration Reference

### File Locations

| Platform | Path |
|----------|------|
| Linux | `~/.config/snp/` |
| macOS | `~/.config/snp/` |
| Windows | `%APPDATA%\snp\` |

### Config Files

| File | Purpose |
|------|---------|
| `snippets.toml` | Main snippet storage |
| `sync.toml` | Sync server settings |
| `libraries/` | Snippet libraries |
| `libraries.toml` | Library metadata and sync state |
| `premade/` | Downloaded premade libraries |
| `logs/` | Application logs |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `XDG_CONFIG_HOME` | Override config root; snp uses `$XDG_CONFIG_HOME/snp` |
| `SNP_COMMAND_TIMEOUT` | Command execution timeout in seconds; `0` disables |
| `SNP_CLIPBOARD_TIMEOUT` | Clipboard operation timeout in seconds |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | Allow sync API key storage in `sync.toml` when the OS keychain is unavailable |
| `SNP_SYNC_CONNECT_TIMEOUT` | Sync TCP connect timeout in seconds |
| `SNP_SYNC_REQUEST_TIMEOUT` | Sync per-request timeout in seconds |
| `SNP_THEME` | UI theme (`dark`, `bright`, `light`, `auto`, or a Halloy theme name) |
| `EDITOR` | Editor for `snp edit` command |
| `SHELL` | Shell used to run snippets on Unix |
| `COMSPEC` | Shell used to run snippets on Windows |
| `SNP_LOG` | Per-module tracing filter, for example `snp=debug,snip_sync=info` |
| `RUST_LOG` | Standard tracing/env-filter fallback when `SNP_LOG` is unset |
| `SNIP_SYNC_ALLOW_HTTP` | Allow plaintext HTTP for local sync server development only |

### TOML Configuration Format

```toml
# sync.toml

[settings]
[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "@keychain"
device_id = "device-uuid"
sync_interval_minutes = 30
auto_sync = false
sync_direction = "Push"
sync_limit = 1000
clipboard_auto_clear_seconds = 30
```

```toml
# snippets.toml or libraries/<name>.toml
[[Snippets]]
Id = "optional-uuid"
Description = "Snippet description"
Output = ""  # For storing command output
Tag = ["tag1", "tag2"]
command = "the command with <variables>"
folders = []
favorite = false
created_at = 1705312200
updated_at = 1705312200
device_id = ""
deleted = false
```

### Settings Reference

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `settings.sync.enabled` | bool | false | Enable sync |
| `settings.sync.server_url` | string | https://localhost:50051 | Sync server URL |
| `settings.sync.api_key` | string | "" | API key for auth; normally `@keychain` |
| `settings.sync.device_id` | string | "" | Unique device identifier |
| `settings.sync.sync_interval_minutes` | u32 | 30 | Sync interval |
| `settings.sync.auto_sync` | bool | false | Auto-sync on changes |
| `settings.sync.sync_direction` | enum | Push | Push, Pull, or Bidirectional |
| `settings.sync.sync_limit` | integer | 1000 | Max snippets requested per sync page |
| `settings.sync.clipboard_auto_clear_seconds` | integer/null | null | Optional clipboard auto-clear delay |

---

## Advanced Usage

### Using with Shell History

Combine with shell integration:

```bash
# Add to .bashrc or .zshrc
alias snip='eval "$(snp run)"'
```

### Hook Scripts

Run scripts before/after sync using shell wrappers:

```bash
# ~/.local/bin/snp-with-hooks
#!/bin/bash
snp sync "$@"
[ $? -eq 0 ] && ~/.local/bin/snp-post-sync
```

### Performance Tips

- **Large libraries**: Use `--filter` to narrow search
- **Many snippets**: Organize into multiple libraries
- **Slow sync**: Increase `sync_interval_minutes`

---

## Programmatic Usage

snp is a binary application and does not expose a public Rust API. For automation, use the CLI:

```bash
# Create a snippet
snp new "git commit" --tags

# List snippets (JSON format for scripting)
snp list --json

# Run a snippet non-interactively (with filter)
snp run --filter "deploy"

# Copy to clipboard
snp clip --filter "ssh"

# Sync programmatically
snp sync
```

### Shell Integration

```bash
# Add to .bashrc or .zshrc for quick snippet access
alias snp-run='eval "$(snp run)"'

# Use in scripts
SNIPPET=$(snp list --json | jq -r '.[0].command')
eval "$SNIPPET"
```

---

## Troubleshooting

### Sync Not Working

1. Check server is running: `curl localhost:50050/health` or equivalent
2. Verify sync config: `cat ~/.config/snp/sync.toml`
3. Check logs: `tail -f ~/.config/snp/logs/snp.log`
4. Test connection: `snp sync --dry-run`

### TUI Rendering Issues

- Resize terminal window
- Check $TERM variable: `echo $TERM`
- Try different terminal emulator

### Slow Startup

- Reduce log level: `SNP_LOG=snp=error snp run`
- Disable auto-sync: Set `auto_sync = false`

### Data Recovery

If snippets are lost:

1. Check `~/.config/snp/snippets.toml.bak`
2. Check `~/.config/snp/libraries/` for backups
3. Restore from server: `snp sync --pull-only`

### Reset and Recovery

To wipe all local data and start fresh:

```bash
# Remove all snippets, sync settings, libraries, logs, and audit log
rm -rf ~/.config/snp

# Next invocation will recreate the directory with default permissions
snp --version
```

`rm -rf` is destructive. Back up first if you intend to keep anything:

```bash
mv ~/.config/snp ~/.config/snp.backup-$(date +%Y%m%d)
```

To reset just sync state (keep snippets, drop the server connection):

```bash
rm ~/.config/snp/sync.toml
snp register --server https://your-server:50051
```

### Keychain Issues (Linux headless / SSH sessions)

On Linux, the `keyring` crate requires a running Secret Service
(GNOME Keyring, KWallet, or similar). If unavailable, snp logs an
error and refuses to write the API key in plaintext by default.

Fallback for headless / CI usage:

```bash
export SNP_ALLOW_PLAINTEXT_API_KEY=true
snp register --server https://your-server:50051
# API key is now stored in sync.toml with a runtime warning emitted
```

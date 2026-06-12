# snp

[![Crates.io](https://img.shields.io/crates/v/snip-it.svg)](https://crates.io/crates/snip-it)
[![Downloads](https://img.shields.io/crates/d/snip-it.svg)](https://crates.io/crates/snip-it)
[![docs.rs](https://img.shields.io/docsrs/snip-it)](https://docs.rs/snip-it)
![MSRV: 1.88](https://img.shields.io/badge/MSRV-1.88-blue)
![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)
![Rust: 1.88+](https://img.shields.io/badge/Rust-1.88+-orange.svg)

A fast, terminal-based snippet manager. Fuzzy search your command library,
expand `<variable>` placeholders on the fly, organize snippets into
libraries, and keep every device in sync with end-to-end encryption.

## Demo

![snp demo](assets/demo.gif)

> Recording of: launching the TUI, fuzzy-filtering to a snippet, expanding
> a variable, and running the command.
> Regenerate with `vhs demo.tape` (see `assets/demo.tape`).

## Why snp?

Your shell history is unorganized. Generic snippet tools like `pet` don't
sync. Cloud-first managers want your data. **snp** is a single Rust
binary that runs everywhere, stores snippets in plain TOML you can grep,
and adds *optional* end-to-end-encrypted sync without ever holding your
plaintext.

## Installation

### From crates.io (recommended)

```bash
cargo install snip-it
```

### From source

```bash
git clone <repository-url>
cd snip-it
cargo build --release
./target/release/snp --version
```

### Pre-built binaries

Download from the latest release (available after first crates.io publish):

| Platform       | Asset                                    |
| -------------- | ---------------------------------------- |
| Linux x86_64   | `snp-linux-x86_64.tar.gz`                |
| Linux aarch64  | `snp-linux-aarch64.tar.gz`               |
| macOS x86_64   | `snp-macos-x86_64.tar.gz`                |
| macOS Apple Si | `snp-macos-aarch64.tar.gz`               |
| Windows x86_64 | `snp-windows-x86_64.exe.zip`             |

```bash
# Linux / macOS
tar -xzf snp-linux-x86_64.tar.gz
sudo mv snp /usr/local/bin/
```

### Homebrew

```bash
brew install anomalyco/tap/snp
```

### Docker (sync server only)

```bash
docker pull ghcr.io/anomalyco/snip-it/snip-sync:latest
```

## Quickstart

```bash
# Create a snippet with variables
snp new 'ssh <user>@<host>' -t ssh

# Create a snippet with a default value
snp new 'git push origin <branch=main>' -t git

# List all snippets
snp list

# Launch the TUI to search, run, or copy
snp run
snp clip

# Open the snippets file in $EDITOR
snp edit
```

Snippets live in `~/.config/snp/snippets.toml`:

```toml
[[Snippets]]
Description = "git commit with message"
Tag = ["git", "version-control"]
command = "git commit -m \"<message>\""

[[Snippets]]
Description = "ssh with port"
Tag = ["ssh"]
command = "ssh <user=root>@<host> -p <port=22>"
```

## Features

- **Fuzzy search** — find snippets by description, command, or tags (`skim` algorithm).
- **Variable expansion** — `<name=default>` syntax prompts for values at runtime.
- **TUI** — keyboard-driven selection with themes (`SNP_THEME=dark|bright`).
- **Multiple libraries** — separate collections per project, environment, or client.
- **Premade libraries** — download community-built snippet packs (`snp premade sync`).
- **End-to-end encrypted sync** — AES-256-GCM + Argon2id; the server never sees plaintext.
- **Cron-friendly** — `snp sync` is non-interactive by default; safe for headless setups.
- **TOML you can grep** — snippets are plain files; version-control them, edit them, diff them.
- **Cross-platform** — macOS, Linux, Windows; uses the system clipboard and keychain.
- **Shell keyword expansion** — `$HOME`, `~`, `$(date)`, etc. expand at copy time.

## Security

> **Snippet commands are executed as-is via your shell.** Only add snippets
> you trust. Snippets that contain secrets (passwords, tokens, keys) live
> in plaintext TOML — use a sync server with end-to-end encryption rather
> than sharing the file.

- Sync: snippets are encrypted with **AES-256-GCM** before leaving the
  client; the server stores only ciphertext.
- Key derivation: **Argon2id** with OWASP-recommended parameters.
- API keys: stored in the OS keychain (macOS Keychain, GNOME Keyring,
  Windows Credential Manager) by default. A `SNP_ALLOW_PLAINTEXT_API_KEY=true`
  opt-in falls back to a plaintext key with a warning.
- Integrity: `sync.toml` carries a CRC32 checksum comment to detect
  accidental corruption (e.g., partial writes).

See [SECURITY.md](SECURITY.md) for the vulnerability disclosure policy
and a fuller threat model.

## Optional: Sync Server

`snp` is a single binary; sync is **opt-in** and requires a `snip-sync`
server (also in this repo). See [snip-sync/README.md](snip-sync/README.md)
for installation, configuration, and a `docker-compose.yml` example.

```bash
# Register against your server (stores API key in OS keychain)
snp register https://sync.example.com:50051

# Manual sync
snp sync

# Push-only / pull-only
snp sync --push-only
snp sync --pull-only

# Set up a 15-minute sync cron job
snp cron
```

## CLI Overview

```
$ snp --help
A fast, terminal-based snippet manager with fuzzy search, clipboard support, and cloud sync

Usage: snp [COMMAND]

Commands:
  new         Create a new snippet
  list        List all snippets
  run         Run a snippet via TUI
  clip        Copy snippet to clipboard
  search      Search and view snippet details
  edit        Edit snippets file in $EDITOR
  sync        Sync snippets with server
  cron        Setup automatic sync
  library     Manage snippet libraries
  premade     Browse/download premade libraries
  register    Register sync account
  keybindings Show TUI keybindings
  completions Generate shell completions
  version     Show version
```

See [USER_GUIDE.md](USER_GUIDE.md) for the full reference and
[CONTRIBUTING.md](CONTRIBUTING.md) for development setup.

## Environment Variables

| Variable                          | Description                                            | Default     |
| --------------------------------- | ------------------------------------------------------ | ----------- |
| `SNP_CONFIG_HOME`                 | Override config directory                              | `~/.config/snp` |
| `SNP_COMMAND_TIMEOUT`             | Command execution timeout in seconds (`0` disables; direct terminal runs default to no timeout, output-capture runs default to 300s) | - |
| `SNP_CLIPBOARD_TIMEOUT`           | Clipboard operation timeout (seconds)                  | `5`         |
| `SNP_ALLOW_PLAINTEXT_API_KEY`     | Allow API key in config file (not keychain)            | `false`     |
| `SNP_SYNC_CONNECT_TIMEOUT`        | Sync TCP connect timeout (seconds)                     | `10`        |
| `SNP_SYNC_REQUEST_TIMEOUT`        | Sync per-request timeout (seconds)                     | `30`        |
| `SNP_THEME`                       | UI theme (`dark` or `bright`)                          | `dark`      |
| `SNP_LOG_LEVEL`                   | Log level (`trace`, `debug`, `info`, `warn`, `error`)  | `info`      |
| `SNP_LOG`                         | Per-module log filter (e.g., `snp=debug`)              | -           |
| `EDITOR`                          | Editor for `snp edit` command                          | `vim`       |

## Documentation

- **[USER_GUIDE.md](USER_GUIDE.md)** — Detailed guide: libraries, sync, variables, premade libraries, troubleshooting.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — Development setup, code style, testing, release process.
- **[SECURITY.md](SECURITY.md)** — Vulnerability disclosure policy and threat model.
- **[CHANGELOG.md](CHANGELOG.md)** — Release history.
- **[docs.rs/snip-it](https://docs.rs/snip-it)** — API reference (this is a binary crate; doc-comments are for contributor reference).
- **[snip-sync/README.md](snip-sync/README.md)** — Sync server setup, configuration, deployment.

## Contributing

Contributions welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for
development setup, code style, and the release process. Bug reports
and feature requests go through [GitHub Issues](https://github.com/anomalyco/snip-it/issues).

## License

[MIT](LICENSE) © 2026 David Bowman

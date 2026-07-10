# snip-it

[![Crates.io](https://img.shields.io/crates/v/snip-it.svg)](https://crates.io/crates/snip-it)
[![Downloads](https://img.shields.io/crates/d/snip-it.svg)](https://crates.io/crates/snip-it)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`snip-it` (`snp`) is a terminal-first snippet manager for short scripts and
commands. Save them in libraries, find them with fuzzy search, fill in
variables when you use them, then run or copy them from a keyboard-driven TUI.

It was inspired by [pet](https://github.com/knqyf263/pet) and keeps the same
simple, editable TOML approach to command snippets. The optional
[`snip-sync`](snip-sync/README.md) server adds encrypted synchronization for
environments where you want one snippet collection available on multiple
machines.

## What snip-it provides

- Short command and script snippets stored as editable TOML.
- Separate libraries for work, home, projects, or environments.
- Fuzzy search, tags, syntax highlighting, clipboard support, and a TUI.
- Runtime variables such as `<host>` and `<branch=main>`.
- 50 bundled [Halloy](https://github.com/squidowl/halloy)-compatible themes,
  plus support for dropping in additional Halloy theme files.
- Optional self-hosted sync using AES-256-GCM encryption and Argon2id key
  derivation. The server stores encrypted snippet payloads, not their
  descriptions or commands.
- Premade libraries served by `snip-sync`.

Commands are executed through your shell exactly as written. `snip-it` is a
snippet manager, not a sandbox or a secrets manager; only save and run commands
you trust.

## Installation

### Homebrew (macOS)

```bash
brew install eggstack/tap/snip-it
```

This installs the `snp` client with shell completions for Bash, Zsh, and Fish.
The optional `snip-sync` server is not included. To upgrade:

```bash
brew upgrade snip-it
```

To uninstall:

```bash
brew uninstall snip-it
```

User configuration and snippet data are preserved by Homebrew unless manually
removed.

### From crates.io

```bash
cargo install snip-it
```

Rust 1.94 or newer is required when building from source.

### Pre-built binaries

Download the binary for your platform from the
[latest GitHub release](https://github.com/eggstack/snip-it/releases/latest).
Release assets currently include:

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `snip-it-v<VERSION>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux aarch64 | `snip-it-v<VERSION>-aarch64-unknown-linux-gnu.tar.gz` |
| macOS Intel | `snip-it-v<VERSION>-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `snip-it-v<VERSION>-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `snip-it-v<VERSION>-x86_64-pc-windows-msvc.zip` |

On Linux or macOS, extract the archive and install the binary:

```bash
tar xzf snp-*.tar.gz
sudo mv snp /usr/local/bin/snp
```

On Windows, extract the zip and move `snp.exe` to a directory on your `PATH`.

Each release also includes a `SHA256SUMS` file for verifying download integrity.

After installing the client, `snp update` checks the appropriate source and
updates in place: crates.io for Cargo installs, Homebrew for Homebrew installs,
and the matching GitHub release archive for standalone binaries. Use
`snp update --dry-run` to check without installing.

### From source

```bash
git clone https://github.com/eggstack/snip-it.git
cd snip-it
cargo build --release
mkdir -p ~/.local/bin
install target/release/snp ~/.local/bin/snp
```

The Docker image is for the optional sync server, not the interactive client:

```bash
docker pull ghcr.io/eggstack/snip-it/snip-sync:latest
```

## Quickstart

Create a snippet, search it, and run or copy it:

```bash
snp new 'git push origin <branch=main>' --tags
snp list
snp run
snp clip
snp search
```

`snp new` prompts for a description (or accepts `--description`), and `--tags`
also prompts for tags. Variable values are requested when a snippet is run or
copied. In the TUI, press `d` in normal mode and confirm with `y` to delete the
selected snippet; any other key cancels.

### Snippet files

Without libraries, snip-it uses the legacy single-file layout:

```text
$XDG_CONFIG_HOME/snp/snippets.toml
```

When `XDG_CONFIG_HOME` is unset, this is `~/.config/snp/snippets.toml`.

Creating a library switches the installation to library mode. Existing
`snippets.toml` content is migrated to `libraries/snippets.toml` when needed.

```bash
snp library create work
snp library set-primary work
snp new --library work 'kubectl get pods -n <namespace=default>'
snp run --library work
```

The canonical file format is compatible with pet's snippet format:

```toml
[[snippets]]
description = "Git commit with a message"
command = "git commit -m \"<message>\""
tag = ["git", "version-control"]
output = ""
```

The loader also accepts snip-it's older `[[Snippets]]` spelling and legacy
capitalized field names. Snip-it-only metadata such as IDs, folders, favorites,
and sync timestamps is preserved when snip-it writes a library. See
[USER_GUIDE.md](USER_GUIDE.md) for library layout, import/export, and the full
configuration reference.

## Themes

Snip-it reads the same color-theme TOML files used by Halloy. A Halloy theme
file can be copied directly into:

```text
$XDG_CONFIG_HOME/snp/themes/<name>.toml
```

When `XDG_CONFIG_HOME` is unset, use `~/.config/snp/themes/<name>.toml`.

Then press `e` in the normal TUI mode to open the theme picker, preview themes,
and press `Enter` to save the selection. The active theme is recorded in
the config root's `themes.toml`. The `SNP_THEME` environment variable remains
available for compatibility with the older `dark`, `bright`, `light`, and
`auto` values, or a theme filename.

Snip-it uses Halloy's color schema and projects it onto the colors needed by
the TUI. `font_style` and Halloy-specific UI colors that have no snip-it
equivalent are ignored. Copy the theme file itself, not Halloy's main config
entry such as `theme = "..."`. See the [Halloy theme guide](https://halloy.chat/guides/custom-themes)
and [Halloy's theme repository](https://github.com/squidowl/halloy).

## Sync across environments

Sync is optional. It uses a self-hosted `snip-sync` server backed by SQLite.
The client encrypts snippet descriptions, commands, and tags before sending
them; the server handles authentication, library metadata, and ciphertext
storage. The server does not terminate TLS, so a remote deployment must put it
behind a TLS-terminating reverse proxy.

The complete deployment guide, including Docker, Caddy, systemd, configuration,
health checks, and troubleshooting, is in
[snip-sync/README.md](snip-sync/README.md).

### Local test server

```bash
cargo install snip-it snip-sync

# In one terminal:
snip-sync init --skip-cert
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve

# In another terminal:
snp register --server http://127.0.0.1:50051
snp sync --push-only
```

Plaintext HTTP is for loopback development only. Do not expose this server
directly to the internet.

### Remote server and multiple environments

For a remote server, use an HTTPS URL terminated by your reverse proxy:

```bash
snp register --server https://sync.example.com
```

The sync server's current credential model is API-key based. `snp register`
creates a new account and API key, so run it once for the collection you want
to share. Every environment that should see that collection must be configured
with the same server URL and API key; registering independently creates a
separate account and separate libraries. Provision the key through your OS
keychain or a secret manager, and never commit it to a repository or put it in
shell history. The [multi-environment section of USER_GUIDE.md](USER_GUIDE.md#syncing-one-account-across-environments)
shows the settings involved.

After the first environment has pushed its libraries, use bidirectional sync on
each environment so local and remote changes are merged:

```bash
snp sync --push-only       # first environment: seed the server
snp sync --pull-only       # another environment: fetch the existing libraries
snp sync                    # after setting sync_direction = "Bidirectional"
```

Sync uses last-write-wins timestamps for shared fields. Keep the SQLite
database on persistent storage and back it up along with the rest of the server
data.

## CLI overview

```text
snp new          Create a snippet
snp list         List snippets
snp run          Run a snippet from the TUI
snp clip         Copy a snippet from the TUI
snp search       Search and inspect snippets
snp edit         Edit a snippet library in $EDITOR
snp library      Create, list, select, or delete libraries
snp premade      Browse and download premade libraries
snp register     Register with a snip-sync server
snp sync         Push, pull, or bidirectionally sync libraries
snp cron         Print a periodic sync schedule
snp keybindings  Show TUI keybindings
snp update       Check for and install an update
snp completions  Generate shell completions
```

Run `snp <command> --help` for command-specific options.

## Configuration and security

The client configuration root is `$XDG_CONFIG_HOME/snp` when
`XDG_CONFIG_HOME` is set, otherwise `~/.config/snp`. Important files include:

| Path | Purpose |
| --- | --- |
| `snippets.toml` | Legacy single-file library |
| `libraries.toml` | Library metadata and sync links |
| `libraries/*.toml` | User libraries |
| `premade/*.toml` | Downloaded premade libraries |
| `sync.toml` | Sync server settings and direction |
| `themes/*.toml` | Halloy-compatible theme files |
| `themes.toml` | Active theme selection |

API keys are stored in the operating system keychain when available. Set
`SNP_ALLOW_PLAINTEXT_API_KEY=true` only for a deliberately controlled headless
environment where keychain storage is unavailable. This stores the key in
`sync.toml` and should be protected with restrictive file permissions.

Sync payloads use AES-256-GCM with an Argon2id-derived key. CRC32 integrity
headers on local sync settings detect accidental partial writes but are not an
anti-tampering mechanism. See [SECURITY.md](SECURITY.md) for the threat model
and disclosure policy.

Common environment variables:

| Variable | Purpose |
| --- | --- |
| `XDG_CONFIG_HOME` | Change the client configuration root |
| `SNP_THEME` | Select a legacy or file-based theme |
| `SNP_COMMAND_TIMEOUT` | Command execution timeout in seconds; `0` disables it |
| `SNP_CLIPBOARD_TIMEOUT` | Clipboard timeout in seconds; default `5` |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | Permit plaintext API-key storage when keychain storage fails |
| `SNP_SYNC_CONNECT_TIMEOUT` | Sync connection timeout; default `10` seconds |
| `SNP_SYNC_REQUEST_TIMEOUT` | Sync request timeout; default `30` seconds |
| `SNP_LOG` / `RUST_LOG` | Configure tracing output |
| `EDITOR` | Editor used by `snp edit` |

## Documentation

- [USER_GUIDE.md](USER_GUIDE.md) — libraries, pet compatibility, themes, sync,
  multi-environment provisioning, variables, premade libraries, and recovery.
- [snip-sync/README.md](snip-sync/README.md) — self-hosting and deploying the
  optional sync server.
- [SECURITY.md](SECURITY.md) — threat model and vulnerability disclosure.
- [CONTRIBUTING.md](CONTRIBUTING.md) — development and release workflow.
- [CHANGELOG.md](CHANGELOG.md) — release history.

## License

[MIT](LICENSE) © 2026 David Bowman

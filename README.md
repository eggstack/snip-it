# snip-it

[![Crates.io](https://img.shields.io/crates/v/snip-it.svg)](https://crates.io/crates/snip-it)
[![Downloads](https://img.shields.io/crates/d/snip-it.svg)](https://crates.io/crates/snip-it)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![snip-it in use](demo/snip-it-demo.gif)

`snip-it` (`snp`) is a fast, terminal-first snippet manager for commands and
short scripts. Save commands as plain TOML, find them with fuzzy search, fill in
variables at use time, and run, copy, inspect, or insert them from a
keyboard-driven TUI.

It is inspired by [pet](https://github.com/knqyf263/pet) and intentionally keeps
pet's simple editable snippet format. Snip-it adds libraries, richer TUI
navigation, shell integration, themes, local usage metadata, and optional
self-hosted encrypted synchronization.

Commands selected with `snp run` are executed through your shell exactly as
stored after variable expansion. Snip-it is a snippet manager, not a sandbox or
secrets manager; only save and run commands you trust.

## Features

- Fuzzy-searchable command and script snippets stored as editable TOML.
- Vim-style TUI navigation with run, copy, search, delete, and theme actions.
- Runtime variables such as `<host>` and defaults such as `<branch=main>`.
- Pet-compatible choice variables such as
  `<color=|_red_||_green_||_blue_||>`.
- Separate libraries for work, personal, project, or environment-specific
  snippets.
- Tags, favorites, output/notes metadata, and sorting by relevance, recency,
  usage, description, or command.
- Bash, Zsh, and Fish integration for inserting snippets into the current shell
  buffer without executing them.
- Import and diagnostics for existing pet snippet files.
- Bundled Halloy-compatible themes plus support for custom Halloy theme files.
- Optional self-hosted synchronization with client-side AES-256-GCM encryption.
- Backup, restore, validation, repair, and machine-readable output for scripting.

## Installation

The bootstrap installer downloads a verified release binary for supported
hosts, with exact-version Cargo fallback for source-only targets:

```bash
# snp (default)
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash

# snip-sync, or both independently versioned components
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash -s -- --server
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash -s -- --both
```

For Windows, inspect the downloaded PowerShell script before running it:

```powershell
irm https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.ps1 -OutFile .\install-snip-it.ps1
Get-Content .\install-snip-it.ps1
. .\install-snip-it.ps1 -Component Snp
```

The pipe-to-shell forms are convenient but execute remote content directly;
the download-then-inspect form is the safer reviewable alternative. See
[packaging/README.md](packaging/README.md) for pinned installs and the full
verification/fallback contract.

Cargo remains the simplest source/package install:

```bash
cargo install snip-it
```

Building the current release requires Rust 1.94 or newer. Check with
`snp version`. `snp update` uses the stable `snip-it` crates.io version, then
downloads and verifies the exact matching release binary. It works for
bootstrap, Cargo, and directly managed executables without requiring Rust on
supported hosts. A Homebrew-managed `snp` remains owned by Homebrew; use
`brew upgrade snip-it` for that installation. Use `snp update --dry-run` to
inspect the selected version and target without changing files.

### From source

```bash
git clone https://github.com/eggstack/snip-it.git
cd snip-it
cargo build --release
```

The client binary will be at `target/release/snp` (or `snp.exe` on Windows).

## Quick start

Create a snippet:

```bash
snp new 'git push origin <branch=main>' \
  --description 'Push a branch' \
  --tags git,release
```

Then choose what you want to do with it:

```bash
snp run       # fuzzy-select and execute
snp clip      # fuzzy-select and copy to the clipboard
snp search    # fuzzy-select and inspect
snp select    # fuzzy-select and print the command; never executes
snp list      # list snippets without opening the selector
```

If a command contains variables, snip-it prompts for them before execution or
copying:

```text
ssh <user>@<host>
git checkout <branch=main>
```

A default is supplied after `=`. Choice variables use pet-compatible syntax:

```text
kubectl config use-context <context=|_dev_||_staging_||_prod_||>
```

For complete command help, run `snp --help` or `snp <command> --help`.

## TUI

The selector is designed around keyboard navigation. Common normal-mode keys:

| Key | Action |
| --- | --- |
| `j` / `k` or arrows | Move through snippets |
| `/` or `i` | Enter search/input mode |
| `Enter` | Select the highlighted snippet |
| `y` | Copy the selected snippet and quit |
| `d` | Delete the selected snippet; confirm with `y` |
| `e` | Open the theme picker |
| `q` | Quit |
| `gg` / `G` | Jump to top / bottom |
| `Ctrl-d` / `Ctrl-u` | Page down / up |

The exact action performed by `Enter` depends on the command that opened the
selector (`run`, `clip`, `search`, or `select`). Variable entry has its own
insert/normal modal controls. Run `snp keybindings` for the complete reference.

## Libraries

Libraries keep independent groups of snippets in separate TOML files:

```bash
snp library create work
snp library set-primary work
snp new --library work 'kubectl get pods -n <namespace=default>'
snp run --library work
```

Library files live under `$XDG_CONFIG_HOME/snp/libraries/` (default:
`~/.config/snp/libraries/`). The on-disk format is human-editable and
pet-compatible. See [USER_GUIDE.md](USER_GUIDE.md#libraries) for details.

## Importing from pet

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml   # inspect first
snp import pet ~/.config/pet/snippets.toml           # import into a library
snp import pet snippets.toml --merge                 # merge into existing
snp import pet snippets.toml --dry-run               # preview without writing
```

The source file is never modified. See
[USER_GUIDE.md](USER_GUIDE.md#pet-compatibility-and-import) for migration,
replacement, diagnostics, and compatibility details.

## Creating snippets from files, stdin, or an editor

```bash
printf '%s' 'git commit -m "release"' | \
  snp new --command-stdin --description 'Release commit'

snp new --from-file ./deploy.sh --description 'Deploy service'
snp new --editor --description 'Complex pipeline'
```

These modes store valid UTF-8 command text without evaluating it. See
[USER_GUIDE.md](USER_GUIDE.md#shell-integration) for multiline scripts and
shell integration details.

## Shell integration

Snip-it generates shell functions for Bash, Zsh, and Fish that insert the
selected snippet into the current command buffer without executing it.

```bash
# Bash: ~/.bashrc
eval "$(snp shell init bash)"

# Zsh: ~/.zshrc
eval "$(snp shell init zsh)"

# Fish: ~/.config/fish/config.fish
snp shell init fish | source
```

No keybindings are installed automatically. The generated functions:

| Function | Behavior |
| --- | --- |
| `snp_select_raw` | Insert a snippet with placeholders unchanged |
| `snp_select_expanded` | Prompt for variables, then insert the expanded command |
| `snp_new_current` | Save the current shell buffer as a snippet |
| `snp_new_previous` | Save the previous accepted shell-history entry |

See [USER_GUIDE.md](USER_GUIDE.md#shell-integration) for example keybindings,
saving commands, and shell-specific details.

## Themes

Press `e` in the TUI's normal mode to open the theme picker with live preview.
Snip-it ships 50 bundled Halloy-compatible themes. Custom themes go in
`$XDG_CONFIG_HOME/snp/themes/<name>.toml`. See
[USER_GUIDE.md](USER_GUIDE.md#themes) for the supported schema.

## Sync

Sync is optional and self-hosted. The `snp` client encrypts snippets
client-side before sending them to `snip-sync`; the server stores ciphertext
in SQLite. The server does not terminate TLS — use a reverse proxy for remote
deployments.

```bash
cargo install snip-sync

# Local test (loopback only)
snip-sync init --skip-cert
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve &

snp register --server http://127.0.0.1:50051
snp sync --push-only

# Remote deployment
snp register --server https://sync.example.com
snp sync
```

Auto-sync after local mutations is off by default: `snp sync config --auto-sync on`.
See [snip-sync/README.md](snip-sync/README.md) for deployment, Caddy/reverse-proxy
examples, systemd, and troubleshooting. See
[USER_GUIDE.md](USER_GUIDE.md#sync) for multi-device credential setup and
sync policy.

## Command overview

| Command | Purpose |
| --- | --- |
| `snp new` | Create a snippet |
| `snp list` | List/filter snippets without executing |
| `snp run` | Select and execute a snippet |
| `snp clip` | Select and copy a snippet |
| `snp search` | Select and inspect a snippet |
| `snp select` | Select and print a command without executing it |
| `snp get` | Retrieve a snippet deterministically for scripts |
| `snp edit` | Edit a library or a snippet's output/notes metadata |
| `snp library` | Create, list, inspect, select, or delete libraries |
| `snp premade` | Browse and install premade libraries from a sync server |
| `snp import pet` | Import a pet snippet file |
| `snp doctor` | Diagnose files, the local installation, shell integration, or sync |
| `snp status` | Show auto-sync and sync state as JSON or text |
| `snp backup` | Create a checksummed snapshot of local state |
| `snp restore` | Restore local state from a backup snapshot |
| `snp data` | Validate, back up, restore, repair, or inspect local state |
| `snp repair` | Validate and repair configuration and library files |
| `snp validate` | Read-only validation of snippet data and structure |
| `snp register` | Register with a `snip-sync` server |
| `snp sync` | Run or configure synchronization |
| `snp cron` | Print a periodic sync schedule |
| `snp shell init` | Generate interactive shell integration |
| `snp completions` | Generate shell completion definitions |
| `snp keybindings` | Print the complete TUI keybinding reference |
| `snp update` | Check for and install a supported update |
| `snp version` | Print the installed version |

## Configuration and data

The client configuration root is `$XDG_CONFIG_HOME/snp` when
`XDG_CONFIG_HOME` is set, otherwise `~/.config/snp`.

| Path | Purpose |
| --- | --- |
| `snippets.toml` | Legacy single-file snippet collection |
| `libraries.toml` | Library metadata and sync links |
| `libraries/*.toml` | User libraries |
| `premade/*.toml` | Downloaded premade libraries |
| `sync.toml` | Sync settings and server metadata |
| `themes/*.toml` | Custom Halloy-compatible themes |
| `themes.toml` | Active theme selection |
| `usage.toml` | Local usage counts and last-used timestamps |

Sync API keys are stored in the operating-system keychain when available. See
[SECURITY.md](SECURITY.md) before using the plaintext-key fallback in a
headless environment.

## More documentation

| Document | Contents |
| --- | --- |
| [USER_GUIDE.md](USER_GUIDE.md) | Libraries, variables, pet import, shell integration, themes, sync, auto-sync, automation, and recovery |
| [snip-sync/README.md](snip-sync/README.md) | Deploying and operating the optional sync server (Docker, systemd, reverse proxy) |
| [SECURITY.md](SECURITY.md) | Security model, encryption, credential storage, and vulnerability disclosure |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development workflow, testing, and release process |
| [CHANGELOG.md](CHANGELOG.md) | Release history |
| [docs/EXIT_CODES.md](docs/EXIT_CODES.md) | Exit code reference |
| [docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md) | Pet format compatibility details |
| [docs/JSON_SCHEMAS.md](docs/JSON_SCHEMAS.md) | Machine-readable JSON output schemas |
| [docs/SECURITY_AUDIT.md](docs/SECURITY_AUDIT.md) | Security audit findings |
| [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) | Threat model and trust boundaries |

## License

[MIT](LICENSE) © 2026 David Bowman

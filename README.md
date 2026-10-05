# snip-it

[![Crates.io](https://img.shields.io/crates/v/snip-it.svg)](https://crates.io/crates/snip-it)
[![Downloads](https://img.shields.io/crates/d/snip-it.svg)](https://crates.io/crates/snip-it)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![snip-it in use](demo/snip-it-demo.gif)

`snip-it` (`snp`) is a fast, terminal-first snippet manager for commands and
short scripts. Save commands as plain TOML, find them with fuzzy search, fill in
variables at use time, and run, copy, inspect, or insert them from a
keyboard-driven TUI.

It is inspired by [pet](https://github.com/knqyf263/pet) and keeps pet's simple
editable snippet format. Snip-it adds libraries, richer TUI navigation, shell
integration, themes, local usage metadata, and optional self-hosted encrypted
synchronization.

> Commands chosen with `snp run` are executed through your shell exactly as
> stored after variable expansion. Snip-it is a snippet manager, not a sandbox or
> secrets manager — only save and run commands you trust.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash
```

Windows (inspect before running):

```powershell
irm https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.ps1 -OutFile .\install-snip-it.ps1
Get-Content .\install-snip-it.ps1
. .\install-snip-it.ps1 -Component Snp
```

Or from source / crates.io:

```bash
cargo install snip-it      # needs Rust 1.94+
git clone https://github.com/eggstack/snip-it.git && cd snip-it && cargo build --release
```

The binary lands at `target/release/snp` (`snp.exe` on Windows). The installer
also takes `--server` for `snip-sync` and `--both`; see
[packaging/README.md](packaging/README.md) for pinned installs and the
verification/fallback contract. A Homebrew-managed `snp` stays owned by
Homebrew — use `brew upgrade snip-it`.

## Quick start

```bash
# Store a snippet. Variables are prompted for at use time.
snp new 'git push origin <branch=main>' --description 'Push a branch' --tags git,release

# Pick one interactively.
snp run        # fuzzy-select and execute
snp clip       # fuzzy-select and copy to the clipboard
snp search     # fuzzy-select and inspect
snp list       # browse without opening the selector
```

Variables come in three forms:

```text
ssh <user>@<host>                              # prompt for both
git checkout <branch=main>                     # prompt, pre-filled default
kubectl config use-context <ctx=|_dev_||_prod_|>   # pet-compatible choice
```

The `run`/`clip`/`search`/`select` commands need a terminal. For scripts and
pipelines use the deterministic, non-TUI `snp get`:

```bash
snp get --query 'push a branch' --field command   # print the command
snp get --query 'push a branch' --json            # full record as JSON
snp list --json                                  # every snippet as JSON
snp list --csv                                   # every snippet as CSV
```

`snp --help` or `snp <command> --help` covers everything else.

## Commands

| Command | Purpose |
| --- | --- |
| `snp new` | Create a snippet (also `--command-stdin`, `--from-file`, `--editor`) |
| `snp list` | List/filter snippets without executing (`--json`, `--csv`) |
| `snp run` | Select and execute a snippet |
| `snp clip` | Select and copy a snippet |
| `snp search` | Select and inspect a snippet |
| `snp select` | Select and print a command without executing it |
| `snp get` | Retrieve a snippet deterministically for scripts |
| `snp edit` | Edit a library, or one snippet's output/notes metadata |
| `snp library` | `create` / `list` / `show` / `set-primary` / `delete` libraries |
| `snp premade` | Install premade libraries from a sync server |
| `snp import pet` | Import a pet snippet file (`--merge`, `--dry-run`) |
| `snp doctor` | Diagnose a file, the install, shell integration, or sync |
| `snp status` | Show auto-sync and sync state |
| `snp backup` | Checksummed snapshot of local state |
| `snp restore` | Restore from a backup snapshot |
| `snp repair` | Validate and repair config and library files |
| `snp validate` | Read-only validation of snippet data |
| `snp register` | Register with a `snip-sync` server |
| `snp sync` | Run or configure sync (`sync run --push-only`/`--pull-only`/`--dry-run`, `sync config`, `sync retry`) |
| `snp cron` | Print a periodic sync schedule |
| `snp shell init` | Generate shell integration (`bash`/`zsh`/`fish`) |
| `snp completions` | Generate shell completions |
| `snp keybindings` | Print the full TUI keybinding reference |
| `snp mcp` | Read-only stdio MCP server (`serve`/`install`/`instructions`) |
| `snp update` | Check for and install an update |
| `snp data` | Aliases for `validate`, `backup`, `restore`, `repair`, `status` |

## Libraries

```bash
snp library create work && snp library set-primary work
snp new 'kubectl get pods' -d 'Pods' --library work
snp list --library work
```

One library is searched at a time. `snp get --library all` searches every
library; `snp list` is always a single library. Details:
[docs/LIBRARY_SCOPE.md](docs/LIBRARY_SCOPE.md).

## Shell integration

```bash
eval "$(snp shell init bash)"     # or zsh; fish: snp shell init fish | source
```

Adds `snp_select_raw`, `snp_select_expanded`, `snp_new_current`, and
`snp_new_previous` so you can insert a snippet into the current buffer without
executing it. No keybindings are installed automatically. Validate the output
with `snp doctor --check-shell bash`, and see
[USER_GUIDE.md](USER_GUIDE.md#shell-integration) for suggested bindings.

## TUI

Press `e` in normal mode for the theme picker (50 bundled Halloy-compatible
themes; custom ones go in `$XDG_CONFIG_HOME/snp/themes/`). Common normal-mode
keys:

| Key | Action |
| --- | --- |
| `j` / `k` or arrows | Move through snippets |
| `i` or `/` | Enter search/input mode |
| `Enter` | Select the highlighted snippet |
| `y` | Copy the selected snippet and quit |
| `d` | Delete the selected snippet; confirm with `y` |
| `e` | Open the theme picker |
| `q` | Quit |

`snp keybindings` prints the complete reference.

## Import from pet

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml   # inspect first
snp import pet ~/.config/pet/snippets.toml --merge  # merge into the current library
snp import pet snippets.toml --dry-run              # preview, writes nothing
```

`--dry-run` is the safe way to see what a file would produce. The matrix of
behavioural differences is in
[docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md).

## Sync (optional, self-hosted)

The client encrypts snippet content with AES-256-GCM before sending it;
`snip-sync` stores only ciphertext. The server does not terminate TLS, so put a
reverse proxy in front of remote deployments.

```bash
cargo install snip-sync
snip-sync init --skip-cert
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve &     # local loopback test only

snp register --server http://127.0.0.1:50051
snp sync run --push-only
```

Auto-sync after local mutations is off by default: enable it with
`snp sync config --auto-sync on`. Deployment, systemd, and reverse-proxy setup:
[snip-sync/README.md](snip-sync/README.md).

## Coding agents (MCP)

`snp mcp serve` exposes your snippets to a local agent over stdio. It binds no
port, is read-only, and never executes stored commands.

```bash
snp mcp instructions claude    # or codex, vscode, cursor, opencode, zed
snp mcp install claude         # noninteractive registration where supported
```

## Configuration

Everything lives under `$XDG_CONFIG_HOME/snp`, or `~/.config/snp` when that is
unset. A fresh install creates only `snippets.toml` (the default library) and
`logs/`; the rest appear as you use the features.

| Path | Purpose |
| --- | --- |
| `snippets.toml` | Default single-file library |
| `libraries.toml` | Library metadata and sync links |
| `libraries/*.toml` | Additional libraries |
| `premade/*.toml` | Downloaded premade libraries |
| `sync.toml` | Sync settings (the API key is stored in the OS keychain, never here) |
| `usage.toml` | Local usage counts and last-used timestamps |
| `themes/*.toml`, `themes.toml` | Custom themes and the active selection |
| `logs/` | Rotating application and audit logs |

## Documentation

| Document | Contents |
| --- | --- |
| [USER_GUIDE.md](USER_GUIDE.md) | Task-oriented guide: libraries, variables, pet import, shell integration, themes, sync, auto-sync, recovery |
| [docs/README.md](docs/README.md) | **Index of every reference doc**, split into current contracts vs archived snapshots |
| [snip-sync/README.md](snip-sync/README.md) | Running the sync server (systemd, Docker, reverse proxy) |
| [SECURITY.md](SECURITY.md) | Security model, encryption, credential storage, disclosure |
| [packaging/README.md](packaging/README.md) | Installer verification, pinned installs, fallbacks |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Development workflow, testing, releases |
| [CHANGELOG.md](CHANGELOG.md) | Release history |

Contracts worth knowing before you script against `snp`:
[docs/EXIT_CODES.md](docs/EXIT_CODES.md),
[docs/JSON_SCHEMAS.md](docs/JSON_SCHEMAS.md),
[docs/COMMAND_CONTRACTS.md](docs/COMMAND_CONTRACTS.md).

## License

MIT — see [LICENSE](LICENSE).

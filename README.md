# snip-it

[![Crates.io](https://img.shields.io/crates/v/snip-it.svg)](https://crates.io/crates/snip-it)
[![Downloads](https://img.shields.io/crates/d/snip-it.svg)](https://crates.io/crates/snip-it)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

![snip-it in use](demo/snip-it-demo.gif)

`snip-it` (`snp`) is a fast, terminal-first snippet manager for commands and
short scripts. Save commands as plain TOML, find them with fuzzy search, fill in
variables at use time, and run, copy, inspect, or insert them from a
keyboard-driven TUI.

Inspired by [pet](https://github.com/knqyf263/pet) — same simple editable
snippet format — with libraries, richer TUI navigation, shell integration,
themes, and optional self-hosted encrypted sync.

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

Or from crates.io (needs Rust 1.94+):

```bash
cargo install snip-it
```

The installer also takes `--server` for `snip-sync` and `--both`; see
[packaging/README.md](packaging/README.md) for pinned installs and the
verification/fallback contract.

## Quick start

**Store a snippet.**

```bash
snp new 'git push origin <branch=main>' --description 'Push a branch' --tags git,release
```

```
Command> git push origin <branch=main>
Snippet added
```

**See what you have stored.**

```bash
snp list
```

```
-----
Push a branch: git push origin <branch=main>
Tags: git, release
```

**Use it.** All three open the same fuzzy-search TUI: `run` executes the
snippet through your shell, `clip` copies it to the clipboard, `search`
inspects it.

```bash
snp run
snp clip
snp search
```

Variables are prompted for at use time and come in three forms:

```text
ssh <user>@<host>                                 # prompt for both
git checkout <branch=main>                        # prompt, pre-filled default
kubectl config use-context <ctx=|_dev_||_prod_|>  # pet-compatible choice
```

## Scripting

`snp get` retrieves a snippet deterministically — no TUI, no execution. Use it
when a script needs a command:

```bash
snp get --query 'push a branch' --field command
```

```
git push origin <branch=main>
```

`--json` returns the whole record, including the expanded command and the
snippet's stable ID:

```bash
snp get --query 'push a branch' --json
```

```json
{
  "schema": 1,
  "id": "legacy-6e766362842b1bf257b5f65bc7e87fb4afcc6f11f8847f2a4ad1415a76d40155",
  "description": "Push a branch",
  "command": "git push origin <branch=main>",
  "expanded": "git push origin main",
  "tags": [
    "git",
    "release"
  ],
  "library": "snippets",
  "library_id": ""
}
```

`snp get` exits **3** when nothing matches and **5** when a query matches more
than one snippet. For machine-readable listings use `snp list --json` or
`snp list --csv`. The full exit-code table is in
[docs/EXIT_CODES.md](docs/EXIT_CODES.md).

## Libraries

Snippets live in libraries. A fresh install uses a single `snippets` library;
create more when you want to separate contexts.

```bash
snp library create work
snp library set-primary work
snp new 'kubectl get pods' --description 'Pods' --library work
snp list --library work
```

`--library all` targets every library at once. `all` is a reserved name and
cannot be used as a library name. See
[docs/LIBRARY_SCOPE.md](docs/LIBRARY_SCOPE.md).

## Shell integration

`snp shell init` prints functions and widgets for bash, zsh, and fish. Inspect
the output before sourcing it — it installs no key bindings by default.

```bash
snp shell init bash >> ~/.bashrc
snp doctor --check-shell bash
```

## Also available

| Feature | Command | Detail |
|---------|---------|--------|
| Import from [pet](https://github.com/knqyf263/pet) | `snp import pet <file> --dry-run` | [docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md) |
| Self-hosted encrypted sync | `snp register --server <url>` then `snp sync run` | [snip-sync/README.md](snip-sync/README.md) |
| Auto-sync after local edits | `snp sync config --auto-sync on` | [docs/USER_GUIDE.md](docs/USER_GUIDE.md#auto-sync) |
| Coding agents (MCP) | `snp mcp instructions claude` | [docs/MCP.md](docs/MCP.md) |
| Themes | press `e` in the TUI, or edit `themes.toml` | [docs/USER_GUIDE.md](docs/USER_GUIDE.md#themes) |
| Diagnostics | `snp doctor --library <name>` | [docs/USER_GUIDE.md](docs/USER_GUIDE.md#troubleshooting-and-recovery) |

Sync encrypts snippet content (description, command, tags) with AES-256-GCM
before sending it, and the server stores only ciphertext for that content. Sync
metadata — snippet IDs, timestamps, device ID, and deletion markers — travels
in the clear, and the server sees your API key as a bearer token. See
[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) for the trust boundary.

## Configuration

Everything lives under `~/.config/snp/` (or `$XDG_CONFIG_HOME/snp/`). The
first write creates `snippets.toml`, `logs/`, and `.transaction/`; the rest
appear as you use the features.

| Path | Purpose |
|------|---------|
| `snippets.toml` | Your snippets, as plain TOML. |
| `libraries/` | Additional libraries, one TOML file each. |
| `libraries.toml` | Library index and which one is primary. |
| `sync.toml` | Sync server and account settings. |
| `logs/` | Rotating debug logs. |
| `.transaction/` | Crash-recovery journals. Do not edit. |

The API key for sync is kept in your OS keychain, not in `sync.toml`. See
[docs/USER_GUIDE.md](docs/USER_GUIDE.md#configuration) for the full set of
paths, environment variables, and auto-sync settings.

## Documentation

Start with the [docs index](docs/README.md).

| Document | Subject |
|----------|---------|
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | Task-oriented guide: libraries, variables, pet import, shell integration, themes, sync, auto-sync, recovery |
| [docs/COMMANDS.md](docs/COMMANDS.md) | Every subcommand and its flags |
| [docs/EXIT_CODES.md](docs/EXIT_CODES.md) | Stable exit codes 0–10 and their meaning |
| [docs/COMMAND_CONTRACTS.md](docs/COMMAND_CONTRACTS.md) | Per-command behavior, startup recovery, stdout/stderr split |
| [docs/LIBRARY_SCOPE.md](docs/LIBRARY_SCOPE.md) | How `--library` resolves |
| [docs/MCP.md](docs/MCP.md) | The read-only MCP server and its tools |
| [docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md) | Behavioral matrix vs. `pet` import |
| [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) | Sync trust boundary and what is not encrypted |
| [docs/IDENTITY_CONTRACT.md](docs/IDENTITY_CONTRACT.md) | How snippet IDs are derived |
| [SECURITY.md](SECURITY.md) | Reporting a vulnerability and current posture |

Contributors and reviewers should start with
[architecture/overview.md](architecture/overview.md) — the module map, the
cross-cutting invariants, and the index of 61 per-component deep dives.

## License

MIT — see [LICENSE](LICENSE).

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

Or from crates.io (needs Rust 1.94+):

```bash
cargo install snp-it
```

The binary lands at `target/release/snp` (`snp.exe` on Windows). The installer
also takes `--server` for `snip-sync` and `--both`; see
[packaging/README.md](packaging/README.md) for pinned installs and the
verification/fallback contract. A Homebrew-managed `snp` stays owned by
Homebrew — use `brew upgrade snip-it`.

## Quick start

Store a snippet, then use it. Variables are prompted for at use time.

```bash
snp new 'git push origin <branch=main>' --description 'Push a branch' --tags git,release
```

```
Command> git push origin <branch=main>
Snippet added
```

Browse what you have stored:

```bash
snp list
```

```
-----
Push a branch: git push origin <branch=main>
Tags: git, release
```

Then pick one interactively — `run` executes it through your shell, `clip`
copies it, and `search` inspects it:

```bash
snp run
snp clip
snp search
```

Variables come in three forms:

```text
ssh <user>@<host>                              # prompt for both
git checkout <branch=main>                     # prompt, pre-filled default
kubectl config use-context <ctx=|_dev_||_prod_|>   # pet-compatible choice
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

`--json` returns the full record, including the expanded command and the
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
  "tags": ["git", "release"],
  "library": "snippets",
  "library_id": ""
}
```

`snp get` exits **3** when nothing matches and **5** when a query matches more
than one snippet. The full exit-code table is in
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
cannot be used as a library name.

## Shell integration

`snp shell init` prints functions and widgets for bash, zsh, and fish. Inspect
the output before sourcing it — it installs no key bindings by default.

```bash
snp shell init bash >> ~/.bashrc
snp doctor --check-shell bash
```

## Optional extras

**Import from [pet](https://github.com/knqyf263/pet).** Inspect first, then
merge:

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml   # inspect
snp import pet ~/.config/pet/snippets.toml --dry-run # preview, writes nothing
snp import pet ~/.config/pet/snippets.toml --merge   # merge into the library
```

The behavioral matrix is in [docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md).

**Self-hosted sync.** The client encrypts snippet content (description,
command, tags) with AES-256-GCM before sending it, and the server stores only
ciphertext for that content. Sync metadata — snippet IDs, timestamps, device
ID, and deletion markers — travels in the clear, and the server sees your API
key as a bearer token. See [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) for
the trust boundary.

```bash
cargo install snip-sync
snip-sync init --skip-cert
snp register --server http://127.0.0.1:50051
snp sync run --push-only
```

Auto-sync after local mutations is off by default — enable it with
`snp sync config --auto-sync on`. Deployment and reverse-proxy setup:
[snip-sync/README.md](snip-sync/README.md).

**Coding agents (MCP).** `snp mcp serve` exposes your snippets to a local
agent over stdio. It binds no port, is read-only, and never executes stored
commands.

```bash
snp mcp instructions claude    # or codex, vscode, cursor, opencode, zed
snp mcp install claude         # noninteractive registration where supported
```

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

Contributors and reviewers should start with
[architecture/overview.md](architecture/overview.md) — the module map, the
cross-cutting invariants, and the index of 61 per-component deep dives.

| Document | Subject |
|----------|---------|
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | Task-oriented guide: libraries, variables, pet import, shell integration, themes, sync, auto-sync, recovery |
| [docs/COMMANDS.md](docs/COMMANDS.md) | Every subcommand and its flags |
| [docs/EXIT_CODES.md](docs/EXIT_CODES.md) | Stable exit codes 0–10 and their meaning |
| [docs/COMMAND_CONTRACTS.md](docs/COMMAND_CONTRACTS.md) | Per-command behavior, startup recovery, stdout/stderr split |
| [docs/LIBRARY_SCOPE.md](docs/LIBRARY_SCOPE.md) | How `--library` resolves |
| [docs/MCP.md](docs/MCP.md) | The read-only MCP server and its tools |
| [docs/PET_COMPATIBILITY.md](docs/PET_COMPATIBILITY.md) | Behavioral matrix vs. `pet` import |
| [docs/IDENTITY_CONTRACT.md](docs/IDENTITY_CONTRACT.md) | How snippet IDs are derived |
| [SECURITY.md](SECURITY.md) | Reporting a vulnerability and current posture |

## License

MIT — see [LICENSE](LICENSE).

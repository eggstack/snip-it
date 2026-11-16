# snp user guide

This guide covers the parts of `snp` that are easy to outgrow from the
[README](../README.md): libraries, TOML compatibility, themes, synchronization,
and automation.

## Contents

- [Libraries](#libraries)
- [Agent / MCP integration](#agent--mcp-integration)
- [Pet compatibility and import](#pet-compatibility-and-import)
- [Shell integration](#shell-integration)
- [TUI key bindings](#tui-key-bindings)
- [Themes](#themes)
- [Sync](#sync)
- [Syncing one account across environments](#syncing-one-account-across-environments)
- [Premade libraries](#premade-libraries)
- [Sorting and ranking](#sorting-and-ranking)
- [Variables](#variables)
- [Configuration](#configuration)
- [Automation](#automation)
- [Troubleshooting and recovery](#troubleshooting-and-recovery)

## Libraries

Libraries are separate collections of snippets. They are useful for keeping
personal commands, work commands, and project-specific scripts apart.

```bash
snp library create work
snp library create personal
snp library set-primary work
snp library list
snp library show work

snp new --library work 'kubectl get pods -n <namespace=default>' --description 'Kubectl pods'
snp list --library work
snp run --library work
snp clip --library work
snp edit --library work
```

Run `snp edit` to open the primary library file in your editor, or pass
`--library <NAME>` to choose another library. To update an individual snippet's
output/notes field, use the options shown in [Editing output](#editing-output).

In the snippet TUI opened by `snp run`, `snp clip`, or `snp search`, press `d`
in normal mode to delete the selected snippet. (`snp select` is read-only and
does not offer the shortcut.) The confirmation box requires `y` to proceed; any
other key cancels. The key remains ordinary filter input in insert mode.
Deleted snippets are hidden from the TUI, and `--sync` propagates the deletion
when sync is configured.

Library files live under:

```text
$XDG_CONFIG_HOME/snp/libraries/
```

When `XDG_CONFIG_HOME` is not set, the default is `~/.config/snp/libraries/`.
`libraries.toml` stores library metadata, including which library is primary
and which server library it is linked to.

A fresh install starts with `snippets.toml` as the default single-file
library. The first library-mode command copies that file to
`libraries/snippets.toml`, registers it as the primary `snippets` library, and
leaves the original in place. Keep your own backup before making large changes.

## Pet compatibility and import

Snip-it was inspired by [pet](https://github.com/knqyf263/pet), and the core
snippet format is intentionally compatible. Pet's current format uses a
lowercase `[[snippets]]` table and these fields:

```toml
[[snippets]]
description = "Show listening ports"
command = "lsof -iTCP -sTCP:LISTEN"
tag = ["network"]
output = ""
```

The variable syntax is shared too: `<name>` prompts for a value and
`<name=default>` provides a default. Snip-it accepts pet files directly; no
conversion script is needed for ordinary command snippets.

The recommended migration workflow starts with diagnostics:

```bash
snp doctor --pet-file /path/to/pet-snippets.toml
```

Then import based on the findings:

```bash
# Import with automatic library name (derived from filename)
snp import pet /path/to/pet-snippets.toml

# Import with explicit library name
snp import pet /path/to/pet-snippets.toml --library my-snippets

# Preview without writing files
snp import pet /path/to/pet-snippets.toml --dry-run

# Merge into existing library, skipping exact duplicates
snp import pet /path/to/pet-snippets.toml --library existing-lib --merge

# Replace existing library entirely (with backup)
snp import pet /path/to/pet-snippets.toml --library existing-lib --replace

# Get a machine-readable JSON report
snp import pet /path/to/pet-snippets.toml --report json
```

#### Check and import a Pet file

`snp doctor --pet-file <path>` reports compatibility issues without changing
the source. Preview, then import the file as a registered library:

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml
snp import pet ~/.config/pet/snippets.toml --dry-run
snp import pet ~/.config/pet/snippets.toml --library pet-snippets
```

Use `--merge` to add entries to an existing library, `--replace` to replace it
(with a backup), `--report json` for machine-readable output, or `--strict` to
treat warnings as errors. Unknown Pet-specific fields are reported but not
preserved. Import assigns new snippet IDs and resets sync-only state. Review
non-empty `output` values because snip-it uses them as stdout destination paths
when running snippets. See [Pet file compatibility and migration](PET_COMPATIBILITY.md)
for details.

## Agent / MCP integration

The local MCP adapter is a read-only stdio server launched by the agent
client. It reads the same libraries as the CLI, does not start sync or the
TUI, and never executes a returned command.

```bash
snp mcp serve
snp mcp instructions claude
snp mcp instructions codex
snp mcp instructions vscode
snp mcp instructions cursor
snp mcp instructions opencode
snp mcp instructions zed
```

`snippets_list` enumerates metadata, `snippets_search` uses snip-it's canonical
fuzzy ranking over description, command, and tags (with opt-in output/notes
matching and an explicit `tags` filter), and `snippet_get` resolves an exact
ID, unique exact description, or unique exact command. These are the same
identity fields as `snp get`. Each result includes the command as text for the
agent to use under its own execution and approval policy. `snp mcp install <client>` uses
noninteractive client commands where registration is stable. Otherwise, it
prints a schema-specific configuration block without changing user files. See
[MCP.md](MCP.md).

## Shell integration

snip-it generates Bash, Zsh, and Fish functions that integrate snippet
selection directly into your shell prompt. The generated code passes the
current command buffer as the initial search query, inserts the selected
snippet without executing it, and preserves the original buffer on
cancellation.

### Setup

```bash
# Bash: add to ~/.bashrc
eval "$(snp shell init bash)"

# Zsh: add to ~/.zshrc
eval "$(snp shell init zsh)"

# Fish: add to ~/.config/fish/config.fish
snp shell init fish | source
```

The `eval` and `| source` invocations execute trusted generated code from
your installed `snp` binary. Inspect the output first if you prefer:

```bash
snp shell init bash    # review the generated code
snp shell init bash | head -20   # or just the first 20 lines
```

### Generated functions

Each shell gets four public functions:

| Function | Behavior |
| --- | --- |
| `snp_select_raw` | Inserts the snippet with variable placeholders unchanged |
| `snp_select_expanded` | Prompts for variable values, then inserts the resolved command |
| `snp_new_current` | Saves the current shell buffer as a new snippet |
| `snp_new_previous` | Saves the previous accepted shell-history entry as a new snippet |

Internal helper functions handle shared selection and shell-specific capture
logic.

### Saving shell commands

The capture helpers are opt-in functions/widgets; they do not install a
keybinding. Bind them after sourcing, for example:

```bash
# Bash
bind -x '"\C-n": snp_new_current'
bind -x '"\C-p": snp_new_previous'

# Zsh
zle -N snp_new_current
zle -N snp_new_previous
bindkey '^N' snp_new_current
bindkey '^P' snp_new_previous

# Fish
bind \cn snp_new_current
bind \cp snp_new_previous
```

Pass metadata as arguments so the helper can keep stdin dedicated to command
data:

```bash
snp_new_current --description 'Current deployment command' --tags deploy,work
snp_new_previous --description 'Last accepted command' --library work
```

The helpers pass the command to `snp new --command-stdin`. This mode accepts
UTF-8 input up to 16 MiB and rejects NUL bytes. `--description` is required;
pass `--tags` explicitly if you do not want an interactive tag prompt. The
command is saved as data and is not executed.

### File and editor creation

Beyond `--command-stdin`, `snp new` supports two additional non-interactive
creation modes:

```bash
# Read a script file verbatim
snp new --from-file ./deploy.sh --description 'Deploy service'

# Open your editor to author the command
snp new --editor --description 'Complex pipeline'
```

`--from-file` reads a UTF-8 file of up to 16 MiB and stores its contents as the
command. `--editor` opens `$VISUAL`, then `$EDITOR`, then `vim`; editor arguments
such as `code --wait` are supported without invoking a shell. Both modes store
the command without executing it. Since they do not use stdin, the description
is optional and tags can be entered interactively.

The history helpers use each shell's history API rather than reading history
files. The Bash current-buffer helper requires Bash 4 or later; macOS's system
Bash 3.2 supports the previous-history helper but not current-buffer capture.
Shell history can contain credentials or other secrets, so review a command
before saving it.

### Binding keys

No keybindings are installed by default. After sourcing the generated
code, bind your preferred keys:

```bash
# Bash: in ~/.bashrc after the eval
bind -x '"\C-o": snp_select_raw'
bind -x '"\C-o\C-e": snp_select_expanded'

# Zsh: in ~/.zshrc after the eval
bindkey '^O' snp_select_raw
bindkey '^O^E' snp_select_expanded

# Fish: in ~/.config/fish/config.fish after the source
bind \co snp_select_raw
bind \co\e snp_select_expanded
```

These are shell-side bindings. The TUI has separate key maps for the selector,
theme picker, and variable prompts. Run `snp keybindings` for the complete TUI
reference.

### Selection and cancellation

The selected command replaces the current buffer only after a successful
selection. Cancelling restores the original buffer. Selection never executes
the command; press Enter when you are ready to run it.

### Pet migration

If you previously used pet's shell integration (typically a `pet-select`
function bound to Ctrl-T), the `snp_select_raw` function provides the same
workflow: search with the current buffer, insert without executing, edit
placeholders in place.

### Removal

Remove the `eval`/`source` line and any `bind`/`bindkey` lines you added.
No files are written by `snp shell init`; the generated code is printed to
stdout only.

## TUI key bindings

The selector starts in **insert mode**, where typing filters the list. Press
`Esc` to reach **normal mode** and use the navigation keys.

| Key | Normal mode | Insert mode |
|-----|-------------|-------------|
| `j` / `k` or `↓` / `↑` | Move down / up | `j` and `k` are literal characters |
| `h` / `l` or `←` / `→` | Move left / right | n/a |
| `gg` / `G` | Jump to top / bottom | n/a |
| `Ctrl+f` / `Ctrl+b` | Page down / up | n/a |
| `Ctrl+d` / `Ctrl+u` | Page down / up (helix bindings) | n/a |
| `v` / `V` | Visual mode (character / line) | n/a |
| `Enter` | Accept the selected snippet's action | Accept the selected snippet's action |
| `Esc` | No-op (use `q` to quit) | Return to normal mode |
| `/` | Search | Start a search |
| `i` | Insert mode | n/a |
| `y` | Copy the snippet and quit | n/a |
| `d` | Delete the snippet (confirm with `y`) | n/a |
| `e` | Open the theme picker | n/a |
| `q` | Quit | n/a |
| `t` | Toggle the tag filter | n/a |
| `n` / `o` / `a` / `z` | Sort by newest / oldest / a-z / z-a | n/a |
| `x` / `c` | Clear the filter | n/a |
| `Tab` | Toggle list display mode (command only / description + command) | Same |
| `Ctrl+g` | First theme (in the picker) | n/a |

The selected action depends on the command: `run` executes, `clip` copies, and
`search` displays snippet details.

The theme picker (`e`) filters with `i`, moves with `j`/`k`, pages with
`Ctrl+d`/`Ctrl+u`, and saves with `Enter`. `e` or `q` cancels and reverts to the
previous theme.

Variable prompts are modal: `Enter` saves the values, `q` returns to the
snippet selector (it does not quit), and `Ctrl+c` exits. `d` toggles the
default-value hint.

Run `snp keybindings` for the complete, current reference.

## Themes

Snip-it accepts Halloy color-theme files. Copy a theme file, not the Halloy
application config, into:

```text
$XDG_CONFIG_HOME/snp/themes/<name>.toml
```

The default directory is `~/.config/snp/themes/` when `XDG_CONFIG_HOME` is not
set. On first use, snp extracts 50 bundled themes into this directory without
overwriting files you have edited.

Press `e` in normal TUI mode to open the picker. Use `j`/`k` or the arrow keys
to preview, `i` to filter, and `Enter` to save. The selection is stored in
`themes.toml`:

```toml
active = "Dracula"
```

The parser follows Halloy's theme schema. Snip-it projects the available
colors onto its own TUI palette, so Halloy `font_style` and colors for UI
elements that snp does not have are ignored. See the [Halloy custom theme
guide](https://halloy.chat/guides/custom-themes) for the upstream format.

## Sync

Sync is provided by the optional self-hosted
[`snip-sync`](../snip-sync/README.md) server. The server uses SQLite and exposes a
gRPC API plus an HTTP health/metrics port. It does not terminate TLS itself.

For a complete local, Docker, reverse-proxy, and systemd deployment guide, see
[snip-sync/README.md](../snip-sync/README.md).

### Local development

```bash
# Install both binaries.
cargo install snip-it snip-sync

# Terminal 1: initialize and start the loopback server.
snip-sync init --skip-cert
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve

# Terminal 2: register this environment and seed the server.
snp register --server http://127.0.0.1:50051
snp sync run --push-only
```

`SNIP_SYNC_ALLOW_HTTP=true` is intentionally required for direct plaintext
HTTP. Use it only for loopback development.

### Sync direction

Registration enables sync and defaults to push-only. That is safe for a first
upload, but it does not download changes made elsewhere. Use the command flags
for one-off operations:

```bash
snp sync run --push-only
snp sync run --pull-only
snp sync run --dry-run
```

For regular multi-environment use, set the direction in
`$XDG_CONFIG_HOME/snp/sync.toml` (normally `~/.config/snp/sync.toml`):

```toml
[settings.sync]
sync_direction = "Bidirectional"
```

With that setting, plain `snp sync` merges local and remote changes.

### Conflict behavior

Client merges order shared fields by `(updated_at, device_id, SHA-256 of the
synced fields)`. A deletion wins over live content in a client merge, even if
the deletion has an older timestamp. The server uses a separate
`updated_at`-based upsert, so a newer live row can replace a server tombstone.
Local-only fields such as `output`, `folders`, and `favorite` are not synced
and stay local when remote shared fields are applied.

### Delta cursor

The server assigns every write a strictly increasing per-library sequence, and
the client stores that cursor next to `last_sync`, so a pull returns exactly
the rows written since the last sync instead of everything newer than a
second-granular timestamp. The cursor is additive: a client that talks to an
older server keeps using the timestamp cursor. `libraries.toml` records the
cursor type in `sync_cursor_kind`: `unknown`, `timestamp`, or `sequence`.

## Syncing one account across environments

The server authenticates a collection by API key. Every successful
`snp register` call mints a **new** account and API key on the server; it never
joins an existing account. If the environment already has a device ID,
`snp register` prints `Already registered!` and exits without contacting the
server. Pass `--force` to create a new account and key. To share one account:

1. Register one environment against the remote HTTPS endpoint.
2. Use `snp sync run --push-only` there to create and seed the server libraries.
3. Provision the same API key, server URL, and a unique device ID to each other
   environment.
4. Run `snp sync run --pull-only` once on each new environment, then switch all of
   them to `Bidirectional` for normal use.

Registering separately on the other environments creates isolated accounts
that cannot see the first environment's libraries. The server stores only a
hash of the API key, so there is no server-side key recovery. Keep the original
key in a password manager or managed secret store. The CLI prints a masked key
after registration; use the first environment's OS keychain tools or your
secret-management workflow to provision the original value. Copying a
`sync.toml` containing only `api_key = "@keychain"` does not copy the keychain
secret itself.

On an additional environment, the settings have this shape:

```toml
# $XDG_CONFIG_HOME/snp/sync.toml
[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "@keychain"
device_id = "<a unique ID for this environment>"
sync_interval_minutes = 30
auto_sync = false
sync_direction = "Bidirectional"
```

The API key is normally represented as `@keychain` in `sync.toml`; the secret
itself stays in the OS keychain. Moving the config directory or changing
`XDG_CONFIG_HOME` does not remove the keychain entry. On a headless environment,
provision the same key in the supported OS keychain and set a unique
`device_id` in `sync.toml`. Do not run `snp register` there because it creates
a new account and key. Snip-it refuses to save the key as plaintext if the
keychain is unavailable. Do not commit `sync.toml`. See
[THREAT_MODEL.md](THREAT_MODEL.md) for the credential-handling contract.

## Auto-sync

Auto-sync is disabled by default. When enabled, snippet changes schedule a
separate background worker after the local save. The command returns without
waiting for the remote sync to finish.

### Configuration

```bash
snp sync config --show                         # inspect current settings
snp sync config --auto-sync on                 # enable auto-sync
snp sync config --auto-sync off                # disable auto-sync
snp sync config --debounce 5                   # 5-second debounce (0-300 seconds)
snp sync config --failure warn                 # ignore, warn, or error
```

Or edit `~/.config/snp/sync.toml` directly:

```toml
[settings.sync]
enabled = true
auto_sync = true
auto_sync_debounce_seconds = 2
auto_sync_failure = "warn"
```

### Trigger matrix

| Command | Trigger? | Notes |
|---------|----------|-------|
| `snp new` (all sources) | Yes | Positional, stdin, file, editor, multiline, interactive |
| `snp edit` (command/description) | Yes | After editor saves |
| `snp edit --output` | No | Output is local-only, not in sync protocol |
| TUI delete | Yes | After tombstone persisted |
| `snp delete` | Yes | Same tombstone and notification as the TUI delete |
| `snp organize` | Yes | Local-only `favorite` and `folders` updates |
| `snp import pet` (create/merge/replace) | Yes | Once per import, not per snippet |
| `snp import pet --dry-run` | No | Read-only |
| `snp library create` | Yes | |
| `snp library delete` | Yes | |
| `snp library list/show` | No | Read-only |
| `snp library set-primary` | No | Rewrites `libraries.toml` (primary flag and generation); does not trigger auto-sync |
| `snp sync` (explicit) | Clears pending | Prevents duplicate delayed sync |

### Behavior

- The local change is saved before sync starts. A sync failure never rolls back a successful local change.
- A separate worker handles sync, so the command does not wait for the debounce period.
- Manual and automatic sync runs are serialized to prevent overlapping requests.
- Pending work is retained when it cannot be completed, and sync-merge writes do not schedule another sync.

### Failure modes

| Mode | Behavior |
|------|----------|
| `ignore` | The scheduling failure is logged at debug level only. |
| `warn` (default) | The scheduling failure is logged at warn level. |
| `error` | The scheduling failure is logged at error level. |

The setting controls the log level only. If the worker cannot be scheduled,
the mutating command prints a warning to stderr. The local change remains
saved, and scheduling failures do not change the command's exit status. Worker
failures appear in `~/.config/snp/logs/` and in `snp doctor`.

## Premade libraries

Premade libraries are read-only source files served by `snip-sync` and copied
into the local `premade/` directory when installed.

```bash
snp premade list
snp premade search docker
snp premade get docker-essentials
snp premade get all
snp premade sync
snp premade update docker-essentials
```

Server administrators can add `.toml` files to the server's configured
`premade-libraries` directory. The server reads the same snippet format shown
above.

## Sorting and ranking

`run`, `clip`, `search`, `select`, and `list` accept `--sort` to choose an
explicit ordering mode. The default remains fuzzy relevance, so existing
invocations without `--sort` are unchanged.

### Sort modes

| Mode | Behavior |
| --- | --- |
| `relevance` (default) | Fuzzy score descending. Usage counts and timestamps are used only as bounded tie-breakers. |
| `recent` | `updated_at` descending, then `created_at` descending. Malformed timestamps sort after valid ones. |
| `last-used` | Local `last_used_at` descending. Never-used snippets follow used snippets. |
| `most-used` | Local `use_count` descending, then `last_used_at` descending. |
| `description` | Case-insensitive alphabetical. |
| `command` | Case-insensitive alphabetical. |

### Favorites first

`--favorites-first` groups favorited snippets before non-favorites in any sort
mode. Within each group, the selected sort mode applies unchanged. This flag
can be combined with any `--sort` value.

### Usage tracking

Successful `run` and `clip` operations record usage metadata locally:

- `use_count`: incremented on each successful operation
- `last_used_at`: set to the current Unix timestamp

Usage data is stored in `~/.config/snp/usage.toml`, separate from snippet
libraries. It is never synced, never written into library TOML files, and never
logged as command text. Corrupt or missing usage data fails open to zero usage.

Cancellation and failed operations do not update usage. Search preview and
browsing do not count as usage.

### TUI sort indicators

The TUI shows the active sort mode in the status bar:

| Indicator | Mode |
| --- | --- |
| (none) | Relevance (default) |
| `[new]` | Recent |
| `[old]` | Oldest first |
| `[a-z]` | Description A→Z |
| `[z-a]` | Description Z→A |
| `[used]` | Last used (via `--sort last-used`; not reachable by interactive cycling) |
| `[freq]` | Most used (via `--sort most-used`; not reachable by interactive cycling) |
| `[cmd]` | Command text A→Z (initial-only, via `snp run --sort command`; not reachable by interactive cycling) |

Press `n` to toggle sort-by-newest, `o` for sort-by-oldest, `a` for
description A→Z, and `z` for description Z→A. Each key toggles its own mode
rather than cycling through a list. There is no interactive favorites-first
toggle, so pass `--favorites-first` instead. `[used]` and `[freq]` are only
available through `--sort last-used` and `--sort most-used`; no key selects them.

### Machine output ordering

`list --json` and `list --csv` output respects `--sort` and
`--favorites-first` flags. Without explicit flags, output uses the default
relevance ordering.

### Determinism

Sorting is fully deterministic. Identical inputs always produce identical
output, including tie-breaks. Ties are resolved by: primary sort key,
favorites-first grouping, fuzzy relevance (when meaningful), normalized
description, and original index.

### Output / notes metadata

Each snippet can store descriptive metadata in the `output` field (also
called "notes"). The field is preserved during Pet import and backup, but it is
local-only and is not synced.

The field doubles as a stdout redirect target. When `output` is non-empty,
`snp run` writes the snippet's stdout to the file named by `output`, resolved
relative to the current working directory. The output file must not already
exist, and paths outside the working directory are refused. Only stdout is
redirected; stderr still goes to the terminal. The Pet difference is covered in
[PET_COMPATIBILITY.md](PET_COMPATIBILITY.md).

In the TUI preview, output appears below the command separated by
`--- Output / Notes ---` when present. Empty output is hidden.

#### Editing output

```bash
snp edit --output "Example output for this command" --filter "ssh"
snp edit --output-stdin --filter "ssh" < notes.txt
snp edit --clear-output --filter "ssh"
```

`--output`, `--output-stdin`, and `--clear-output` are mutually exclusive.
`--filter` matches by description or command substring, and it is required
only when no exact selector is given. `--id` (snippet ID),
`--description-exact`, and `--command-exact` select the snippet directly and
replace the filter; those three are mutually exclusive with each other.

```bash
snp edit --output "Example output for this command" --description-exact "SSH connection"
```

IDs are derived from snippet content, so look one up rather than pasting a
literal:

```bash
snp get --query "SSH connection" --field id
snp edit --output "Example output for this command" --id "<that id>"
```

#### Output-aware search

```bash
snp list --search-output --filter "sample"
```

Fuzzy search matches description, command, and tags. The
`--search-output` flag additionally includes the output field in fuzzy
matching (bounded to 512 characters for scoring). Output is always included
in JSON and CSV export regardless of this flag.

#### Tag-aware search

Tags participate in fuzzy matching alongside description and command, so
`snp list --filter <tag-text>` and `snp get --query <tag-text>` can find
snippets by their organizational metadata. For a predictable exact-tag
restriction, MCP `snippets_search` also accepts an explicit `tags` array
(every listed tag must be present, case-insensitive); the CLI equivalent is
to refine the fuzzy filter.

## Variables

Variables are expanded when a snippet is run or copied:

| Syntax | Behavior |
| --- | --- |
| `<name>` | Prompt for a required value |
| `<name=default>` | Show a default that can be replaced |
| `<name=\|_opt1_\|\|_opt2_\|\|_opt3_\|\|>` | Pet-style multiple choice selector |
| `\<` or `\>` | Use a literal angle bracket |

Example:

```toml
[[snippets]]
description = "SSH connection"
command = "ssh <user=root>@<host> -p <port=22>"
tag = ["ssh"]
```

### Pet Multiple-Choice Variables

snip-it recognizes Pet-compatible multiple-choice syntax:

```toml
[[snippets]]
description = "Deploy target"
command = "ssh <user>@<host> -p <port=|_22_||_8022_||_2222_||>"
tag = ["ssh"]
```

When the snippet is executed or copied, a list selector is shown with the
available choices. The first choice is the default. Use arrow keys (or `j`/`k`
in normal mode) to select, then press Enter.

The raw command text stays unchanged in storage. Choices are expanded only
during interactive prompting.

Shell expressions such as `$HOME` or `$(date)` are passed to the selected
shell; snp does not expand them itself.

## Configuration

The client root is `$XDG_CONFIG_HOME/snp`, or `~/.config/snp` when that
variable is unset. The first write creates `snippets.toml` (the default
single-file library), `logs/`, and `.transaction/`; reading a library creates
nothing at all. The rest appear as you use the features.

| Path | Purpose |
| --- | --- |
| `snippets.toml` | Default single-file library (copied into `libraries/` by library mode) |
| `libraries/*.toml` | User library files |
| `libraries.toml` | Library metadata and server links |
| `premade/*.toml` | Installed premade libraries |
| `sync.toml` | Sync settings (the key itself lives in the OS keychain; this file holds only the `@keychain` marker) |
| `themes/*.toml` | Halloy-compatible themes |
| `themes.toml` | Active theme |
| `usage.toml` | Local usage counts and last-used timestamps |
| `logs/` | Rolling application logs |
| `audit.log` | Audit trail of local snippet mutations (rotated at 10 MB, 30-day retention) |
| `backups/` | Timestamped library backups |
| `.transaction/` | Transaction journals and the local-data lock for crash-safe writes |
| `auto-sync-pending.toml` | Pending auto-sync marker (latest mutation generation) |
| `auto-sync-status.toml` | Result of the last auto-sync attempt and its retry schedule |
| `auto-sync-execution.lock` | Kernel lock serializing every sync execution |
| `auto-sync-worker.lock` | PID+nonce lock preventing concurrent auto-sync workers |

Useful environment variables:

| Variable | Purpose | Default |
| --- | --- | --- |
| `SNP_COMMAND_TIMEOUT` | Command timeout in seconds; `0` disables it | no timeout (300 s when the snippet writes to an `output` file) |
| `SNP_CLIPBOARD_TIMEOUT` | Clipboard timeout in seconds | `5` |
| `SNP_SYNC_CONNECT_TIMEOUT` | Sync connection timeout in seconds | `10` |
| `SNP_SYNC_REQUEST_TIMEOUT` | Sync request timeout in seconds | `30` |
| `SNP_THEME` | Legacy theme or theme filename | bundled default |
| `SNP_LOG` / `RUST_LOG` | Tracing filters | `snip_it=info` (a bare level such as `info` also works) |
| `EDITOR` | Editor used when `VISUAL` is unset or empty | `vim` |
| `VISUAL` | Editor for `snp edit` and `snp new --editor` (takes precedence over `EDITOR`) | unset |

## Automation

`snp list` supports machine-readable output:

```bash
snp list --json
snp list --csv
snp list --filter docker --json
snp list --search-output --filter sample
```

An empty filter is "no filter": `snp list --filter ""` returns exactly what
`snp list` returns. The same rule applies to `snp get --query ""` and to the
MCP `snippets_search` tool, so one query selects the same snippets on all
three surfaces.

### Scripting with `snp get`

`snp get` is a deterministic, non-TUI lookup for scripts. It never opens a
selector or executes a command:

```bash
snp get --query 'push a branch' --field command   # print the command
snp get --query 'push a branch' --json            # full record as JSON
```

It exits 3 when nothing matches and 5 when a query matches more than one
snippet, so a script can branch on the result instead of parsing stderr. The
full table is [EXIT_CODES.md](EXIT_CODES.md).

### Scripting with `snp delete`

`snp delete` tombstones a snippet without a TUI and without a confirmation
prompt, producing exactly the state the TUI's `d` key produces:

```bash
snp delete --id "$SNIPPET_ID"       # tombstone one snippet
snp delete --query 'push a branch'  # fuzzy match; 5 if it is ambiguous
```

It exits 0 when the snippet was deleted, 3 when nothing matched, 5 when the
selector matched more than one snippet, and 9 when the snippet was already
deleted. Nothing is deleted unless exactly one snippet matches.

`snp data delete` is the same command. A delete targets one snippet in one
library, so `--library all` is refused. Add `--json` for a machine-readable
result whose `status` is `deleted` or `already_deleted`.

### Scripting with `snp organize`

`snp organize` sets the local-only `favorite` flag and folder memberships on
one snippet:

```bash
snp organize --id "$SNIPPET_ID" --favorite
snp organize --id "$SNIPPET_ID" --add-folder work
snp organize --query 'deploy staging' --unfavorite --json
```

It uses the same selector and ambiguity rules as `snp delete`. Re-applying the
current state exits 0 with `status: "unchanged"` in `--json` mode and does not
rewrite the library file. `snp data organize` mirrors the command.

`snp cron` prints a cron entry and offers to copy it to the clipboard:

```bash
snp cron          # every 15 minutes
snp cron --interval 60
```

On Linux, a user systemd timer is another option:

```ini
# ~/.config/systemd/user/snp-sync.service
[Unit]
Description=Sync snip-it libraries

[Service]
Type=oneshot
ExecStart=%h/.local/bin/snp sync
```

```ini
# ~/.config/systemd/user/snp-sync.timer
[Unit]
Description=Periodic snip-it sync

[Timer]
OnBootSec=5min
OnUnitActiveSec=15min
Unit=snp-sync.service

[Install]
WantedBy=default.target
```

Enable it with:

```bash
systemctl --user daemon-reload
systemctl --user enable --now snp-sync.timer
```

### Shell completions

`snp completions` writes a completion script to stdout:

```bash
snp completions bash > ~/.local/share/bash-completion/completions/snp
snp completions zsh  > ~/.local/share/zsh/site-functions/_snp
snp completions fish > ~/.config/fish/completions/snp.fish
```

The accepted shells are `bash`, `elvish`, `fish`, `powershell`, and `zsh`.

### Keeping snp current

```bash
snp update --dry-run   # report the available version and how it would install
snp update             # install using the detected installation method
```

`snp update` detects whether the binary was installed with Cargo, Homebrew, or
a direct download, then follows that method. A Homebrew-managed `snp` stays
owned by Homebrew.

## Troubleshooting and recovery

Check the server before investigating the client:

```bash
curl http://127.0.0.1:50050/health
```

For a remote deployment, check the reverse proxy's HTTPS endpoint and the
server logs. `snip-sync paths` prints the active database, config, data, and
PID paths.

### Local state commands

| Command | Purpose |
| --- | --- |
| `snp status` | Show library counts, sync state, pending auto-sync work, and the log directory (`--json`, `--sync-only`) |
| `snp validate` | Read-only validation of every library (`--library <name>`, `--strict`, `--json`) |
| `snp backup` | Write a checksummed snapshot of the libraries and `libraries.toml` (`--output`, `--include-usage`, `--include-sync-state`, `--json`) |
| `snp restore <BACKUP_DIR>` | Restore a snapshot (`--mode merge\|replace\|dry-run`, `--json`) |
| `snp repair` | Inspect and repair config, library, and interrupted-transaction state (`--dry-run`, `--apply`, `--library`, `--json`) |

`snp status` is read-only and never starts a sync, so it is safe to run at
any time. `snp validate` only reports findings and never writes.

`snp backup` writes a timestamped snapshot directory under `backups/`
containing `manifest.toml` plus the library files, each entry recorded with a
SHA-256 checksum. Usage metadata is excluded unless `--include-usage` is
passed, and `sync.toml` is excluded unless `--include-sync-state` is passed
(the API key is redacted when it is).

`snp restore <BACKUP_DIR>` validates the manifest and the checksummed
artifacts before touching anything. `--mode dry-run` prints the planned
restore without writing. `--mode merge` (the default) combines each library
with the local copy by snippet ID, keeping whichever version has the newer
`updated_at` and adding snippets that only exist in the backup; an identical
file is skipped, and an existing `libraries.toml`, `usage.toml`, or
`sync.toml` is left as it is. `--mode replace` overwrites the library files
and takes a pre-restore backup of the config directory first. Every restore
runs inside a transaction, so a failure rolls back.

`snp repair` reports problems without changing anything until `--apply` is
passed. It creates a backup before applying safe repairs. Findings are labelled
`[auto]` when they can be fixed mechanically or `[manual]` when they need an
operator decision. Manual findings are reported with a `Fix:` line and are
never applied automatically.

Exit codes follow the findings:

| Exit | Meaning |
|------|---------|
| 0 | No findings; only `[auto]` findings with none applied; `--dry-run`; or `--apply` applied every safe repair |
| 10 | At least one finding is `[manual]` and awaits an operator decision |
| 1 | `--apply` was passed and at least one repair failed |

Only `[auto]` findings are attempted by `--apply`; `[manual]` ones are skipped
and still counted as requiring review. With `--json`, the report's
`exit_status` is one of `clean`, `findings_safe`, `repaired`, `partial_failure`,
`unsafe_only`, or `dry_run`.

If a sync configuration is damaged, snp keeps a `.corrupt.bak` copy and falls
back to defaults. Library writes create timestamped backups in `backups/`.
Use `snp restore` to bring back a whole snapshot; to recover a single library
file, copy it back from `backups/` before attempting another sync.

To disconnect sync while keeping local snippets, remove `sync.toml` and
register again later. To reset all local data, back up the config directory
first, then remove it:

```bash
mv "${XDG_CONFIG_HOME:-$HOME/.config}/snp" \
   "${XDG_CONFIG_HOME:-$HOME/.config}/snp.backup-$(date +%Y%m%d)"
```

For security issues, follow [../SECURITY.md](../SECURITY.md) rather than opening a
public issue.

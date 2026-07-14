# snp user guide

This guide covers the parts of `snp` that are easy to outgrow from the
[README](README.md): libraries, TOML compatibility, themes, synchronization,
and automation.

## Contents

- [Libraries](#libraries)
- [Pet compatibility and import](#pet-compatibility-and-import)
- [Shell integration](#shell-integration)
- [Themes](#themes)
- [Sync](#sync)
- [Syncing one account across environments](#syncing-one-account-across-environments)
- [Premade libraries](#premade-libraries)
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

snp new --library work 'kubectl get pods -n <namespace=default>'
snp list --library work
snp run --library work
snp clip --library work
snp edit --library work
```

In the snippet TUI, press `d` in normal mode to delete the selected snippet.
The confirmation box requires `y` to proceed; any other key cancels. The key
remains ordinary filter input in insert mode. Deleted snippets are hidden from
the TUI, and `--sync` propagates the deletion when sync is configured.

Library files live under:

```text
$XDG_CONFIG_HOME/snp/libraries/
```

When `XDG_CONFIG_HOME` is not set, the default is `~/.config/snp/libraries/`.
`libraries.toml` stores library metadata, including which library is primary
and which server library it is linked to.

If a legacy `snippets.toml` exists when library mode starts, snp migrates it to
`libraries/snippets.toml` and retains the original file. Keep your own backup
before making large changes.

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

#### Pre-migration diagnostics

`snp doctor` has four modes:

- **`--pet-file <path>`** — Analyze a pet snippet file for compatibility issues.
  Reports TOML parse status, unknown fields, missing required fields, empty
  commands, choice variables, duplicates, output fields, and normalization
  previews. Suggests the exact import command to run based on findings.

- **`--compatibility`** — Audit the installed snp environment. Checks binary
  version, config directory, library directory, primary library, sync config,
  shell availability (bash/zsh/fish), shell init syntax validation, editor
  configuration, legacy paths, Release 1 `snp select` availability, Release 2
  acquisition flags (`--command-stdin`, `--from-file`, `--editor`), and Release 3
  choice-variable parser.

- **`--library <name>`** — Analyze a specific library file (resolved from
  `~/.config/snp/libraries/` or a literal path). Same analysis as `--pet-file`
  but targets a snp library.

- **`--check-shell <bash|zsh|fish>`** — Validate the syntax of `snp shell init`
  output for the specified shell. Generates the init code, then runs the shell's
  syntax checker (`bash -n`, `zsh -n`, or `fish --no-execute`).

All modes support `--report human|json` for output format and `--strict` to
treat warnings as errors. Diagnostics include `SourceSpan` byte-offset
locations for precise positioning within the source file.

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml
snp doctor --pet-file snippets.toml --report json
snp doctor --compatibility
snp doctor --check-shell zsh
```

The source file is never modified. Merged and replaced libraries are backed up
before overwrite.

Alternatively, you can copy the file directly:

```bash
SNP_CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/snp"
snp library create pet
cp /path/to/pet-snippets.toml "$SNP_CONFIG_DIR/libraries/pet.toml"
snp library set-primary pet
```

Copy the file only after `snp library create pet`, so the library metadata is
registered.

Snip-it also reads older `[[Snippets]]` files and capitalized aliases such as
`Description`, `Command`, `Tag`, and `Output`. Snip-it adds optional metadata
(`id`, `folders`, `favorite`, timestamps, and sync state); pet will ignore data
it does not know about, so use a separate copy if you need to preserve that
metadata while editing the same collection with both tools.

## Shell integration

snip-it generates Bash, Zsh, and Fish functions that integrate snippet
selection directly into your shell prompt. The generated code passes the
current command buffer as the initial search query, inserts the selected
snippet without executing it, and preserves the original buffer on
cancellation.

### Setup

```bash
# Bash — add to ~/.bashrc
eval "$(snp shell init bash)"

# Zsh — add to ~/.zshrc
eval "$(snp shell init zsh)"

# Fish — add to ~/.config/fish/config.fish
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

The binary receives the command through `snp new --command-stdin`. It stores
valid UTF-8 exactly, including zero or more supplied trailing newlines; it does
not trim, normalize, evaluate, execute, or log the command body. The stdin
ingestion limit is 16 MiB, and invalid UTF-8 or NUL bytes are rejected before
the library is changed. `--description` is required in this mode; omit
`--tags` or pass an explicit comma/space-separated value rather than using the
interactive tag prompt.

### File and editor creation

Beyond `--command-stdin`, `snp new` supports two additional non-interactive
creation modes:

```bash
# Read a script file verbatim
snp new --from-file ./deploy.sh --description 'Deploy service'

# Open your editor to author the command
snp new --editor --description 'Complex pipeline'
```

`--from-file` reads the specified file as exact UTF-8 command data. It rejects
invalid UTF-8, NUL bytes, files larger than 16 MiB, and non-regular files
(directories, FIFOs, sockets, device nodes, broken symlinks). Symlinks to
regular files are followed. The file content is stored verbatim — no trimming,
evaluation, or execution.

`--editor` opens `$VISUAL` (if set), then `$EDITOR`, falling back to `vim`, with
a temporary file. The temp file is created atomically in the OS temp directory
with `0600` permissions and cleaned up automatically after the editor exits.
Editor command specifications support arguments — values like `code --wait`,
`nvim -f`, or `"/path with spaces/bin/code" --wait` are parsed with shell-word
semantics and passed through to the editor directly; no shell is invoked. After
the editor exits, empty content or a failed invocation returns an error.
Non-empty content is stored verbatim.

Both modes do not consume stdin, so `--description` is optional and tags can be
prompted interactively.

The Bash and Zsh previous helpers use their native history APIs and avoid
self-capture when invoked as ordinary functions or widgets. Fish uses its
native history search API. No helper reads `.bash_history`, `.zsh_history`, or
Fish history files, and none prints the captured command as a status message.
The Bash current-buffer widget requires a Bash Readline with `READLINE_LINE`
(Bash 4+); the macOS system Bash 3.2 can still use the generated history
helper, but cannot expose the active buffer to this widget.
Shell history may contain credentials, access tokens, private URLs, or other
secrets; inspect a command before saving it and avoid putting secrets in
history in the first place.

### Binding keys

No keybindings are installed by default. After sourcing the generated
code, bind your preferred keys:

```bash
# Bash — in ~/.bashrc after the eval
bind -x '"\C-o": snp_select_raw'
bind -x '"\C-o\C-e": snp_select_expanded'

# Zsh — in ~/.zshrc after the eval
bindkey '^O' snp_select_raw
bindkey '^O^E' snp_select_expanded

# Fish — in ~/.config/fish/config.fish after the source
bind \co snp_select_raw
bind \co\e snp_select_expanded
```

### How it works

1. The shell function reads the current command buffer.
2. It writes a temp file path and calls `snp select --query <buffer> --raw`
   (or `--expanded`) with `--output-file <tmpfile>`.
3. On success the temp file contents replace the buffer; on cancellation
   (exit code 4) the original buffer is restored exactly.
4. The temp file is removed immediately after reading.

The temp-file transport is lossless: multiline snippets, quotes, backslashes,
Unicode, and shell metacharacters are preserved byte-for-byte. The generated
code never uses `eval` on selected content.

### Cancellation and errors

- **User cancels** (Esc/Ctrl-C in the TUI): exit code 4, original buffer
  restored, no output printed.
- **Operational failure** (snp not found, library missing, etc.): stderr
  diagnostic shown, original buffer restored, non-zero exit code.
- **Success**: the selected command replaces the buffer; cursor moves to the
  end. The user presses Enter explicitly to execute.

### Pet migration

If you previously used pet's shell integration (typically a `pet-select`
function bound to Ctrl-T), the `snp_select_raw` function provides the same
workflow: search with the current buffer, insert without executing, edit
placeholders in place.

### Removal

Remove the `eval`/`source` line and any `bind`/`bindkey` lines you added.
No files are written by `snp shell init`; the generated code is printed to
stdout only.

## Themes

Snip-it accepts Halloy color-theme files. Copy a Halloy theme file—not the
Halloy application config—into:

```text
$XDG_CONFIG_HOME/snp/themes/<name>.toml
```

The default is `~/.config/snp/themes/` when `XDG_CONFIG_HOME` is not set. On
first use, snp extracts 50 bundled themes into this directory without
overwriting files you have edited.

Press `e` in normal TUI mode to open the picker. Use `j`/`k` or the arrow keys
to preview, `i` to filter, and `Enter` to save. The selection is stored in
`themes.toml`:

```toml
[active]
name = "Dracula"
```

The parser follows Halloy's theme schema. Snip-it projects the available
colors onto its own TUI palette, so Halloy `font_style` and colors for UI
elements that snp does not have are ignored. See the [Halloy custom theme
guide](https://halloy.chat/guides/custom-themes) for the upstream format.

## Sync

Sync is provided by the optional self-hosted
[`snip-sync`](snip-sync/README.md) server. The server uses SQLite and exposes a
gRPC API plus an HTTP health/metrics port. It does not terminate TLS itself.

For a complete local, Docker, reverse-proxy, and systemd deployment guide, see
[snip-sync/README.md](snip-sync/README.md).

### Local development

```bash
# Install both binaries.
cargo install snip-it snip-sync

# Terminal 1: initialize and start the loopback server.
snip-sync init --skip-cert
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve

# Terminal 2: register this environment and seed the server.
snp register --server http://127.0.0.1:50051
snp sync --push-only
```

`SNIP_SYNC_ALLOW_HTTP=true` is intentionally required for direct plaintext
HTTP. Use it only for loopback development.

### Sync direction

Registration enables sync and defaults to push-only. That is safe for a first
upload, but it does not download changes made elsewhere. Use the command flags
for one-off operations:

```bash
snp sync --push-only
snp sync --pull-only
snp sync --dry-run
```

For regular multi-environment use, set the direction in
`$XDG_CONFIG_HOME/snp/sync.toml` (normally `~/.config/snp/sync.toml`):

```toml
[settings.sync]
sync_direction = "Bidirectional"
```

With that setting, plain `snp sync` merges local and remote changes.

### Conflict behavior

Shared fields use last-write-wins ordering based on `updated_at`; the server
wins when timestamps are equal. Local-only fields such as `output`, `folders`,
and `favorite` are not synchronized and are preserved locally when a remote
version replaces the shared fields.
Deleted snippets are retained as tombstones so a later sync can converge.

## Syncing one account across environments

The server authenticates a collection by API key. `snp register` creates a new
account and API key on every invocation; it does not join an existing account.
Therefore:

1. Register one environment against the remote HTTPS endpoint.
2. Use `snp sync --push-only` there to create and seed the server libraries.
3. Provision the same API key, server URL, and a unique device ID to each other
   environment.
4. Run `snp sync --pull-only` once on each new environment, then switch all of
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
api_key = "<the same API key as the first environment>"
device_id = "<a unique ID for this environment>"
sync_interval_minutes = 30
auto_sync = false
sync_direction = "Bidirectional"
```

The API key is normally represented as `@keychain` after snp stores it in the
OS keychain. When provisioning a headless environment, use its secret manager
or set `SNP_ALLOW_PLAINTEXT_API_KEY=true` only when you deliberately accept a
protected plaintext `sync.toml`. Do not commit this file.

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

The raw command text is preserved in storage — choices are only expanded
during interactive prompting.

Shell expressions such as `$HOME` or `$(date)` are passed to the selected
shell; snp does not expand them itself.

## Configuration

The client root is `$XDG_CONFIG_HOME/snp`, or `~/.config/snp` when that
variable is unset.

| Path | Purpose |
| --- | --- |
| `snippets.toml` | Legacy single-file storage |
| `libraries/*.toml` | User library files |
| `libraries.toml` | Library metadata and server links |
| `premade/*.toml` | Installed premade libraries |
| `sync.toml` | Sync settings |
| `themes/*.toml` | Halloy-compatible themes |
| `themes.toml` | Active theme |
| `logs/` | Rolling application logs |
| `backups/` | Timestamped library backups |

Useful environment variables:

| Variable | Purpose | Default |
| --- | --- | --- |
| `SNP_COMMAND_TIMEOUT` | Command timeout in seconds; `0` disables it | `0` for direct terminal runs |
| `SNP_CLIPBOARD_TIMEOUT` | Clipboard timeout in seconds | `5` |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | Permit plaintext API-key storage when keychain storage fails | unset |
| `SNP_SYNC_CONNECT_TIMEOUT` | Sync connection timeout in seconds | `10` |
| `SNP_SYNC_REQUEST_TIMEOUT` | Sync request timeout in seconds | `30` |
| `SNP_THEME` | Legacy theme or theme filename | bundled default |
| `SNP_LOG` / `RUST_LOG` | Tracing filters | `snp=info` |
| `EDITOR` | Editor for `snp edit` and fallback for `snp new --editor` | `vim` |
| `VISUAL` | Editor for `snp new --editor` (overrides `EDITOR` if non-empty) | unset |

## Automation

`snp list` supports machine-readable output:

```bash
snp list --json
snp list --csv
snp list --filter docker --json
```

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

## Troubleshooting and recovery

Check the server before investigating the client:

```bash
curl http://127.0.0.1:50050/health
```

For a remote deployment, check the reverse proxy's HTTPS endpoint and the
server logs. `snip-sync paths` prints the active database, config, data, and
PID paths.

If a sync configuration is damaged, snp keeps a `.corrupt.bak` copy and falls
back to defaults. Library writes create timestamped backups in `backups/`.
Restore a library file from there before attempting another sync.

To disconnect sync while keeping local snippets, remove `sync.toml` and
register again later. To reset all local data, back up the config directory
first, then remove it:

```bash
mv ~/.config/snp ~/.config/snp.backup-$(date +%Y%m%d)
```

For security issues, follow [SECURITY.md](SECURITY.md) rather than opening a
public issue.

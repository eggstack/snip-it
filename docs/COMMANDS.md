# snp Command Overview

This page summarizes the main commands and common selection behavior. For the
complete, current flag list, run `snp --help` or `snp <command> --help`. The
[User Guide](USER_GUIDE.md) has task-oriented examples, and
[Command Contracts](COMMAND_CONTRACTS.md) documents behavior for scripts and
operators.

## Everyday commands

| Command | Alias | What it does |
|---------|-------|--------------|
| `snp new` | `n` | Create a snippet interactively or supply its command on the command line. |
| `snp list` | `l` | List snippets. Use `--json` or `--csv` for structured output. |
| `snp get` | none | Retrieve one or more snippets without opening the TUI or executing commands. |
| `snp delete` | none | Delete one snippet by adding a syncable tombstone. No confirmation prompt. |
| `snp organize` | `o` | Set a snippet's favorite flag or update its folder memberships. |
| `snp run` | `r` | Select and run a snippet through your shell. |
| `snp clip` | `c` | Select a snippet and copy its expanded command to the clipboard. |
| `snp search` | `s` | Browse and search snippets in the TUI. |
| `snp select` | `sel` | Select a snippet and print its command without running it. |
| `snp edit` | `e` | Open a library file in your editor, or edit one snippet's output/notes field. |
| `snp version` | `v` | Print the installed version. |

Commands that execute snippets run them through your configured shell. Only run
commands you trust. `snp run` returns the snippet process's exit code.

## Finding snippets

Use `snp get` for scripts and other non-interactive lookups. Choose exactly one
selector:

| Selector | Matches |
|----------|---------|
| `--id <ID>` | Exact, case-sensitive snippet ID. |
| `--description-exact <TEXT>` | Exact description, ignoring case. |
| `--command-exact <TEXT>` | Exact command text, ignoring case. |
| `--query <TEXT>` | Fuzzy match over description, command, and tags. |

For example:

```bash
snp get --query 'push a branch' --field command
snp get --query 'push a branch' --json
```

The default `--resolution unique` requires exactly one match. Use
`--resolution first` or `--resolution all` when your workflow intentionally
allows multiple matches. `snp get` exits 3 when there is no match and 5 when the
selected resolution treats multiple matches as ambiguous. See
[Exit Codes](EXIT_CODES.md).

For a list, use `snp list --filter <TEXT>`. Search includes descriptions,
commands, and tags. An empty filter means no filter. Add `--search-output` to
include output/notes in a list search. `snp get --query` does not search
output/notes.

## Deleting and organizing

`snp delete` removes one snippet from normal views and retains a tombstone for
sync. It does not prompt, so use an exact selector or a query that resolves to
one intended snippet:

```bash
snp delete --id "$SNIPPET_ID"
snp delete --query 'temporary branch'
```

Exactly one selector is required. `--library all` is not supported for delete.
Use `--json` for a machine-readable result. The command exits 3 if there is no
match, 5 if the selector is ambiguous, and 9 if the snippet is already deleted.

`snp organize` updates one snippet's local-only `favorite` and `folders` fields:

```bash
snp organize --id "$SNIPPET_ID" --favorite
snp organize --id "$SNIPPET_ID" --add-folder work
```

It requires exactly one selector and one action, and does not accept
`--library all`. See `snp organize --help` for the available actions.

## Libraries

| Command | What it does |
|---------|--------------|
| `snp library list` | List registered libraries and identify the primary. |
| `snp library create <NAME>` | Create a library. The name `all` is reserved. |
| `snp library delete <NAME>` | Delete a library. Use `--force` to skip confirmation. |
| `snp library set-primary <NAME>` | Choose the default library for new snippets and unscoped list operations. |
| `snp library show [NAME]` | Show library details. |

A fresh install starts with `snippets.toml`. The first library-mode command
registers that file as the primary `snippets` library. Use `--library <NAME>`
to select another library or `--library all` where the command supports
cross-library operations. See [Library Scope](LIBRARY_SCOPE.md).

## Sync and accounts

| Command | What it does |
|---------|--------------|
| `snp register --server <URL>` | Create a sync account and register this environment. `--force` registers again with a new account and key. |
| `snp sync run` | Run a sync cycle. `snp sync` with no subcommand also runs sync. |
| `snp sync config` | View or change sync and auto-sync settings. |
| `snp sync retry` | Retry a failed auto-sync. |
| `snp sync clear-failure` | Clear auto-sync failure state while keeping pending work. |
| `snp sync discard-pending` | Discard pending auto-sync work. |
| `snp sync repair` | Repair sync control files. |
| `snp cron` | Print a cron entry for scheduled sync. |

Registration creates a new account. To use one account on multiple environments,
provision the same API key and server URL on each environment, with a unique
device ID. See [Syncing one account across environments](USER_GUIDE.md#syncing-one-account-across-environments)
and the [server setup guide](../snip-sync/README.md).

## Import, diagnostics, and data maintenance

| Command | What it does |
|---------|--------------|
| `snp import pet <FILE>` | Import a Pet-format TOML file. Use `--dry-run` to preview, `--merge` to add to an existing library, or `--replace` to replace it. |
| `snp doctor --pet-file <FILE>` | Check a Pet file before import. |
| `snp doctor --compatibility` | Check the local installation and environment. |
| `snp doctor --library <NAME_OR_PATH>` | Check a library file. |
| `snp doctor --sync` | Check sync configuration and health without starting a sync. |
| `snp doctor --check-shell <SHELL>` | Check generated shell integration syntax. |
| `snp validate` | Validate snippet data without changing it. |
| `snp status` | Show library and auto-sync status. |
| `snp backup` | Create a backup snapshot. Sync credentials are excluded by default. |
| `snp restore <BACKUP_DIR>` | Restore from a backup snapshot. |
| `snp repair` | Inspect and repair configuration or library data. |

`snp doctor` requires one mode. Run `snp doctor --help` for details and report
options. The equivalent maintenance commands are also available under `snp
data`, including `validate`, `status`, `backup`, `restore`, `repair`, `delete`,
and `organize`.

## Other commands

| Command | What it does |
|---------|--------------|
| `snp premade list`, `search`, `get`, `sync`, `update` | Browse and install premade libraries from a configured server. |
| `snp shell init <SHELL>` | Print shell integration for Bash, Zsh, or Fish. It installs no key bindings by default. |
| `snp completions <SHELL>` | Print shell completions. |
| `snp keybindings` | Print the TUI key reference. |
| `snp mcp serve` | Run the read-only stdio MCP server. |
| `snp mcp instructions <CLIENT>` | Print setup instructions for a supported MCP client. |
| `snp mcp install <CLIENT>` | Configure a supported MCP client. This may run the client's command-line tool. |
| `snp update` | Check for and install an update. |

The MCP server can list, search, and fetch snippets, but it never executes them.
See [MCP](MCP.md).

## See also

- [User Guide](USER_GUIDE.md)
- [Exit Codes](EXIT_CODES.md)
- [Command Contracts](COMMAND_CONTRACTS.md)
- [Library Scope](LIBRARY_SCOPE.md)
- [MCP](MCP.md)
- [Snippet Identity](IDENTITY_CONTRACT.md)

# snp Command Reference

Complete reference for every `snp` subcommand. This is the *what exists* map;
the *how to use it* walkthrough lives in [`USER_GUIDE.md`](USER_GUIDE.md), and the
behavioral contract (startup recovery, stdout/stderr split) lives in
[`COMMAND_CONTRACTS.md`](COMMAND_CONTRACTS.md).

Run `snp --help` or `snp <command> --help` for the authoritative flag list —
that output is generated from the same clap schema the binary uses.

## Exit codes

Every command maps failures onto this stable set. Full definitions in
[`EXIT_CODES.md`](EXIT_CODES.md).

| Code | Meaning |
|------|---------|
| 0 | success |
| 1 | general error |
| 2 | usage / argument error (clap) |
| 3 | not found |
| 4 | cancelled by the user |
| 5 | ambiguous match |
| 6 | validation failure |
| 7 | sync failure |
| 8 | execution failure |
| 9 | conflict / refused |
| 10 | unsafe repairs pending |

`snp run` propagates the executed snippet's exit code verbatim, so a failed
snippet can exit with a code other than 8 — see
[`EXIT_CODES.md`](EXIT_CODES.md).

## Everyday commands

| Command | Alias | What it does |
|---------|-------|--------------|
| `snp new` | `n` | Create a snippet. Takes the command as a positional argument, or `--command-stdin` to read it from a pipe. |
| `snp list` | `l` | List snippets. Never executes anything. `--json` / `--csv` for machine output. |
| `snp get` | — | Retrieve one snippet deterministically. No TUI, no execution. The scripting entry point. |
| `snp delete` | — | Tombstone one snippet deterministically. No TUI, no prompt. |
| `snp run` | `r` | Pick a snippet in the TUI and execute it through your shell. |
| `snp clip` | `c` | Pick a snippet in the TUI and copy it to the clipboard. |
| `snp search` | `s` | Fuzzy-search snippets in the TUI. |
| `snp select` | `sel` | Pick a snippet and print its command to stdout. Does not execute. |
| `snp edit` | `e` | Open the snippet file in `$EDITOR`. `--output` / `--output-stdin` / `--clear-output` set the snippet's output/notes field instead. |
| `snp version` | `v` | Print the version. |

## Snippet selection

`snp get` and `snp list` resolve a target differently — see
[`LIBRARY_SCOPE.md`](LIBRARY_SCOPE.md) for the scope model and
`src/selector.rs` for the resolver.

`snp get` accepts exactly one of:

| Flag | Matches |
|------|---------|
| `--id <ID>` | Exact snippet ID (case-sensitive). |
| `--description-exact <TEXT>` | Exact description (case-insensitive). |
| `--command-exact <TEXT>` | Exact command text (case-insensitive). |
| `--query <TEXT>` | Fuzzy match across description, command, and tags. |

`snp get` exits **3** when nothing matches and **5** when a query matches more
than one snippet.

`snp list` takes `--filter <TEXT>` for a fuzzy match over description,
command, and tags, plus `--library`, `--json`, `--csv`, `--search-output`,
`--sort`, and `--favorites-first`. It has no `--field`; use `snp get --field`
to pull a single field out of one snippet.

### The search contract

`snp get --query`, `snp list --filter`, and MCP `snippets_search` share one
definition of which snippet fields participate in a search, and one match
primitive (`selector::score_fuzzy_matches`). Two rules follow from that.

**An empty query is "no filter".** `snp get --query ""`, `snp list --filter ""`,
and MCP `snippets_search` with an empty `query` all return every non-deleted
snippet in scope, including snippets whose description, command, and tags are
all empty. An empty argument filters nothing.

**The order differs across libraries, by design.** `snp get --query` resolves
one library at a time and then aggregates, so across `--library all` its order
is library name, then description, then snippet id. `snp list --filter` and MCP
`snippets_search` merge the libraries first and then score the merged set, so
their order is global relevance. Within a single library the two orders are
the same.

`snp list --search-output` and MCP `search_output` opt the output/notes field
into the search. `snp get --query` has no such opt-in, so output and notes
never participate in a `get` match.

## Deleting a snippet

`snp delete` tombstones one snippet without a TUI and without a confirmation
prompt. It is the scripting counterpart of the TUI's `d` key and produces the
same persisted state.

| Flag | Matches |
|------|---------|
| `--id <ID>` | Exact snippet ID (case-sensitive). |
| `--description-exact <TEXT>` | Exact description (case-insensitive). |
| `--command-exact <TEXT>` | Exact command text (case-insensitive). |
| `--query <TEXT>` | Fuzzy match across description, command, and tags. |

Exactly one selector is required. `--library <NAME>` scopes the delete to one
library; `--library all` is refused, because a delete tombstones one snippet in
one library file. `--json` emits the machine-readable result.

| Outcome | Exit |
|---------|------|
| The snippet was deleted | 0 |
| The selector matched more than one snippet (nothing was deleted) | 5 |
| No snippet matched | 3 |
| The snippet was already deleted (nothing was changed) | 9 |

The delete is a tombstone, not a removal: the snippet keeps its identifier and
its `updated_at` advances past the live version, so the tombstone still syncs
and still wins over live content in a merge.

## Libraries

| Command | What it does |
|---------|--------------|
| `snp library list` | List every registered library, marking the primary. |
| `snp library create <NAME>` | Create a library. `all` is reserved and rejected. |
| `snp library delete <NAME>` | Delete a library. Prompts for confirmation; `--force` skips it (and is required in a non-interactive session). |
| `snp library set-primary <NAME>` | Change which library new snippets go to and which one `snp list` reads by default. |
| `snp library show [NAME]` | Show one library's details. |

A fresh install stores snippets in a single file, `snippets.toml`. The first
library-mode command copies it to `libraries/snippets.toml` and registers it as
the primary `snippets` library. `snp list --library all` concatenates every
library.

## Sync

| Command | What it does |
|---------|--------------|
| `snp sync run` | Run a sync cycle. Also the default when `snp sync` is given no subcommand. |
| `snp sync config` | View or update auto-sync policy. |
| `snp sync retry` | Retry a failed auto-sync now. |
| `snp sync clear-failure` | Clear failure state without discarding pending intent. |
| `snp sync discard-pending` | Discard pending sync intent. |
| `snp sync repair` | Repair sync control artifacts. |
| `snp register` | Register this environment with a sync server (`--server <URL>`, `--force` to re-register). Mints a new account and API key. |
| `snp cron` | Print a crontab entry that syncs every 15 minutes (`--interval` to change). Offers to copy it to the clipboard. |

## Agent / MCP integration

| Command | What it does |
|---------|--------------|
| `snp mcp serve` | Run the read-only newline-delimited stdio MCP server. |
| `snp mcp instructions <CLIENT>` | Print setup instructions for a supported MCP client. |
| `snp mcp install <CLIENT>` | Register snp-it with a supported MCP client. |

The MCP surface is read-only: it lists, searches, and fetches snippets, and
never executes anything. See [`MCP.md`](MCP.md).

## Import, backup, repair

| Command | What it does |
|---------|--------------|
| `snp import pet <FILE>` | Import a `pet` snippet file. `--dry-run` previews; `--merge` merges into an existing library; `--replace` replaces it. |
| `snp doctor --pet-file <FILE>` | Analyze a `pet` file for compatibility before importing. |
| `snp doctor --compatibility` | Audit the installed snp environment. |
| `snp doctor --library <NAME_OR_PATH>` | Check one library file for compatibility. |
| `snp doctor --sync` | Focused sync diagnostics. |
| `snp doctor --check-shell <SHELL>` | Check the generated shell integration for syntax errors. |
| `snp validate` | Validate snippet data. Read-only. |
| `snp status` | Show library and auto-sync status. Read-only. |
| `snp backup` | Create a secret-free backup snapshot. The snapshot is always a directory layout; the `--format` flag was removed. |
| `snp restore <BACKUP_DIR>` | Restore from a backup snapshot. |
| `snp repair` | Repair configuration and library files. |

`snp doctor` requires a mode — one of `--pet-file`, `--compatibility`,
`--sync`, `--check-shell`, or `--library`. Run it without one and it exits 1
with the list of valid modes.

`snp doctor --report json` emits the same report as JSON.

## Premade libraries

| Command | What it does |
|---------|--------------|
| `snp premade list` | List available premade libraries from the server. |
| `snp premade get <NAME>` | Download and install a premade library (`all` for every one). |
| `snp premade sync` | Download any missing premade libraries. |
| `snp premade search <QUERY>` | Search premade libraries. |
| `snp premade update <NAME>` | Show a diff and re-download. |

## Shell integration

| Command | What it does |
|---------|--------------|
| `snp shell init bash\|zsh\|fish` | Print the shell integration functions and widgets. Inspect before sourcing. |
| `snp completions <SHELL>` | Print shell completions. |
| `snp keybindings` | Print the TUI key bindings. |

The generated integration installs no key bindings by default — bind them
yourself, as shown in [`USER_GUIDE.md`](USER_GUIDE.md#binding-keys).

## `snp data` — advanced data maintenance

`snp data` is a mirror spelling of the maintenance commands. It is not a second
implementation: it routes to the same handlers with the same arguments and
the same exit codes, and `tests/cli_surface_compat.rs` asserts the two
spellings stay equivalent.

| Command | Equivalent |
|---------|------------|
| `snp data validate` | `snp validate` |
| `snp data backup` | `snp backup` |
| `snp data restore <BACKUP_DIR>` | `snp restore <BACKUP_DIR>` |
| `snp data repair` | `snp repair` |
| `snp data status` | `snp status` |
| `snp data delete` | `snp delete` |

## Self-update

| Command | What it does |
|---------|--------------|
| `snp update` | Check for and install an update using the current installation method. |

## See also

- [`USER_GUIDE.md`](USER_GUIDE.md) — task-oriented walkthrough
- [`EXIT_CODES.md`](EXIT_CODES.md) — exit-code contract
- [`COMMAND_CONTRACTS.md`](COMMAND_CONTRACTS.md) — per-command behavioral contract
- [`LIBRARY_SCOPE.md`](LIBRARY_SCOPE.md) — `--library` resolution
- [`MCP.md`](MCP.md) — MCP protocol and tools
- [`IDENTITY_CONTRACT.md`](IDENTITY_CONTRACT.md) — snippet ID rules

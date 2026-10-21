# Command Contracts

> **Evergreen contract.** Single source of truth for every CLI command's
> behavioral contract. Exit codes are defined in [`EXIT_CODES.md`](EXIT_CODES.md)
> and implemented in `src/outcome.rs`.
> **Note:** Line numbers are approximate and may drift as the codebase evolves.

---

## Contract Table

| Command | Interactive | Reads stdin | Writes stdout | Writes stderr | May prompt | May mutate | May clipboard | May execute shell | May network | Machine-output modes | Exit categories |
|---------|------------|-------------|---------------|---------------|------------|------------|---------------|-------------------|-------------|---------------------|-----------------|
| `new` | No (unless `--editor`) | Yes (`--command-stdin`) | Yes | Yes | Yes (`--tags`) | Yes | No | Yes (`--editor`) | No | None | 0, 1 |
| `list` | No | No | Yes | Yes | No | No | No | No | No | `--json`, `--csv` | 0, 1 |
| `run` | Yes (TUI) or No (`--id`) | No | No | Yes | Yes | Yes (execution) | No | Yes | No | None | 0, 1, 3, 5, 8 |
| `clip` | Yes (TUI) or No (`--id`) | No | No | Yes | Yes | Yes (clipboard) | Yes | No | No | None | 0, 1, 3, 5 |
| `search` | Yes (TUI) | No | Yes | Yes | Yes | No | No | No | No | None | 0, 1 |
| `select` | Yes (TUI) | No | Yes | Yes | Yes | No | No | No | No | `--raw`, `--expanded`, `--output-file` | 0, 1, 4 |
| `edit` | Yes (`$EDITOR`) or No (`--output`) | Yes (`--output-stdin`) | No | Yes | Yes (`$EDITOR`) | Yes | No | Yes (`$EDITOR`) | No | None | 0, 1, 3, 5 |
| `get` | No | No | Yes | Yes | No | No | No | No | No | `--json`, `--raw`, `--field` | 0, 1, 3, 5 |
| `status` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1 |
| `validate` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1, 6 |
| `doctor` | No | No | Yes | Yes | No | No | No | Yes (diagnostic probes) | No | `--report json` | 0, 1, 6 |
| `backup` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1 |
| `restore` | No | No | Yes | Yes | No | Yes | No | No | No | `--json` | 0, 1 |
| `repair` | No | No | Yes | Yes | No | Yes (`--apply`) | No | No | No | `--json` | 0, 1, 10 |
| `import` | No | No | Yes | Yes | No | Yes | No | No | No | `--report json` | 0, 1 |
| `sync` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1, 6, 9 |
| `register` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1 |
| `cron` | No | No | Yes | Yes | Yes | Yes (clipboard copy) | Yes (on confirmation) | No | No | None | 0, 1 |
| `library` | No | No | Yes | Yes | No | Yes | No | No | No | None | 0, 1 |
| `premade` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1 |
| `shell` | No | No | Yes | Yes | No | No | No | No | No | None | 0, 1 |
| `completions` | No | No | Yes | No | No | No | No | No | No | shell completion | 0, 1 |
| `mcp` | No (stdio protocol) | No | Yes (protocol only) | Yes (diagnostics) | No | Yes (`mcp install`) | No | Yes (`mcp install`) | No | JSON-RPC 2.0 over stdio | 0, 1 |
| `update` | No | No | Yes | Yes | No | Yes (binary replace) | No | Yes (self-replace) | Yes | `--dry-run` | 0, 1 |
| `version` | No | No | Yes | No | No | No | No | No | No | None | 0 |
| `keybindings` | No | No | Yes | No | No | No | No | No | No | None | 0 |
| `data` | — | — | — | — | — | — | — | — | — | — | — |

`snp data <subcommand>` is a **mirror spelling** of the top-level commands
(`validate`, `status`, `backup`, `restore`, `repair`); it has no independent
contract. `auto-sync-worker` and `__self-replace` are internal/replace
subprocesses and are not user-facing contracts.

`snp mcp serve` is the only stdio-protocol command: **stdout carries protocol
messages only**, all diagnostics go to stderr, and it is read-only and
non-executing. `snp mcp install` is the exception inside the `mcp` group: it
spawns the MCP client's own CLI to register this binary and therefore mutates
that client's configuration. See [`MCP.md`](MCP.md).

## Column Definitions

- **Interactive**: Command renders a TUI (crossterm raw mode) or opens an editor. No = fully non-interactive.
- **Reads stdin**: Command consumes *data* bytes from stdin (e.g., `--command-stdin`, `--output-stdin`). TUI keystroke input is not counted here.
- **Writes stdout**: Command writes human-readable or machine-readable data to stdout via `println!`.
- **Writes stderr**: Command writes errors, progress, or diagnostics to stderr via `eprintln!`.
- **May prompt**: Command may prompt the user for input (interactive TUI, `$EDITOR`, `--tags` prompt, conflict resolution).
- **May mutate**: Command writes to the local snippet library, clipboard, or filesystem.
- **May clipboard**: Command writes to the system clipboard.
- **May execute shell**: Command spawns a child process — snippet execution, an editor, the MCP client's own CLI, or the self-replace helper.
- **May network**: Command makes gRPC or HTTP requests to a remote server.
- **Machine-output modes**: Flags that produce structured or script-consumable output (JSON, CSV, raw command text, a file) for scripting.
- **Exit categories**: See Exit Code Legend below.

## Exit Code Legend

| Code | Name | Meaning |
|------|------|---------|
| 0 | Success | Operation completed successfully |
| 1 | General error | Unclassified operational failure |
| 2 | `USAGE_ERROR` | CLI usage or argument error (Clap-controlled). Validation findings use **6**, not 2. |
| 3 | Not found | Requested resource does not exist |
| 4 | Cancelled | User cancelled interactive action |
| 5 | Ambiguous match | Multiple candidates, unique match expected |
| 6 | Validation failure | Diagnostic finding or invalid data (see `docs/EXIT_CODES.md`) |
| 7 | Sync failure | Reserved but currently unreachable — sync failures surface as exit 1 |
| 8 | Execution failure | Snippet execution (child process) failed |
| 9 | Conflict/refused | Destructive action refused or generation changed |
| 10 | Unsafe repairs | Every finding awaits an operator decision and was not applied |

Every command can also exit **2** on a clap usage/argument error; the table's
"Exit categories" column lists only the command-level outcomes.

Also see `docs/EXIT_CODES.md` for the authoritative `CliOutcome` → exit-code mapping.

## Startup Recovery Classification

Commands are classified by `command_behavior()` (`src/main.rs:924`) into a
`StartupRecoveryPolicy` **and** a `StartupServices` level, both in one `match`
so the two cannot drift. `snp data <subcommand>` is a mirror of the top-level
spellings and is classified in the same table.

| Policy | Commands |
|--------|----------|
| `SuppressReadOnly` (also `StartupServices::Minimal`) | `version`, `list`, `select`, `status`, `mcp`, `get`, `validate`, `backup`, `library list`, `library show` |
| `SuppressReadOnly` (dry-run variants only) | `restore --mode dry-run`, `repair --dry-run`, `import pet --dry-run` — plus `snp data restore --mode dry-run` and `snp data repair --dry-run` (there is no `snp data import`) |
| `Allow` (also `StartupServices::Logging`) | `new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, `library create`, `library delete`, `library set-primary`; also **no subcommand** (default TUI) |
| `SuppressExplicitSync` | `sync`, `cron`, `register` |
| `SuppressInternal` | `auto-sync-worker` (internal subprocess) |
| `SuppressConfiguration` | `update`, `__self-replace`, `doctor`, `completions`, `shell`, `keybindings` |

Two easy-to-miss entries: **`run`, `clip`, and `search` are `Allow`**, because
they can record usage (`run`, `clip`) and schedule auto-sync; and **`mcp` is
`SuppressReadOnly`**, because the MCP surface never mutates the snippet library
— only `snp mcp install` touches an MCP client's own configuration. Extending
either list means editing `command_behavior()` — do not infer the policy from the
command's name.

Dry-run variants are classified by their **flag**, not by their command category. `command_behavior()` matches the dry-run variants first and classifies them `SuppressReadOnly` with `Minimal` services — `restore --mode dry-run`, `repair --dry-run`, and `import pet --dry-run`, plus the `data` spellings of `restore` and `repair`. The mutating spellings of those same commands are `Allow`.

## Notes

- **TUI commands** (`run`, `clip`, `search`, `select`) render directly to the terminal via crossterm raw mode — they bypass stdout/stderr for the interactive portion.
- **`list` default format** writes colored table output to stdout (includes ANSI escapes — not pipe-safe without `--json` or `--csv`).
- **`edit --output`** is non-interactive; it writes the output field to **stderr**, not stdout (`src/commands/edit_cmd.rs:218,220,284,286` all use `eprintln!`). This matches the `edit` row's "Writes stdout: No". The `--output-stdin` variant reads from stdin.
- **`--json`** and **`--csv`** flags conflict with each other (enforced by clap).
- **`ExecutionFailed`** exit code: if the child process had a valid exit code, that code is propagated; otherwise exit code 8 is used.
- **`PersistenceFailed`** maps to exit code 1 (general error) — no dedicated public exit code for persistence failures.
- **Exit 4 (cancelled)** comes only from `snp select`. `run`, `clip`, and `search` treat a cancelled TUI selection as normal completion (exit 0).
- **Exit 5 (ambiguous)** comes only from the exact-selector paths: `run`, `clip`, and `edit` with `--id` / `--description-exact` / `--command-exact`, and from `snp get`. The interactive TUI resolves ambiguity in-terminal instead.
- **Exit 6 (validation failure)** comes from `snp validate`, `snp doctor`, and `snp sync discard-pending` (corrupt pending state). Exit **9 (conflict/refused)** comes from `snp sync discard-pending`.
- **Exit 7 (sync failure)** is reserved but currently unreachable — no command constructs `CliOutcome::SyncFailed`; sync failures surface as exit 1.

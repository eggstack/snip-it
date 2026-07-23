# Command Contracts

> Phase 10 — Workstream A (corrective closure)
> Single source of truth for every CLI command's behavioral contract.

---

## Contract Table

| Command | Interactive | Reads stdin | Writes stdout | Writes stderr | May prompt | May mutate | May clipboard | May execute shell | May network | Machine-output modes | Exit categories |
|---------|------------|-------------|---------------|---------------|------------|------------|---------------|-------------------|-------------|---------------------|-----------------|
| `new` | No (unless `--editor`) | Yes (`--command-stdin`) | No | Yes | Yes (`--tags`) | Yes | No | No | No | None | 0, 1 |
| `list` | No | No | Yes | Yes | No | No | No | No | No | `--json`, `--csv` | 0, 1 |
| `run` | Yes (TUI) or No (`--id`) | No | No | Yes | Yes | Yes (execution) | No | Yes | No | None | 0, 1, 3, 4, 5, 8 |
| `clip` | Yes (TUI) or No (`--id`) | No | No | Yes | Yes | Yes (clipboard) | Yes | No | No | None | 0, 1, 3, 4, 5 |
| `search` | Yes (TUI) | No | No | Yes | Yes | No | No | No | No | None | 0, 1, 4, 5 |
| `select` | Yes (TUI) | No | Yes | Yes | Yes | No | No | No | No | `--raw`, `--expanded`, `--output-file` | 0, 1, 4, 5 |
| `edit` | Yes (`$EDITOR`) or No (`--output`) | Yes (`--output-stdin`) | No | Yes | Yes (`$EDITOR`) | Yes | No | No | No | None | 0, 1 |
| `get` | No | No | Yes | Yes | No | No | No | No | No | `--json`, `--raw`, `--field` | 0, 1, 3, 4, 5 |
| `status` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1 |
| `validate` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1, 2 |
| `doctor` | No | No | Yes | Yes | No | No | No | No | No | `--report json` | 0, 1, 2 |
| `backup` | No | No | Yes | Yes | No | No | No | No | No | `--json` | 0, 1 |
| `restore` | No | No | Yes | Yes | No | Yes | No | No | No | `--json` | 0, 1, 6, 9 |
| `repair` | No | No | Yes | Yes | No | Yes (`--apply`) | No | No | No | `--json` | 0, 1 |
| `import` | No | No | Yes | Yes | No | Yes | No | No | No | `--report json` | 0, 1 |
| `sync` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1, 7 |
| `register` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1, 7 |
| `cron` | No | No | Yes | Yes | No | Yes | No | No | No | None | 0, 1 |
| `library` | No | No | Yes | Yes | No | Yes | No | No | No | None | 0, 1 |
| `premade` | No | No | Yes | Yes | No | Yes | No | No | Yes | None | 0, 1, 7 |
| `shell` | No | No | Yes | Yes | No | No | No | No | No | None | 0, 1 |
| `completions` | No | No | Yes | No | No | No | No | No | No | shell completion | 0, 1 |

## Column Definitions

- **Interactive**: Command renders a TUI (crossterm raw mode) or opens an editor. No = fully non-interactive.
- **Reads stdin**: Command consumes bytes from stdin (e.g., `--command-stdin`, `--output-stdin`).
- **Writes stdout**: Command writes human-readable or machine-readable data to stdout via `println!`.
- **Writes stderr**: Command writes errors, progress, or diagnostics to stderr via `eprintln!`.
- **May prompt**: Command may prompt the user for input (interactive TUI, `$EDITOR`, `--tags` prompt, conflict resolution).
- **May mutate**: Command writes to the local snippet library, clipboard, or filesystem.
- **May clipboard**: Command writes to the system clipboard.
- **May execute shell**: Command spawns a child process to execute snippet content.
- **May network**: Command makes gRPC or HTTP requests to a remote server.
- **Machine-output modes**: Flags that produce structured output (JSON, CSV) for scripting.
- **Exit categories**: See Exit Code Legend below.

## Exit Code Legend

| Code | Name | Meaning |
|------|------|---------|
| 0 | Success | Operation completed successfully |
| 1 | General error | Unclassified operational failure |
| 2 | Validation error | CLI usage/argument error or diagnostic finding |
| 3 | Not found | Requested resource does not exist |
| 4 | Cancelled | User cancelled interactive action |
| 5 | Ambiguous match | Multiple candidates, unique match expected |
| 6 | Persistence failure | Atomic write or local persistence error |
| 7 | Sync failure | Synchronization with remote server failed |
| 8 | Execution failure | Snippet execution (child process) failed |
| 9 | Conflict/refused | Destructive action refused or generation changed |

## Startup Recovery Classification

Commands are classified by `StartupRecoveryPolicy` at startup. Only mutation commands (`new`, `edit`, `import`, `delete`, `library create/delete`) trigger auto-sync recovery. Read-only commands (`list`, `search`, `get`, `status`, `validate`, `backup`, `select`) suppress recovery. Explicit sync commands (`sync`, `cron`, `register`) and internal subprocesses also suppress recovery.

Dry-run commands are classified based on their command category, not the dry-run flag. `restore` and `import` are classified as `Allow` (mutation) because the command itself is a mutation command; dry-run mode prevents local mutation but the recovery policy applies to the command class, not the mode.

## Notes

- **TUI commands** (`run`, `clip`, `search`, `select`) render directly to the terminal via crossterm raw mode — they bypass stdout/stderr for the interactive portion.
- **`list` default format** writes colored table output to stdout (includes ANSI escapes — not pipe-safe without `--json` or `--csv`).
- **`edit --output`** is non-interactive; it writes the output field to stdout. The `--output-stdin` variant reads from stdin.
- **`--json`** and **`--csv`** flags conflict with each other (enforced by clap).
- **`ExecutionFailed`** exit code: if the child process had a valid exit code, that code is propagated; otherwise exit code 8 is used.
- **`PersistenceFailed`** maps to exit code 1 (general error) — no dedicated public exit code for persistence failures.

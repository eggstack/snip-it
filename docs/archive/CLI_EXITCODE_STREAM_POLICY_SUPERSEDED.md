# CLI Exit Code and Stream Policy — Superseded Block

> **ARCHIVED — historical proposal. Not a current contract.**
> Archived 2026-10-21. This is the retired pre-`CliOutcome` section that used to
> sit at the bottom of `docs/CLI_EXITCODE_STREAM_POLICY.md`. It is kept only for
> design rationale: the 2–6 exit-code table below was **never shipped**, and the
> stream contract it proposes (human-readable output moving to stderr, plus a
> transitional `--stdout` flag) **is still unimplemented**.
>
> **The live contract is `docs/EXIT_CODES.md`** (codes 0–10 via `CliOutcome` in
> `src/outcome.rs`) together with the "Current Behavior" section of
> `docs/CLI_EXITCODE_STREAM_POLICY.md`. Do not read the tables below as the
> current stream contract.

## Exit Code Contract (IMPLEMENTED) vs Stream Contract (ASPIRATIONAL)

> **Status**: The exit-code portion of this contract **is implemented** — codes
> 0–10 exist in `src/outcome.rs` (superseding the 2-6 proposal below; see the
> Current Behavior table). The **stream contract is still aspirational** and has
> not been implemented: human-readable output still goes to stdout, no
> `--stdout` transitional flag exists. The stream sections below describe a
> possible future direction only.

### Historical Proposal: Exit Codes (SUPERSEDED)

The original Release 1B proposal below was narrower than what shipped.
It is retained for design rationale only:

| Code | Name | Meaning | Examples |
|------|------|---------|----------|
| 0 | `SUCCESS` | Operation completed successfully | Snippet executed, clipboard copied, list printed |
| 1 | `ERROR` | General/unclassified error | Default for unclassified failures |
| 2 | `USAGE` | Invalid arguments or missing required input | Bad CLI flags |
| 3 | `NOT_FOUND` | Requested resource does not exist | Snippet/library not found |
| 4 | `CANCELLED` | User cancelled TUI interaction | `q`/`Esc`/Ctrl-C in selector (`snp select`) |
| 5 | `IO` | Filesystem or clipboard failure | Cannot write file |
| 6 | `PARSE` | Configuration or data format error | Malformed TOML |

The shipped mapping instead distinguishes `AMBIGUOUS` (5), `VALIDATION_FAILED`
(6), `SYNC_FAILED` (7), `EXECUTION_FAILED` (8), `CONFLICT_OR_REFUSED` (9), and
`UNSAFE_REPAIRS` (10) — see `docs/EXIT_CODES.md`.

**Migration path**: New exit codes are additive. Existing scripts checking
`exit != 0` will continue to work.

**Note**: `run_snippet_selection()` returns `SelectionOutcome` (Selected or
Cancelled). For existing commands (`run`, `clip`, `search`), cancellation is
treated as normal completion (exit 0). For `snp select`, cancellation
maps to exit 4 via `CliOutcome::Cancelled`, which is returned to the CLI
boundary in `main.rs`. Ctrl+C in the TUI (normal mode) also maps to
`SelectionOutcome::Cancelled` → exit 4 for `select`.

### Stream Contract

| Stream | Content | Examples |
|--------|---------|----------|
| **stdout** | Machine-readable output only | JSON, CSV, selected command text, shell completions |
| **stderr** | Human-readable output | Tables, progress, errors, prompts, keybinding docs |
| **terminal** (raw) | TUI rendering | Snippet selector, variable prompt, theme picker |

**Rules**:

1. **stdout** must never contain ANSI escape sequences, prompts, or progress
   messages. It is safe for piping and redirection.

2. **stderr** is for anything a human reads on the terminal: colored tables,
   status messages, error messages, interactive prompts.

3. **TUI commands** continue to render directly to the terminal. When a TUI
   command selects a snippet and needs to emit machine-readable output, it
   goes to stdout (e.g., `snp select` prints the command to stdout).

4. **Error messages** always go to stderr, prefixed with `error:` (current
   behavior in `main.rs:1063`).

### Command-by-Command Stream Changes

| Command | Current stdout | Proposed stdout | Current stderr | Proposed stderr |
|---------|---------------|-----------------|----------------|-----------------|
| `list` (default) | Colored table | *Move to stderr* | Nothing | Table |
| `list --json` | JSON | JSON (no change) | Nothing | Nothing |
| `list --csv` | CSV | CSV (no change) | Nothing | Nothing |
| `search` | Snippet details | Snippet details | Nothing | Nothing |
| `select` | Command string | Command string (no change) | Nothing | Nothing |
| `keybindings` | Keybinding docs | *Move to stderr* | Nothing | Keybinding docs |
| `cron` | Crontab entry | *Move to stderr* | Nothing | Crontab entry |
| `new` prompts | "Command> " | *Move to stderr* | Nothing | "Command> " |
| `new` success | "Snippet added" | *Move to stderr* | Nothing | "Snippet added" |
| `version` | Version string | Version string (no change) | Nothing | Nothing |
| `completions` | Completions | Completions (no change) | Nothing | Nothing |
| `sync` progress | Status messages | *Move to stderr* | Errors | Errors |
| `register` | Success + keys | *Move to stderr* | Errors | Errors |
| `library` subcmds | Metadata | *Move to stderr* | Errors | Errors |
| `premade` subcmds | Results | *Move to stderr* | "Not enabled" | "Not enabled" |

**Key changes**:
- `list` default format moves to stderr (colored table is human-readable)
- `keybindings`, `cron`, `new`, `sync`, `register`, `library`, `premade`
  status output moves to stderr
- `--json` and `--csv` remain on stdout (machine-readable)
- `version` and `completions` remain on stdout (machine-readable / standard)

### For `snp select` (Release 1B — implemented)

A `snp select` primitive provides non-TUI snippet selection for scripting:

| Scenario | stdout | stderr | Exit Code |
|----------|--------|--------|-----------|
| Selection to stdout | exact command | empty except tracing | 0 |
| Selection to output file | empty | empty except tracing | 0 |
| User cancellation (`q`/`Esc`/Ctrl-C) | empty | empty | 4 |
| Variable prompt cancelled | empty | empty | 4 |
| `SnipError` (all variants) | empty | `error: ...` | 1 |

**Usage**:

```bash
# Run selected snippet
command=$(snp select -f "git") && eval "$command"

# Check for cancellation
if ! snp select -f "deploy" > /tmp/cmd.sh; then
    case $? in
        4) echo "Cancelled" ;;
        *) echo "Error" ;;
    esac
fi
```

### Backward Compatibility

- **Exit code 0/1**: No change. All existing scripts checking `exit == 0` or
  `exit != 0` continue to work.
- **New exit codes (2-10)**: Additive and stable (see `docs/EXIT_CODES.md`).
  Only scripts that explicitly check for these codes will be affected.
- **Stream moves** (aspirational): Moving human-readable output from stdout to
  stderr would break scripts that `grep` or parse stdout from `snp list`,
  `snp keybindings`, etc. This is a **breaking change** for those scripts —
  document in release notes and provide a `--stdout` flag during transition.
- **`--stdout` flag** (transitional, not yet implemented): If human-readable
  output ever moves to stderr, a `--stdout` flag will force it back to stdout
  for backward compatibility. Deprecated after two releases.

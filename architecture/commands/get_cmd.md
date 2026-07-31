# get_cmd — Deterministic Snippet Retrieval

[← Back to Overview](../overview.md)

## Purpose

`get` retrieves a snippet by ID, exact description, exact command, or fuzzy query without opening a TUI, executing, or touching the clipboard. It is the deterministic, machine-friendly snippet retrieval command.

**File**: `src/commands/get_cmd.rs`

## Selectors

At least one selector must be provided:

| Flag | Behavior |
|------|----------|
| `--id` | Match by exact snippet UUID |
| `--description-exact` | Match by exact description (case-insensitive) |
| `--command-exact` | Match by exact command text (case-insensitive) |
| `--query` | Fuzzy query match |

## Output Modes

- `--field command|description|id|tags` — emit a single field
- `--raw` — output raw stored bytes (no variable expansion, no trailing newline)
- `--expanded` — output with variables substituted
- `--json` — JSON output with schema version, library info, and expansion
- `--vars key=value ...` — provide variable assignments for `--expanded` mode

Output modes are mutually exclusive: `--raw` + `--expanded`, `--json` + `--raw`/`--expanded`, or `--field` + `--json`/`--raw`/`--expanded` all return an error.

## Resolution

Uses `selector::resolve_selector()` with a configurable `--resolution` policy (`unique`, `first`, `all`). Fails on ambiguity unless `--resolution first` is specified.

## Data Flow

```
get run() → resolve_selector() → (optionally) expand_command() → emit field/JSON/raw
```

No TUI, no execution, no clipboard. Pure data retrieval.

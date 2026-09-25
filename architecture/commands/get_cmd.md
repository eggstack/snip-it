# get_cmd — Deterministic Non-TUI Snippet Retrieval

[← Back to Overview](../overview.md)

## Overview

`src/commands/get_cmd.rs` (433 lines) retrieves snippets
deterministically — no TUI, no execution, no clipboard. It resolves
through the side-effect-free `resolve_selector_readonly` (legacy
single-file checkouts read in place, never migrated) and renders one
of four output modes. Shares `selector::searchable_text` parity with
`list --filter` and MCP `snippets_search`.

## CLI surface

`GetArgs` (`get_cmd.rs:16`), `snp get`:

| Flag | Meaning |
|------|---------|
| `--id` | Exact UUID (case-sensitive; conflicts with other selectors) |
| `--description-exact` | Exact description (case-insensitive) |
| `--command-exact` | Exact command (case-insensitive) |
| `-q/--query` | Fuzzy query |
| `-l/--library` | Scope: name or `all` |
| `--field <command\|description\|id\|tags>` | Single-field output (conflicts with json/raw/expanded) |
| `--raw` | Stored bytes, no expansion, no trailing newline (conflicts with `--expanded`) |
| `--expanded` | Non-interactive expansion via defaults/`--var` |
| `--json` | Pretty `GetJsonOutput` (conflicts with raw/expanded/field) |
| `--resolution <unique\|first\|all>` | Multi-match policy (default `Unique`) |
| `--var KEY=VALUE` | Repeatable explicit assignments |

At least one of the four selectors is required.

## Flow / steps

`run()` (`get_cmd.rs:96`):

1. Parse `--var` pairs into `VariableAssignments` (repeatable).
2. Validate selector presence and conflicting output modes (four
   explicit guards, `:119-144`).
3. Build the selector via `exact_selector(library, None, None, None)`
   (single home for `"all"` + case-sensitivity rules), apply
   `resolution`, then re-apply the chosen targeting field.
4. `resolve_selector_readonly(&selector)` → `NotFound → NotFound`;
   `Ambiguous(ids)` → stderr list + `Ambiguous` (JSON mode stays
   silent); `One(m)` / `Many` → `output_match` each, then `Success`.
5. `output_match` (`:198`): JSON → `GetJsonOutput{schema:1, id,
   description, command, expanded, tags, library, library_id}`;
   field/raw/expanded/default per the mode table. `raw` and `field`
   write bytes with no trailing newline; others `println!`.
6. `expand_without_prompt` (`:279`): explicit assignments win, then
   `<name=default>` defaults and choice fallbacks; unassigned required
   `<name>` tokens stay verbatim (never a bare-name substitution);
   choice tokens are located in the *original* command to avoid
   re-matching substituted text.

## Mutation vs read-only

Strictly read-only: `resolve_selector_readonly` never migrates legacy
state or creates files; no gate, no lock, no save.

## Auto-sync trigger

None. No `notify_mutation`, no runtime, no explicit sync. Retrieval is
invisible to auto-sync.

## Error / exit mapping

- `NotFound` → exit 3; `Ambiguous` → exit 5; success → exit 0
  (`CliOutcome`, `docs/EXIT_CODES.md`).
- Usage errors (no selector, conflicting modes, bad `--var`) →
  `runtime_error` (exit 1/2 family).
- JSON serialization failure → `runtime_error`.

## Key invariants

- Search parity: description + command always searchable, tags by
  default, output/notes only with opt-in (`list --search-output`,
  MCP `search_output`); folders/favorite/sync metadata never searchable.
- ID match is case-sensitive; description/command are case-insensitive
  — shared with run/clip/edit/MCP via `exact_selector`.
- `--raw`/`--field` are byte-exact for scripting; default strips escape
  sequences (`\<`/`\>`) for display.
- Non-interactive expansion never prompts and never invents values.

## File / line references

- `GetArgs`: `src/commands/get_cmd.rs:16`; `GetField`: `:55`
- `GetJsonOutput`: `:68`; `run`: `:96`; `output_match`: `:198`
- `expand_without_prompt`: `:279`; tests: `:332-433`
- Selector: `src/selector.rs` (`exact_selector`, `resolve_selector_readonly`)

# get_cmd — Deterministic Non-TUI Snippet Retrieval

[← Back to Overview](../overview.md)

## Overview

`src/commands/get_cmd.rs` (433 lines) retrieves snippets
deterministically — no TUI, no execution, no clipboard. It resolves
through the side-effect-free `resolve_selector_readonly` (legacy
single-file checkouts read in place, never migrated) and renders one
of four output modes. Shares `selector::searchable_text` parity with
`list --filter` and MCP `snippets_search`.

This is the machine-facing read path: it is the recommended spelling for
scripts and CI, where the TUI is unavailable (see the tty guard in
`src/ui/mod.rs:533-540`).

## CLI surface

`GetArgs` (`get_cmd.rs:16`), `snp get`:

| Flag | Type | Default | Meaning |
|------|------|---------|---------|
| `--id` | `Option<String>` | – | Exact snippet ID (case-sensitive; conflicts with the other three selectors) |
| `--description-exact` | `Option<String>` | – | Exact description (case-insensitive) |
| `--command-exact` | `Option<String>` | – | Exact command text (case-insensitive) |
| `-q/--query` | `Option<String>` | – | Fuzzy query |
| `-l/--library` | `Option<String>` | – | Scope: name or `all` |
| `--field` | `Option<GetField>` | – | Single-field output (`command`/`description`/`id`/`tags`; conflicts with json/raw/expanded — runtime guard) |
| `--raw` | bool | `false` | Stored bytes, no expansion, no trailing newline (conflicts with `--expanded`) |
| `--expanded` | bool | `false` | Non-interactive expansion via defaults/`--var` (conflicts with `--raw`) |
| `--json` | bool | `false` | Pretty `GetJsonOutput` (conflicts with raw/expanded/field — runtime guard) |
| `--resolution` | `ResolutionPolicy` | `Unique` | Multi-match policy (`unique`/`first`/`all`) |
| `--var KEY=VALUE` | `Option<Vec<String>>` | – | Repeatable explicit assignments |

At least one of the four selectors is required (`get_cmd.rs:119-124`).

`GetField` is a `clap::ValueEnum` with `kebab-case` renames
(`get_cmd.rs:53-64`). `GetJsonOutput` (`get_cmd.rs:68`) is
`{schema, id, description, command, expanded, tags, library, library_id}`
with `schema: 1`.

## Flow / steps

`run()` (`get_cmd.rs:96`):

1. Parse `--var` pairs into `VariableAssignments` (repeatable;
   `get_cmd.rs:109-117`). A malformed pair is an error.
2. Validate selector presence and conflicting output modes (four
   explicit guards, `get_cmd.rs:119-144`).
3. Build the selector via `exact_selector(library, None, None, None)`
   (`get_cmd.rs:149`) — the single home for the `"all"` mapping and the
   case-sensitivity rules — apply `resolution` (`get_cmd.rs:151`), then
   re-apply the chosen targeting field with
   `with_id` / `with_description_exact` / `with_command_exact` / `with_query`
   (`get_cmd.rs:154-165`).
4. `resolve_selector_readonly(&selector)` (`get_cmd.rs:168`) → `NotFound →
   NotFound`; `Ambiguous(ids)` → stderr list + `Ambiguous` (JSON mode stays
   silent); `One(m)` / `Many` → `output_match` each, then `Success`
   (`get_cmd.rs:170-195`).
5. `output_match` (`get_cmd.rs:198`): JSON → `GetJsonOutput` with
   `expanded` populated only when the command has variables
   (`get_cmd.rs:206-230`); field/raw/expanded/default per the mode table
   (`get_cmd.rs:232-252`). `raw` and `field` write bytes with no trailing
   newline (`get_cmd.rs:254-262`); others `println!` (`get_cmd.rs:264`).
6. `expand_without_prompt` (`get_cmd.rs:279`): explicit assignments win,
   then `<name=default>` defaults and choice fallbacks; unassigned required
   `<name>` tokens stay verbatim (never a bare-name substitution,
   `get_cmd.rs:288-295`); choice tokens are located in the *original*
   command to avoid re-matching substituted text (`get_cmd.rs:317-325`).

## Read-only guarantee

`resolve_selector_readonly` (`src/selector.rs:623`) consumes
`library::readonly_library_sources` (`src/selector.rs:629`), which performs
no migration and creates no files or directories. It never calls
`ensure_library_mode()` — contrast `resolve_selector`
(`src/selector.rs:681`), which does (`src/selector.rs:683`) and is used
only by the exact-selector `run`/`clip`/`edit` paths.

`get` never calls `gate_mutation_on_interrupted_transactions()`; it takes
no lock and never saves.

`src/main.rs:896` classifies `Commands::Get(_)` under
`StartupRecoveryPolicy::SuppressReadOnly` + `StartupServices::Minimal`, so
startup recovery is suppressed and no network service is started.

## Library scope

`--library all` is supported by `get`. The literal `"all"` is mapped by
`LibraryScope::from_filter_arg` (`src/selector.rs:65`) inside
`exact_selector` (`src/selector.rs:586`) — the keyword owner for the
selector paths — and `resolve_selector_readonly` then hands the same
literal to `readonly_library_sources`
(`src/library/manager.rs:278`), so it means the same thing as on
`snp list`, exact-mode `run`/`clip`/`edit`, and MCP.

Resolution semantics of `resolve_selector_readonly`
(`src/selector.rs:623-674`):

| `--library` | Result |
|-------------|--------|
| absent (`Primary`) | primary library; if no visible source, `runtime_error("No primary library")` (`src/selector.rs:633-638`) |
| `all` (`AllLibraries`) | every visible library; with none visible → `NotFound` (`src/selector.rs:639`) |
| a name (`Named`) | that named library; unknown name → `library_not_found`; no visible libraries → `NotFound` (`src/selector.rs:640-644`) |

With more than one source, matches are collected per library under
`ResolutionPolicy::All` and then aggregated with the caller's real policy
(`src/selector.rs:654-673`).

A library actually *named* `all` can no longer be created:
`validate_library_name` rejects the reserved name
(`src/library/model.rs:213-223`).

## Mutation vs read-only

Strictly read-only: `resolve_selector_readonly` never migrates legacy
state or creates files; no gate, no lock, no save. Retrieval is invisible
to auto-sync.

## Auto-sync trigger

None. No `notify_mutation`, no runtime, no explicit sync.

## Error / exit mapping

- `NotFound` → exit 3; `Ambiguous` → exit 5; success → exit 0
  (`CliOutcome`, `src/outcome.rs:50-66`).
- Usage errors (no selector, conflicting modes, bad `--var`) →
  `runtime_error` (exit 1/2 family).
- JSON serialization failure → `runtime_error` (`get_cmd.rs:224-227`).
- `src/main.rs:842-857` returns the `CliOutcome` directly.

## Key invariants

- Search parity: description + command always searchable, tags by
  default, output/notes only with opt-in (`list --search-output`,
  MCP `search_output`); folders/favorite/sync metadata never searchable.
- ID match is case-sensitive; description/command are case-insensitive
  — shared with run/clip/edit/MCP via `exact_selector`.
- `--raw` is byte-exact for scripting (no trailing newline); `--field`
  suppresses the trailing newline too, but `--field command` without `--raw`
  still strips escape sequences (`\<`/`\>`) for display. The default mode
  strips them too.
- Non-interactive expansion never prompts and never invents values.
- Never route this command through `resolve_selector` — the mutating
  resolver reintroduces recovery side effects.

## Gotchas

- The ambiguity list is suppressed in `--json` mode
  (`get_cmd.rs:173`), so a JSON consumer sees `Ambiguous` only through the
  exit code, not the payload.
- `--field` and `--raw` suppress the trailing newline; `--json` and
  `--expanded` do not. Scripts that compare bytes must pick the right mode.
- `--expanded` and `--json` both expand with the *non-interactive*
  `expand_without_prompt` routine (`get_cmd.rs:279`), never
  `expand_command` (the interactive path in `src/utils/variables.rs:644`);
  required variables with no assignment stay verbatim in the `--expanded`
  output but would prompt in the interactive path.

## Key Files

- `src/commands/get_cmd.rs` — the command: args, validation, selector
  construction, and the four output modes.
- `src/selector.rs` — `exact_selector`, `resolve_selector_readonly`,
  `LibraryScope`, `SearchFields`, `searchable_text`.
- `src/utils/variables.rs` — `VariableAssignments`, `parse_variables`,
  `strip_escape_sequences`.
- `src/library/manager.rs` — `readonly_library_sources`, the read-only
  scope resolver.
- `src/outcome.rs` — `CliOutcome` and the exit-code table.
- `src/main.rs` — dispatch (`src/main.rs:842-857`).

## File / line references

- `GetArgs`: `src/commands/get_cmd.rs:16`; `GetField`: `:55`
- `GetJsonOutput`: `:68`; `run`: `:96`; `output_match`: `:198`
- `expand_without_prompt`: `:279`; tests: `:332-433`
- Selector: `src/selector.rs` (`exact_selector` `:579`,
  `resolve_selector_readonly` `:623`)

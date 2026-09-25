# new_cmd — Create New Snippet

[← Back to Overview](../overview.md)

## Overview

`src/commands/new_cmd.rs` (920 lines) creates snippets from six mutually
exclusive command-body sources and funnels every source through one
metadata, validation, library-loading, and atomic-save pipeline. Exact
sources (stdin/file/editor) preserve bytes verbatim; interactive sources
(multiline/prompt/positional) keep their historical prompting behavior.

## CLI surface

`NewArgs` (`new_cmd.rs:18`), dispatched as `snp new` (alias `n`):

| Flag | Conflicts | Meaning |
|------|-----------|---------|
| `COMMAND` (positional) | `command_stdin, multiline, from_file, editor` | Inline command text |
| `-t/--tags [TAGS]` | — | `Set` with `0..=1` args; bare `-t` prompts (sentinel `__snp_prompt_tags__`) |
| `-m/--multiline` | `command_stdin, editor` | Two-blank-line stdin prompt |
| `--command-stdin` | `command, multiline, from_file, editor` | Byte-exact stdin ingestion |
| `--from-file PATH` | `command, command_stdin, editor` | Byte-exact file ingestion |
| `--editor` | `command, command_stdin, from_file` | `$VISUAL`/`$EDITOR` composition |
| `-d/--description` | — | Description (else prompted) |
| `-c/--config PATH` | — | Legacy single-file path fallback |
| `-l/--library NAME` | — | Target library |

`--command-stdin` requires `--description` (stdin is reserved for the
body) and rejects tag prompting (tags must be explicit or omitted).

## Flow / steps

`run()` (`new_cmd.rs:541`):

1. Guard stdin-mode preconditions (description/tags rules above).
2. Resolve `CommandSource` (`new_cmd.rs:72`): Stdin / File / Editor /
   MultilinePrompt / Positional / InteractivePrompt.
3. Acquire command data **before touching library state** (malformed
   stdin never triggers migration):
   - Stdin → `read_command_stdin` (`:178`); File → `read_file_command`
     (`:196`); Editor → `read_editor_command` (`:427`); Multiline →
     `read_multiline_command` (`:513`); Prompt/Positional → colored
     `Command>` line (`:518`).
   - All exact sources share `validate_exact_command_bytes` (`:131`):
     16 MiB cap (`MAX_COMMAND_STDIN_BYTES`), UTF-8, no NUL, non-blank.
     Accepted bytes are never modified (trailing newlines preserved).
4. Resolve description (flag or `Description>` prompt) and tags
   (`parse_tags` splits on space/comma, `:532`).
5. Resolve destination: `get_library_path(library)` → `load_library`;
   else legacy `load_snippets(fallback)` where fallback is `--config`
   or the primary library path.
6. `Snippet::new(description, command, tags)`, stamp `device_id` from
   sync settings (server rejects device-less snippets), push, then
   `save_library` / `save_snippets`.
7. `notify_mutation(SnippetCreate, User)` after the commit; print
   `Snippet added`.

Editor handling: `parse_editor_spec` (shell-word split, no shell),
`resolve_editor` (absolute/CWD-relative/PATH search, Windows
extensions, symlink-escape rejection), precedence
`$VISUAL > $EDITOR > vim`, private `snp-editor-*.sh` tempfile, direct
spawn with no shell, exit-status check, re-read + shared validator.

## Mutation vs read-only

Mutating. Writes one library file through `save_library` /
`save_snippets`, both of which gate on interrupted transactions and take
the local-data lock internally (`LibraryManager::gate_mutation`,
`mod.rs:180`).

## Auto-sync trigger

`notify_mutation(MutationKind::SnippetCreate, MutationOrigin::User)`
(`new_cmd.rs:650-653`), reported via `report_notification_result`
(Workstream B1). No explicit sync here; the detached worker picks it up.

## Error / exit mapping

`SnipResult<()>`; `main.rs` maps `Err` to exit 1 (general) with the
`SnipError` detail. Notable errors: stdin without description, tag
prompt with stdin, oversized/non-UTF-8/NUL/empty input (labeled by
`CommandSourceKind`: stdin/file/editor), directory/non-regular file,
missing file, editor not found/failed, library not found.

## Key invariants

- Input resolution precedes any library load/save.
- Exact sources are byte-exact; `--multiline` is **not** (two-blank-line
  terminator is consumed; trailing blanks unrepresentable).
- Never sanitize the command (by design); never log the body.
- `Snippet::new` validates; `device_id` stamped only when configured.
- Removing CLI flags is breaking (deprecate first).

## File / line references

- `NewArgs`: `src/commands/new_cmd.rs:18`; `CommandSource`: `:72`
- `validate_exact_command_bytes`: `:131`; `read_command_stdin`: `:178`
- `read_file_command`: `:196`; editor stack: `:238-475`
- `read_multiline_from`: `:485`; `run`: `:541`
- notify: `:650`; tests: `:659-920`

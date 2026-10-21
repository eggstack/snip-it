# new_cmd — Create New Snippet

[← Back to Overview](../overview.md)

## Overview

`src/commands/new_cmd.rs` (920 lines) creates snippets from six mutually
exclusive command-body sources and funnels every source through one
metadata, validation, library-loading, and atomic-save pipeline. Exact
sources (stdin/file/editor) preserve bytes verbatim; interactive sources
(multiline/prompt/positional) keep their historical prompting behavior.

This command creates snippets only. It does not edit existing ones (see
[edit_cmd.md](edit_cmd.md)) and it does not set the `output`/notes field
of the new snippet.

## CLI surface

`NewArgs` (`new_cmd.rs:18`), dispatched as `snp new` (alias `n`):

| Flag | Type | Default | Conflicts | Meaning |
|------|------|---------|-----------|---------|
| `COMMAND` (positional) | `Option<String>` | – | `command_stdin, multiline, from_file, editor` | Inline command text |
| `-t/--tags [TAGS]` | `Option<String>` | – | – | `Set` with `0..=1` args; bare `-t` prompts (sentinel `__snp_prompt_tags__`) |
| `-m/--multiline` | bool | `false` | `command_stdin, editor` | Two-blank-line stdin prompt |
| `--command-stdin` | bool | `false` | `command, multiline, from_file, editor` | Byte-exact stdin ingestion |
| `--from-file PATH` | `Option<PathBuf>` | – | `command, command_stdin, editor` | Byte-exact file ingestion |
| `--editor` | bool | `false` | `command, command_stdin, from_file` | `$VISUAL`/`$EDITOR` composition |
| `-d/--description` | `Option<String>` | – | – | Description (else prompted) |
| `-c/--config PATH` | `Option<PathBuf>` | – | – | Legacy single-file path fallback |
| `-l/--library NAME` | `Option<String>` | – | – | Target library |

`--command-stdin` requires `--description` (stdin is reserved for the
body, `new_cmd.rs:552-557`) and rejects tag prompting (tags must be
explicit or omitted, `new_cmd.rs:558-563`).

## Flow / steps

`run()` (`new_cmd.rs:541`):

1. **Guard stdin-mode preconditions** — description required
   (`new_cmd.rs:552-557`); tags must not use the prompt sentinel
   (`new_cmd.rs:558-563`).
2. **Resolve `CommandSource`** (`new_cmd.rs:565-577`): `Stdin` / `File` /
   `Editor` / `MultilinePrompt` / `Positional` / `InteractivePrompt`
   (the enum at `new_cmd.rs:73`).
3. **Acquire command data before touching library state** — malformed
   stdin never triggers migration (`new_cmd.rs:579-607`):
   - Stdin → `read_command_stdin` (`new_cmd.rs:178`);
   - File → `read_file_command` (`new_cmd.rs:196`);
   - Editor → `read_editor_command` (`new_cmd.rs:427`);
   - Multiline → `read_multiline_command` (`new_cmd.rs:514`);
   - Prompt/Positional → colored `Command>` line (`new_cmd.rs:518`).
   - All exact sources share `validate_exact_command_bytes`
     (`new_cmd.rs:131`): 16 MiB cap (`MAX_COMMAND_STDIN_BYTES`,
     `new_cmd.rs:11`), UTF-8, no NUL, non-blank
     (`new_cmd.rs:135-171`). Accepted bytes are never modified (trailing
     newlines preserved).
4. **Resolve description** (flag or `Description>` prompt,
   `new_cmd.rs:609-612`) and **tags** (`parse_tags` splits on
   space/comma, `new_cmd.rs:532`, applied at `new_cmd.rs:614-618`).
5. **Resolve the destination** (`new_cmd.rs:620-631`): the fallback is
   `--config` or the primary library file under
   `<libraries_dir>/<library>.toml`; the target is
   `get_library_path(library)` → `load_library(path)`, else
   `load_snippets(fallback)`.
6. **Create and stamp** (`new_cmd.rs:633-641`): `Snippet::new(description,
   command, tags)` (`src/library/model.rs:231`, which rejects an empty
   command or description), then stamp `device_id` from the sync settings
   (the server rejects device-less snippets, `new_cmd.rs:634-640`).
7. **Persist** (`new_cmd.rs:643-647`): `save_library` or `save_snippets`.
8. **Notify** `notify_mutation(SnippetCreate, User)` after the commit
   (`new_cmd.rs:650-653`); print `Snippet added` (`new_cmd.rs:655`).

## The six command sources

| Source | Flag | Byte-exact? | Notes |
|--------|------|-------------|-------|
| `Stdin` | `--command-stdin` | Yes | `read_command_stdin` (`new_cmd.rs:178`) reads at most `MAX + 1` bytes and never trims or appends a newline |
| `File` | `--from-file PATH` | Yes | `read_file_command` (`new_cmd.rs:196`): the resolved path must be a regular file; symlinks are followed and their targets validated; directories, FIFOs, sockets, and device nodes are rejected (`new_cmd.rs:208-220`) |
| `Editor` | `--editor` | Yes | `read_editor_command` (`new_cmd.rs:427`) |
| `MultilinePrompt` | `--multiline` | **No** | `read_multiline_from` (`new_cmd.rs:485`) terminates on two consecutive blank lines; the terminator is consumed and trailing blank lines are unrepresentable |
| `Positional` | `COMMAND` | Yes (as given) | An empty positional falls back to the `Command>` prompt (`new_cmd.rs:596-606`) |
| `InteractivePrompt` | – | No (trimmed) | `read_prompt_line` trims the input (`new_cmd.rs:529`) |

## The editor stack

- `parse_editor_spec` (`new_cmd.rs:360`) — shell-word split of
  `$VISUAL`/`$EDITOR`; **no shell** is invoked, so arguments pass through
  verbatim.
- `resolve_editor_spec` (`new_cmd.rs:400`) — precedence
  `$VISUAL` (non-empty) > `$EDITOR` (non-empty) > `vim`
  (`new_cmd.rs:401-409`).
- `resolve_editor` (`new_cmd.rs:245`) — absolute path must exist and be
  a file (`new_cmd.rs:248-267`); a CWD-relative path must resolve inside
  the CWD, with a symlink-escape rejection (`new_cmd.rs:269-325`); a bare
  name is searched along `PATH`, with `.exe`/`.cmd`/`.bat` extensions on
  Windows (`new_cmd.rs:327-346`).
- `read_editor_command` (`new_cmd.rs:427`) — private `snp-editor-*.sh`
  tempfile (`new_cmd.rs:431-435`), direct spawn with no shell, exit-status
  check (`new_cmd.rs:455-464`), re-read + shared validator
  (`new_cmd.rs:468-474`). The command body is never logged.

## Mutation vs read-only

Mutating. Writes one library file through `save_library`
(`src/library/persistence.rs:217`) or `save_snippets`
(`src/commands/mod.rs:206`), both of which call
`gate_mutation_on_interrupted_transactions()` and then
`acquire_local_data_lock()` internally
(`src/library/persistence.rs:225-233`, `src/commands/mod.rs:212-218`).

`new` also resolves the target through the **mutating**
`get_library_path` (`new_cmd.rs:626`) and `init_library_manager`
(`new_cmd.rs:621`), so it can migrate a legacy single-file checkout.

## Auto-sync trigger

`notify_mutation(MutationKind::SnippetCreate, MutationOrigin::User)`
(`new_cmd.rs:650-653`), reported via `report_notification_result`. No
explicit sync here; the detached worker picks it up.

`src/main.rs:952` classifies `Commands::New(_)` as a mutation command
(`StartupRecoveryPolicy::Allow` + `StartupServices::Logging`).

## Tokio runtime

`new` is a local-only command: it never initializes the global Tokio
runtime and `src/main.rs:467-479` passes no runtime.

## Error / exit mapping

`SnipResult<()>`; `src/main.rs` maps `Err` to exit 1 (general) with the
`SnipError` detail. Notable errors:

- stdin without description (`new_cmd.rs:552`); tag prompt with stdin
  (`new_cmd.rs:558`).
- Oversized / non-UTF-8 / NUL / empty input, labeled by
  `CommandSourceKind` (`new_cmd.rs:110-117`): `stdin`, `file`, or
  `editor`.
- Directory or non-regular file for `--from-file` (`new_cmd.rs:208-220`);
  missing file (`new_cmd.rs:198-206`).
- Editor not found (`new_cmd.rs:251`, `new_cmd.rs:279`, `new_cmd.rs:349`)
  or editor exited non-zero (`new_cmd.rs:455`).
- Library not found (`src/commands/mod.rs:102` in `get_library_path`).
- Empty command or description from `Snippet::new`
  (`src/library/model.rs:232-243`).

## Key invariants

- Input resolution precedes any library load/save.
- Exact sources are byte-exact; `--multiline` is **not** (the
  two-blank-line terminator is consumed; trailing blanks are
  unrepresentable).
- Never sanitize the command (by design); never log the body.
- `Snippet::new` validates; `device_id` is stamped only when configured.
- Removing CLI flags is breaking (deprecate first).
- The save path never post-processes `toml::to_string_pretty` — the
  golden corpus must survive a save/load round trip.

## Gotchas

- `--multiline` is not byte-exact. Use `--command-stdin`, `--from-file`,
  or `--editor` when full fidelity is required (`new_cmd.rs:509-513`).
- The six sources are not all clap-exclusive with each other: `--multiline`
  and `--from-file` do not conflict (`new_cmd.rs:35-55`), so
  `snp new --multiline --from-file PATH` is accepted and `--from-file`
  silently wins, because the source resolution order is
  stdin → file → editor → multiline → positional → prompt
  (`new_cmd.rs:565-577`). Every other pairing is rejected by clap.
- A bare `-t` (no value) triggers the tags prompt, which is incompatible
  with `--command-stdin` because stdin is already reserved for the body.
- The positional `COMMAND` and the interactive prompt are not equivalent:
  the positional path preserves the string as given, the prompt path
  trims it (`new_cmd.rs:529`).
- `new` never sets the `output` field; a new snippet always starts with
  an empty output (`src/library/model.rs:250`).
- `get_library_path` is the mutating resolver, so `snp new` can migrate a
  legacy checkout even before it writes the snippet.

## Key Files

- `src/commands/new_cmd.rs` — the command: args, the six command
  sources, the validator, the editor stack, and the save.
- `src/commands/mod.rs` — `get_library_path`, `init_library_manager`,
  `load_snippets`, `save_snippets`.
- `src/library/model.rs` — `Snippet::new` and `validate_library_name`.
- `src/library/persistence.rs` — `load_library` / `save_library`, the
  gate → lock → atomic-write path.
- `src/commands/edit_cmd.rs` — the sibling command that reuses
  `resolve_editor_spec`.
- `src/auto_sync/notification.rs` — `notify_mutation` and
  `report_notification_result`.
- `src/main.rs` — dispatch (`src/main.rs:467-479`).

## File / line references

- `NewArgs`: `src/commands/new_cmd.rs:18`; `CommandSource`: `:73`
- `validate_exact_command_bytes`: `:131`; `read_command_stdin`: `:178`
- `read_file_command`: `:196`; editor stack: `:245-475`
- `read_multiline_from`: `:485`; `run`: `:541`
- notify: `:650`; tests: `:659-920`

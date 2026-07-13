# new_cmd — Create New Snippet

## Overview

`src/commands/new_cmd.rs` owns command-source resolution and then sends every
source through the same metadata, validation, library-loading, backup, and
atomic-save pipeline. The positional command and existing prompts remain the
compatibility path; Release 2A adds exact stdin ingestion for shell helpers;
Release 2B adds file-based ingestion and editor-based creation.

## Sources

The CLI accepts these mutually exclusive command-body sources:

| CLI form | Internal source | Behavior |
| --- | --- | --- |
| `snp new '<command>'` | `CommandSource::Positional` | Existing positional behavior; non-empty commands are echoed as before. |
| `snp new` | `CommandSource::InteractivePrompt` | Existing single-line prompt, with the existing trim behavior. |
| `snp new --multiline` | `CommandSource::MultilinePrompt` | Existing two-blank-line stdin prompt. |
| `snp new --command-stdin` | `CommandSource::Stdin` | Reads stdin as exact UTF-8 command data. |
| `snp new --from-file <path>` | `CommandSource::FromFile` | Reads the specified file as exact UTF-8 command data. Symlinks are followed; the resolved target must be a regular file. |
| `snp new --editor` | `CommandSource::Editor` | Opens `$VISUAL` (if set), then `$EDITOR`, then `vim` for authoring the command body. Editor arguments are parsed with shell-word semantics. |

`--command-stdin` conflicts with a positional command, `--multiline`, `--from-file`, and `--editor`.
`--from-file` conflicts with a positional command, `--command-stdin`, `--multiline`, and `--editor`.
`--editor` conflicts with a positional command, `--command-stdin`, `--multiline`, and `--from-file`.
Source resolution happens before library resolution so malformed input cannot
trigger migration or persistence.

## Exact stdin contract

`read_command_stdin()` reads at most 16 MiB, validates UTF-8, rejects NUL bytes,
and returns the resulting `String` without trimming, shell parsing, evaluation,
or an appended newline. A supplied trailing newline (or multiple trailing
newlines) is stored unchanged. The command is data only: it is never executed
by the ingestion path and is not echoed to stdout or included in normal logs.

The data model rejects commands that are empty or whitespace-only, matching
`Snippet::new()` and existing positional creation behavior.

## File-based ingestion (`--from-file`)

`read_command_from_file(path)` reads the specified file as exact UTF-8 command
data. It applies the same validation as `read_command_stdin()`: at most 16 MiB,
valid UTF-8, no NUL bytes. Symlinks are followed; the resolved target must be a
regular file (directories, FIFOs, sockets, and device nodes are rejected). File
content is not trimmed, evaluated, or executed; it is stored verbatim.

Because the command body comes from a file rather than stdin, `--description` is
optional — it can be supplied as a flag or prompted interactively. The file path
is consumed but never included in the snippet data.

## Editor-based creation (`--editor`)

`read_command_from_editor()` resolves the editor using `$VISUAL` (if non-empty),
then `$EDITOR` (if non-empty), then falls back to `vim`. The editor
specification is parsed with `shell-words` so values like `code --wait`,
`nvim -f`, or `"/path with spaces/bin/code" --wait` work without invoking a
shell. The parsed program and arguments are passed through to the editor
verbatim.

The temporary file is created atomically via `tempfile::Builder` in the OS
temporary directory with prefix `snp-editor-`. On Unix, the file is created
with `0600` permissions. The `NamedTempFile` RAII guard ensures cleanup
regardless of success or failure.

After the editor exits, the temp file content is validated using
`validate_exact_command_bytes()` (shared with `--command-stdin` and
`--from-file`): 16 MiB cap, valid UTF-8, no NUL bytes, no empty or
whitespace-only content. Editor errors identify the editor executable and exit
status but never include the command body.

Because the command body comes from the editor rather than stdin, `--description`
is optional — it can be supplied as a flag or prompted interactively.

## Common exact-source validation

All exact sources (`--command-stdin`, `--from-file`, `--editor`) share a single
validation function:

```rust
fn validate_exact_command_bytes(
    bytes: Vec<u8>,
    source_name: &'static str,
) -> SnipResult<String>
```

This function enforces: 16 MiB maximum size, valid UTF-8 decoding, no NUL
bytes, and no empty or whitespace-only content. Using one shared validator
prevents validation drift across sources. The validator does not trim accepted
content — leading and trailing whitespace in non-empty commands is preserved.

## Metadata and persistence

`--description` accepts a direct description. With `--command-stdin`, it is
required because stdin is reserved for the command body; metadata prompts must
not consume the command stream. With `--from-file` and `--editor`, `--description`
is optional — these modes do not consume stdin, so interactive prompts are
available. `--tags` remains a prompt when passed without a value, and accepts
comma/space-separated values when given a value. The prompt-only form is
rejected for stdin ingestion.

After source and metadata resolution:

1. `get_library_path()` resolves a named or primary library.
2. The existing library or legacy single-file loader reads the collection.
3. `Snippet::new()` validates the command and description and assigns ID/time
   fields through the existing model.
4. The snippet is appended and saved through `save_library()` or
   `save_snippets()`, preserving backup and atomic-write behavior.

No separate stdin persistence implementation exists.

## Errors and atomicity

Invalid UTF-8, NUL bytes, oversized input, missing noninteractive metadata,
empty commands, missing libraries, non-existent files, broken symlinks,
directories, non-regular files passed to `--from-file`, failed or empty editor
output, and save failures return the existing general error status. They occur
before the new snippet is appended; save operations use the existing backup and
atomic replacement path, so an input or write failure does not leave a partial
snippet.

## Testing

- `src/commands/new_cmd.rs` unit tests cover exact newlines, invalid UTF-8,
  NUL rejection, tag parsing, `CommandSource` resolution for all modes,
  multiline input (including delimiter behavior, EOF before delimiter,
  leading/trailing blank lines), editor tempfile permissions and cleanup,
  `--from-file` symlink following, and `validate_exact_command_bytes` edge
  cases.
- `tests/integration.rs` verifies exact TOML round-trips, metadata, leading
  hyphens, Unicode, metacharacters, no trailing newline, invalid input
  atomicity, conflicts, legacy `--tags` prompting, golden corpus preservation
  across all sources (stdin, file, editor, positional), multiline terminator
  limitations, and `snp run` execution.
- `tests/integration.rs` also covers `--from-file` (valid files, symlinks,
  broken symlinks, directories, invalid UTF-8, NUL bytes, oversized files)
  and `--editor` (empty output, failed editor, successful creation, tempfile
  cleanup).
- `src/commands/shell_cmd.rs` unit tests verify Bash previous-command capture
  preserves leading whitespace and quotes/backslashes.
- Shell integration tests stub `snp` and verify that generated helpers pass
  command data over stdin without evaluating it.

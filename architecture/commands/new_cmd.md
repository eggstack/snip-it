# new_cmd — Create New Snippet

## Overview

`src/commands/new_cmd.rs` owns command-source resolution and then sends every
source through the same metadata, validation, library-loading, backup, and
atomic-save pipeline. The positional command and existing prompts remain the
compatibility path; Release 2A adds exact stdin ingestion for shell helpers.

## Sources

The CLI accepts these mutually exclusive command-body sources:

| CLI form | Internal source | Behavior |
| --- | --- | --- |
| `snp new '<command>'` | `CommandSource::Positional` | Existing positional behavior; non-empty commands are echoed as before. |
| `snp new` | `CommandSource::InteractivePrompt` | Existing single-line prompt, with the existing trim behavior. |
| `snp new --multiline` | `CommandSource::MultilinePrompt` | Existing two-blank-line stdin prompt. |
| `snp new --command-stdin` | `CommandSource::Stdin` | Reads stdin as exact UTF-8 command data. |

`--command-stdin` conflicts with a positional command and `--multiline`.
Source resolution happens before library resolution so malformed stdin cannot
trigger migration or persistence.

## Exact stdin contract

`read_command_stdin()` reads at most 16 MiB, validates UTF-8, rejects NUL bytes,
and returns the resulting `String` without trimming, shell parsing, evaluation,
or an appended newline. A supplied trailing newline (or multiple trailing
newlines) is stored unchanged. The command is data only: it is never executed
by the ingestion path and is not echoed to stdout or included in normal logs.

The data model rejects commands that are empty or whitespace-only, matching
`Snippet::new()` and existing positional creation behavior.

## Metadata and persistence

`--description` accepts a direct description. With `--command-stdin`, it is
required because stdin is reserved for the command body; metadata prompts must
not consume the command stream. `--tags` remains a prompt when passed without a
value, and accepts comma/space-separated values when given a value. The
prompt-only form is rejected for stdin ingestion.

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
empty commands, missing libraries, and save failures return the existing
general error status. They occur before the new snippet is appended; save
operations use the existing backup and atomic replacement path, so an input or
write failure does not leave a partial snippet.

## Testing

- `src/commands/new_cmd.rs` unit tests cover exact newlines, invalid UTF-8,
  NUL rejection, and tag parsing.
- `tests/integration.rs` verifies exact TOML round-trips, metadata, leading
  hyphens, Unicode, metacharacters, no trailing newline, invalid input
  atomicity, conflicts, and legacy `--tags` prompting.
- Shell integration tests stub `snp` and verify that generated helpers pass
  command data over stdin without evaluating it.

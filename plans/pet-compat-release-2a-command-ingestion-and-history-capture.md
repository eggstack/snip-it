# Release 2A Plan: Safe Command Ingestion and Shell History Capture

## Purpose

Add maintained, opt-in workflows for saving commands discovered during normal shell use without requiring users to copy and paste command text manually or edit TOML.

This release track closes a practical migration gap for established `pet` users: saving the current command buffer, saving the previously accepted command, and optionally selecting an item from shell history through shell-native facilities.

The implementation must remain additive. Existing `snp new` positional behavior, prompts, storage semantics, shell-buffer selection, synchronization, and TUI behavior must not change when the new options and shell helpers are not used.

This document is intended for implementation-agent handoff. Inspect the repository state before editing. File paths and signatures below are architectural guidance rather than immutable assumptions.

## Relationship to the Roadmap

This plan implements Release 2A from `plans/pet-migration-compatibility-roadmap.md`.

Release 1 is already complete and provides:

- `snp select` with stable success/cancellation/error outcomes;
- generated Bash, Zsh, and Fish buffer-insertion helpers;
- PTY-backed selector validation;
- strict stdout and exit-code contracts.

Release 2A builds on the generated shell integration and current `snp new` path. It must not reimplement shell selection, parse history files directly, or begin Release 3 migration-diagnostic work.

## Goals

1. Add a safe binary ingestion path for arbitrary command text, preferably `snp new --command-stdin`.
2. Preserve command bytes exactly except for an explicitly documented stdin EOF/newline policy.
3. Reuse the existing description, tags, library selection, validation, timestamping, backup, and persistence pipeline.
4. Add generated Bash, Zsh, and Fish helpers for saving the current buffer.
5. Add generated Bash, Zsh, and Fish helpers for saving the previous accepted command using shell-native history APIs.
6. Avoid command execution, shell evaluation, history-file parsing, and normal-level logging of command bodies.
7. Preserve cancellation and failure semantics without mutating libraries on partial input or prompt cancellation.
8. Add real-shell and integration coverage for quoting, multiline text, Unicode, leading hyphens, and secret-like strings.

## Non-Goals

- Parsing `.bash_history`, `.zsh_history`, or Fish history database/file formats in Rust.
- Automatically saving every command.
- Installing keybindings or shell hooks without explicit user action.
- Capturing command output, exit status, working directory, environment variables, timestamps from the shell, or process metadata.
- Executing, validating, linting, or normalizing captured shell syntax.
- Adding shell-history synchronization.
- Replacing current `snp new '<command>'` behavior.
- Implementing arbitrary history search inside the Rust binary.
- Starting multiline editor/file creation beyond the stdin primitive required by this track; broader creation modes belong to Release 2B.

## Product Invariants

### Existing `snp new` behavior remains stable

The existing positional command form and interactive prompt flow must continue to behave exactly as documented when no new ingestion flag is passed.

### Captured command text is data

The command must never be:

- passed through `eval`;
- executed;
- sourced;
- interpolated into a new shell command string;
- re-tokenized or normalized;
- printed to logs at normal verbosity.

### Shell-native history access only

Generated helpers should ask the active shell for current-buffer or previous-history content. The Rust binary receives explicit command text over stdin or another exact transport. It must not infer shell history formats.

### Local mutation is atomic

A cancelled description/tag prompt, failed validation, failed write, or shell-helper error must not leave a partial snippet or corrupt a library.

# Workstream A: Audit and Consolidate the Creation Pipeline

## A1. Inventory `snp new`

Inspect:

- clap definition for `Commands::New`;
- `src/commands/new_cmd.rs`;
- description/tag prompting;
- multiline handling already present, if any;
- library resolution;
- snippet construction and ID/timestamp generation;
- duplicate or empty-command validation;
- backups and atomic saves;
- sync hooks, if any;
- tests covering `--description`, positional command input, and noninteractive creation.

Document current behavior for:

- no command argument;
- positional command argument;
- empty string;
- command beginning with `-`;
- embedded newlines;
- prompt cancellation/EOF;
- `--description` without a TTY;
- explicit `--library`.

Do not add a second persistence implementation.

## A2. Introduce a unified command-source model

Refactor only as needed so all creation inputs feed one internal representation before validation and persistence.

A suitable internal type could be:

```rust
pub enum CommandSource {
    Positional(String),
    Stdin,
    InteractivePrompt,
}
```

or a resolved form:

```rust
pub struct NewSnippetInput {
    pub command: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub library: Option<String>,
}
```

The important property is that source-specific reading occurs before a shared validation/prompt/persistence pipeline.

Acceptance criteria:

- positional and current interactive paths remain behaviorally unchanged;
- source selection is mutually exclusive and validated by clap where possible;
- persistence code is not duplicated;
- tests can exercise source resolution independently from storage.

# Workstream B: `--command-stdin` Contract

## B1. Add the CLI option

Preferred surface:

```text
snp new --command-stdin
```

It should compose with:

```text
--description <text>
--tags <...>
--library <name>
```

The implementation must define conflicts with positional command input and any existing `--multiline` mode.

Recommended clap behavior:

- `--command-stdin` conflicts with positional `command`;
- `--command-stdin` conflicts with other command-body source flags;
- missing stdin data is a clear error or documented empty-command case, not an implicit prompt fallback;
- `--description` permits a fully noninteractive creation path.

Do not overload ordinary stdin prompting ambiguously. The flag must explicitly transfer ownership of stdin to command-body ingestion.

## B2. Define exact stdin preservation

Read stdin as bytes, validate UTF-8 because the current data model stores `String`, and preserve all valid UTF-8 content exactly.

Explicitly decide the EOF newline policy.

Preferred policy:

- preserve stdin exactly, including trailing newlines;
- do not apply `.trim()`, `.trim_end()`, line iteration, or shell parsing;
- reject NUL bytes if downstream TOML, terminal, or execution semantics cannot support them;
- document that command substitution in many shells strips trailing newlines, so callers requiring exact preservation should pipe or redirect rather than use `$(...)`.

Example:

```bash
printf '%s' 'git commit -m "message"' | snp new --command-stdin --description 'Commit with message'
```

For multiline content:

```bash
cat script.sh | snp new --command-stdin --description 'Deploy script'
```

Acceptance criteria:

- quotes, tabs, backslashes, Unicode, leading hyphens, pipes, redirects, semicolons, `$()`, backticks, and newlines round-trip;
- the implementation does not append a newline that was not supplied;
- the implementation does not remove a supplied trailing newline;
- invalid UTF-8 fails before any library mutation;
- oversized input follows an explicit bound or the repository's existing practical file-size policy.

## B3. Separate command stdin from prompt input

When stdin is consumed as command data, description and tag prompts cannot safely read from the same exhausted stream.

Define one of these coherent policies:

1. require `--description` in non-TTY/stdin ingestion contexts; or
2. open the controlling terminal for remaining prompts; or
3. support a dedicated `--interactive-metadata` mode that reads prompts from `/dev/tty` on Unix.

Preferred initial policy:

- if `--command-stdin` is used and no interactive controlling terminal is available, require `--description`;
- if a controlling terminal is available, prompts may use it only through an explicit, tested abstraction;
- never mix command bytes and prompt responses from the same stdin stream.

The implementation agent should choose the least invasive option consistent with current prompt architecture and cross-platform support.

## B4. Logging and audit policy

Audit all tracing, debug, and audit-log calls in creation paths.

Requirements:

- do not log the command body at info/warn/error levels;
- avoid debug logging full command bodies unless an existing explicit unsafe diagnostic mode already permits it;
- audit records should identify the action, library, snippet ID, and perhaps description, but not captured command text;
- errors should not echo the entire command body.

Add tests or targeted assertions where feasible.

# Workstream C: Generated Current-Buffer Capture Helpers

## C1. Extend `snp shell init`

Add maintained public functions/widgets alongside existing selection helpers.

Suggested names:

```text
snp_new_current
snp_new_previous
```

Names must be consistent across Bash, Zsh, and Fish unless shell conventions force a documented difference.

Do not install keybindings by default.

## C2. Bash current-buffer helper

Use `READLINE_LINE` as data. Pipe it to `snp new --command-stdin` without evaluation.

The helper should:

- preserve `READLINE_LINE` exactly;
- leave the shell buffer unchanged on success, cancellation, or failure unless a separately documented clear-on-success option is explicitly approved;
- forward optional arguments such as `--description`, `--tags`, and `--library` safely as an argument array;
- return the `snp new` exit status;
- avoid exposing command text through `echo` interpretation; use `printf '%s'`.

Representative intent:

```bash
printf '%s' "$READLINE_LINE" | snp new --command-stdin "$@"
```

Do not use `eval`.

## C3. Zsh current-buffer helper

Use `BUFFER` as data from a ZLE widget.

Requirements:

- preserve `BUFFER` and `CURSOR`;
- invoke `zle redisplay` after returning where needed;
- use `printf '%s' -- "$BUFFER"` or equivalent exact transport;
- register the widget through `zle -N`;
- do not execute the buffer.

## C4. Fish current-buffer helper

Use `commandline` to obtain the current buffer without executing it.

Requirements:

- capture the entire buffer rather than a tokenized fragment;
- preserve multiline content if Fish exposes it;
- pipe with exact semantics;
- leave the commandline unchanged;
- expose a bind example only in comments/documentation.

## C5. Metadata argument forwarding

Generated functions should support direct calls such as:

```bash
snp_new_current --description 'Useful command' --tags git,release --library work
```

Argument forwarding must be array/list based. No string concatenation or re-parsing.

If interactive metadata prompts are supported, the helper must ensure command stdin and prompt TTY are distinct.

# Workstream D: Previous-Command Capture Helpers

## D1. Shell-specific history semantics

Define “previous accepted command” per shell using supported shell-native facilities.

Do not read history files directly.

Potential approaches:

- Bash: `history 1`, `fc -ln -1`, or Readline/history builtins, after carefully removing history numbering without damaging command text;
- Zsh: `fc -ln -1`, event expansion APIs, or history arrays;
- Fish: `history search --max=1 ...` or the most reliable current Fish API.

The implementation agent must verify behavior in the supported shell versions and avoid assumptions that strip multiline structure or leading spaces.

## D2. Prevent helper self-capture

A naïve `snp_new_previous` invocation may become the newest history entry and capture itself.

The generated helper must account for each shell's timing model. Possible strategies include:

- retrieving history before invoking external commands;
- requesting the prior event rather than the current helper event;
- defining a widget/function invocation that is not inserted into history;
- detecting and rejecting the helper command itself only as a last resort.

This must be tested in real shells.

## D3. Multiline history entries

Preserve multiline history entries where the shell API supports them.

Do not flatten multiline commands with spaces or semicolons merely for convenience. If a shell API cannot provide an exact prior multiline entry, document the limitation rather than silently altering content.

## D4. Empty and sensitive history

If no previous history item exists, return a clear nonzero result and do not invoke `snp new` with an empty command.

Documentation must warn that shell history can contain credentials, tokens, private URLs, and other secrets. The helper must never print the captured command as a status message.

# Workstream E: Optional Native History Selection

This workstream is optional for Release 2A and should only be implemented if it remains narrow.

Possible public helper:

```text
snp_new_from_history
```

It should use the shell's native history search or an already-installed user tool, then pipe the chosen text to `snp new --command-stdin`.

Constraints:

- do not add `fzf` as a required dependency;
- do not add history parsing to the Rust binary;
- do not make this helper necessary for Release 2A completion;
- avoid duplicating `snp select` UI for shell history.

If omitted, document current/previous capture as the supported scope and leave history selection for later evidence-driven work.

# Workstream F: Error, Cancellation, and Atomicity Semantics

## F1. Creation outcomes

Define and test:

| Scenario | Expected result |
| --- | --- |
| Valid command and metadata | snippet persisted, exit 0 |
| Metadata prompt cancelled | no mutation, stable cancellation/error status |
| Invalid UTF-8 | no mutation, exit 1 |
| Empty command rejected | no mutation, exit 1 or usage code per policy |
| Missing library | no mutation, exit 1 |
| Save failure | original library remains valid |
| Shell history unavailable | no `snp new` invocation, helper nonzero |

Do not introduce a new global exit-code taxonomy in this pass.

## F2. Duplicate and empty command policy

Use existing repository behavior. Do not add new duplicate suppression or empty-command semantics unless required to make the ingestion path coherent.

If current positional creation permits an empty command, preserve compatibility but ensure shell helpers do not accidentally create one when no history item exists.

## F3. Backups and atomic writes

The new source modes must flow through existing backup and atomic-save code. Add a failure-injection or regression test if the current creation path lacks proof that malformed input cannot truncate the library.

# Workstream G: Testing

## G1. Command-ingestion integration tests

Add tests for:

- stdin single-line command;
- stdin multiline command;
- no trailing newline;
- one and multiple trailing newlines;
- tabs and Unicode;
- quotes, backslashes, `$()`, backticks, pipes, redirects, semicolons, ampersands;
- command beginning with `-`;
- invalid UTF-8;
- positional command plus `--command-stdin` conflict;
- explicit description/tags/library;
- prompt cancellation or missing metadata in noninteractive mode;
- no command body in stdout/stderr/log output.

Verify stored TOML by loading through project APIs, not fragile text matching alone.

## G2. Real-shell helper tests

For Bash, Zsh, and Fish, source generated integration and stub `snp` to capture arguments and stdin.

Verify:

- current buffer reaches stdin byte-for-byte;
- previous command reaches stdin without numbering or helper text;
- metadata arguments are forwarded exactly;
- buffers/cursors remain unchanged;
- no command execution;
- no `eval` of captured content;
- missing history returns nonzero;
- secret-like input is not echoed.

Use actual shells where available. Skip with a clear reason when a shell is absent.

## G3. PTY tests

Add targeted PTY coverage for at least one real shell and the real `snp` binary:

- populate a buffer containing quotes and metacharacters;
- invoke the current-buffer save widget;
- complete metadata entry or provide noninteractive metadata;
- verify the resulting snippet exactly;
- verify shell state remains usable afterward.

Keep PTY tests serialized and deterministic.

## G4. Regression suite

Run:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
```

Preserve all Release 1 tests and existing CLI behavior.

# Workstream H: Documentation

Update:

- `README.md` with concise setup and examples;
- `USER_GUIDE.md` with exact stdin semantics and shell helpers;
- `docs/PET_COMPATIBILITY.md` with Release 2A status;
- `docs/CLI_EXITCODE_STREAM_POLICY.md` if prompt/stream ownership changes;
- `docs/ARCHITECTURE_INVENTORY.md` with command-source and helper architecture;
- `AGENTS.md` with test commands and safety invariants.

Documentation must include:

- warning about secrets in history;
- statement that captured commands are not executed;
- distinction between current buffer and previous accepted command;
- no default keybindings;
- exact examples for Bash, Zsh, and Fish;
- newline-preservation policy;
- noninteractive metadata requirements.

# Implementation Order

1. Audit and test current `snp new` behavior.
2. Introduce the shared command-source/creation pipeline.
3. Add and validate `--command-stdin`.
4. Add current-buffer helpers for all three shells.
5. Add previous-command helpers for all three shells.
6. Add real-shell and PTY tests.
7. Reconcile documentation and compatibility matrix.
8. Run full workspace validation.

# Definition of Done

Release 2A is complete when:

1. `snp new --command-stdin` stores exact valid UTF-8 command text through the existing creation pipeline.
2. It composes safely with description, tags, and library options.
3. Bash, Zsh, and Fish generated integrations expose maintained current-buffer and previous-command capture helpers.
4. Helpers do not execute, evaluate, print, or directly parse history files.
5. No-history, cancellation, invalid-input, and save-failure paths leave libraries unchanged.
6. Real-shell tests verify exact stdin and argument forwarding.
7. At least one PTY-backed end-to-end capture path is proven.
8. Existing Release 1 selection and shell-buffer insertion behavior remains unchanged.
9. Documentation accurately describes security, newline, and history semantics.

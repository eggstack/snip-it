# Release 1B Plan: Stable Machine-Facing Selection Primitive

## Purpose

Add a new non-executing selection command that reuses snip-it's existing search and TUI capabilities while providing a strict machine-facing output contract suitable for shell-buffer integration.

The preferred command is:

```text
snp select
```

This command is the core compatibility primitive for established `pet` users. It should allow a shell widget to seed search from the current command line, let the user choose a snippet, and receive the selected command as data without executing it or routing it through the clipboard.

This phase must not add shell startup code or keybindings. Those belong to Release 1C. Release 1B should deliver a stable, independently testable CLI contract that any shell integration can consume.

## Prerequisites

Before implementation, verify that Release 1A is complete:

- the pet compatibility matrix exists;
- current run/clip/search behavior is covered by regression tests;
- canonical and legacy fixtures exist;
- variable expansion semantics are protected;
- TUI cleanup has baseline coverage;
- stdout/stderr and cancellation policy has been recorded;
- the internal selection architecture has been inventoried.

Do not proceed by copying logic out of `run`, `clip`, or `search` if Release 1A identified a shared extraction path.

## Goals

1. Add a dedicated selection command that never executes the selected command.
2. Emit only the requested selected payload on stdout.
3. Support initial query, library constraint, and tag constraint.
4. Support raw stored command output and expanded variable output.
5. Define deterministic success, cancellation, and failure behavior.
6. Reuse current fuzzy matching, library resolution, TUI, and variable semantics.
7. Restore terminal state under every exit path.
8. Keep existing command behavior unchanged.
9. Preserve Windows compilation and existing functionality even if pseudo-terminal tests are Unix-specific.

## Non-Goals

- Generate Bash, Zsh, or Fish functions.
- Install shell keybindings.
- Capture shell history.
- Add multiline creation.
- Add pet multiple-choice variables.
- Add sorting modes.
- Add import commands.
- Change existing fuzzy ranking.
- Change `snp run`, `snp clip`, or `snp search` output.
- Add a general-purpose external selector interface.
- Execute, validate, lint, or shell-parse selected command text.

## Proposed CLI Surface

The final syntax should follow the project's current clap conventions, but the intended shape is:

```text
snp select [OPTIONS]
```

Recommended options:

```text
--query <TEXT>
--library <NAME>
--tag <TAG>
--raw
--expanded
--emit <FIELD>
--format <FORMAT>
```

The initial release does not need every option if the current CLI has equivalent established naming. The minimum required surface is:

```text
snp select --query <TEXT>
snp select --library <NAME>
snp select --raw
snp select --expanded
```

### Default mode

Use the decision recorded in Release 1A. Raw output is recommended as the default because shell-buffer insertion should preserve placeholders and avoid surprising interactive prompts.

If raw is the default, `--raw` may remain available as an explicit self-documenting flag while `--expanded` switches behavior.

Reject contradictory combinations such as `--raw --expanded` with a clap-level conflict where possible.

### Emit field

Recommended initial values:

```text
command
description
json
```

`command` should be the default.

A JSON form is useful for future integrations and testing, but should not delay the core command if it adds disproportionate scope. If included, use a stable documented object with fields such as library, ID, description, command, tags, and whether expansion occurred.

Do not emit human-formatted search rows from this command.

### Tag behavior

Follow existing tag semantics exactly. If current commands allow multiple `--tag` options or a comma-separated form, reuse that behavior.

Do not invent tag conjunction/disjunction rules solely for `select`.

## Stream Contract

### Stdout

On successful selection, stdout contains exactly the selected payload plus the project's documented trailing-newline convention.

No logs, headings, prompts, progress messages, descriptions, or terminal escape sequences may appear on stdout.

For command output:

```text
<selected command>\n
```

If exact preservation requires handling a command that already ends in newlines, document whether the CLI adds one record-separating newline or provides a zero-terminated/JSON alternative. Avoid silently trimming command content.

### Stderr

Stderr is reserved for:

- warnings;
- operational errors;
- invalid arguments not already rendered by clap;
- configuration or library load failures;
- non-cancellation variable expansion failures.

Normal cancellation should not print an error.

### Controlling terminal

The TUI and interactive variable prompts should communicate through the controlling terminal rather than captured stdout.

Audit the current terminal backend carefully. A shell adapter will typically invoke:

```sh
selected="$(snp select --query "$buffer")"
```

If TUI rendering uses stdout directly, command substitution may capture escape sequences or prevent display. The implementation may need to open `/dev/tty` on Unix or use an equivalent terminal handle abstraction.

Do not implement a Unix-only architecture in the core command. Encapsulate controlling-terminal access and provide clear behavior on platforms where a controlling terminal is unavailable.

## Exit Contract

Use the cancellation code selected in Release 1A.

Required categories:

- Success: selection completed and payload emitted.
- Cancellation: user intentionally cancelled; no payload; no error text.
- Empty/no match: decide explicitly whether this is cancellation-like or a separate failure based on current TUI semantics.
- Operational failure: load, parse, terminal, prompt, or serialization error.
- Usage failure: handled by clap.

Shell integrations must be able to distinguish cancellation from operational failure.

Do not use panic or process abort as ordinary control flow.

## Workstream A: Shared Selection Architecture

### A1. Identify the minimal reusable service

Based on the Release 1A inventory, create or expose a selection service that accepts an explicit request structure.

Representative model:

```rust
struct SelectionRequest {
    library: Option<String>,
    query: Option<String>,
    tags: Vec<String>,
    expansion: ExpansionMode,
}

struct SelectedSnippet {
    library: String,
    snippet: Snippet,
    rendered_command: String,
    expansion: ExpansionMode,
}

enum SelectionOutcome {
    Selected(SelectedSnippet),
    Cancelled,
}
```

Names should match repository conventions.

The shared layer should own:

- library resolution;
- snippet loading;
- filtering constraints;
- initial query propagation;
- TUI invocation;
- selected snippet return;
- optional variable expansion.

It should not own:

- command execution;
- clipboard I/O;
- stdout formatting;
- shell integration.

### A2. Preserve existing callers

Where practical, adapt `run` and `clip` to the shared service only after regression tests prove equivalence.

A safer sequence is:

1. extract a narrow service behind current behavior;
2. keep current run/clip adapters unchanged in visible semantics;
3. add `select` as a new adapter;
4. compare behavior through tests.

Do not force `search` into the same service if its inspection semantics are materially different.

### A3. Avoid cloning TUI state

The new command should pass an initial query and mode into the current TUI rather than maintaining a separate selector implementation.

If current TUI constructors hard-code an empty query or action, generalize them through an explicit configuration structure with defaults matching current behavior.

## Workstream B: Initial Query and Constraints

### B1. Initial query

`--query` should initialize the same filter/search input used when the user types after opening the TUI.

Requirements:

- preserve Unicode;
- preserve whitespace;
- do not shell-parse the query;
- do not execute it;
- place cursor and mode consistently with existing TUI expectations;
- show the filtered result set immediately.

Decide whether the TUI opens in insert/filter mode when a query is supplied. Prefer the least surprising behavior for current users and shell invocation. Record and test the decision.

### B2. Library resolution

Reuse existing explicit-library and primary-library behavior.

Required cases:

- explicit existing library;
- explicit missing library;
- no explicit library in legacy single-file mode;
- no explicit library in library mode with primary library;
- no primary library where current behavior defines an error or fallback.

Do not introduce a new implicit all-library search unless already supported or separately approved.

### B3. Tag filtering

Apply tag constraints before or consistently with fuzzy matching using existing semantics.

Ensure the initial query remains independent of tag parsing.

## Workstream C: Raw and Expanded Output

### C1. Raw mode

Raw mode emits the stored command exactly as selected, including unresolved placeholders.

Requirements:

- no variable prompt;
- no shell expansion;
- no environment-variable expansion by snip-it;
- no trimming beyond current storage normalization;
- no command execution;
- multiline preservation.

### C2. Expanded mode

Expanded mode invokes the same variable parser and prompt behavior used by existing run/copy workflows.

Requirements:

- same required/default handling;
- same escape handling;
- same repeated-variable behavior;
- same cancellation semantics;
- no execution after expansion;
- expanded result emitted only after all prompts succeed.

If variable prompts currently depend on stdout, refactor them to use the controlling terminal without changing visible run/clip behavior.

### C3. Expansion cancellation

Cancelling during parameter entry should produce the same cancellation category as cancelling snippet selection unless Release 1A explicitly chose otherwise.

No partial command should be emitted.

## Workstream D: TTY and Terminal Lifecycle

### D1. Interactive TTY detection

Before launching the selector, verify that an appropriate controlling terminal is available.

When unavailable, fail clearly and quickly. Do not hang waiting for input or emit terminal escapes into a pipe.

The error should suggest that `snp select` is interactive and requires a terminal.

Future deterministic noninteractive selection modes may be added separately; they are not required here.

### D2. Terminal I/O isolation

Ensure command substitution can display the TUI while capturing only the final payload.

Likely design:

- open the controlling terminal for TUI input/output;
- retain process stdout for the final selected payload;
- retain stderr for diagnostics.

On Unix, `/dev/tty` may be appropriate. On Windows, use the current console handles or existing terminal backend abstractions.

Avoid scattering platform conditionals through selection logic.

### D3. Cleanup guards

Use RAII or an equivalent scoped guard to restore:

- raw mode;
- alternate screen;
- cursor visibility;
- mouse capture if enabled;
- bracketed paste or focus modes if enabled.

Cleanup should occur on normal return, cancellation, error propagation, and unwind where Rust safety permits.

### D4. Signal behavior

Audit current Ctrl-C and termination handling.

Do not regress current behavior. A signal during selection should restore the terminal before process exit where the current architecture supports it.

## Workstream E: Output Encoding and Exact Preservation

### E1. Command text as data

Do not route the selected command through a shell, `eval`, format-string interpretation, or lossy escaping.

Test commands containing:

```text
' single quotes
" double quotes
\\ backslashes
| pipes
> and < redirects
&& and ||
; semicolons
$(command substitution syntax)
`backticks`
$VARIABLE references
Unicode
leading hyphens
embedded tabs
embedded newlines
```

### E2. Trailing newline policy

A standard CLI writes a terminating newline, but a multiline command may already contain one or more trailing newlines.

Choose and document one of these policies:

1. Preserve payload bytes and append one record newline.
2. Preserve exact payload with no automatic newline under an explicit flag.
3. Offer JSON or NUL-delimited output for exact machine use.

For Release 1 shell integration, command substitution strips trailing newlines in common shells, so shell widgets may need a more robust transport for exact multiline preservation. Do not conceal this limitation. If Release 1C requires exact multiline buffer insertion, add a JSON/base64 or temporary-file transport rather than pretending command substitution is byte-perfect.

Prefer a structured `--format json` path if it solves this cleanly without making the default cumbersome.

### E3. Broken pipe

If the consumer closes stdout, handle broken pipe without a panic or noisy backtrace, following the project's existing CLI policy.

## Workstream F: Tests

### F1. Argument and help tests

Verify:

- `snp select --help`;
- conflicting raw/expanded options;
- missing option values;
- invalid emit/format values;
- unknown library handling;
- tag argument behavior.

### F2. Stdout/stderr tests

Use process-level tests to assert:

- success emits only payload on stdout;
- cancellation emits empty stdout;
- operational errors use stderr;
- logs do not contaminate stdout;
- JSON output parses when enabled.

### F3. Pseudo-terminal tests

Cover:

- initial query visible and applied;
- selection success;
- selection cancellation;
- empty result behavior;
- expanded-variable success;
- expanded-variable cancellation;
- terminal restoration;
- command substitution-style invocation.

Tests should interact with the actual binary where feasible.

### F4. Preservation fixtures

Select and emit snippets containing all special-character and multiline cases identified above.

Compare semantic or byte output according to the documented newline policy.

### F5. Regression tests

Run existing run/clip/search tests and add explicit assertions that their help, output, and action behavior did not change.

### F6. Cross-platform tests

Unix-specific pseudo-terminal tests should be gated.

Windows CI or local validation must at minimum compile all targets and run non-PTY unit/integration tests for:

- command parsing;
- selection request construction;
- raw/expanded logic where mockable;
- output formatting;
- error mapping.

## Documentation

Update README CLI overview with one concise line for `snp select`.

Add full documentation to the user guide covering:

- non-executing purpose;
- raw versus expanded mode;
- initial query;
- library and tag constraints;
- cancellation behavior;
- stdout contract;
- examples using command substitution for simple single-line commands;
- warning that shell integration should use `snp shell init` after Release 1C rather than hand-written brittle wrappers;
- multiline transport caveats until Release 1C addresses them.

Do not document shell commands that are not yet implemented.

## Validation

Run:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --doc --workspace
cargo build --workspace --all-features
```

Also perform manual checks in a real terminal:

```bash
snp select
snp select --query git
snp select --library work
snp select --raw
snp select --expanded
```

Verify command substitution with a simple fixture:

```bash
selected="$(snp select --query git)"
printf '<%s>\n' "$selected"
```

Confirm the TUI remains visible, stdout contains only the payload, cancellation leaves `selected` empty, and operational errors remain visible.

## Acceptance Criteria

This plan is complete when:

1. `snp select` exists as a dedicated non-executing command.
2. The command reuses the existing selector and variable semantics rather than duplicating them.
3. An initial query can seed the TUI.
4. Existing library and tag constraints work consistently.
5. Raw mode emits stored command text without prompts.
6. Expanded mode emits fully resolved command text without execution.
7. Successful stdout contains only the selected payload.
8. Cancellation has a stable distinct exit code, emits no payload, and is not presented as an error.
9. Operational failures use stderr and a non-cancellation status.
10. The TUI works when stdout is captured by command substitution.
11. Terminal state is restored after success, cancellation, variable cancellation, empty results, and errors.
12. Special-character, Unicode, and multiline fixtures are covered according to the documented transport policy.
13. Existing `run`, `clip`, and `search` behavior remains unchanged.
14. The full validation suite passes on available platforms, with any unavailable validation explicitly recorded.

## Handoff Notes for Release 1C

Release 1C should treat the following as stable dependencies:

- command name and options;
- success/cancellation/error exit contract;
- stdout payload contract;
- raw and expanded semantics;
- initial-query behavior;
- multiline transport mechanism.

Release 1C must not parse human-readable output or depend on terminal escape behavior. It should call only the documented machine-facing interface.
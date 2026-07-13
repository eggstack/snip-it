# Release 3A Plan: Pet Multiple-Choice Variable Compatibility

## Purpose

Teach snip-it to recognize and safely execute valid Pet multiple-choice placeholder syntax while preserving every existing snip-it variable behavior.

This is the first Release 3 implementation track. It should improve migration fidelity for existing Pet snippet collections without changing the semantics of current `<name>` and `<name=default>` placeholders, command execution, clipboard expansion, raw selection, or shell integration.

This plan is intended for implementation-agent handoff. Inspect the current parser, expansion pipeline, prompt abstractions, storage model, and compatibility fixtures before editing.

## Goals

1. Parse valid Pet choice syntax into an explicit internal representation.
2. Preserve existing required and default-valued variables exactly.
3. Present choices through a deterministic prompt or native list UI.
4. Support the same choice semantics in run, clip, search expansion, and `snp select --expanded`.
5. Keep `snp select --raw` and stored command text unchanged.
6. Produce actionable diagnostics for malformed or ambiguous syntax.
7. Preserve Pet-compatible TOML source text without destructive migration.

## Non-Goals

- Reproduce Pet's exact prompt rendering.
- Change the existing `<name>` or `<name=default>` grammar.
- Add arbitrary expression evaluation.
- Execute shell substitutions while parsing variables.
- Automatically rewrite imported snippet commands.
- Begin the explicit import or doctor commands from Release 3B/3C.

## Workstream A: Establish the Compatibility Grammar

### A1. Inspect Pet's canonical syntax

Use Pet source, documentation, and fixtures to determine the exact supported multiple-choice grammar, including:

- delimiter characters;
- choice ordering;
- whether one choice is a default;
- escaping rules;
- whitespace behavior;
- duplicate choices;
- empty choices;
- interaction with variable names;
- malformed sequences.

Do not infer a grammar from examples alone. Record the accepted grammar in `docs/PET_COMPATIBILITY.md` and parser tests.

### A2. Define an internal variable model

Evolve the current variable representation toward an explicit type such as:

```rust
pub enum VariableKind {
    Required,
    DefaultValue(String),
    Choices {
        values: Vec<String>,
        default_index: Option<usize>,
    },
}

pub struct VariableSpec {
    pub name: String,
    pub kind: VariableKind,
    pub source_span: Range<usize>,
}
```

The exact type may differ, but it must distinguish required, default, and choice variables without encoding semantics into ad hoc strings.

### A3. Preserve source spans

The parser should retain enough location information to:

- substitute only the intended placeholder;
- report malformed syntax with useful context;
- support repeated variables consistently;
- avoid global string replacement errors.

## Workstream B: Parser Implementation

### B1. Extend, do not replace, the existing parser

Add choice recognition after protecting current syntax with regression tests.

Required regression cases:

- `<name>`;
- `<name=default>`;
- escaped angle brackets;
- adjacent variables;
- repeated variables;
- defaults containing punctuation allowed today;
- malformed legacy inputs that currently remain literal.

### B2. Parse deterministically

The parser must not:

- reinterpret ordinary shell redirection as variables;
- consume across unrelated angle brackets;
- treat malformed choice syntax as a valid different variable form;
- panic on Unicode or incomplete input.

### B3. Define malformed-input policy

Choose and document one policy per context:

- permissive library loading may preserve malformed placeholders literally while recording diagnostics;
- expanded execution may fail with an actionable error if ambiguity prevents safe prompting;
- strict import/doctor modes in later tracks may reject the same input.

Do not silently pick a choice or collapse malformed syntax.

## Workstream C: Choice Prompting

### C1. Reuse the native interaction layer

Present choices using the existing terminal/TUI primitives where practical.

The prompt must support:

- visible variable name;
- ordered choices;
- default indication;
- keyboard selection;
- cancellation;
- terminal restoration;
- non-interactive failure when no controlling terminal is available.

### C2. Preserve cancellation contracts

For `snp select --expanded`, cancellation must return `CommandOutcome::Cancelled` and process exit code 4.

For existing `run`, `clip`, and `search`, preserve their current cancellation semantics.

### C3. Repeated variables

If the same named choice variable occurs multiple times, prompt once and reuse the selected value unless existing variable semantics explicitly require otherwise.

If repeated definitions conflict, emit a diagnostic rather than choosing unpredictably.

## Workstream D: Expansion Pipeline Integration

Integrate choice variables into the shared expansion service used by:

- `run`;
- `clip`;
- `search` where expansion is supported;
- `select --expanded`.

Do not duplicate parsing or prompting in command-specific modules.

Raw paths must remain raw:

- `select --raw` returns the original command text;
- list/export/storage preserve the original placeholder syntax;
- shell-buffer raw insertion remains unchanged.

## Workstream E: Serialization and Compatibility

Choice variables are command syntax, not new required metadata. Existing Pet TOML should load without conversion.

Tests must prove:

- load/save does not rewrite choice syntax unexpectedly;
- backups preserve it;
- sync preserves it;
- JSON/CSV output contains the original command;
- import/export paths do not expand choices.

## Workstream F: Diagnostics

Add structured parser diagnostics suitable for reuse by Release 3B and 3C.

A diagnostic should include:

- severity;
- snippet identity or index;
- variable name when available;
- source span or excerpt;
- machine-readable code;
- concise human-readable explanation;
- suggested correction when possible.

Keep the parser API usable without printing directly to stdout/stderr.

## Workstream G: Tests

### Unit tests

Cover:

- every valid choice syntax form;
- default choice handling;
- escaping;
- Unicode choices;
- spaces and punctuation;
- duplicates;
- empty alternatives;
- missing delimiters;
- nested or overlapping angle brackets;
- repeated compatible variables;
- repeated conflicting variables;
- fuzz/property tests ensuring no panic.

### Integration tests

Use fixture libraries containing choice variables and verify:

- raw selection returns exact source;
- expanded selection returns selected choice;
- default selection behavior;
- cancellation exit code;
- run/clip expansion;
- no-terminal behavior;
- storage and sync round trips.

### PTY tests

Drive at least:

- choice list selection;
- default acceptance;
- cancellation;
- repeated-variable single prompt;
- terminal restoration.

Run PTY tests serialized.

## Documentation

Update:

- `README.md`;
- `USER_GUIDE.md`;
- `docs/PET_COMPATIBILITY.md`;
- `docs/ARCHITECTURE_INVENTORY.md`;
- variable-parser architecture docs;
- `CHANGELOG.md`.

Document syntax precisely with examples and malformed cases.

## Acceptance Criteria

Release 3A is complete when:

- valid Pet choice variables parse into an explicit internal model;
- existing variable syntax remains regression-clean;
- all expansion consumers use the shared implementation;
- raw paths preserve original text;
- cancellation and terminal contracts are tested;
- malformed inputs produce reusable diagnostics;
- full workspace, Clippy, formatting, and PTY suites pass.

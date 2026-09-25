# variables.rs — Snippet Variable Parsing

[← Back to Overview](../overview.md)

## Overview

Variables allow snippets to be parameterized at runtime. Syntax: `<name>`, `<name=default>`, or `<name=|_opt1_||_opt2_||_opt3_||>` for Pet-style multiple choice.

**File**: `src/utils/variables.rs` (1919 lines — parser, Pet-choice support, diagnostics, `VariableAssignments`, and ~100 unit tests)

## Data Structures

### Variable

```rust
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
    pub default: Option<String>,
}
```

`default` is a backward-compatible convenience: `None` for `Required`, `Some(val)` for `DefaultValue`, `Some(first_choice)` for `Choices`.

### VariableKind

```rust
pub enum VariableKind {
    Required,
    DefaultValue(String),
    Choices {
        values: Vec<String>,
        default_index: Option<usize>,
    },
}
```

### VariableDiagnostic

```rust
pub struct VariableDiagnostic {
    pub severity: DiagnosticSeverity, // Warning | Error
    pub message: String,
    pub span: Option<std::ops::Range<usize>>,
    pub code: &'static str,           // "choice.malformed" | "choice.empty" |
                                      // "choice.unclosed" | "var.duplicate"
    pub suggested_fix: Option<String>,
}
```

### VariableAssignments

```rust
pub struct VariableAssignments(BTreeMap<String, String>);
```

Explicit non-interactive assignments from `--var key=value` CLI args. `parse_arg` splits on the first `=` (empty key rejected); `from_pairs` deduplicates identical pairs but rejects conflicting values for the same key.

## Parsing

### parse_variables()

```rust
pub fn parse_variables(command: &str) -> Vec<Variable>
```

- `<name>` → `Required`; `<name=default>` → `DefaultValue`; `<name=|_opt1_||_opt2_||>` → `Choices { values, default_index: Some(0) }` with `default = Some(first_choice)`.

### parse_variables_diagnostics()

```rust
pub fn parse_variables_diagnostics(command: &str) -> (Vec<Variable>, Vec<VariableDiagnostic>)
```

Same parse plus warnings: malformed/unclosed/empty choice syntax and duplicate names. Malformed choices fall back to plain `DefaultValue` rather than failing.

### Internals

`extract_variable_tokens()` walks the command char-by-char: backslash-state tracking, escaped `\<` skipped, `<`/`>` matched with a depth counter for nesting, `\\`/angle escapes handled inside the body, content trimmed, `name=default` split on the first `=`. Empty names (`<>`) are dropped. `is_choice_syntax` / `extract_choices` detect and parse the Pet `|_..._||` form (`None` if malformed).

## Expansion

### expand_command()

```rust
pub fn expand_command(command: &str, values: &[(String, String)]) -> String
```

Looks up `name` in provided values (repeated names consume successive entries positionally); falls back to the default (first choice for `Choices`), then to the bare variable name for missing required variables — never errors. Escaped `\<` emits a literal `<`; unparsed `<...>` regions echo verbatim.

## Escape Sequences

### strip_escape_sequences()

```rust
pub fn strip_escape_sequences(command: &str) -> String
```

`\<` → `<`, `\>` → `>`, `\\` → `\`. Unknown escapes (`\n`) and trailing `\` are preserved. Call whenever a command is copied or executed, even without variables.

### has_unmatched_angle_bracket()

```rust
pub fn has_unmatched_angle_bracket(command: &str) -> bool
```

Nesting- and escape-aware unclosed-`<` detector used for validation warnings.

## Edge Cases

- Unmatched `<` echoes literally (`echo <hello` → `echo <hello`).
- `\<`/`\>` inside a variable name are stripped: `<x\>foo` → `<x>foo`.
- Malformed choice syntax and duplicate names warn (`choice.*`, `var.duplicate`) but never fail.

## Choice Variables

Pet-compatible `<name=|_opt1_||_opt2_||_opt3_||>`: choices delimited by `||` in `|_` ... `_|` markers, first choice is the default, TUI renders a navigable list selector, raw text stays in storage until prompting, `expand_command` treats the selection like any value.

## Usage in Commands

Parse → prompt (defaults / `--var` pre-fill) → expand → execute.

## Related

- `src/ui/variables.rs` — TUI variable prompt (see [ui.md](../ui.md))
- [run_cmd.md](../commands/run_cmd.md) — Variable expansion during execution

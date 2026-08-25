# variables.rs — Snippet Variable Parsing

## Overview

Variables allow snippets to be parameterized at runtime. Syntax: `<name>` or `<name=default>` or `<name=|_opt1_||_opt2_||_opt3_||>` for Pet-style multiple choice.

## Data Structures

### Variable

```rust
pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
    pub default: Option<String>,
}
```

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

## Parsing

### parse_variables()

```rust
pub fn parse_variables(command: &str) -> Vec<Variable>
```

Extracts all variables from a command string:
- `<name>` → `Variable { name, kind: Required, default: None }`
- `<name=default>` → `Variable { name, kind: DefaultValue("default"), default: Some("default") }`
- `<name=|_opt1_||_opt2_||_opt3_||>` → `Variable { name, kind: Choices { values: ["opt1", "opt2", "opt3"], default_index: Some(0) }, default: Some("opt1") }`

### extract_variable_tokens() *(internal)*

Returns raw `<...>` tokens for internal use by `parse_variables` and `expand_command`.

### is_choice_syntax() / extract_choices() *(internal)*

Private helpers. `is_choice_syntax` detects Pet-compatible multiple-choice syntax (`|_..._||_..._||`) within a default value string. `extract_choices` parses the individual choice values and returns `Option<Vec<String>>` — `None` if malformed.

## Expansion

### expand_command()

```rust
pub fn expand_command(
    command: &str,
    values: &[(String, String)],
) -> String
```

Substitutes values into command:
- Looks up `name` in provided values
- Uses default if value not provided but default exists
- Falls back to the variable name itself for missing required variables (no error returned)
- For `Choices` variables, the selected value is used just like a required variable value

## Escape Sequences

### strip_escape_sequences()

Converts escaped angle brackets and backslashes:
- `\<` → `<`
- `\>` → `>`
- `\\` → `\`

This allows literal angle brackets in commands without triggering variable substitution.

## Edge Cases

- Unmatched `<` without a matching `>` is treated as a literal `<` in the output (no variable substitution, character preserved). For example, `echo <hello` expands to `echo <hello`.
- Escape sequences (`\<`, `\>`) inside a variable name are stripped during parsing: `<x\>foo` expands to `<x>foo` — the backslash is silently dropped because the `>` is consumed as the variable terminator.
- Malformed choice syntax (e.g., `<name=|>` with no options) triggers a parser diagnostic warning.
- Duplicate variable names within a single command are warned about but not rejected.

## Choice Variables

Pet uses a `Choice|choice1|choice2|choice3` syntax for multiple-choice parameters. snip-it recognizes the Pet-compatible form `<name=|_opt1_||_opt2_||_opt3_||>` where:

- Choices are delimited by `||` and wrapped in `|_` ... `_|` markers
- The first choice is the default
- During TUI prompting, choice variables render as a navigable list selector (arrow keys / j/k in normal mode)
- Raw command text is preserved in storage — choices are only expanded during interactive prompting
- `expand_command` treats the selected value identically to a required variable value

## Usage in Commands

Variables are expanded before shell execution:
1. Parse variables from command
2. Prompt user for values (or use defaults)
3. Expand command with provided values
4. Execute expanded command

## Related

- `src/ui/variables.rs` — TUI variable prompt (see [ui.md](../ui.md))
- [run_cmd.md](../commands/run_cmd.md) — Variable expansion during execution

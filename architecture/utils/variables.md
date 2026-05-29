# variables.rs — Snippet Variable Parsing

## Overview

Variables allow snippets to be parameterized at runtime. Syntax: `<name>` or `<name=default>`.

## Data Structures

### Variable

```rust
pub struct Variable {
    pub name: String,
    pub default: Option<String>,
}
```

## Parsing

### parse_variables()

```rust
pub fn parse_variables(command: &str) -> Vec<Variable>
```

Extracts all variables from a command string:
- `<name>` → `Variable { name, default: None }`
- `<name=default>` → `Variable { name, default: Some("default") }`

### extract_variable_tokens()

Returns raw `<...>` tokens for display without parsing defaults.

## Expansion

### expand_command()

```rust
pub fn expand_command(
    command: &str,
    variables: &[(String, Option<String>)],
) -> SnipResult<String>
```

Substitutes values into command:
- Looks up `name` in provided variables
- Uses default if value not provided but default exists
- Returns error for missing required variables

## Escape Sequences

### strip_escape_sequences()

Converts escaped angle brackets:
- `\<` → `<`
- `\>` → `>`

This allows literal angle brackets in commands without triggering variable substitution.

## Known Edge Case

Unmatched `<` without `>` creates a phantom variable and drops the `<` character. For example, `echo <hello` would interpret `hello` as a variable name.

## Usage in Commands

Variables are expanded before shell execution:
1. Parse variables from command
2. Prompt user for values (or use defaults)
3. Expand command with provided values
4. Execute expanded command

## Related

- [ui/variables.rs](../../ui/variables.md) — TUI variable prompt
- [run_cmd.md](run_cmd.md) — Variable expansion during execution

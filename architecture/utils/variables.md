# variables.rs — Variable Parsing, Expansion & Escape Stripping

[← Back to Overview](../overview.md)

## Overview

`src/utils/variables.rs` implements the `<variable>` mini-language used to
parameterize snippet commands: it parses variables out of a command, expands
them against user-supplied values, and strips the escape sequences that let a
literal `<` or `>` live inside a command.

**File**: `src/utils/variables.rs` (1947 lines — parser, Pet-choice support,
diagnostics, `VariableAssignments`, and the unit-test tail that starts at
`src/utils/variables.rs:794`).

**Not covered here:** the TUI prompt dialog (`src/ui/variables.rs`), the
non-interactive `--var` CLI surface (`src/commands/get_cmd.rs`), and the
selector/search field rules (those are a separate subsystem — see
`.skills/selector-and-search-parity.md`). This document covers only the
grammar and the algorithms.

## Grammar

| Form | Meaning |
|------|---------|
| `<name>` | Required variable — the user must supply a value |
| `<name=default>` | Variable with a default value |
| `<name=\|_opt1_\|\|_opt2_\|\|_opt3_\|\|>` | Pet-compatible multiple choice; the **first** choice is the default |
| `\<` / `\>` | Escaped angle brackets — literal `<` / `>` |
| `\\` | Escaped backslash — literal `\` |

The name and the default are trimmed of surrounding whitespace
(`src/utils/variables.rs:248-249`); the split is on the **first** `=`, so a
default may itself contain `=`.

## Data Structures

### Variable / VariableKind

```rust
// src/utils/variables.rs:33-53
pub enum VariableKind {
    /// `<name>` — required, no default value.
    Required,
    /// `<name=default>` — has a default value.
    DefaultValue(String),
    /// `<name=|_opt1_||_opt2_||_opt3_||>` — multiple choice (Pet syntax).
    Choices {
        values: Vec<String>,
        default_index: Option<usize>,
    },
}

pub struct Variable {
    pub name: String,
    pub kind: VariableKind,
    /// Backward-compatible convenience field: `None` for Required, `Some(val)` for DefaultValue,
    /// `Some(first_choice)` for Choices.
    pub default: Option<String>,
}
```

`default` is a convenience mirror of `kind`: `None` for `Required`,
`Some(default)` for `DefaultValue`, `Some(first_choice)` for `Choices`
(`src/utils/variables.rs:300-315`).

### VariableDiagnostic

```rust
// src/utils/variables.rs:13-29
pub enum DiagnosticSeverity { Warning, Error }

pub struct VariableDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub span: Option<std::ops::Range<usize>>,
    /// Machine-readable diagnostic code (e.g., `"choice.malformed"`, `"var.duplicate"`).
    pub code: &'static str,
    /// Optional suggested correction the caller may present to the user.
    pub suggested_fix: Option<String>,
}
```

Codes emitted by the parser: `choice.malformed`, `choice.empty`,
`choice.unclosed`, `var.duplicate` (`src/utils/variables.rs:432`, `:444`,
`:456`, `:477`).

### VariableAssignments

```rust
// src/utils/variables.rs:571
#[derive(Debug, Clone, Default)]
pub struct VariableAssignments(std::collections::BTreeMap<String, String>);
```

Explicit **non-interactive** assignments from `--var key=value` CLI arguments
(`src/commands/get_cmd.rs:112-114`).

- `parse_arg(s)` (`src/utils/variables.rs:575-591`) splits on the **first**
  `=`, trims the key, rejects a missing `=` and an empty key. The value keeps
  everything after the first `=` verbatim.
- `from_pairs(pairs)` (`src/utils/variables.rs:596-612`) **deduplicates**
  identical keys and **rejects** conflicting values for the same key with a
  typed `SnipError`.
- `get(name)` (`src/utils/variables.rs:615-617`) is the lookup used by the
  non-interactive expansion path.

## Parsing

### parse_variables()

```rust
// src/utils/variables.rs:296-323
pub fn parse_variables(command: &str) -> Vec<Variable>
```

Returns the variables of a command **in source order, left to right** — the
order is positional and is what `expand_command` relies on.

Internally it delegates to `extract_variable_tokens()`
(`src/utils/variables.rs:178-294`), which walks the command character by
character:

1. Tracks backslash state so an escaped `\<` never opens a variable
   (`src/utils/variables.rs:187-197`).
2. On an unescaped `<`, scans forward with a **depth counter** so nested
   brackets are matched (`src/utils/variables.rs:200-243`). Inside the body,
   `\\`, `\<`, and `\>` are unescaped to a single character; any other
   backslash sequence keeps its backslash (`src/utils/variables.rs:203-225`).
3. Splits the body on the first `=` into `name` and `default`
   (`src/utils/variables.rs:247-249`).
4. Detects Pet choice syntax via `is_choice_syntax`
   (`src/utils/variables.rs:102-104`: starts with `|_` **and** ends with `||`)
   and parses it with `extract_choices`
   (`src/utils/variables.rs:117-156`).
5. Drops tokens with an empty name — a bare `<>` yields no variable
   (`src/utils/variables.rs:286-289`).

A token whose `depth != 0` at the end of the scan (an unclosed `<`) is
discarded entirely (`src/utils/variables.rs:245`).

### parse_variables_diagnostics()

```rust
// src/utils/variables.rs:333-511
pub fn parse_variables_diagnostics(command: &str) -> (Vec<Variable>, Vec<VariableDiagnostic>)
```

Same parse, plus a **second scanner** over the raw command that reports
choice-syntax problems the token extractor silently tolerates:

| Code | Condition | Site |
|------|-----------|------|
| `choice.malformed` | body starts with `\|_` and ends with `\|\|`, but `extract_choices` returns `None` | `src/utils/variables.rs:428-436` |
| `choice.empty` | body parses to zero choices | `src/utils/variables.rs:437-449` |
| `choice.unclosed` | body starts with `\|_` but does not end with `\|\|` or `_\|` | `src/utils/variables.rs:450-461` |
| `var.duplicate` | a variable name appears a second time | `src/utils/variables.rs:469-481` |

The duplicate diagnostic fires on the **second** occurrence only
(`*count == 2`, `src/utils/variables.rs:472`), so a name repeated three
times produces one warning.

Diagnostics are warnings — they never fail a parse. Malformed choice syntax
falls back to a plain `DefaultValue`
(`src/utils/variables.rs:259-265`).

### extract_variables_for_display()

```rust
// src/utils/variables.rs:513-525
pub fn extract_variables_for_display(command: &str) -> Vec<String>
```

Formats each variable for display: `"name (prompt)"` for `Required`,
`"name = default"` for `DefaultValue`, `"name = [a | b | c]"` for `Choices`.
Used by the TUI preview (`src/ui/mod.rs:489`).

## Expansion

### expand_command()

```rust
// src/utils/variables.rs:644-792
pub fn expand_command(command: &str, values: &[(String, String)]) -> String
```

Substitutes `<var>` tokens with values. It **never errors** — every failure
mode degrades to a literal substitution.

#### Algorithm

1. Tokenize the command once with `extract_variable_tokens()`
   (`src/utils/variables.rs:645`). Each token remembers its raw, trimmed body
   in `VariableToken::content` (`src/utils/variables.rs:170-176`) — that string
   is the **match key** between the two parsers.
2. Walk the command with its own scanner. Escaped `\<` is emitted as a
   literal `<` (`src/utils/variables.rs:666-670`); `\\` becomes `\`
   (`src/utils/variables.rs:657-663`).
3. On an unescaped `<`, scan the body with the same depth-counter logic and
   compare it against `tokens[token_idx].content`.
4. On a match, advance the cursor and look up a value:

```rust
// src/utils/variables.rs:751-757
let count = usage_count.entry(token.name.clone()).or_insert(0);
let replacement = values
    .iter()
    .filter(|(n, _)| n.trim() == token.name.trim())
    .nth(*count)
    .map(|(_, v)| v.trim());
*count += 1;
```

Values are matched **by name** and consumed **positionally**: the first
`<a>` takes the first pair whose name is `a`, the second `<a>` takes the
second, and so on. The `usage_count` map is what makes repeated variables
work. A supplied value is inserted **trimmed**.

5. If no value is supplied, the fallback is chosen by kind
   (`src/utils/variables.rs:759-774`):

| Kind | Fallback when no value was supplied |
|------|--------------------------------------|
| `DefaultValue(default)` | the default string |
| `Choices(choices)` | `choices.first()` (empty string if the list is empty) |
| `Required` | the **bare variable name** (e.g. `<host>` → `host`) |

The `Required` fallback is deliberate: the command still names what is
missing rather than silently substituting something wrong.

6. If the body does not match any token, it is echoed verbatim — `<`,
   the raw body, and (when the bracket was closed) `>`
   (`src/utils/variables.rs:775-781`).

#### Token-cursor resynchronization

`extract_variable_tokens()` and the scanner inside `expand_command()` are
**two separate parsers**. If they ever disagree on one token, matching purely
by position leaves `token_idx` stuck on the bad entry, and **every later
variable in the command silently fails to expand too**.

`expand_command` therefore resynchronizes (`src/utils/variables.rs:718-744`):

```rust
// src/utils/variables.rs:724-744
let current_matches = tokens
    .get(token_idx)
    .is_some_and(|t| t.content == var_content.trim());
let resync_offset = if current_matches {
    None
} else {
    tokens
        .iter()
        .skip(token_idx + 1)
        .position(|t| t.content == var_content.trim())
};
if let Some(offset) = resync_offset {
    let found = token_idx + 1 + offset;
    tracing::debug!(
        variable = %var_content.trim(),
        expected_position = token_idx,
        found_position = found,
        "variable token stream out of sync; resynchronized"
    );
    token_idx = found;
}
```

When the token at the cursor does not match, the parser looks **ahead** for
the next token with identical content and moves the cursor there. The
disagreement stays local: only the tokens between the expected and the found
position are affected, and the resync is logged at `debug` level.

## Escape Sequences

### strip_escape_sequences()

```rust
// src/utils/variables.rs:61-98
pub fn strip_escape_sequences(command: &str) -> String
```

Removes the escaping that protects variable syntax, for commands that are
about to be **copied or executed**:

| Input | Output |
|-------|--------|
| `\<` | `<` |
| `\>` | `>` |
| `\\` | `\` |
| `\n` (unknown escape) | `\n` — **preserved** |
| trailing `\` | `\` — **preserved** |

Only `<` and `>` are unescaped; every other backslash sequence is passed
through unchanged (`src/utils/variables.rs:79-88`). A trailing lone backslash
is re-emitted at the end of the scan (`src/utils/variables.rs:93-95`).

**Call it whenever a command is copied or executed**, regardless of whether
it contains variables (`src/utils/variables.rs:55-59`).

#### Why the TUI needs it

Stored snippet commands keep their escapes, so the raw text a user sees
contains `\<` where the executed command must contain `<`. Every surface that
hands a command to the shell or the clipboard therefore strips first:

| Surface | Site |
|---------|------|
| TUI snippet preview panel | `src/ui/mod.rs:505`, `src/ui/mod.rs:513` |
| TUI **visual-range multi-select copy** | `src/ui/mod.rs:1730` — the selected range is mapped through `strip_escape_sequences` before being joined with newlines and written to the clipboard |
| `snp get` output (default, `--field command`) | `src/commands/get_cmd.rs:238`, `:251`, `:282`, `:329` |
| Interactive run path with no variables | `src/commands/mod.rs:322` |

The multi-select case is the one that matters most: the user selects a range
of snippets, presses copy, and the clipboard must contain the *executable*
text — not the storage form with backslashes.

### has_unmatched_angle_bracket()

```rust
// src/utils/variables.rs:528-562
pub fn has_unmatched_angle_bracket(command: &str) -> bool
```

Nesting- and escape-aware detector for an unclosed `<`. It is used by the
TUI preview to append a "unmatched '<' found — will be treated as literal"
warning (`src/ui/mod.rs:501`, `:394`).

## Edge Cases

### Duplicate names

- `parse_variables` returns **every** occurrence — `<a> <a> <b>` yields three
  variables (`src/utils/variables.rs:1277`).
- `parse_variables_diagnostics` emits one `var.duplicate` warning on the
  second occurrence.
- `expand_command` consumes values **positionally per name**, so `<a> <a>`
  with `values = [("a","1"), ("a","2")]` expands to `1 2`. With only one
  value supplied, the second `<a>` falls back to its default or bare name.

### Nested angle brackets

The depth counter means `<` and `>` nest:

| Command | Parsed name |
|---------|-------------|
| `<outer<inner>>` | `outer<inner>` (`src/utils/variables.rs:995-999`) |
| `a<b<c>>d` | `b<c>` (`src/utils/variables.rs:1011-1015`) |
| `<<>>` | `<>` (`src/utils/variables.rs:1018-1022`) |

An escaped `>` inside the body does not close the variable:
`<var\>>` parses to the name `var>` (`src/utils/variables.rs:1038-1040`).

### Unmatched delimiters

| Command | Behaviour |
|---------|-----------|
| `echo <unclosed` | No variable is produced; the text is echoed literally (`src/utils/variables.rs:1081-1090`) |
| `<a<b>` | `has_unmatched_angle_bracket` is `true` (`src/utils/variables.rs:1026`) |
| `<a<b>>` | balanced — `false` (`src/utils/variables.rs:1031`) |
| `\<literal\>` | fully escaped — `false` (`src/utils/variables.rs:1272`) |

### Backslash runs

The parity between the two parsers is defined by a fixed table:

| Input | `parse_variables` | `expand_command` | `strip_escape_sequences` |
|-------|-------------------|------------------|--------------------------|
| `\\<foo>` | 1 variable `foo` | `\bar` (variable expands) | `\<foo>` |
| `\\\<foo>` | **0** variables | `\<foo>` (no expansion) | `\<foo>` |
| `\\\\<foo>` | 1 variable `foo` | `\\bar` | — |

An odd number of backslashes before `<` escapes it; an even number does not.

## Key Files

| File | Role |
|------|------|
| `src/utils/variables.rs` | The whole grammar: parsing, expansion, escape stripping, diagnostics |
| `src/commands/mod.rs:316-335` | `expand_snippet_command` — parse → prompt → expand (interactive path) |
| `src/commands/get_cmd.rs:427-…` | `expand_without_prompt` — the non-interactive `--var` path (does **not** use `expand_command`, but applies the same single-pass rule) |
| `src/ui/variables.rs:78` | `prompt_variables` — the TUI prompt dialog |
| `src/ui/mod.rs:489-513`, `:1639` | Preview rendering and the visual-range multi-select copy |
| `src/commands/import_cmd.rs:216-222` | Detects snippets that contain variables on import |
| `src/commands/doctor_cmd.rs:309-315` | Reports snippets that contain variables |

## Invariants

- **Commands are never sanitized.** The `<var>` syntax is a feature; the
  library stores commands verbatim and expansion happens at execution time.
- **`expand_command` never fails.** A missing value degrades to the default,
  then the first choice, then the bare variable name.
- **Values are matched by name and consumed positionally.** The `usage_count`
  map is the only mechanism that makes a repeated variable work.
- **The token cursor resynchronizes.** A disagreement between the two parsers
  must stay local to the affected token.
- **`strip_escape_sequences` only unescapes `<` and `>`.** Unknown escapes
  and a trailing backslash are preserved byte-for-byte.
- **The TUI copies the stripped form.** Anything written to the clipboard or
  handed to the shell must be stripped first.

## Gotchas

- **`expand_command` and `strip_escape_sequences` differ on unknown escapes.**
  At the top level `expand_command` special-cases only `\<`
  (`src/utils/variables.rs:666-670`); a backslash before any other character
  is **dropped**, so `expand_command(r"echo \n", &[])` yields `echo n`, while
  `strip_escape_sequences(r"echo \n")` yields `echo \n`. The two agree on
  angle brackets and on `\\`. If a command relies on an unknown escape
  surviving expansion, that behaviour is not shared.
- **`expand_command` is not the non-interactive path.** `snp get` uses its own
  `expand_without_prompt` (`src/commands/get_cmd.rs:427-…`), which leaves an
  unassigned `Required` variable **untouched** rather than substituting the
  bare name. Both paths resolve every token against the **original** command
  in a single left-to-right pass, so text produced by one substitution is
  never re-scanned for another token: a value that happens to look like
  `<other>` stays verbatim.
- **The default is applied at expansion time, not at parse time.** The parsed
  `Variable` always carries the default; whether it is used depends on the
  values handed to the expander.
- **A malformed choice list silently becomes a default value.**
  `parse_variables` does not warn — only `parse_variables_diagnostics` does.
- **Empty variable names are dropped.** `<>` produces no variable at all.
- **Pet choice syntax requires both ends.** `|_a_||_b_||` is a choice list;
  `|_a_` alone is just a default value that happens to start with `|_`.
- **Choice values cannot contain nested `|_ … _|` markers** —
  `extract_choices` does not support nesting and splits on the first matching
  `_|` (`src/utils/variables.rs:113-116`).
- **`VariableAssignments` rejects conflicting values.** `--var a=1 --var a=2`
  is an error, not a last-wins rule (`src/utils/variables.rs:599-608`).

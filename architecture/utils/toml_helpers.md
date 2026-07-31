# toml_helpers.rs — TOML Escape Handling

## Overview

Handles edge cases with backslash-containing strings in TOML configuration files.

**File**: `src/utils/toml_helpers.rs`

## Problem

TOML double-quoted strings interpret `\<` as an escape sequence, which fails because `\<` is not a valid TOML escape. This breaks snippet commands containing literal `<` or `>` characters (e.g., HTML tags, heredocs).

## Solution

Converts problematic double-quoted strings to single-quoted raw literals, which do not interpret escape sequences.

## Key Functions

### fix_invalid_toml_escapes()

```rust
pub fn fix_invalid_toml_escapes(toml_str: &str) -> String
```

Scans TOML content and rewrites single-line double-quoted strings containing `\<` or `\>` as single-quoted literals.

### quote_strings_containing_backslashes()

```rust
pub fn quote_strings_containing_backslashes(toml_str: &str) -> String
```

Reverses the conversion on save: converts single-quoted strings with backslashes back to double-quoted.

## Scanner Behavior

The hand-written scanner correctly handles:
- Line and block comments
- Table headers (`[table]`, `[[array]]`)
- Keys
- Single-quoted literal strings (passed through)
- Multi-line basic strings (`"""..."""`) — passed through
- Multi-line literal strings (`'''...'''`) — passed through
- Single-line basic strings (`"..."`) — checked and potentially rewritten

## Scope Limitations

- Only handles single-line strings (acceptable since snippet commands are single-line)
- snip-it's own save pipeline writes `toml::to_string_pretty` output verbatim without calling these helpers
- The helpers are for legacy/imported TOML files that pre-date the code path

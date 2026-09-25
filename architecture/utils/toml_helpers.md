# toml_helpers.rs — TOML Escape Handling

[← Back to Overview](../overview.md)

## Overview

Handles edge cases with backslash-containing strings in TOML configuration files.

**File**: `src/utils/toml_helpers.rs` (456 lines — hand-written scanner plus ~25 round-trip tests)

## Problem

TOML double-quoted strings interpret `\<` as an escape sequence, which fails because `\<` is not a valid TOML escape. This breaks snippet commands containing literal `<` or `>` characters (e.g., HTML tags, heredocs).

## Solution

Two complementary functions sharing one internal scanner:

| Function | Direction | Rewrites when content contains |
|----------|-----------|-------------------------------|
| `fix_invalid_toml_escapes(toml_str)` | **On load** (public) | `\<` or `\>` |
| `quote_strings_containing_backslashes(toml_str)` | **Hand-written TOML only** (`pub(crate)`) | any `\` |

```rust
pub fn fix_invalid_toml_escapes(toml_str: &str) -> String
pub(crate) fn quote_strings_containing_backslashes(toml_str: &str) -> String
```

For each affected single-line basic string:
1. If no single quotes in content → convert to single-quoted raw literal.
2. If single quotes present → stay double-quoted with backslashes doubled (`\\`), unescaping only the already-valid `\\` and `\"` first.

Example (`fix_invalid_toml_escapes`):

```toml
# before: invalid TOML — \< is not a legal escape
command = "sudo iptables-restore \< /path"
# after: raw literal, content preserved
command = 'sudo iptables-restore \< /path'
```

## Scanner Behavior

The internal `fix_toml_strings(toml_str, needs_fix)` scanner is hand-written (no regex) so it respects TOML token boundaries:

- Line comments (`# ...`) — copied verbatim (a `#` mid-key is not a comment)
- Table headers (`[table]`, `[[array]]`), keys, whitespace — passthrough
- Single-quoted literal strings (`'...'`) — copied verbatim (no escapes)
- Multi-line basic strings (`"""..."""`) — copied verbatim, never rewritten
- Multi-line literal strings (`'''...'''`) — copied verbatim
- Single-line basic strings (`"..."`) — content captured with escape awareness, then passed to `needs_fix`
- Unterminated basic strings (newline/EOF before close) — passed through verbatim so the parse error stays attributable to the real input

Non-ASCII UTF-8 survives intact: untouched strings reuse the original bytes.

## Scope Limitations

- Only single-line strings are rewritten; snippet commands are single-line, and triple-quoted regions always carry valid TOML escapes.
- snip-it's own save pipeline writes `toml::to_string_pretty` output **verbatim** without calling these helpers. That is deliberate: the serializer already picks correct quoting with native `\t`/`\r`/`\n` escapes, and converting those basic strings to single-quoted raw literals would silently corrupt tabs, carriage returns, and newlines.
- The helpers exist for legacy/hand-written TOML files that pre-date the save pipeline.

## Golden-Corpus Guarantee

Save/load must preserve tabs, trailing spaces, and CRLF byte-for-byte (see the `save_then_load_round_trip_with_escapes_and_crlf` test). The scanner's verbatim passthrough of triple-quoted regions is what makes that safe.

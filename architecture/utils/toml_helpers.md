# toml_helpers.rs — Legacy TOML Escape Repair

[← Back to Overview](../overview.md)

## Overview

`src/utils/toml_helpers.rs` contains a hand-written TOML token scanner and
the two functions built on top of it. Their single job is to repair **legacy**
snippet files that were written with escapes TOML does not allow — `\<` and
`\>` inside double-quoted strings — so that such files can still be parsed.

**File**: `src/utils/toml_helpers.rs` (456 lines — the scanner at `:34-172`,
the two entry points at `:190-209`, and the unit-test tail that starts at
`src/utils/toml_helpers.rs:211`).

**Not covered here:** TOML *serialization* fidelity (the save path — see
[persistence.md](../persistence.md) and
`.skills/persistence-and-toml-fidelity.md`), the schema-version writer, and
the `toml_cache` read cache (`src/config/toml_cache.rs`).

**Both helpers run on the load path only.** Neither may be applied to
`toml::to_string_pretty` output — doing so is exactly the bug that the golden
corpus in `.skills/persistence-and-toml-fidelity.md` guards against.

## The Scanner

```rust
// src/utils/toml_helpers.rs:34
fn fix_toml_strings(toml_str: &str, needs_fix: impl Fn(&str) -> bool) -> String
```

`fix_toml_strings` is a **predicate-driven scanner**, not a TOML parser. It
walks the document byte by byte, recognises enough TOML token structure to
know where a *string literal* begins and ends, and applies `needs_fix` to the
**content** of each single-line double-quoted string. Everything else is
copied through unchanged.

Token classes it recognises:

| Token | Handling | Site |
|-------|----------|------|
| Line comment `# …` | copied verbatim to end of line | `src/utils/toml_helpers.rs:43-50` |
| Whitespace / structural bytes | copied verbatim | `src/utils/toml_helpers.rs:53-57` |
| Multi-line basic string `"""…"""` | copied verbatim, **never rewritten** | `src/utils/toml_helpers.rs:60-75` |
| Multi-line literal string `'''…'''` | copied verbatim | `src/utils/toml_helpers.rs:78-93` |
| Single-line literal string `'…'` | copied verbatim (no escapes to fix) | `src/utils/toml_helpers.rs:96-110` |
| Single-line basic string `"…"` | content captured, `needs_fix` applied | `src/utils/toml_helpers.rs:113-165` |
| Unterminated basic string | copied verbatim so the parse error stays attributable to the real input | `src/utils/toml_helpers.rs:135-145` |
| Everything else (keys, numbers, headers) | copied byte by byte | `src/utils/toml_helpers.rs:167-168` |

Two details matter for fidelity:

- **Non-fixed strings reuse the original bytes**
  (`src/utils/toml_helpers.rs:161-162`) so non-ASCII UTF-8 survives intact.
- **Multi-line regions are skipped because they always contain valid TOML
  escapes** and the scanner cannot safely rewrite them
  (`src/utils/toml_helpers.rs:31-33`).

## fix_invalid_toml_escapes

```rust
// src/utils/toml_helpers.rs:205-209
pub fn fix_invalid_toml_escapes(toml_str: &str) -> String {
    fix_toml_strings(toml_str, |content| {
        content.contains("\\<") || content.contains("\\>")
    })
}
```

Repairs the two escapes that TOML's basic-string grammar does not allow.

### The invalid escapes

| Sequence | Why it is invalid |
|----------|-------------------|
| `\<` | `\` followed by `<` is not one of TOML's legal basic-string escapes (`\b \t \n \f \r \" \\ \uXXXX \UXXXXXXXX`), so `toml::from_str` rejects the document |
| `\>` | Same — `>` is not a legal escape target |

Every other backslash sequence (`\n`, `\t`, `\"`, `\\`, `\u00e9`, …) is legal
and is left untouched.

### The conversion

A double-quoted string that contains `\<` or `\>` is rewritten **only if it
can be represented as a literal string**. The decision is made at
`src/utils/toml_helpers.rs:148-159`:

| Content contains `'`? | Rewrite | Site |
|------------------------|---------|------|
| **No** | Convert to a **single-quoted raw literal**; `\\` → `\` and `\"` → `"` are unescaped | `src/utils/toml_helpers.rs:155-158` |
| **Yes** | Keep the **double quotes** and escape every backslash as `\\` | `src/utils/toml_helpers.rs:149-153` |

The single-quoted form is preferred because TOML literal strings have no
escape processing at all — the backslashes survive the round trip and the
original command text is preserved exactly. When the content itself contains
a single quote, a literal string is impossible, so the scanner falls back to
double quotes with doubled backslashes.

| Input | Output |
|-------|--------|
| `"echo \<name\>"` | `'echo \<name\>'` |
| `"echo 'x' \<name\>"` | `"echo 'x' \\<name\\>"` |
| `"a\nb"` | `"a\nb"` — unchanged, `\n` is legal |
| `"plain"` | `"plain"` — unchanged |

### Load-path only

`fix_invalid_toml_escapes` is called **immediately before** a parse, never
after a serialize:

```rust
// src/library/persistence.rs:168 (shape)
let content = fix_invalid_toml_escapes(&content);
```

The save path is explicitly forbidden from post-processing
`toml::to_string_pretty` output. The module states this in its own doc
comment (`src/utils/toml_helpers.rs:181-189`): the serializer already picks
the correct quoting style and uses TOML native escapes (`\t`, `\r`, `\n`)
inside basic strings, and running the helper on that output silently
corrupts tabs, carriage returns, and newlines.

### Call sites

Six call sites, in five files:

| File | Line | Enclosing function | What is repaired |
|------|------|--------------------|------------------|
| `src/commands/mod.rs` | `:177` | `load_snippets` (`:157`) | The legacy single-file `snippets.toml` |
| `src/library/persistence.rs` | `:168` | `load_library` (`:158`) | A library file in `<config>/libraries/` |
| `src/library/manager.rs` | `:55` | `LibraryManager::new` (`:41`) | `libraries.toml` (the registry, not a library) |
| `src/library/manager.rs` | `:106` | `LibraryManager::with_config_dir` (`:99`, `#[cfg(test)]`) | `libraries.toml` under an explicit config dir |
| `src/config/sync_settings.rs` | `:536` | `load_sync_settings` (`:498`) | `sync.toml` |
| `src/commands/pet_analysis.rs` | `:103` | `parse_pet_toml` (`:102`) | An imported Pet-format TOML file |

Every one of these is a **read** path: file bytes → repair → parse. No call
site writes the repaired text back to disk, so the repair is a compatibility
shim, not a migration.

### The server has its own copy

`snip-sync/src/premade.rs:13` defines a **separate, private**
`fix_invalid_toml_escapes` with the same name and the same purpose. It is
used when the server parses uploaded premade libraries
(`snip-sync/src/premade.rs:166`, `:292`).

The duplication is a consequence of the workspace split: `snip-sync` does not
depend on the `snip-it` binary crate, so it cannot call
`crate::utils::toml_helpers`. The two implementations are independent — a
change to one is **not** automatically a change to the other.

## quote_strings_containing_backslashes

```rust
// src/utils/toml_helpers.rs:190-193
#[allow(dead_code)]
pub(crate) fn quote_strings_containing_backslashes(toml_str: &str) -> String {
    fix_toml_strings(toml_str, |content| content.contains('\\'))
}
```

Converts double-quoted strings that contain **any** backslash — a broader
rewrite than `fix_invalid_toml_escapes`, which only reacts to `\<` and `\>`.
It uses the same quote-aware rewrite (literal form when there is no `'`,
escaped double quotes otherwise).

Two properties make it deliberately inert:

1. It is `pub(crate)` and marked `#[allow(dead_code)]` — **nothing calls it**.
2. It only applies to **hand-written** TOML, never to `toml::to_string_pretty`
   output. The doc comment at `src/utils/toml_helpers.rs:181-189` spells out
   exactly what it would corrupt.

It exists as the **trap that the save path used to fall into**. Applying it
to serialized output would turn every `\n` in a snippet command into a
literal backslash-n, changing the stored command text. Keeping the function
in the tree — but unreachable — makes the regression visible in review
instead of silently available.

## Key Files

| File | Role |
|------|------|
| `src/utils/toml_helpers.rs` | Scanner + the two repair helpers |
| `src/commands/mod.rs:177` | Repair on the legacy `snippets.toml` load path |
| `src/library/persistence.rs:168` | Repair on the library load path |
| `src/library/manager.rs:55`, `:106` | Repair on the `libraries.toml` registry load path |
| `src/config/sync_settings.rs:536` | Repair on the `sync.toml` load path |
| `src/commands/pet_analysis.rs:103` | Repair on the Pet TOML import path |
| `snip-sync/src/premade.rs:13` | The server's **separate** copy of the same repair |

## Invariants

- **The helpers run on the load path only.** The save path serializes with
  `toml::to_string_pretty` and does **not** post-process the result.
- **`write_schema_version` must use `toml::Table`, not `toml::Value`**, so
  array-of-tables survives (see [persistence.md](../persistence.md)).
- **Multi-line regions (`"""…"""` and `'''…'''`) are never rewritten.**
- **Only `\<` and `\>` are repaired.** A legal escape such as `\n` is left
  exactly as it is.
- **The repair is not written back.** Files are repaired in memory for the
  parse only; the on-disk bytes are unchanged until the next legitimate save.
- **`quote_strings_containing_backslashes` stays unreachable.**

## Gotchas

- **Do not call either helper on serialized output.** That is the single
  failure mode the golden corpus exists to catch — see
  `.skills/persistence-and-toml-fidelity.md`.
- **A string containing `'` is *not* converted to a literal.** It keeps its
  double quotes and every backslash is doubled
  (`src/utils/toml_helpers.rs:149-153`). The repair therefore changes the
  *representation* of the string, and the stored command text still round
  trips correctly — but a naive "expect single quotes" assertion will fail
  for such a string.
- **The repair is lossy about TOML *syntax*, not content.** Converting a
  double-quoted string to a literal string means any *other* legal escape in
  the same string (e.g. `\n` next to a `\<`) is no longer interpreted as an
  escape — it becomes a literal backslash-n. This is intentional (the stored
  command text is what must survive), but the repaired document is not
  byte-equivalent to the original intent.
- **The two implementations can drift.** `snip-sync/src/premade.rs:13` and
  `src/utils/toml_helpers.rs:205` are separate code with the same name.
  Fixing a bug in one does not fix the other.
- **The scanner is not a validator.** It happily rewrites strings inside a
  document that is invalid for other reasons; the parse still fails, and the
  caller handles it (a missing/empty file yields defaults; a malformed file
  fails closed — see [persistence.md](../persistence.md)).
- **`quote_strings_containing_backslashes` being `#[allow(dead_code)]` is
  intentional.** Wiring the function up is the bug, not the dead-code
  warning.

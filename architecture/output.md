# Output Presentation Module

[← Back to Overview](overview.md)

## Overview

Safe rendering for the snippet `output` field — free-form notes / example
output / reminders that travel with a snippet (`src/output.rs`, ~291
lines, domain/core layer). Display paths sanitize; storage, sync-merge
preservation, and JSON/CSV export keep the raw value.

## Key types — `src/output.rs:11-17`

```rust
pub const OUTPUT_SEARCH_BUDGET: usize = 512;
pub struct OutputPresentation<'a> { raw: &'a str } // borrowed; never owns/mutates
```

## Key functions — `src/output.rs:19-176`

```rust
OutputPresentation::new(raw: &'a str) -> Self
pub fn is_present(&self) -> bool               // !raw.is_empty()
pub fn raw(&self) -> &'a str                   // unmodified value
pub fn line_count(&self) -> usize              // 0 when empty
pub fn summary(&self, max_chars: usize) -> String // first line, sanitized, "..." on truncate
pub fn display(&self) -> String                // full content, sanitized
pub fn display_bounded(&self, max_lines: usize) -> String // + "... (N lines total)"
pub fn for_scoring(&self) -> String            // sanitized, ≤512 bytes, char-boundary safe
pub fn sanitize_for_terminal(input: &str) -> String
```

- `summary()` takes only the first line, then truncates by chars
  (`max_chars - 3 + "..."`).
- `for_scoring()` caps at `OUTPUT_SEARCH_BUDGET` bytes without splitting
  UTF-8 (backs up to the last char boundary) — the bound the selector's
  `SearchFields::with_output()` relies on.
- `sanitize_for_terminal()` strips ANSI CSI (any final byte `0x40-0x7E`),
  OSC (BEL- or `ESC \`-terminated), other `ESC x` pairs, C0 controls
  except `\n`/`\t`, DEL, and C1 (`U+0080-009F`). Preserves `\n` and
  `\t`; operates on a copy.

## Security properties

- Output is never evaluated, executed, interpolated, or shell-expanded;
  no variable substitution applies to it.
- ANSI/OSC is stripped for *display* but preserved in storage and
  JSON/CSV export — what you see sanitized is not what is saved.
- Sanitization never mutates the stored value.

## Local-only sync contract

`output` is **not** in `ProtoSnippet`, never uploaded or downloaded, and
preserved locally when remote data wins the merge (local-only alongside
`folders`/`favorite`, all excluded from the sync fingerprint). Another
device does not receive the value automatically. Consequences:

- Searchable only opt-in: `list --search-output` / MCP `search_output`
  via `for_scoring()`; default fuzzy paths ignore it (see `selector.md`).
- `snp edit --output` requires `--filter` (explicit targeting for a
  field sync cannot see).

## Invariants / gotchas (from AGENTS.md)

- Do not "fix" display by mutating storage — sanitization is
  presentation-copy only.
- `for_scoring()` budget (512) is a search-path DoS bound for
  pathological inputs; keep it in sync with `SearchFields` docs.
- Multibyte truncation must stay panic-free (covered by the CJK test).

## File / line refs

- `src/output.rs:11-17` (type + budget), `:49-64` (`summary`),
  `:67-89` (`display`/`display_bounded`), `:93-110` (`for_scoring`),
  `:118-176` (`sanitize_for_terminal`), `:178-291` (15 unit tests:
  empty, truncation, multiline, ANSI/OSC/BEL, controls, budget,
  CJK-boundary).
- Consumers: `src/ui/mod.rs` (preview `display()`),
  `src/commands/list_cmd.rs` (`for_scoring()` + `summary()` + raw
  export), `src/commands/edit_cmd.rs` (set/replace/clear),
  `src/selector.rs:150-157` (opt-in scoring input).

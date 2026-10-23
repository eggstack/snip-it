# Output Presentation Module

[← Back to Overview](overview.md)

## Overview

`src/output.rs` (291 LOC, 15 unit tests) is the shared presentation model for
the snippet `output` field. It provides sanitized renderings of that field
for terminals, a bounded form for fuzzy-search scoring, and a summary form
for inline display. It never interprets, evaluates, or mutates the value.

**What this document does not cover.** The `output` field's *data model* is
`Snippet::output` in [library.md](library.md). Its *search-field contract*
is in [selector.md](selector.md). Sync behavior is in [sync.md](sync.md).
The `snp run` stdout-redirection branch that also reads this field is in
[commands/run_cmd.md](commands/run_cmd.md) and is covered here only because
it shares the field.

---

## The field has two roles

The single most important thing to know about `output` is that **the same
string field carries two different meanings in the current code**.

| Role | Where | Meaning |
|---|---|---|
| **Metadata** | `Snippet::output` (`src/library/model.rs:35-36`), the display/sync/export paths below | Free-form notes, example output, reminders that travel with the snippet |
| **Stdout redirect target** | `src/commands/run_cmd.rs:208-216` | A **relative file path**. If the field is non-empty, `snp run` writes the command's stdout into `cwd.join(&snippet.output)` |

This is inherited pet-compatibility behavior and is documented as such in
[`docs/PET_COMPATIBILITY.md`](../docs/PET_COMPATIBILITY.md) ("snp treats
`output` as a file path for stdout redirection, not just metadata display").

The collision is real and user-visible: `snp edit --output "my notes"` writes
notes into the field, and a later `snp run` of that snippet takes the
output-file branch and tries to create a file named `my notes` in the current
directory. There is no separate flag or field distinguishing the two
meanings.

The hardening on the redirect branch (all in `src/commands/run_cmd.rs`):

- `cwd.join(&snippet.output)` — the path is joined onto the current working
  directory (`:216`). Note that `Path::join` **replaces** the base when its
  argument is absolute, so an absolute `output` value escapes the join — which
  is exactly why the containment check below is mandatory.
- Pre-open containment check: the canonicalized output path must start with
  the canonicalized CWD, else `"Invalid output path"` (`:225-259` — the
  existing-path branch at `:225-239`, the not-yet-created branch at
  `:240-259`).
- `OpenOptions::create_new(true)` — fails closed rather than following an
  existing symlink (`:261-262`).
- Post-open re-canonicalization + `dev`/`ino` identity check against the
  path — closes the parent-directory swap window between check and open
  (`:278-316`).
- The child is spawned with `cmd.stdout(file)`, so **stdout only** is
  redirected; stderr still reaches the terminal (`:323`, `:131-133`).
- The output-file branch uses the default timeout policy; the ordinary
  branch uses `TimeoutPolicy::NoDefault` (`:319-321` vs `:330`).

---

## Key types — `src/output.rs:11-17`

```rust
pub const OUTPUT_SEARCH_BUDGET: usize = 512;

pub struct OutputPresentation<'a> {
    raw: &'a str,
}
```

`OutputPresentation` **borrows** the raw string. It never owns, copies, or
mutates it; every rendering produces a new `String`.

---

## Key functions — `src/output.rs:19-176`

```rust
OutputPresentation::new(raw: &'a str) -> Self
pub fn is_present(&self) -> bool                     // !raw.is_empty()
pub fn raw(&self) -> &'a str                        // unmodified value
pub fn line_count(&self) -> usize                    // 0 when empty
pub fn summary(&self, max_chars: usize) -> String    // first line, sanitized, "..." on truncate
pub fn display(&self) -> String                      // full content, sanitized
pub fn display_bounded(&self, max_lines: usize) -> String // + "... (N lines total)"
pub fn for_scoring(&self) -> String                  // sanitized, ≤512 bytes, char-boundary safe

pub fn sanitize_for_terminal(input: &str) -> String
```

| Method | Line | Behavior |
|---|---|---|
| `summary(max_chars)` | `:49-64` | Takes **only the first line**, sanitizes it, and truncates by **chars** to `max_chars - 3` plus a `"..."` suffix. Empty input → empty string. |
| `display()` | `:67-69` | Full content, sanitized. |
| `display_bounded(max_lines)` | `:73-89` | First `max_lines` lines, each sanitized, joined with `\n`; when truncated, appends `"\n... (N lines total)"`. |
| `for_scoring()` | `:93-110` | Sanitized content capped at `OUTPUT_SEARCH_BUDGET` (512) **bytes**, backing up to the last UTF-8 char boundary so multi-byte input is never split mid-character. |

`is_present()`, `raw()`, `line_count()`, and `display_bounded()` are part of
the API but currently have no production caller. Each carries an item-level
`#[allow(dead_code)]` with an adjacent comment saying so (`src/output.rs`).

### `sanitize_for_terminal` — the safe-rendering rules

`src/output.rs:118-176`. It produces a **copy**; the stored value is never
modified.

| Input | Treatment | Line |
|---|---|---|
| `ESC [` … (CSI) | Parameter bytes `0x30-0x3F`, intermediate bytes `0x20-0x2F`, and the final byte `0x40-0x7E` are all consumed — the whole ANSI SGR/color sequence is removed | `:129-143` |
| `ESC ]` … (OSC) | Consumed up to `BEL` (`\x07`) or `ST` (`ESC \`) — hyperlinks, terminal titles, etc. | `:144-160` |
| `ESC` + any other char | Both characters dropped | `:161-162` |
| `\n`, `\t` | **Preserved** | `:165` |
| Other C0 controls (`0x00-0x08`, `0x0B`, `0x0C`, `0x0E-0x1F`) | Stripped | `:167` |
| `DEL` (`0x7F`) | Stripped | `:169` |
| C1 controls (`U+0080-009F`) | Stripped | `:171` |
| Everything else | Preserved | `:172` |

This is the "terminal escape injection" defense: a snippet whose `output`
field contains `ESC[31m` or an OSC 8 hyperlink cannot repaint or rewrite the
user's terminal when the field is rendered.

---

## Where the field is consumed

| Consumer | Line | Rendering |
|---|---|---|
| TUI detail pane | `src/ui/mod.rs:519`, `:1178` | `OutputPresentation::new(output).display()` — **sanitized**, full content, below a `--- Output / Notes ---` separator |
| `snp list` (default format) | `src/commands/list_cmd.rs:199-207` | `summary(80)` — **sanitized**, first line, 80 chars; the `Output:` line is omitted entirely when the field is empty |
| `snp list --json` | `src/commands/list_cmd.rs:158` | **raw** `s.output`, verbatim |
| `snp list --csv` | `src/commands/list_cmd.rs:184` | **raw** `s.output`, CSV-escaped |
| `snp search` | `src/commands/search_cmd.rs:68` | **raw** `println!("Output: {}", snippet.output)` — **not sanitized** |
| Fuzzy-search scoring | `src/selector.rs:170-177` | `OutputPresentation::for_scoring()` — sanitized and bounded |
| `snp edit` | `src/commands/edit_cmd.rs:209`, `:275` | Writes the new value; confirmation goes to **stderr** (`edit_cmd.rs:218,220,284,286`) |

### Surfaces that deliberately do **not** show `output`

| Surface | Why |
|---|---|
| `snp get` | `GetField` (`src/commands/get_cmd.rs:55-64`) offers only `command`, `description`, `id`, `tags`; `GetJsonOutput` (`src/commands/get_cmd.rs:68-77`) has **no** `output` key. `snp get` cannot display the field in any format. |
| MCP `snippet_json` | The serialized object (`src/mcp/tools.rs:263-273`) contains `id`, `library`, `description`, `command`, `tags`, `folders`, `favorite` — **no** `output`. |
| MCP `snippet_identity_json` | Only `id`, `library`, `description` (`src/mcp/tools.rs:275-281`). |

`snp search` printing the raw value (`src/commands/search_cmd.rs:68`) is the
one interactive surface that does **not** go through
`OutputPresentation`. It is the display path where a stored ANSI sequence
would reach the terminal unmodified.

---

## How the field is written

| Path | Behavior |
|---|---|
| `snp edit --output <text>` | Sets the field (`src/commands/edit_cmd.rs:209`) |
| `snp edit --output-stdin` | Sets the field from stdin (`src/commands/edit_cmd.rs:15-17`, `:209`) |
| `snp edit --clear-output` | Sets the field to the empty string (`src/commands/edit_cmd.rs:18-20`, `:209`) |
| `snp new` | **No `--output` flag exists** — `NewArgs` (`src/commands/new_cmd.rs:18-69`) has no output field |
| Import / hand-edited TOML | The field is serialized as `output` with the alias `Output` (`src/library/model.rs:35-36`) |

**Targeting rule.** `snp edit --output` / `--output-stdin` / `--clear-output`
require `--filter` **unless** an exact selector is given
(`--id`, `--description-exact`, `--command-exact`)
(`src/main.rs:641-710`):

- With an exact selector, the snippet is resolved through
  `selector::resolve_exact_target()` and edited by ID
  (`src/main.rs:676-698`, `src/commands/edit_cmd.rs:233-290`).
- Without one, `--filter` is mandatory; otherwise the command fails with
  `--filter is required when using --output, --output-stdin, or --clear-output`
  (`src/main.rs:699-706`).

The reason is the ambiguity risk recorded in the changelog: a substring
filter can match more than one snippet, and the fuzzy match's first hit was
previously influenced by the local-only `output` field.

---

## Local-only sync contract

`output` is **local-only**. It is not part of the wire type.

- `ProtoSnippet` (`snip_proto::Snippet`, imported as `ProtoSnippet` in
  `src/sync_commands.rs`) carries `id`, `description`, `command`, `tags`,
  `created_at`, `updated_at`, `device_id`, `deleted`, and `encrypted` —
  **no `output`** (`src/sync_commands.rs:425-435`).
- The conflict fingerprint (`fingerprint()`,
  `src/sync_commands.rs:1140-1163`) hashes id, description, command, tags,
  created_at, updated_at, device_id, and deleted — **no `output`**, and no
  `folders` or `favorite` either.
- The merge **preserves the local value** when the remote side wins:
  `output: local_snip.output.clone()` (`src/sync_commands.rs:1277`, `:1289`,
  `:1302`), alongside `folders` and `favorite`.
- Because the field is local-only, **output-only edits do not trigger sync**
  (see `CHANGELOG.md`).

Consequences:

- The value never leaves the device and never arrives from another device.
- It is searchable only opt-in — `snp list --search-output` and MCP
  `search_output: true`, both via `SearchFields::with_output()` and
  `for_scoring()` (see [selector.md](selector.md)).
- `snp edit --output` requires explicit targeting for a field sync cannot
  see.

---

## Security properties

- The value is never evaluated, executed, interpolated, or
  shell-expanded. No variable substitution applies to it.
- ANSI / OSC is stripped for **display** but preserved in **storage** and in
  the JSON / CSV exports — what you see sanitized is not what is saved.
- Sanitization is presentation-copy only; it never mutates the stored value.
- The `snp run` output-file branch treats the field as a **path**, and
  enforces containment inside the CWD with symlink and race hardening
  (`src/commands/run_cmd.rs:216-316`).

---

## Related

- [selector.md](selector.md) — `SearchFields`, `searchable_text`, and the
  opt-in search surfaces.
- [library.md](library.md) — the `Snippet` data model.
- [commands/run_cmd.md](commands/run_cmd.md) — the execution path whose
  output-file branch reads the same field.
- [commands/edit_cmd.md](commands/edit_cmd.md) — the edit path that writes it.
- [`docs/PET_COMPATIBILITY.md`](../docs/PET_COMPATIBILITY.md) — the
  upstream contract that `output` doubles as a stdout path.

---

## Key Files

| Path | Role |
|---|---|
| `src/output.rs` | `OutputPresentation`, `sanitize_for_terminal`, `OUTPUT_SEARCH_BUDGET` |
| `src/output.rs:118-176` | `sanitize_for_terminal` — the ANSI / OSC / control-character rules |
| `src/output.rs:49-64` | `summary()` — first line, sanitized, char-truncated |
| `src/output.rs:93-110` | `for_scoring()` — the 512-byte, char-boundary-safe bound |
| `src/library/model.rs:35-36` | `Snippet::output` — the field and its serde aliases |
| `src/ui/mod.rs:519`, `:1178` | TUI detail pane — sanitized `display()` under a `--- Output / Notes ---` separator |
| `src/commands/list_cmd.rs:199-207` | `snp list` default — sanitized `summary(80)` |
| `src/commands/list_cmd.rs:158`, `:184` | `snp list --json` / `--csv` — raw export |
| `src/commands/search_cmd.rs:68` | `snp search` — raw print (not sanitized) |
| `src/commands/edit_cmd.rs:143-290` | Set / replace / clear the field, by filter or by ID |
| `src/main.rs:641-710` | The `--filter`-or-exact-selector gate for output edits |
| `src/commands/run_cmd.rs:208-216`, `:261-323` | The stdout-redirect branch that also reads this field |
| `src/selector.rs:170-177` | The opt-in scoring input |
| `src/sync_commands.rs:425-435` | `ProtoSnippet` — proof `output` is not on the wire |
| `src/sync_commands.rs:1140-1163` | `fingerprint()` — proof `output` is not in the conflict key |
| `src/sync_commands.rs:1277`, `:1289`, `:1302` | Merge — local `output` preserved |

---

## Invariants

- Do not "fix" display by mutating storage — sanitization is
  presentation-copy only.
- `for_scoring()`'s 512-byte budget is a search-path DoS bound for
  pathological inputs; keep it in sync with the `SearchFields` docs in
  [selector.md](selector.md) and
  [`.skills/selector-and-search-parity.md`](../.skills/selector-and-search-parity.md).
- Multi-byte truncation must stay panic-free (covered by the CJK test).
- `output` must never appear in `ProtoSnippet` or in the sync fingerprint.
- The merge must keep the local `output` when the remote side wins.

## Gotchas

- **The field is overloaded.** A value set with `snp edit --output` becomes
  a stdout-redirect *path* on the next `snp run`. This is the single largest
  trap in this module.
- `snp get` cannot show `output` at all — no flag, no JSON key. Do not
  assume `snp get --json` is a full dump of the snippet.
- `snp list --json` / `--csv` emit the **raw** value; `snp list` (default)
  emits a **sanitized 80-char summary**. Machine output is not
  display-sanitized.
- `snp search` prints the **raw** value (`src/commands/search_cmd.rs:68`) —
  the one interactive display path that bypasses `OutputPresentation`.
- `snp new` cannot set the field; only `snp edit` (or import / hand-editing)
  can.
- `summary()` truncates by **characters**, but `for_scoring()` truncates by
  **bytes** with a char-boundary backup — the two bounds are not the same
  unit.
- `display_bounded()`, `is_present()`, `raw()`, and `line_count()` are retained
  with a justified item-level `#[allow(dead_code)]` — changing them has no
  runtime effect today.
- `summary()` keeps only the **first line**; a multi-line note is invisible
  in `snp list` even though it is fully visible in the TUI preview.

# pet_analysis — Pet file reading, field detection, and entry analysis

[← Back to Overview](../overview.md) · [Command index](mod.md) · [import_cmd.md](import_cmd.md) · [doctor_cmd.md](doctor_cmd.md)

`src/commands/pet_analysis.rs` is the **analysis layer** for the Pet snippet format. It is a library module with no CLI surface: it reads and validates Pet TOML files, detects unknown or mistyped fields, analyzes individual entries, and classifies duplicates. `snp import` and `snp doctor --pet-file` both consume it.

It never writes to the source file and never touches the config directory — the only I/O is reading the source.

## Supported source format

A Pet export is a TOML document with a top-level `[[snippets]]` array-of-tables. Each entry uses the canonical snip-it snippet fields plus Pet-specific aliases.

### Known field names

```rust
pub const KNOWN_SNIPPET_FIELDS: &[&str] = &[
    "id", "description", "command", "output", "tag", "tags", "folders",
    "favorite", "created_at", "updated_at", "device_id", "deleted",
    "name", "cmd", "Tag", "Tags", "Description", "Command", "Output",
    "Id", "ID",
];
```

(`pet_analysis.rs:12-35`.) The list is **case-sensitive** and includes both the canonical snake_case names and the Pet export's Capitalised aliases.

### Expected field types

`detect_unknown_fields` type-checks the known fields (`pet_analysis.rs:132-139`):

| Fields | Expected type |
|---|---|
| `tag`, `tags`, `Tag`, `Tags`, `folders` | array |
| `favorite`, `deleted` | boolean |
| `created_at`, `updated_at` | integer |
| `id`, `description`, `command`, `Command`, `cmd`, `output`, `Output`, `device_id`, `name`, `Description` | string |

## Reading the source

```rust
pub const MAX_SOURCE_FILE_BYTES: usize = 16 * 1024 * 1024;

pub fn read_source_file(path: &Path) -> SnipResult<String>;
```

(`pet_analysis.rs:10`, `:37-99`.) The source is **never modified**. The function:

1. stats the path — a missing file gives "Source file not found", a directory gives "Path is a directory", anything else that is not a regular file gives "Unsupported file type" (`:41-64`);
2. reads at most `MAX_SOURCE_FILE_BYTES + 1` bytes through a `BufReader::take` (`:66-72`) so an oversized file is detected without loading it entirely;
3. errors with "Source file too large" if the cap is exceeded (`:74-82`);
4. requires valid UTF-8 (`:84-89`);
5. rejects content containing NUL bytes (`:91-96`).

## Parsing

```rust
pub fn parse_pet_toml(content: &str) -> SnipResult<Snippets>;
```

(`pet_analysis.rs:101-105`.) The content is first passed through `crate::utils::toml_helpers::fix_invalid_toml_escapes`, which repairs invalid TOML escape sequences that Pet exports are known to produce. The result is then parsed into the canonical `Snippets` shape — the same type a native library uses, so no field remapping is needed for the fields that exist.

A parse failure surfaces as a TOML error ("parse pet TOML").

## Field detection

```rust
pub fn detect_unknown_fields(raw_toml: &str) -> Vec<CompatibilityDiagnostic>;
```

(`pet_analysis.rs:107-210`.) This runs on the **raw** TOML — parsed as a generic `toml::Value`, not as `Snippets` — so it can observe fields that `Snippets` would silently drop.

If the raw TOML does not parse, or has no `snippets` array, the function returns an **empty** vector (`:112-120`) rather than an error. That is deliberate: `parse_pet_toml` has already reported the parse failure.

| Code | Severity | Condition |
|---|---|---|
| `W-TYPE-MISMATCH` | Warning | A known field has the wrong TOML type (`:151-163`). Includes a suggestion. |
| `I-FIELD-UNKNOWN` | Info | A field is not in `KNOWN_SNIPPET_FIELDS` and will be ignored (`:166-175`). |
| `W-DESC-MISSING` | Warning | The entry has none of `description`, `Description`, `name` — the description will be empty (`:178-191`). |
| `W-CMD-MISSING` | Warning | The entry has none of `command`, `Command`, `cmd` — the command will be empty (`:193-206`). |

The "missing" checks accept the Pet aliases: an entry with only `Description` or only `Command` does **not** trigger `W-DESC-MISSING` / `W-CMD-MISSING`.

## Per-entry analysis

```rust
pub fn analyze_entry(index: usize, pet: &Snippet) -> Vec<CompatibilityDiagnostic>;
```

(`pet_analysis.rs:212-297`.) Runs on the **parsed** snippet, after `convert_entry` has normalized it (see `import_cmd.md`).

| Code | Severity | Condition |
|---|---|---|
| `W-DESC-EMPTY` | Warning | `description.trim()` is empty (`:216-226`). |
| `E-CMD-EMPTY` | Error | `command.trim()` is empty (`:228-238`). |
| `I-OUTPUT-PRESENT` | Info | The entry has a non-empty `output` field — preserved, not an error (`:240-250`). |
| `I-TAGS-EMPTY` | Info | The entry has no tags (`:252-261`). |
| `W-TAG-EMPTY` | Warning | A tag is empty or whitespace-only (`:262-276`). Includes a suggestion. |
| `I-CHOICE-VARS` | Info | The command contains a choice variable (`:278-294`). |

`E-CMD-EMPTY` is the only error severity, and it is the one that makes `snp import --strict` abort.

## Duplicate classification

```rust
pub fn is_exact_duplicate(a: &Snippet, b: &Snippet) -> bool;
pub fn same_command_different_description(a: &Snippet, b: &Snippet) -> bool;
pub fn same_description_different_command(a: &Snippet, b: &Snippet) -> bool;
```

(`pet_analysis.rs:299-312`.) Exact duplicate = same command **and** same description. The other two predicates cover the near-miss cases.

```rust
pub fn detect_duplicates(
    snippets: &[Snippet],
) -> (Vec<ImportDuplicate>, Vec<CompatibilityDiagnostic>);
```

(`pet_analysis.rs:314-361`.) Compares every pair `(i, j)` with `i < j` inside a **single** list, returning exact duplicates as `ImportDuplicate` records and the near-misses as `W-DUP-CMD` / `W-DUP-DESC` diagnostics. This is the within-source comparison; `snp import --merge` performs the *cross-collection* comparison against the destination library separately (`import_cmd.rs:338-393`).

## Consumers

| Consumer | Which functions it uses |
|---|---|
| `snp import pet` (`import_cmd.rs:1-4`) | `read_source_file`, `parse_pet_toml`, `detect_unknown_fields`, `analyze_entry`, `same_command_different_description`, `same_description_different_command` |
| `snp doctor --pet-file` (`doctor_cmd.rs`) | `read_source_file`, `parse_pet_toml`, plus the doctor's own `detect_unsupported_concepts` |

The import layer additionally calls `detect_duplicates` indirectly through its own merge-time comparison; the standalone `detect_duplicates` helper is the within-source variant.

## Key Files

| Path | Role |
|---|---|
| `src/commands/pet_analysis.rs` | This module: reading, parsing, field detection, entry analysis, duplicate predicates. |
| `src/commands/import_cmd.rs` | The command layer that converts and writes. |
| `src/commands/doctor_cmd.rs` | The other consumer (`--pet-file` analysis). |
| `src/diagnostics.rs` | `CompatibilityDiagnostic`, `DiagnosticSeverity`, `ImportDuplicate`. |
| `src/library/model.rs` | `Snippet` / `Snippets` — the parsed shape and the target shape. |
| `src/utils/toml_helpers.rs` | `fix_invalid_toml_escapes`. |
| `architecture/commands/import_cmd.md` | The command that reuses this module. |

## Invariants

- The source file is opened read-only and never modified.
- The source size is capped at 16 MiB, and the cap is enforced during the read rather than after it.
- The source must be valid UTF-8 and must not contain NUL bytes.
- Unknown fields are **reported, not rejected**: an entry with an unknown key still imports, and the key is dropped.
- Type mismatches on known fields are warnings; they do not abort the import.
- Missing description and command are detected on the **raw** TOML so Pet aliases are honoured.
- `analyze_entry` is read-only: it never mutates the snippet it is given.

## Gotchas

- `detect_unknown_fields` **swallows parse errors** — it returns an empty vector when the raw TOML does not parse (`:112-115`). A Pet file with a syntax error therefore produces "no unknown-field diagnostics", which can look like a clean file. `parse_pet_toml` is the function that surfaces the error.
- `KNOWN_SNIPPET_FIELDS` is case-sensitive. `Tags` is known; `tags` is known; `TAGS` is not, and would be reported as `I-FIELD-UNKNOWN`.
- `W-DESC-MISSING` / `W-CMD-MISSING` accept `name` as a description alias and `cmd`/`Command` as command aliases, but `convert_entry` does **not** copy `name` into `description` or `cmd` into `command` — the alias resolution happens in `Snippet`'s own deserialization, not here.
- `analyze_entry` runs on the **already-normalized** snippet in the import path, so its `W-DESC-EMPTY` reflects the post-conversion state.
- `detect_duplicates` is O(n²) over a single list. It is a within-source check; the import merge path uses an O(n+m) hash-based comparison instead (`import_cmd.rs:311-336`).
- The 16 MiB cap applies to the **Pet source file**. It is a different limit from restore's 10 MiB per-artifact cap (`restore_cmd.rs:189`).

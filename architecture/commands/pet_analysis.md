# pet_analysis — Pet File Reading, Field Detection, Import Analysis

[← Back to Overview](../overview.md)

## Overview

`src/commands/pet_analysis.rs` (642 lines) is the shared,
side-effect-free analysis kernel for pet TOML files, used by both
`import pet` (conversion input) and `doctor --pet-file/--library`
(compatibility preview). It covers bounded file reads, TOML parsing,
structural field/type diagnostics, per-entry analysis, and duplicate
detection. It never converts, writes, or reports — callers own those.

## CLI surface

None directly (no `*Args`, no dispatch). Surfaced through
`PetImportOptions.source` (`import_cmd.rs:37`) and
`DoctorArgs{pet_file, library}` (`doctor_cmd.rs:18`). Size bound:
`MAX_SOURCE_FILE_BYTES = 16 MiB` (`pet_analysis.rs:10`).
`KNOWN_SNIPPET_FIELDS` (`:13-35`) lists canonical + alias keys
(`id/description/command/output/tag/tags/folders/favorite/created_at/
updated_at/device_id/deleted/name/cmd` plus capitalized variants).

## Flow / steps

1. `read_source_file(path)` (`:40`): `metadata` (NotFound → runtime
   error; dir/non-regular → dedicated errors), open + bounded read
   (`limit+1` then length check), UTF-8 decode, NUL rejection. Source
   never modified.
2. `parse_pet_toml(content)` (`:102`): `fix_invalid_toml_escapes`
   then `toml::from_str::<Snippets>`.
3. `detect_unknown_fields(raw_toml)` (`:109`, raw `toml::Value`
   scan): per `[[snippets]]` entry, known-field type expectations
   (`tag/tags/folders→array`, `favorite/deleted→boolean`,
   `created_at/updated_at→integer`, text fields→string) with
   `W-TYPE-MISMATCH` on drift; unknown keys → `I-FIELD-UNKNOWN`;
   missing description/command/name and command/cmd variants →
   `W-DESC-MISSING` / `W-CMD-MISSING`. Unparseable TOML yields no
   structural findings (the parse error itself is the finding).
4. `analyze_entry(index, pet)` (`:213`): empty description →
   `W-DESC-EMPTY`; empty command → `E-CMD-EMPTY` (the sole error);
   present output → `I-OUTPUT-PRESENT`; plus tag/variable findings.
5. Duplicate predicates (`:300-316`): `is_exact_duplicate`
   (description + command), `same_command_different_description`,
   `same_description_different_command`; `detect_duplicates` (`:318`)
   returns `(Vec<ImportDuplicate>, Vec<CompatibilityDiagnostic>)`
   with `W-DUP-CMD` / `W-DUP-DESC` diagnostics.

## Mutation vs read-only

Pure analysis: no gate, no lock, no writes, no ID regeneration, no
normalization. `convert_entry` (import) and `build_pet_report`
(doctor) consume these findings and own all mutation.

## Auto-sync trigger

None. Analysis creates no pending intent and records no usage.

## Error / exit mapping

`read_source_file`/`parse_pet_toml` return `SnipResult` (not-found,
dir, special-file, oversized, non-UTF-8, NUL, TOML parse). The
`detect_*`/`analyze_*` functions return finding vectors, never
errors — absence of findings is an empty vec, and callers decide
fatality (import strict mode vs doctor `--strict` elevation).

## Key invariants

- One analysis implementation serves doctor-preview and
  import-execution: predictions cannot drift between the two.
- Alias tolerance at read (`cmd→command`, `name→description`,
  `Tag/Tags→tags`) with normalization records at write; unknown
  fields are ignored-with-info, never fatal.
- `E-CMD-EMPTY` is the only entry-level error; everything else
  degrades to warning/info (strict modes elevate explicitly).
- The 16 MiB bound is checked pre-allocation via `take(limit+1)`.

## File / line references

- Bounds/fields: `src/commands/pet_analysis.rs:10,13`
- `read_source_file`: `:40`; `parse_pet_toml`: `:102`
- `detect_unknown_fields`: `:109`; `analyze_entry`: `:213`
- Duplicate API: `:300-330+`; callers: `import_cmd.rs`, `doctor_cmd.rs:1-4`

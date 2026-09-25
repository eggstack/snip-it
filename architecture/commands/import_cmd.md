# import_cmd — Pet Snippet File Import

[← Back to Overview](../overview.md)

## Overview

`src/commands/import_cmd.rs` (690 lines) imports pet TOML exports into
native libraries. Analysis primitives (`read_source_file`,
`parse_pet_toml`, `detect_unknown_fields`, `analyze_entry`,
duplicate predicates) live in `pet_analysis.rs`; this module owns
conversion (fresh IDs, timestamps, sync-field clearing), destination
handling (`Create`/`Merge`/`Replace`), duplicate triage, and the
`PetImportReport`. `doctor --pet-file` previews the same diagnostics
without writing.

## CLI surface

`snp import pet` (alias `i`), `main.rs:774-802`:

| Flag | Meaning |
|------|---------|
| `path` (positional) | Source pet TOML file |
| `--library NAME` | Destination (default: sanitized file stem) |
| `--merge` | Upsert into existing (skip exact duplicates) |
| `--replace` | Replace destination (with backup) |
| `--dry-run` | Convert + report, write nothing |
| `--strict` | Empty command aborts instead of skipping |
| `--report <human\|json>` | Report rendering |
| `--report-file PATH` | Also write report to file |

`PetImportOptions{source, destination_library, mode, strict, dry_run,
report_format, report_file}` (`import_cmd.rs:37`);
`ImportMode{Create(default),Merge,Replace}` (`:15`);
`ReportFormat{Human(default),Json}` (`:27`). `--replace` wins over
`--merge`; neither means `Create`.

## Flow / steps

`run_import_pet(options)` (`:167`):

1. `read_source_file` (size/UTF-8/NUL guards) → reject empty →
   `parse_pet_toml` (with `fix_invalid_toml_escapes`) → reject zero
   entries → `detect_unknown_fields` on the raw TOML.
2. `LibraryManager::new + ensure_library_mode`; destination =
   `--library` or `derive_library_name` (lowercase, non-alnum → `-`,
   collapse/trim, fallback `imported`).
3. `PetImportReport::new` + capability census (`toml_format`,
   `snippet_count=N`, `variables`, `choice_variables`,
   `output_fields`, `tags`).
4. Per entry `convert_entry` (`:96`): record normalizations (zero
   timestamps, sync fields, non-empty ID), regenerate UUID, stamp
   `now` for zero timestamps, clear `device_id`/`deleted=false`, then
   `analyze_entry` diagnostics. Empty command → skip + `had_fatal`
   (strict aborts with an error naming the index).
5. Destination: `Create` refuses existing (suggest `--merge/--replace`);
   `Merge` loads-or-defaults, skips exact duplicates via O(n+m)
   lookup tables, records `ImportDuplicate`s; `Replace` backs up then
   overwrites. `dry_run` skips all writes.
6. `save_library` (gated + locked internally) unless dry-run; notify;
   emit human/JSON report (+ file when requested).

## Mutation vs read-only

Mutating unless `--dry-run`: one `save_library` of the destination.
Gating/locking ride the library persistence layer. Dry-run converts
fully in memory and writes nothing (still validates everything).

## Auto-sync trigger

`notify_mutation(MutationKind::Import, MutationOrigin::Import)`
(`import_cmd.rs:473-475`) after a successful commit — the only
`Import`-origin notify in the codebase. Dry-run and failures notify
nothing. (Restore uses the transactional `ensure_pending_for_transaction`
with `Import` kind instead — same sync effect, crash-safe path.)

## Error / exit mapping

`SnipResult<()>`: empty source, zero entries, strict empty-command,
existing destination in `Create` mode, and save failures propagate
(exit 1 family with remediation text). Non-strict empty commands are
skipped and counted, not fatal.

## Key invariants

- Imported snippets always get fresh UUIDs; pet IDs become
  normalization records, never live keys.
- Sync fields are cleared at import (`device_id=""`,
  `deleted=false`) so imports can never resurrect or misattribute.
- Exact duplicates are skipped in `Merge`, preserved as report data;
  near-duplicates (`same_command_different_description`,
  `same_description_different_command`) are reported, not merged.
- Destination-name derivation matches `doctor`'s
  `sanitize_library_name` (single home per module, same rules).

## File / line references

- Modes/options: `src/commands/import_cmd.rs:15,27,37`
- `convert_entry`: `:96`; `run_import_pet`: `:167`; notify: `:473`
- Analysis: `src/commands/pet_analysis.rs`; report: `src/diagnostics.rs`
- Dispatch: `src/main.rs:774-802`

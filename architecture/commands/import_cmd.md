# import_cmd — Pet Snippet File Import

**Source:** `src/commands/import_cmd.rs`

## Purpose

Imports snippets from pet-format TOML files into native snip-it libraries. Handles the full lifecycle: parsing, compatibility analysis, conversion, deduplication, and persistence.

## Import Modes

| Mode | Flag | Behavior |
|------|------|----------|
| Create | (default) | Creates a new library; fails if it already exists |
| Merge | `--merge` | Imports into an existing library, skipping exact duplicates |
| Replace | `--replace` | Replaces the destination library entirely (with backup) |

## Flow

1. **Read source** — `pet_analysis::read_source_file()` reads and validates the pet TOML file (max 16 MB)
2. **Parse** — `pet_analysis::parse_pet_toml()` deserializes the TOML, applying `fix_invalid_toml_escapes()` for compatibility
3. **Analyze** — For each entry:
   - `analyze_entry()` checks variable syntax, field presence, command validity
   - `detect_unknown_fields()` flags non-standard TOML keys
   - Duplicate detection via `is_exact_duplicate()`, `same_command_different_description()`, `same_description_different_command()`
4. **Convert** — `convert_entry()` transforms pet `Snippet` to snip-it `Snippet`:
   - Generates UUID for `id`
   - Sets `created_at` / `updated_at` to current timestamp
   - Preserves command text semantically
   - Records normalization diagnostics
5. **Deduplicate** — In merge mode, exact duplicates (same description + command) are skipped
6. **Persist** — Writes the library file via `LibraryManager::create_library()` or `save_library()`
7. **Report** — Outputs diagnostics as human-readable or JSON

## Duplicate Detection

Three types of duplicates are detected:

| Type | Condition | Severity |
|------|-----------|----------|
| Exact | Same description AND command | Info (skipped in merge) |
| Same command, different description | Same command text, different descriptions | Warning |
| Same description, different command | Same description, different command text | Warning |

## Diagnostic Codes

| Code | Severity | Meaning |
|------|----------|---------|
| `W-MALFORMED-VAR` | Warning | Invalid `<name>` variable syntax |
| `W-DUP-CMD` | Warning | Duplicate command text |
| `W-DUP-DESC` | Warning | Duplicate description |
| `W-DEST-CONFLICT` | Warning | Import destination conflict |
| `W-UNKNOWN-FIELD` | Warning | Unknown TOML field |
| `W-DESC-MISSING` | Warning | Missing description |
| `W-CMD-MISSING` | Warning | Missing command |
| `W-CMD-EMPTY` | Warning | Empty command |
| `W-DESC-EMPTY` | Warning | Empty description |
| `W-TYPE-MISMATCH` | Warning | Field type mismatch |

## Report Formats

| Format | Flag | Destination |
|--------|------|-------------|
| Human | `--report human` (default) | stderr |
| JSON | `--report json` | stdout |
| JSON file | `--report-file <path>` | File |

## Strict Mode

`--strict` aborts the import if any error-severity diagnostic is produced.

## Dry Run

`--dry-run` previews the import without writing any files. Shows what would be created/merged/replaced.

## Integration Points

- **`pet_analysis`** — Core parsing, analysis, and duplicate detection
- **`LibraryManager`** — Library creation, deletion, and persistence
- **`diagnostics`** — `PetImportReport`, `ImportDuplicate`, `NormalizationRecord`
- **`utils/toml_helpers`** — TOML escape handling for cross-format compatibility

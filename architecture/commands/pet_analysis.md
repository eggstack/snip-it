# pet_analysis — Pet Snippet File Analysis

[← Back to Overview](../overview.md)

## Purpose

Helper module for analyzing and importing [pet](https://github.com/knqyf263/pet) snippet files. Used by both `snp doctor` (analysis) and `snp import` (migration).

**File**: `src/commands/pet_analysis.rs`

## Source File Reading

`read_source_file()` validates the pet TOML source:
- Rejects directories, non-regular files
- Enforces 16 MiB size limit (`MAX_SOURCE_FILE_BYTES`)
- Requires valid UTF-8
- Rejects NUL bytes

## Known Pet Fields

```rust
pub const KNOWN_SNIPPET_FIELDS: &[&str] = &[
    "id", "description", "command", "output", "tag", "tags",
    "folders", "favorite", "created_at", "updated_at",
    "device_id", "deleted", "name", "cmd",
    "Tag", "Tags", "Description", "Command", "Output", "Id", "ID",
];
```

Used to detect unrecognized fields during analysis and to distinguish pet-format snippets from snip-it format.

## Analysis Capabilities

- **Field detection**: identifies known vs. unknown pet fields
- **Duplicate detection**: finds snippets with matching descriptions/commands
- **Format validation**: checks TOML structure and required fields
- **Import report**: `PetImportReport` with diagnostics, duplicates, and importable snippet counts

## Integration Points

- `snp doctor` — uses `read_source_file()` and field analysis for compatibility diagnostics
- `snp import` — uses analysis to plan conversion from pet format to snip-it format
- `diagnostics.rs` — `CompatibilityDiagnostic` and `ImportDuplicate` types for structured reporting

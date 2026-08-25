# validate_cmd — Read-Only Data Validation

[← Back to Overview](../overview.md)

## Purpose

`validate` performs comprehensive read-only validation of all snippet libraries and configuration files. It never mutates data.

**File**: `src/commands/validate_cmd.rs`

## Validation Report

Returns a `ValidationReport` containing:

- Schema version, tool version, strict/dry-run flags
- Total library and snippet counts
- Sorted diagnostics with severity, code, and repairability

### Diagnostic Severity

| Level | Meaning |
|-------|---------|
| `Info` | Informational (duplicate IDs resolved, etc.) |
| `Warning` | Non-fatal (missing timestamps, empty commands) |
| `Error` | Fatal (parse failure, corrupt data) |

### Repairability

| Class | Meaning |
|-------|---------|
| `Auto` | Can be fixed by `snp repair` |
| `Manual` | Requires user intervention |
| `Unrepairable` | Data is fundamentally broken |

## Checks Performed

The source uses lettered markers (`a`–`l`) across four validation functions:

### Per-library checks (`validate_library`)

1. **File readability** — cannot read library file
2. **Empty file** — library file exists but contains no data
3. **TOML parse** — parse errors in library file
4. **Duplicate snippet IDs** — same ID appears more than once
5. **Empty snippet IDs** — snippet has an empty ID field
6. **Empty commands / descriptions** — blank command or description
7. **Same-ID divergent content** — same ID with differing content across raw TOML entries
8. **Exact duplicate entries** — identical description + command pair
9. **Corrupt backup artifact** — leftover `.toml.corrupt.bak` file

### Index cross-reference checks (`validate_index`)

10. **Index references missing file** — registered library has no `.toml` file on disk
11. **Orphaned library file** — `.toml` file in `libraries/` not registered in index
12. **Invalid primary library** — primary library file missing, or no primary set when libraries exist

### Usage and permissions checks

13. **Orphaned usage entries** — usage index references a snippet ID not found in any library
14. **Insecure file permissions** — sensitive config files have group/other access bits set (Unix only)

## Output

- Default: human-readable diagnostic list
- `--json`: machine-readable JSON report

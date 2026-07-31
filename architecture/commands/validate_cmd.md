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

1. **Schema version** — current vs. expected
2. **Library index** — duplicates, missing primary, orphaned entries
3. **Snippet IDs** — duplicates, missing UUIDs
4. **Timestamps** — missing, zero, or out-of-range `created_at`/`updated_at`
5. **Tags** — empty tags, duplicates, leading/trailing whitespace
6. **Commands** — empty commands, whitespace-only
7. **Output field** — absolute paths, traversal attempts
8. **Usage index** — orphaned entries for deleted snippets

## Output

- Default: human-readable diagnostic list
- `--json`: machine-readable JSON report

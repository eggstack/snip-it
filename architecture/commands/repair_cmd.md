# repair_cmd — Conservative Data Repair

[← Back to Overview](../overview.md)

## Purpose

`repair` validates configuration and library files, identifies safe repair candidates, and applies fixes only when explicitly requested. Always creates a backup before any mutations.

**File**: `src/commands/repair_cmd.rs`

## Repair Actions

| Action | Category | Safe? | Description |
|--------|----------|-------|-------------|
| `PruneOrphanedUsage` | usage | Yes | Remove usage index entries for deleted snippets |
| `RollbackTransaction` | transaction | Yes | Roll back an interrupted transaction |
| `ResumeCleanup` | transaction | Yes | Resume cleanup for a `CleaningUp` transaction |
| `FinalizeCommittedLocal` | transaction | Yes | Complete pending + cleanup for `CommittedLocal` |
| `CleanupLegacyCommitted` | transaction | Yes | Clean up legacy `Committed` journals |
| `CleanupLegacyRolledBack` | transaction | Yes | Clean up legacy `RolledBack` journals |
| `RemoveTerminalJournal` | transaction | Yes | Remove terminal journal with no artifacts |
| `RemoveOrphanedArtifact` | transaction | Yes | Remove artifact dir with no matching journal |
| `RepairLibraryIndex` | index | No | Fix duplicate/missing primary in library index |
| `RepairSnippetIds` | ids | No | Fix duplicate/missing snippet IDs |
| `RepairTimestamps` | timestamps | No | Fix missing/invalid timestamps |

## Safety Model

- All actions marked `is_safe() = true` are applied automatically with `--apply`
- Unsafe actions are reported but skipped (not applied)
- A backup snapshot is always created before any mutation

## Transaction Repair

The primary use case is recovering from interrupted transactions:
- `Prepared`, `BackupsDurable`, `Committing`, `RollingBack` states → `RollbackTransaction`
- `CleaningUp` state → `ResumeCleanup`
- `CommittedLocal` state → `FinalizeCommittedLocal`

## Output

- Default: lists discovered repair candidates
- `--apply`: executes safe repairs (with backup)
- `--json`: machine-readable JSON report

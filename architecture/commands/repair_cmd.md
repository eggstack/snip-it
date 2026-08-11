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
- Unsafe actions require explicit `--action` flags or interactive confirmation
- A backup snapshot is always created before any mutation
- The `gate_mutation_on_interrupted_transactions()` check runs before repair mutations

## Transaction Repair

The primary use case is recovering from interrupted transactions:
- `Prepared`, `BackupsDurable`, `Committing`, `RollingBack` states → `RollbackTransaction`
- `CleaningUp` state → `ResumeCleanup`
- `CommittedLocal` state → `FinalizeCommittedLocal`

## Output

- Default: lists discovered repair candidates
- `--apply`: executes safe repairs (with backup)
- `--action <category>`: targets specific repair category

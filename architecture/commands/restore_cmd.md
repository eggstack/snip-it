# restore_cmd — Restore from Backup

[← Back to Overview](../overview.md)

## Purpose

`restore` restores snippet libraries and configuration from a backup snapshot created by `snp backup`.

**File**: `src/commands/restore_cmd.rs`

## Restore Modes

| Mode | Behavior |
|------|----------|
| `DryRun` | Show what would be restored without writing |
| `Merge` | Restore missing files, skip existing |
| `Replace` | Overwrite existing files with backup contents |

## Destination Permission Policy

The `DestinationClass` enum controls file permissions after restore:

| Class | When | Unix Permission |
|-------|------|-----------------|
| `NewPrivate` | File didn't exist before restore | `0o600` |
| `ExistingPreserved` | File existed, being overwritten | Original mode |
| `Restore` | Restoring from backup | Original captured mode |

## Backup Manifest

Reads `BackupManifest` from the backup directory (created by `backup_cmd`):

```rust
pub struct BackupManifest {
    pub entries: Vec<BackupManifestEntry>,
    pub created_at: String,
    pub version: String,
}
```

Each entry has a `BackupEntryKind`: `Library`, `Index`, `Usage`, or `SyncConfig`.

## Path Validation

`BackupRelativePath` enforces security:
- Rejects absolute paths, parent traversal (`../`), NUL bytes
- Rejects Windows drive letters and UNC paths
- Rejects reserved Windows device names (CON, NUL, etc.)

## Integrity Verification

- SHA-256 checksums verified before restore
- Permission class applied per destination file
- Atomic writes via `atomic_replace()` with fsync

## Data Flow

```
restore run() → read manifest → verify checksums → classify destinations → atomic_replace per file
```

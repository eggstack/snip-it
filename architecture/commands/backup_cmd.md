# backup_cmd — Secret-Free Backup Snapshot

[← Back to Overview](../overview.md)

## Purpose

`backup` creates a portable, secret-free backup snapshot of snippet libraries, metadata, and usage data. Sync credentials and API keys are excluded.

**File**: `src/commands/backup_cmd.rs`

## Backup Format

Currently supports directory layout (`BackupFormat::Directory`).

## Backup Entry Kinds

| Kind | Description |
|------|-------------|
| `Library` | Individual library TOML files |
| `Index` | Library metadata (`libraries.toml`) |
| `Usage` | Local usage counters (`usage.toml`) |
| `SyncConfig` | Sync settings (without API keys) |

## Backup Manifest

Each backup includes a `BackupManifest`:

```rust
pub struct BackupManifest {
    pub schema: u32,
    pub created_at_unix_ms: i64,
    pub snip_it_version: String,
    pub layout: String,
    pub files: Vec<BackupManifestEntry>,
}
```

Each entry records:
- `kind` — entry type
- `path` — relative path within backup
- `size` — file size in bytes
- `sha256` — content checksum

## Path Validation

`BackupRelativePath` enforces:
- Relative paths only (no absolute)
- No parent traversal (`../`)
- No NUL bytes
- No Windows drive letters or UNC paths
- No reserved Windows device names

## Security Properties

- API keys are **never** included in backups
- Sync tokens are excluded
- All paths validated against traversal attacks
- SHA-256 checksums for integrity verification

## Output

- Creates timestamped directory under `~/.config/snp/backups/`
- Prints manifest summary and backup location

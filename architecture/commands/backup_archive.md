# backup_archive — backup format, manifest, and integrity layer

[← Back to Overview](../overview.md) · [Command index](mod.md) · [backup_cmd.md](backup_cmd.md) · [restore_cmd.md](restore_cmd.md)

`src/commands/backup_archive.rs` is the **shared format layer** between `snp backup` and `snp restore`. It owns the on-disk backup layout, the manifest schema, path validation, hashing, secret redaction, and the atomic write. Neither command defines these independently: `backup_cmd.rs` produces them, `restore_cmd.rs` consumes and re-validates them.

It is a **pure library module**: no CLI surface, no state directory, no locks, no journal. Its only external dependency for atomicity is `crate::utils::atomic::write_private_atomic` (`backup_archive.rs:318`).

## Format types

### `BackupFormat`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BackupFormat {
    Directory,
}
```

(`backup_archive.rs:13-17`.) There is exactly one variant. The enum exists so the CLI can accept `--format` without a breaking change; `backup_cmd::run()` discards the parsed value (`src/commands/backup_cmd.rs:49`).

### `BackupEntryKind`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupEntryKind {
    Library,
    Index,
    Usage,
    SyncConfig,
}
```

(`backup_archive.rs:19-27`.) Serialized as `library`, `index`, `usage`, `sync_config`. Because the enum is a closed serde enum, a manifest naming an unrecognized kind **fails to deserialize** — an unknown kind is a hard error, not a skipped entry.

| Kind | Backup-relative path | Destination after restore | Included by |
|---|---|---|---|
| `library` | `libraries/<name>.toml` | `<config>/libraries/<name>.toml` | always |
| `index` | `libraries.toml` | `<config>/libraries.toml` | always |
| `usage` | `usage.toml` | `<config>/usage.toml` | `--include-usage` |
| `sync_config` | `sync.toml` | `<config>/sync.toml` | `--include-sync-state` |

### `BackupManifest` / `BackupManifestEntry`

```rust
pub struct BackupManifestEntry {
    pub path: String,
    pub kind: BackupEntryKind,
    pub size: u64,
    pub sha256: String,
}

pub struct BackupManifest {
    pub schema: u32,
    pub created_at_unix_ms: i64,
    pub snip_it_version: String,
    pub layout: String,
    pub files: Vec<BackupManifestEntry>,
}
```

(`backup_archive.rs:335-355`.) The manifest is serialized with `toml::to_string_pretty` and written to `<backup>/manifest.toml` via `write_private_atomic` (`backup_archive.rs:315-318`), so the manifest itself is written with private permissions and atomically.

Restore accepts only `schema == 1` and `layout == "directory"` (`restore_cmd.rs:183-186`, `restore_cmd.rs:241-254`).

## Path validation

### `BackupRelativePath`

```rust
pub struct BackupRelativePath(PathBuf);

impl BackupRelativePath {
    pub fn parse(input: &str, expected_kind: BackupEntryKind) -> SnipResult<Self>;
    pub fn resolve_source(&self, backup_root: &Path) -> PathBuf;
}
```

(`backup_archive.rs:40-197`.) `parse` rejects:

- empty paths (`backup_archive.rs:57-60`);
- absolute paths;
- `..` parent traversal;
- NUL bytes;
- reserved Windows device names — `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9` (`backup_archive.rs:49-53`).

`restore` re-runs this validation on every manifest entry (`restore_cmd.rs:973-976`, `restore_cmd.rs:289-292`), so a hand-edited manifest cannot escape the backup root.

### `validate_canonical_containment`

```rust
pub(crate) fn validate_canonical_containment(
    path: &Path,
    root: &Path,
    label: &str,
) -> SnipResult<PathBuf>;
```

(`backup_archive.rs:256-280`.) Canonicalizes both the candidate path and the root and requires the canonical path to `starts_with` the canonical root. Used by `backup_cmd` for every source file so a symlinked library file cannot pull content from outside the config directory into a backup.

## File helpers

### `read_for_snapshot`

```rust
pub(crate) fn read_for_snapshot(path: &Path, label: &str) -> SnipResult<Vec<u8>>;
```

(`backup_archive.rs:199-223`.) Uses `fs::symlink_metadata` (does **not** follow the link) and rejects:

- symlinks — "Refusing to back up symlink … (could point outside config)";
- anything that is not a regular file — "Expected regular file, got …".

Only after those checks does it call `fs::read`.

### `sha256_hex`

```rust
pub(crate) fn sha256_hex(bytes: &[u8]) -> String;
```

(`backup_archive.rs:225-236`.) Lowercase hex SHA-256 over the exact stored bytes. This is the **whole-file integrity digest**; it is unrelated to the deterministic snippet identity hash (`legacy-<sha256 hex>` over content + occurrence index, see `docs/IDENTITY_CONTRACT.md`).

### `validate_toml_content`

```rust
pub(crate) fn validate_toml_content(bytes: &[u8], label: &str) -> SnipResult<()>;
```

(`backup_archive.rs:282-291`.) Rejects non-UTF-8 and non-TOML content. `backup_cmd` applies it to every library file and to `libraries.toml` (`src/commands/backup_cmd.rs:144-150`).

### `read_library_generation`

```rust
pub(crate) fn read_library_generation(config_dir: &Path) -> SnipResult<u64>;
```

(`backup_archive.rs:238-254`.) Reads `<config>/libraries.toml`, parses it as `LibraryConfig`, and returns `config.generation`. Returns `0` when the index does not exist. `backup_cmd` samples this before and after the capture and **aborts** if it changed (`src/commands/backup_cmd.rs:79`, `:134-143`).

## Secret redaction

```rust
pub(crate) fn redact_sync_config(content: &str) -> SnipResult<String>;
```

(`backup_archive.rs:357-374`.) Line-based redaction applied to `sync.toml` before it is hashed, manifested, or written:

- a line whose trimmed, lowercased text contains `api_key`, `apikey`, or `api-key` **and** contains `=` is replaced by `<prefix> = "<redacted>"`;
- all other lines are copied verbatim.

The redaction is **not schema-aware**. It catches the token in any position (including inside a nested table, where the prefix is everything before the `=`), but a credential stored under a different key name is copied unchanged.

## Atomic write

```rust
pub(crate) fn atomic_write_backup(
    staging_dir: &Path,
    final_dir: &Path,
    files: &[(&str, &str, &[u8])],
    manifest: &BackupManifest,
) -> SnipResult<()>;
```

(`backup_archive.rs:293-333`.)

1. `create_dir_all(staging_dir)`.
2. Write each file into the staging tree, creating parent directories as needed.
3. Serialize the manifest and write `manifest.toml` with `crate::utils::atomic::write_private_atomic` — private permissions, atomic replace.
4. `create_dir_all(final_dir.parent())`.
5. `fs::rename(staging_dir, final_dir)`.

If any step fails, the staging directory is removed and the error is returned (`backup_archive.rs:329-331`). The **final directory never exists in a partial state** — it only appears via the rename in step 5.

The staging path is derived by the caller with `Path::with_extension("<ext>.staging.<uuid>")` (`src/commands/backup_cmd.rs:65-72`), which *replaces* the last extension rather than appending to the file name.

## Manifest determinism

`backup_cmd` sorts `manifest.files` by `path` before writing (`src/commands/backup_cmd.rs:223`), so two backups of identical content produce byte-identical manifests (modulo `created_at_unix_ms`). The module's own test suite pins this (`backup_archive.rs:559`, `test_deterministic_manifest_ordering`).

## How restore consumes this layer

| Restore step | Source | Purpose |
|---|---|---|
| Load manifest | `restore_cmd.rs:542-588` (`load_manifest`) | Reads `manifest.toml`, falling back to `manifest.json`. |
| Contract validation | `restore_cmd.rs:239-353` | Schema, layout, entry cardinality, destination uniqueness, SHA-256 shape, size ceiling. |
| Path validation | `restore_cmd.rs:973-976` | Re-runs `BackupRelativePath::parse` per entry. |
| Artifact checks | `restore_cmd.rs:978-1057` | Existence, symlink rejection, regular-file check, size ceiling (10 MiB), manifest-size match, duplicate snippet-ID check for libraries. |
| Checksum verification | `restore_cmd.rs:1059-1074` | Recomputes SHA-256 and compares against the manifest. |
| Semantic validation | `restore_cmd.rs:381-514` | Parses the index and enforces index↔library consistency, at most one primary, and (in replace mode) full index coverage. |

The **10 MiB per-file ceiling** (`MAX_RESTORE_SOURCE_SIZE`, `restore_cmd.rs:189`) is enforced twice: on the manifest-declared size (`restore_cmd.rs:341-349`) and on the actual artifact size (`restore_cmd.rs:1019-1029`).

## Key Files

| Path | Role |
|---|---|
| `src/commands/backup_archive.rs` | This module: format types, path validation, hashing, redaction, atomic write. |
| `src/commands/backup_cmd.rs` | Producer — snapshot collection and manifest assembly. |
| `src/commands/restore_cmd.rs` | Consumer — manifest contract, checksum, semantic validation. |
| `src/utils/atomic.rs` | `write_private_atomic`, used for the manifest write. |
| `src/library/model.rs` | `LibraryConfig` / `LibraryMeta` — the index schema the manifest carries. |
| `architecture/commands/backup_cmd.md` | Producer-side reference. |
| `architecture/commands/restore_cmd.md` | Consumer-side reference. |

## Invariants

- The manifest is the single source of truth: it names every stored artifact, its kind, exact size, and SHA-256.
- `schema` is `1` and `layout` is `"directory"`; anything else is rejected by restore.
- Unknown entry kinds fail deserialization rather than being silently skipped.
- Backup-relative paths can never be absolute, traverse upward, contain NUL, or name a reserved Windows device.
- Symlinks and non-regular files are never captured.
- The API key is redacted **before** the bytes are hashed, so the manifest digest covers the redacted content.
- The final backup directory is only ever created by an atomic rename of a complete staging directory.

## Gotchas

- `BackupFormat` has a single variant and the `--format` CLI value is discarded — there is no tarball or zip archive today.
- The manifest is written with `toml::to_string_pretty`; the save path for *snippet libraries* likewise does not post-process TOML (see `.skills/persistence-and-toml-fidelity.md`), but the manifest is machine-generated rather than user-authored, so golden-corpus fidelity does not apply to it.
- `redact_sync_config` is line-based. It is not a guarantee that *no* secret is present — only that `api_key`/`apikey`/`api-key` tokens are masked.
- `read_library_generation` returns `0` for a missing index. Two snapshots of a config with no index therefore compare equal by generation alone; the local-data lock is what actually guarantees consistency.
- `atomic_write_backup` writes file bodies with plain `fs::write` (not the private-atomic helper). Only `manifest.toml` gets `write_private_atomic`.

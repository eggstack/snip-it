# snp backup

[← Back to Overview](../overview.md) · [Command index](mod.md) · [backup_archive.md](backup_archive.md) · [restore_cmd.md](restore_cmd.md)

`snp backup` captures a **point-in-time snapshot** of the local configuration directory into a self-contained backup directory. It is the write-side half of the backup/restore pair: `snp restore` consumes what this command produces.

The command is **read-only with respect to snippet data** — it never modifies a library, the index, or the usage index — but it does **create files** (the backup directory) and it **takes the local-data lock** for the duration of the capture. It does **not** create a transaction journal: a backup is not a mutation of library state, so no rollback is required if the process dies mid-capture (the staging directory is discarded instead).

It does **not** cover: encrypted keychain material, the `snip-sync` server, remote sync state, or the auto-sync pending marker.

## CLI arguments

`snp backup` and `snp data backup` share one argument struct (`src/commands/backup_cmd.rs:22-39`):

```rust
pub struct BackupArgs {
    /// Output directory (default: ~/.config/snp/backups/{timestamp}/)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Include usage metadata in backup
    #[arg(long)]
    pub include_usage: bool,

    /// Include sync.toml in backup (API key redacted)
    #[arg(long)]
    pub include_sync_state: bool,
    /// Backup format
    #[arg(long, value_enum, default_value = "directory")]
    pub format: BackupFormat,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}
```

| Flag | Type | Default | Effect |
|---|---|---|---|
| `-o`, `--output <PATH>` | `Option<PathBuf>` | `None` | Destination backup directory. When `None`, the command writes to `<config>/backups/<YYYYMMDD_HHMMSS>/` (`src/commands/backup_cmd.rs:61-64`). |
| `--include-usage` | flag | off | Also snapshot `usage.toml` (usage/frequency metadata) into the backup. |
| `--include-sync-state` | flag | off | Also snapshot `sync.toml`. The API key is **redacted before hashing and writing** (`src/commands/backup_cmd.rs:208-219`, `backup_archive.rs:358-374`). |
| `--format <FORMAT>` | `BackupFormat` | `directory` | Currently the only variant is `Directory` (`backup_archive.rs:15-17`). **The argument is accepted but ignored** — `run()` discards it with `let _ = format;` (`src/commands/backup_cmd.rs:49`). |
| `--json` | flag | off | Emit a machine-readable JSON summary on **stdout**. The human summary always goes to **stderr**. |

The `data` spelling is an alias, not a separate command: `DataCommands::Backup(BackupArgs)` in `src/main.rs:260-261` dispatches to the same `handle_backup` (`src/main.rs:411-420`, `src/main.rs:829-831`).

## Control flow

`run()` (`src/commands/backup_cmd.rs:42-260`) executes in this order:

1. **Config directory must exist.** A missing config directory is a hard error (`src/commands/backup_cmd.rs:51-56`) — backup does not create the config tree.
2. **Canonicalize the config directory** (`:57-59`). This becomes the containment root for every source file.
3. **Compute the timestamp and destination** (`:60-64`): `chrono::Utc::now().format("%Y%m%d_%H%M%S")`.
4. **Build the staging directory** (`:65-72`): `final_backup_dir.with_extension("<existing-extension>.staging.<uuid>")`. Because `with_extension` *replaces* the last extension, `…/20260101_120000` becomes `…/20260101_120000.staging.<uuid>`, while `…/backup.tar.gz` becomes `…/backup.tar.staging.<uuid>`. The final directory only appears via an atomic rename at the end.
5. **Acquire the local-data lock** (`:77-78`):

   ```rust
   let state_dir = crate::local_data::transaction_dir();
   let _local_lock = crate::local_data::acquire_local_data_lock(&state_dir)?;
   ```

   The lock is taken for the whole capture so the snapshot is either the complete before-state or the complete after-state, never a mix. `acquire_local_data_lock` takes the **`.transaction` directory** (not the config root), so the lock file is `<config>/.transaction/local-data.lock` (`src/local_data.rs:111-115`).
6. **Read the library generation** before capture (`:79`) and again after (`:134`). `read_library_generation` parses `libraries.toml` and returns `config.generation`, or `0` when the index does not exist (`backup_archive.rs:239-254`). **If the generation changed, the command aborts** with "Library generation changed during snapshot" (`:135-143`) — this is the second consistency barrier behind the lock.
7. **Collect the snapshot set** into memory:
   - every `*.toml` in `<config>/libraries/` that does not start with `.` (`:88-105`);
   - `<config>/libraries.toml` if present (`:106-111`);
   - `<config>/usage.toml` only when `--include-usage` (`:112-119`);
   - `<config>/sync.toml` only when `--include-sync-state` (`:120-133`).
8. **Validate every collected file**: canonical containment under the config root (`:101`, `:108`, `:115`, `:123` — `backup_archive.rs:257-280`), symlink/non-regular-file rejection (`backup_archive.rs:200-223`), and TOML well-formedness for library files (`:144-150`) and for the index (`:151-155`) (`backup_archive.rs:283-291`).
9. **Build the manifest entries** (`:156-222`) and sort the file list by path (`:223`) so the manifest is deterministic.
10. **Write atomically** (`:228`): `atomic_write_backup` writes every file plus `manifest.toml` into the staging directory and then renames staging → final (`backup_archive.rs:295-333`). On any error the staging directory is removed (`:329-331`), so a failed backup never leaves a half-written final directory.
11. **Emit the report** (`:229-258`).

### What is included and excluded by default

| Path | Included by default | Flag to include |
|---|---|---|
| `<config>/libraries/*.toml` | yes | — |
| `<config>/libraries.toml` (index) | yes | — |
| `<config>/usage.toml` | no | `--include-usage` |
| `<config>/sync.toml` | no | `--include-sync-state` (API key redacted) |
| Keychain / OS credential store | never | — |
| `<config>/.transaction/` (journals, locks, artifacts) | never | — |
| Auto-sync pending marker | never | — |

### Secret redaction

When `--include-sync-state` is used, `redact_sync_config` (`backup_archive.rs:358-374`) rewrites the file **line by line** before it is hashed or written. Any line whose trimmed, lowercased form contains `api_key`, `apikey`, or `api-key` **and** contains an `=` is replaced with `<prefix> = "<redacted>"`. Every other line is copied verbatim.

The redaction is **line-based, not schema-based**: a key stored in a nested table is still redacted (the prefix is everything before the `=`), but a secret under a name that does not contain one of the three tokens is copied as-is. The redacted text is what the manifest's `sha256` covers, so restore verifies the redacted bytes — the original key is never written to disk.

## Manifest format

The manifest is written to `<backup>/manifest.toml` and is the authoritative description of the snapshot.

```rust
pub struct BackupManifest {
    pub schema: u32,
    pub created_at_unix_ms: i64,
    pub snip_it_version: String,
    pub layout: String,
    pub files: Vec<BackupManifestEntry>,
}

pub struct BackupManifestEntry {
    pub path: String,
    pub kind: BackupEntryKind,
    pub size: u64,
    pub sha256: String,
}
```

(`backup_archive.rs:340-355`.)

| Field | Meaning |
|---|---|
| `schema` | Always `1` for this layout. `restore` rejects anything else (`restore_cmd.rs:241-246`). |
| `created_at_unix_ms` | Wall-clock creation time of the snapshot. |
| `snip_it_version` | The `snp` version that produced the backup (`crate::diagnostics::version()`). |
| `layout` | Always `"directory"`. `restore` rejects anything else (`restore_cmd.rs:249-254`). |
| `files[].path` | Backup-relative path, e.g. `libraries/work.toml`. |
| `files[].kind` | `library`, `index`, `usage`, or `sync_config` (`BackupEntryKind`, `backup_archive.rs:22-27`). Unknown kinds fail deserialization. |
| `files[].size` | Exact byte length of the stored artifact. |
| `files[].sha256` | Lowercase hex SHA-256 of the stored bytes (`sha256_hex`, `backup_archive.rs:226-236`). |

### SHA-256 integrity

`sha256_hex` (`backup_archive.rs:226-236`) is a plain SHA-256 over the exact bytes that are stored. It is **not** the snippet-identity hash — snippet IDs are deterministic `legacy-<sha256 hex>` values over content plus occurrence index (see `docs/IDENTITY_CONTRACT.md`); the manifest digest is a separate, whole-file integrity digest used only by backup/restore.

## Directory layout

```
<backup>/
  manifest.toml            # authoritative manifest (schema 1, layout "directory")
  libraries/
    <name>.toml           # one file per library
  libraries.toml          # index
  usage.toml              # only with --include-usage
  sync.toml               # only with --include-sync-state (redacted)
```

Paths inside the manifest are validated against absolute paths, `..` traversal, NUL bytes, and reserved Windows device names — by `restore`'s own `validate_backup_path` (`restore_cmd.rs:604-725`). The shared `BackupRelativePath::parse` (`backup_archive.rs:40-197`) implements the same rules but is **not called** by either command — it is only `pub use`-re-exported by `backup_cmd.rs:5-7`.

### Archive destinations

The destination is always a **directory**. `--output` names the final directory; the intermediate `<dest>.staging.<uuid>` directory is created and removed by `atomic_write_backup`. There is currently **no single-file tarball/zip archive format** — `BackupFormat` has exactly one variant (`Directory`) and the `--format` argument is discarded by `run()`.

## Output and exit codes

| Mode | Stream | Content |
|---|---|---|
| `--json` | stdout | `{"backup_dir", "schema", "version", "file_count", "total_bytes"}` (`src/commands/backup_cmd.rs:229-242`). |
| default | stderr | `Backup created: <dir>`, schema, version, file count, total size, then one line per manifest entry: `<kind> (<path>): <size> bytes, sha256:<first 16 hex chars>` (`:243-257`). The digest is **truncated to 16 hex characters** in the human output; the full digest is in the manifest. |

`snp backup` returns `CliOutcome::Success` (exit `0`) on success (`src/main.rs:411-420`). Any failure surfaces as a `SnipError`, which the CLI maps to the canonical exit codes in `src/outcome.rs:46-70` — in practice `1` (`GENERAL_ERROR`) for I/O and runtime failures, `2` (`USAGE_ERROR`) for clap argument errors.

## Interaction with the transaction journal and locks

- **Transaction journal:** none. `snp backup` never calls `begin_transaction`, so it creates no journal and needs no rollback.
- **Mutation gate:** not called. The gate (`gate_mutation_on_interrupted_transactions`) protects *mutations*; backup does not mutate library state.
- **Local-data lock:** acquired for the whole capture (`src/commands/backup_cmd.rs:77-78`). This is the same lock `save_library` takes (`src/library/persistence.rs:233`), which is what makes the snapshot atomic with respect to library writes.
- **Lock hierarchy position:** the local-data lock only. No transaction lock is taken, because there is no journal to protect.

## Key Files

| Path | Role |
|---|---|
| `src/commands/backup_cmd.rs` | CLI args, snapshot collection, manifest build, report emission. |
| `src/commands/backup_archive.rs` | Manifest types, path validation, hashing, redaction, atomic write. |
| `src/commands/restore_cmd.rs` | The consumer of this manifest (validation + install). |
| `src/local_data.rs` | `acquire_local_data_lock`, `transaction_dir`. |
| `src/library/persistence.rs` | `save_library` — takes the same local-data lock. |
| `architecture/persistence.md` | Journal, lock hierarchy, durability classes. |
| `architecture/commands/restore_cmd.md` | Restore-side contract for the same manifest. |

## Invariants

- The final backup directory appears only via an atomic rename of a fully-written staging directory; a failed backup leaves no partial final directory (`backup_archive.rs:295-333`).
- The manifest always lists every file present in the backup, with the exact size and SHA-256 of the stored bytes.
- The snapshot is consistent: the local-data lock is held across capture, and the library generation is compared before and after (`:79`, `:134-143`).
- Symlinks and non-regular files are never backed up (`backup_archive.rs:200-223`).
- The API key is redacted before the bytes are hashed, written, or manifested.

## Gotchas

- `--format` is a **no-op**. Only `directory` exists, and `run()` discards the parsed value (`src/commands/backup_cmd.rs:49`). Do not document it as selecting a layout.
- `usage.toml` and `sync.toml` are **opt-in**. A backup without those flags will restore without them.
- `sync.toml` in a backup is **redacted**; restoring it over a live `sync.toml` replaces the real API key with `<redacted>`.
- The backup includes the `libraries.toml` index, so restoring into a directory with a different index can rewrite library registration — see `restore_cmd.md` for the merge/replace semantics.
- The local-data lock has a 30 s acquisition deadline (`src/local_data.rs:128`) and a bounded ladder for empty/malformed lock records (`src/local_data.rs:209-257`). A backup that cannot take the lock fails with an error rather than hanging.

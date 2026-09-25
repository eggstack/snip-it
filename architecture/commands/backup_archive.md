# backup_archive — Backup Archive I/O, Manifest, Integrity

[← Back to Overview](../overview.md)

## Overview

`src/commands/backup_archive.rs` (677 lines) is the independently
testable I/O kernel behind `snp backup` / `snp restore`: manifest
types, backup-relative path validation, symlink-safe snapshot reads,
SHA-256 integrity, TOML/content guards, canonical-containment checks,
atomic staging publish, and sync-secret redaction. It owns no CLI,
takes no locks, and schedules nothing — `backup_cmd::run` owns
orchestration.

## CLI surface

None directly. `BackupFormat{Directory}` (`backup_archive.rs:15`) is
exposed as the `--format` value enum (re-exported through `backup_cmd`
for CLI-schema compatibility). All other types are library-internal:
`BackupEntryKind{Library,Index,Usage,SyncConfig}` (`:22`, snake_case
serde, `Display`), `BackupManifestEntry{path,kind,size,sha256}`
(`:340`), `BackupManifest{schema,created_at_unix_ms,snip_it_version,
layout,files}` (`:349`), `BackupRelativePath` (`:46`).

## Flow / steps

1. **Path validation** — `BackupRelativePath::parse(input,
   expected_kind)` (`:56`): rejects empty, NUL, Windows drive/UNC
   (all platforms), absolute/prefix/parent components; then
   kind-specific rules — Library must be a flat `*.toml` filename
   without reserved device names (`CON PRN AUX NUL COM1-9 LPT1-9`,
   case-insensitive); Index must be exactly `libraries.toml`; Usage
   exactly `usage.toml`; SyncConfig exactly `sync.toml`.
   `resolve_source(root)` joins without re-validation (`:194`).
2. **Snapshot reads** — `read_for_snapshot` (`:200`):
   `symlink_metadata` (never follows), reject symlinks and
   non-regular files, then `fs::read`.
3. **Integrity** — `sha256_hex` (`:226`, 64-char lowercase);
   `read_library_generation` (`:239`, missing index → 0, used as the
   concurrent-mutation guard); `validate_canonical_containment`
   (`:257`, both sides canonicalized, prefix check);
   `validate_toml_content` (`:283`, UTF-8 then `toml::Value` parse).
4. **Publish** — `atomic_write_backup(staging, final, files, manifest)`
   (`:295`): create staging, write each file (+ parents), write
   `manifest.toml` via `write_private_atomic`, create final parent,
   `rename(staging, final)`; any failure removes staging.
5. **Redaction** — `redact_sync_config` (`:358`): line-oriented; any
   line whose left-hand side mentions `api_key|apikey|api-key`
   (case-insensitive) becomes `<name> = "<redacted>"`; all other lines
   (tables, comments, ordering) pass through byte-identical.

## Mutation vs read-only

Filesystem-writing but data-preserving: writes only the staging/final
backup directories. Never modifies live config, never gates, never
locks (callers hold the local-data lock), never notifies.

## Auto-sync trigger

None. Archive I/O is invisible to auto-sync by design.

## Error / exit mapping

All functions return `SnipResult`; violations are `runtime_error`
(empty path, NUL, drive/UNC, absolute, traversal, kind mismatch,
reserved name, symlink, non-regular, non-UTF-8, invalid TOML,
containment escape) or `io_error` (stat/read/write/rename with path
context). Unknown manifest `kind` values fail deserialization —
never accepted for restore.

## Key invariants

- Defense in depth: manifest-string validation at the edge,
  canonical-containment at capture, `symlink_metadata` at read, size +
  SHA-256 at restore.
- Symlinks are never followed for backup content (both directions).
- Redaction is conservative-substring on the key token; verify with
  the four `redact_sync_config` unit tests (plain/table/indented).
- Manifest file order is the caller's responsibility (backup sorts by
  path); unknown `kind`s reject, never ignore.
- 20+ unit tests (`:376-677`) pin symlink/dir rejection, UTF-8/TOML
  guards, containment, staging cleanup, and generation reads.

## File / line references

- Format/kinds: `src/commands/backup_archive.rs:15,22`
- `BackupRelativePath::parse`: `:56`; `resolve_source`: `:194`
- `read_for_snapshot`: `:200`; `sha256_hex`: `:226`
- Generation/containment/TOML: `:239,257,283`
- `atomic_write_backup`: `:295`; manifest types: `:340,349`
- `redact_sync_config`: `:358`; tests: `:376-677`

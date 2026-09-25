# backup_cmd — Secret-Free Backup Snapshot

[← Back to Overview](../overview.md)

## Overview

`src/commands/backup_cmd.rs` (260 lines) orchestrates `snp backup`:
consistent snapshot capture under the local-data lock, library
generation guards against concurrent mutation, per-file SHA-256
manifesting, and atomic staging publish. Archive primitives live in
`backup_archive.rs` (see `backup_archive.md`); `run` owns locking,
guards, and reporting. Never touches snippet semantics — bytes are
copied verbatim (modulo sync-secret redaction).

## CLI surface

`BackupArgs` (`backup_cmd.rs:22`), `snp backup` and `snp data backup`
(alias `b`) via `handle_backup` (`main.rs:410`):

| Flag | Meaning |
|------|---------|
| `-o/--output DIR` | Destination (default `<config>/backups/<YYYYMMDD_HHMMSS>/`) |
| `--include-usage` | Include `usage.toml` (default: excluded) |
| `--include-sync-state` | Include redacted `sync.toml` (default: excluded) |
| `--format <directory>` | `BackupFormat` (only `Directory` today) |
| `--json` | Machine report to stdout (else human report to stderr) |

## Flow / steps

`run(output, include_usage, include_sync_state, format, json)`
(`backup_cmd.rs:42`):

1. Require the config dir to exist; canonicalize it (traversal anchor).
   Derive `final_backup_dir` + a sibling UUID staging dir.
2. Acquire the local-data lock for the whole capture (`:78`) — the
   snapshot is either the complete before- or after-state, never mixed.
3. `generation_before = read_library_generation` (`libraries.toml`
   `generation`; missing → 0).
4. Collect: every non-hidden `libraries/*.toml` (canonical-containment
   + `read_for_snapshot` symlink/regular-file checks), plus
   `libraries.toml`, optional `usage.toml`, optional `sync.toml`
   (read as text for redaction).
5. `generation_after`; mismatch → `runtime_error` directing a retry
   (`:135-143`).
6. `validate_toml_content` over libraries + index; build `BackupManifest{
   schema:1, created_at_unix_ms, snip_it_version, layout:"directory",
   files}` with `sha256_hex` + sizes, paths sorted (`:223`).
7. Redact sync config (`api_key|apikey|api-key → "<redacted>"`),
   then `atomic_write_backup(staging, final, files, manifest)`.
8. Report: JSON (`backup_dir/schema/version/file_count/total_bytes`)
   or human lines with per-file kind/path/size/sha-prefix.

## Mutation vs read-only

Read-only with respect to live data: takes the local-data lock and
creates a new backup directory, but never writes libraries, index,
usage, or sync settings. Needs no transaction gate (no library
mutation) — the generation guard + lock provide consistency instead.

## Auto-sync trigger

None. No `notify_mutation`, no runtime, no sync. Backups (including
restored-later content, which notifies on restore) leave no pending
intent themselves.

## Error / exit mapping

`SnipResult<()>` → `Success` via `handle_backup`. Errors: missing
config dir, lock acquisition failure, containment/symlink/type
violations, invalid TOML bytes, generation race (retryable),
staging/rename I/O. JSON serialization failure is a runtime error.

## Key invariants

- Lock-then-generate-check-then-validate-then-publish ordering is
  load-bearing; do not reorder capture after the second generation
  read.
- Secrets never land in backups: sync inclusion is opt-in *and*
  redacted; API keys stay in the OS keychain.
- Manifest entries sort by path (deterministic, diffable).
- Directory layout only — `format` is accepted and ignored (`let _ =
  format`) until a second layout exists.
- Backup/restore wire parity is covered by `backup_contracts.rs` and
  `destination_permissions.rs` (exact-count assertions).

## File / line references

- `BackupArgs`: `src/commands/backup_cmd.rs:22`; `run`: `:42`
- Lock/generation: `:78-79,134`; manifest build: `:156-223`
- Publish: `:228`; reports: `:229-258`; handler: `src/main.rs:410`
- Primitives: `src/commands/backup_archive.rs` (+ `backup_archive.md`)

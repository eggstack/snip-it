# restore_cmd — Restore from Backup Snapshot

[← Back to Overview](../overview.md)

## Overview

`src/commands/restore_cmd.rs` (2703 lines, largest command module)
restores a `backup_cmd` snapshot into the live config with checksum,
contract, semantic, and permission verification, executed inside a
transaction journal (`Prepared → Committing → CommittedLocal →
cleanup`) under the `LocalDataLock → TransactionLock` hierarchy.
Three modes: `DryRun` (plan only), `Merge` (default; upsert),
`Replace` (target libraries cleared first).

## CLI surface

`RestoreArgs` (`restore_cmd.rs:26`), `snp restore` and
`snp data restore` via `handle_restore` (`main.rs:422`):
`BACKUP_DIR` (positional), `--mode <dry-run|merge|replace>`
(default `merge`), `--json`. Supporting types: `RestoreMode` (`:16`),
`DestinationClass{NewPrivate,ExistingPreserved,Restore}` (`:47`) with
`for_destination` / `apply_permissions` / `verify_permissions`,
`RestoreConflict` (`:166`), `RestoreReport` (`:174`).

## Flow / steps

`run(backup, mode, json)` (`:956`):

1. Manifest load → `validate_manifest_contract` (schema/layout,
   cardinality, destination uniqueness, index consistency) **before**
   any artifact access, lock, or write (`:965-971`).
2. Per-entry `validate_backup_path` (traversal rejection) → artifact
   checks (existence, symlink rejection via `symlink_metadata`,
   regular-file, `MAX_RESTORE_SOURCE_SIZE`, manifest size match,
   library duplicate-ID domain check) → SHA-256 verification →
   `validate_manifest_semantics` (index/library consistency for the
   chosen mode).
3. `DryRun`: print the plan (human/`--json`), return — no locks, no
   transaction, no writes.
4. Live modes: `gate_mutation_on_interrupted_transactions` (`:1142`)
   with `sync_state_dir` vs `transaction_dir` kept distinct, acquire
   local-data then transaction locks, `begin_transaction("restore")`
   over the affected destinations.
5. Stage installs per entry (library → `libraries/<name>.toml`,
   index → `libraries.toml`, usage → `usage.toml`, sync →
   `sync.toml`), `copy_sync_verify` pre-restore backups into
   `.transaction/artifacts/<txn-id>/`, hash-verify after install,
   `verify_metadata` + destination-permission verify.
6. Merge vs Replace: merge upserts entries and preserves unlisted
   local libraries; replace clears in-scope targets first (with
   backup) so the result equals the snapshot.
7. Finalize **only if files were restored** (`:1635+`):
   `advance_to_committed_local(NotRecorded)` →
   `ensure_pending_for_transaction(sync_state_dir, txn_id,
   Mutation{kind: Import})` (Created/Reused/Conflict-covered) →
   `advance_to_committed_local(Recorded/CoveredByExisting)` →
   `commit_transaction` (journal + artifact cleanup) →
   `schedule_existing_pending` (no second pending generation).
   Failpoints (`RESTORE_AFTER_*`) pin each crash window.

## Mutation vs read-only

Mutating (merge/replace): transaction-gated, double-locked, journaled
with durable pre-restore backups. `DryRun` is strictly read-only and
exempt from gating/locking (mirrored in `main.rs:command_behavior`).

## Auto-sync trigger

Transactional, not `notify_mutation`: restored content is published as
one pending generation with `MutationKind::Import`
(`restore_cmd.rs:1657`) via the idempotent
`ensure_pending_for_transaction` API (exactly one generation across
crashes/retries; unrelated newer generations are preserved, not
clobbered), then the worker is scheduled. No-op restores record
nothing.

## Error / exit mapping

`SnipResult<()>`; `handle_restore` maps success to `Success`.
Fail-closed errors (exit 1 family): missing backup path, manifest
contract/semantic violations, traversal, missing/symlinked/oversized/
size-mismatched artifacts, duplicate library IDs, checksum mismatch,
permission-verification failure, gate refusal (directs to `snp
repair`), lock acquisition failure.

## Key invariants

- No lock/transaction/write before the manifest contract validates.
- Journals live in `.transaction/`; pending markers in the state dir —
  never swap the two arguments.
- Destination permissions: new files `0o600`, existing preserved,
  restored-from-backup restored; verified post-install on Unix.
- Sync config restores never resurrect secrets (backups are redacted;
  re-register to restore credentials).
- Tests assert exact counts, prove server-side effects, verify
  pending-clear ordering; barrier tests run `--test-threads=1`.

## File / line references

- Args/mode/permissions: `src/commands/restore_cmd.rs:16-165`
- `run`: `:956`; gate+locks: `:1142-1147`; finalize: `:1635-1720`
- Handler/behavior: `src/main.rs:422,907-914`

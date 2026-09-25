# repair_cmd — Conservative, Backed-Up, Idempotent Repair

[← Back to Overview](../overview.md)

## Overview

`src/commands/repair_cmd.rs` (1079 lines) turns `validate` diagnostics
and transaction-journal scans into a typed, previewable, backed-up
repair plan. Default (no flags) only summarizes; `--dry-run` previews;
`--apply` snapshots a pre-repair backup, applies **safe** actions only,
and reports applied/skipped/failed with a typed exit status. Running it
twice without new damage is a no-op (idempotent).

## CLI surface

`RepairArgs` (`repair_cmd.rs:19`), `snp repair` (alias `rp`) and
`snp data repair` via `handle_repair` (`main.rs:431`):
`--dry-run` (preview, no changes), `--apply` (backup + apply safe
repairs), `-l/--library` (scope candidate collection), `--json`
(machine report). `--dry-run --apply` prints the plan without changing
anything (`show_only = !apply || dry_run`).

`RepairAction` (`:36`): `PruneOrphanedUsage`, `RollbackTransaction{id}`,
`ResumeCleanup{id}`, `FinalizeCommittedLocal{id}`,
`CleanupLegacyCommitted{id}`, `CleanupLegacyRolledBack{id}`,
`RemoveTerminalJournal{id}`, `RemoveOrphanedArtifact`,
`RepairLibraryIndex`, `RepairSnippetIds`, `RepairTimestamps`.
`category()` (`:81`) groups for display; `is_safe()` (`:98`) admits
only usage-prune and transaction-journal actions — index/ID/timestamp
repairs are planned but never auto-applied. `RepairExitStatus` (`:144`):
`Clean | Repaired | PartialFailure | UnsafeOnly | DryRun`.

## Flow / steps

`run(dry_run, apply, library, json)` (`:181`):

1. `collect_repair_candidates` (`:259`): library validation → typed
   `RepairItem{action, category, problem, fix, safe, target_path}`
   (typed path, never string-parsed).
2. `collect_transaction_repairs`: scan `<config>/.transaction/` journals
   — Prepared/Committing/RollingBack → rollback; CleaningUp → resume;
   CommittedLocal → finalize; legacy terminal journals with/without
   artifacts → cleanup/remove; orphaned artifact dirs → remove.
3. If `apply` and items exist: no safe items → `UnsafeOnly` + single
   report; else `create_repair_backup()` first, then `apply_repair`
   per safe item, counting applied/failed/skipped(unsafe).
   `PartialFailure` if any failed, else `Repaired`.
4. No items → `Clean`; `dry_run` with items → `DryRun` (+ `(dry run —
   no changes made)`); items without `--apply` → `UnsafeOnly`.
5. Emit exactly one final report (human to stderr, JSON to stdout),
   then return the status. `main.rs:375-384 exit_on_repair_status`:
   Clean/Repaired/DryRun → continue `Success`; PartialFailure →
   exit 1; UnsafeOnly → exit 10 (unsafe repairs pending).

## Mutation vs read-only

Conditionally mutating: read-only unless `--apply` (with safe items),
in which case it is the most privileged local writer — backup first,
journal-driven, idempotent. Preview paths take no locks and need no
gate; apply paths go through the transaction APIs (which own gating
and locking).

## Auto-sync trigger

None. `repair_cmd.rs` contains no `notify_mutation`; repaired content
syncs via the normal post-repair mutation path (the next user mutation
or explicit `snp sync`), never from repair itself.

## Error / exit mapping

- Exit 0: Clean / Repaired / DryRun. Exit 1: PartialFailure (some
  safe repair failed). Exit 10: UnsafeOnly (needs operator judgment).
- JSON counters (`applied/skipped/failed`) and `exit_status` reflect
  the final state — the report emits once, after all work.
- Per-item failure never aborts the batch; it is counted and reported.

## Key invariants

- Backup before any mutation; repairs are idempotent and safe to
  re-run (`--apply` on clean state → `Clean`).
- Only `is_safe()` actions auto-apply; unsafe candidates are reported
  with manual fixes, never executed.
- `target_path` is typed — display strings are never re-parsed.
- Kernel-lock files are diagnostic-only: never inspected, rewritten,
  or removed by repair. `kill(pid,0)`: only `ESRCH` proves absence.
- Barrier-gated tests (`repair_transactions`, `test-support`) cover
  crash/restore interleavings; run serially.

## File / line references

- `RepairArgs`: `src/commands/repair_cmd.rs:19`; actions: `:36-124`
- `RepairItem/Report/Status`: `:128-169`; `run`: `:181`
- Candidates: `:259`; exit mapping: `src/main.rs:375-384,431-435`

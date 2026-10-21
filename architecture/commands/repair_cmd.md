# snp repair

[← Back to Overview](../overview.md) · [Command index](mod.md) · [restore_cmd.md](restore_cmd.md) · [validate_cmd.md](validate_cmd.md)

`snp repair` is the **recovery command**. It is what the mutation gate and the restore path point an operator at when a transaction was interrupted. It finds problems (interrupted transactions, orphaned artifacts, library-level defects, orphaned usage entries), reports them, and — with `--apply` — fixes the ones it considers safe.

It is **not** a general-purpose "fix my snippets" command: several detected problems are deliberately marked *unsafe* and can only be reported, never auto-applied.

Critically, `snp repair` does **not** call `gate_mutation_on_interrupted_transactions()`. It is the escape hatch from the gate — the gate refuses to proceed and says "run `snp repair`"; running repair resolves the condition the gate tripped on.

## CLI arguments

`snp repair` and `snp data repair` share one struct (`src/commands/repair_cmd.rs:19-32`):

```rust
pub struct RepairArgs {
    /// Show planned repairs without making changes
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub dry_run: bool,
    /// Apply safe repairs (creates backup first)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub apply: bool,
    /// Repair a specific library
    #[arg(short, long)]
    pub library: Option<String>,
    /// Output as JSON
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub json: bool,
}
```

| Flag | Type | Default | Effect |
|---|---|---|---|
| `--dry-run` | flag | off | Analyse and print planned repairs; make no changes. |
| `--apply` | flag | off | Take a pre-repair backup, then apply the **safe** repairs. |
| `-l`, `--library <NAME>` | `Option<String>` | `None` | Restrict library-level checks to one library. Transaction repairs are unaffected. An unknown name is a hard error ("Library not found"). |
| `--json` | flag | off | Emit the report as JSON on **stdout**. |

`--dry-run` and `--apply` are separate flags; passing both is accepted, but the **apply branch wins** — `run()` checks `apply && !report.items.is_empty()` first, so the repairs are really applied and the exit status is `Repaired` / `PartialFailure` (the trailing `(dry run — no changes made)` line at `repair_cmd.rs:255-257` is still printed — see [Gotchas](#gotchas)).

## Repair item inventory

Every finding is a `RepairItem`:

```rust
pub struct RepairItem {
    pub action: RepairAction,
    pub category: String,
    pub problem: String,
    pub fix: String,
    pub safe: bool,
    pub target_path: Option<PathBuf>,
}
```

(`src/commands/repair_cmd.rs:128-142`.) `action` is the typed category; `category` is the short display string; `target_path` replaces fragile string parsing of `problem` for the one action that needs a path.

### `RepairAction` variants

```rust
pub enum RepairAction {
    PruneOrphanedUsage,
    RollbackTransaction { transaction_id: String },
    ResumeCleanup { transaction_id: String },
    FinalizeCommittedLocal { transaction_id: String },
    CleanupLegacyCommitted { transaction_id: String },
    CleanupLegacyRolledBack { transaction_id: String },
    RemoveTerminalJournal { transaction_id: String },
    RemoveOrphanedArtifact,
    RepairLibraryIndex,
    RepairSnippetIds,
    RepairTimestamps,
}
```

(`src/commands/repair_cmd.rs:36-77`.) `RepairAction::category()` (`repair_cmd.rs:81-…`) maps these to the display categories `usage`, `transaction`, `index`, `ids`, `timestamps`.

### Item categories as produced

| `category` | Source | `safe` | Notes |
|---|---|---|---|
| `config` | `LibraryManager::new()` failed, or a library file failed to parse (`repair_cmd.rs:271-283`, `:373-385`) | `false` | Reported, never auto-applied. |
| `ids` | snippet with empty ID, or duplicate ID across libraries (`:324-352`) | `false` | Reported only — `apply_repair` cannot execute `RepairSnippetIds` either, see [Gotchas](#gotchas). |
| `timestamps` | `created_at == 0` or `updated_at == 0` (`:355-371`) | `false` | Same caveat as `ids`. |
| `primary` | primary library missing its file, or no primary set (`:392-432`) | `false` | Uses the shared `LibraryManager::inspect_library_index()` so `repair` and `validate` classify the primary state identically. |
| `usage` | orphaned usage entries (`:444-453`) | `true` | Count comes from `crate::library::find_orphaned_ids`, the same classifier `validate` uses. |
| `transaction` | interrupted or legacy journals (`:461-588`) | `true` | Recovered via `crate::transaction::recover_transaction_by_id`. |
| `unsafe` | corrupt journals, unsafe artifacts, `Failed` transactions (`:466-481`, `:487-505`, `:535-552`) | `false` | Reported only; the operator must inspect the journal and artifacts manually. |

### Transaction recovery classes

`collect_transaction_repairs` scans `<config>/.transaction/` with `crate::transaction::scan_transaction_journals` and classifies each journal with `crate::transaction::classify_journal_recovery` (`repair_cmd.rs:461-506`). The resulting `RecoveryClass` selects the action:

| `RecoveryClass` | Action | What it does |
|---|---|---|
| `Rollback` | `RollbackTransaction` | Rolls the interrupted transaction back using the journal's recorded backups. |
| `FinalizeCommittedLocal` | `FinalizeCommittedLocal` | Completes the pending-marker finalization, then cleans up. |
| `ResumeCleanup` | `ResumeCleanup` | Resumes an interrupted cleanup phase. |
| `CleanupLegacyCommitted` | `CleanupLegacyCommitted` | Removes a legacy `Committed` journal that still has artifacts. |
| `CleanupLegacyRolledBack` | `CleanupLegacyRolledBack` | Removes a legacy `RolledBack` journal that still has artifacts. |
| `RemoveTerminalJournal` | `RemoveTerminalJournal` | Removes a terminal journal with no artifacts. |
| `UnsafeFailed` | *(reported as unsafe)* | The transaction is in a `Failed` state; manual investigation required. |

`collect_orphan_artifact_repairs` (`repair_cmd.rs:593-657`) additionally scans `<config>/.transaction/artifacts/` for directories with **no matching journal** — stale artifacts left by a crash during cleanup. These become `RemoveOrphanedArtifact` items.

## Modes

`snp repair` has three effective modes, chosen by the flags (`repair_cmd.rs:184-267`):

| Flags | Behavior | Exit status |
|---|---|---|
| neither | Analyse and print the summary only; nothing is changed | `Clean` if no items, `UnsafeOnly` if any finding is unsafe, `FindingsSafe` if every finding is safe |
| `--dry-run` | Analyse and print planned repairs; nothing is changed | `DryRun` if there are items, `Clean` otherwise |
| `--apply` | Take the pre-repair backup, then apply the safe repairs | `Repaired` if every safe repair succeeded, `PartialFailure` if any failed |

The decision tree in `run()`:

- `apply && !report.items.is_empty()` → filter to `item.safe`, apply each, count `applied` / `failed` / `skipped` (`:199-238`).
- `apply` with **no safe items** → `UnsafeOnly`, and the report is emitted immediately (`:202-212`).
- `!apply && report.items.is_empty()` → `Clean` (`:239-240`).
- `!apply && dry_run && !items.is_empty()` → `DryRun` (`:241-242`).
- `!apply && !dry_run && !items.is_empty()` → `UnsafeOnly` when at least one finding is unsafe, otherwise `FindingsSafe` (`:243-252`).

The report is emitted **exactly once**, after all work is complete, so JSON counters and exit status always reflect the final result (`:182-183`, `:259-264`).

## The backup taken before repair

`create_repair_backup` (`repair_cmd.rs:873-905`) runs **before** any repair is applied (`:214-216`). It creates:

```
<config>/backups/repair-<YYYYMMDD_HHMMSS>-<uuid>/
  libraries/…        # recursive copy of <config>/libraries
  libraries.toml      # copy of the index
  usage.toml          # copy of the usage index
```

The backup is a **plain recursive copy** (`copy_dir_recursive`, `:908-926`) — not a manifest-backed `snp backup` snapshot, and not a transaction. It is therefore **not** restorable by `snp restore` (which requires a `manifest.toml`); it is a manual escape hatch.

Note that `libraries.toml` and `usage.toml` copies use `let _ = fs::copy(...)` — a **copy failure is silently ignored** (`:895`, `:901`).

## `apply_repair` — what each action actually does

`apply_repair` (`repair_cmd.rs:660-844`):

| Action | Implementation |
|---|---|
| `PruneOrphanedUsage` | Loads `UsageIndex`, computes the active snippet-ID set from every library, calls `usage_index.prune(&active_ids)`, then `save()` (`:668-674`). |
| `RemoveOrphanedArtifact` | Validates that `target_path` is under `<config>/.transaction/artifacts/` after canonicalization, rejects symlinks, then `remove_dir_all` (`:675-720`). |
| `RollbackTransaction` / `ResumeCleanup` / `FinalizeCommittedLocal` / `CleanupLegacyCommitted` / `CleanupLegacyRolledBack` / `RemoveTerminalJournal` | Each calls `crate::transaction::recover_transaction_by_id(&sync_state_dir, &state_dir, transaction_id, <RecoveryClass>)` (`:721-828`). |
| `RepairLibraryIndex` / `RepairSnippetIds` / `RepairTimestamps` | **Not implemented.** Returns `Err("Auto-repair not implemented for this category")` (`:829-841`). |

The transaction recovery path takes the **transaction lock** inside `recover_transaction_by_id`; it does **not** take the local-data lock. The `PruneOrphanedUsage` path is the exception: `UsageIndex::save()` takes the local-data lock itself, scoped to that single write (`src/usage.rs:71-76`). `snp repair` therefore sits at a different point in the lock hierarchy than `snp restore`:

| Command | Local-data lock | Transaction lock | Gate |
|---|---|---|---|
| `snp restore` | yes, held for the whole run (`restore_cmd.rs:1146`) | yes, held for the whole run (`:1147`) | yes (`:1142`) |
| `snp repair` | only inside `UsageIndex::save()` during `PruneOrphanedUsage` (`src/usage.rs:74`) | yes, inside `recover_transaction_by_id` | **no — by design** |
| `snp backup` | yes, held for the whole capture (`backup_cmd.rs:78`) | no | no |

Under the `test-support` feature, `apply_repair` also honours an injected failure keyed by transaction ID (`repair_cmd.rs:661-665`, `crate::test_failpoints::maybe_injected_error("repair-transaction-<id>")`).

## Idempotency

Repair is **idempotent by construction**: every action is derived from the *current* state on each run, and every action either restores a known-good state or removes an artifact that has no owner.

- Transaction actions are classified from the journal's current state; re-running after a successful recovery finds nothing to do.
- `RemoveOrphanedArtifact` is a no-op when the directory no longer exists (`repair_cmd.rs:710`).
- `PruneOrphanedUsage` recomputes the orphan set from the current libraries each time.
- The pre-repair backup directory name includes a fresh UUID, so repeated runs never collide — but they also never reuse an earlier backup.

## Output and exit codes

```rust
pub struct RepairReport {
    pub items: Vec<RepairItem>,
    pub backups: Vec<PathBuf>,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub exit_status: RepairExitStatus,
}

pub enum RepairExitStatus {
    Clean,
    Repaired,
    PartialFailure,
    UnsafeOnly,
    FindingsSafe,
    DryRun,
}
```

(`src/commands/repair_cmd.rs:146-161`; `FindingsSafe` is declared at `:158`.) The human report is written to **stderr** (`emit_human_report`, `:929-983`); the JSON report is written to **stdout** (`emit_json_report`, `:986-…`).

`snp repair` uses the dedicated `exit_on_repair_status` mapping (`src/main.rs:401-414`), **not** the generic `CliOutcome` table:

| `RepairExitStatus` | Exit code | Meaning |
|---|---|---|
| `Clean` | `0` | Nothing to repair. |
| `DryRun` | `0` | Dry run completed; no changes made. |
| `Repaired` | `0` | All safe repairs applied successfully. |
| `FindingsSafe` | `0` | Findings exist, but all of them are safe; neither `--apply` nor `--dry-run` was passed. |
| `PartialFailure` | `1` (`GENERAL_ERROR`) | Some repairs failed. |
| `UnsafeOnly` | `10` (`UNSAFE_REPAIRS`) | Issues found but no safe repair was available. |

`10` deliberately does **not** reuse `2` (`USAGE_ERROR`): a valid invocation that found unsafe repairs awaiting an operator decision is not a usage error (`src/main.rs:395-400`, `src/outcome.rs:69-71`).

## Interaction with the transaction journal and locks

- The command **reads** the journal directory via `scan_transaction_journals` (`repair_cmd.rs:463`) and the artifact root for orphans (`:585`).
- It **mutates** journals through `recover_transaction_by_id`, which takes the **transaction lock** with the same ownership record, PID-reuse detection, and quarantine ladder as any other transaction operation.
- It does **not** create a new journal: repair is the recovery path for existing journals, not a new mutation with rollback.
- It does **not** call the mutation gate.

## Key Files

| Path | Role |
|---|---|
| `src/commands/repair_cmd.rs` | This command: item collection, apply logic, backup, report, exit status. |
| `src/transaction.rs` | `scan_transaction_journals`, `classify_journal_recovery`, `recover_transaction_by_id`. |
| `src/library/manager.rs` | `inspect_library_index` — the shared primary-state classifier. |
| `src/library/persistence.rs` | `find_orphaned_ids` / usage classifier used for the orphan count. |
| `src/usage.rs` | `UsageIndex` load / prune / save. |
| `architecture/persistence.md` | Journal states, recovery classes, lock hierarchy. |
| `architecture/commands/restore_cmd.md` | The command whose interrupted restores this one recovers. |

## Invariants

- Repair **never** auto-applies an unsafe item; unsafe findings are reported with a manual fix description.
- The report is emitted exactly once, after all work completes.
- A pre-repair backup of `libraries/`, `libraries.toml`, and `usage.toml` is taken before any repair is applied.
- Orphaned artifact deletion is containment-checked against `<config>/.transaction/artifacts/` and rejects symlinks (`repair_cmd.rs:688-708`).
- Primary-state and orphan-usage classification are **shared** with `validate` and `doctor` via `inspect_library_index` and `find_orphaned_ids`, so the three commands never disagree about what "orphaned" or "no primary" means.
- The exit code distinguishes "found unsafe work" (`10`) from "usage error" (`2`) and from "some repairs failed" (`1`).

## Gotchas

- **`--apply` never attempts the library-level categories.** `RepairSnippetIds`, `RepairTimestamps`, and `RepairLibraryIndex` items are pushed with `safe: false` (`repair_cmd.rs:335`, `:348`, `:367`, `:399`, `:416`, `:425`), so `--apply` filters them out and `apply_repair`'s `"Auto-repair not implemented for this category"` branch (`repair_cmd.rs:829-841`) is **unreachable from the item inventory**. Treat `ids` / `timestamps` / `primary` findings as *advisory*: reported, never auto-fixed.
- **`--apply` wins over `--dry-run`.** With both flags, the apply branch runs first (`repair_cmd.rs:199`), so repairs are really applied — but the closing `(dry run — no changes made)` line is still printed (`repair_cmd.rs:255-257`).
- The `safe` **field on the item** is what `--apply` filters on (`repair_cmd.rs:200`); `RepairAction::is_safe()` is a separate, unused helper (`:98-110`). Do not assume the enum method drives the filter.
- **`FindingsSafe` is not `UnsafeOnly`.** A report-only run whose findings are all safe exits `0` with `FindingsSafe`; only a run that finds at least one unsafe finding exits `10` (`repair_cmd.rs:243-252`, `src/main.rs:401-414`).
- `snp repair` does **not** hold the local-data lock for the whole run, and it takes no transaction lock for the non-transaction actions. It can therefore run concurrently with a `snp backup` snapshot; only the individual actions take their own locks.
- The pre-repair backup is **not** a `snp backup` snapshot: it has no `manifest.toml` and cannot be restored with `snp restore`.
- `libraries.toml` / `usage.toml` copy failures in the pre-repair backup are swallowed (`repair_cmd.rs:895`, `:901`).
- `--library` restricts only the **library-level** checks. Transaction and orphan-artifact repairs always run against the whole transaction directory.
- A corrupt journal is reported as `unsafe` with the fix "Requires manual quarantine or deletion" (`repair_cmd.rs:466-481`) — repair will not delete a corrupt journal for you.

# Canonical Operation Inventory

> Generated: Phase 06A Workstream D
> Purpose: Single source of truth for every behavior-critical operation, its canonical
> implementation, all callers/adapters, and any semantic deviations.

---

## 1. Load Library

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/library.rs:659` — `load_library(path: &Path) -> SnipResult<Snippets>` |
| **Semantics** | Reads TOML, applies `fix_invalid_toml_escapes`, deduplicates snippet IDs, returns default on missing/corrupt (with backup). Uses `cached_read_toml`. |

### Callers

| Caller | Location | Adapts? |
|--------|----------|---------|
| `load_snippets(config)` | `src/commands/mod.rs:114` | Yes — resolves path via `get_config_path(config)`, uses raw `fs::read_to_string` instead of `cached_read_toml`, returns error (not default) on corrupt. **Semantics differ: does not dedup IDs, does not use `cached_read_toml`.** |
| `status_snapshot::load_library` | `src/status_snapshot.rs:562` | No — direct call |
| `sync_commands::run_sync` | `src/sync_commands.rs:530` | No — direct call |
| `edit_cmd::run` | `src/commands/edit_cmd.rs:76` | No — direct call |
| `new_cmd::run` | `src/commands/new_cmd.rs:559` | No — fallback path only |
| `import_cmd::run_import_pet` | `src/commands/import_cmd.rs:309` | No — direct call |
| `sync_cmd` (3 sites) | `src/commands/sync_cmd.rs:14,79,252` | No — direct call |
| `doctor_cmd` | `src/commands/doctor_cmd.rs:539` | No — direct call |
| `list_cmd` | `src/commands/list_cmd.rs:41` | No — but called via `load_snippets` wrapper |

### Duplicates / Concern

`load_snippets` (`src/commands/mod.rs:114`) is a **parallel implementation** that does not:
- Use `cached_read_toml` (reads directly with `fs::read_to_string`)
- Deduplicate snippet IDs
- Return `Snippets::default()` on parse error (returns `SnipError` instead)

This is the only significant loader variant. It is used by `list_cmd` and `new_cmd` fallback path.

---

## 2. Save Library

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/library.rs:726` — `save_library(path: &Path, snippets: &Snippets) -> SnipResult<()>` |
| **Semantics** | Creates backup, sorts by `updated_at` desc, serializes via `toml::to_string_pretty`, writes atomically. No TOML post-processing. |

### Callers

| Caller | Location | Adapts? |
|--------|----------|---------|
| `save_snippets(s, config)` | `src/commands/mod.rs:163` | Yes — resolves path via `get_config_path(config)`, uses `backup_library` but different temp-file/atomic-write path. **Semantics differ: does not sort by `updated_at`.** |
| `sync_commands::merge_and_save` | `src/sync_commands.rs:350` | No — direct call |
| `edit_cmd::run` | `src/commands/edit_cmd.rs:108` | No |
| `import_cmd` (4 sites) | `src/commands/import_cmd.rs:295,386,394,416` | No |
| `new_cmd::run` | `src/commands/new_cmd.rs:568` | No |
| `mod.rs:310` | `src/commands/mod.rs:310` | No |
| `sync_cmd` (3 sites) | `src/commands/sync_cmd.rs:37,81,969` | No |

### Duplicates / Concern

`save_snippets` (`src/commands/mod.rs:163`) is a **parallel implementation** that skips the `updated_at` sort. Used only by `new_cmd` fallback.

---

## 3. Load/Save Sync Settings

| Attribute | Value |
|-----------|-------|
| **Load** | `src/config.rs:526` — `load_sync_settings() -> SnipResult<SyncSettings>` |
| **Save** | `src/config.rs:504` — `save_sync_settings(settings: &SyncSettings) -> SnipResult<()>` |
| **Semantics (Load)** | Reads via `cached_read_toml`, verifies CRC32 integrity, strips integrity line, applies `fix_invalid_toml_escapes`, returns defaults on missing/corrupt. |
| **Semantics (Save)** | Serializes, computes CRC32, prepends integrity line, writes via `write_private_atomic`, invalidates caches. |

### Callers — `load_sync_settings`

| Caller | Location |
|--------|----------|
| `status_snapshot::sync_configuration_state` | `src/status_snapshot.rs:195` |
| `sync_commands::run_default_sync` | `src/sync_commands.rs:867` |
| `executor::run_executor` | `src/auto_sync/executor.rs:192` |
| `register_cmd::run` | `src/commands/register_cmd.rs:7,21` |
| `doctor_cmd` | `src/commands/doctor_cmd.rs:588` |
| `sync_cmd` (3 sites) | `src/commands/sync_cmd.rs:165,354,502` |

### Callers — `save_sync_settings`

| Caller | Location |
|--------|----------|
| `sync_cmd::run_config` | `src/commands/sync_cmd.rs:492` |
| `register_cmd::run` | `src/commands/register_cmd.rs:40` |

### Duplicates

None. Single canonical pair.

---

## 4. Match/Select Snippet

| Attribute | Value |
|-----------|-------|
| **Canonical (list mode)** | `src/sort.rs:108` — `rank_snippets(indices, snippets, fuzzy_scores, usage, opts) -> Vec<usize>` |
| **Canonical (TUI mode)** | `src/ui/mod.rs:187` — `sort_filtered_indices(filtered, filter_state, snippets, ...)` |
| **Semantics** | `rank_snippets` applies multi-key sort (favorites-first, mode-dependent primary, tie-break chain). `sort_filtered_indices` is the TUI's inline re-sort (favorites-first + fuzzy scores + display order). |

### Callers — `rank_snippets`

| Caller | Location |
|--------|----------|
| `list_cmd::run` | `src/commands/list_cmd.rs:86` |

### Callers — `sort_filtered_indices`

| Caller | Location |
|--------|----------|
| TUI render loop | `src/ui/mod.rs:770` |

### Duplicates / Concern

These are **not duplicates** — `rank_snippets` is for non-interactive list output, `sort_filtered_indices` is for the TUI's live filter. They share the favorites-first and fuzzy-score logic but have different signatures and sort strategies.

---

## 5. Expand Variables

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/utils/variables.rs:548` — `expand_command(command: &str, values: &[(String, String)]) -> String` |
| **Semantics** | Tokenizes `<var>` placeholders, handles `\` escapes, supports `<var=default>`, `<var=_opt1_||_opt2_>`, `<var=_red_||_green_||_blue_||>` list prompts. Pure string transformation (no I/O). |

### Callers

| Caller | Location | Adapts? |
|--------|----------|---------|
| `expand_snippet_command` | `src/commands/mod.rs:236` | No — called after TUI prompt collects values |

### Duplicates

None. Single canonical implementation. The TUI variable prompt (`src/ui/variables.rs`) collects values but does not expand — expansion is always via `expand_command`.

---

## 6. Import/Export

| Attribute | Value |
|-----------|-------|
| **Import** | `src/commands/import_cmd.rs:167` — `run_import_pet(options: PetImportOptions) -> SnipResult<()>` |
| **Semantics** | Reads pet TOML, converts entries via `convert_pet_entry`, deduplicates against destination, supports Create/Merge/Replace modes, emits `PetImportReport`. |

### Callers

| Caller | Location |
|--------|----------|
| CLI dispatch | `src/main.rs` (via clap) |

### Export

There is **no dedicated export command or function**. The library TOML format is the de facto export. Snippets can be read via `load_library` and serialized via `save_library` to any path.

### Duplicates

None for import. No export implementation exists.

---

## 7. Record Usage

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/usage.rs:78` — `UsageIndex::record_use(&mut self, snippet_id: &str)` |
| **Semantics** | Increments `use_count`, sets `last_used_at` to current timestamp. Creates entry if missing. |

### Persistence

| Operation | Function | Location |
|-----------|----------|----------|
| Load | `UsageIndex::load()` | `src/usage.rs:64` |
| Save | `UsageIndex::save()` | `src/usage.rs:69` |

### Callers — `record_use`

Called after snippet execution/clip via `run_snippet_selection` → `process_fn` callbacks in `run_cmd`, `clip_cmd`, `search_cmd`.

### Callers — `UsageIndex::load`

| Caller | Location |
|--------|----------|
| `run_snippet_selection` | `src/commands/mod.rs:272` |
| `list_cmd::run` | `src/commands/list_cmd.rs:86` |

### Duplicates

None. Single canonical implementation.

---

## 8. Perform Sync

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/sync_commands.rs:365` — `run_sync(sync_settings, library_name, push_only, pull_only, runtime) -> SnipResult<()>` |
| **Semantics** | Resolves direction, ensures config, creates client, health check, iterates libraries, merges per-library via `merge_and_save`, records status. |

### Callers

| Caller | Location | Adapts? |
|--------|----------|---------|
| `executor::run_executor` | `src/auto_sync/executor.rs:256` | No — maps direction from `effective_sync_direction` |
| `sync_cmd::run` | `src/commands/sync_cmd.rs:285` | No — resolves push/pull from CLI flags + config |
| `sync_cmd::run_retry` | `src/commands/sync_cmd.rs:557` | No — passes through to `run_sync` |
| `run_default_sync` | `src/sync_commands.rs:866` | Convenience wrapper — resolves settings, calls `run_sync` |
| `run_premade_sync` | `src/sync_commands.rs:229` | Separate function — syncs premade libraries specifically |

### Variants

| Function | Location | Purpose |
|----------|----------|---------|
| `run_sync` | `src/sync_commands.rs:365` | Primary bidirectional sync |
| `run_premade_sync` | `src/sync_commands.rs:229` | Premade library sync (different flow) |
| `run_default_sync` | `src/sync_commands.rs:866` | Convenience — loads settings, calls `run_sync` |
| `premade_cmd::run_sync` | `src/commands/premade_cmd.rs:145` | CLI entry for `snp premade sync` (calls `run_premade_sync`) |

### Duplicates / Concern

`run_premade_sync` is a **separate flow** for premade libraries (different server endpoints, different merge logic). This is intentional, not a duplicate.

---

## 9. Resolve Sync Direction

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/auto_sync/executor.rs:162` — `effective_sync_direction(settings, cli_push_only, cli_pull_only) -> SyncDirection` |
| **Semantics** | CLI flags override config. No CLI override → use `settings.sync_direction`. |

### Callers

| Caller | Location |
|--------|----------|
| `executor::run_executor` | `src/auto_sync/executor.rs:231` |

### Inline Resolution

`sync_cmd::run` (`src/commands/sync_cmd.rs:253`) also resolves direction inline from CLI flags + config, but does **not** call `effective_sync_direction`. This is a **duplication of logic** with slightly different behavior (the sync_cmd version checks both push_only and pull_only flags independently).

### Callers — `SyncDirection` enum

Defined at `src/config.rs:477`. Used by `SyncSettings.sync_direction`, `effective_sync_direction`, `run_sync` direction mapping, and `status::compute_config_fingerprint`.

### Duplicates / Concern

Two direction-resolution paths exist:
1. `effective_sync_direction` (executor) — CLI overrides config
2. `sync_cmd::run` inline resolution — CLI flags interact with config direction

These are **semantically equivalent** but the duplication is a maintenance risk.

---

## 10. Record Pending Mutation

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/auto_sync/pending.rs:64` — `record_pending_mutation(state_dir, snapshot) -> Result<PendingState, PendingError>` |
| **Semantics** | Acquires pending txn lock, reads existing state, increments generation, writes atomically. |

### Callers (production)

| Caller | Location |
|--------|----------|
| `notification::notify_local_mutation_with_dir` | `src/auto_sync/notification.rs:93` |
| `schedule::schedule_and_spawn` (via `record_pending_mutation`) | `src/auto_sync/schedule.rs:206,223,238,267,300,341,360,396,411,435,454,463` |

### Callers (tests only)

`src/auto_sync/worker.rs` (15+ test sites), `src/status_snapshot.rs` (6 test sites), `src/auto_sync/pending.rs` (14 test sites).

### Duplicates

None. Single canonical implementation.

---

## 11. Schedule Worker

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/auto_sync/schedule.rs:48` — `schedule_sync(state_dir, policy, caller) -> ScheduleDecision` |
| **Semantics** | Central scheduling authority. Checks: sync configured, policy enabled, pending work exists, execution lock not held, backoff not active, config-change deferral release. Returns `ScheduleDecision` enum. |

### Convenience Wrapper

| Function | Location | Purpose |
|----------|----------|---------|
| `schedule_sync_from_config` | `src/auto_sync/schedule.rs:140` | Resolves policy from current config, calls `schedule_sync` |

### Spawn Path

| Function | Location | Purpose |
|----------|----------|---------|
| `schedule_and_spawn` | `src/auto_sync/schedule.rs:149` | Translates `SpawnNow` → actual worker spawn |

### Callers — `schedule_sync`

| Caller | Location |
|--------|----------|
| `schedule_sync_from_config` | `src/auto_sync/schedule.rs:143` |
| `schedule_and_spawn` | `src/auto_sync/schedule.rs:154` |

### Callers — `schedule_sync_from_config`

Called by all mutation notification paths and startup recovery.

### Callers — `schedule_and_spawn`

| Caller | Location |
|--------|----------|
| `notification::schedule_after_record` | `src/auto_sync/notification.rs` |
| `notification::startup_recover_pending` | `src/auto_sync/notification.rs:180` |

### Duplicates

None. `schedule_sync` is the sole scheduling authority. All paths converge here.

---

## 12. Supervise Executor

| Attribute | Value |
|-----------|-------|
| **Spawn** | `src/auto_sync/spawn.rs:69` — `spawn_executor(state_dir) -> Result<Child, SpawnError>` |
| **Run** | `src/auto_sync/executor.rs:189` — `run_executor(state_dir) -> i32` |
| **Callers (spawn)** | `worker::execute_sync` at `src/auto_sync/worker.rs:463` |
| **Callers (run)** | CLI dispatch at `src/main.rs:839` |

### Semantics

- `spawn_executor`: Forks `snp auto-sync-execute` subprocess with `--state-dir`
- `run_executor`: Loads settings, resolves direction, runs `run_sync`, classifies errors, maps to `ExecutorExitCode`

### Worker Supervision

| Function | Location | Purpose |
|----------|----------|---------|
| `worker::execute_sync` | `src/auto_sync/worker.rs:446` | Holds `SyncExecutionLock`, spawns executor, waits for exit, maps exit code → `FailureClass` → status |

### Duplicates

None. Single spawn + single run entry points.

---

## 13. Record Status

| Attribute | Value |
|-----------|-------|
| **Write** | `src/auto_sync/status.rs:140` — `write_status(state_dir, status) -> Result<(), String>` |
| **Record success** | `src/auto_sync/status.rs:203` — `record_success(state_dir, pending_generation, message)` |
| **Record failure** | `src/auto_sync/status.rs:227` — `record_failure(state_dir, pending_generation, failure_class, ...)` |
| **Semantics** | CRC32 integrity, bounded file size (8KB), atomic write, chmod 0o600 on Unix, fsync. |

### Callers — `write_status`

| Caller | Location |
|--------|----------|
| `record_success` | `src/auto_sync/status.rs:223` |
| `record_failure` | `src/auto_sync/status.rs:261` |
| `sync_cmd::run_clear_failure` | `src/commands/sync_cmd.rs:612` |
| `sync_cmd::run_repair` | `src/commands/sync_cmd.rs:883` |
| Tests | `src/auto_sync/status.rs` (5 test sites) |

### Callers — `record_success` / `record_failure`

| Caller | Location |
|--------|----------|
| `executor::run_executor` | `src/auto_sync/executor.rs` (via worker) |
| `worker::execute_sync` | `src/auto_sync/worker.rs` |

### Duplicates

None. Single canonical status persistence path.

---

## 14. Conditional Pending Clear

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/auto_sync/pending.rs:102` — `clear_if_generation_matches(state_dir, observed_generation) -> Result<ConditionalClearResult, PendingError>` |
| **Semantics** | Read-compare-delete under `PendingTxnGuard`. Only deletes if generation matches (prevents stale clears). |

### Convenience Wrappers

| Function | Location | Purpose |
|----------|----------|---------|
| `pending::record_success` | `src/auto_sync/pending.rs:136` | Delegates to `clear_if_generation_matches` |
| `worker::clear_after_explicit_sync` | `src/auto_sync/worker.rs:736` | Wraps with sync_succeeded check |
| `notification::clear_pending_after_explicit_sync` | `src/auto_sync/notification.rs:136` | Resolves state_dir, delegates |

### Callers — `clear_if_generation_matches`

| Caller | Location |
|--------|----------|
| `pending::record_success` | `src/auto_sync/pending.rs:140` |
| `worker::execute_sync` (success path) | `src/auto_sync/worker.rs:256` |
| `worker::clear_after_explicit_sync` | `src/auto_sync/worker.rs:744` |
| `sync_cmd::run_discard_pending` | `src/commands/sync_cmd.rs:671` |
| `sync_cmd::run_repair` | `src/commands/sync_cmd.rs:574` |

### Callers — `clear_pending_after_explicit_sync`

| Caller | Location |
|--------|----------|
| `commands/mod.rs` (run/clip/search post-sync) | `src/commands/mod.rs:329,402` |
| `sync_cmd::run` | `src/commands/sync_cmd.rs:328` |

### Duplicates

None. Single canonical clear with generation guard. Wrappers are thin adapters.

---

## 15. Inspect Status

| Attribute | Value |
|-----------|-------|
| **Canonical** | `src/status_snapshot.rs:137` — `capture_snapshot() -> StatusSnapshot` |
| **Semantics** | Assembles read-only projection from: `sync_configuration_state`, `pending_state_view`, `execution_state_view`, `read_status_typed`, library counts. Includes `derive_top_level` for summary state and `collect_diagnostics` for issues. |

### Supporting Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `sync_configuration_state` | `src/status_snapshot.rs:194` | NotConfigured / Configured / LoadFailed |
| `pending_state_view` | `src/status_snapshot.rs:210` | Pending generation + age |
| `attempt_state_view` | `src/status_snapshot.rs:241` | Last attempt result + backoff |
| `execution_state_view` | `src/status_snapshot.rs:269` | Worker + executor lock status |
| `derive_top_level` | `src/status_snapshot.rs:327` | Top-level health summary |
| `collect_diagnostics` | `src/status_snapshot.rs:376` | List of diagnostic issues |

### Callers — `capture_snapshot`

| Caller | Location |
|--------|----------|
| `status_cmd::run` | `src/commands/status_cmd.rs:6` |

### Callers — `read_status_typed`

| Caller | Location |
|--------|----------|
| `capture_snapshot` | `src/status_snapshot.rs:151` |
| `schedule_sync` | `src/auto_sync/schedule.rs:85` |
| `sync_cmd::run_repair` | `src/commands/sync_cmd.rs:707` |

### Duplicates

None. Single canonical snapshot builder.

---

## 16. Validate/Repair State

| Attribute | Value |
|-----------|-------|
| **Validate (doctor)** | `src/commands/doctor_cmd.rs:1306` — `run(pet_file, compatibility, sync, check_shell, library, strict, report_format)` |
| **Repair** | `src/commands/sync_cmd.rs:698` — `run_repair(dry_run, apply)` |
| **Semantics (doctor)** | Multi-mode: `--pet-file` (format validation), `--compatibility` (pet format analysis), `--sync` (sync state checks), `--check-shell` (shell integration), `--library` (library validation). Emits `CompatibilityDiagnostic` report. |
| **Semantics (repair)** | Reads status/pending/locks, diagnoses corrupt artifacts, quarantines or recreates. Dry-run by default. |

### Callers

| Function | Caller |
|----------|--------|
| `doctor_cmd::run` | CLI dispatch |
| `sync_cmd::run_repair` | CLI dispatch |

### Duplicates

None. `doctor` is read-only diagnostics; `repair` is write-capable state recovery. Complementary, not overlapping.

---

## Summary of Duplicates and Concerns

| Issue | Severity | Location |
|-------|----------|----------|
| `load_snippets` parallel implementation | Medium | `src/commands/mod.rs:114` vs `src/library.rs:659` — no ID dedup, no cached_read_toml, returns error not default |
| `save_snippets` parallel implementation | Low | `src/commands/mod.rs:163` vs `src/library.rs:726` — no `updated_at` sort |
| Inline sync direction resolution | Low | `src/commands/sync_cmd.rs:253` vs `src/auto_sync/executor.rs:162` — equivalent logic, duplication risk |
| No export function | Info | Export is implicit via `save_library` to arbitrary path |

### No Duplicates Found

- **Expand variables**: Single `expand_command` implementation
- **Record usage**: Single `UsageIndex::record_use`
- **Record pending**: Single `record_pending_mutation`
- **Schedule worker**: Single `schedule_sync` authority
- **Supervise executor**: Single spawn + run pair
- **Record status**: Single `write_status` / `record_success` / `record_failure`
- **Conditional clear**: Single `clear_if_generation_matches`
- **Inspect status**: Single `capture_snapshot`
- **Validate/repair**: Separate non-overlapping commands

# Maintenance and Duplicate-Policy Consolidation Milestone 002 — Backup and snapshot policy convergence

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap: [`plans/subsystems/maintenance-consolidation-roadmap.md` § M002 — Backup and snapshot policy convergence](../../subsystems/maintenance-consolidation-roadmap.md#m002--backup-and-snapshot-policy-convergence)

Long-term requirements:

- [`plans/000-long-term-specification.md` § 2. Primary product goals](../../000-long-term-specification.md#2-primary-product-goals)
- [`plans/000-long-term-specification.md` § 3. Non-goals](../../000-long-term-specification.md#3-non-goals)
- [`plans/000-long-term-specification.md` § 4. Crate and module ownership](../../000-long-term-specification.md#4-crate-and-module-ownership)
- [`plans/000-long-term-specification.md` § 5. Durability invariants](../../000-long-term-specification.md#5-durability-invariants)
- [`plans/000-long-term-specification.md` § 10. Verification and evidence rules](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md` § 4. Library and persistence terms](../../001-terminology-and-domain-model.md#4-library-and-persistence-terms)
- [`plans/002-long-term-roadmap.md` § Cross-phase execution rules](../../002-long-term-roadmap.md#cross-phase-execution-rules)
- [`plans/003-planning-process.md`](../../003-planning-process.md)

Applicable ADRs: none

Primary class: invariant

---

## 1. Objective

Make every snapshot, backup, and rollback-point path share **one** copy policy:

1. A copy source that is a symlink, a non-regular file, or a path resolving outside the
   configuration directory is **refused**, and the refusal names the offending path.
2. The material a copy produces is **fsynced** under a **named durability class** that already
   exists in the atomic layer — no new durability mechanism is introduced.
3. The pre-repair backup becomes a **valid rollback point**: complete, durable, and restorable by
   `snp restore`.

The shared primitive lives in the atomic/persistence layer (`src/utils/atomic.rs`), so "refuses a
symlink" and "fsyncs the output" are properties of the primitive rather than properties one call
site happens to remember to apply. No command spelling, exit code, stdout/stderr split, or
machine-output schema changes.

## 2. Why this milestone is ready

- M001 is closed (`plans/closure/maintenance-consolidation/001-status.md`), so the dead migration
  framework and the `snp backup --format` flag are gone and the live backup surface is the one
  this plan touches.
- The roadmap records M002 as dependency-ready (§ 12, milestone table) and the M001 closure record
  lists it as the next milestone to plan.
- The defect is fully reproducible from the current tree: the four copy paths exist, they use
  three different mechanisms, and two of them have neither symlink refusal nor fsync.
- The atomic layer already exposes the named durability classes and the atomic replace machinery
  this milestone reuses (`src/utils/atomic.rs:23`, `:376`), so the primitive extends an existing
  mechanism rather than inventing one.

## 3. Current implementation evidence

### 3.1 The four backup paths

| # | Path | Entry point | Copy mechanism |
|---|------|-------------|----------------|
| 1 | Per-library save-time `.bak` sibling (rotated, max 10 kept) | `src/library/persistence.rs:381` `backup_library` | plain `fs::copy` at `src/library/persistence.rs:411` |
| 2 | User-requested snapshot (`snp backup`) | `src/commands/backup_cmd.rs:39` `run` | read into memory by `read_for_snapshot` (`src/commands/backup_archive.rs:194`), written by `atomic_write_backup` (`src/commands/backup_cmd.rs:223`) |
| 3 | Pre-repair whole-config backup | `src/commands/repair_cmd.rs:873` `create_repair_backup` | plain `fs::copy`, recursive, at `src/commands/repair_cmd.rs:925` |
| 4 | Pre-restore backup | `src/commands/restore_cmd.rs:756` `create_pre_restore_backup` | plain `fs::copy` at `src/commands/restore_cmd.rs:804` |

Callers of path 1: `src/library/persistence.rs:342` (in `save_library_internal`; failure logged as
a warning at `:343`), `src/commands/mod.rs:319` (legacy `save_snippets_at`; warning at `:320`), and
`src/commands/import_cmd.rs:447` (failure propagated with `?`). Path 3 is created at
`src/commands/repair_cmd.rs:215`, only when `apply` is true and a safe repair exists.

### 3.2 Symlink refusal today — the roadmap's framing is incomplete

| Path | Refuses a symlink source? | Evidence |
|------|--------------------------|----------|
| 1 | **No** | `src/library/persistence.rs:411` is a bare `fs::copy`, which follows symlinks. |
| 2 | **Yes** | `src/commands/backup_cmd.rs:96` calls `validate_canonical_containment`; `:97` calls `read_for_snapshot`, which rejects a symlink at `src/commands/backup_archive.rs:197-205` and a non-regular file at `:206-215`. |
| 3 | **No** | `src/commands/repair_cmd.rs:925` is a bare `fs::copy`, which follows symlinks. |
| 4 | **Partially** | `src/commands/restore_cmd.rs:786-792` refuses a symlink source — but by **skipping it with a `tracing::warn!`**, not by failing. Non-regular files are skipped the same way at `:793-799`. The hazard is named in the source comment at `:771-777`. |

**Premise correction.** The roadmap's M002 entry says "restore_cmd explicitly refuses symlinks and
non-regular files" and that "repair_cmd's `copy_dir_recursive` and `persistence::backup_library`
perform a plain `fs::copy` with no such check". The second half is **true**. The first half is
true of `restore_cmd`, but it is **not true that restore is the only path that refuses**: `snp
backup` already refuses symlinked library files through two independent checks (containment at
`src/commands/backup_cmd.rs:96`, then the `read_for_snapshot` symlink test at
`src/commands/backup_archive.rs:197-205`). The roadmap's exit condition — "a symlinked library
file is refused by every backup path, not only by restore" — is therefore misleading about the
current state: the gap is in paths 1 and 3 (and in path 4's silent-skip policy), not in path 2.

Two tests already pin path 2's refusal and must not regress:
`tests/backup_contracts.rs:53` `test_backup_rejects_symlinked_library` and
`tests/backup_snapshot_concurrency.rs:156` `test_backup_rejects_symlink_source` (plus `:456`
`test_backup_rejects_non_regular_file`).

### 3.3 Durability today — the postures are not uniform

| Path | fsyncs the produced material? | Durability class claimed |
|------|-------------------------------|--------------------------|
| 1 | **Yes, best-effort** | none — `sync_backup_file` (`src/library/persistence.rs:427-441`) opens the copy and calls `sync_all()`; the flush failure is logged as a warning at `:437`, not propagated. **The containing directory is never fsynced**, so the new `.bak` directory entry is not guaranteed durable. |
| 2 | **Yes** | `Durability::DurableUserData` — `src/commands/backup_archive.rs:310-319` calls `atomic_write_bytes` with that class, which fsyncs the file (`src/utils/atomic.rs:428-434`) and the parent directory (`src/utils/atomic.rs:460`); the staging directory is fsynced at `src/commands/backup_archive.rs:326`. |
| 3 | **No** | none — `fs::copy` at `src/commands/repair_cmd.rs:925`, `:896`, `:904` writes bytes with no flush at all. |
| 4 | **No** | none — `fs::copy` at `src/commands/restore_cmd.rs:804`. |

The roadmap's statement that "the repair backup — the rollback point for a repair operation — is
never fsynced" is **true**, and it is the most consequential gap in this milestone.

The atomic layer's durability classes are the only sanctioned vocabulary
(`src/utils/atomic.rs:23`, documented at `architecture/persistence.md:60-88`):
`DurableUserData` (`:26`) fsyncs the file **and** the parent directory and fails closed on a
parent-dir fsync error; `SensitiveConfig` (`:29`) adds `mode(0o600)` and rejection of a symlink
*destination* (`src/utils/atomic.rs:55`); `RecoverableMetadata` (`:32`) performs **no fsync at
all** — the explicitly "rebuildable" class.

### 3.4 The repair backup is not a rollback point today

`architecture/commands/repair_cmd.md:137` states that the repair backup "is therefore **not**
restorable by `snp restore` (which requires a `manifest.toml`); it is a manual escape hatch."
Verified: `create_repair_backup` writes `libraries/`, `libraries.toml`, and `usage.toml`
(`src/commands/repair_cmd.rs:886-906`) and **no `manifest.toml`**, while `snp restore` requires
one. The layout the repair backup already produces maps one-to-one onto the `snp backup` snapshot
layout, and `BackupEntryKind` already has an `Usage` variant
(`src/commands/backup_archive.rs:16`), so a repair backup **can** become a restore-consumable
snapshot without changing the restore contract.

### 3.5 Two documentation defects in the files this milestone touches

- `architecture/commands/repair_cmd.md:139` says the `libraries.toml` and `usage.toml` copies use
  `let _ = fs::copy(...)` and that "a copy failure is silently ignored (`:895`, `:901`)". That is
  **false at this baseline**: `src/commands/repair_cmd.rs:896-897` and `:904-905` propagate the
  error with `?` through `SnipError::io_error`.
- `architecture/overview.md:459` says "All four classes apply `mode(0o600)`" and describes a
  "Windows hazard" at `:468-474` in which every durable write on Windows fails after the data was
  committed. Both are **stale**: `Durability` has three variants at this baseline
  (`src/utils/atomic.rs:23-38`), and the Windows hazard was fixed — `src/utils/atomic.rs:232-239`
  now treats `ErrorKind::Unsupported` as a warning and returns `Ok(Some(false))`. The same stale
  hazard text is repeated at `architecture/utils/atomic.md:100-111` and
  `docs/PERSISTENCE_INVENTORY.md:370-375`.

### 3.6 Other copy sites of configuration and library material

| Site | What is copied | Error policy today |
|------|----------------|---------------------|
| `src/library/persistence.rs:216-217` | a corrupt library file → `<name>.toml.corrupt.bak` | best-effort (warn) |
| `src/commands/mod.rs:274` | a corrupt legacy `snippets.toml` → `.corrupt.bak` | best-effort (warn) |
| `src/library/manager.rs:63` and `:113` | a corrupt `libraries.toml` → `.corrupt.bak` | best-effort (warn) |
| `src/usage.rs:148` `backup_corrupt_file`, copy at `:150` | a corrupt `usage.toml` → `.corrupt.bak` | best-effort (warn); `:145-147` documents usage history as "low-value data" |
| `src/config/sync_settings.rs:538` | a corrupt `sync.toml` → `.corrupt.bak` | best-effort |
| `src/commands/sync_cmd.rs:864` | a corrupt auto-sync status file → quarantine directory | best-effort; the containing directory **is** fsynced afterwards |

These are the "corrupt-file forensic" copies. They are copies of configuration and library
material, so the roadmap's exit condition ("every copy of configuration or library material goes
through the shared primitive") reaches them; work package F brings them in, with their **error
policy explicitly preserved**.

## 4. Invariants that must not regress

1. **Named durability classes keep their meaning** (`architecture/persistence.md:60-88`,
   `src/utils/atomic.rs:23-38`). The primitive claims one of the three existing classes; it must
   not invent a fourth, nor widen or narrow what an existing class means.
2. **The atomic write layer stays the only path that mutates a file on disk.** A copy that
   produces a destination must land through `atomic_replace` (`src/utils/atomic.rs:376`), so a
   partially-written destination is never observable.
3. **The mutation gate is not bypassed.** The primitive performs no repository mutation and must
   not be reachable from a path that skips `gate_mutation_on_interrupted_transactions()`
   (`architecture/overview.md` § 7). Path 1 runs inside `save_library_internal`
   (`src/library/persistence.rs:337`), whose callers already hold the local-data lock (`:340`);
   that ordering must not change.
4. **The save path does not post-process TOML.** A copy is a byte copy; it must never parse,
   re-serialize, or normalize what it copies (`architecture/overview.md` § 7; golden-corpus
   contract in `.skills/persistence-and-toml-fidelity.md`).
5. **The golden corpus survives save/load.** `golden_corpus()` (`tests/support/mod.rs:157`, 24
   edge-case commands) must stay byte-identical; the pinned check is
   `tests/local_contracts.rs:217` `test_golden_corpus_preserves_exact_text`.
6. **The `snp data` mirror group is unchanged** — `validate`, `backup`, `restore`, `repair`,
   `status`, `delete` (`src/main.rs`, `DataCommands`; `docs/COMMAND_CONTRACTS.md:44`). No command
   or spelling is added, so the `DataCommands` variants, the dispatch arms, and the
   `command_behavior()` arms are untouched.
7. **Exit codes and the stdout/stderr split are unchanged.** `snp backup` exits 0 or 1
   (`docs/COMMAND_CONTRACTS.md:26`), `snp repair` exits 0, 1, or 10
   (`docs/COMMAND_CONTRACTS.md:28`). A refusal introduced here is a **runtime error with exit
   code 1** — never a new exit code, never a change of stream.
8. **Read-only paths stay read-only.** `snp validate` and `snp doctor` copy nothing.
9. **No credential is copied by a backup.** `snp backup` redacts the API key when sync state is
   included (`tests/backup_contracts.rs:140`, default mode at `:98`); the primitive must not
   change redaction.
10. **`test-support` seams stay compiled out of production builds** — the primitive must not add a
    non-gated environment-variable seam (`scripts/ci/test-production-seams.sh` greps the production
    binary for seam names).

## 5. Scope

### In scope

- One shared copy primitive in `src/utils/atomic.rs`, exposing a **source-admission** half
  (refuses a symlink source, a non-regular source, and — when a root is supplied — a source that
  resolves outside the configuration directory) and a **copy** half (admission, then landing
  through `atomic_replace` under a named `Durability` class).
- Migration of the four backup paths in § 3.1 onto that primitive.
- Refusal and fsync become properties of the primitive, not of one call site: the inline checks in
  `src/commands/backup_archive.rs:194-217` (`read_for_snapshot`) and
  `src/commands/restore_cmd.rs:779-802` (`copy_regular_file`'s admission) are replaced by the
  primitive's admission half rather than duplicated.
- The pre-repair backup becomes restorable by `snp restore` (it gains a `manifest.toml` produced
  by the same manifest writer `snp backup` uses).
- Migration of the six corrupt-file forensic copy sites in § 3.6, with their error policy
  preserved.
- Documentation (§ 12) and a `CHANGELOG.md` entry.

### Explicitly out of scope

- **The transaction journal's own rollback mechanism** (`src/transaction.rs`) — a separate
  subsystem with its own state machine and closure gate; the roadmap calls it "a fifth rollback
  mechanism with its own rules". Its crash-failpoint suites are still **run** as a regression gate
  (§ 11).
- **`src/update.rs:788` and `:846`** — these copy *updater artifacts*, not configuration or
  library material.
- **`src/utils/config.rs:137` `copy_recursively`** (copy at `:145`) — the legacy
  *configuration-directory migration*, not a backup, snapshot, or rollback point; it moves a whole
  directory tree and is already followed by its own recursive fsync pass.
- **`src/commands/sync_cmd.rs:864`** — the auto-sync status quarantine copy. It already fsyncs the
  destination directory and copies a transient worker-status file, not configuration or library
  material.
- **Changing `snp backup`'s refusal behavior.** Path 2 already refuses symlinks; this milestone
  preserves that, it does not relax it.
- **Changing any command spelling, exit code, stdout/stderr split, or JSON report schema.**
- **Making `snp repair` or `snp restore` acquire the local-data lock.** The pre-repair backup
  (`src/commands/repair_cmd.rs:215`) and the pre-restore backup take no lock today. That is a
  pre-existing gap, recorded in § 8 as a known risk rather than fixed here (fixing it changes the
  lock hierarchy and belongs to a separate milestone).

## 6. Required production changes

| Area | Change |
|------|--------|
| Core/domain | `src/utils/atomic.rs` — add the shared copy primitive (API in work package A). It must import nothing outside `crate::error`, so it stays in the core layer (`docs/LOGICAL_LAYERS.md`) and is reachable from `src/commands/*` and `src/library/*` without a layer inversion. `src/commands/backup_archive.rs` — `read_for_snapshot` (`:194`) becomes a wrapper over the primitive's admission half; `atomic_write_backup` (`:289`) keeps its signature and its `Durability::DurableUserData` choice. |
| Storage | `src/library/persistence.rs` — `backup_library` (`:381`) and `sync_backup_file` (`:427`) are replaced by the primitive; the **caller-visible error policy is unchanged** (`src/library/persistence.rs:342`, `src/commands/mod.rs:319` warn; `src/commands/import_cmd.rs:447` propagates). `sync_backup_file` disappears because `DurableUserData` fsyncs the file **and** the parent directory — strictly stronger than the file-only best-effort flush it replaces. `src/commands/repair_cmd.rs` — `create_repair_backup` (`:873`) is rewritten to produce a manifest-backed snapshot through the same writer `snp backup` uses; `copy_dir_recursive` (`:912`) is removed. `src/commands/restore_cmd.rs` — `create_pre_restore_backup` (`:756`) and `copy_regular_file` (`:778`) are replaced by the primitive; the restore's transaction, journal, and manifest handling are untouched. The six forensic sites in § 3.6 are migrated with their best-effort error policy preserved. |
| Runtime | The primitive performs no locking of its own and inherits the caller's context: path 1 runs under the local-data lock (`src/library/persistence.rs:340`); path 2 takes the lock itself (`src/commands/backup_cmd.rs:73`); paths 3 and 4 take no lock today, unchanged here. |
| Operator surface | No new command, flag, or spelling. The pre-repair backup directory gains a `manifest.toml`; the repair JSON report's schema is unchanged because it carries only the backup **path** (`src/commands/repair_cmd.rs:216`), not the directory's contents. |
| Security | The refusal message names the offending path and the reason (symlink / non-regular file / outside the configuration root), replacing path 4's `tracing::warn!`-and-continue behavior (`src/commands/restore_cmd.rs:786-792`). |
| Docs and guards | Update § 12. Add a check that fails when a direct `fs::copy` of configuration or library material is introduced outside a small allow-list holding only the sites § 5 declares out of scope, each with a written reason. |

## 7. Ordered work packages

### Work package A — Add the shared copy primitive to the atomic layer

**Required changes**

- `src/utils/atomic.rs`:
  - `pub enum CopySourcePolicy` — `Contained { root: PathBuf }` and `RegularFileOnly`.
  - `pub struct CopyOptions { pub source: CopySourcePolicy, pub durability: Durability }`.
  - `pub fn read_copy_source(src: &Path, options: &CopyOptions) -> SnipResult<Vec<u8>>`:
    (1) `symlink_metadata(src)` — a symlink source is refused with a message naming the path;
    (2) the source must be a regular file — a directory, FIFO, socket, or device is refused;
    (3) under `Contained { root }`, canonicalize source and root and refuse a source that does not
    resolve under the root; (4) read the bytes.
  - `pub fn copy_file(src: &Path, dst: &Path, options: &CopyOptions) -> SnipResult<AtomicWriteReport>`:
    call `read_copy_source`, then land the bytes with `atomic_replace` under `options.durability`
    and return the report.
- The durability class is **`Durability::DurableUserData`** at every production call site. The
  copied material is user-visible data (library files, `libraries.toml`, `usage.toml`) that must
  survive power loss — exactly what `DurableUserData` means (`src/utils/atomic.rs:26`,
  `architecture/persistence.md:60-88`); it is the only class that fsyncs the file **and** the
  parent directory and fails closed when the parent-dir fsync fails
  (`src/utils/atomic.rs:216-267`); `RecoverableMetadata` does not fsync at all
  (`src/utils/atomic.rs:34`) and is disqualified by the milestone's second objective; and
  `SensitiveConfig` (`src/utils/atomic.rs:30`) adds destination-symlink rejection and is
  documented for credentials — a library file is not a credential, and `snp backup` already writes
  `sync.toml` under `DurableUserData` (`src/commands/backup_archive.rs:313`).
- **Important:** `DurableUserData` does **not** enable `reject_symlink` — that flag is set only
  for `SensitiveConfig` (`src/utils/atomic.rs:55`) and it governs the *destination*, not the
  source. The source refusal must therefore be implemented by the primitive itself. State this
  in the primitive's doc comment so a later reader does not "fix" it by flipping the flag.
- The primitive must not import from `crate::commands`, `crate::ui`, `crate::logging`,
  `crate::auto_sync`, `crate::clipboard`, `crate::sync_commands`, or `crate::sync`
  (`docs/LOGICAL_LAYERS.md`, core enforced set).

**Acceptance evidence**

- Unit tests in `src/utils/atomic.rs`: symlink refused and the message names the path;
  non-regular refused; a source outside the root refused while one inside is admitted; a
  successful copy is byte-identical; a copy over an existing destination replaces it atomically.
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.

### Work package B — Migrate path 1 (per-library `.bak`)

**Required changes**

- `src/library/persistence.rs`: `backup_library` (`:381`) uses `copy_file` with
  `CopySourcePolicy::Contained { root: <the libraries directory> }` and
  `Durability::DurableUserData`; `sync_backup_file` (`:427`) is deleted; the rotation and
  retention logic (`:399`) is unchanged.
- The **caller-visible error policy is unchanged**: `src/library/persistence.rs:342` and
  `src/commands/mod.rs:319` still warn and proceed; `src/commands/import_cmd.rs:447` still
  propagates.

**Acceptance evidence**

- A saved library produces a `.bak` whose bytes equal the pre-save bytes, and the `.bak` is a
  **regular file** even when the library path is a symlink — the refusal must not be expressed by
  "copy the target"; and a save against a symlinked library path copies none of the target's
  content, with a warning that names the path.

### Work package C — Migrate path 2 (`snp backup`)

**Required changes**

- `src/commands/backup_archive.rs`: `read_for_snapshot` (`:194`) is replaced by a wrapper over
  `read_copy_source`; the inline symlink and non-regular checks at `:197-215` are removed.
- `src/commands/backup_cmd.rs`: the admission call sites at `:96-97`, and the analogous ones for
  the index (`:103-104`), usage (`:110-111`), and sync config (`:118`), use the shared admission
  half with `CopySourcePolicy::Contained { root: <canonical config dir> }`.
- `atomic_write_backup` (`src/commands/backup_archive.rs:289`) keeps its behavior, including
  `Durability::DurableUserData` at `:310-319`, the staging-directory fsync at `:326`, the rename at
  `:332-334`, and the parent fsync at `:338-339`.
- **No user-visible change.** The refusal messages must stay compatible with the existing
  assertions in `tests/backup_contracts.rs:89` and `tests/backup_snapshot_concurrency.rs:190`,
  which accept either "symlink" or "escapes config root".

**Acceptance evidence**

- `tests/backup_contracts.rs` and `tests/backup_snapshot_concurrency.rs` stay green unchanged; a
  new test shows a `snp backup` of a symlinked-library config directory fails, leaves no partial
  snapshot, and names the path.

### Work package D — Migrate path 3 (pre-repair backup) and make it a valid rollback point

**Required changes**

- `src/commands/repair_cmd.rs`:
  - `create_repair_backup` (`:873`) is rewritten to build a manifest-backed snapshot through the
    same manifest writer `snp backup` uses (`src/commands/backup_archive.rs:289`), so the result
    carries a `manifest.toml` with `BackupEntryKind::{Library, Index, Usage}` entries and the
    SHA-256 hashes the restore side verifies;
  - `copy_dir_recursive` (`:912`) is deleted;
  - the directory name keeps the existing `repair-<timestamp>-<uuid>` shape
    (`src/commands/repair_cmd.rs:880`) so operators' tooling still recognizes it;
  - the snapshot is written to a staging directory and renamed into place, so a partially-written
    backup is never presented as complete.
- The repair backup **refuses** a symlinked or non-regular source and **fails the repair** rather
  than proceeding with an incomplete rollback point. This is a deliberate, visible behavior change
  and must be recorded in `CHANGELOG.md`. The refusal message names the path.

**Acceptance evidence**

- `snp repair --apply` against a symlinked-library config directory **fails**, applies nothing,
  and names the path.
- A successful `snp repair --apply` leaves a backup directory `snp restore` accepts: restoring
  from it yields library bytes equal to the pre-repair bytes. This is the "valid rollback point"
  proof and the milestone's headline acceptance test.
- The repair backup contains a `manifest.toml` whose entry hashes match the files it describes.

### Work package E — Migrate path 4 (pre-restore backup)

**Required changes**

- `src/commands/restore_cmd.rs`:
  - `copy_regular_file` (`:778`) is deleted; its admission checks (`:786-799`) are replaced by the
    primitive's admission half;
  - `create_pre_restore_backup` (`:756`) uses `copy_file` with
    `CopySourcePolicy::Contained { root: <canonical config dir> }`;
  - the hazard comment at `:771-777` moves to the primitive's doc comment, which becomes the one
    place that names it.
- **Behavior change, deliberate and recorded:** a symlinked or non-regular source is now a **hard
  error** that aborts the restore, instead of a `tracing::warn!` that skipped the entry and
  continued (`:786-792`). Rationale: a pre-restore backup that silently omits a library is not a
  valid rollback point, and the roadmap requires "never leave a partially copied rollback point
  presented as complete".
- The restore's transaction, journal, and manifest handling are untouched.

**Acceptance evidence**

- A restore against a symlinked-library config directory fails before any live configuration is
  modified, and the message names the path; `tests/restore_security.rs`,
  `tests/restore_transactions.rs`, `tests/restore_manifest_gate.rs`, and
  `tests/manifest_contracts.rs` stay green.

### Work package F — Migrate the corrupt-file forensic copies

**Required changes**

- The six sites in § 3.6 use the primitive's copy half with
  `CopySourcePolicy::Contained { root: <the owning directory> }` and `Durability::DurableUserData`.
- The **error policy is preserved exactly**: every one of these sites is best-effort today and
  stays best-effort. In particular `src/usage.rs:145-147` documents usage history as "low-value
  data" whose "loading never refuses to proceed"; that decision is unchanged.
- The distinction between "the copy failed" and "the file is a symlink" is preserved in the log
  message: a refused source is logged with the reason and the path, not as a generic copy error.

**Acceptance evidence**

- Per family (library, index, usage), a corrupt file produces a `.corrupt.bak` that is a regular
  file with the same bytes as the corrupt original; a corrupt file that is a **symlink** copies
  none of the target's content into the `.corrupt.bak`.

### Work package G — Documentation, static guard, and changelog

**Required changes**

- Add the static guard described in § 6. Update the documents listed in § 12. Correct the two
  stale documentation defects recorded in § 3.5. Add a `CHANGELOG.md` entry recording the two
  visible behavior changes and the repair backup's new `manifest.toml`.

**Acceptance evidence**

- `bash scripts/check.sh` passes; the static guard fails when a deliberate `fs::copy` is added to
  `src/commands/repair_cmd.rs` (proved during implementation, not in the merge commit).

## 8. Failure, cancellation, restart, and contention semantics

**Power loss during a copy.** The destination is written to a temporary file in the destination
directory and renamed (`src/utils/atomic.rs:376-460`), so a power loss leaves either the previous
destination or the new one — never a truncated file at the destination path.
`Durability::DurableUserData` fsyncs the contents before the rename
(`src/utils/atomic.rs:428-434`) and the parent directory after it (`src/utils/atomic.rs:460`); a
power loss after the rename but before the directory fsync may leave the previous destination
reappearing. On Windows the parent directory cannot be fsynced through `std`
(`src/utils/atomic.rs:170-179`); the limitation is reported as a warning and the result is
`Ok(Some(false))` (`src/utils/atomic.rs:232-239`), not an error. The Windows guarantee is
therefore "the file's content survives", not "the directory entry survives" — the existing
documented platform limitation. **Any test or doc claiming a Windows directory-entry guarantee is
wrong** (see § 3.5).

**A partially-written backup.** No path may present a partially written backup as complete. Path 2
already writes to a staging directory and renames (`src/commands/backup_archive.rs:326-339`) and
removes the staging directory on failure (`:343-345`); path 3 must adopt the same discipline (work
package D); paths 1 and 4 land through `atomic_replace` and are inherently partial-copy-free; the
forensic copies write to a `.corrupt.bak` sibling through the primitive, so a failed forensic copy
leaves the previous `.corrupt.bak` intact rather than a truncated one.

**A symlink that points outside the configuration directory.** The primitive refuses it, with an
error carrying the path and the reason. The refusal is decided on the **source**, before any bytes
are read, so no content from outside the configuration directory can reach a backup through the
migrated paths. Path 2 refuses today and must keep refusing (work package C preserves the message
wording the existing tests assert); paths 1 and 3 gain the refusal (work packages B and D); path 4
changes from "skip with a warning" to "fail the restore" (work package E).

**A source that disappears mid-copy.** The source is read into memory before the destination is
written, so a source deleted between the admission check and the read fails the read, and the copy
fails with an I/O error; no destination is created, because the write happens only after a
successful read. A source *replaced* between the check and the read is read at its new contents —
the copy is a snapshot of whatever the read returned — and the atomic landing guarantees the
destination is wholly the old content or wholly the new content.

**The rollback-point validity of the repair backup.** "Valid" has three parts and all three must
hold: (1) **complete** — no source is silently omitted, and a refused source fails the repair;
(2) **durable** — the material is fsynced under `Durability::DurableUserData`, including the
parent directory; (3) **restorable** — the backup carries a `manifest.toml` that `snp restore`
accepts. The backup is created **before** any repair is applied
(`src/commands/repair_cmd.rs:215`, before the apply loop at `:218`); that ordering must not change,
so a failed backup means no repair runs.

**Contention.** The primitive takes no locks. Path 2 keeps taking the local-data lock
(`src/commands/backup_cmd.rs:73`); path 1 keeps running under the caller's lock
(`src/library/persistence.rs:340`); paths 3 and 4 keep their current unlocked behavior. The
pre-repair backup is **not** taken under the local-data lock — a pre-existing gap (§ 5), recorded
as a known risk: a concurrent writer could change a library between the backup's read and the
repair's write, so the backup may not correspond exactly to the pre-repair state. This milestone
does not fix it and must not make it worse.

## 9. Compatibility and migration

- **Command spellings, aliases, exit codes, stdout/stderr split, and machine-output schemas are
  unchanged.** No `command_behavior()` entry and no `docs/COMMAND_CONTRACTS.md` row are added,
  because no command is added. The `snp data` mirror group is unchanged.
- **Two visible behavior changes**, both in `CHANGELOG.md`:
  1. `snp repair --apply` now **fails** when the configuration directory contains a symlinked or
     non-regular library file, instead of silently copying the symlink's target into the backup.
  2. `snp restore` now **fails** before touching the live configuration when the configuration
     directory contains a symlinked or non-regular library file, instead of skipping that entry in
     the pre-restore backup.
  - Both turn a silent data-integrity hazard into a visible refusal: **exit code 1**, message on
    **stderr**.
- **One artifact-layout change**: the pre-repair backup directory gains a `manifest.toml`. The
  directory name shape (`repair-<timestamp>-<uuid>`) is unchanged and the repair JSON report's
  schema is unchanged (it carries the backup path, not its contents).
- **No data migration.** Nothing on disk is rewritten by this milestone.
- **Rollback**: reverting the code restores the previous behavior. A repair backup produced by the
  new code is still a directory of files — it is *additionally* restorable — so a revert loses
  that capability but loses nothing.

## 10. Required tests

### Focused unit tests

- `src/utils/atomic.rs` unit tests for the primitive (work package A's acceptance evidence).
- A test that the primitive claims `Durability::DurableUserData` and that the class's documented
  meaning is unchanged: the destination's content is fsynced and the destination's parent
  directory is fsynced.

### Integration tests

- **Symlink refusal on every backup path — not only restore.** One test per path, all four:
  (1) path 1 — `backup_library` refuses a symlinked library and does not copy the target;
  (2) path 2 — `snp backup` refuses a symlinked library (already covered by
  `tests/backup_contracts.rs:53` and `tests/backup_snapshot_concurrency.rs:156`; the new test
  asserts the message names the path); (3) path 3 — `snp repair --apply` refuses and applies
  nothing; (4) path 4 — `snp restore` refuses before modifying the live configuration.
- **The fsync property**: a test that a copy produced by the primitive is a regular file whose
  content is durable — verified by asserting the primitive routes through `atomic_replace` with
  `Durability::DurableUserData`, plus an integration test that the destination exists with the
  full content after the copy returns. A true power-loss test is not possible in-process, so the
  assertion is that the durable path is taken, backed by the atomic layer's existing class tests.
- **The golden-corpus round trip**: the corpus must survive the change —
  `tests/local_contracts.rs:217` `test_golden_corpus_preserves_exact_text` and the round trips in
  `tests/integration.rs:2111` and `:2159`. Because the corpus is about *command* text, add a test
  that a library saved through the migrated path 1 keeps byte-exact TOML (tabs, trailing spaces,
  CRLF) — the file fidelity the milestone could regress.
- **The repair backup is a valid rollback point (restorable)**: run `snp repair --apply` on a
  repository with a repairable defect, capture the pre-repair library bytes, then run `snp restore`
  against the repair backup directory and assert the restored library bytes equal the captured
  pre-repair bytes. This is the milestone's headline test.
- **A source that disappears mid-copy**: a copy whose source is removed before the read fails with
  an error and creates no destination.
- **A partially written backup**: a failing copy leaves the previous destination intact and never
  a truncated destination.

### Restart, recovery, contention, and cancellation tests

- `tests/restore_crash_failpoints.rs` and `tests/cleanup_crash_failpoints.rs` must be run by hand
  (§ 11): the restore path's crash boundaries must be unaffected by the pre-restore backup change.
- `tests/backup_snapshot_concurrency.rs` (whole file) stays green — it exercises concurrent
  backups and the symlink/non-regular refusals.
- `tests/repair_transactions.rs` stays green — it covers the repair path's transaction
  interaction.

### Security and negative tests

- A test that no migrated path copies content from outside the configuration directory into a
  backup: point a symlink at an external file and assert the external file's content does not
  appear anywhere under the backup directory.

### Migration and compatibility tests

- `tests/backup_contracts.rs` (whole file) — unchanged, including
  `test_backup_does_not_include_api_key_in_default_mode` (`:98`) and
  `test_backup_redacts_api_key_when_sync_state_included` (`:140`): redaction must be unaffected.
- `tests/manifest_contracts.rs` (whole file) — the repair backup's new `manifest.toml` must
  satisfy the same contract the `snp backup` manifest satisfies.

## 11. Required verification commands

Run these in order. **Never run a cargo command while another test run is in flight** — the
integration tests execute `target/debug/snp`, and a concurrent rebuild replaces that binary
mid-run and produces false failures (most visibly in the crash-failpoint suites).

```bash
# Focused: the new primitive and the migrated paths
cargo test --features test-support --lib atomic
cargo test --features test-support --test backup_contracts
cargo test --features test-support --test backup_snapshot_concurrency
cargo test --features test-support --test manifest_contracts
cargo test --features test-support --test local_contracts

# Serial targets (one at a time; these are serial by contract)
cargo test --features test-support --test repair_transactions -- --test-threads=1
cargo test --features test-support --test restore_manifest_gate -- --test-threads=1
cargo test --features test-support --test restore_security -- --test-threads=1
cargo test --features test-support --test restore_transactions -- --test-threads=1

# Crash / restore suites: invoked by NO script — run them by hand
cargo test --features test-support --test restore_crash_failpoints -- --test-threads=1
cargo test --features test-support --test cleanup_crash_failpoints -- --test-threads=1

# Lint gates (NOT --all-features; the six required-features targets are
# never linted by check.sh and must be linted explicitly)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Authoritative Linux CI gate
bash scripts/check.sh
```

Notes:

- `repair_transactions`, `local_data_lock_barriers`, and `process_lock_concurrency` are the serial
  targets; `repair_transactions` is in the list above and must run with `-- --test-threads=1`.
- `cleanup_crash_failpoints` and `restore_crash_failpoints` are invoked by **no** script
  (`scripts/check.sh`, `scripts/release-check.sh`, `scripts/ci/test-production-seams.sh` do not
  mention them). They must be run by hand before merging a restore or cleanup change, and this
  milestone changes the restore path's pre-backup.
- Those suites depend on `SNP_TEST_FAILPOINT`, which exists only in a binary built with
  `test-support`. Build with `--features test-support` and do not rebuild the binary while they
  run.
- `bash scripts/check.sh` is the authoritative Linux CI gate and is equivalent to CI.

## 12. Documentation updates

| Document | Change |
|----------|--------|
| `architecture/utils/atomic.md` | Add the copy primitive to the API section (`:49`) and the durability-classes section (`:60`); record that the source refusal lives in the primitive, not in `Durability::SensitiveConfig`'s destination-symlink flag; correct the stale Windows-hazard text at `:100-111`; refresh the line references the edit touches. |
| `architecture/persistence.md` | Record the copy primitive in the atomic-write section (`:43-88`) and state which durability class a copy claims and why. |
| `architecture/overview.md` § 7 | Update the durability table (`:455-475`): the class count is three, not four; the Windows hazard at `:468-474` is stale; add the copy primitive's class. |
| `architecture/commands/backup_cmd.md` | State that the snapshot is produced through the shared copy primitive and that the source refusal is inherited. |
| `architecture/commands/backup_archive.md` | Document the shared admission half that replaced `read_for_snapshot`. |
| `architecture/commands/repair_cmd.md` | Record the manifest-backed repair backup, the new refusal semantics, the restored rollback-point property, and correct the false "silently ignored" claim at `:139`. |
| `architecture/commands/restore_cmd.md` | Record the pre-restore backup's new refusal semantics and the shared primitive. |
| `docs/PERSISTENCE_INVENTORY.md` | Update the backup-snapshot artifact entry (`:232`), the durability-classes section (`:331`), and correct the stale Windows-hazard text at `:370-375`. |
| `docs/COMMAND_CONTRACTS.md` | No new row (no new command). Update the `repair` row only if the pre-repair backup's artifact description must mention the manifest. |
| `CHANGELOG.md` | One entry describing the shared copy policy, the two refusal behavior changes, and the repair backup's new `manifest.toml`. |

## 13. Acceptance criteria

1. One shared copy primitive exists in `src/utils/atomic.rs`, and **all four** backup paths in
   § 3.1 use it. No path performs its own copy, its own symlink check, or its own fsync.
2. The primitive claims a **named, existing** durability class — `Durability::DurableUserData` —
   and no fourth class is introduced.
3. A symlinked library file is refused by **all four** paths, and each refusal names the path; a
   non-regular source is refused by all four; a source resolving outside the configuration
   directory is refused by all four; and no content from outside the configuration directory
   appears in any produced backup.
4. The material every migrated path produces is fsynced, including the parent directory, under the
   named class.
5. The pre-repair backup is **restorable by `snp restore`**: a test runs a repair, restores from
   the repair backup, and asserts the restored bytes equal the pre-repair bytes.
6. The golden corpus is unchanged: `tests/local_contracts.rs:217` and
   `tests/integration.rs:2111`/`:2159` stay green, and a new test proves byte-exact TOML fidelity
   through the migrated path.
7. `tests/backup_contracts.rs`, `tests/manifest_contracts.rs`, `tests/restore_manifest_gate.rs`,
   `tests/repair_transactions.rs`, `tests/backup_snapshot_concurrency.rs`,
   `tests/restore_security.rs`, and `tests/restore_transactions.rs` are green.
8. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
   `bash scripts/check.sh` all pass, and the two hand-run crash-failpoint suites pass.
9. No command spelling, flag, exit code, stdout/stderr split, or machine-output schema changed,
   and the `snp data` mirror group is unchanged.
10. A static guard rejects a new direct `fs::copy` of configuration or library material outside the
    declared allow-list.
11. The two stale documentation defects in § 3.5 are corrected, and `CHANGELOG.md` records the two
    visible behavior changes and the artifact-layout change.

## 14. Stop conditions

Stop and report to the parent session — do **not** improvise — if any of the following holds:

1. Making the pre-repair backup restorable would require changing `snp restore`'s manifest
   contract, its exit codes, or its JSON output. Restorability is to be achieved by producing the
   **same** manifest artifact `snp backup` produces; if that is not achievable, stop rather than
   widening the restore contract.
2. The shared primitive would have to live outside the atomic/persistence layer (for example in
   `src/commands/`), which would invert the layer rules in `docs/LOGICAL_LAYERS.md`.
3. A durability class would have to be widened, narrowed, or added to express "fsync the copy".
   The requirement is satisfiable with the existing three.
4. The static guard would have to forbid a `fs::copy` in a path § 5 declares out of scope
   (`src/update.rs`, `src/utils/config.rs`, `src/commands/sync_cmd.rs`) — the allow-list must
   stay small and each entry must carry a written reason.
5. Any existing test in `tests/backup_contracts.rs`, `tests/manifest_contracts.rs`,
   `tests/restore_manifest_gate.rs`, `tests/repair_transactions.rs`, or
   `tests/backup_snapshot_concurrency.rs` fails for a reason that cannot be fixed without
   weakening an existing refusal or durability guarantee.
6. The restore crash-failpoint suites fail after the work lands. Do not "fix" them by relaxing a
   crash boundary; report the exact failing failpoint.
7. The work would have to touch `src/transaction.rs`'s journal and rollback mechanism — a separate
   milestone.

## 15. Closure evidence required

- The closure record must cite, with `file:line` at the closing commit: the shared primitive's
  definition and its `Durability` claim; each of the four migrated paths' new call site; the six
  forensic copy sites' new call sites, if work package F landed; the two stale documentation
  defects, now corrected; and the new artifact-layout change (the repair backup's
  `manifest.toml`).
- It must list, by name, every test added and every existing suite re-run, with the exact
  commands and their results, and include the output of `cargo fmt --all -- --check`, both
  `cargo clippy` invocations, `bash scripts/check.sh`, and the two hand-run crash-failpoint
  suites.
- It must state explicitly which of the § 3.5 documentation defects were fixed and which (if any)
  were not, with the reason, and record the two visible behavior changes in the same wording as
  the `CHANGELOG.md` entry.

## 16. Handoff notes

- **Start with work package A.** Nothing else can land before the primitive exists, and its API
  shape determines how the four migrations read.
- **The primitive's API is a two-half API on purpose.** `snp backup` must read the whole snapshot
  into memory *before* writing any of it, because the capture is taken under the local-data lock
  (`src/commands/backup_cmd.rs:73`) and must be a consistent snapshot. Splitting the primitive
  into an admission half and a copy half lets path 2 keep that property while paths 1, 3, and 4
  use the one-shot copy.
- **Do not implement the source refusal by flipping `reject_symlink`.** That flag
  (`src/utils/atomic.rs:55`) governs the *destination* and is set only for
  `Durability::SensitiveConfig`. The source refusal is the primitive's own job.
- **The two visible behavior changes are the point of the milestone, not accidents.** A silent skip
  is what produced the current asymmetry. Do not soften them to "warn and continue" to keep a test
  green — a failing test means the test encoded the old asymmetry.
- **The repair backup's restorability is achieved by producing the same artifact `snp backup`
  produces**, not by adding a second manifest format and not by teaching `snp restore` a new input
  shape.
- **The restore path's crash boundaries are load-bearing.** Run
  `tests/restore_crash_failpoints.rs` and `tests/cleanup_crash_failpoints.rs` by hand — no script
  runs them.
- **Two documentation defects are known and in scope:** `architecture/commands/repair_cmd.md:139`
  (the "silently ignored" claim is false) and `architecture/overview.md:459`/`:468-474` plus
  `architecture/utils/atomic.md:100-111` and `docs/PERSISTENCE_INVENTORY.md:370-375` (the "four
  classes" and Windows-hazard claims are stale). Fix them in work package G.
- **Adjacent work deliberately not done here:** the pre-repair and pre-restore backups are still
  taken without the local-data lock, and the transaction journal still has its own separate
  rollback mechanism. Both are recorded in § 5 and § 8 as known risks, not as defects this
  milestone was expected to fix.

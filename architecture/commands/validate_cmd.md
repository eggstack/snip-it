# snp validate

[← Back to Overview](../overview.md) · [Command index](mod.md) · [repair_cmd.md](repair_cmd.md) · [doctor_cmd.md](doctor_cmd.md)

`snp validate` performs a **read-only health check** of the local snippet data: it parses every visible library, cross-checks the library index, checks for orphaned usage entries, and inspects file permissions. It reports what it finds as structured diagnostics and never changes anything.

It does **not** repair anything, does **not** migrate legacy state, and does **not** create files.

## CLI arguments

`snp validate` and `snp data validate` share one struct (`src/commands/validate_cmd.rs:18-28`):

```rust
pub struct ValidateArgs {
    /// Validate a specific library
    #[arg(short, long)]
    pub library: Option<String>,
    /// Treat warnings as errors
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub strict: bool,
    /// Output as JSON
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub json: bool,
}
```

| Flag | Type | Default | Effect |
|---|---|---|---|
| `-l`, `--library <NAME>` | `Option<String>` | `None` | Validate only this library. `None` validates all libraries. |
| `--strict` | flag | off | Elevate a designated set of warnings to errors (see [Strict mode](#strict-mode)). |
| `--json` | flag | off | Emit the `ValidationReport` as JSON on **stdout**. |

## The diagnostic model

```rust
pub enum Severity { Info, Warning, Error }

pub enum Repairability { Auto, Manual, Unrepairable }

pub struct ValidationDiagnostic {
    pub code: String,
    pub severity: Severity,
    pub path: Option<PathBuf>,
    pub library: Option<String>,
    pub snippet_id: Option<String>,
    pub message: String,
    pub repairability: Repairability,
}

pub struct ValidationReport {
    pub schema_version: String,
    pub tool_version: String,
    pub strict_mode: bool,
    pub dry_run: bool,
    pub total_libraries: usize,
    pub total_snippets: usize,
    pub diagnostics: Vec<ValidationDiagnostic>,
}
```

(`src/commands/validate_cmd.rs:30-68`.) `schema_version` is `"1.0.0"`, `dry_run` is always `true`, and `tool_version` comes from `crate::diagnostics::version()`.

There is **no generic finding DSL** shared with `doctor` or `import`. `validate` owns `Severity` / `Repairability` / `ValidationDiagnostic` / `ValidationReport`; `doctor` and `import` own the separate `DiagnosticSeverity` / `CompatibilityDiagnostic` / `DoctorReport` / `PetImportReport` family in `src/diagnostics.rs`. Each consumer keeps its own semantics; only the *classification inputs* are shared (see [Shared classifiers](#shared-classifiers)).

## Check categories

### a–g. Per-library checks (`validate_library`, `validate_cmd.rs:133-366`)

| Code | Severity | Repairability | Condition |
|---|---|---|---|
| `E-FILE-READ` | Error | Manual | The library file could not be read (`:144-159`). |
| `I-FILE-EMPTY` | Info | Auto | The library file is empty — no snippets (`:161-173`). |
| `E-TOML-PARSE` | Error | Manual | The raw file is not valid TOML (`:176-191`). |
| `E-DUP-ID` | Error | Auto | The same snippet ID appears more than once (`:196-216`). |
| `W-ID-EMPTY` | Warning | Auto | A snippet has an empty ID — `load_library` assigns IDs on read (`:218-235`). |
| `E-CMD-EMPTY` | Error | Manual | A snippet has an empty command (`:238-253`). |
| `W-DESC-EMPTY` | Warning (Error in strict) | Manual | A snippet has an empty description (`:254-277`). |
| `W-SAME-ID-DIVERGENT` | Warning | Auto | The same ID appears multiple times with **divergent** description/command (`:280-313`). |
| `W-DUP-CHECK-SKIPPED` | Warning | Manual | The raw re-parse failed, so the divergence check was skipped (`:314-325`). |
| `W-EXACT-DUP` | Warning | Manual | Two entries share both description and command (`:327-347`). |
| `W-CORRUPT-BAK` | Warning | Auto | A `<library>.toml.corrupt.bak` file exists, indicating a previous parse failure (`:349-365`). |

### h–j. Index cross-references (`validate_index`, `validate_cmd.rs:374-454`)

| Code | Severity | Repairability | Condition |
|---|---|---|---|
| `E-INDEX-MISSING-FILE` | Error | Manual | A library is registered in the index but its file does not exist (`:377-392`). |
| `W-ORPHAN-FILE` | Warning | Auto | A file exists in `libraries/` but is not registered in the index (`:394-413`). |
| `E-PRIMARY-MISSING` | Error | Manual | The primary library's file does not exist (`:416-431`). |
| `W-NO-PRIMARY` | Warning | Auto | Libraries exist but no primary is set (`:432-451`). |

### k. Orphaned usage entries (`validate_usage`, `validate_cmd.rs:461-531`)

| Code | Severity | Repairability | Condition |
|---|---|---|---|
| `W-LIB-UNREADABLE` | Warning | Manual | A library could not be read, so it was excluded from the orphan computation (`:480-494`). |
| `W-LIB-INDEX-UNREADABLE` | Warning | Manual | The library index could not be read; the orphan check was skipped entirely (`:498-509`). |
| `W-USAGE-ORPHAN` | Warning | Auto | A usage entry references a snippet ID that exists in no library (`:517-530`). |

### l. File permissions (`validate_permissions`, `validate_cmd.rs:534-574`)

Unix only. Checks `snippets.toml`, `libraries.toml`, `sync.toml`, and `usage.toml` in the config directory.

| Code | Severity | Repairability | Condition |
|---|---|---|---|
| `W-INSECURE-PERMS` | Warning | Auto | The file's mode has any group- or world-accessible bit (`mode & 0o077 != 0`); recommended mode is `0600` (`:548-571`). |

## Strict mode

`--strict` has two distinct effects.

### 1. Global warning elevation

After all checks run, the following warning codes are rewritten to `Severity::Error` (`validate_cmd.rs:706-719`):

```rust
const STRICT_CODES: &[&str] = &[
    "W-ID-EMPTY",
    "W-DESC-EMPTY",
    "W-SAME-ID-DIVERGENT",
    "W-EXACT-DUP",
];
```

### 2. Per-check severity selection

`W-DESC-EMPTY` is emitted as `Error` directly when `strict` is true, and as `Warning` otherwise (`validate_cmd.rs:254-277`). The elevation pass then catches it regardless.

Strict mode therefore changes the **exit code**: warnings in the elevated set turn a passing run into a failing one.

## Shared classifiers

`validate` deliberately does **not** re-implement index or usage semantics. It consumes the same shared views that `repair`, `doctor`, and `status` use:

| Shared input | Used by | Where |
|---|---|---|
| `LibraryManager::inspect_library_index()` → `LibraryIndexInspection` (missing files, orphan files, `PrimaryState`) | `validate`, `repair`, `doctor` | `validate_cmd.rs:375`, `repair_cmd.rs:381`, `doctor_cmd.rs:563` |
| `crate::library::find_orphaned_ids(active_ids, usage_ids)` | `validate`, `repair` | `validate_cmd.rs:517`, `repair_cmd.rs:432` |
| `crate::library::readonly_library_sources(...)` | `validate` | `validate_cmd.rs:677-688`, `:471` |

Rendering into `ValidationDiagnostic` stays local to `validate`, which preserves its pass/fail-plus-warnings semantics without dragging in the doctor/import diagnostic families.

## The read-only guarantee

`run()` (`validate_cmd.rs:666-737`) is documented as:

> Read-only: uses `LibraryManager::new` plus the canonical read-only resolver. Never migrates legacy state and never creates files.

Concretely:

- Library discovery goes through `crate::library::readonly_library_sources(...)` (`validate_cmd.rs:677-688`), **not** `resolve_selector`. The mutating resolver runs `ensure_library_mode()`, which can migrate and rewrite metadata — using it here would reintroduce recovery side effects into a read-only command.
- The index cross-check uses `LibraryManager::inspect_library_index()`, a read-only inspection.
- `UsageIndex::load()` is a read.
- The report declares `dry_run: true` unconditionally (`validate_cmd.rs:76`).
- The command **never** calls `gate_mutation_on_interrupted_transactions()` and never takes the local-data or transaction lock — it does not mutate, so it needs neither.

The one nuance: `validate_library` calls `crate::library::load_library(lib_path)` in some paths, and `load_library` assigns deterministic IDs to snippets that lack them. `validate` therefore reads through the raw file (`std::fs::read_to_string`) for its own structural checks (`validate_cmd.rs:144`) precisely so it can observe pre-normalization state without writing anything back. The `W-ID-EMPTY` message states this explicitly ("load_library assigns IDs on read").

## Output and exit codes

| Mode | Stream | Content |
|---|---|---|
| `--json` | stdout | the serialized `ValidationReport` (`validate_cmd.rs:722-726`) |
| default | stderr | header (version, strict mode, library/snippet counts), then Errors / Warnings / Info sections, each line as `[CODE] library [snippet:id]: message (auto-repairable \| manual fix required \| unrepairable)` (`validate_cmd.rs:576-645`) |

Exit outcome (`validate_cmd.rs:731-736`):

| Condition | Outcome | Exit code |
|---|---|---|
| No `Severity::Error` diagnostics | `CliOutcome::Success` | `0` |
| At least one `Severity::Error` diagnostic | `CliOutcome::ValidationFailed` | `6` (`VALIDATION_FAILED`, `src/outcome.rs:60`) |

Because `--strict` elevates warnings to errors, `--strict` can turn exit `0` into exit `6`.

## Interaction with the transaction journal and locks

| Concern | Behavior |
|---|---|
| Mutation gate | **Not called.** `validate` does not mutate. |
| Local-data lock | **Not taken.** |
| Transaction lock | **Not taken.** |
| Journal | Read-only: none. |
| Side effects | None. `validate` creates no files, including no backups. |

## Key Files

| Path | Role |
|---|---|
| `src/commands/validate_cmd.rs` | This command: check implementations, report model, exit mapping. |
| `src/library/manager.rs` | `LibraryManager::inspect_library_index` — the shared index view. |
| `src/library/mod.rs` | `readonly_library_sources`, `find_orphaned_ids`. |
| `src/usage.rs` | `UsageIndex::load` — the usage index read. |
| `src/commands/repair_cmd.md` | The command that acts on some of these findings. |
| `architecture/library.md` | Library layout, index, primary state. |
| `architecture/persistence.md` | Durability classes and the read-only-vs-mutating resolver rule. |

## Invariants

- `snp validate` never writes, migrates, or creates files.
- Every diagnostic carries a stable `code`, a severity, a repairability class, and — where applicable — a path, library, and snippet ID.
- The index and usage classifications are shared with `repair` and `doctor`, so the commands never disagree about what "orphaned" or "no primary" means.
- The exit code is `6` if and only if the report contains at least one `Error` diagnostic.
- The report schema is versioned (`schema_version: "1.0.0"`).

## Gotchas

- `snp validate` and `snp data validate` are the same command (`src/main.rs:258`, `:825-827`).
- `--library` takes a library **name**, resolved by `readonly_library_sources`. An unknown name surfaces as "Library not found" (`validate_cmd.rs:679-685`).
- `W-CORRUPT-BAK` is reported for a **leftover** `.toml.corrupt.bak` file. It is a historical signal, not a live parse failure — the live failure would be `E-TOML-PARSE`.
- `W-LIB-UNREADABLE` and `W-LIB-INDEX-UNREADABLE` mean the orphan-usage check was **partially or wholly skipped**. The report does not record "check skipped" as a separate flag; the diagnostic *is* the signal.
- `W-SAME-ID-DIVERGENT` and `W-DUP-CHECK-SKIPPED` are mutually exclusive: if the raw re-parse fails you get the latter instead of the former.
- `Severity::Error` is what drives exit `6`; `Repairability::Auto` does **not** mean `snp repair --apply` will fix it — see `repair_cmd.md` for the cases where a safe-marked item still fails to apply.
- The permission check only looks at **four specific files** in the config directory. Library files under `libraries/` are not permission-checked.

# snp import

[← Back to Overview](../overview.md) · [Command index](mod.md) · [pet_analysis.md](pet_analysis.md) · [doctor_cmd.md](doctor_cmd.md)

`snp import` converts a **Pet-format** snippet export into a native snip-it library. It is the only command that creates a new library from an external source.

The currently supported external format is the **Pet TOML** export — a file containing a `[[snippets]]` array-of-tables with the canonical snippet fields plus Pet-specific aliases. There is no JSON, CSV, or YAML importer.

`snp import` **reuses** `src/commands/pet_analysis.rs` for all source reading and field detection; this document covers the command layer, and `pet_analysis.md` covers the analysis layer.

## CLI arguments

The import command is a subcommand group with exactly one member, `pet` (`src/main.rs:337-367`):

```rust
#[derive(Debug, Subcommand)]
enum ImportSubcommands {
    /// Import a Pet snippet file into a native library
    #[command(alias = "p")]
    Pet {
        #[arg(value_name = "PATH")]
        path: PathBuf,
        #[arg(short, long)]
        library: Option<String>,
        #[arg(long, conflicts_with = "replace")]
        merge: bool,
        #[arg(long, conflicts_with = "merge")]
        replace: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        strict: bool,
        #[arg(long, value_enum, default_value = "human")]
        report: commands::import_cmd::ReportFormat,
        #[arg(long)]
        report_file: Option<PathBuf>,
    },
}
```

| Flag | Type | Default | Effect |
|---|---|---|---|
| `PATH` (positional) | `PathBuf` | — | The Pet TOML file to import. |
| `-l`, `--library <NAME>` | `Option<String>` | derived from the file stem | Destination library name. When omitted, `derive_library_name` sanitizes the file stem (lowercase, non-alphanumerics → `-`, collapsed runs, trimmed) (`import_cmd.rs:48-89`). |
| `--merge` | flag | off | Import into an existing library, skipping exact duplicates. Mutually exclusive with `--replace`. |
| `--replace` | flag | off | Replace the destination library entirely (with a backup). Mutually exclusive with `--merge`. |
| `--dry-run` | flag | off | Analyse and report without writing any file. |
| `--strict` | flag | off | Abort on the first error-severity diagnostic instead of skipping the entry. |
| `--report <FORMAT>` | `ReportFormat` | `human` | `human` (stderr) or `json` (stdout). |
| `--report-file <PATH>` | `Option<PathBuf>` | `None` | Additionally write the JSON report to this file (skipped in dry-run). |

Mode resolution happens in the dispatcher (`src/main.rs:786-792`): `--replace` → `Replace`, else `--merge` → `Merge`, else `Create`.

```rust
pub enum ImportMode {
    Create,   // default
    Merge,
    Replace,
}
```

(`import_cmd.rs:13-23`.)

## Control flow

`run_import_pet(options)` (`import_cmd.rs:167-553`) runs in eight phases.

### Phase 1 — read and validate the source

- `read_source_file(&options.source)` (`pet_analysis.rs:40-99`): rejects a missing file, a directory, a non-regular file, a file larger than 16 MiB, non-UTF-8 content, and content containing NUL bytes.
- The content must be non-empty (`:171-176`).
- `parse_pet_toml(&content)` (`pet_analysis.rs:102-105`) parses it, first running `fix_invalid_toml_escapes` to repair invalid escape sequences.
- The file must contain at least one `[[snippets]]` entry (`:180-185`).
- `detect_unknown_fields(&content)` (`pet_analysis.rs:109-210`) scans the **raw** TOML for unknown fields, type mismatches, and missing required keys.

### Phase 2 — library manager

`LibraryManager::new()` followed by `mgr.ensure_library_mode()` (`:191-192`). Unlike `validate`, import **does** migrate to library mode — it is a mutating command.

### Phase 3 — destination

`--library` wins; otherwise the name is derived from the source file stem (`:195-200`). The destination path is `<libraries_dir>/<dest_name>.toml`.

### Phase 4 — convert in memory

A `PetImportReport` is created (`:203-209`), then the **detected capabilities** are recorded from the source content (`:211-247`):

| Capability string | Condition |
|---|---|
| `toml_format` | always |
| `snippet_count=<N>` | always |
| `variables` | any command contains a variable |
| `choice_variables` | any command contains a choice variable |
| `output_fields` | any snippet has a non-empty `output` |
| `tags` | any snippet has non-empty `tags` |

Each entry is then converted by `convert_entry` (`:96-164`), which:

1. Records `NormalizationRecord`s for zero timestamps, for a non-empty `device_id` or `deleted` flag, and for a pre-existing `id` (`:107-142`).
2. **Regenerates the snippet ID** with `uuid::Uuid::new_v4()` (`:144-145`).
3. Fills zero `created_at` / `updated_at` with the current UTC timestamp (`:147-154`).
4. **Clears the sync-only fields** `device_id` and `deleted` (`:156-158`).
5. Delegates diagnostics to `analyze_entry` (`pet_analysis.rs:213-297`).

An entry whose command is empty is **fatal**: it is skipped, `had_fatal_error` is set, and in `--strict` mode the whole import aborts with "Import aborted" (`:257-270`).

### Phase 5 — destination handling by mode

#### `Create` (default)

- If the destination library already exists → hard error "Destination already exists. Use --merge or --replace, or specify a different --library name." (`:279-287`).
- Otherwise write the library with `crate::library::save_library` (`:289-296`).

#### `Merge`

- Load the existing library (or start empty) (`:301-305`).
- Build O(n+m) lookup tables over the existing entries: `first_exact` keyed by `(command, description)`, `by_command`, and `by_description` (`:315-336`).
- For each converted candidate:
  - an exact `(command, description)` match → an `ImportDuplicate` is recorded and the candidate is skipped (`:340-349`);
  - otherwise, entries sharing a command or a description produce `W-DUP-CMD` / `W-DUP-DESC` warnings (`:363-391`).
- Non-duplicate candidates are appended; duplicates increment `report.skipped` (`:396-407`).
- If the destination file does not exist, it is created; if it exists, `crate::library::backup_library` runs first and then `save_library` (`:409-426`).

#### `Replace`

- If the destination does not exist → hard error "Destination does not exist" (`:429-436`).
- Otherwise `backup_library` then `save_library` with the converted entries only (`:438-447`).

### Phase 6 — registration

If the library file was written but is not registered in the index, `mgr.add_existing_library(&dest_name)` is attempted (`:454-468`). A failure prints a warning and is **returned as an error** — the file exists but is unregistered, and the operator is told to run `snp validate` then re-run with `--merge`.

### Phase 7 — auto-sync notification

`notify_mutation(MutationKind::Import, MutationOrigin::Import)` fires **once**, only when the import was not a dry run and at least one entry was imported (`:470-477`).

### Phase 8 — report emission and report file

The report is rendered as human text to **stderr** or JSON to **stdout** (`:479-540`). When `--report-file` is given and this is not a dry run, the JSON report is additionally written with `write_private_atomic` (`:542-550`).

## Output format

The human report (`import_cmd.rs:481-533`):

```
Pet Import Report
=================
Source:      <path>
Destination: <library>
Total:       <n> entries
Imported:    <n>
Skipped:     <n>
Duplicates:  <n>
  [<source_index>] <description> — <reason>
Diagnostics: <n>
  [i|w|e] [<entry_index>] <field>: <message>
Normalized:  <n>
  [<entry_index>] <field>: '<original>' -> '<normalized>'

(dry run — no files were modified)   |   Import complete.
```

The JSON report is `PetImportReport` (`src/diagnostics.rs:58-77`):

```rust
pub struct PetImportReport {
    pub schema_version: String,
    pub tool_version: String,
    pub source: String,
    pub destination: Option<String>,
    pub analysis_mode: String,
    pub mutation_flag: bool,
    pub total_entries: usize,
    pub imported: usize,
    pub skipped: usize,
    pub duplicates: Vec<ImportDuplicate>,
    pub diagnostics: Vec<CompatibilityDiagnostic>,
    pub normalizations: Vec<NormalizationRecord>,
    pub detected_capabilities: Vec<String>,
    pub dry_run: bool,
    pub strict_mode: bool,
    pub had_fatal_error: bool,
}
```

## Exit codes

`snp import` returns `SnipResult<()>` and the dispatcher returns `CliOutcome::Success` (`src/main.rs:775-804`). Failures map through the canonical table in `src/outcome.rs:46-70`:

| Outcome | Exit code |
|---|---|
| Success | `0` |
| General runtime / I/O failure | `1` (`GENERAL_ERROR`) |
| clap argument error (e.g. `--merge --replace` together) | `2` (`USAGE_ERROR`) |

Note that a **partial** import (some entries skipped because of empty commands, without `--strict`) still exits `0`; the skip count is reported but does not change the exit code.

## Interaction with the transaction journal and locks

`snp import` does **not** take the transaction lock or create its own journal. The write path goes through `crate::library::save_library`, which performs the gate and lock acquisition itself:

```rust
let sync_state_dir = crate::config::derive_sync_state_dir();
let transaction_dir = crate::local_data::transaction_dir();
crate::transaction::gate_mutation_on_interrupted_transactions(&sync_state_dir, &transaction_dir)?;
let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;
```

(`src/library/persistence.rs:217-238`.) Two consequences worth auditing:

- The gate and the local-data lock are derived from the **config directory**, not from the target library path (`:223-224`). This is what makes the gate effective for every library, including ones outside the default config root.
- Import therefore takes the **local-data lock only** — no transaction lock, no journal. The lock hierarchy position is *below* `snp restore`, which takes both.

`LibraryManager` gates its own mutations separately (`LibraryManager::gate_mutation`, `src/library/manager.rs:636-642`), so two more import paths are covered:

- `mgr.ensure_library_mode()` (`import_cmd.rs:192`) reaches the gate only when a single-file migration is actually needed, via `migrate_from_single_file` (`src/library/manager.rs:163-168`, `:181-183`, which takes the gate and the local-data lock).
- `mgr.add_existing_library(&dest_name)` (`import_cmd.rs:454-468`) always gates (`src/library/manager.rs:585-586`).

`backup_library` (used in merge and replace) is a plain copy, not a journaled operation.

## Key Files

| Path | Role |
|---|---|
| `src/commands/import_cmd.rs` | This command: modes, conversion, destination handling, report emission. |
| `src/commands/pet_analysis.rs` | Source reading, field detection, per-entry analysis, duplicate predicates. |
| `src/diagnostics.rs` | `PetImportReport`, `CompatibilityDiagnostic`, `ImportDuplicate`, `NormalizationRecord`. |
| `src/library/persistence.rs` | `save_library` — the gate + local-data lock + write. |
| `src/library/model.rs` | `Snippet`, `Snippets` — the target shape. |
| `architecture/commands/pet_analysis.md` | The analysis layer this command reuses. |
| `architecture/persistence.md` | Gate semantics, lock hierarchy, durability classes. |

## Invariants

- The source file is **never modified**.
- The source is read through `read_source_file`, which rejects symlinked-path surprises indirectly by requiring a regular file, enforces a 16 MiB cap, and rejects non-UTF-8 and NUL bytes.
- Imported snippets always get a **fresh UUID** and cleared sync-only fields (`device_id`, `deleted`), so imported content cannot resurrect remote deletions or impersonate another device.
- Zero timestamps are filled with the current time; the change is recorded as a `NormalizationRecord`.
- Merge never overwrites an existing snippet: exact duplicates are skipped, and near-duplicates only produce warnings.
- Replace and merge take a `backup_library` copy of the destination before writing.
- The auto-sync notification fires at most once, and only after a real write.
- A dry run writes **nothing** — not the library, not the report file.

## Gotchas

- `snp import` is `snp import pet <PATH>`; the subcommand alias is `p` (`src/main.rs:340`).
- The default mode is `Create`, which **fails** if the destination library already exists. This is the most common first-run failure.
- Import calls `mgr.ensure_library_mode()` (`:192`) — unlike `validate`, it **can migrate** legacy state. It is a mutating command even before it writes a library.
- The `id` field of an imported snippet is **discarded and regenerated** (`:144-145`). A Pet export that carried snip-it IDs does not preserve them.
- Imported snippet IDs are UUID v4 here, which differs from the deterministic `legacy-<sha256 hex>` IDs used elsewhere (see `docs/IDENTITY_CONTRACT.md`).
- The `device_id` and `deleted` fields are **cleared** (`:156-158`). A `deleted = true` entry in the source is imported as a live snippet.
- `--strict` aborts the whole import on the first empty-command entry (`:263-268`); without it, the entry is skipped and the rest is imported.
- `--merge` and `--replace` are mutually exclusive at the clap level (`src/main.rs:349`, `:352`), so clap rejects both together with exit `2`.
- `--report-file` is ignored in dry-run mode (`:543-545`).
- `had_fatal_error` in the JSON report records that at least one entry was skipped; it does **not** change the exit code.

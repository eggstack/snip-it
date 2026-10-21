# Diagnostics (shared report types)

[← Back to Overview](overview.md)

## Overview

`src/diagnostics.rs` (524 lines, **Layer: Domain/Core**) is pure data: no
I/O, no platform calls, serde only. It defines the `CompatibilityDiagnostic`
family plus the `PetImportReport` / `DoctorReport` envelopes shared by the
doctor, import, and pet-analysis paths.

There is deliberately **no generic finding DSL**. Each consumer — doctor,
validate, status, import — keeps its *own* diagnostic types and its *own*
severity/rendering semantics over the one shared fact source,
`LibraryManager::inspect_library_index`. A shared enum that every consumer had
to bend to was rejected; the shared module holds only the pieces that really
are common (a diagnostic record, a duplicate record, a normalization record,
and the two report envelopes).

This doc covers the shared types, the shared index inspection, and the
`snp doctor` consumer in depth. It does **not** cover the import pipeline
itself (see [commands/mod.md](commands/mod.md)), the selector
([selector.md](selector.md)), the MCP surface ([mcp.md](mcp.md)), or the
sync/auto-sync state machine ([sync.md](sync.md),
[auto_sync.md](auto_sync.md)).

## Key Types (`src/diagnostics.rs`)

| Type | Location | Shape |
|------|----------|-------|
| `DiagnosticSeverity` | `:10` | `Info` (informational) / `Warning` (recoverable; imported with changes) / `Error` (cannot proceed safely) — serde round-trips |
| `SourceSpan` | `:22` | Byte-offset `{ start, end }` into the source file |
| `CompatibilityDiagnostic` | `:29` | `code`, `severity`, `message`, `entry_index?`, `field?`, `suggestion?`, `span?` (`span` is `skip_serializing_if = "Option::is_none"`, `:36`) |
| `ImportDuplicate` | `:42` | `source_index`, `destination_index`, `description`, `reason` |
| `NormalizationRecord` | `:51` | `entry_index`, `field`, `original`, `normalized` |
| `PetImportReport` | `:60` | `schema_version "1.0.0"`, `tool_version`, `source`, `destination?`, `analysis_mode` (`diagnostic` vs `mutating`), `mutation_flag = !dry_run`, `total_entries`, `imported`, `skipped`, `duplicates`, `diagnostics`, `normalizations`, `detected_capabilities`, `dry_run`, `strict_mode`, `had_fatal_error`; `new()` at `:80` |
| `DoctorReport` | `:108` | `schema_version`, `tool_version`, `source?`, `analysis_mode`, `total_entries`, `diagnostics`, `duplicates`, `normalizations`, `has_toml_error`, `toml_error_detail?`, `recommended_import_command?`, `detected_capabilities`, `strict_mode`, `dry_run` (always `true`, `:142`); `new()` at `:127` |
| `version()` | `:148` | `CARGO_PKG_VERSION` at compile time |
| `diagnostic_counts()` | `:155` | Returns `(info_count, warning_count, error_count)` — used by every renderer |

`DoctorReport` is the `PetImportReport` envelope minus the import counters
(`imported`, `skipped`, `mutation_flag`, `had_fatal_error`), plus the
TOML-error fields and `recommended_import_command`, with `source` optional
(doctor can run without a source file).

Code conventions (test-pinned at `src/diagnostics.rs:342`): entry codes use
`E-`/`W-`/`I-` prefixes; sync-mapped codes use the dotted `sync.<area>.<detail>`
form; doctor-run markers use `compat.<area>.<detail>`. Report JSON never
contains `api_key`, `token`, or `secret` (test at `src/diagnostics.rs:496`).

## Shared Index Inspection (no generic DSL)

```rust
// src/library/model.rs:141
pub struct LibraryIndexInspection {
    pub missing_files: Vec<(String, PathBuf)>,
    pub orphan_files: Vec<PathBuf>,
    pub primary: PrimaryState,
}

// src/library/model.rs:123
pub enum PrimaryState {
    Present { name: String },
    FileMissing { name: String, path: PathBuf },
    NoPrimary { count: usize },
    NoLibraries,
}
```

`LibraryManager::inspect_library_index()` (`src/library/manager.rs:297`) is the
single fact source. It is **side-effect-free** — it inspects the index and the
`libraries/` directory without creating, migrating, or rewriting anything.
Each consumer maps the *same* view into *its own* semantics:

| Consumer | Call site | Own diagnostic type | Codes |
|----------|-----------|---------------------|-------|
| validate | `src/commands/validate_cmd.rs:374` (`validate_index`) | `ValidationDiagnostic` / `Severity` / `Repairability` (`validate_cmd.rs:48`, `:32`, `:40`) | `E-INDEX-MISSING-FILE` (`:384`), `W-ORPHAN-FILE` (`:405`), `E-PRIMARY-MISSING` (`:423`), `W-NO-PRIMARY` (`:445`) |
| doctor | `src/commands/doctor_cmd.rs:563` | `CompatibilityDiagnostic` (shared) | `compat.primary_library.ok` (`:576`), `compat.primary_library.missing` (`:588`), `compat.library_manager.error` (`:602`) |
| repair | `src/commands/repair_cmd.rs:381` | repair-plan specific | — |
| status | `src/status_snapshot.rs:549` | `StatusDiagnostic` / `status_snapshot::DiagnosticSeverity` (`status_snapshot.rs:123`, `:131`) | — |

Why it stays split:

- **validate** has a pass/fail-plus-warnings contract and a `Repairability`
  classification (`Auto` / `Manual` / `Unrepairable`) that the shared types do
  not model.
- **status** renders a snapshot, not a report: `StatusSnapshot`
  (`status_snapshot.rs:18`) with `StatusDiagnostic { severity, code, message,
  remediation }` and its own severity enum.
- **doctor** renders into `CompatibilityDiagnostic` and owns the `compat.*`
  code namespace inline (the comment at `doctor_cmd.rs:561` says so
  explicitly).

A shared "finding" enum would have forced one of these vocabularies onto the
others. The index view is shared; the findings are not.

## `snp doctor`

**Files**: `src/commands/doctor_cmd.rs` (1406 lines) — orchestration;
`src/commands/doctor_report.rs` (123 lines) — rendering only.

### CLI Surface

```rust
// src/commands/doctor_cmd.rs:18
pub struct DoctorArgs {
    pub pet_file: Option<PathBuf>,   // --pet-file
    pub compatibility: bool,         // --compatibility
    pub sync: bool,                  // --sync
    pub check_shell: Option<ShellIntegration>, // --check-shell
    pub library: Option<String>,     // --library
    pub strict: bool,                // --strict
    pub report: DiagnosticReportFormat, // --report human|json
}
```

`run()` (`doctor_cmd.rs:1228`) requires **one** mode — no mode is an error
("No mode selected", `doctor_cmd.rs:1240-1247`) — and picks exactly one
builder:

| Mode | Builder | Report `source` |
|------|---------|-----------------|
| `--pet-file <path>` | `build_pet_report` (`:201`) | the file path |
| `--library <name-or-path>` | `build_pet_report` (`:201`), after `resolve_library_path` (`:1049`) | the resolved path |
| `--compatibility` | `build_compatibility_report` (`:470`) | `None` |
| `--sync` | `DoctorReport::new(strict)` + `append_sync_diagnostics(report, false)` (`:1276-1282`) | `None` |

`--check-shell <shell>` is **additive**: it runs after the selected mode and
appends to the same report (`doctor_cmd.rs:1287-1290`).

### The Environment Audit (`--compatibility`)

`build_compatibility_report()` (`doctor_cmd.rs:470-1044`) audits the installed
environment. Every check appends a `CompatibilityDiagnostic` with a
`compat.*` code:

| Area | Codes | What it checks |
|------|-------|----------------|
| Binary | `compat.binary_version` (`:481`) | `snp version` + the resolved executable path |
| Config dir | `compat.config_dir.ok` / `.readonly` / `.missing` (`:498`, `:511`, `:522`) | the config directory exists and is writable |
| Libraries dir | `compat.libraries_dir.ok` / `.missing` (`:536`, `:546`) | `<config>/libraries` exists (absence means single-file mode) |
| Primary library | `compat.primary_library.ok` / `.missing` (`:576`, `:588`) | from `inspect_library_index().primary`; the ok case reports the snippet count |
| Library manager | `compat.library_manager.error` (`:602`) | the manager could not be constructed |
| Sync config | `compat.sync.enabled` / `.disabled` / `.config_error` / `.absent` (`:620`, `:630`, `:642`, `:654`) | `<config>/sync.toml` presence and loadability |
| Shells on PATH | `compat.shell.<bash\|zsh\|fish>.ok` / `.missing` (`:676`, `:686`) | `which <shell>` |
| `snp select` | `compat.select.ok` / `.missing` (`:708`, `:718`) | `snp select --help` exits 0 |
| Acquisition flags | `compat.acquire.command_stdin` / `.from_file` / `.editor` (`:741`, `:763`, `:785`) | `snp new --help` mentions `--command-stdin`, `--from-file`, `--editor` |
| Choice parser | `compat.choice_parser.ok` / `.missing` (`:818`, `:828`) | `parse_variables("echo <color=\|_red_\|\|_green_\|\|_blue_\|\|>")` yields a `Choices` variable |
| Shell init syntax | `compat.shell_syntax.<shell>.ok` / `.invalid` / `.init_failed` / `.unavailable` (`:890`, `:901`, `:916`, `:850`) | pipes `snp shell init <shell>` into `bash -n` / `zsh -n` / `fish --no-execute` |
| Editor | `compat.editor.ok` / `.unset` (`:942`, `:952`) | `$VISUAL` / `$EDITOR` (unset means `snp new --editor` falls back to vim) |
| Pet TOML | `compat.pet_toml.ok` / `.error` (`:974`, `:985`) | a canonical Pet TOML snippet parses |
| Legacy paths | `compat.legacy.single_file` / `.config_dir` (`:1002`, `:1021`) | `~/.config/snippets.toml` and `~/.snip-it` still exist |
| Sync snapshot | `compat.sync.checked` + `sync.*` (`:170`, via `append_sync_diagnostics(report, true)`) | the canonical `StatusSnapshot` |

Notes:

- The shell-syntax audit **shells out** to `snp shell init <shell>` and pipes
  the result into the shell's own syntax checker (`doctor_cmd.rs:861-885`);
  the shell binary must be on `PATH` or the check is skipped with
  `.unavailable`.
- The audit ends with `report.total_entries = 0` (`doctor_cmd.rs:1041`) —
  there are no snippet entries in an environment audit.
- `--compatibility` mode calls `append_sync_diagnostics(&mut report, true)`,
  which **downgrades** `CONFIG_LOAD_FAILED` from Error to Warning
  (`doctor_cmd.rs:142-147`) for backward compatibility. `--sync` mode calls it
  with `false`, preserving the snapshot's native severities (including Error).

### The Pet-File Report (`--pet-file` / `--library`)

`build_pet_report()` (`doctor_cmd.rs:201-370`) is a read-only analysis of one
Pet TOML file:

1. `detect_unknown_fields(content)` (`pet_analysis.rs:109`) — raw structural
   scan for unknown fields (`I-FIELD-UNKNOWN`).
2. `parse_pet_toml(content)` (`pet_analysis.rs:102`) — on failure the report
   returns early with `has_toml_error = true` and `toml_error_detail` set
   (`doctor_cmd.rs:215-222`).
3. `analyze_entry(i, pet)` per entry (`pet_analysis.rs:213`).
4. `detect_unsupported_concepts()` (`doctor_cmd.rs:373`) — malformed variable
   placeholders (`W-MALFORMED-VAR`, unmatched `<` or `>`; escaped brackets are
   skipped) and preserved `folders` (`I-FIELD-FOLDERS`).
5. Destination conflict (`W-DEST-CONFLICT`, `doctor_cmd.rs:243`) — the
   sanitized file stem (`sanitize_library_name`, `doctor_cmd.rs:437`) already
   exists as a library.
6. `detect_duplicates()` (`pet_analysis.rs:318`) — same command / different
   description (`W-DUP-CMD`) and same description / different command
   (`W-DUP-DESC`), plus the `ImportDuplicate` records.
7. **Normalization preview** — one `NormalizationRecord` per field that the
   import would change: timestamps (`created_at`/`updated_at` zero → "will be
   set to current time"), sync fields (`device_id` cleared, `deleted=false`),
   and id regeneration (`doctor_cmd.rs:261-301`).
8. **Detected capabilities** — `toml_format`, `snippet_count=N`, and
   conditionally `variables`, `choice_variables`, `output_fields`, `tags`
   (`doctor_cmd.rs:303-343`).
9. `recommended_import_command` — `snp import pet <path>`, prefixed with a
   "Resolve errors first" comment when the report has errors
   (`doctor_cmd.rs:345-357`). Errors without a suggestion get
   "Fix this entry before importing" (`doctor_cmd.rs:359-364`).

### The Shell-Init Syntax Check (`--check-shell`)

`check_shell_init()` (`doctor_cmd.rs:1084-1225`) is separate from the
environment audit's shell check:

- The shell name is mapped to `ShellType::{Bash, Zsh, Fish}`
  (`doctor_cmd.rs:1085-1101`); anything else → `compat.shell_init.<name>.unknown`
  (Warning) and no check runs.
- The init code is generated **in-process** via
  `shell_cmd::generate_bash()/generate_zsh()/generate_fish()`
  (`doctor_cmd.rs:1104-1108`) — no subprocess is used to produce it.
- If the shell is not on `PATH` → `compat.shell_init.<name>.unavailable`
  (Error under `--strict`, else Warning) and the check stops.
- The code is written to a `tempfile::NamedTempFile` and checked with
  `bash -n <file>` / `zsh -n <file>` / `fish --no-execute <file>`
  (`doctor_cmd.rs:1169-1182`).
- Success → `compat.shell_init.<name>.ok` (Info, with the byte count of the
  generated script); a syntax failure → `.invalid` (Error under `--strict`,
  else Warning, with the shell's stderr); a temp-file or spawn failure →
  `.error` (Warning).

### Strict Elevation

`--strict` is applied by `apply_strict_elevation()` (`doctor_cmd.rs:187-198`):
every `Warning` whose code is in `STRICT_WARNING_CODES`
(`doctor_cmd.rs:53-63`) is promoted to `Error`. The list is:

`W-MALFORMED-VAR`, `W-DUP-CMD`, `W-DUP-DESC`, `W-DEST-CONFLICT`,
`W-DESC-MISSING`, `W-CMD-MISSING`, `W-DESC-EMPTY`, `W-TYPE-MISMATCH`,
`W-TAG-EMPTY`.

The elevation lives in `doctor_cmd`, never in the shared types — the shared
`DiagnosticSeverity` has no notion of "strict".

### Sync Diagnostics Mapping

`append_sync_diagnostics(report, compat_mode)` (`doctor_cmd.rs:165-184`) always
emits `compat.sync.checked` (Info, carrying `snapshot.sync.top_level`), then
maps every `StatusDiagnostic` through `map_snapshot_diagnostic()`
(`doctor_cmd.rs:70-159`):

| Snapshot code | Mapped code | Field |
|---------------|-------------|-------|
| `CONFIG_LOAD_FAILED` | `sync.config.load_failed` | `sync_config` |
| `NOT_CONFIGURED` | `sync.config.not_configured` | `sync_config` |
| `PENDING_CORRUPT` | `sync.pending.corrupt` | `pending` |
| `PENDING_INACCESSIBLE` | `sync.pending.inaccessible` | `pending` |
| `EXECUTION_LOCK_STALE` | `sync.execution.dead_stale` | `execution_lock` |
| `EXECUTION_LOCK_MALFORMED` / `_INACCESSIBLE` | `sync.execution.malformed` | `execution_lock` |
| `WORKER_LOCK_STALE` | `sync.worker_lock.dead_stale` | `worker_lock` |
| `WORKER_LOCK_MALFORMED` / `_INACCESSIBLE` | `sync.worker_lock.malformed` | `worker_lock` |
| `STATUS_CORRUPT` | `sync.status.corrupt` | `auto_sync_status` |
| `ATTENTION_REQUIRED` | `sync.attention.{authentication\|configuration\|credential_store\|conflict\|partial\|local_persistence}` | `auto_sync_status` |
| anything else | `sync.<lowercased code>` | `sync` |

`ATTENTION_REQUIRED` is classified by inspecting the message text
(`doctor_cmd.rs:113-132`). The snapshot's `remediation` becomes the
diagnostic's `suggestion` (`doctor_cmd.rs:156`).

### Rendering: Human vs JSON

`DiagnosticReportFormat` (`doctor_report.rs:10`) is `Human` (default) or
`Json`.

- **Human** — `emit_human_report()` (`doctor_report.rs:17-123`) writes to
  **stderr** via `eprintln!`. Layout: the `Doctor Report` header, `Source:`,
  `Version:`, `Entries:`, then — when non-empty — a `TOML Error:` block, then
  `Errors (n):` / `Warnings (n):` / `Info (n):` groups (each line
  `[e|w|i] [entry_index] field: message`, with `suggestion:` on the following
  line for errors and warnings only), then `Duplicates (n):`, `Normalizations
  (n):`, `Supported features:`, and finally `Suggested next command:`.
- **JSON** — `serde_json::to_string_pretty(&report)` printed to **stdout**
  (`doctor_cmd.rs:1297-1302`). The JSON is the `DoctorReport` struct
  verbatim, so the envelope is the schema.

> **Human goes to stderr, JSON goes to stdout.** A script that pipes
> `snp doctor --report json` gets only JSON on stdout; the human renderer
> never pollutes it.

### Exit Code

`run()` ends by scanning the collected diagnostics for any `Error` severity
(`doctor_cmd.rs:1306-1315`):

- any `Error` → `CliOutcome::ValidationFailed` (exit code 6, see
  [outcome.md](outcome.md))
- otherwise → `CliOutcome::Success` (exit code 0)

The comment at `doctor_cmd.rs:1312-1313` records that this deliberately
routes through the canonical outcome table instead of a raw usage error.

## Import Consumer

**Files**: `src/commands/import_cmd.rs`, `src/commands/pet_analysis.rs`.

- `PetImportReport::new(source, destination, dry_run, strict_mode)`
  (`diagnostics.rs:80`) — a dry run keeps `analysis_mode = "diagnostic"` and
  `mutation_flag = false`; a real run flips both to `"mutating"` / `true`
  (`diagnostics.rs:86-91`).
- `import_cmd.rs:203` constructs the report; `:343` pushes `ImportDuplicate`
  records; `:366` and `:380` push `CompatibilityDiagnostic`s.
- Field analysis comes from `pet_analysis.rs`: `read_source_file` (`:40`),
  `parse_pet_toml` (`:102`), `detect_unknown_fields` (`:109`),
  `analyze_entry` (`:213`), `detect_duplicates` (`:318`).
- `import_cmd.rs` reuses the same `CompatibilityDiagnostic` codes as doctor
  (`W-DUP-CMD`, `W-DUP-DESC`) because both speak the same shared vocabulary.

## Code-Prefix Table

| Prefix / form | Meaning | Examples |
|---------------|---------|----------|
| `E-` | Entry cannot be imported / hard failure | `E-CMD-EMPTY`, `E-INDEX-MISSING-FILE`, `E-PRIMARY-MISSING` |
| `W-` | Recoverable; entry imported with changes | `W-DESC-EMPTY`, `W-TYPE-MISMATCH`, `W-TAG-EMPTY`, `W-DUP-CMD`, `W-DUP-DESC`, `W-ORPHAN-FILE`, `W-MALFORMED-VAR`, `W-DEST-CONFLICT` |
| `I-` | Informational capability/field note | `I-FIELD-UNKNOWN`, `I-OUTPUT-PRESENT`, `I-TAGS-EMPTY`, `I-CHOICE-VARS`, `I-FIELD-FOLDERS` |
| `sync.*` | Mapped snapshot diagnostics | `sync.config.load_failed`, `sync.pending.corrupt`, `sync.execution.dead_stale`, `sync.attention.authentication` |
| `compat.*` | Doctor-run markers | `compat.sync.checked`, `compat.config_dir.readonly`, `compat.shell_init.bash.ok` |

Sources: `pet_analysis.rs:152-348`, `doctor_cmd.rs:170-1021`,
`validate_cmd.rs:384-445`.

## Report Lifecycle (Doctor)

1. `DoctorReport::new(strict)` (`diagnostics.rs:127`) — `analysis_mode =
   "diagnostic"`, `dry_run = true`, empty vectors, `source = None`.
2. The selected builder appends diagnostics, duplicates, normalizations, and
   capabilities (`doctor_cmd.rs:201` / `:470` / `:1276-1282`).
3. `--check-shell` appends to the same report (`doctor_cmd.rs:1287-1290`).
4. `apply_strict_elevation` (`doctor_cmd.rs:187`) promotes the designated
   warnings when `--strict`.
5. `diagnostic_counts` (`diagnostics.rs:155`) splits the triple for rendering.
6. `emit_human_report` (`doctor_report.rs:17`) or serde `Json` serialization
   emits.
7. Any `Error` severity → `CliOutcome::ValidationFailed`.

Import mirrors it with `PetImportReport::new(source, dest, dry_run, strict)`
(`diagnostics.rs:80`).

## Tests

`src/diagnostics.rs:171` (unit) and `src/commands/doctor_cmd.rs:1320` (unit):

| Test | Location | Protects |
|------|----------|----------|
| `test_diagnostic_counts` | `:188` | the `(info, warning, error)` triple |
| `test_pet_import_report_new` | `:205` | import envelope defaults |
| `test_doctor_report_new` | `:221` | doctor envelope defaults (`dry_run = true`) |
| `test_diagnostic_severity_serialization` | `:240` | severity serde round-trip |
| `test_compatibility_diagnostic_serialization` | `:255` | diagnostic serde round-trip |
| `test_source_span_serialization` | `:281` | span shape |
| `test_diagnostic_span_skip_none` | `:290` | `skip_serializing_if` on `span` |
| `test_diagnostic_ordering_preserved` | `:308` | order is preserved through serde |
| `test_diagnostic_codes_follow_convention` | `:342` | `E-`/`W-`/`I-` prefix convention |
| `test_strict_mode_error_classification` | `:368` | strict-mode severity classification |
| `test_diagnostic_bounded_message` | `:412` | message length bounds |
| `test_doctor_report_recommendation` | `:431` | `recommended_import_command` |
| `test_diagnostic_counts_empty` | `:447` | empty counts |
| `test_pet_import_report_full_roundtrip` | `:455` | full import-report round-trip |
| `test_doctor_report_no_secrets_in_json` | `:496` | no `api_key` / `token` / `secret` in JSON |
| `test_sanitize_library_name` (+ fallback) | `doctor_cmd.rs:1326`, `:1336` | library-name sanitization |
| `test_malformed_variable_*` | `doctor_cmd.rs:1342`, `:1364`, `:1386` | `W-MALFORMED-VAR` detection and escape handling |

## Key Files

- `src/diagnostics.rs` — severities, spans, diagnostics, both report envelopes, counts
- `src/commands/doctor_cmd.rs` — orchestration: mode selection, environment audit, pet analysis, strict elevation, shell checks
- `src/commands/doctor_cmd.rs:70` — `map_snapshot_diagnostic` (snapshot → dotted `sync.*` codes)
- `src/commands/doctor_cmd.rs:165` — `append_sync_diagnostics`
- `src/commands/doctor_cmd.rs:187` — `apply_strict_elevation` + `STRICT_WARNING_CODES` (`:53`)
- `src/commands/doctor_cmd.rs:201` / `:470` — the two report builders
- `src/commands/doctor_cmd.rs:1084` — `check_shell_init`
- `src/commands/doctor_cmd.rs:1228` — `run`, the mode dispatch, and the exit-code decision
- `src/commands/doctor_report.rs:10`, `:17` — `DiagnosticReportFormat` and the human renderer
- `src/commands/validate_cmd.rs:32`, `:48`, `:60`, `:374` — validate's *own* report types over the shared index view
- `src/commands/pet_analysis.rs:109`, `:213`, `:318` — pet field/entry analysis and duplicate detection
- `src/commands/import_cmd.rs:203` — import report construction
- `src/status_snapshot.rs:18`, `:123`, `:131`, `:144` — the snapshot feeding both `snp status` and doctor's sync section
- `src/library/manager.rs:297`, `src/library/model.rs:123`, `:141` — `inspect_library_index` and its view types

## Invariants

- `diagnostics.rs` stays dependency-free (serde only); layer rule: core must
  not import commands/ui/logging.
- Doctor reports are always `dry_run = true`; import reports set
  `mutation_flag = !dry_run` and `analysis_mode` accordingly.
- Severity meaning is per-consumer: strict elevation and compat downgrades
  live in `doctor_cmd`, never in the shared types.
- Diagnostic codes are conventional (`E-`/`W-`/`I-`, dotted `sync.*`,
  `compat.*`) but unenforced across consumers — each renderer owns its code
  list.
- `inspect_library_index()` stays side-effect-free: it never creates,
  migrates, or rewrites library state.
- The human report never writes to stdout; only the JSON report does.

## Gotchas

- **There is no shared "finding" type.** If you are looking for a common
  diagnostic that doctor, validate, and status all emit, it does not exist.
  `CompatibilityDiagnostic` is shared by doctor and import; validate uses
  `ValidationDiagnostic`; status uses `StatusDiagnostic`. Only the index view
  (`LibraryIndexInspection`) is genuinely shared.
- **`DoctorReport.dry_run` is always `true`** and exists only for schema
  uniformity with `PetImportReport` (`diagnostics.rs:122-123`). It is not a
  signal that doctor might mutate something.
- **`DoctorReport.source` is `Option<String>`** — it is `None` for
  `--compatibility` and `--sync` modes.
- **`--compatibility` downgrades `CONFIG_LOAD_FAILED` to Warning**; `--sync`
  does not. The two sync modes deliberately differ in severity.
- **The two shell checks are different code paths.** The environment audit
  shells out to `snp shell init <shell>`; `--check-shell` generates the init
  in-process and checks the generated text. A broken `snp shell init` binary
  therefore fails the audit but not `--check-shell`.
- **`compat.shell_init.*.unavailable` becomes an Error under `--strict`**,
  which changes the exit code to 6 — a missing shell on `PATH` can fail a
  strict doctor run.
- **The human renderer prints to stderr.** Scripts must use
  `--report json` (stdout) to consume the report.
- **Code-prefix enforcement is only a unit test** (`diagnostics.rs:342`), not a
  typed constraint; a new consumer can introduce any code string.

## Cross-References

- [commands/doctor_cmd.md](commands/doctor_cmd.md) — the `snp doctor` command.
- [commands/doctor_report.md](commands/doctor_report.md) — the report renderer.
- [commands/pet_analysis.md](commands/pet_analysis.md) — the pet-file
  analysis rules behind `I-*`/`W-*`/`E-*` codes.
- [commands/validate_cmd.md](commands/validate_cmd.md) — validate's own
  report vocabulary over the same index view.
- [status.md](status.md) — the status snapshot that feeds doctor's sync section.
- [outcome.md](outcome.md) — exit codes, including `ValidationFailed` (6).

# Diagnostics (shared report types)

[← Back to Overview](overview.md)

## Overview

`src/diagnostics.rs` (524 lines, **Layer: Domain/Core**) is pure data: no
I/O, no platform calls. It defines the `CompatibilityDiagnostic` family plus
the `PetImportReport` / `DoctorReport` envelopes shared by doctor, import,
and validation paths. There is deliberately **no generic finding DSL** —
each consumer classifies over the shared `LibraryManager::inspect_library_index`
view but keeps its own severity/rendering semantics.

## Key types (`src/diagnostics.rs`)

| Type | Location | Shape |
|------|----------|-------|
| `DiagnosticSeverity` | `:10` | `Info` (informational) / `Warning` (recovered with changes) / `Error` (cannot proceed) — serde round-trips |
| `SourceSpan` | `:22` | Byte-offset `{ start, end }` into the source file |
| `CompatibilityDiagnostic` | `:29` | `code`, `severity`, `message`, `entry_index?`, `field?`, `suggestion?`, `span?` (`skip_serializing_if None`) |
| `ImportDuplicate` | `:42` | `source_index`, `destination_index`, `description`, `reason` |
| `NormalizationRecord` | `:51` | `entry_index`, `field`, `original`, `normalized` |
| `PetImportReport` | `:60` | `schema_version "1.0.0"`, `tool_version` (`version()`, `:148` = `CARGO_PKG_VERSION`), `source`, `destination?`, `analysis_mode` (`diagnostic` vs `mutating`), `mutation_flag = !dry_run`, totals, `duplicates`, `diagnostics`, `normalizations`, `detected_capabilities`, `dry_run`, `strict_mode`, `had_fatal_error`; `new()` at `:80` |
| `DoctorReport` | `:108` | Same envelope minus import counts, plus `has_toml_error`, `toml_error_detail?`, `recommended_import_command?`; `dry_run` always true (`:142`); `new()` at `:127` |
| `diagnostic_counts` | `:155` | Returns `(info, warning, error)` triple used by every renderer |

Code conventions (pinned by tests, `:342`): entry codes use `E-`/`W-`/`I-`
prefixes; sync-mapped codes use dotted `sync.<area>.<detail>` form (see
`map_snapshot_diagnostic` below). Report JSON never contains `api_key`,
`token`, or `secret` (test, `:496`).

## Shared index inspection (no generic DSL)

`LibraryManager::inspect_library_index` (`src/library/manager.rs:297`,
states in `src/library/model.rs:121`) is the common fact source; rendering
stays per-consumer:

- `validate_index` (`src/commands/validate_cmd.rs:374`) maps the inspection to **local** `ValidationDiagnostic`/`Severity`/`Repairability` (`validate_cmd.rs:32`): `E-INDEX-MISSING-FILE` (`:384`), `W-ORPHAN-FILE` (`:405`), `E-PRIMARY-MISSING` (`:423`), `W-NO-PRIMARY` (`:445`) — preserving validate's pass/fail-plus-warnings contract.
- `doctor_cmd.rs:563` renders the same inspection into `CompatibilityDiagnostic`s inline (comment at `:561`: rendering stays local to doctor).
- `repair_cmd.rs:381` and `status_snapshot.rs:549` classify the identical view for repair planning and status display.

## Consumers

**`snp doctor`** (`src/commands/doctor_cmd.rs:1228`, rendering in
`src/commands/doctor_report.rs`):

- `build_compatibility_report(strict)` (`doctor_cmd.rs:470`) — environment audit: config, libraries, sync snapshot, shell integration.
- `build_pet_report(source, content, strict)` (`:201`) — pet-file analysis path.
- `append_sync_diagnostics(report, compat_mode)` (`:165`) — always emits `compat.sync.checked`, then maps the canonical `StatusSnapshot` (`src/status_snapshot.rs:144`) via `map_snapshot_diagnostic` (`:70`): `CONFIG_LOAD_FAILED` → `sync.config.load_failed` (downgraded to Warning in `--compatibility` mode), lock states → `sync.execution.*` / `sync.worker_lock.*`, attention reasons → `sync.attention.*`.
- `apply_strict_elevation` (`:187`) — `--strict` promotes listed `STRICT_WARNING_CODES` to Error.
- `detect_unsupported_concepts` (`:373`), `check_shell_init` (`:1084`) — snippet/shape and shell-rc checks.
- `DiagnosticReportFormat` (`doctor_report.rs:10`, `Human` default / `Json`); `emit_human_report` (`:17`) groups Errors, Warnings, Info (via `diagnostic_counts`), then duplicates, normalizations, capabilities, and the suggested import command.

**Import** (`src/commands/import_cmd.rs:203` builds `PetImportReport::new`;
`:366`, `:380` push entry diagnostics): field analysis comes from
`src/commands/pet_analysis.rs` — `detect_unknown_fields` (`:109`),
`analyze_entry` (`:213`, `E-CMD-EMPTY` / `W-DESC-*` / `W-TYPE-MISMATCH` /
`W-TAG-*` / `I-*`), duplicate detection (`:320`).

**Validate / repair / status**: validate's local report (`validate_cmd.rs:60`)
with index section (`:374`); repair consumes the inspection for planning
(`repair_cmd.rs:381`); `capture_snapshot` (`status_snapshot.rs:144`) with
`StatusDiagnostic` (`:123`) + own `DiagnosticSeverity` (`:131`) feeds both
`snp status` and doctor's sync section.

**MCP** (adjacent, not a report-type consumer): `src/mcp/tools.rs:13`
shares `selector::searchable_text` (`:99`) for search parity with
`snp get --query` / `snp list --filter`; it keeps its own tool schemas and
emits no `CompatibilityDiagnostic`s.

## Report lifecycle (doctor)

1. `DoctorReport::new(strict)` (`diagnostics.rs:127`) — `dry_run: true`, empty vectors.
2. Builders append: compatibility checks (`doctor_cmd.rs:470`), pet analysis (`:201`), sync snapshot mapping (`:165`), shell checks (`:1084`).
3. `apply_strict_elevation` (`:187`) promotes listed warnings when `--strict`.
4. `diagnostic_counts` (`diagnostics.rs:155`) splits the triple for rendering.
5. `emit_human_report` (`doctor_report.rs:17`) or serde `Json` serialization emits.

Import mirrors it with `PetImportReport::new(source, dest, dry_run, strict)`
(`diagnostics.rs:80`): dry runs stay `analysis_mode = "diagnostic"` with
`mutation_flag = false`; real runs flip both (`:86`, `:91`).

## Code-prefix table

| Prefix / form | Meaning | Examples |
|---------------|---------|----------|
| `E-` | Entry cannot be imported / hard failure | `E-CMD-EMPTY`, `E-INDEX-MISSING-FILE`, `E-PRIMARY-MISSING` |
| `W-` | Recoverable; entry imported with changes | `W-DESC-EMPTY`, `W-TYPE-MISMATCH`, `W-TAG-EMPTY`, `W-DUP-CMD`, `W-ORPHAN-FILE` |
| `I-` | Informational capability/field note | `I-FIELD-UNKNOWN`, `I-OUTPUT-PRESENT`, `I-TAGS-EMPTY`, `I-CHOICE-VARS` |
| `sync.*` | Mapped snapshot diagnostics | `sync.config.load_failed`, `sync.pending.corrupt`, `sync.execution.dead_stale`, `sync.attention.authentication` |
| `compat.*` | Doctor-run markers | `compat.sync.checked` |

## Tests (selected, `diagnostics.rs:171`)

Counting (`:188`), both `new()` constructors (`:206`, `:222`), severity and
full-report serde round-trips (`:241`, `:254`, `:456`), span skip-when-`None`
(`:290`), ordering preservation (`:308`), code-prefix convention (`:342`),
strict error classification (`:368`), bounded messages (`:412`),
recommendation field (`:431`), empty counts (`:447`), no-secrets JSON (`:496`).

## Invariants

- `diagnostics.rs` stays dependency-free (serde only); layer rule: core must not import commands/ui/logging.
- Doctor reports are always `dry_run = true`; import reports set `mutation_flag = !dry_run` and `analysis_mode` accordingly.
- Severity meaning is per-consumer: strict elevation and compat downgrades live in `doctor_cmd`, never in the shared types.
- Diagnostic codes are conventional (`E-`/`W-`/`I-`, dotted `sync.*`) but unenforced across consumers — each renderer owns its code list.

## Key files

- `src/diagnostics.rs` — severities, spans, diagnostics, both reports, counts
- `src/commands/doctor_cmd.rs:70` (snapshot mapping), `:165` (sync section), `:187` (strict), `:201`/`:470` (builders), `:1084` (shell), `:1228` (entry)
- `src/commands/doctor_report.rs:10`, `:17` — format + human rendering
- `src/commands/validate_cmd.rs:32`, `:374` — local report + shared index view
- `src/commands/pet_analysis.rs:109`, `:213` — pet field/entry analysis
- `src/commands/import_cmd.rs:203` — import report construction
- `src/status_snapshot.rs:18`, `:123`, `:144` — snapshot feeding status + doctor
- `src/library/manager.rs:297` — `inspect_library_index` shared fact source

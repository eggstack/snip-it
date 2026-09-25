# doctor_report — Doctor Human/JSON Rendering

[← Back to Overview](../overview.md)

## Overview

`src/commands/doctor_report.rs` (123 lines) is the pure display half
of `snp doctor`: the `DiagnosticReportFormat` CLI enum and the
human-readable emitter. Check orchestration stays in `doctor_cmd.rs`
(`doctor_cmd.md`); JSON serialization is a one-line `serde_json` call
at the `doctor_cmd::run` site. No I/O beyond stderr, no logic beyond
grouping and ordering.

## CLI surface

`DiagnosticReportFormat` (`doctor_report.rs:9`, `ValueEnum`,
`Default=Human`): `--report human` (default) → `emit_human_report`
to stderr; `--report json` → pretty `DoctorReport` JSON to stdout
(`doctor_cmd.rs:1297-1302`). Re-exported via `doctor_cmd.rs:50` so
the `DoctorArgs` schema cannot drift from the renderer.

## Flow / steps

`emit_human_report(&DoctorReport)` (`doctor_report.rs:17`):

1. Header: blank line, `Doctor Report`, `=============`, optional
   `Source:`, `Version:`, `Entries:`.
2. `diagnostic_counts` split → TOML-error block (`TOML Error:` +
   detail) if `has_toml_error`.
3. `Errors (n):` — `[e] [index|−] field: message` + indented
   `suggestion:` per error diagnostic.
4. `Warnings (n):` / `Info (n):` — same shape with `[w]`/`[i]`
   markers (info omits suggestions).
5. `Duplicates (n):` — `[source_index] description — reason` per
   `ImportDuplicate`.
6. `Normalizations (n):` — `[index] field: 'original' -> 'normalized'`.
7. `Supported features:` — one capability per line
   (`toml_format`, `snippet_count=N`, `variables`,
   `choice_variables`, `output_fields`, `tags`).
8. `Suggested next command:` — the `recommended_import_command`
   (commented-out when errors block import).

Empty sections are omitted entirely; ordering is fixed
TOML → errors → warnings → info → duplicates → normalizations →
capabilities → suggestion.

## Mutation vs read-only

Pure display. No gate, no lock, no file access, no runtime. Safe to
call on partially built reports (e.g. TOML-parse failure returns a
report with only source + TOML detail, still rendered).

## Auto-sync trigger

None. Rendering never touches sync state.

## Error / exit mapping

Infallible (`-> ()`): all fields are optional-aware (`as_deref()
.unwrap_or("-")`, `map_or("-")`), so missing indices/fields render
as `-` instead of panicking. Exit-code decisions live in
`doctor_cmd::run` (errors → exit 6), never here.

## Key invariants

- Human goes to **stderr**, JSON to **stdout** — scripts parsing
  `--report json` never see human framing.
- Severity groups render errors first; within a group, report order
  is preserved (no re-sorting that would detach indices).
- Every error diagnostic carries a suggestion (doctor backfills
  `Fix this entry before importing` when the check left none).
- Keep the renderer free of check semantics: new diagnostics appear
  automatically via the shared `CompatibilityDiagnostic` shape.

## File / line references

- Format enum: `src/commands/doctor_report.rs:9`
- `emit_human_report`: `:17` (sections: `:28-122`)
- Caller: `src/commands/doctor_cmd.rs:1292-1317`
- Types: `src/diagnostics.rs` (`DoctorReport`, `diagnostic_counts`)

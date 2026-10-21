# doctor_report — doctor report rendering

[← Back to Overview](../overview.md) · [Command index](mod.md) · [doctor_cmd.md](doctor_cmd.md)

`src/commands/doctor_report.rs` is the **pure display layer** for `snp doctor`. It owns the output-format enum and the human-readable rendering; it performs no checks, touches no files, and knows nothing about the config directory.

Its only inputs are the report types from `src/diagnostics.rs`.

## Output format

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum DiagnosticReportFormat {
    #[default]
    Human,
    Json,
}
```

(`doctor_report.rs:9-14`.) The CLI flag is `--report` with `default_value = "human"` (`src/commands/doctor_cmd.rs:46-47`).

| Format | Stream | Producer |
|---|---|---|
| `human` | **stderr** | `emit_human_report(&report)` (`doctor_report.rs:16-123`) |
| `json` | **stdout** | `serde_json::to_string_pretty(&report)` (`src/commands/doctor_cmd.rs:1297-1302`) |

The human report goes to stderr so that the JSON on stdout stays machine-parseable; the two are never mixed on one stream.

## The report being rendered

```rust
pub struct DoctorReport {
    pub schema_version: String,
    pub tool_version: String,
    pub source: Option<String>,
    pub analysis_mode: String,
    pub total_entries: usize,
    pub diagnostics: Vec<CompatibilityDiagnostic>,
    pub duplicates: Vec<ImportDuplicate>,
    pub normalizations: Vec<NormalizationRecord>,
    pub has_toml_error: bool,
    pub toml_error_detail: Option<String>,
    pub recommended_import_command: Option<String>,
    pub detected_capabilities: Vec<String>,
    pub strict_mode: bool,
    /// Always true for doctor; included for report schema uniformity.
    pub dry_run: bool,
}
```

(`src/diagnostics.rs:106-124`.) `DoctorReport::new(strict_mode)` sets `schema_version` to `"1.0.0"`, `analysis_mode` to `"diagnostic"`, `dry_run` to `true`, and `source` to `None` (`:126-145`). `tool_version` comes from `crate::diagnostics::version()`, which is `env!("CARGO_PKG_VERSION")` (`:147-150`).

## Human rendering

`emit_human_report` (`doctor_report.rs:16-123`) writes, in order:

1. **Header** — `Doctor Report`, then `Source:` (only when `report.source` is `Some`), `Version:`, and `Entries:` (`:17-25`).
2. **TOML Error** section — printed only when `report.has_toml_error` is true, with the detail from `report.toml_error_detail` (`:29-35`).
3. **Errors (N)** — one line per `DiagnosticSeverity::Error`, followed by an indented `suggestion:` line when the diagnostic has one (`:37-53`).
4. **Warnings (N)** — same shape, with suggestions (`:55-71`).
5. **Info (N)** — same shape, without suggestions (`:73-86`).
6. **Duplicates (N)** — `[<source_index>] <description> — <reason>` (`:88-97`).
7. **Normalizations (N)** — `[<entry_index>] <field>: '<original>' -> '<normalized>'` (`:99-108`).
8. **Supported features** — the `detected_capabilities` list, one per line (`:110-116`).
9. **Suggested next command** — the `recommended_import_command` (`:118-122`).

Counts come from `crate::diagnostics::diagnostic_counts(&report.diagnostics)`, which returns `(info_count, warning_count, error_count)` in that order (`src/diagnostics.rs:152-169`). Each severity section is emitted only when its count is greater than zero.

### Diagnostic line format

```
  [e] [<entry_index or "-">] [<field or "-">]: <message>
        suggestion: <suggestion>
```

(`doctor_report.rs:42-52`, `:60-68`, `:78-83`.) The severity marker is `e` / `w` / `i`. The `entry_index` renders as `-` when `None`; the `field` renders as `-` when `None`. The suggestion line is only printed for Errors and Warnings — Info diagnostics never show a suggestion, even when one is set.

## JSON rendering

The JSON form is the plain `serde_json` serialization of `DoctorReport` — the struct is `Serialize`/`Deserialize` (`src/diagnostics.rs:107`). There is no JSON-specific shaping, renaming, or field filtering: the JSON output and the human output are two views of the same struct.

`CompatibilityDiagnostic::span` is `#[serde(skip_serializing_if = "Option::is_none")]` (`src/diagnostics.rs:36-37`), so the `span` key is **absent** from the JSON unless a span is actually present. In practice doctor never sets a span, so the key is normally omitted.

## Boundary with `doctor_cmd`

| Concern | `doctor_report.rs` | `doctor_cmd.rs` |
|---|---|---|
| Output format enum | ✅ owns it | re-exports it (`doctor_cmd.rs:50`) |
| Human rendering | ✅ owns it | calls it |
| JSON rendering | — | owns the `serde_json::to_string_pretty` call |
| Environment audit | — | ✅ |
| File analysis | — | ✅ |
| Strict elevation | — | ✅ |
| Exit-code mapping | — | ✅ |

`doctor_cmd.rs:50` re-exports the enum:

```rust
pub use super::doctor_report::DiagnosticReportFormat;
```

so `src/main.rs` can name it as `commands::doctor_cmd::DiagnosticReportFormat` while the definition stays in the rendering module.

## Key Files

| Path | Role |
|---|---|
| `src/commands/doctor_report.rs` | This module: `DiagnosticReportFormat`, `emit_human_report`. |
| `src/commands/doctor_cmd.rs` | The check orchestration that produces the report. |
| `src/diagnostics.rs` | `DoctorReport`, `CompatibilityDiagnostic`, `ImportDuplicate`, `NormalizationRecord`, `diagnostic_counts`. |
| `architecture/commands/pet_analysis.md` | The analysis layer behind the file-analysis report. |
| `architecture/diagnostics.md` | The shared diagnostic model reference. |
| `architecture/commands/doctor_cmd.md` | The command-side reference. |

## Invariants

- The human report is written to **stderr** and the JSON report to **stdout**; they never share a stream.
- The report is rendered exactly once per invocation, after all checks are complete.
- Counts in the human report come from the same `diagnostic_counts` helper the rest of the codebase uses, so the header counts always match the listed entries.
- The JSON output is a faithful serialization of `DoctorReport` — no lossy projection.
- `dry_run` is always `true` for doctor, because doctor never mutates.

## Gotchas

- The human report does **not** print a "no issues found" line (unlike `snp validate`, which prints "No issues found." — `validate_cmd.rs:620-623`). An empty doctor report is just the header.
- Info diagnostics never display their `suggestion`, even though the field is populated in some cases (e.g. `W-TYPE-MISMATCH` carries one, `I-FIELD-UNKNOWN` does not).
- The `Duplicates` and `Normalizations` sections are rendered from dedicated report fields, **not** from the diagnostics list. A duplicate that also produced a `W-DUP-CMD` diagnostic appears in both sections.
- `report.source` is `None` in `--compatibility` and `--sync` modes (`doctor_cmd.rs:472`, `:1280`), so the `Source:` line is omitted there.
- The rendering layer holds no state and performs no I/O beyond writing to stderr — it is safe to reuse for other report types, but nothing else does so today.

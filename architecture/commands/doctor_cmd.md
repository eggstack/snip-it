# doctor_cmd — Diagnostics, Pet Analysis, Environment Audit

[← Back to Overview](../overview.md)

## Overview

`src/commands/doctor_cmd.rs` (1406 lines) orchestrates `snp doctor`'s
five diagnostic modes and builds the `DoctorReport`; rendering lives
in `doctor_report.rs` (`doctor_report.md`), analysis primitives in
`pet_analysis.rs` (`pet_analysis.md`), shared types in
`src/diagnostics.rs`. All modes are read-only and side-effect-free
apart from report output and the `--check-shell` scratch tempfile.

## CLI surface

`DoctorArgs` (`doctor_cmd.rs:18`), `snp doctor` (no alias):

| Flag | Conflicts | Meaning |
|------|-----------|---------|
| `--pet-file PATH` | `compatibility, library, sync` | Analyze a pet TOML file |
| `--library NAME_OR_PATH` | `pet_file, compatibility, sync` | Analyze a library file in place |
| `--compatibility` | `pet_file, library` | Installed-environment audit (may combine with `--sync`, `--check-shell`) |
| `--sync` | `pet_file, library` | Focused sync diagnostics from the canonical snapshot |
| `--check-shell <bash\|zsh\|fish>` | — | Validate generated shell-init syntax (combinable) |
| `--strict` | — | Elevate 9 `W-*` codes to errors |
| `--report <human\|json>` | — | Rendering (default `human`) |

`DiagnosticReportFormat` is defined in `doctor_report.rs` and
re-exported here (`:50`) for CLI-schema compatibility. No mode →
`runtime_error("No mode selected")`.

## Flow / steps

`run(pet_file, compatibility, sync, check_shell, library, strict,
report_format)` (`:1228`):

1. `get_existing_library_names()` for destination-conflict checks.
2. Pet-file / library mode → `read_source_file` (empty → error) →
   `build_pet_report` (`:201`): raw structural scan
   (`detect_unknown_fields`), TOML parse (failure recorded as
   `has_toml_error`, report returned early), per-entry
   `analyze_entry`, unsupported-concept scan
   (`detect_unsupported_concepts` `:373`: unmatched `<`/`>` →
   `W-MALFORMED-VAR`, `folders` → `I-FIELD-FOLDERS`),
   destination-name conflict
   (`W-DEST-CONFLICT`), in-file duplicates, normalization preview
   (timestamps/sync-fields/ID), capability census, recommended
   `snp import pet …` command (commented-out when errors exist).
3. Compatibility mode → `build_compatibility_report(strict)` (`:470`):
   environment audit (binary version, config/library dirs, primary
   library via `inspect_library_index`, sync config presence, `snp
   select`, `snp new` acquisition flags, choice-variable parser, shell
   init syntax, `$VISUAL`/`$EDITOR`, canonical Pet TOML load, known
   legacy paths) **plus** `append_sync_diagnostics(report,
   compat_mode=true)` with `CONFIG_LOAD_FAILED` downgraded
   Error→Warning. No clipboard or theme audit exists.
4. Sync mode → `append_sync_diagnostics(report, compat_mode=false)`
   preserving native severities; always seeds a
   `compat.sync.checked` info entry, then maps each
   `StatusDiagnostic` via `map_snapshot_diagnostic` (`:70`) to dotted
   codes (`sync.config.*`, `sync.pending.*`, `sync.execution.*`,
   `sync.worker_lock.*`, `sync.status.*`, `sync.attention.*`).
5. Optional `check_shell_init` (`:1084`): maps the name to `ShellType`
   (unknown → `compat.shell_init.<name>.unknown` warning), checks
   availability with `which` (missing →
   `compat.shell_init.<name>.unavailable`, Error under `--strict`),
   then generates the init script into a tempfile and syntax-validates
   it.
6. `apply_strict_elevation` (9 `STRICT_WARNING_CODES`, `:53-63`) runs
   inside the report builders (`:368` pet/library, `:1042`
   compatibility) — the `--sync`-only and `--check-shell`-only paths
   never call it. `run` then emits human (`emit_human_report`, stderr)
   or pretty JSON (stdout); any `Error` finding → `ValidationFailed`,
   else `Success`.

## Mutation vs read-only

Read-only. No gate, no lock, no save, no runtime. Library files are
read through `read_source_file`, never loaded through the migrating
path. `--check-shell` is the one exception to "no writes": it dumps the
generated init script into a `tempfile::NamedTempFile` for syntax
validation.

## Auto-sync trigger

None. Doctor observes the snapshot/diagnostics and never writes
pending markers, status, or snippet data.

## Error / exit mapping

- Findings with `Error` severity → `CliOutcome::ValidationFailed`
  (exit 6); clean → `Success`.
- Usage errors (no mode, empty file, unknown `--library`) →
  `SnipError::runtime_error` → `main.rs`'s `Err` path, exit 1.
- `--strict` changes finding severities before the exit decision, so
  strict runs fail on warnings in the 9 listed codes.
- JSON mode prints the full `DoctorReport`; human mode writes the
  formatted sections to stderr.

## Key invariants

- Orchestration vs rendering split: `doctor_cmd` never formats;
  `doctor_report` never checks. No generic finding DSL — each consumer
  keeps its own semantics over `inspect_library_index`.
- Sync diagnostics always derive from `capture_snapshot()` — no
  second, driftable sync-health implementation.
- Pet analysis and import share `pet_analysis` primitives, so
  `doctor --pet-file` predictions match `import pet` behavior.
- Escaped `\<`/`\>` never trigger `W-MALFORMED-VAR`.

## File / line references

- `DoctorArgs`: `src/commands/doctor_cmd.rs:18`; strict codes: `:53`
- Snapshot mapping: `:70,165`; pet report: `:201`; compat: `:470`
- Shell check: `:1084`; `run`: `:1228`; sanitize: `:437`
- Rendering: `src/commands/doctor_report.rs`; dispatch: `src/main.rs:756-768`

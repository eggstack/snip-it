# snp doctor

[← Back to Overview](../overview.md) · [Command index](mod.md) · [doctor_report.md](doctor_report.md) · [pet_analysis.md](pet_analysis.md) · [status.md](../status.md)

`snp doctor` audits the installed snp environment and analyses snippet files. It answers two different questions:

- **Environment audit** (`--compatibility`): is this installation healthy — config directory, library directory, primary library, sync config, shells, `snp select`, `snp new` acquisition flags, the choice-variable parser, shell-init syntax, editor configuration, the Pet TOML parser, legacy paths, and the auto-sync state?
- **File analysis** (`--pet-file <PATH>` / `--library <NAME>`): what will this snippet file do when imported — unknown fields, type mismatches, malformed variables, duplicates, and the normalizations that import would apply?

`snp doctor` is **read-only**. It writes nothing to the config directory, takes no lock, and calls no mutation gate.

## CLI arguments

```rust
pub struct DoctorArgs {
    #[arg(long = "pet-file", value_name = "PATH", conflicts_with_all = ["compatibility", "library", "sync"])]
    pub pet_file: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["pet_file", "library"])]
    pub compatibility: bool,
    #[arg(long, conflicts_with_all = ["pet_file", "library"])]
    pub sync: bool,
    #[arg(long, value_enum)]
    pub check_shell: Option<super::shell_cmd::ShellIntegration>,
    #[arg(long, value_name = "NAME_OR_PATH", conflicts_with_all = ["pet_file", "compatibility", "sync"])]
    pub library: Option<String>,
    #[arg(long)]
    pub strict: bool,
    #[arg(long, value_enum, default_value = "human")]
    pub report: DiagnosticReportFormat,
}
```

(`src/commands/doctor_cmd.rs:16-48`.)

| Flag | Type | Default | Effect |
|---|---|---|---|
| `--pet-file <PATH>` | `Option<PathBuf>` | `None` | Analyse a Pet TOML snippet file. Mutually exclusive with `--compatibility`, `--library`, `--sync`. |
| `--compatibility` | flag | off | Run the environment audit. Mutually exclusive with `--pet-file` and `--library`. |
| `--sync` | flag | off | Focused sync diagnostics from the canonical status snapshot. Mutually exclusive with `--pet-file` and `--library`. |
| `--check-shell <SHELL>` | `Option<ShellIntegration>` | `None` | Validate the shell-init output for `bash`, `zsh`, or `fish`. May be combined with the other modes. |
| `--library <NAME_OR_PATH>` | `Option<String>` | `None` | Analyse an existing library. Accepts a library **name** or a literal file path. Mutually exclusive with `--pet-file`, `--compatibility`, `--sync`. |
| `--strict` | flag | off | Elevate a designated set of warnings to errors. |
| `--report <FORMAT>` | `DiagnosticReportFormat` | `human` | `human` (stderr) or `json` (stdout). |

**A mode is required.** With no mode selected the command errors with "No mode selected" and lists the options (`doctor_cmd.rs:1237-1247`).

### Mode selection order

`run()` (`doctor_cmd.rs:1228-1318`) picks the mode in this precedence: `--pet-file` → `--library` → `--compatibility` → `--sync` → empty report (`:1251-1285`). `--check-shell` is applied **after** whichever mode was selected, on top of the same report (`:1288-1290`).

## Diagnostic model

Doctor uses the `CompatibilityDiagnostic` family from `src/diagnostics.rs`, **not** the `ValidationDiagnostic` family that `snp validate` uses:

```rust
pub enum DiagnosticSeverity { Info, Warning, Error }

pub struct CompatibilityDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub entry_index: Option<usize>,
    pub field: Option<String>,
    pub suggestion: Option<String>,
    pub span: Option<SourceSpan>,
}
```

(`src/diagnostics.rs:9-38`.) There is **no generic finding DSL**: `validate`, `doctor`, and `import` each keep their own diagnostic types and their own pass/fail semantics. What they *do* share is the classification input — `LibraryManager::inspect_library_index()` and `crate::library::find_orphaned_ids` — so the commands never disagree about the underlying state.

Doctor's report is `DoctorReport` (`src/diagnostics.rs:106-145`), documented in `doctor_report.md`.

## Environment audit (`--compatibility`)

`build_compatibility_report(strict)` (`doctor_cmd.rs:470-1044`) emits these diagnostic families:

| Code family | What it checks | Severity |
|---|---|---|
| `compat.binary_version` | The running binary's version and path | Info |
| `compat.config_dir.ok` / `.readonly` / `.missing` | Config directory exists and is writable | Info / Warning / Warning |
| `compat.libraries_dir.ok` / `.missing` | Library directory exists (missing = single-file mode, not an error) | Info |
| `compat.primary_library.ok` / `.missing` | Primary library resolved via `inspect_library_index()` | Info / Warning |
| `compat.library_manager.error` | `LibraryManager::new()` failed | Warning |
| `compat.sync.enabled` / `.disabled` / `.config_error` / `.absent` | `sync.toml` presence, parseability, and enablement | Info / Info / Warning / Info |
| `compat.shell.<name>.ok` / `.missing` | `bash`, `zsh`, `fish` on `PATH` | Info |
| `compat.select.ok` / `.missing` | `snp select --help` succeeds | Info / Warning |
| `compat.acquire.command_stdin` / `.from_file` / `.editor` | `snp new --help` advertises the acquisition flags | Info / Warning |
| `compat.choice_parser.ok` / `.missing` | The choice-variable parser is functional | Info / Warning |
| `compat.shell_syntax.<name>.ok` / `.invalid` / `.init_failed` / `.unavailable` | `snp shell init <shell>` piped through the shell's syntax checker | Info / Warning / Warning / Info |
| `compat.editor.ok` / `.unset` | `$EDITOR` / `$VISUAL` | Info |
| `compat.pet_toml.ok` / `.error` | The Pet TOML parser works on a canonical sample | Info / **Error** |
| `compat.legacy.single_file` / `.config_dir` | Legacy `~/.config/snippets.toml` and `~/.snip-it` are present | Info |
| `compat.sync.checked` | Always emitted — records that sync was inspected | Info |

Notes on the trickier ones:

- **Shell syntax** (`:838-928`): for each of `bash`, `zsh`, `fish` the command shells out to `snp shell init <shell>` and pipes the output into `<shell> -n` (bash/zsh) or `fish --no-execute`. A missing shell produces `.unavailable` (Info); a non-zero syntax check produces `.invalid` (Warning); a failed `snp shell init` produces `.init_failed` (Warning).
- **Choice parser** (`:806-836`): parses the literal string `echo <color=|_red_||_green_||_blue_||>` and checks that a `VariableKind::Choices` variable comes out.
- **Primary library** (`:562-611`): uses `LibraryManager::new()` then `&mgr.inspect_library_index().primary`. `PrimaryState::Present` and `PrimaryState::FileMissing` are both reported as `compat.primary_library.ok` with the snippet count (0 for a missing file); `NoPrimary` and `NoLibraries` produce `compat.primary_library.missing`.

## Sync diagnostics (`--sync` and the tail of `--compatibility`)

`append_sync_diagnostics(report, compat_mode)` (`doctor_cmd.rs:165-184`) calls `status_snapshot::capture_snapshot()` and maps every `StatusDiagnostic` into a `CompatibilityDiagnostic` via `map_snapshot_diagnostic` (`:70-163`).

- `--sync` alone uses `compat_mode = false`: the snapshot's native severities are preserved, including `Error` (`doctor_cmd.rs:1276-1282`).
- `--compatibility` uses `compat_mode = true`: the `CONFIG_LOAD_FAILED` severity is downgraded from Error to Warning for backward compatibility (`doctor_cmd.rs:68-70`).

A summary diagnostic `compat.sync.checked` is always pushed first so the report shows that sync was inspected (`doctor_cmd.rs:168-177`).

## File analysis (`--pet-file`, `--library`)

`build_pet_report(source_path, content, strict, existing_library_names)` (`doctor_cmd.rs:200-370`):

1. `detect_unknown_fields(content)` on the raw TOML (`doctor_cmd.rs:211`).
2. `parse_pet_toml(content)`; a parse failure sets `has_toml_error` / `toml_error_detail` and **returns early** (`:215-222`).
3. `analyze_entry(i, pet)` per entry (`:227-230`).
4. `detect_unsupported_concepts(&pet_snippets, &mut report)` (`:233`) — unmatched `<` / `>` in a command produce `W-MALFORMED-VAR`, and a non-empty `folders` field produces `I-FIELD-FOLDERS` (`:373-434`). The angle-bracket scan skips escaped characters (`\`) (`:380-384`).
5. **Destination conflict** (`:236-254`): the source file stem is sanitized with `sanitize_library_name` (`:437-467` — lowercase, non-alphanumerics → `-`, collapse runs, trim hyphens, fallback `"imported"`). If a library with that name already exists, `W-DEST-CONFLICT` warns that the import will need `--merge` or `--replace`.
6. **Within-source duplicates** via `detect_duplicates` (`:257-259`).
7. **Normalization preview** (`:262-301`): zero timestamps, a non-empty `device_id` / `deleted` flag, and a pre-existing `id` — each recorded as a `NormalizationRecord` describing what import *would* do.
8. **Detected capabilities** (`:304-343`): `toml_format`, `snippet_count=<N>`, and conditionally `variables`, `choice_variables`, `output_fields`, `tags`.
9. **Recommended import command** (`:345-357`): `snp import pet <path>`, or a commented-out version prefixed with "# Resolve errors first, then run:" when the report contains errors.
10. A default suggestion "Fix this entry before importing" is attached to every error diagnostic that lacks one (`:359-364`).

`--library <NAME_OR_PATH>` resolves the target with `resolve_library_path` (`:1049-1069`): a literal file path wins; otherwise `<config>/libraries/<name>.toml`; otherwise "Library not found". The file is then read through the same `read_source_file` path and analysed identically to a Pet file.

## Strict mode

`--strict` elevates the following warning codes to errors (`doctor_cmd.rs:52-63`, applied by `apply_strict_elevation` at `:186-198`):

```rust
const STRICT_WARNING_CODES: &[&str] = &[
    "W-MALFORMED-VAR", "W-DUP-CMD", "W-DUP-DESC", "W-DEST-CONFLICT",
    "W-DESC-MISSING", "W-CMD-MISSING", "W-DESC-EMPTY",
    "W-TYPE-MISMATCH", "W-TAG-EMPTY",
];
```

Note this list is **doctor's own** and differs from `validate`'s `STRICT_CODES` (`validate_cmd.rs:708-713`).

`check_shell_init` also uses `strict` to choose Error over Warning for an unavailable shell or an invalid syntax check (`doctor_cmd.rs:1119-1137`, `:1196-1212`).

## Output and exit codes

| Mode | Stream | Content |
|---|---|---|
| `--report json` | stdout | the serialized `DoctorReport` (`doctor_cmd.rs:1297-1302`) |
| `--report human` (default) | stderr | the rendered report — see `doctor_report.md` |

Exit outcome (`doctor_cmd.rs:1305-1317`):

| Condition | Outcome | Exit code |
|---|---|---|
| No `DiagnosticSeverity::Error` diagnostic | `CliOutcome::Success` | `0` |
| At least one `DiagnosticSeverity::Error` diagnostic | `CliOutcome::ValidationFailed` | `6` (`VALIDATION_FAILED`) |

`--strict` can turn exit `0` into exit `6` by elevating warnings.

## Interaction with the transaction journal and locks

| Concern | Behavior |
|---|---|
| Mutation gate | **Not called.** |
| Local-data lock | **Not taken.** |
| Transaction lock | **Not taken.** |
| Journal | None. |
| Writes | None. The only side effects are reading files and spawning helper processes (`which`, `snp select --help`, `snp new --help`, `snp shell init`, the shell syntax checker). |

The only temp file doctor creates is the shell-init syntax-check fixture, which `tempfile::NamedTempFile` removes on drop (`doctor_cmd.rs:1140-1154`).

## Key Files

| Path | Role |
|---|---|
| `src/commands/doctor_cmd.rs` | This command: modes, the environment audit, file analysis, strict elevation. |
| `src/commands/doctor_report.rs` | Report rendering (`DiagnosticReportFormat`, `emit_human_report`). |
| `src/diagnostics.rs` | `DoctorReport`, `CompatibilityDiagnostic`, `DiagnosticSeverity`. |
| `src/commands/pet_analysis.rs` | `read_source_file`, `parse_pet_toml`, `detect_unknown_fields`, `analyze_entry`, `detect_duplicates`. |
| `src/status_snapshot.rs` | `capture_snapshot` — the canonical sync status view. |
| `src/library/manager.rs` | `inspect_library_index` — the shared primary-state view. |
| `src/commands/shell_cmd.rs` | `generate_bash` / `generate_zsh` / `generate_fish`. |
| `architecture/commands/doctor_report.md` | The rendering layer. |
| `architecture/diagnostics.md` | The shared diagnostic model. |

## Invariants

- Doctor is read-only: it never writes to the config directory, never creates a journal, never takes a lock.
- Every diagnostic carries a stable dotted or dashed `code`, a severity, a message, and (where applicable) an entry index, field, and suggestion.
- The primary-library classification comes from the shared `inspect_library_index()`, so `doctor`, `validate`, and `repair` agree.
- The sync view comes from the canonical `status_snapshot::capture_snapshot()`, so doctor does not re-implement sync state.
- `--check-shell` is additive: it augments whichever mode was selected rather than replacing it.
- A file-analysis report is produced even when the file does not parse — `has_toml_error` / `toml_error_detail` carry the failure instead of aborting.

## Gotchas

- **A mode is mandatory.** `snp doctor` with no flags is an error, not an environment audit.
- `--compatibility` **downgrades** `CONFIG_LOAD_FAILED` from Error to Warning, while `--sync` keeps the snapshot's native severity (`doctor_cmd.rs:68-70`, `:1276-1282`). The same underlying condition therefore yields different exit codes in the two modes.
- The environment audit spawns **child processes** (`which`, `snp select --help`, `snp new --help`, `snp shell init`). In an environment where `snp` is not on `PATH`, the acquisition-flag checks will report "missing" even though the running binary supports them.
- `compat.pet_toml.error` is the only `Error` severity produced by the environment audit itself; every other environment problem is a Warning.
- `--library` accepts a **path or a name**. A relative path that happens to exist on disk wins over the name lookup (`doctor_cmd.rs:1050-1053`).
- The doctor strict-code list and the validate strict-code list are **different**. A code elevated by one is not necessarily elevated by the other.
- `W-DEST-CONFLICT` compares against existing library **names** using doctor's `sanitize_library_name`, which is a separate implementation from the import layer's `derive_library_name` (`import_cmd.rs:48-80`). The two sanitizers agree today but are not shared code.

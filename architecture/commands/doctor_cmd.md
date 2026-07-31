# doctor_cmd — Diagnostics and Compatibility Analysis

**Source:** `src/commands/doctor_cmd.rs`

## Purpose

Provides three distinct diagnostic modes:

1. **Pet file analysis** — Analyzes a pet TOML snippet file for compatibility with snp
2. **Environment audit** — Checks the installed snp environment for common issues
3. **Sync diagnostics** — Runs focused sync diagnostics using the canonical status snapshot
4. **Shell syntax check** — Validates generated shell integration code
5. **Library check** — Validates a specific library file

## Modes

### `--pet-file <PATH>`
Analyzes a pet TOML file for import compatibility:
- Parses the file using `pet_analysis::parse_pet_toml()`
- Runs `analyze_entry()` on each snippet for variable syntax, field presence, etc.
- Detects duplicates via `detect_duplicates()`
- Detects unknown fields via `detect_unknown_fields()`
- Reports diagnostics as human-readable or JSON

### `--compatibility`
Audits the installed snp environment:
- Checks config directory existence
- Validates `libraries.toml` structure
- Checks library file permissions (0o600 on Unix)
- Verifies backup directory state
- Reports findings as `CompatibilityDiagnostic` list

### `--sync`
Runs sync-focused diagnostics:
- Captures a `StatusSnapshot` via `status_snapshot::capture_snapshot()`
- Maps `StatusDiagnostic` entries to doctor-compatible codes
- Reports pending state, lock health, execution status, and config validity

### `--check-shell <bash|zsh|fish>`
Validates generated shell integration code:
- Generates code via `shell_cmd::generate_*()`
- Runs the shell's syntax checker (`bash -n`, `zsh -n`, `fish --no-execute`)
- Reports pass/fail with stderr output

### `--library <NAME_OR_PATH>`
Validates a specific library file:
- Loads the library via `load_library()`
- Checks snippet structure, field validity, ID uniqueness
- Reports diagnostics

## Output Formats

| Format | Flag | Destination |
|--------|------|-------------|
| Human | `--report human` (default) | stderr |
| JSON | `--report json` | stdout |

## Strict Mode

`--strict` elevates designated warning codes to errors. The `STRICT_WARNING_CODES` list includes:
- `W-MALFORMED-VAR` — invalid variable syntax
- `W-DUP-CMD` / `W-DUP-DESC` — duplicate commands or descriptions
- `W-DEST-CONFLICT` — import destination conflict
- `W-DESC-MISSING` / `W-CMD-MISSING` — missing required fields
- `W-DESC-EMPTY` / `W-CMD-EMPTY` / `W-TAG-EMPTY` — empty field values
- `W-TYPE-MISMATCH` — field type mismatch

## Diagnostic Code Mapping

The doctor command maps `StatusDiagnostic` codes from the status snapshot to dotted diagnostic codes for the doctor report:

| Snapshot Code | Doctor Code |
|--------------|-------------|
| `CONFIG_LOAD_FAILED` | `sync.config.load_failed` |
| `NOT_CONFIGURED` | `sync.config.not_configured` |
| `PENDING_CORRUPT` | `sync.pending.corrupt` |
| `EXECUTION_LOCK_STALE` | `sync.execution.dead_stale` |
| `WORKER_LOCK_STALE` | `sync.worker_lock.dead_stale` |
| `ATTENTION_REQUIRED` | `sync.attention.*` (varies by failure class) |

## Integration Points

- **`pet_analysis`** — Core analysis functions for pet file compatibility
- **`status_snapshot`** — Canonical status projection for sync diagnostics
- **`shell_cmd`** — Code generation for shell syntax validation
- **`library`** — Library loading and validation
- **`diagnostics`** — `CompatibilityDiagnostic`, `DoctorReport`, and related types

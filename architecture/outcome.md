# CLI Outcome Types

[← Back to Overview](overview.md) · [← Back to CLI](cli.md)

## Overview

The outcome module (`src/outcome.rs`) provides typed CLI exit codes and a
centralized exit mapper. It defines `CliOutcome` for public command results
and the `exit_code` module for stable, documented process exit codes.

## `CliOutcome`

The typed application outcome for public CLI exit-code mapping:

```rust
#[non_exhaustive]
pub enum CliOutcome {
    Success,
    NotFound,
    Ambiguous,
    Cancelled,
    ValidationFailed,
    PersistenceFailed,
    SyncFailed,
    ExecutionFailed { child_code: Option<i32> },
    ConflictOrRefused,
}
```

Each variant maps to a stable exit code via `CliOutcome::exit_code()`.

## Exit Code Table

| Constant | Code | Variant | Meaning |
|----------|------|---------|---------|
| `SUCCESS` | 0 | `Success` | Command completed successfully |
| `GENERAL_ERROR` | 1 | `PersistenceFailed` | General operational failure |
| `USAGE_ERROR` | 2 | — | CLI usage/argument error (Clap-controlled) |
| `NOT_FOUND` | 3 | `NotFound` | Snippet or resource not found |
| `CANCELLED` | 4 | `Cancelled` | User cancelled an interactive action |
| `AMBIGUOUS` | 5 | `Ambiguous` | Multiple matches found, unique policy requested |
| `VALIDATION_FAILED` | 6 | `ValidationFailed` | Data validation or persistence failure |
| `SYNC_FAILED` | 7 | `SyncFailed` | Synchronization with remote server failed |
| `EXECUTION_FAILED` | 8 | `ExecutionFailed` | Snippet execution failed (no child code) |
| `CONFLICT_OR_REFUSED` | 9 | `ConflictOrRefused` | Destructive action refused or generation changed |
| `UNSAFE_REPAIRS` | 10 | — | Repairs found but not applied — unsafe items await operator decision |

## Special Cases

- **`PersistenceFailed`** maps to code 1 (`GENERAL_ERROR`), not a unique code.
  This is intentional: persistence failures are operational errors, not a
  distinct user-facing category.

- **`ExecutionFailed`** propagates the child process exit code when available
  (e.g., `Some(127)` for command-not-found). When no child code is available,
  it falls back to code 8.

- **`USAGE_ERROR` (2)** is not a `CliOutcome` variant. It is produced by
  Clap's error handling when invalid arguments are provided. Commands never
  return this code directly.

## Centralized Exit Mapper

The exit code mapping is centralized in `CliOutcome::exit_code()`:

```rust
impl CliOutcome {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliOutcome::Success => exit_code::SUCCESS,
            CliOutcome::NotFound => exit_code::NOT_FOUND,
            // ... etc
        }
    }
}
```

The `main.rs` dispatch converts the final `SnipResult<CliOutcome>` to a
process exit code by calling `outcome.exit_code()` on success, or mapping
`SnipError` variants to appropriate codes.

## Relationship to Other Outcome Types

```
SnippetSelection (TUI layer)
    ↓
SelectionOutcome (lib layer: Selected / Cancelled / ExecutionFailed)
    ↓
CliOutcome (public exit code layer)
```

- `SnippetSelection` is the TUI-level selection result (Selected, Cancelled, etc.)
- `SelectionOutcome` is the raw TUI-loop result preserved because it models UI-only state
- `ProcessResult` is the per-snippet loop control (`Cancel`/`Continue`/`Done`/`Failed`) inside `run_snippet_selection`; not an exit-code layer
- `CliOutcome` is the final typed outcome for exit-code mapping

Commands convert their internal outcomes to `CliOutcome` directly before
returning. There is no intermediate command-outcome layer. `main.rs`
maps `Ok(CliOutcome)` to `outcome.exit_code()` (with `Success` → no exit)
and `Err(SnipError)` to exit 1. `repair` uses `exit_on_repair_status`
for its `UnsafeOnly` (10) / `PartialFailure` (1) cases, which have no
`CliOutcome` variant.

## Non-Exhaustive

`CliOutcome` is marked `#[non_exhaustive]`, allowing future variant additions
without breaking downstream callers.

## Tests

Unit tests in `src/outcome.rs` verify:

- All variants map to their documented exit codes
- `ExecutionFailed` with a child code propagates that code
- `PersistenceFailed` maps to general error (1)
- All exit codes are distinct (no collisions)
- `OutputContext` suppresses ANSI in machine modes
- `OutputContext` strips ANSI sequences when `suppress_ansi()` is true

## `OutputContext`

The machine-output guard ensures stdout is not contaminated in
non-interactive and machine-readable modes:

```rust
pub struct OutputContext {
    pub mode: OutputMode,      // Human, Json, Csv, Raw, Field, Expanded
    pub color: ColorPolicy,    // Auto, Always, Never
    pub interactive: bool,
}
```

### Rules

| Rule | Enforcement |
|------|-------------|
| Data only on stdout | `write_stdout()` handles broken pipe gracefully |
| Diagnostics on stderr | `diagnostic()` writes to stderr |
| No ANSI in machine mode | `suppress_ansi()` returns true for machine modes |
| No update notices | Commands check `ctx.is_machine_mode()` before printing |
| No tracing on stdout | Tracing subscriber uses stderr |
| No prompts | Machine modes never prompt |
| Exact-byte output | `write_all()` without trailing newline |

### Construction

```rust
OutputContext::human()   // Interactive, Auto color
OutputContext::json()    // Machine, Never color
OutputContext::csv()     // Machine, Never color
OutputContext::raw()     // Machine, Never color
OutputContext::field()   // Machine, Never color
```

### Integration

Commands check `ctx.suppress_ansi()` before formatting output, and
use `ctx.write_stdout()` for byte-safe output that handles broken pipe
without noise or backtraces.

### Key functions — `src/outcome.rs:151-272`

```rust
OutputContext::human() -> Self
OutputContext::json() / csv() / raw() / field() -> Self // machine, Never color, non-interactive
pub fn is_machine_mode(&self) -> bool    // Json | Csv | Raw | Field
pub fn suppress_ansi(&self) -> bool      // Never, machine mode, or Auto + non-interactive
pub fn write_stdout(&self, data: &[u8]) -> io::Result<()> // BrokenPipe → Ok
pub fn writeln(&self, text: &str) -> io::Result<()>       // adds trailing newline
pub fn diagnostic(&self, text: &str)                     // stderr only
pub fn strip_ansi_if_needed(&self, text: &str) -> String // CSI (any final byte) + OSC (BEL/ST)
```

`Expanded` is intentionally *not* machine mode (variable-expanded human
display). Exact-byte modes (`Raw`, `Field`) use `write_all` with no
trailing newline.

## Stream policy

Authoritative refs: `docs/EXIT_CODES.md`, `docs/CLI_EXITCODE_STREAM_POLICY.md`.

- **Exit codes are implemented and stable** (0–10 via `CliOutcome`,
  table above). Stream separation is **aspirational**: human-readable
  output still goes to stdout in several commands; only `import`/`doctor`
  JSON splits are clean (JSON → stdout, human → stderr).
- Enforced today: data only on stdout, diagnostics on stderr
  (`diagnostic()`), no ANSI in machine modes (`suppress_ansi()` +
  `strip_ansi_if_needed()`), no update notices / auto-sync advisories /
  prompts / spinners in machine mode, tracing subscriber on stderr,
  broken pipe swallowed.
- `main.rs` maps `Ok(outcome)` → `outcome.exit_code()` (`Success` → no
  exit) and `Err(SnipError)` → exit 1; `repair` bypasses `CliOutcome`
  via `exit_on_repair_status` for `UnsafeOnly` (10) / `PartialFailure` (1).
- Detached auto-sync worker codes are internal, not part of the public
  contract. With `auto_sync_failure = "error"`, a post-commit spawn
  failure surfaces exit 1 via `SnipError::Runtime` — local mutation stays
  committed; worker-side failures surface via logs / `snp doctor` only.
- Cancellation: `run`/`clip`/`search` treat TUI cancel as success
  (exit 0); `snp select` maps cancel → `CliOutcome::Cancelled` → exit 4
  at the CLI boundary. Failed `run` records no usage metadata.

## Invariants / gotchas (from AGENTS.md)

- `CliOutcome` is `#[non_exhaustive]` — add variants, never renumber
  codes (`docs/EXIT_CODES.md` + `--help` document them).
- `PersistenceFailed` → 1 (`GENERAL_ERROR`) deliberately shares the
  general code for backward compat; `USAGE_ERROR` (2) comes from Clap,
  never from command code; `UNSAFE_REPAIRS` (10) has no variant.
- `ExecutionFailed` propagates the child code when known (0–255),
  else 8 (spawn failure, signal kill, `SNP_COMMAND_TIMEOUT`).
- Never leak secrets through `diagnostic()` or stdout payloads; machine
  output must stay pipe-safe (no prompts, no ANSI, exact bytes).

## File / line refs

- `src/outcome.rs:18-40` (`CliOutcome`), `:46-70` (`exit_code::*`
  0–10), `:72-91` (`exit_code()` mapper), `:93-149`
  (`ColorPolicy`, `OutputMode`, `OutputContext`), `:151-272`
  (constructors + guards), `:274-376` (tests: mapping, child-code
  propagation, distinctness, ANSI stripping).
- `docs/EXIT_CODES.md` (stable table, child-code rules, shell example),
  `docs/CLI_EXITCODE_STREAM_POLICY.md` (per-command streams, implemented
  vs aspirational split).

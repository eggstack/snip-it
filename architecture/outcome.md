# CLI Outcome Types

[← Back to Overview](overview.md) · [← Back to CLI](cli.md)

## Overview


The outcome module (`src/outcome.rs`) provides typed CLI exit codes and a
centralized exit mapper. It defines `CliOutcome` for public command results,
the `exit_code` module for stable, documented process exit codes, and
`OutputContext` for the stdout/stderr and ANSI discipline.

This doc covers the outcome types and their exit codes. It does not cover
which command produces which outcome (see [cli.md](cli.md) and
[commands/mod.md](commands/mod.md)), the error type that feeds them (see
[core.md](core.md)), or the per-command JSON field schemas (see
`docs/JSON_SCHEMAS.md`).

## `CliOutcome`


The typed application outcome for public CLI exit-code mapping
(`src/outcome.rs:18-42`):

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

Each variant maps to a stable exit code via `CliOutcome::exit_code()`
(`src/outcome.rs:76-93`).

## Exit Code Table


| Constant | Code | Variant | Meaning |
|----------|------|---------|---------|
| `SUCCESS` | 0 | `Success` | Command completed successfully |
| `GENERAL_ERROR` | 1 | `PersistenceFailed` | General operational failure |
| `USAGE_ERROR` | 2 | — | CLI usage/argument error (Clap-controlled) |
| `NOT_FOUND` | 3 | `NotFound` | Snippet or resource not found |
| `CANCELLED` | 4 | `Cancelled` | User cancelled an interactive action |
| `AMBIGUOUS` | 5 | `Ambiguous` | Multiple matches found, unique policy requested |
| `VALIDATION_FAILED` | 6 | `ValidationFailed` | Data validation failure |
| `SYNC_FAILED` | 7 | `SyncFailed` | Synchronization with remote server failed |
| `EXECUTION_FAILED` | 8 | `ExecutionFailed` | Snippet execution failed (no child code) |
| `CONFLICT_OR_REFUSED` | 9 | `ConflictOrRefused` | Destructive action refused or generation changed |
| `UNSAFE_REPAIRS` | 10 | — | Repairs found but not applied — unsafe items await operator decision |

The constants live in `outcome::exit_code` (`src/outcome.rs:48-72`).

## Special Cases


- **`PersistenceFailed`** maps to code 1 (`GENERAL_ERROR`), not a unique code.
  This is intentional: persistence failures are operational errors, not a
  distinct user-facing category (`src/outcome.rs:83`).

- **`ExecutionFailed`** propagates the child process exit code when available
  (e.g., `Some(127)` for command-not-found). When no child code is available,
  it falls back to code 8 (`src/outcome.rs:85-89`).

- **`USAGE_ERROR` (2)** is not a `CliOutcome` variant. It is produced by
  Clap's error handling when invalid arguments are provided. Commands never
  return this code directly.

- **`UNSAFE_REPAIRS` (10)** is not a `CliOutcome` variant either — see
  [Exit code 10](#exit-code-10--the-one-code-without-an-outcome).

## Exit code 10 — the one code without an outcome


`exit_on_repair_status()` (`src/main.rs:401-414`) maps
`commands::repair_cmd::RepairExitStatus` straight to a process exit code and
calls `std::process::exit` **inside** the handler:

| `RepairExitStatus` | Exit code | Mechanism |
|--------------------|-----------|-----------|
| `Clean` / `Repaired` / `FindingsSafe` / `DryRun` | 0 | implicit return, no `exit` call |
| `PartialFailure` | 1 (`GENERAL_ERROR`) | `std::process::exit(1)` |
| `UnsafeOnly` | 10 (`UNSAFE_REPAIRS`) | `std::process::exit(10)` |

`UnsafeOnly` deliberately does **not** reuse 2 (`USAGE_ERROR`): a valid
invocation that found unsafe repairs awaiting an operator decision is not a
usage error, and scripts must be able to distinguish the two
(`src/main.rs:395-400`).

Because the exit happens inside the handler, `handle_repair`
(`src/main.rs:458-462`) never returns and there is deliberately **no**
`CliOutcome` variant for it. A reviewer looking for "the variant that maps to
10" in `src/outcome.rs` will not find one — that is by design, not an
omission.

## Centralized Exit Mapper


The exit code mapping is centralized in `CliOutcome::exit_code()`
(`src/outcome.rs:74-93`):

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

`main()` performs the final conversion (`src/main.rs:1055-1070`):

| Handler result | Behavior | Source |
|----------------|----------|--------|
| `Ok(CliOutcome::Success)` | fall through, exit 0 | `src/main.rs:1056` |
| `Ok(other)` | `std::process::exit(other.exit_code())` | `src/main.rs:1061` |
| `Err(SnipError)` | `eprintln!("error: {e}")` then exit 1 | `src/main.rs:1063-1069` |

There is no `From<SnipError> for CliOutcome`. A handler that wants code 3, 4,
5, 6, 7, or 9 returns the matching `CliOutcome` variant instead of
propagating the error; a handler with nothing more specific returns
`Err(SnipError)` and the process exits 1. See
[core.md](core.md#error--outcome--exit-code).

Two command paths bypass `CliOutcome` entirely and call `std::process::exit`
themselves:

- `repair` / `data repair` — exit 1 or 10 via `exit_on_repair_status`
  (`src/main.rs:401-414`, `src/main.rs:458-462`).
- `run` (exact and TUI) — exit the child's code (fallback 8) after a failed
  execution (`src/main.rs:545`, `src/main.rs:569`).

`snp clip` does **not** propagate the child's exit code: the exact path
returns `CliOutcome::Success` unconditionally (`src/main.rs:589`) and the TUI
path returns `SnipResult<()>`, so a clipboard failure surfaces as
`Err(SnipError)` → exit 1, not as the child's code.

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
and `Err(SnipError)` to exit 1.

## Non-Exhaustive


`CliOutcome` is marked `#[non_exhaustive]` (`src/outcome.rs:17`), allowing
future variant additions without breaking downstream callers.

## JSON output (machine-readable emitters)


There is **no single JSON envelope** shared by all commands. Each command
serializes its own top-level structure directly to stdout. `docs/JSON_SCHEMAS.md`
is the authoritative field-level reference; this is the emitter index.

| Command | Flag | Top-level JSON shape | Version field |
|---|---|---|---|
| `snp list --json` | `--json` | Bare **array** of snippet objects (`description`, `command`, `output`, `tags`, `folders`, `favorite`) — no wrapper | none |
| `snp get --json` | `--json` | Object: `schema`, `id`, `description`, `command`, `expanded`, `tags`, `library`, `library_id` (`src/commands/get_cmd.rs:66-77`) | `schema: 1` |
| `snp status --json` | `--json` | Object: `StatusSnapshot` (`src/status_snapshot.rs:17-29`) | `schema: u32` |
| `snp doctor --report json` | `--report json` | Object: diagnostic report with findings + remediation | per report |
| `snp validate --json` | `--json` | Object: `ValidationReport` — `schema_version`, `tool_version`, `strict_mode`, … (`src/commands/validate_cmd.rs:60-63`) | `schema_version: "1.0.0"` (string) |
| `snp backup --json` | `--json` | Object: `BackupManifest` (`src/commands/backup_archive.rs:349-355`) | `schema: 1` |
| `snp restore --json` | `--json` | Object: restore plan/result (`mode`, `manifest_schema`, `manifest_version`, `files_in_backup`, …) (`src/commands/restore_cmd.rs:1094-1098`) | `manifest_schema` (from the manifest) |
| `snp repair --json` | `--json` | Object: `items`, `backups`, `applied`, `skipped`, `failed`, `exit_status` (`src/commands/repair_cmd.rs:997-1005`) | `exit_status` string |
| `snp import --report json` | `--report json` | Object: import report (`analysis_mode`, `duplicates`, `diagnostics`, `normalizations`, `detected_capabilities`) | per report |
| Local MCP tools | — | JSON object in `structuredContent` **and** a text content item | per tool |

Rules that hold for every emitter (from `docs/JSON_SCHEMAS.md`):

- Field names are `snake_case`; no ANSI escapes; no secret values.
- Timestamps are epoch integers (`*_unix_ms`, or `created_at` / `updated_at`
  in epoch seconds) — no ISO-8601 strings anywhere.
- Struct-variant enums serialize as **objects**
  (`{"LiveExecution":{…}}`), not bare strings.
- Optional fields are sometimes `null` and sometimes omitted entirely
  (`#[serde(skip_serializing_if = "Option::is_none")]`) — do not assume a key
  is always present.
- Breaking changes bump the command's schema version; additive changes do not.

`repair --json` is the only emitter that also reports the repair exit status
inline (`exit_status` ∈ `clean` / `repaired` / `partial_failure` /
`unsafe_only` / `findings_safe` / `dry_run`,
`src/commands/repair_cmd.rs:1007-1014`).

## Tests


Unit tests in `src/outcome.rs` verify:

- All variants map to their documented exit codes (`src/outcome.rs:280-301`)
- `ExecutionFailed` with a child code propagates that code
- `PersistenceFailed` maps to general error (1)
- All exit codes are distinct (no collisions) (`src/outcome.rs:350-377`)
- `OutputContext` suppresses ANSI in machine modes
- `OutputContext` strips ANSI sequences when `suppress_ansi()` is true

## `OutputContext`


The machine-output guard ensures stdout is not contaminated in
non-interactive and machine-readable modes (`src/outcome.rs:144-151`):

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

### Key functions — `src/outcome.rs:153-274`

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

- **Exit codes are implemented and stable** (0–10 per the table above; 10
  bypasses `CliOutcome`). Stream separation is **aspirational**: human-readable
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
  failure is logged at error level only
  (`src/auto_sync/notification.rs:242-253`) — the command still exits 0 and
  the local mutation stays committed; the pending marker is preserved for
  recovery, and `report_notification_result` prints a stderr warning
  (`src/auto_sync/notification.rs:43-55`). Worker-side failures surface via
  logs / `snp doctor` only.
- Cancellation: `run`/`clip`/`search` treat TUI cancel as success
  (exit 0); `snp select` maps cancel → `CliOutcome::Cancelled` → exit 4
  at the CLI boundary. Failed `run` records no usage metadata.

## Invariants


- `CliOutcome` is `#[non_exhaustive]` — add variants, never renumber
  codes (`docs/EXIT_CODES.md` + `--help` document them).
- `PersistenceFailed` → 1 (`GENERAL_ERROR`) deliberately shares the
  general code for backward compat; `USAGE_ERROR` (2) comes from Clap,
  never from command code; `UNSAFE_REPAIRS` (10) has no variant.
- `ExecutionFailed` propagates the child code when known (0–255),
  else 8 (spawn failure, signal kill, `SNP_COMMAND_TIMEOUT`).
- Every exit code 0–10 has a defined producer. Code 2 belongs to Clap, code
  10 belongs to `exit_on_repair_status`, and codes 0/3–9 belong to
  `CliOutcome::exit_code()`. Code 1 has three producers —
  `CliOutcome::PersistenceFailed`, the `Err(SnipError)` boundary in
  `main()`, and `RepairExitStatus::PartialFailure`.
- Never leak secrets through `diagnostic()` or stdout payloads; machine
  output must stay pipe-safe (no prompts, no ANSI, exact bytes).

## Gotchas


- **Code 10 is unreachable through `CliOutcome`.** Grepping `src/outcome.rs`
  for `UNSAFE_REPAIRS` finds only the constant — never a variant arm.
- **Code 7 (`SyncFailed`) is currently unreachable.** `CliOutcome::SyncFailed`
  is defined and maps to 7, but no command constructs it: `main.rs` maps
  every `Err` returned by a command to `GENERAL_ERROR` (1). The code is
  **reserved** as part of the stable exit-code contract
  (`docs/EXIT_CODES.md`) — do not "clean it up" by renumbering, and do not
  assume a sync failure currently exits 7.
- **`PersistenceFailed` and `GENERAL_ERROR` are the same code (1).** Do not
  use them to distinguish two different failure families.
- **`Exit code 2` is Clap-only.** A command that validates its own arguments
  should return `ValidationFailed` (6), not try to produce 2.
- **`ExecutionFailed { child_code: None }` is 8**, but the `run` paths bypass
  `CliOutcome` and exit directly with the child's code
  (`src/main.rs:545`, `src/main.rs:569`). `snp clip` never propagates a
  child's code — it exits 1 on error.
- **JSON output has no shared envelope.** `list --json` is a bare array;
  `get --json` is a bare object; `repair --json` is an object with an
  `exit_status` field. Consumers must not assume a wrapper key.
- **Some optional JSON fields are omitted rather than `null`.** Check the
  serde attribute before assuming a key exists.

## Key Files


- `src/outcome.rs` — `CliOutcome` (`:16-40`), `exit_code::*` 0–10 (`:46-70`), `exit_code()` mapper (`:72-91`), `ColorPolicy` / `OutputMode` / `OutputContext` (`:95-149`), guards (`:151-272`), tests (`:274-376`)
- `src/main.rs` — the outcome → process boundary (`:1018-1033`) and the exit-10 path (`:375-388`)
- `src/commands/repair_cmd.rs:1007-1014` — the `repair --json` `exit_status` mapping
- `src/commands/get_cmd.rs:66-77` — the versioned `get --json` object
- `src/commands/list_cmd.rs:150-163` — the bare-array `list --json` shape
- `docs/EXIT_CODES.md` — the stable code table and child-code rules
- `docs/CLI_EXITCODE_STREAM_POLICY.md` — per-command streams, implemented vs aspirational
- `docs/JSON_SCHEMAS.md` — authoritative per-command JSON field schemas

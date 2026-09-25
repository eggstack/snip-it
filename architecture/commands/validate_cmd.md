# validate_cmd — Read-Only Data Validation

[← Back to Overview](../overview.md)

## Overview

`src/commands/validate_cmd.rs` (1234 lines) validates every library,
the `libraries.toml` index cross-references, the usage index, and file
permissions — without migrating, creating, or modifying anything.
Reports carry machine-stable codes with `Severity` × `Repairability`
and render as human text or JSON. The read-only counterpart to
`repair_cmd`.

## CLI surface

`ValidateArgs` (`validate_cmd.rs:18`), `snp validate` (alias `val`)
and `snp data validate` (alias `v`) sharing `handle_validate`
(`main.rs:405`): `-l/--library NAME` (default: all libraries),
`--strict` (elevate designated warnings), `--json`.

Core types: `Severity{Info,Warning,Error}` (`:32`),
`Repairability{Auto,Manual,Unrepairable}` (`:40`),
`ValidationDiagnostic{code,severity,path,library,snippet_id,message,
repairability}` (`:48`), `ValidationReport{schema_version:"1.0.0",
tool_version,strict_mode,dry_run:true,totals,diagnostics}` (`:60`).

## Flow / steps

`run(library, strict, json)` (`:670`):

1. `LibraryManager::new` (no `ensure_library_mode` writes) +
   `ValidationReport::new(strict)`.
2. Resolve sources via `readonly_library_sources`: named library
   (unknown name re-mapped to the historical `library_not_found`
   detail) or `"all"`.
3. Per library, `validate_library` (`:133`): file-read check
   (`E-FILE-READ`), empty-file info (`I-FILE-EMPTY`), raw TOML parse
   (`E-TOML-PARSE`), duplicate IDs (`E-DUP-ID`), empty IDs
   (`W-ID-EMPTY`), empty commands (`E-CMD-EMPTY`), empty descriptions
   (`W-DESC-EMPTY`, strict-sensitive), same-ID divergent content
   (`W-SAME-ID-DIVERGENT`), exact description+command duplicates
   (`W-EXACT-DUP`), stale `.corrupt.bak` presence.
4. `validate_index` (h–j): duplicate index rows, dangling paths,
   missing primary; `validate_usage` (k): orphaned usage entries;
   `validate_permissions` (l): Unix mode expectations.
5. Strict elevation: `W-ID-EMPTY, W-DESC-EMPTY, W-SAME-ID-DIVERGENT,
   W-EXACT-DUP` warnings become errors.
6. Emit human (`emit_human`) or pretty JSON; `has_errors() →
   ValidationFailed`, else `Success`.

Raw content is re-read alongside `load_library` so structural issues
the loader silently fixes are still reported.

## Mutation vs read-only

Strictly read-only. Uses the canonical read-only resolver, never
migrates legacy checkouts, never creates files, takes no locks and no
transaction gate (there is nothing to serialize — no writes occur).

## Auto-sync trigger

None. No `notify_mutation`, no runtime, no sync. Validation never
creates pending intent.

## Error / exit mapping

- Errors found → `CliOutcome::ValidationFailed` (exit 6,
  `docs/EXIT_CODES.md`); clean → `Success` (exit 0).
- Unknown `--library` name → `library_not_found` error (exit 3 family).
- Report serialization failure → `runtime_error` (exit 1).
- JSON and human reports carry identical diagnostics; `strict_mode`
  and `dry_run:true` are echoed in the JSON envelope.

## Key invariants

- Validation is safe to run on corrupt state — it must never make
  state worse (fail-closed reads, no backups written, no repairs).
- Codes are stable contracts: tests and `repair_cmd` key off them
  (`E-DUP-ID` ↔ `RepairSnippetIds`, orphaned usage ↔
  `PruneOrphanedUsage`).
- `truncate_desc`/`truncate_cmd` keep human output one-line and
  bounded (`:650-664`).
- Counts in tests assert exact numbers, never `>= 1`.

## File / line references

- `ValidateArgs`: `src/commands/validate_cmd.rs:18`; report types: `:32-109`
- `validate_library`: `:133`; `run`: `:670`; strict codes: `:707-719`
- Handler: `src/main.rs:405`; tests: `validate_cmd.rs:739+`

# CLI Outcome Types

[← Back to CLI](cli.md)

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
SelectionOutcome (lib layer)
    ↓
CommandOutcome (command layer)
    ↓
CliOutcome (public exit code layer)
```

- `SnippetSelection` is the TUI-level selection result (Selected, Cancelled, etc.)
- `SelectionOutcome` wraps snippet data with selection context
- `CommandOutcome` captures the result of a command operation
- `CliOutcome` is the final typed outcome for exit-code mapping

Commands convert their internal outcomes to `CliOutcome` before returning.
This ensures all commands share the same exit code semantics.

## Non-Exhaustive

`CliOutcome` is marked `#[non_exhaustive]`, allowing future variant additions
without breaking downstream callers.

## Tests

Unit tests in `src/outcome.rs` verify:

- All variants map to their documented exit codes
- `ExecutionFailed` with a child code propagates that code
- `PersistenceFailed` maps to general error (1)
- All exit codes are distinct (no collisions)

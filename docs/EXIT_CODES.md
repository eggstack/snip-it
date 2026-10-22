# Exit Codes

> **Evergreen contract.** Stable public CLI exit codes for `snp`.
> Implemented in `src/outcome.rs`; the `snp --help` after_help mirrors this table.

---

## Stable Exit Codes

| Code | Constant | Name | Meaning |
|------|----------|------|---------|
| 0 | `SUCCESS` | Success | Operation completed successfully |
| 1 | `GENERAL_ERROR` | General error | Unclassified operational failure |
| 2 | `USAGE_ERROR` | Usage error | CLI usage or argument error (Clap-enforced, or a Rust-side `SnipError::UsageError`) |
| 3 | `NOT_FOUND` | Not found | Snippet or resource does not exist |
| 4 | `CANCELLED` | Cancelled | User cancelled an interactive action |
| 5 | `AMBIGUOUS` | Ambiguous | Multiple matches found, unique match expected |
| 6 | `VALIDATION_FAILED` | Validation failure | Data validation failure |
| 7 | `SYNC_FAILED` | Sync failure | Synchronization with remote server failed |
| 8 | `EXECUTION_FAILED` | Execution failure | Snippet execution (child process) failed wrapper |
| 9 | `CONFLICT_OR_REFUSED` | Conflict/refused | Destructive action refused or generation changed |
| 10 | `UNSAFE_REPAIRS` | Unsafe repairs pending | Valid invocation found repairs, but every one of them needs an operator decision and was not applied (not a usage error) |

## `CliOutcome` Enum

The `CliOutcome` enum (`src/outcome.rs:18-42`) is the canonical mapping type. Every variant maps to one stable exit code, with two deliberate exceptions: `PersistenceFailed` shares code 1 with `GENERAL_ERROR`, and `ExecutionFailed { child_code: Some(n) }` propagates the child's code verbatim instead of a fixed code.

```rust
#[non_exhaustive]
pub enum CliOutcome {
    Success,          // → 0
    NotFound,         // → 3
    Ambiguous,        // → 5
    Cancelled,        // → 4
    ValidationFailed, // → 6
    PersistenceFailed,// → 1 (maps to GENERAL_ERROR)
    SyncFailed,       // → 7
    ExecutionFailed { child_code: Option<i32> }, // → child_code or 8
    ConflictOrRefused,// → 9
}
```

`#[non_exhaustive]` means downstream crates cannot match the enum exhaustively, so a new variant is not a breaking change for them.

### Variant-to-Code Mapping

| `CliOutcome` variant | Exit code | Notes |
|----------------------|-----------|-------|
| `Success` | 0 | |
| `NotFound` | 3 | |
| `Ambiguous` | 5 | |
| `Cancelled` | 4 | `select` returns `CliOutcome::Cancelled` (`src/commands/select_cmd.rs:110`); `main.rs` turns it into exit 4 via `outcome.exit_code()` |
| `ValidationFailed` | 6 | |
| `PersistenceFailed` | 1 | Shares code with `GENERAL_ERROR` |
| `SyncFailed` | 7 | **Reserved.** No command constructs this variant today — `main.rs` maps every command `Err` to `GENERAL_ERROR` (1), except `SnipError::UsageError`, which maps to `USAGE_ERROR` (2). The code remains part of the stable contract. |
| `ExecutionFailed { child_code: None }` | 8 | Wrapper code when child exit is unknown |
| `ExecutionFailed { child_code: Some(n) }` | n | Propagates child process exit code |
| `ConflictOrRefused` | 9 | |

## Internal Worker/Executor Codes

The detached auto-sync worker's exit status is internal and is not part of the
public CLI contract. `snp auto-sync-worker` exits 1 (`GENERAL_ERROR`) when the
helper cycle fails, and exits 0 for successful and no-work outcomes
(`src/main.rs:895-904`).

Sync failure classification is recorded in `auto-sync-status.toml` by the helper
(`src/auto_sync/status.rs:17`); there is no executor exit-code translation layer.

## Execution Failure Exit Code

The `ExecutionFailed` variant in `CliOutcome` has two sub-cases:

| Scenario | Exit code |
|----------|-----------|
| Child process exit code available | Propagated directly (0–255) |
| Child exit code unknown (spawn failure, signal kill, timeout) | 8 (`EXECUTION_FAILED`) |

This ensures scripts can distinguish between `snp` infrastructure failures (exit 1) and snippet execution failures (exit = child code or 8). Execution failures do not record usage metadata.

### Unified execution helper

`spawn_and_wait_execution(shell, command, timeout, stdout_file)` in `src/commands/run_cmd.rs:123` is the unified execution helper used by both the output-file and ordinary execution branches. It returns `ProcessResult::Failed { exit_code: None }` for spawn failures, timeouts, and signal kills, which maps to exit code 8. This eliminates the previous inconsistency where the output-file branch could return `SnipError` through `?` and reach exit code 1.

## Child Exit Code Propagation

When `snp run` or exact `run` executes a child snippet process:

1. If the child exits with a valid exit code (0–255), that code is propagated directly as the CLI exit code.
2. If the child is killed by a signal (Unix), exit code 8 (`EXECUTION_FAILED`) is used.
3. If the child process cannot be spawned, exit code 8 is used.
4. If the child times out (via `SNP_COMMAND_TIMEOUT`), exit code 8 is used.
5. Successful execution (child exit 0) records usage metadata; failed execution does not.

This ensures scripts can distinguish between `snp` infrastructure failures (exit 1) and snippet execution failures (exit = child code or 8).

## Exit-Code Mapping in `main.rs`

The central exit-code mapper in `src/main.rs:1055-1075` handles two paths:

1. **`CliOutcome`** path: `Ok(Success)` → exit 0, `Ok(Cancelled)` → exit 4 via `outcome.exit_code()`, other outcomes → their stable codes, `Err(e)` → exit 1, except `Err(SnipError::UsageError { .. })` → exit 2.
2. **`repair`** path: `RepairExitStatus::UnsafeOnly` → exit 10 and `PartialFailure` → exit 1 via `exit_on_repair_status` (`src/main.rs:401-414`) — there is **no** `CliOutcome` variant for exit 10, so it cannot be produced by the mapper above. `Clean`, `Repaired`, `FindingsSafe`, and `DryRun` all exit 0.

   `FindingsSafe` means the report-only invocation (no `--apply`, no
   `--dry-run`) found only `[auto]`-repairable issues. Those are *not* unsafe
   repairs pending an operator decision, so they do not claim exit 10 — a
   script can tell "nothing is blocked" from "a human must act".

## Usage Errors

`SnipError::UsageError` is the Rust-side counterpart of Clap's own argument
conflicts. It is raised by validations the Clap schema cannot express
(mutually exclusive output modes, a missing selector, a missing required
option) and maps to the same exit code 2 that Clap produces, so the exit
status tells "bad invocation" apart from "operation failed" regardless of
which layer rejected the invocation.

```rust
match dispatch_command(cli.command) {
    Ok(CliOutcome::Success) => {}
    Ok(outcome) => {
        std::process::exit(outcome.exit_code());
    }
    Err(e) => {
        eprintln!("error: {e}");
        std::process::exit(match e {
            SnipError::UsageError { .. } => exit_code::USAGE_ERROR,
            _ => exit_code::GENERAL_ERROR,
        });
    }
}
```

The real mapper interleaves `log_shutdown_info()` calls guarded by
`behavior.services != StartupServices::Minimal`; they do not affect the exit
code.

## Backward Compatibility

- Exit codes 0 and 1 remain unchanged from pre-08A behavior.
- Exit codes 2-9 are additive — scripts checking `exit != 0` continue to work.
- `PersistenceFailed` maps to 1 (not a dedicated code) to avoid breaking existing scripts. **Code 6 is validation only** — a persistence failure is exit 1, not 6.
- `ExecutionFailed` propagates the child's exit code when available, preserving backward compatibility for scripts checking the child's exit code directly.
- Exit code 7 (`SYNC_FAILED`) is **reserved but currently unreachable**: no command constructs `CliOutcome::SyncFailed`, and every sync failure surfaces as exit 1.
- Exit code 10 (`UNSAFE_REPAIRS`) has **no `CliOutcome` variant**; it is produced only by `snp repair` through `exit_on_repair_status`.
- The process exit code is **not** confined to 0–10: `snp run` propagates the executed child's exit code verbatim.

## Shell Integration Example

```bash
snp select -f "deploy" > /tmp/cmd.sh
exit_code=$?
case $exit_code in
    0) echo "Selected successfully" ;;
    4) echo "Cancelled by user" ;;
    *) echo "Error (exit $exit_code)" ;;
esac
```

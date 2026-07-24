# Exit Codes

> Phase 10 — Workstream F extension (corrective closure)
> Stable public CLI exit codes for `snp`.

---

## Stable Exit Codes

| Code | Constant | Name | Meaning |
|------|----------|------|---------|
| 0 | `SUCCESS` | Success | Operation completed successfully |
| 1 | `GENERAL_ERROR` | General error | Unclassified operational failure |
| 2 | `USAGE_ERROR` | Usage error | CLI usage or argument error (Clap-controlled) |
| 3 | `NOT_FOUND` | Not found | Snippet or resource does not exist |
| 4 | `CANCELLED` | Cancelled | User cancelled an interactive action |
| 5 | `AMBIGUOUS` | Ambiguous | Multiple matches found, unique match expected |
| 6 | `VALIDATION_FAILED` | Validation failure | Data validation or local persistence failure |
| 7 | `SYNC_FAILED` | Sync failure | Synchronization with remote server failed |
| 8 | `EXECUTION_FAILED` | Execution failure | Snippet execution (child process) failed wrapper |
| 9 | `CONFLICT_OR_REFUSED` | Conflict/refused | Destructive action refused or generation changed |

## `CliOutcome` Enum

The `CliOutcome` enum (`src/outcome.rs`) is the canonical mapping type. Each variant maps to exactly one stable exit code.

```rust
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

### Variant-to-Code Mapping

| `CliOutcome` variant | Exit code | Notes |
|----------------------|-----------|-------|
| `Success` | 0 | |
| `NotFound` | 3 | |
| `Ambiguous` | 5 | |
| `Cancelled` | 4 | Also mapped in `main.rs` for `CommandOutcome::Cancelled` |
| `ValidationFailed` | 6 | |
| `PersistenceFailed` | 1 | Shares code with `GENERAL_ERROR` |
| `SyncFailed` | 7 | |
| `ExecutionFailed { child_code: None }` | 8 | Wrapper code when child exit is unknown |
| `ExecutionFailed { child_code: Some(n) }` | n | Propagates child process exit code |
| `ConflictOrRefused` | 9 | |

## Internal Worker/Executor Codes

Exit codes used by the auto-sync worker (`snp auto-sync-worker`) and executor (`snp auto-sync-execute`) subprocesses are internal. They are not part of the public CLI contract and may change without notice.

Worker/executor codes are defined in `src/auto_sync/executor.rs` as `ExecutorExitCode` and mapped by the worker's exit-code translation layer.

## Execution Failure Exit Code

The `ExecutionFailed` variant in `CliOutcome` has two sub-cases:

| Scenario | Exit code |
|----------|-----------|
| Child process exit code available | Propagated directly (0–255) |
| Child exit code unknown (spawn failure, signal kill, timeout) | 8 (`EXECUTION_FAILED`) |

This ensures scripts can distinguish between `snp` infrastructure failures (exit 1) and snippet execution failures (exit = child code or 8). Execution failures do not record usage metadata.

### Phase 11C: Unified Execution Helper

`spawn_and_wait_execution(shell, command, timeout, stdout_file)` in `src/commands/run_cmd.rs` is the unified execution helper used by both the output-file and ordinary execution branches. It returns `ProcessResult::Failed { exit_code: None }` for spawn failures, timeouts, and signal kills, which maps to exit code 8. This eliminates the previous inconsistency where the output-file branch could return `SnipError` through `?` and reach exit code 1.

## Child Exit Code Propagation

When `snp run` or exact `run` executes a child snippet process:

1. If the child exits with a valid exit code (0–255), that code is propagated directly as the CLI exit code.
2. If the child is killed by a signal (Unix), exit code 8 (`EXECUTION_FAILED`) is used.
3. If the child process cannot be spawned, exit code 8 is used.
4. If the child times out (via `SNP_COMMAND_TIMEOUT`), exit code 8 is used.
5. Successful execution (child exit 0) records usage metadata; failed execution does not.

This ensures scripts can distinguish between `snp` infrastructure failures (exit 1) and snippet execution failures (exit = child code or 8).

## Exit-Code Mapping in `main.rs`

The central exit-code mapper in `src/main.rs` (lines 1207-1218) handles two paths:

1. **`CommandOutcome`** path: `Ok(Success)` → exit 0, `Ok(Cancelled)` → exit 4, `Err(e)` → exit 1.
2. **`CliOutcome`** path: Commands that return `CliOutcome` directly call `outcome.exit_code()` and pass it to `std::process::exit()`.

```rust
match dispatch_command(cli.command) {
    Ok(CommandOutcome::Success) => {}
    Ok(CommandOutcome::Cancelled) => {
        std::process::exit(4);
    }
    Ok(_) => {}
    Err(e) => {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
```

## Backward Compatibility

- Exit codes 0 and 1 remain unchanged from pre-08A behavior.
- Exit codes 2-9 are additive — scripts checking `exit != 0` continue to work.
- `PersistenceFailed` maps to 1 (not a dedicated code) to avoid breaking existing scripts.
- `ExecutionFailed` propagates the child's exit code when available, preserving backward compatibility for scripts checking the child's exit code directly.

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

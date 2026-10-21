# CLI & Outcome Contract Skill

## Purpose
Guide agents through the public CLI surface of `snp`: argument definitions,
dispatch, startup-recovery classification, stream discipline, and the
`CliOutcome` → exit-code contract.

This is the surface every user script depends on. Breaking it is a **release-level
compatibility event**, and nothing in the compiler tells you — only
`tests/cli_surface_compat.rs`, `tests/execution_outcomes.rs`, and
`docs/COMPATIBILITY.md` will.

Contract of record: [`../docs/EXIT_CODES.md`](../docs/EXIT_CODES.md),
[`../docs/COMMAND_CONTRACTS.md`](../docs/COMMAND_CONTRACTS.md),
[`../docs/CLI_EXITCODE_STREAM_POLICY.md`](../docs/CLI_EXITCODE_STREAM_POLICY.md).
Deep dive: [`../architecture/cli.md`](../architecture/cli.md),
[`../architecture/outcome.md`](../architecture/outcome.md).

---

## The dispatch chain

```
clap parse → Commands (src/main.rs:91)
            → command_behavior(cmd)          (src/main.rs:924)  startup recovery policy
            → dispatch_command(cli)          (src/main.rs:470)  returns SnipResult<CliOutcome>
            → CliOutcome::exit_code()        (src/outcome.rs:76) → stable exit code
```

`dispatch_command` returns `SnipResult<CliOutcome>`. **`Ok(...)` carries the
outcome; `Err(...)` is an unclassified operational failure and maps to exit 1.**
A `CliOutcome` becomes a process exit in exactly one place
(`src/main.rs:1061`) — do not call `std::process::exit` from a command
module.

### Exit codes are a stable public contract

`exit_code` constants live in `src/outcome.rs:48`. Codes 0 and 1 are
pre-existing and frozen; 2–10 are additive and documented in
`docs/EXIT_CODES.md`. Three rules that are easy to get wrong:

1. **Validation findings are exit 6, not 2.** Exit 2 is Clap's usage error. A
   command that parsed fine but found bad data exits 6.
2. **`PersistenceFailed` maps to 1**, not to a dedicated code — deliberately, so
   pre-existing scripts checking `exit 1` keep working.
3. **`ExecutionFailed` propagates the child's exit code** when one exists, and
   falls back to 8 only for spawn failure, signal kill, or timeout. Use the
   unified helper `spawn_and_wait_execution()` in
   `src/commands/run_cmd.rs:123` — it returns
   `ProcessResult::Failed { exit_code: None }` for exactly those
   three cases. Do not let an output-file branch return `SnipError` through `?`
   and reach exit 1 instead of 8.

### `repair` is the one command outside `CliOutcome`

`snp repair` uses `RepairExitStatus` (`src/commands/repair_cmd.rs:146`) and
`exit_on_repair_status()` (`src/main.rs:401`): `UnsafeOnly` → exit 10,
`PartialFailure` → exit 1. Exit 10 means "the invocation was valid, but every
repair found was unsafe" — it is **not** a usage error.

---

## Startup recovery classification — one table, two axes

`command_behavior()` (`src/main.rs:924`) returns a `CommandBehavior` holding
**both** a `StartupRecoveryPolicy` and a `StartupServices` level
(`src/main.rs:911`) in a single `match`, so the two cannot drift.

| Policy | Commands |
|--------|----------|
| `SuppressReadOnly` (+ `StartupServices::Minimal`) | `version`, `list`, `select`, `status`, `mcp`, `get`, `validate`, `backup`, `library list`, `library show` |
| `SuppressReadOnly` (dry-run only) | `restore --mode dry-run`, `repair --dry-run`, `import pet --dry-run` — plus `data restore --mode dry-run` and `data repair --dry-run` |
| `Allow` (full logging + audit) | `new`, `run`, `clip`, `search`, `edit`, `import`, `repair`, `restore`, `premade`, `library create`, `library delete`, `library set-primary`; **and no subcommand** (default TUI) |
| `SuppressExplicitSync` | `sync`, `cron`, `register` |
| `SuppressInternal` | `auto-sync-worker` (internal subprocess) |
| `SuppressConfiguration` | `update`, `self-replace`, `doctor`, `completions`, `shell`, `keybindings` |

Two easy-to-miss entries:

- **`run`, `clip`, and `search` are `Allow`**, because they can record usage
  metadata and schedule auto-sync.
- **`mcp` is `SuppressReadOnly`**, because the MCP surface never mutates.

**Never infer the policy from the command's name.** The dry-run *variants*
are the one flag-sensitive exception: `restore --mode dry-run`,
`repair --dry-run`, and `import pet --dry-run` are matched **before** the
generic arms and get `SuppressReadOnly` + `StartupServices::Minimal`
(`src/main.rs:945-962`); only their non-dry-run spellings fall through to
`Allow`.

Adding a command means editing `command_behavior()`. Adding a *subcommand* to an
existing command means checking whether the parent already covers it —
`library list` / `library show` are read-only while `library create` /
`library delete` / `library set-primary` are not, and they live in one command.

---

## Stream discipline

**stdout carries data; stderr carries diagnostics.** A script must be able to
pipe stdout without filtering ANSI escapes or interleaved prose.

- Machine modes are `--json`, `--csv`, `--raw`, `--field`. `OutputContext`
  (`src/outcome.rs:144`) is the guard: `is_machine_mode()` and
  `suppress_ansi()` decide, and `write_stdout()` swallows `BrokenPipe` so
  `snp list | head` does not spew an error.
- `snp list` default output is a **colored table with ANSI escapes** — not
  pipe-safe without `--json` or `--csv`.
- `snp mcp serve` is the extreme case: stdout is **protocol-only**. Any
  diagnostic on stdout corrupts the JSON-RPC stream.
- `snp edit --output` writes the output field to **stderr**, not stdout
  (`src/commands/edit_cmd.rs:218,220,284,286`), matching the `edit` row of
  `docs/COMMAND_CONTRACTS.md` ("Writes stdout: No").
- TUI commands (`run`, `clip`, `search`, `select`) render directly to the
  terminal via crossterm raw mode and bypass stdout/stderr for the
  interactive portion.

If you add a command that writes to stdout, pick the discipline deliberately
and record it in `docs/COMMAND_CONTRACTS.md` — the table is the contract, not
a description.

---

## `snp data` is a mirror, not a second implementation

`snp data <subcommand>` (`DataCommands`, `src/main.rs:281`) is a **mirror
spelling** of `validate`, `status`, `backup`, `restore`, and `repair`. It has
no independent behavior:

- It dispatches to the same handlers (`src/main.rs:862-878`).
- It shares the same argument schema and the same startup-recovery
  classification.
- `tests/cli_surface_compat.rs` asserts both spellings succeed with the same
  exit codes and equivalent machine-readable output.

If you change one spelling's behavior, you change both — and the compat test
will tell you if you did not.

Adding a new `snp data <x>` spelling takes **three edits** in `src/main.rs`: a
`DataCommands` variant, a dispatch arm, and a `command_behavior()` arm. Those
five data spellings are the only commands with single-path `handle_*`
functions in `main.rs`; the other ~20 commands dispatch inline inside
`dispatch_command`.

---

## Compatibility rules for changing the CLI

1. **Removing a flag is breaking.** Users have scripts. Deprecate first
   (warn, keep the flag working), then remove in a major release. See
   `docs/COMPATIBILITY.md`.
2. **Reusing an exit code for a new meaning is breaking**, even if the number
   is unchanged. Exit codes are a published interface.
3. **Adding a flag is safe**; changing a default is only safe if the new
   default preserves the old behavior for existing scripts.
4. **Changing the stdout format of a default (non-machine) mode** is a
   soft break — acceptable, but update `docs/COMMAND_CONTRACTS.md` and
   `docs/USER_GUIDE.md`.
5. **Machine modes (`--json`, `--csv`) are versioned contracts.** Field
   additions are fine; renames and removals are not. `docs/JSON_SCHEMAS.md`
   records the envelopes and `tests/schema.rs` enforces them.

---

## Tests that protect this

| Target | Protects |
|--------|----------|
| `tests/cli_surface_compat.rs` | `snp data` mirror equivalence, same exit codes |
| `tests/execution_outcomes.rs` | child exit-code propagation, usage recording |
| `tests/schema.rs` | JSON envelope shape |
| `tests/tui_requires_terminal.rs` | TUI commands fail cleanly without a terminal |
| `src/outcome.rs` unit tests | `CliOutcome` → exit-code mapping |

```bash
cargo test --test cli_surface_compat
cargo test --test execution_outcomes
cargo test --test schema
```

---

## Common mistakes

- Calling `std::process::exit()` from a command module instead of returning a
  `CliOutcome`.
- Using exit 2 for a validation finding (that is exit 6).
- Letting an output-file execution branch return `SnipError` through `?`, so a
  child failure exits 1 instead of 8.
- Adding a command and forgetting `command_behavior()` — it silently gets the
  wrong startup recovery policy.
- Assuming the command name implies the recovery policy (`run` is `Allow`,
  `mcp` is `SuppressReadOnly` — both counter-intuitive).
- Writing a human-readable progress line to stdout in a machine-output mode.
- Adding a `snp data` spelling that does not route to the same handler.
- Removing a flag without a deprecation period.

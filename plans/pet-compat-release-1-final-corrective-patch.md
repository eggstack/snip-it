# Release 1 Final Corrective Patch: Explicit Primary-Selector Cancellation and PTY Proof

## Purpose

Close the final known Release 1 correctness gap before beginning Release 2.

The current implementation has already completed the broad Release 1 work:

- Release 1A established the Pet compatibility contract, regression fixtures, stream policy, and architecture inventory.
- Release 1B added the non-executing `snp select` command with raw and expanded output modes.
- Release 1C added generated Bash, Zsh, and Fish shell-buffer integrations.
- The Release 1 closure pass introduced `CommandOutcome`, moved exit-code mapping to the CLI boundary, removed the ignored `--sync` flag, hardened shell error handling, added behavioral shell tests, and rejected symlink/directory output paths.

One contract defect remains: cancellation from the primary snippet-selection TUI is still not represented explicitly. `select_cmd` can detect cancellation from the variable prompt, but ordinary selector cancellation appears to return with no selected command and no cancellation flag, causing `snp select` to return success instead of exit code 4.

This patch must be narrowly corrective. Do not redesign the selector, change existing `run`, `clip`, or `search` cancellation semantics, add Release 2 functionality, or broaden the public CLI beyond what is necessary to close the documented Release 1 contract.

This document is intended for implementation-agent handoff. Inspect the current repository state before editing; names and signatures below describe the required behavior, not immutable file-layout assumptions.

## Required Outcome

After this patch:

1. `snp select` returns exit code 4 when the primary selector is cancelled with `q`, `Esc`, or the supported interrupt path.
2. `snp select --expanded` also returns exit code 4 when variable prompting is cancelled.
3. Successful selection returns exit code 0 and emits exactly the selected command to stdout or the requested output file.
4. Operational failures return exit code 1 and remain distinguishable from cancellation.
5. Existing `snp run`, `snp clip`, and `snp search` continue treating ordinary TUI cancellation exactly as they do today.
6. PTY-backed tests prove the primary-selector contract end to end.
7. Documentation matches tested behavior.

## Current Defect

The current `select_cmd` flow keeps two callback-local cells:

```rust
let cancelled = Cell::new(false);
let selected_command = Cell::new(None);
```

The cancellation cell is set only when variable expansion returns `ExpandedCommand::Cancel`. If the user exits the main selector before choosing a snippet, the selection callback is never invoked. The command therefore reaches the end with:

```text
cancelled == false
selected_command == None
```

and returns `CommandOutcome::Success`.

Do not fix this by inferring cancellation from `selected_command.is_none()` inside `select_cmd`. The shared selector can also finish without a selected command for reasons that should remain explicit, and inference would preserve ambiguity rather than close it.

The shared selection layer must return a semantic outcome.

## Design Constraint: Preserve Existing Command Semantics

The selector is reused by `run`, `clip`, `search`, and `select`. Existing commands intentionally treat cancellation as a normal completion state. This patch must not globally map all TUI cancellation to exit code 4.

The required layering is:

```text
TUI event loop
  -> shared selection runner reports Selected / Cancelled / other explicit result
  -> each command interprets that result according to its existing contract
```

For `run`, `clip`, and `search`:

```text
Cancelled -> successful command completion, as today
```

For `select`:

```text
Cancelled -> CommandOutcome::Cancelled -> exit code 4 at main.rs
```

## Workstream A: Introduce an Explicit Shared Selection Outcome

### A1. Define the outcome type at the correct layer

Add a compact internal enum near the shared selector orchestration code, for example:

```rust
pub(crate) enum SelectionOutcome {
    Selected,
    Cancelled,
}
```

If the existing architecture naturally needs richer variants, a slightly broader type is acceptable:

```rust
pub(crate) enum SelectionOutcome {
    Selected,
    Cancelled,
    DeletedAndContinued,
}
```

Do not expose UI-internal indexes or command-specific data unless required. The callback already handles selected-snippet processing; the shared return value only needs to report how the interaction terminated.

The outcome type should not be the public `CommandOutcome`. `SelectionOutcome` is an internal TUI/orchestration result; `CommandOutcome` remains the top-level CLI semantic result.

### A2. Make the TUI selection API distinguish cancellation

Inspect the existing `SnippetSelection` type and event-loop return paths. The likely current variants include selected and delete behavior, while cancellation is represented by `None`, `ProcessResult::Cancel`, or an ordinary return.

Change the lowest reasonable layer so cancellation is explicit. Acceptable shapes include:

```rust
pub enum SnippetSelection {
    Selected(usize, bool),
    Delete(usize),
    Cancelled,
}
```

or:

```rust
pub enum SnippetSelection {
    Selected(usize, bool),
    Delete(usize),
}

pub enum SelectorExit {
    Selection(SnippetSelection),
    Cancelled,
}
```

Prefer the smallest change that makes cancellation impossible to confuse with successful no-selection completion.

All cancellation paths must converge on the explicit variant:

- `q` in normal mode;
- `Esc` in the selector mode where it means exit;
- Ctrl-C or the supported signal/interrupt path;
- any existing dedicated quit key.

Do not reinterpret keys in insert/filter mode. Preserve all current keybindings and modal behavior.

### A3. Refactor `run_snippet_selection`

Change the shared runner from an implicit `SnipResult<()>` contract to an explicit result, for example:

```rust
pub(crate) fn run_snippet_selection<F>(...) -> SnipResult<SelectionOutcome>
```

Required behavior:

- Return `SelectionOutcome::Cancelled` when the primary selector exits without selection through a user cancellation path.
- Return `SelectionOutcome::Selected` after the callback accepts and completes a selected snippet.
- Preserve deletion loops and any continue behavior exactly as today.
- Preserve sync, library loading, filtering, and TUI terminal cleanup.
- Do not duplicate selector loops in `select_cmd`.

If callback results such as `ProcessResult::Continue`, `Cancel`, or `Done` currently affect the loop, document their relationship to the new outcome. In particular, distinguish:

```text
primary-selector cancellation
```

from:

```text
callback/variable-prompt cancellation
```

Both eventually mean cancellation for `snp select`, but they originate at different layers.

### A4. Update existing command callers conservatively

Update all call sites to handle the new return value.

For `run`, `clip`, and `search`, preserve current behavior by ignoring or explicitly accepting `SelectionOutcome::Cancelled` and returning `Ok(())`.

Representative intent:

```rust
match run_snippet_selection(...)? {
    SelectionOutcome::Selected | SelectionOutcome::Cancelled => Ok(()),
}
```

Do not change their documented exit codes, stdout behavior, sync behavior, audit logging, or terminal flow.

## Workstream B: Correct `snp select` Outcome Mapping

### B1. Map primary-selector cancellation directly

In `select_cmd`, capture the shared selection outcome:

```rust
let selection_outcome = run_snippet_selection(...)?;
```

Then map it explicitly:

```rust
if matches!(selection_outcome, SelectionOutcome::Cancelled) {
    cleanup_output_file_if_safe(...);
    return Ok(CommandOutcome::Cancelled);
}
```

Do not infer cancellation from the absence of selected output.

### B2. Preserve variable-prompt cancellation

Keep the existing expanded-mode cancellation path, but simplify it if the new architecture permits. The final command-level rule is:

```text
primary selector cancelled -> CommandOutcome::Cancelled
variable prompt cancelled -> CommandOutcome::Cancelled
```

Avoid two unrelated flags if a clearer command-local enum can represent the callback result.

For example:

```rust
enum SelectProcessingOutcome {
    Selected(String),
    Cancelled,
    Continue,
}
```

The implementation agent may retain the existing cell pattern if minimal, but the resulting control flow must be mechanically clear and testable.

### B3. Treat impossible no-output success as an error

Once the shared outcome is explicit, this state should be impossible:

```text
SelectionOutcome::Selected
selected_command == None
```

Do not silently return success in that case. Return a `SnipError::runtime_error` or equivalent internal contract error.

This protects the shell adapters from future regressions where exit code 0 is returned without an output file payload.

Representative intent:

```rust
match (selection_outcome, selected_command.take()) {
    (SelectionOutcome::Cancelled, _) => Ok(CommandOutcome::Cancelled),
    (SelectionOutcome::Selected, Some(command)) => write_or_print(command),
    (SelectionOutcome::Selected, None) => Err(internal_contract_error),
}
```

### B4. Preserve top-level exit mapping

Keep `main.rs` as the only process-level mapping point:

```rust
CommandOutcome::Success => exit 0
CommandOutcome::Cancelled => exit 4
SnipError => exit 1
```

Do not reintroduce `std::process::exit` inside `select_cmd`, the TUI, or shared command helpers.

## Workstream C: Output-File Cleanup Consistency

This patch is not a broad output transport redesign, but cancellation behavior must be consistent.

### C1. Centralize safe cancellation cleanup

Extract or reuse a helper for cancellation cleanup so both primary-selector and variable-prompt cancellation follow the same rule.

Required behavior:

- If no output file was requested, do nothing.
- If the path exists and is a regular non-symlink file, remove it.
- Do not follow or remove symlinks.
- Do not remove directories.
- Ignore cleanup failure on a cancellation path unless the project has an established safer reporting convention; cancellation must not become an overwrite or deletion primitive.

### C2. Preserve operational error distinction

Do not convert output-file validation or write errors into cancellation. They remain `SnipError` and exit code 1.

## Workstream D: PTY-Backed End-to-End Tests

Unit and stub-shell tests are not sufficient for this defect. Add true pseudo-terminal tests that exercise the real TUI and real binary.

### D1. Select a test harness

Use an existing development dependency if the repository already has PTY support. Otherwise add a narrowly scoped dev dependency suitable for cross-platform or Unix-gated PTY testing, such as a maintained `portable-pty`, `expectrl`, or equivalent crate.

Keep production dependencies unchanged.

If PTY tests are Unix-only, gate them with `#[cfg(unix)]` and retain non-PTY unit coverage on Windows. Document the limitation explicitly.

### D2. Build deterministic fixture setup

Each PTY test should:

1. create an isolated temporary `XDG_CONFIG_HOME`;
2. create a deterministic library containing one or two snippets;
3. set the primary library explicitly;
4. launch the actual `snp` binary under a PTY;
5. wait for a stable selector marker or bounded startup condition;
6. send key input;
7. collect exit status and output;
8. enforce a timeout so CI cannot hang.

Disable or control theme extraction, logging noise, and unrelated sync behavior through existing supported environment variables where necessary.

Do not assert full-screen ANSI snapshots unless the repository already has a robust normalization strategy. Assert semantic outcomes.

### D3. Required primary cancellation test

Add an end-to-end test that:

- launches `snp select`;
- sends the documented quit key, preferably `q` in normal mode;
- waits for process exit;
- asserts exit code 4;
- asserts no selected command is written to stdout;
- asserts the process exits within the timeout;
- verifies the terminal/PTY is not left in a hung state.

Add a second case for `Esc` if it follows a distinct event path.

### D4. Required successful selection test

Add an end-to-end test that:

- launches `snp select` against a one-snippet fixture;
- selects the snippet with Enter;
- asserts exit code 0;
- asserts stdout contains exactly the command plus the documented terminating newline;
- asserts no execution side effect occurs.

Use a command string that would create a marker file if executed, then assert that the marker file does not exist.

### D5. Required output-file selection test

Add a PTY test for:

```text
snp select --output-file <pre-created-temp-file>
```

Assert:

- exit code 0;
- stdout is empty;
- file contents exactly equal the selected command;
- multiline and trailing-newline policy matches documentation;
- no shell execution occurs.

### D6. Required expanded cancellation test

Add a PTY test that:

- selects a snippet containing a required variable using `--expanded`;
- cancels the variable prompt;
- asserts exit code 4;
- asserts stdout/output file remains empty or is safely removed;
- confirms this path is distinct from main-selector cancellation but maps to the same command outcome.

### D7. Operational failure control test

Retain or add a non-PTY integration test that malformed TOML, missing permissions, or another deterministic operational failure returns exit code 1 rather than 4.

The purpose is to keep the cancellation/error distinction locked.

## Workstream E: Unit and Regression Coverage

### E1. Shared outcome unit tests

Add direct tests for the shared runner or the smallest extractable mapping function:

- primary `Cancelled` maps to command cancellation for `select`;
- primary `Cancelled` remains normal success for existing commands;
- `Selected + Some(command)` succeeds;
- `Selected + None` returns an internal error;
- variable-prompt cancellation maps to cancellation;
- output-file cleanup refuses symlinks/directories.

### E2. Existing-command regression tests

Add or retain tests proving that cancellation semantics for existing commands are unchanged. At minimum, document and test the command-layer mapping even if driving every TUI command through a PTY would be excessive.

No existing command should begin returning exit code 4 because of this patch.

### E3. Shell adapter regression

Run the existing Bash, Zsh, and Fish behavioral tests unchanged. They should continue receiving exit code 4 for cancellation and nonzero operational errors for failures.

Do not modify generated shell API names or keybinding examples in this patch.

## Workstream F: Documentation Reconciliation

Update documentation only where implementation details change.

Required checks:

- `docs/CLI_EXITCODE_STREAM_POLICY.md` accurately describes both primary-selector and variable-prompt cancellation.
- `docs/PET_COMPATIBILITY.md` continues to state exit code 4 only for `snp select`, not for existing commands.
- `docs/ARCHITECTURE_INVENTORY.md` documents the new shared `SelectionOutcome` and its callers.
- `AGENTS.md` identifies the explicit selector outcome contract for future agents.
- README and USER_GUIDE require no user-facing change unless current wording is inaccurate.

Remove any wording that implies cancellation is inferred from empty output.

## Validation Matrix

Run the complete repository validation required by current contributor guidance.

At minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Also run targeted tests separately so failures are easy to diagnose:

```bash
cargo test --test integration select
cargo test shell_cmd
cargo test pty
```

Use the actual test names adopted by the implementation.

Validate on:

- Linux x86_64;
- macOS Apple Silicon where available;
- Windows for compilation and non-PTY regression tests.

PTY tests may be Unix-gated, but Windows must still compile cleanly with no unused imports, dead conditional code, or dependency issues.

## Acceptance Criteria

This corrective patch is complete only when all of the following are true:

1. The shared snippet-selection path returns an explicit cancellation outcome.
2. `snp select` maps primary-selector cancellation to `CommandOutcome::Cancelled`.
3. Main-selector `q` cancellation exits with status 4 in a real PTY test.
4. Main-selector `Esc` cancellation is tested if it uses a separate path.
5. Expanded variable-prompt cancellation exits with status 4.
6. Successful selection exits 0 and emits exactly one command without executing it.
7. `SelectionOutcome::Selected` without command output is treated as an internal error, not success.
8. Operational failures remain exit 1 and cannot be mistaken for cancellation.
9. `run`, `clip`, and `search` preserve their existing cancellation behavior.
10. Existing Bash, Zsh, and Fish behavioral tests continue to pass.
11. Full workspace tests, formatting, and Clippy pass.
12. Documentation matches the tested behavior.
13. No Release 2 functionality is introduced.

## Non-Goals

- Adding shell-history capture.
- Adding multiline snippet creation beyond existing behavior.
- Redesigning `CommandOutcome` into a broad exit-code taxonomy.
- Changing the default shell keybindings or generated function names.
- Replacing temp-file transport.
- Generalizing output-file handling beyond the minimal safety and cleanup requirements already established.
- Changing cancellation semantics for `run`, `clip`, or `search`.
- Refactoring the entire TUI event loop solely for stylistic reasons.

## Recommended Commit Shape

Prefer one focused implementation commit containing:

- explicit shared selection outcome;
- updated command call sites;
- corrected `select_cmd` mapping;
- PTY and regression tests;
- documentation reconciliation.

A two-commit sequence is acceptable if the first commit is pure internal outcome refactoring and the second contains PTY tests/documentation. Do not mix unrelated cleanup, dependency upgrades, or Release 2 work into this patch.

## Release 1 Closure Decision

After this patch passes the acceptance criteria, Release 1 can be considered closed:

```text
Release 1A: compatibility contract and baseline — complete
Release 1B: machine-facing selection primitive — complete
Release 1C: generated shell integration — complete
Release 1 closure/corrective work — complete
```

The next planned work may then begin with Release 2 acquisition ergonomics: shell-history capture and first-class multiline/stdin creation.
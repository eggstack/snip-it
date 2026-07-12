# Release 1 Closure Pass: Selection Contract, Shell Adapter Correctness, and End-to-End Validation

## Purpose

Close the remaining correctness, contract, and validation gaps in the implemented Pet compatibility Release 1 work before beginning Release 2.

Release 1A, 1B, and 1C are already substantially implemented:

- Release 1A established the Pet compatibility matrix, CLI stream/exit-code policy, architecture inventory, fixture corpus, and regression baseline.
- Release 1B added the non-executing `snp select` command with raw and expanded output modes.
- Release 1C added generated Bash, Zsh, and Fish integrations that insert selected snippets into the current shell buffer without executing them.

The current implementation has the correct product shape and should not be redesigned. This closure pass is narrowly corrective. It must preserve the existing additive feature surface while repairing cancellation propagation, shell error handling, exit mapping, output-file safety, and end-to-end validation.

This plan is intended for implementation-agent handoff. The implementing agent must inspect the repository at execution time and adapt file paths or internal APIs where the code has evolved. Do not start Release 2 work in this pass.

## Current Known Issues

The review of commits after the Release 1 plans identified the following closure items.

### 1. Selector cancellation is not represented explicitly

`select_cmd` currently records cancellation only when variable expansion returns `ExpandedCommand::Cancel`. Normal cancellation from the primary TUI appears to return without selecting a snippet, leaving both the cancellation flag and selected command unset. This can produce exit code 0 with no output instead of the documented exit code 4.

### 2. Exit code 4 is produced through deep `std::process::exit`

`select_cmd::run()` calls `std::process::exit(4)` directly. This bypasses normal stack unwinding, makes the cancellation path harder to test, and places process-level policy inside command implementation code.

### 3. Shell wrappers can misclassify operational failures as cancellation

The generated shell adapters check for an empty temporary file before checking a nonzero exit status. Any operational error that leaves the file empty can therefore be treated as a successful cancellation.

### 4. `snp select --sync` is accepted but ignored

The CLI exposes `--sync`, but the dispatch path discards it. A visible option must not silently do nothing.

### 5. `--output-file` is broader and sharper than required

The shell integration uses `--output-file` for lossless transport. The current implementation writes to an arbitrary supplied path with ordinary file-write semantics. This permits accidental overwrite and symlink following.

### 6. Shell tests are primarily generation and syntax tests

The current tests validate generated code and shell syntax, but do not yet fully prove real behavior across success, cancellation, operational failure, buffer restoration, cursor restoration, multiline transport, and cleanup.

### 7. Documentation contains minor contract drift

The documentation overstates cancellation reliability until the core outcome path is fixed, and at least one architecture note refers to `snp check` where the generated code actually uses `command -v snp`.

## Goals

1. Make selector cancellation an explicit outcome from the shared selection pipeline.
2. Map cancellation to exit code 4 only at the top-level CLI boundary.
3. Ensure successful selection, user cancellation, and operational failure are distinguishable in every shell adapter.
4. Remove or implement the currently ignored `--sync` option; prefer removal unless existing command semantics justify it.
5. Harden output-file transport against accidental overwrite and symlink abuse.
6. Add real shell-level and targeted PTY validation for Bash, Zsh, and Fish.
7. Verify no existing `run`, `clip`, `search`, library, variable, sync, or TUI behavior changes.
8. Reconcile documentation with the final implemented contract.
9. Finish Release 1 with a clean full test, formatting, lint, and platform validation pass.

## Non-Goals

- Do not begin Release 2 history capture work.
- Do not add multiline creation, stdin creation, or previous-command capture in this pass.
- Do not add Pet multiple-choice parameter syntax.
- Do not add import commands, migration diagnostics, ranking changes, output/notes redesign, or auto-sync hooks.
- Do not replace the native TUI with `fzf`, `peco`, or another external selector.
- Do not add automatic keybindings or edit shell startup files.
- Do not alter the existing default behavior of `snp run`, `snp clip`, `snp search`, or `snp new`.
- Do not change the canonical Pet-compatible TOML format.
- Do not broaden shell support beyond Bash, Zsh, and Fish in this pass.

## Required Product Invariants

The closure work must preserve these invariants:

- `snp select` never executes a snippet.
- Raw mode remains the default.
- Expanded mode uses the same variable parser and prompt UI as existing run/copy behavior.
- Generated shell integration inserts selected content into the current buffer and never executes it.
- No keybindings are installed by default.
- Selected snippet content is never passed through `eval`.
- Existing commands retain their current exit-code and stream behavior unless explicitly documented by the existing policy.
- User cancellation is not an operational error.
- Operational errors are never silently converted into successful cancellation.
- Snippet content containing quotes, backslashes, shell operators, Unicode, or newlines is preserved exactly.

# Workstream A: Explicit Selection Outcomes

## A1. Audit the current shared selection pipeline ✓

Inspect:

- `run_snippet_selection()` and its return type.
- `ui::select_snippet()` / `select_snippet_inner()`.
- `SnippetSelection` and any existing cancellation representation.
- How `run`, `clip`, and `search` currently distinguish selection, deletion, and cancellation.
- Signal and terminal restoration behavior on Escape, `q`, Ctrl-C, and process signals.

Document the exact existing flow before changing types. The implementation should minimize churn to commands unrelated to `select`.

## A2. Introduce an explicit outcome type ✓

Add or extend a shared result type that can represent at least:

```rust
pub enum SelectionOutcome<T> {
    Selected(T),
    Cancelled,
    NoSelection,
}
```

The exact shape may differ, but normal TUI cancellation must no longer be inferred from absent callback effects.

If deletion is part of the shared outer contract, use an outcome that can represent it explicitly, for example:

```rust
pub enum SelectionOutcome<T> {
    Selected(T),
    Cancelled,
    Deleted,
    EmptyLibrary,
}
```

Do not force unrelated commands to adopt new user-visible semantics. Existing commands may map `Cancelled` back to their current `Ok(())` behavior.

## A3. Make `snp select` return a structured command outcome ✓

Refactor `select_cmd::run()` so it returns a normal Rust value rather than terminating the process.

Preferred shape:

```rust
pub enum SelectOutcome {
    Selected,
    Cancelled,
}
```

or:

```rust
pub fn run(...) -> SnipResult<CommandOutcome>
```

The command implementation should:

- return selected output through the configured transport;
- return `Cancelled` for normal selector cancellation;
- return `Cancelled` for variable-prompt cancellation in expanded mode;
- return an error for operational failure;
- never call `std::process::exit`.

## A4. Preserve existing behavior for other commands ✓

`run`, `clip`, and `search` currently treat ordinary cancellation as success. Preserve that behavior unless the existing policy says otherwise.

The shared outcome refactor must not cause:

- `snp run` to return exit code 4;
- `snp clip` to emit new diagnostics;
- `snp search` to stop treating cancellation as a normal exit;
- deletion behavior to change;
- TUI terminal restoration regressions.

## A5. Add outcome-level tests ✓

Add unit tests for:

- selection returns `Selected`;
- Escape returns `Cancelled`;
- `q` returns `Cancelled` where supported;
- Ctrl-C returns `Cancelled` or the documented signal outcome;
- variable prompt cancellation returns `Cancelled`;
- deletion remains distinct from cancellation;
- empty library behavior remains unchanged;
- callback errors propagate as operational errors.

Acceptance criteria:

- No command implementation calls `std::process::exit` for selector cancellation.
- Normal TUI cancellation is observable as an explicit outcome.
- Existing commands preserve their prior user-visible cancellation semantics.

# Workstream B: Top-Level Exit-Code Mapping

## B1. Add a top-level command outcome contract ✓

Introduce a narrow process-boundary mapping in `main.rs` or the central dispatcher.

A suitable design is:

```rust
pub enum CommandOutcome {
    Success,
    Cancelled,
}
```

The dispatcher returns this outcome, and `main()` maps:

- `Success` -> exit 0;
- `Cancelled` -> exit 4 only for commands whose public contract defines it;
- `Err` -> existing error printing and exit 1.

Avoid broad exit-code redesign. This pass should not introduce a taxonomy for every `SnipError` variant.

## B2. Scope exit 4 to `snp select` ✓

The new cancellation exit code must apply to:

```text
snp select
snp select --raw
snp select --expanded
```

It must not silently alter cancellation behavior for `run`, `clip`, or `search`.

## B3. Verify stream behavior ✓

For `snp select`:

- success: selected command only on stdout unless `--output-file` is used;
- success with output file: no selected command on stdout;
- cancellation: no stdout, no generic error text, exit 4;
- operational error: diagnostic on stderr, no selected command on stdout, exit 1;
- help and Clap validation: retain Clap's normal behavior.

Update `docs/CLI_EXITCODE_STREAM_POLICY.md` only after tests establish the final behavior.

Acceptance criteria:

- Exit 4 is produced only at the process boundary.
- Stack unwinding and destructors run on cancellation.
- Terminal and temporary-file cleanup are not bypassed by process exit.

# Workstream C: Resolve the Ignored `--sync` Option

## C1. Decide based on existing command semantics ✓

Inspect why `--sync` was added to `Select` and whether the current shared selection API assumed parity with `run` or `search`.

The preferred closure decision is to remove `--sync` from `snp select` because:

- shell-buffer insertion should be fast and deterministic;
- implicit network work is undesirable in an interactive shell widget;
- the roadmap did not require selection-triggered sync;
- scheduled and explicit sync already exist.

Only retain and implement `--sync` if repository conventions clearly require it and behavior can be made consistent with an existing command.

## C2. Remove all stale documentation and tests if the option is removed ✓

Update:

- Clap definition;
- dispatch code;
- help tests;
- README/USER_GUIDE examples if present;
- architecture inventory;
- design-decision documentation.

Do not leave compatibility aliases or hidden no-op flags.

Acceptance criteria:

- No visible CLI option is accepted and ignored.
- `snp select --help` accurately reflects implemented behavior.

# Workstream D: Correct Shell Adapter Status Handling

## D1. Establish a strict status decision tree ✓

All generated shell adapters must implement this ordering:

1. Invoke `snp select`.
2. Capture its exit status immediately.
3. If exit status is 4, treat as user cancellation.
4. Else if exit status is nonzero, treat as operational failure.
5. Else validate the output transport contract.
6. On valid success, replace the buffer.

Do not use file emptiness before checking the exit status.

Representative intent:

```bash
snp select "${args[@]}"
rc=$?

if [[ $rc -eq 4 ]]; then
  restore_buffer
  cleanup
  return 0
elif [[ $rc -ne 0 ]]; then
  restore_buffer
  cleanup
  return "$rc"
elif [[ ! -f "$tmp_file" ]]; then
  echo "snp: selection output was not produced" >&2
  restore_buffer
  cleanup
  return 1
fi
```

Adapt syntax appropriately for Zsh and Fish.

## D2. Define empty-command behavior ✓

Determine whether an empty snippet command is valid in the current data model.

Preferred policy:

- Reject empty commands during snippet creation and library validation if this is already consistent with current expectations.
- Do not use `-s` or equivalent file-size checks as the sole success signal.

If legacy files can contain empty commands and must remain loadable, then shell transport must distinguish “success with empty content” from “no output produced.” Use file existence, an explicit status marker, or a structured transport file rather than file size.

Document the chosen policy and add tests.

## D3. Preserve original status on operational failure ✓

For shell functions:

- user cancellation should return shell success after restoring the buffer;
- operational failure should return nonzero;
- where practical, preserve the original `snp` exit status rather than collapsing everything to 1;
- diagnostics already emitted by `snp` should not be duplicated unnecessarily.

## D4. Preserve buffer and cursor exactly ✓

On cancellation or failure:

- Bash: restore both `READLINE_LINE` and `READLINE_POINT`.
- Zsh: restore both `BUFFER` and `CURSOR`, then redisplay.
- Fish: restore the original commandline and cursor position using supported Fish APIs.

On success:

- replace the buffer with the selected command;
- place cursor at the end unless the documented shell behavior intentionally differs;
- never execute the buffer.

Acceptance criteria:

- Operational errors cannot return shell success merely because the output file is empty.
- Cancellation and failure are distinguishable in every supported shell.
- Original buffer and cursor survive both cancellation and failure.

# Workstream E: Harden Output Transport

## E1. Reassess the pathname transport contract ✓

The current temp-file transport is acceptable for multiline fidelity, but `snp select --output-file` must not behave as a generic arbitrary overwrite primitive.

Choose one of the following approaches, in priority order.

### Preferred option: pre-created file contract

The shell wrapper creates the file securely with `mktemp`. `snp select` must:

- require the path to already exist;
- open it without following symlinks where supported;
- require it to be a regular file;
- truncate and write only after successful selection;
- preserve restrictive permissions;
- reject directories, FIFOs, sockets, devices, and symlinks;
- avoid creating parent directories.

On Unix, use an API or `OpenOptions` flags that can enforce `O_NOFOLLOW` where available. Add platform-conditional behavior where needed.

### Alternative option: inherited file descriptor

If practical without excessive complexity, allow shell wrappers to pass a writable inherited descriptor. This reduces path races but may be harder to express consistently across Bash, Zsh, Fish, and Windows.

Do not adopt this option if it substantially broadens the pass.

## E2. Consider hiding the option ✓

If `--output-file` exists only for generated integrations, mark it hidden from normal help while documenting the stable public behavior through `snp shell init`.

Do not hide it if users are expected to rely on it as a supported automation API. Make the decision explicit in code comments and design docs.

## E3. Ensure cancellation and failure do not leave stale content ✓

The wrapper creates a fresh temporary file. Still ensure:

- cancellation leaves no selected content;
- failure leaves no selected content;
- the wrapper removes the file on every branch;
- writing is atomic enough that the wrapper never reads a partially written command after success;
- errors during writing are surfaced as operational failures.

A straightforward write followed by close before returning success is sufficient for a unique private temp file.

## E4. Add filesystem safety tests ✓

Test:

- regular pre-created temp file succeeds;
- nonexistent destination is rejected if using the pre-created contract;
- symlink destination is rejected;
- directory destination is rejected;
- read-only destination fails cleanly;
- selected multiline content is exact;
- cancellation does not write content;
- operational failure does not write content;
- existing unrelated files cannot be overwritten through expected shell paths.

Acceptance criteria:

- The transport does not follow symlinks.
- The transport does not create arbitrary files.
- Shell wrappers remain lossless for multiline content.

# Workstream F: Real Shell and PTY Validation

## F1. Add shell-level tests with a stub `snp` ✓

For each supported shell, source the generated integration and place a stub `snp` executable earlier in `PATH`.

The stub should simulate:

- success with single-line output;
- success with multiline output;
- success with quotes, backslashes, Unicode, pipes, redirects, and command substitutions as literal text;
- user cancellation with exit 4;
- operational failure with exit 1;
- operational failure with another nonzero code;
- exit 0 without a valid output artifact;
- output-file write failure;
- missing `snp` executable.

Verify:

- buffer contents;
- cursor position;
- function return status;
- temporary-file cleanup;
- no execution of selected text;
- no `eval` of selected text.

## F2. Use actual shells, not only parser checks ✓

Run generated code in:

- Bash;
- Zsh;
- Fish.

If a shell is not available in the default CI image, install it in the relevant job or use dedicated jobs. Do not replace runtime tests with syntax-only checks.

Tests may call internal generated helper functions directly with simulated buffer variables where necessary, but at least one path per shell should exercise the actual public function.

## F3. Add targeted PTY tests for the real binary ✓

Use a PTY-capable test harness to validate the actual `snp select` binary against a temporary fixture library.

Required cases:

- open selector, choose a snippet, verify exact raw stdout;
- seed search with `--query`, verify intended item is selected;
- cancel from primary selector, verify exit 4 and empty stdout;
- expanded mode with a default value;
- expanded mode cancellation, verify exit 4;
- output-file mode writes exact content and no stdout;
- terminal state is restored after success;
- terminal state is restored after cancellation;
- terminal state is restored after operational error where feasible.

Keep PTY tests deterministic. Use a small fixture with uniquely searchable descriptions and commands. Avoid relying on terminal dimensions or timing more than necessary.

## F4. Add a test seam if necessary ✓

If the current TUI is too difficult to drive reliably, add a narrow internal abstraction for event input or selector outcomes. Do not add a public bypass that undermines the TUI contract.

A deterministic event-source abstraction under `cfg(test)` is preferable to flaky sleeps and raw byte timing.

## F5. Cross-platform test expectations ✓

The shell integration is Unix-oriented, but the core crate remains cross-platform.

Required matrix:

- Linux: full Bash/Zsh/Fish shell tests and PTY tests.
- macOS: at least Bash/Zsh generation and one real shell smoke test; Fish where available.
- Windows: core build, unit tests, CLI help, and non-shell `snp select` argument tests must continue to pass. Unix-only shell runtime tests should be properly gated.

Acceptance criteria:

- Real-shell tests prove status and buffer behavior.
- PTY tests prove the actual cancellation and output contract.
- No shell adapter is considered validated solely because its generated code parses.

# Workstream G: Documentation Reconciliation

## G1. Update the compatibility contract ✓

Revise `docs/PET_COMPATIBILITY.md` to state the final Release 1 behavior precisely:

- raw insertion;
- expanded insertion;
- current-buffer query seeding;
- exit 4 on user cancellation for `snp select`;
- shell wrappers restore the original buffer on cancellation;
- operational errors remain nonzero;
- no automatic keybindings;
- no execution of inserted content.

## G2. Update exit-code and stream policy ✓

Update `docs/CLI_EXITCODE_STREAM_POLICY.md` with tested final behavior.

Include an explicit table for `snp select`:

| Condition | Exit | stdout | stderr |
| --- | ---: | --- | --- |
| Selection to stdout | 0 | exact command | empty except tracing policy |
| Selection to output file | 0 | empty | empty except tracing policy |
| User cancellation | 4 | empty | empty |
| Operational failure | 1 | empty | diagnostic |
| Clap usage error | Clap-defined | help/usage as Clap emits | usage error |

Adjust if the implementation intentionally preserves non-1 operational statuses.

## G3. Correct architecture inventory drift ✓

Remove or correct stale statements, including any reference to `snp check` if the generated code uses `command -v snp`.

Update the test-infrastructure section to reflect the fixture corpus and new shell/PTY tests.

## G4. Ensure README and USER_GUIDE do not overpromise ✓

Verify all shell examples are syntactically correct and match generated function names.

Document:

- how cancellation is represented;
- how failures are surfaced;
- that no keybindings are installed;
- that the output-file mechanism is internal if hidden;
- that Windows does not use Bash/Zsh/Fish integration unless running an appropriate shell environment.

Acceptance criteria:

- Documentation matches tested behavior.
- No stale flags, function names, or internal mechanisms remain.

# Workstream H: Regression and Release Validation

## H1. Full local validation ✓

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If some feature combinations are intentionally unsupported, document the exact validated command set and reason.

## H2. Preserve the Release 1A baseline ✓

All fixtures and compatibility tests introduced in Release 1A must continue to pass.

Specifically verify:

- canonical Pet TOML loading;
- legacy uppercase aliases;
- mixed field aliases;
- native snip-it metadata preservation;
- variable parsing edge cases;
- empty library behavior;
- serialization round trips;
- current `run`, `clip`, `search`, and `list` behavior.

## H3. Add negative regression tests ✓

Add tests proving:

- `snp select --sync` is rejected if the option is removed;
- cancellation does not exit 0;
- operational failure is not converted to cancellation;
- no arbitrary output path is created;
- symlink output is rejected;
- shell wrappers do not execute selected commands;
- shell wrappers do not use `eval` on selected content;
- raw mode does not prompt for variables;
- expanded mode does prompt when required.

## H4. CI verification ✓

Ensure the head commit receives visible CI results for the repository's normal workflow matrix.

At minimum confirm:

- Linux test job passes;
- macOS test job passes;
- Windows test job passes;
- formatting/lint job passes;
- shell-specific jobs pass where configured.

Do not declare closure based only on commit-message claims.

## H5. Manual smoke checklist ✓

Before marking Release 1 closed, manually test on at least one real interactive shell:

### Bash

```bash
eval "$(snp shell init bash)"
bind -x '"\C-o": snp_select_raw'
```

Verify selection, cancellation, failure, and multiline insertion.

### Zsh

```zsh
eval "$(snp shell init zsh)"
bindkey '^O' snp_select_raw
```

Verify selection, cancellation, failure, and cursor restoration.

### Fish

```fish
snp shell init fish | source
bind \co snp_select_raw
```

Verify selection, cancellation, failure, and multiline insertion.

Record the shell versions used in the implementation summary or PR description.

# Recommended Implementation Sequence

Implement in this order:

1. Refactor the shared selection outcome.
2. Move exit-code mapping to the top-level command boundary.
3. Remove or implement the ignored `--sync` flag.
4. Correct shell status handling.
5. Harden the output-file contract.
6. Add shell-level stub tests.
7. Add targeted PTY tests for the real binary.
8. Reconcile documentation.
9. Run full cross-platform validation.

Do not start shell test expansion before the cancellation and output contracts are stable, or tests will encode transient behavior.

# Suggested Commit Structure

A clean implementation series could use:

1. `refactor: propagate explicit snippet selection outcomes`
2. `fix: map snp select cancellation at CLI boundary`
3. `fix: remove ignored select sync option`
4. `fix: distinguish shell cancellation from operational failure`
5. `fix: harden select output file transport`
6. `test: add real shell and pty coverage for selection integration`
7. `docs: finalize pet compatibility release 1 contract`

The implementing agent may combine commits where appropriate, but avoid one large undifferentiated commit if possible.

# Completion Criteria

Release 1 is closed only when all of the following are true:

- Primary TUI cancellation from `snp select` reliably exits 4.
- Expanded-variable cancellation also exits 4.
- No deep `std::process::exit` remains in the command implementation path.
- `run`, `clip`, and `search` retain their existing cancellation behavior.
- Operational errors remain distinguishable from cancellation in Bash, Zsh, and Fish.
- The ignored `--sync` option is removed or fully implemented.
- Output-file transport rejects unsafe destination types and does not follow symlinks.
- Multiline and special-character content survives end to end.
- Cancellation and failure preserve the original shell buffer and cursor.
- Selected content is never executed or evaluated by the adapter.
- Real shell tests pass for Bash, Zsh, and Fish.
- Targeted PTY tests pass for real selection, cancellation, and expanded mode.
- Release 1A compatibility and regression tests remain green.
- Formatting, Clippy, workspace tests, and CI matrix pass.
- README, USER_GUIDE, architecture inventory, compatibility matrix, and stream policy match the final behavior.

# Handoff Summary

This pass is a closure and correctness pass, not a redesign. The feature surface is already appropriate:

- `snp select` is the stable non-executing primitive.
- `snp shell init bash|zsh|fish` generates opt-in integrations.
- raw and expanded insertion remain separate.
- no keybindings are installed automatically.

The implementing agent should focus on making the existing promises mechanically true, especially the distinction between selection, cancellation, and failure. Once this plan is complete, Release 1 can be treated as closed and work can proceed to Release 2 history capture and multiline acquisition.
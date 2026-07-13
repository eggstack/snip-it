# Release 2 Corrective and Closure Pass: Secure Editor Input, Source-Contract Alignment, and End-to-End Preservation

## Purpose

Close the remaining correctness, security, portability, and validation gaps in the implemented Release 2 acquisition work before beginning Release 3.

Release 2 is substantially implemented:

- `snp new --command-stdin` provides exact bounded UTF-8 ingestion for shell helpers and pipelines.
- Generated Bash, Zsh, and Fish integrations provide current-buffer and previous-command capture helpers.
- `snp new --from-file` and `snp new --editor` add file-based and editor-based command creation.
- A golden command corpus verifies preservation across stdin and file sources.
- PTY and shell-generation tests cover key Release 1 and Release 2 paths.

The current product shape is correct and should not be redesigned. This pass is narrowly corrective and integrative. It must harden the editor workflow, reconcile the `--from-file` symlink contract, support ordinary editor command specifications, expand cross-source preservation coverage, and prove interoperability with storage, export, backup, sync, and shell-history capture.

This plan is intended for implementation-agent handoff. Inspect the repository at execution time. File names and signatures below describe required behavior rather than immutable implementation details. Do not begin Release 3 variable-choice, import, or diagnostic work in this pass.

## Current Known Issues

### 1. Editor temporary-file creation is not atomic

The current editor workflow derives a pathname from process ID and wall-clock nanoseconds, then creates it with ordinary `File::create` semantics before applying restrictive permissions.

Problems:

- `File::create` follows symlinks.
- An existing path is truncated.
- Uniqueness is probabilistic rather than guaranteed atomically.
- Permissions are tightened after creation rather than at creation time.
- The workflow duplicates functionality already available through the `tempfile` dependency.

This is the primary security blocker for Release 2 closure.

### 2. `--from-file` documentation and implementation disagree on symlinks

The documentation states that symlinks are rejected, while the implementation checks only for existence and directories before opening the path. A symlink to a regular file is therefore accepted.

The project must make an explicit decision and implement/document/test it consistently.

### 3. `$EDITOR` handling does not support arguments

The current editor resolver treats the full environment variable as one executable path. Common values such as the following fail:

```text
EDITOR="code --wait"
EDITOR="nvim -f"
EDITOR="emacsclient -c"
VISUAL="zed --wait"
```

The workflow must support an executable plus arguments without invoking a shell or evaluating arbitrary text.

### 4. Release 2C preservation validation is incomplete

The golden corpus currently proves stdin and file behavior, but does not comprehensively prove:

- editor-source equivalence;
- delimiter-based multiline behavior and documented limitations;
- positional-source equivalence where appropriate;
- retrieval through `select`, `list`, and clipboard-safe paths;
- export/import preservation where those commands exist;
- backup/recovery preservation;
- encrypted sync round trips;
- shell previous-command capture across supported shells.

### 5. Interactive multiline semantics need an explicit contract

The existing `--multiline` path terminates after two blank lines. It cannot represent every possible sequence of trailing or consecutive blank lines and is therefore not equivalent to exact stdin/file/editor sources.

This is acceptable if documented clearly as a convenience input mode. It must not be described as lossless or fully cross-source equivalent.

### 6. Previous-command helpers contain shell-specific portability assumptions

The Bash helper strips a fixed formatter prefix from `fc -ln` output. Previous-history behavior can vary across shell versions, invocation context, multiline history settings, and operating systems.

The implementation needs direct behavioral tests and, if necessary, more robust normalization that removes shell-added formatting without mutating real command bytes.

## Product Invariants

The closure pass must preserve these invariants:

1. Existing positional and interactive `snp new` behavior remains unchanged when new source flags are not used.
2. `snp new --command-stdin`, `--from-file`, and `--editor` never execute or evaluate command content.
3. Existing Release 1 selection and shell-buffer insertion behavior remains unchanged.
4. Existing `run`, `clip`, `search`, library, sync, and serialization semantics remain unchanged.
5. No shell startup files or keybindings are modified automatically.
6. Shell history files are never parsed directly.
7. Command bodies are not emitted in ordinary status messages or logs.
8. Source-resolution failures occur before library mutation.
9. All successful source modes use one validation and persistence pipeline.
10. Release 3 work is out of scope.

## Required Final Behavior

Release 2 is closed only when all of the following are true:

1. Editor temporary files are created atomically with private permissions and reliable cleanup.
2. `$VISUAL` and `$EDITOR` may contain executable arguments without shell evaluation.
3. `--from-file` symlink behavior is deliberate, tested, and documented.
4. Exact sources preserve valid command text byte-for-byte within the UTF-8 model.
5. Interactive `--multiline` limitations are documented and tested.
6. Shell current-buffer and previous-command helpers preserve command data and caller buffer state.
7. Golden commands survive storage rewrites, structured output, backup/recovery, and sync.
8. Failures do not append partial snippets or corrupt libraries.
9. Full repository validation and hosted CI are green.

# Workstream A: Secure Editor Temporary Files

## A1. Replace custom pathname generation

Use `tempfile::Builder`, `NamedTempFile`, or an equivalent atomic create-new abstraction.

Preferred characteristics:

- created in the operating system temporary directory;
- unique name chosen atomically;
- restrictive permissions from creation time where supported;
- file handle retained by the owning object;
- automatic deletion on normal return and error paths;
- explicit persistence never used for this workflow.

Representative shape:

```rust
let mut temp = tempfile::Builder::new()
    .prefix("snp-editor-")
    .tempfile()
    .map_err(...)?;

let path = temp.path().to_owned();
```

Do not generate a predictable filename and then call `File::create`.

## A2. Ensure editor access while retaining ownership

The editor must receive a stable pathname while the `NamedTempFile` owner remains alive.

After the editor exits:

- flush or reopen safely as needed;
- read the content from the owned file or its path;
- validate size, UTF-8, NUL bytes, and empty/whitespace policy;
- allow the tempfile object to delete the path automatically.

Do not transfer ownership to code that can leak the file unintentionally.

## A3. Verify permissions

On Unix, test that the temporary editor file is not group- or world-readable while the editor is running.

The test editor may inspect the file mode and write the observed mode to a separate test-controlled path.

Acceptance requirement:

```text
mode & 0o077 == 0
```

On Windows, rely on the platform behavior of the tempfile implementation and do not emulate Unix permission checks.

## A4. Cleanup tests

Add tests for cleanup after:

- successful editor exit;
- editor exits nonzero;
- editor writes empty content;
- editor writes invalid UTF-8;
- editor writes NUL bytes;
- editor replaces or deletes the path;
- command validation fails after editor exit.

No `snp-editor-*` file from the test invocation may remain after the command returns.

## A5. Avoid content leakage

Editor errors may identify the editor executable and exit status, but must not print the authored command body.

Do not include editor content in tracing spans, debug logs, audit messages, or error context.

# Workstream B: Editor Command Specification

## B1. Establish precedence

Use the conventional preference order:

1. `$VISUAL` when non-empty;
2. `$EDITOR` when non-empty;
3. existing project fallback, currently `vim`, unless repository conventions specify another fallback.

Document the precedence.

## B2. Parse executable and arguments without a shell

Support values such as:

```text
code --wait
nvim -f
emacsclient -c
zed --wait
"/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code" --wait
```

Use a shell-word parser suitable for command specifications, such as the existing or new `shell-words` dependency, but do not execute through `/bin/sh -c`, `cmd /C`, PowerShell, or `eval`.

The parser must produce:

```rust
struct EditorCommand {
    program: OsString,
    args: Vec<OsString>,
}
```

Then launch:

```rust
Command::new(program)
    .args(args)
    .arg(temp_path)
```

## B3. Define malformed-value behavior

Reject:

- an empty parsed command;
- unmatched quotes;
- invalid syntax according to the selected parser;
- a program that cannot be located or executed.

Diagnostics should identify `$VISUAL` or `$EDITOR` as the source without echoing sensitive command-body data.

## B4. Preserve cross-platform behavior

On Unix, resolve bare executable names through `PATH` naturally via `Command` or the existing editor resolver.

On Windows, retain support for executable extensions and quoted paths with spaces. Do not split Windows paths at spaces after parsing.

Avoid canonicalization rules that unnecessarily reject legitimate symlinked editor executables in `PATH`.

## B5. Add editor-spec tests

Test at least:

- one bare executable;
- executable plus one argument;
- executable plus multiple arguments;
- quoted executable path containing spaces;
- `$VISUAL` overriding `$EDITOR`;
- empty `$VISUAL` falling back to `$EDITOR`;
- malformed quoted value;
- nonexistent program;
- nonzero editor exit;
- successful editor write.

Use deterministic test scripts/executables rather than relying on developers having a particular editor installed.

# Workstream C: `--from-file` Symlink Contract

## C1. Make an explicit decision

Choose one of these policies and record it in code comments and documentation.

### Preferred policy: allow symlinks for read-only ingestion

Rationale:

- users commonly keep scripts behind symlinks or dotfile-managed paths;
- ingestion is read-only;
- the user explicitly supplies the path;
- rejecting symlinks does not materially improve confidentiality or integrity for the invoking user;
- descriptor-level race-free symlink rejection is additional complexity.

Under this policy:

- require that the resolved target can be opened as a regular file;
- reject directories and non-regular special files where practical;
- document that symlinks are followed;
- remove claims that symlinks are rejected;
- add tests for a symlink to a valid file and a broken symlink.

### Alternative policy: reject symlinks

If the repository intentionally requires this:

- use `symlink_metadata()` before opening;
- reject when `file_type().is_symlink()`;
- where practical, use no-follow open flags to avoid a check/open race;
- add Unix-specific tests.

Do not retain the current state where documentation and implementation disagree.

## C2. Define special-file behavior

Audit FIFOs, sockets, devices, and other non-regular paths.

Preferred behavior is to accept only regular files after resolution, preventing indefinite reads from FIFOs or device input.

Use metadata from the opened descriptor where possible rather than only pathname metadata.

## C3. Preserve bounded reads

Keep the 16 MiB cap and one-byte-over-limit detection.

Tests must cover:

- exactly 16 MiB;
- 16 MiB plus one byte;
- ordinary file;
- empty file;
- directory;
- missing path;
- invalid UTF-8;
- NUL byte;
- chosen symlink behavior;
- non-regular file where supported.

# Workstream D: Shared Command Validation

## D1. Remove validation drift

Create or confirm a shared helper for exact-source command data:

```rust
fn validate_exact_command_bytes(
    bytes: Vec<u8>,
    source_name: &'static str,
) -> SnipResult<String>
```

or an equivalent abstraction.

It should own:

- maximum size;
- UTF-8 decoding;
- NUL rejection;
- empty/whitespace-only validation policy where appropriate;
- source-specific diagnostic labels.

Use it for:

- `--command-stdin`;
- `--from-file`;
- editor output.

Do not maintain three independent implementations that can drift.

## D2. Keep source resolution separate from persistence

All command-source reading and validation must finish before:

- primary-library migration;
- backup creation;
- snippet append;
- atomic save;
- sync side effects.

Add negative tests that snapshot the library before and after each source failure and prove byte-identical persistence state.

## D3. Define empty and whitespace-only behavior

Align exact sources with `Snippet::new()` and existing command validation.

Document whether:

- empty input is rejected;
- whitespace-only input is rejected;
- a command containing only newlines is rejected;
- leading/trailing whitespace around non-empty command content is preserved.

The validator must not trim accepted content merely to test emptiness.

Representative approach:

```rust
if command.trim().is_empty() {
    return Err(...);
}
Ok(command)
```

# Workstream E: Multiline Input Contract

## E1. Document delimiter semantics

State clearly that `snp new --multiline` is an interactive convenience mode terminated by two blank lines.

It is not byte-exact for all possible scripts because the delimiter cannot be represented as content at the termination point.

Recommend these exact alternatives when fidelity matters:

```bash
snp new --command-stdin --description ...
snp new --from-file script.sh
snp new --editor
```

## E2. Preserve existing behavior

Do not silently change the multiline terminator in this closure pass unless there is a demonstrated correctness bug and a backward-compatible design.

Existing users may rely on the two-blank-line terminator.

## E3. Add multiline tests

Test and document:

- ordinary two-line command;
- internal single blank line;
- termination after two blank lines;
- EOF before delimiter;
- leading blank line;
- trailing newline behavior;
- whitespace-only delimiter lines;
- inability to represent the delimiter sequence as terminal content.

The expected result should be explicit, not described as exact equivalence.

# Workstream F: Shell History Capture Portability

## F1. Audit each shell helper

Inspect generated helpers for:

- Bash current buffer;
- Bash previous command;
- Zsh current buffer;
- Zsh previous command;
- Fish current buffer;
- Fish previous command.

For each, document:

- widget invocation behavior;
- ordinary function invocation behavior;
- whether the helper invocation itself enters history;
- multiline history representation;
- added formatter bytes or line terminators;
- buffer and cursor preservation;
- failure status propagation.

## F2. Harden Bash previous-command capture

Avoid relying on an unexplained unconditional two-byte removal unless behavior is proven across supported Bash versions.

Possible approaches:

- use a Bash format that suppresses history numbers without adding indentation;
- detect and remove only a known formatter prefix;
- use a shell-native array/value that provides the raw entry;
- maintain separate tested paths for Bash 3.2 and Bash 4+ if required.

Do not trim legitimate leading tabs or spaces from the command.

## F3. Test actual shells

Run behavioral tests in available native shells:

- Bash 3.2 or document why unavailable in CI;
- current supported Bash;
- Zsh;
- Fish.

Required cases:

1. ordinary previous command;
2. command with leading spaces;
3. command with tabs;
4. quotes and backslashes;
5. shell operators;
6. Unicode;
7. multiline history entry where supported;
8. no history available;
9. helper invoked as a widget;
10. helper invoked as a normal function.

The saved snippet must match the shell-visible command entry according to the documented shell contract.

## F4. Preserve caller state

On success and failure:

- current buffer remains unchanged;
- cursor remains unchanged;
- command is not executed;
- captured command is not printed;
- helper returns the meaningful `snp` status;
- temporary files are removed.

# Workstream G: Golden Corpus Expansion

## G1. Establish one canonical corpus

Move the command corpus into a reusable test helper or fixture rather than duplicating entries across many tests.

Include at least:

1. simple ASCII;
2. leading hyphen;
3. single and double quotes;
4. backslashes and Windows-style paths;
5. pipes, redirects, semicolons, and ampersands;
6. command substitution and backticks;
7. Unicode;
8. tabs;
9. leading spaces;
10. trailing spaces;
11. multiline script;
12. internal blank lines;
13. no trailing newline;
14. one trailing newline;
15. multiple trailing newlines;
16. variable placeholders;
17. escaped angle brackets;
18. carriage-return/line-feed input where platform policy is defined.

Correct documentation counts so they match the actual corpus.

## G2. Cross-source equivalence matrix

For exact sources, prove equivalent stored commands:

| Source | Required equivalence |
| --- | --- |
| `--command-stdin` | Canonical exact source |
| `--from-file` | Must match stdin for same bytes |
| `--editor` | Must match stdin for same editor-written bytes |
| positional argument | Compare for single-line strings representable losslessly by process arguments |
| shell current-buffer helper | Must match stdin for shell buffer content |
| shell previous-command helper | Must match documented shell history entry |
| `--multiline` | Test separately; do not require equivalence for delimiter-sensitive cases |

## G3. Verify metadata consistency

Across sources, verify identical handling of:

- description;
- explicit tags;
- named library;
- primary library;
- IDs and timestamps generated by the common model;
- favorite/folder defaults if applicable;
- backup creation;
- atomic persistence.

Source mode must not alter snippet metadata except where explicitly documented.

# Workstream H: Downstream Preservation

## H1. Storage rewrite stability

Create golden snippets, force multiple full-library rewrites, and verify command content remains unchanged after each rewrite.

Exercise at least:

- adding another snippet;
- editing through supported code paths if available;
- deletion/tombstone update;
- primary-library switch where relevant;
- backup creation.

## H2. Structured output

Verify exact logical command value through:

- `snp list --json`;
- `snp list --csv`;
- any existing export command or structured export path.

CSV tests should parse CSV with a real parser rather than asserting raw line fragments.

## H3. Selection and clipboard paths

Use PTY tests to verify:

- raw `snp select` emits exact multiline content to an output file;
- raw selection preserves trailing newlines;
- shell buffer insertion preserves the documented content;
- expanded selection behavior remains unchanged;
- clipboard path does not corrupt multiline content, where clipboard tests are feasible in CI.

Do not execute golden commands in tests unless the command is a deliberately safe fixture.

## H4. Run behavior

For a controlled multiline fixture, verify `snp run` passes the stored command to the shell as expected.

Use a temporary output target and an inert script body. Do not use destructive shell operations.

The purpose is to prove storage-to-execution plumbing, not to redesign shell invocation.

## H5. Backup and recovery

Verify that:

- a backup contains the exact serialized logical command values;
- restoring or loading the backup preserves the golden corpus;
- no source-specific data is lost;
- a failed write leaves the prior library and backup state coherent.

## H6. Sync round trip

Use the existing in-process `snip-sync` test infrastructure.

Required cases:

1. push a multiline/trailing-newline snippet;
2. retrieve it on a second client/device fixture;
3. verify exact command equality;
4. update metadata without changing command content;
5. verify command remains unchanged after merge;
6. verify tombstone behavior is unaffected.

Do not log plaintext command bodies in sync test diagnostics unless a test fails, and even then prefer labels/hashes over secret-like fixture contents.

# Workstream I: Security and Privacy Review

## I1. Command-body logging audit

Search tracing, debug, audit, error, and status paths for command bodies introduced by Release 2.

Ensure ingestion paths do not log:

- stdin payload;
- file content;
- editor content;
- current shell buffer;
- previous history command.

Logging source type, byte length, library name, and success/failure is acceptable where consistent with current privacy policy.

## I2. Argument-injection audit

Verify that metadata forwarded by shell helpers remains separate arguments.

Verify editor specifications are parsed to program/args and never concatenated into a shell command.

Test values containing:

- spaces;
- quotes;
- leading hyphens;
- semicolons;
- command substitution syntax.

None may be evaluated by the helper or editor launcher.

## I3. File-path audit

Review:

- `--from-file` path handling;
- editor tempfile path handling;
- shell helper temporary files;
- output-file transport from Release 1.

Document which paths follow symlinks, which reject them, and why.

Do not make broad unrelated filesystem changes unless needed to close a demonstrated issue.

# Workstream J: Documentation Reconciliation

## J1. README and user guide

Update user-facing documentation to state:

- exact-source options and their preservation guarantees;
- `--multiline` delimiter semantics and limitations;
- `$VISUAL`/`$EDITOR` precedence;
- editor values may contain arguments;
- editor execution does not use a shell;
- final `--from-file` symlink policy;
- 16 MiB limit;
- UTF-8 and NUL restrictions;
- shell-history privacy warning;
- shell-version limitations where applicable.

## J2. Architecture documentation

Update:

- `architecture/commands/new_cmd.md`;
- `architecture/cli.md`;
- `docs/ARCHITECTURE_INVENTORY.md`;
- `docs/PET_COMPATIBILITY.md`;
- `docs/CLI_EXITCODE_STREAM_POLICY.md` where relevant;
- `AGENTS.md` test commands and invariants.

Document the common source-validation pipeline and editor command parser.

## J3. Plan status

Do not mark all Release 2C tasks complete until the code, tests, and CI evidence exist.

Once closure criteria pass, update the Release 2 plan files with a concise completion note or checklist rather than mechanically checking unsupported claims.

# Workstream K: Test Organization and CI

## K1. Split monolithic acquisition tests where practical

`tests/integration.rs` has grown substantially. Move Release 2-specific tests into focused modules or files if this can be done without destabilizing shared helpers.

Suggested structure:

```text
tests/new_stdin.rs
tests/new_file.rs
tests/new_editor.rs
tests/shell_capture.rs
tests/acquisition_golden.rs
tests/pty_integration.rs
```

A shared `tests/common/mod.rs` may own temporary config setup, command invocation, and the golden corpus.

Do not make test reorganization a blocker if it causes disproportionate churn, but prevent further uncontrolled growth.

## K2. Full validation commands

Run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
```

Also run focused Release 2 suites explicitly so failures are easy to diagnose.

## K3. Cross-platform expectations

Linux:

- full unit/integration suite;
- Bash/Zsh/Fish tests where shells are installed;
- Unix tempfile permission checks;
- symlink/special-file tests.

macOS:

- full client suite;
- system Bash 3.2 compatibility test where practical;
- Zsh shell-helper tests;
- editor command with a path containing spaces;
- PTY suite.

Windows:

- compilation and non-shell acquisition tests;
- file and editor source tests using Windows-compatible fixture executables;
- no Unix-only permission assertions;
- shell-generation output may be tested as text without requiring Bash/Zsh/Fish.

## K4. Hosted CI evidence

Confirm visible CI results for the head commit. Do not declare Release 2 closed solely from local test counts or commit-message claims.

Record any skipped platform-specific tests and the reason.

# Recommended Implementation Order

Implement in this sequence:

1. Workstream A — secure tempfile replacement.
2. Workstream B — editor specification parsing and precedence.
3. Workstream C — resolve file symlink and special-file contract.
4. Workstream D — consolidate exact-source validation.
5. Workstream E — document and test multiline semantics.
6. Workstream F — harden and validate shell history capture.
7. Workstream G — expand cross-source golden corpus.
8. Workstream H — downstream storage/export/backup/sync validation.
9. Workstream I — security/privacy audit.
10. Workstream J — documentation reconciliation.
11. Workstream K — full CI and release closure.

Keep implementation commits focused. A reasonable commit series is:

```text
fix: create editor input files atomically
feat: support VISUAL and EDITOR command arguments
fix: align from-file path contract and validation
refactor: unify exact command source validation
test: expand acquisition cross-source preservation corpus
test: validate shell history capture portability
test: verify backup export and sync preservation
docs: close Release 2 acquisition contract
```

# Acceptance Criteria

Release 2 may be marked complete only when all criteria below are satisfied.

## Editor security

- Temp files are atomically created through a secure tempfile abstraction.
- Unix permissions are private from creation time.
- Temp files are removed on every success and failure path.
- Editor content is not logged.
- `$VISUAL` and `$EDITOR` arguments work without shell evaluation.

## File input

- Symlink behavior is explicitly chosen, implemented, documented, and tested.
- Directories and unsupported special files are rejected.
- Size, UTF-8, NUL, and empty-command rules match the shared validator.

## Acquisition correctness

- Stdin, file, and editor sources preserve the golden corpus exactly.
- Positional and shell-current sources match exact sources for representable commands.
- Shell previous-command capture matches documented native-history behavior.
- Interactive multiline behavior is stable and its delimiter limitation is explicit.

## Downstream integrity

- Commands survive repeated storage rewrites.
- JSON and CSV outputs remain valid.
- Selection and shell insertion preserve multiline content.
- Backup/recovery preserves command values.
- Sync round trip preserves multiline and trailing-newline content.
- Existing Release 1 behavior remains green.

## Regression and release readiness

- Existing positional and prompt creation behavior is unchanged.
- Source failures do not mutate libraries.
- No command-body evaluation or execution occurs during ingestion.
- Full formatting, lint, workspace, PTY, and platform CI checks pass.
- Documentation matches tested behavior.

# Release 2 Exit Statement

After this plan is completed, the project should be able to state:

> Snip-it can safely acquire commands from stdin, files, an external editor, the current shell buffer, and native shell history. Exact sources preserve valid UTF-8 command text without evaluation, all sources use one persistence pipeline, editor temporary files are private and atomic, shell helpers preserve caller state, and multiline commands survive storage, structured output, backup, and encrypted synchronization. Existing snippet creation and Release 1 shell-selection behavior remain unchanged.

# Release 2C Plan: Acquisition Integration, Preservation Audit, and Release Closure

## Purpose

Integrate and close the Release 2 acquisition work after safe stdin/history capture and multiline/file/editor creation have landed.

This pass is not a new feature phase. It exists to prove that every supported command-acquisition path enters the same creation pipeline, preserves command text as specified, leaves existing behavior unchanged, and interoperates correctly with the rest of snip-it.

Release 2 should not be considered complete based only on individual unit tests or command help output. The defining requirement is that a user can acquire a command from normal shell work or author a short script through any supported source, store it without accidental interpretation, and retrieve or use it through existing snip-it workflows without corruption.

This document is intended for implementation-agent handoff. Inspect the repository and the actual Release 2A/2B implementations before applying this plan. Correct implementation drift rather than assuming the earlier plan signatures landed verbatim.

## Scope

This closure pass covers:

- `snp new --command-stdin`;
- current-buffer shell capture;
- previous-command shell capture;
- optional shell-native history selection if implemented;
- `snp new --multiline`;
- `snp new --from-file`;
- `snp new --editor`;
- shared metadata prompting and noninteractive creation;
- exact-text preservation across storage and retrieval;
- shell, PTY, export, sync, backup, and cross-platform validation;
- documentation reconciliation.

## Non-Goals

- Adding Release 3 variable-choice syntax.
- Adding pet import or doctor commands.
- Adding sorting, usage ranking, notes, or external libraries.
- Adding automatic synchronization.
- Expanding history capture into a persistent shell telemetry system.
- Adding a built-in editor or script runner.
- Redesigning the TUI for large scripts beyond narrow correctness fixes.
- Reopening the Release 1 selection and shell-buffer architecture unless a regression is found.

# Workstream A: Implementation Audit

## A1. Build a source-to-persistence map

Document the actual flow for every command source:

```text
positional argument
interactive prompt
--command-stdin
--multiline
--from-file
--editor
current shell buffer
previous shell command
optional history selection
```

For each source, identify:

- clap parsing and conflicts;
- source reader;
- UTF-8 and size validation;
- newline policy;
- metadata prompt ownership;
- command validation;
- snippet construction;
- library resolution;
- backup and atomic save;
- logging and audit behavior;
- returned exit status.

There should be one shared persistence path after source resolution. If Release 2A and 2B introduced parallel creation implementations, consolidate them before closure.

## A2. Compare implementation to documented contracts

Review:

- README;
- USER_GUIDE;
- command help output;
- PET compatibility matrix;
- CLI stream/exit policy;
- architecture inventory;
- AGENTS instructions;
- Release 2 plan files.

List every mismatch before modifying code or docs.

## A3. Confirm existing behavior did not drift

Specifically compare pre-Release 2 behavior for:

- `snp new` with no arguments;
- positional `snp new '<command>'`;
- `--description` noninteractive creation;
- tags and library selection;
- backups and serialization;
- `run`, `clip`, `search`, and `select`;
- shell insertion functions from Release 1.

Any intentional change must be documented and separately justified. Compatibility work should not silently alter established defaults.

# Workstream B: Canonical Source Contract

## B1. Publish a source behavior matrix

Add a concise authoritative table to documentation and tests:

| Source | Exact input | Trailing newline | Metadata input | Cancellation | Empty input |
| --- | --- | --- | --- | --- | --- |
| positional | argument bytes as UTF-8 | preserved | normal prompt/flags | existing behavior | existing policy |
| stdin | full stdin | preserved | flags or controlling TTY policy | before save | explicit policy |
| multiline | entered lines | documented | controlling TTY/flags | explicit | explicit policy |
| file | opened file content | preserved | normal prompt/flags | before save | explicit policy |
| editor | final temp-file content | preserved | normal prompt/flags | editor/empty policy | explicit policy |

Shell helper rows should refer to the stdin contract rather than defining separate storage semantics.

## B2. Resolve empty-command consistency

Audit whether empty command strings can enter through any source.

Choose one coherent policy:

- reject empty commands across all new sources while preserving legacy loading; or
- permit them everywhere because the existing product does.

Do not allow accidental empty snippets from missing history or failed shell capture.

Tests must distinguish:

- zero-byte input;
- a single newline;
- whitespace-only command;
- no history result;
- editor file unchanged/empty.

## B3. Resolve newline and line-ending consistency

Verify the implementation does not accidentally mix:

- `read_to_string` exact behavior;
- `.lines()` reconstruction;
- `println!`-added newlines;
- command-substitution trimming;
- CRLF conversion.

The stored command should follow the documented source policy, not whichever helper API was convenient.

# Workstream C: Shell Capture Closure

## C1. Audit generated Bash helpers

Verify current-buffer and previous-command functions:

- use `printf '%s'`, not `echo`;
- pass arguments as arrays;
- do not use `eval` on captured command content;
- do not execute captured content;
- preserve the current buffer and cursor;
- retrieve the intended previous command rather than the helper invocation;
- propagate `snp new` status;
- do not print command bodies;
- handle no-history cleanly.

Test under a real interactive Bash session, not only `bash -n`.

## C2. Audit generated Zsh helpers

Verify:

- ZLE widget registration;
- `BUFFER` and `CURSOR` preservation;
- previous-event selection timing;
- multiline history behavior;
- metadata argument forwarding;
- redisplay after return;
- no execution or eval.

Test under real Zsh/ZLE where available.

## C3. Audit generated Fish helpers

Verify:

- full commandline capture;
- native history command semantics;
- no token-only truncation;
- buffer preservation;
- argument forwarding;
- multiline limitations documented if unavoidable;
- no execution or eval.

Test under an actual Fish process.

## C4. Sensitive-data behavior

Use secret-like fixtures such as:

```text
curl -H 'Authorization: Bearer test-secret-token' https://example.invalid
```

Verify the command does not appear in:

- helper status messages;
- stderr on success;
- normal logs;
- audit logs, unless the repository already explicitly logs full commands and that policy is separately accepted;
- test failure diagnostics under ordinary passing behavior.

Do not store real credentials in fixtures.

# Workstream D: Multiline and Source Interoperability

## D1. Golden command corpus

Create a reusable fixture corpus containing:

1. single-line ASCII command;
2. command beginning with a hyphen;
3. single and double quotes;
4. backslashes and Windows-like paths;
5. pipes, redirects, semicolons, and ampersands;
6. `$()` and backticks;
7. Unicode;
8. tabs;
9. multiline shell script;
10. blank internal lines;
11. no trailing newline;
12. one trailing newline;
13. multiple trailing newlines;
14. variable placeholders and escaped angle brackets;
15. CRLF content where platform handling is relevant.

Feed the same corpus through every applicable source and compare the resolved command strings.

## D2. Cross-source equivalence

For sources that receive the same bytes, assert the stored command is identical.

Examples:

- stdin vs file;
- stdin vs editor fake output;
- current-buffer helper vs direct stdin;
- previous-command helper vs direct stdin;
- multiline input vs equivalent file, excluding only the documented terminator behavior.

## D3. Storage round trip

For each golden command:

1. create snippet;
2. save TOML;
3. reload library;
4. compare command exactly;
5. save again;
6. reload again;
7. confirm no progressive normalization.

Include legacy pet-compatible serialization expectations.

# Workstream E: Existing Feature Compatibility

## E1. List and structured export

Verify multiline and special-character commands through:

- default list display;
- `list --json`;
- `list --csv`.

JSON and CSV must remain structurally valid. CSV fields containing embedded newlines and quotes must be escaped according to the existing format contract.

## E2. Search and TUI preview

Use PTY tests to ensure a multiline entry does not:

- break terminal layout;
- leak raw mode;
- make selection impossible;
- cause unreasonable rendering loops;
- corrupt adjacent fields.

Narrow clipping or preview fixes are allowed; broad TUI redesign is not.

## E3. Select and shell insertion

Verify raw and expanded selection preserve the multiline structure and trailing-newline policy through:

- stdout;
- output-file transport;
- Bash insertion;
- Zsh insertion;
- Fish insertion where supported.

A shell may not support a visually pleasant multiline buffer, but content must not be silently flattened.

## E4. Run and clip

Verify:

- `clip` receives exact command text;
- `run` executes the same stored command semantics as before;
- multiline variables expand without collapsing lines;
- output redirection metadata behavior is unchanged.

Do not use destructive commands in tests.

## E5. Backups and recovery

Create multiline snippets, trigger backup rotation, reload backups, and verify exact command recovery.

Test a simulated failed save if the repository has a suitable fault-injection seam.

## E6. Sync

Use the existing in-process sync test infrastructure to verify:

- encrypted round trip of multiline commands;
- trailing newline preservation;
- merge behavior;
- local-only field preservation;
- tombstone behavior remains unchanged;
- configured gRPC message limits are respected.

# Workstream F: Security and Resource Review

## F1. Command body confidentiality

Audit all new code for:

- `tracing` fields containing commands;
- `Debug` derives on input structures used in logs;
- errors that include full input;
- shell helper `set -x` implications;
- temp files readable by other users;
- stale temp files after failure.

Redact or omit command bodies from normal diagnostics.

## F2. File and editor safety

Verify:

- editor temp files use exclusive private creation;
- cleanup occurs on every ordinary exit path;
- file ingestion opens once and reads only;
- directory and invalid-UTF-8 errors are clear;
- symlink policy matches docs;
- source files are never modified.

## F3. Size limits and denial-of-service behavior

Test the chosen command-size policy near and beyond its limit.

Ensure:

- errors occur before expensive serialization or sync where practical;
- size messages do not echo content;
- large but valid short-script inputs do not cause pathological TUI behavior;
- the limit is consistent across stdin, file, editor, and multiline sources.

## F4. Shell injection review

Generated helpers necessarily invoke `snp`, but captured content must only travel through stdin/data channels.

Search generated code for:

- `eval` involving captured text;
- unquoted expansions;
- command construction strings;
- `echo` option interpretation;
- history numbering stripping that invokes shell parsing;
- temporary files with predictable names.

# Workstream G: Outcome and Stream Contracts

## G1. Define command outcomes

Document and test the final status for:

| Scenario | Exit | stdout | stderr |
| --- | --- | --- | --- |
| successful creation | 0 | current stable status policy | empty/status per existing behavior |
| usage conflict | clap usage code | empty/help | diagnostic |
| invalid UTF-8 | 1 | empty | diagnostic |
| source read failure | 1 | empty | diagnostic |
| metadata cancellation | stable cancellation policy | empty | empty/minimal |
| editor nonzero exit | 1 | empty | diagnostic |
| no previous history | helper nonzero | empty | concise diagnostic |
| save failure | 1 | empty | diagnostic |

Do not accidentally map all shell-helper errors to success.

## G2. Controlling terminal ownership

Where stdin is command data, verify prompts use the chosen controlling-terminal policy and cannot consume command bytes.

Tests should cover:

- piped stdin with `--description`;
- piped stdin without description and no TTY;
- piped stdin with a PTY available;
- redirected stdin;
- closed stdin.

# Workstream H: Test Architecture

## H1. Unit tests

Keep pure tests for:

- source conflict resolution;
- exact reading;
- newline policy;
- size validation;
- empty-command policy;
- fake editor result handling;
- history output cleanup/parsing where unavoidable.

## H2. CLI integration tests

Spawn the real binary with temporary config roots and verify library content after each source mode.

Avoid relying solely on stdout text.

## H3. Real-shell behavioral tests

Source generated scripts in Bash, Zsh, and Fish with a stub or real `snp` as appropriate.

At minimum prove:

- current-buffer capture;
- previous-command capture;
- no-history failure;
- metadata forwarding;
- exact metacharacter preservation;
- no execution.

## H4. PTY tests

Use serialized PTY tests for:

- multiline input completion and cancellation;
- editor flow if practical;
- current-buffer capture in at least one real shell;
- prior-command capture in at least one real shell;
- terminal restoration after each flow;
- subsequent shell command execution proving the PTY remains usable.

Avoid fixed sleeps where output/event synchronization can be used.

## H5. Platform gates

Keep Unix-only shell and permission tests under appropriate cfg gates. Ensure the entire workspace still compiles and non-PTY tests pass on Windows.

## H6. Full validation commands

Run and record:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
```

If Release 2 adds a separate PTY target, run it serialized as well.

# Workstream I: Documentation Closure

## I1. User documentation

Ensure README and USER_GUIDE clearly distinguish:

- selecting an existing snippet into the shell buffer;
- saving the current buffer as a new snippet;
- saving the previous accepted command;
- creating from stdin;
- multiline terminal entry;
- creating from a file;
- authoring in an editor.

## I2. Security guidance

Document:

- shell history may contain secrets;
- no automatic capture occurs;
- captured commands are not executed;
- source files are not modified;
- editor temp files are private and removed;
- command size and UTF-8 limitations.

## I3. Compatibility matrix

Mark the exact pet acquisition workflows now supported and identify intentional differences, especially:

- no default keybindings;
- no direct history-file parsing;
- native shell helpers instead of external selector dependencies;
- snip-it libraries remain canonical.

## I4. Agent documentation

Update AGENTS and architecture inventory with:

- unified source model;
- shell helper names;
- real-shell test requirements;
- PTY serialization requirement;
- confidentiality invariant for command bodies.

# Release 2 Closure Checklist

Release 2 is complete only when all of the following are true:

1. Every command source routes through one persistence pipeline.
2. Existing `snp new` behavior remains compatible.
3. Source flag conflicts fail before side effects.
4. Exact-text and newline policies are documented and tested.
5. Current-buffer helpers work in Bash, Zsh, and Fish.
6. Previous-command helpers work in Bash, Zsh, and Fish or documented shell limitations are explicit and acceptable.
7. No helper parses history files directly.
8. Captured text is never executed or evaluated.
9. Command bodies do not appear in normal logs or status output.
10. Multiline commands round-trip through storage, selection, clipboard, export, backup, and sync.
11. Editor temp files are private and cleaned up.
12. File ingestion is read-only and bounded.
13. PTY tests prove terminal restoration and at least one real shell capture flow.
14. Release 1 selection/insertion tests remain green.
15. Hosted CI is green on the repository's normal platform matrix.

# Recommended Commit Structure

Keep the closure work reviewable. A sensible sequence is:

1. `test: add Release 2 preservation corpus and source matrix`
2. `fix: unify acquisition sources through shared creation pipeline`
3. `fix: close shell history capture edge cases`
4. `test: add Release 2 shell and PTY integration coverage`
5. `docs: finalize Release 2 acquisition contracts`

Do not combine unrelated Release 3 work into these commits.

# Definition of Done

Release 2 is closed when the acquisition workflows are not merely present but proven equivalent at the storage boundary, safe at the shell boundary, compatible with existing commands, and documented without ambiguity.

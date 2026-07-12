# Release 2B Plan: Multiline, File, and Editor-Based Snippet Creation

## Purpose

Make short scripts and complex commands first-class creation inputs without requiring users to edit library TOML manually.

This track adds coherent, additive creation modes for multiline terminal input, stdin, file content, and editor-based authoring while preserving all existing `snp new` behavior and reusing one validation and persistence pipeline.

This document is intended for implementation-agent handoff. Inspect the repository state before editing. The exact clap hierarchy, prompt abstractions, and module boundaries may have evolved since this plan was written.

## Relationship to Release 2A

Release 2A establishes safe command ingestion and generated shell history capture, preferably through `snp new --command-stdin`.

Release 2B must reuse that source-resolution and persistence architecture rather than adding parallel code paths.

If Release 2A has not yet landed, implement the shared creation-input abstraction first and coordinate option names so both plans converge on one command-source model.

## Goals

1. Add explicit multiline terminal input.
2. Add file-based command ingestion.
3. Add editor-based command authoring.
4. Preserve command text exactly according to documented source-specific policies.
5. Route every source through existing metadata prompting, validation, snippet construction, backups, atomic saves, and library selection.
6. Ensure multiline commands work through list, search, run, clip, select, shell insertion, export, import-compatible TOML, and sync.
7. Use restrictive temporary-file permissions and reliable cleanup.
8. Add cross-platform tests where the functionality is portable and Unix PTY tests where terminal behavior matters.

## Proposed CLI Surface

```text
snp new --multiline
snp new --command-stdin
snp new --from-file <path>
snp new --editor
```

These are mutually exclusive command-body sources and must conflict with a positional command argument.

Existing forms remain valid:

```text
snp new 'git status'
snp new 'git status' --description 'Status'
snp new
```

## Non-Goals

- Executing or syntax-checking imported scripts.
- Automatically choosing a shell language.
- Adding a general script runner distinct from existing `snp run`.
- Embedding binary files or invalid UTF-8.
- Recursive directory import.
- Importing metadata from comments or shebangs.
- Adding rich editor integration, LSP support, or a built-in text editor.
- Changing existing TOML format merely because content is multiline.
- Capturing command output.
- Starting Release 3 import diagnostics.

## Product Invariants

### One creation pipeline

All source modes resolve to a command string, then use one shared metadata, validation, and persistence path.

### Source flags are explicit

Do not guess that piped stdin should become a command unless `--command-stdin` is present. Do not silently open an editor because stdin is absent. Existing interactive behavior remains the default.

### Text is not normalized

Do not reindent, shell-escape, line-wrap, trim, convert tabs, or normalize line endings without an explicit and documented platform policy.

### No execution during creation

File content, editor output, and terminal input are always treated as data.

# Workstream A: Unified Source Semantics

## A1. Finalize the command-source enum

Extend the shared source model from Release 2A to represent:

```rust
pub enum CommandSource {
    Positional(String),
    Prompt,
    Stdin,
    MultilineTerminal,
    File(PathBuf),
    Editor,
}
```

The exact type may differ, but source selection and source reading must remain separate from persistence.

## A2. Define flag conflicts

Clap should reject combinations such as:

- positional command plus `--command-stdin`;
- positional command plus `--from-file`;
- `--multiline` plus `--editor`;
- `--from-file` plus `--command-stdin`;
- multiple file arguments where only one is supported.

Usage errors must occur before any file read, editor launch, prompt, or library mutation.

## A3. Define source-specific newline policy

Use a documented matrix:

| Source | Newline policy |
| --- | --- |
| Positional argument | preserve argument exactly |
| `--command-stdin` | preserve exact stdin bytes after UTF-8 validation |
| `--from-file` | preserve exact file content after UTF-8 validation |
| `--editor` | preserve editor file content, subject to explicit empty-template policy |
| `--multiline` | preserve entered line separators; terminator itself is not stored |

Do not use global trimming after source resolution.

# Workstream B: Multiline Terminal Entry

## B1. Specify interaction model

Preferred behavior:

```text
snp new --multiline
```

The command prints a concise prompt to the controlling terminal and reads lines until a clear termination action.

Possible termination models:

- EOF (`Ctrl-D` on Unix);
- a dedicated line containing a documented sentinel;
- editor-style confirmation key sequence if a TUI input component already exists.

Prefer EOF if it can be implemented predictably across supported terminals without colliding with metadata prompts.

Do not use an ambiguous sentinel such as a single `.` unless documented and escapable.

## B2. Keep prompts off command data stdin

If multiline data consumes stdin, metadata prompts must use a controlling-terminal abstraction or require noninteractive metadata flags.

The same stream must never contain both command body and metadata responses without a framed protocol.

## B3. Cancellation semantics

Distinguish:

- completing multiline input;
- entering an empty command;
- cancelling the operation;
- terminal read failure.

Cancellation must leave the library unchanged.

If EOF is completion, provide another supported cancellation path and document it.

## B4. Terminal restoration

If raw mode or a TUI input widget is used, restore:

- canonical input mode;
- echo;
- cursor visibility;
- alternate screen;
- mouse capture;
- signal handling.

Add PTY tests for success, cancellation, and interruption.

# Workstream C: File-Based Creation

## C1. Add `--from-file`

Preferred surface:

```text
snp new --from-file ./deploy.sh --description 'Deploy service'
```

Read the file as data and store its content as the command.

## C2. File validation

Requirements:

- reject directories;
- reject invalid UTF-8;
- define symlink policy explicitly;
- reject or bound impractically large files;
- provide useful path-aware errors;
- never execute, source, chmod, or modify the source file.

A reasonable policy is to permit ordinary user-selected symlinks for read-only ingestion, because the command is explicitly asking to read a path. If symlinks are rejected for consistency with repository security posture, document that decision and test it.

## C3. Race and consistency behavior

Open the file once and read from the opened handle. Do not check metadata and then reopen by pathname unless necessary.

If size limits are enforced, validate the opened file metadata and also bound the actual read.

## C4. File-origin metadata

Do not persist the source path in the pet-compatible snippet schema unless separately approved. The description may default from the file name only if this is opt-in or consistent with current prompt behavior.

# Workstream D: Editor-Based Creation

## D1. Add `--editor`

Preferred surface:

```text
snp new --editor
```

Launch the user's configured editor to author only the command body, then continue through normal description/tag/library handling.

Do not repurpose `snp edit`, which edits an entire library.

## D2. Editor resolution

Reuse the repository's existing editor resolution and safe process-spawn logic.

Expected order should remain consistent with existing behavior, such as:

1. explicit supported configuration, if present;
2. `$VISUAL` if the project supports it;
3. `$EDITOR`;
4. documented fallback.

Do not invoke through `sh -c` when a direct executable-and-arguments model is available.

## D3. Temporary-file security

Create the editor file with:

- unpredictable name;
- restrictive permissions, preferably `0600` on Unix;
- exclusive creation;
- RAII cleanup;
- no reuse of user-controlled predictable paths.

The temp file may contain sensitive command text and must be removed after success, cancellation, editor failure, or panic where recoverable.

## D4. Initial template

Prefer an empty file unless a concise comment template provides substantial value.

If comments are included, stripping them risks modifying legitimate shell content. Therefore, do not add comment-template stripping in the initial implementation.

## D5. Editor exit semantics

Define:

- editor exit 0 with nonempty content: continue creation;
- editor exit 0 with empty content: cancel or reject according to existing empty-command policy;
- editor nonzero exit: error, no mutation;
- editor not found: clear error;
- editor process interrupted: no mutation.

Do not print the command body in errors.

# Workstream E: Shared Metadata and Persistence

## E1. Metadata prompts

Every new source mode must support current options for:

- `--description`;
- tags;
- target library;
- any existing config override.

Interactive metadata behavior should remain identical after command acquisition.

## E2. Validation

Reuse current validation for:

- empty command;
- malformed tags;
- library existence/name safety;
- unmatched variable syntax warnings or errors;
- duplicate IDs and timestamps.

Do not introduce shell-language parsing.

## E3. Atomic mutation

All source modes must preserve:

- backup behavior;
- atomic write behavior;
- TOML cache invalidation;
- sync metadata generation;
- failure rollback.

Add failure tests if the shared creation path is refactored significantly.

# Workstream F: End-to-End Multiline Compatibility

## F1. TOML round trip

Verify commands containing:

- multiple lines;
- blank lines;
- trailing newline;
- triple quotes or quote-heavy shell fragments;
- backslashes;
- Unicode;
- tabs;
- carriage-return/newline input where relevant.

Serialization must remain pet-compatible and loadable by snip-it after save.

## F2. TUI display and search

Audit list and preview rendering for multiline commands.

Requirements:

- no terminal corruption;
- predictable truncation or wrapping;
- search can match text from intended fields without pathological rendering;
- selection remains usable with large multiline entries.

Do not redesign the TUI unless a concrete correctness issue requires a narrow fix.

## F3. Run semantics

Verify existing shell execution receives the stored command exactly as intended.

Do not change execution semantics to use a temporary script unless current shell-command execution cannot preserve multiline input. If a temporary script becomes necessary, that is a separate security-sensitive design decision and must be documented.

## F4. Clip and select semantics

Verify:

- clipboard receives full multiline content;
- `snp select --raw` emits exact stored content;
- `snp select --expanded` preserves line structure while replacing variables;
- shell-buffer insertion handles multiline snippets according to each shell's capabilities;
- output-file transport preserves trailing newlines.

## F5. Export and sync

Verify multiline content through:

- JSON export;
- CSV export, including quoting/newlines;
- encrypted sync serialization and merge;
- tombstone and local-only metadata behavior;
- backup and recovery.

Do not add a new export format in this pass.

# Workstream G: Size and Resource Limits

## G1. Define practical limits

The current library format is file-based, so unbounded file/editor/stdin ingestion can degrade TUI performance and sync payload size.

Inspect existing sync message limits and library-size assumptions.

Choose a documented command-size limit only if necessary. It should be high enough for short scripts and low enough to prevent accidental binary or multi-megabyte ingestion.

If no explicit limit is added, add tests for a reasonably large multiline snippet and document that snip-it is intended for commands and short scripts rather than arbitrary files.

## G2. Error quality

Oversize and invalid-input errors should name the source and limit without echoing content.

# Workstream H: Testing

## H1. CLI conflict tests

Cover all mutually exclusive source combinations and missing option values.

## H2. Source preservation tests

For each source mode, test:

- no trailing newline;
- one trailing newline;
- multiple trailing newlines;
- blank internal lines;
- tabs;
- Unicode;
- quotes and backslashes;
- shell metacharacters;
- leading hyphen;
- variable placeholders.

## H3. File tests

Cover:

- regular file;
- empty file;
- directory;
- missing file;
- permission denied where portable;
- invalid UTF-8;
- symlink according to chosen policy;
- size limit.

## H4. Editor tests

Use a deterministic fake editor executable to test:

- successful content write;
- empty content;
- nonzero exit;
- arguments with spaces;
- temp-file permissions;
- cleanup after success and failure;
- command content absent from logs/errors.

## H5. PTY multiline tests

Add serialized PTY tests for:

- multiline entry completion;
- multiline cancellation;
- terminal restoration;
- subsequent shell usability;
- `snp select` round trip of the created snippet.

## H6. Cross-platform matrix

Ensure Windows builds and noninteractive source tests compile and run. Gate Unix-only PTY/editor-permission assertions appropriately.

## H7. Full validation

Run:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
```

# Workstream I: Documentation

Update:

- README command overview;
- USER_GUIDE creation section;
- `snp new --help` examples;
- pet compatibility matrix;
- architecture inventory;
- stream policy where stdin/TTY ownership matters;
- AGENTS test instructions.

Document:

- all source modes and conflicts;
- exact newline policies;
- editor resolution;
- temp-file security;
- no execution during creation;
- file size/UTF-8 restrictions;
- multiline behavior in run, clip, select, export, and sync.

# Implementation Order

1. Audit current creation and Release 2A source abstraction.
2. Finalize source conflicts and preservation policy.
3. Implement `--from-file`.
4. Implement `--editor` with secure temp files.
5. Implement or tighten `--multiline` terminal entry.
6. Validate shared metadata and persistence behavior.
7. Add end-to-end multiline compatibility tests.
8. Add PTY/editor/file tests.
9. Reconcile documentation.
10. Run full workspace validation.

# Definition of Done

Release 2B is complete when:

1. All four creation sources are explicit and mutually coherent.
2. Existing positional and prompt-based `snp new` behavior remains unchanged.
3. Valid UTF-8 command text is preserved according to documented source policies.
4. Editor temp files are private and reliably removed.
5. File ingestion is read-only, bounded, and path-safe.
6. Multiline snippets round-trip through TOML, TUI, run, clip, select, export, backups, and sync.
7. Cancellation and source failures leave libraries unchanged.
8. PTY and fake-editor tests cover terminal and cleanup behavior.
9. Documentation is precise and does not overpromise arbitrary-file or script-runner behavior.

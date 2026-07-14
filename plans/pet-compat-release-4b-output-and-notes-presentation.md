# Release 4B Plan: Output and Notes Presentation

## Purpose

Expose the existing Pet-compatible `output` field consistently across preview, editing, structured export, and optional search without changing command execution or automatically capturing runtime output.

Release 3 import and diagnostics already preserve and report `output`. Release 4B turns that preserved data into a usable first-class presentation surface while keeping source compatibility and serialization semantics explicit.

This document is intended for implementation-agent handoff. Inspect the current `Snippet` model, serde aliases, TUI preview, edit workflow, list/search formats, import reports, and synchronization model before editing.

## Product Invariants

1. Existing `output` values round-trip unchanged.
2. Commands are never executed to populate or refresh output.
3. Runtime stdout/stderr is never captured automatically.
4. Default text output and TUI layouts must not become noisy for snippets without output.
5. Machine-facing schemas must be versioned or changed additively.
6. Pet-compatible source files remain readable without migration.
7. A future `notes` abstraction must not silently reinterpret existing data.

## Terminology Decision

Before implementation, decide whether the user-facing label remains `output` or whether a broader `notes` label is introduced.

Preferred Release 4 policy:

- preserve the serialized field name `output`;
- present it as “Output / Notes” only where additional context is useful;
- do not add a second persisted notes field in this release;
- do not rewrite imported `output` values;
- document that the field is descriptive metadata, not automatically captured execution output.

If a `notes` alias is added to CLI flags, it must map losslessly to the canonical `output` field and reject simultaneous conflicting values.

## Workstream A: Audit Existing Field Semantics

Inspect:

- `Snippet.output` type and defaults;
- canonical and legacy/Pet aliases;
- import preservation behavior;
- edit behavior;
- JSON/CSV output;
- TUI preview and detail panels;
- sync serialization;
- backup/export/import round trips;
- any code that assumes output is single-line.

Add regression tests for exact preservation of:

- empty output;
- single-line text;
- multiline text;
- tabs and trailing spaces;
- CRLF;
- Unicode;
- quotes and backslashes;
- text resembling shell commands or secrets.

## Workstream B: Shared Presentation Model

Avoid formatting the field independently in each command. Add a shared view/presentation helper that can provide:

- whether output is present;
- a short single-line summary;
- full multiline content;
- line count and truncation metadata;
- safe terminal rendering without execution or escape-sequence interpretation.

Sanitize terminal control sequences for human display while preserving raw values in storage and structured output.

Do not redact arbitrary content by mutating the stored field. Privacy controls belong at presentation/report boundaries.

## Workstream C: TUI Preview

Expose output in the existing snippet preview/details area.

Required behavior:

- hidden or collapsed when empty;
- clearly separated from command text;
- multiline scrolling where needed;
- no terminal escape interpretation;
- no variable expansion;
- no syntax that implies the content is live process output;
- layout remains usable on small terminals;
- selection, filtering, and terminal restoration remain unchanged.

Consider a toggle for full output when the preview area is constrained. Do not introduce mandatory extra keystrokes for snippets without output.

Add PTY or deterministic UI-state tests for:

- output absent;
- short output;
- multiline output;
- Unicode/control-sequence content;
- narrow terminal dimensions.

## Workstream D: Editing

Extend existing edit surfaces so users can inspect and modify output metadata.

Possible CLI additions, depending on the current edit hierarchy:

```text
snp edit --output <text>
snp edit --output-stdin
snp edit --clear-output
```

or an interactive field in the existing editor/TUI.

Requirements:

- reuse exact-source ingestion where practical;
- preserve multiline text exactly;
- avoid mixing command stdin with output stdin ambiguously;
- conflicts are enforced by clap;
- cancellation leaves the snippet unchanged;
- atomic save and backup behavior remain intact;
- editing output does not update local usage count;
- editing output updates normal metadata only according to existing policy.

Do not implement automatic runtime capture.

## Workstream E: Search

Add optional output-aware search without changing the default search corpus.

Possible surface:

```text
--search-output
--fields description,command,tags,output
```

Preferred behavior:

- default remains current fields only;
- output search is explicitly enabled;
- fuzzy scoring records which field matched;
- displayed highlights cannot corrupt terminal state;
- large multiline output is bounded for scoring;
- exact command ranking remains unchanged when output search is off.

If field-aware search is already available, extend it rather than adding a parallel flag system.

## Workstream F: Structured Output and Export

Ensure `output` is represented consistently in:

- `list --json`;
- `list --csv`;
- import reports where appropriate;
- any dedicated export command;
- backups;
- sync payloads.

JSON should preserve the full string exactly.

CSV must quote multiline values correctly and remain parseable by a standard CSV reader. Add round-trip tests rather than string-fragment assertions.

Avoid changing established JSON field names. If output is currently omitted, add it additively and document the schema change.

Human text formats should include output only when explicitly requested or in detail/preview mode.

## Workstream G: Notes Alias Decision

If introducing a `notes` user-facing alias:

- keep one canonical storage field;
- accept either `--output` or `--notes`, not both;
- ensure help text explains the mapping;
- preserve Pet export compatibility;
- do not emit duplicate fields;
- add de/serialization alias tests;
- define whether structured JSON exposes `output`, `notes`, or both.

Preferred: retain `output` in storage and structured schemas, use “notes” only as explanatory UI terminology.

## Workstream H: Security and Privacy

Treat output metadata as untrusted text.

Required protections:

- no `eval`, shell execution, or interpolation;
- no ANSI/OSC escape interpretation in terminal previews;
- no automatic inclusion in logs;
- no diagnostic dumps of full content on parse/render failure;
- report-file permissions remain private;
- shell-selection stdout remains command-only and never includes output;
- clipboard and run paths remain command-only unless a separate explicit feature is approved.

Add sentinel tests containing ANSI sequences, OSC hyperlinks, API-key-like strings, command substitutions, and multiline shell fragments.

## Workstream I: Tests

Required integration matrix:

1. Pet import preserves output exactly.
2. Doctor reports presence without leaking full content by default.
3. TUI preview renders safe text and restores terminal state.
4. Edit sets, replaces, clears, and cancels output atomically.
5. JSON preserves output exactly.
6. CSV parses and reconstructs multiline output.
7. Optional output search finds expected snippets.
8. Default search ordering/results are unchanged when output search is off.
9. Select/run/clip emit or act on command only.
10. Backup and sync preserve output.
11. Legacy files without output remain unchanged.
12. Control sequences are neutralized in human rendering.
13. Notes alias, if implemented, has conflict and round-trip tests.

## Documentation

Update:

- README;
- USER_GUIDE;
- PET_COMPATIBILITY;
- architecture inventory;
- CLI stream policy;
- import/doctor documentation;
- CHANGELOG.

State explicitly:

- output is stored metadata;
- it is not automatically captured;
- default search does not include output unless requested;
- raw command selection never includes output;
- storage and structured schema use the canonical `output` field.

## Validation Commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration -- output notes csv json search_output edit_output
cargo test --test pty_integration -- --test-threads=1
```

## Completion Criteria

Release 4B is complete only when:

- existing output values are exactly preserved;
- preview and editing are usable and safe;
- JSON/CSV behavior is defined and tested;
- output-aware search is opt-in;
- command-only run/clip/select semantics are unchanged;
- no automatic output capture exists;
- documentation clearly distinguishes metadata from runtime output.

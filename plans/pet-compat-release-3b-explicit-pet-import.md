# Release 3B Plan: Explicit Pet Import Workflow

## Purpose

Add a first-class, diagnostic Pet import command that safely migrates existing Pet snippet files into snip-it named libraries without modifying the source file or relying only on permissive implicit loading.

This plan builds on Release 3A's structured variable diagnostics and the existing Pet-compatible TOML reader. The importer should provide migration safety, explicit reporting, duplicate handling, normalization visibility, and atomic destination writes while preserving snip-it's native named-library model.

## Proposed CLI

```text
snp import pet <path>
```

Suggested options:

```text
--library <name>
--merge
--replace
--dry-run
--strict
--report human|json
--report-file <path>
```

The exact clap layout should follow current repository conventions.

## Default Behavior

By default, the importer should:

1. read one Pet TOML file;
2. leave the source untouched;
3. validate and analyze all entries;
4. create a new named library;
5. choose a deterministic safe destination name when `--library` is omitted;
6. fail atomically if the destination already exists or any fatal error occurs;
7. emit a concise human-readable report to stderr or the normal human output stream without contaminating machine-readable stdout;
8. return success only after the complete destination library is durably saved.

## Non-Goals

- Edit the source Pet file.
- Import arbitrary directories recursively in this release.
- Add GitHub/GitLab/Gist synchronization.
- Rewrite shell startup files.
- Guarantee preservation of unknown fields Pet itself does not define.
- Merge snip-it-only metadata back into Pet source files.
- Begin optional external-library indexing from Release 4.

## Workstream A: Import Domain Model

Create an import service independent of CLI printing.

Suggested types:

```rust
pub struct PetImportOptions {
    pub source: PathBuf,
    pub destination_library: Option<String>,
    pub mode: ImportMode,
    pub strict: bool,
    pub dry_run: bool,
}

pub enum ImportMode {
    Create,
    Merge,
    Replace,
}

pub struct PetImportReport {
    pub source: PathBuf,
    pub destination: Option<String>,
    pub total_entries: usize,
    pub imported: usize,
    pub skipped: usize,
    pub duplicates: Vec<ImportDuplicate>,
    pub diagnostics: Vec<CompatibilityDiagnostic>,
    pub normalizations: Vec<NormalizationRecord>,
}
```

The importer must return data, not print from the parsing layer.

## Workstream B: Source Loading and Detection

### B1. Read safely

Requirements:

- bounded file size;
- valid UTF-8 unless the current TOML layer supports otherwise;
- reject directories and special files;
- define symlink behavior consistently with `--from-file`;
- no source modification;
- no command execution or variable expansion.

### B2. Detect Pet structure

Recognize canonical Pet structures and supported historical variants already accepted by snip-it.

Record:

- canonical lowercase `[[snippets]]`;
- legacy capitalization or aliases;
- missing optional fields;
- unknown fields;
- malformed entries;
- duplicated tables or keys where the TOML parser permits detection.

### B3. Preserve command text

Import must preserve commands semantically, including multiline text, whitespace, variable syntax, shell metacharacters, and choice placeholders from Release 3A.

## Workstream C: Entry Conversion

Convert each source entry into the native `Snippet` representation through one centralized mapping function.

Map at minimum:

- `description`;
- `command`;
- `tag`/tags according to current compatibility rules;
- `output`;
- supported variables as command syntax.

Generate snip-it-native fields through existing constructors and persistence policy:

- IDs;
- timestamps;
- sync metadata defaults;
- folders/favorites defaults;
- deletion state.

Do not invent source provenance fields unless separately approved.

## Workstream D: Duplicate Policy

Define duplicates explicitly. Analyze at least:

- same command and same description;
- same command with different description;
- same description with different command;
- exact duplicate source entries;
- destination collisions during merge.

Default create mode should preserve non-identical entries and report exact duplicates according to a documented policy.

Merge mode must be deterministic and non-destructive. Prefer skipping exact duplicates and importing distinct records rather than silently overwriting.

Replace mode must require an explicit existing destination and should create a backup before atomic replacement.

## Workstream E: Destination Semantics

### E1. New library default

When no library name is supplied, derive a sanitized candidate from the source filename and resolve collisions deterministically, or fail with a clear suggestion if repository policy prefers explicit naming.

### E2. Atomicity

Build and validate the complete destination in memory before writing.

For create:

- fail if destination exists;
- write a temporary file in the destination directory;
- fsync/rename according to current save helpers;
- update library-manager metadata only after durable file creation, or perform rollback on metadata failure.

For merge/replace:

- load existing destination;
- create normal backup;
- apply changes in memory;
- save atomically;
- leave existing library unchanged on failure.

### E3. Dry run

`--dry-run` performs all parsing, diagnostics, duplicate detection, naming, and conversion without mutating files, library metadata, backups, or sync state.

## Workstream F: Strict and Permissive Modes

Permissive default:

- import valid entries;
- report recoverable diagnostics;
- skip entries that cannot be represented safely;
- fail only on source-level or destination-level fatal errors.

Strict mode:

- any error-severity compatibility diagnostic aborts the entire import;
- no destination mutation occurs;
- warnings remain visible but do not necessarily abort unless documented.

Define stable diagnostic severity and codes for Release 3C reuse.

## Workstream G: Reporting

### Human report

Include:

- source and destination;
- total entries;
- imported/skipped counts;
- duplicates;
- malformed entries;
- normalized fields;
- choice-variable detection;
- preserved output fields;
- unsupported concepts;
- whether any mutation occurred.

### JSON report

Provide a versioned schema suitable for automation.

Machine-readable stdout must contain only JSON when selected. Human diagnostics go to stderr.

If `--report-file` is supported, apply safe overwrite policy and atomic writing.

## Workstream H: Security and Privacy

- Never execute imported commands.
- Never expand variables during import.
- Avoid logging full command bodies at normal log levels.
- Error excerpts should be bounded and redact nothing automatically unless a clear secret policy exists; prefer entry indices and field names over dumping commands.
- Do not follow source includes or external references.
- Do not contact sync servers automatically.

## Workstream I: Tests

### Fixture corpus

Add fixtures for:

- canonical Pet TOML;
- tags and output fields;
- multiline commands;
- required/default/choice variables;
- unknown fields;
- malformed entries;
- exact and semantic duplicates;
- invalid TOML;
- empty files;
- large files;
- legacy forms already supported.

### Integration tests

Verify:

- default new-library import;
- explicit destination;
- destination collision;
- merge;
- replace with backup;
- dry run causes zero mutation;
- strict atomic failure;
- permissive partial import;
- JSON report validity;
- exact command preservation;
- source file unchanged;
- no sync side effects;
- imported library works with list/select/run/clip/export.

### Failure injection

Where feasible, test failure between destination file write and library-manager update to prove rollback or recoverability.

## Documentation

Update:

- README migration section;
- USER_GUIDE import chapter;
- `docs/PET_COMPATIBILITY.md`;
- CLI/architecture docs;
- exit-code and stream policy;
- CHANGELOG;
- shell migration examples.

Document merge and replace semantics precisely.

## Acceptance Criteria

Release 3B is complete when:

- `snp import pet` creates a native named library safely;
- source files are untouched;
- dry-run and strict modes are genuinely non-mutating and atomic;
- duplicate and normalization policies are deterministic;
- human and JSON reports are tested;
- imported commands and metadata survive native workflows;
- no command execution, expansion, or automatic sync occurs;
- full workspace validation passes.

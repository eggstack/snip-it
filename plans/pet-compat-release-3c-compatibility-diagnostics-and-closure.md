# Release 3C Plan: Pet Compatibility Diagnostics and Release Closure

## Purpose

Add a reusable compatibility-diagnostic surface and close Release 3 after variable-choice support and explicit Pet import have landed.

This track should let users inspect Pet files and the current snip-it environment before migration, understand exactly what will be preserved or normalized, and obtain machine-readable diagnostics without mutating data.

It also serves as the integration and closure pass for Release 3A and 3B.

## Proposed CLI

```text
snp doctor --pet-file <path>
snp doctor --compatibility
```

Possible additive options:

```text
--strict
--report human|json
--library <name>
--check-shell bash|zsh|fish
```

Use current clap conventions and avoid colliding with any existing `check` or diagnostic commands.

## Goals

1. Inspect Pet files without importing or modifying them.
2. Reuse the same parser, diagnostics, and duplicate logic as `snp import pet`.
3. Report malformed TOML, unsupported syntax, duplicate records, normalization, output-field behavior, and metadata caveats.
4. Audit the installed snip-it compatibility surface and shell integrations.
5. Provide stable human and JSON output contracts.
6. Validate Release 3 end to end and prevent divergence between doctor, import, and runtime parsing.

## Non-Goals

- Automatically repair source files.
- Execute imported or diagnosed commands.
- Install shell integrations.
- Contact sync servers by default.
- Add ranking, usage metadata, external libraries, or Release 4 features.
- Add automatic post-mutation sync from Release 5.

## Workstream A: Shared Diagnostic Model

Create or finalize a shared diagnostic model used by:

- choice-variable parsing;
- Pet import;
- doctor file analysis;
- future migration tooling.

Suggested shape:

```rust
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

pub struct CompatibilityDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub entry_index: Option<usize>,
    pub field: Option<String>,
    pub span: Option<SourceSpan>,
    pub suggestion: Option<String>,
}
```

Requirements:

- stable machine-readable codes;
- bounded excerpts;
- deterministic ordering;
- no direct printing from parser layers;
- no command execution or expansion;
- reusable severity policy for strict import.

## Workstream B: Pet File Doctor

### B1. Read-only analysis

`snp doctor --pet-file <path>` must perform all applicable import analysis without creating a destination library.

It should report:

- TOML parse status;
- recognized snippet table form;
- entry count;
- missing required fields;
- field type mismatches;
- duplicate commands/descriptions;
- invalid or unusual tags;
- `output` field preservation;
- required/default/choice variables;
- malformed placeholders;
- unknown fields;
- command-text preservation risks;
- normalization that import would apply;
- unsupported concepts;
- likely destination naming conflicts where determinable.

### B2. Exit policy

Define a stable contract, for example:

- 0: no error-severity diagnostics;
- 1: operational failure such as unreadable path;
- 2 or another documented code: compatibility errors detected.

Do not introduce a broader exit taxonomy without reconciling `docs/CLI_EXITCODE_STREAM_POLICY.md`.

### B3. Strict mode

`--strict` should elevate designated warnings or treat any error diagnostic as a non-success result. It must not mutate anything.

## Workstream C: Installed Compatibility Audit

`snp doctor --compatibility` should inspect the current installation and configuration without exposing secrets.

Possible checks:

- active binary/version;
- config and library directories readable/writable;
- primary library resolution;
- canonical Pet TOML loading;
- Release 1 `snp select` availability;
- Release 2 acquisition flags;
- Release 3 choice-variable parser availability;
- shell-init generation for Bash/Zsh/Fish;
- optional syntax validation when shells are installed;
- editor configuration parseability;
- known legacy paths;
- sync configuration presence without contacting servers unless explicitly requested.

This should be a compatibility audit, not a general system-health framework.

## Workstream D: Human Output

Human output should be concise but actionable.

Recommended grouping:

```text
Summary
Errors
Warnings
Normalizations
Supported features
Suggested next command
```

For file analysis, recommend an exact import command based on findings, such as:

```text
snp import pet <path> --dry-run --report json
```

Avoid dumping full command bodies by default. Use entry indices, descriptions, fields, and bounded excerpts.

## Workstream E: JSON Output

Define a versioned schema containing:

- report schema version;
- tool version;
- analysis mode;
- source metadata;
- summary counts;
- diagnostics;
- detected capabilities;
- normalization preview;
- recommended actions;
- mutation flag fixed to false for doctor.

When JSON is requested:

- stdout contains only JSON;
- diagnostics/logging do not contaminate stdout;
- ordering is deterministic where arrays represent source order;
- tests validate against representative fixtures.

## Workstream F: Import/Doctor Consistency

Doctor and importer must use the same underlying analysis service.

Add invariants:

- doctor diagnostics for a file equal import dry-run diagnostics for the same options;
- duplicate counts match;
- normalization previews match actual import behavior;
- choice-variable detection matches runtime parser behavior;
- strict doctor and strict dry-run fail on the same error classes.

Do not maintain separate parsers or duplicate policies.

## Workstream G: Release 3 Integration Matrix

Validate combinations across Release 3A and 3B:

1. Pet file with required/default variables.
2. Pet file with valid choice variables.
3. Pet file with malformed choice variables.
4. Duplicate entries plus output fields.
5. Multiline commands and exact whitespace.
6. Unknown metadata.
7. Permissive import.
8. Strict import rejection.
9. Doctor human report.
10. Doctor JSON report.
11. Imported library raw selection.
12. Imported library expanded choice selection.
13. Backup/export/sync preservation after import.

## Workstream H: Security and Privacy Review

Confirm:

- doctor never mutates source, destination, config, or library state;
- import dry-run remains non-mutating;
- no commands are executed;
- no variables are expanded;
- no full command bodies appear in ordinary logs;
- JSON output includes command excerpts only if explicitly designed and documented;
- source paths are handled safely;
- report-file output, if supported, is atomic and does not follow unsafe symlinks;
- compatibility audit does not print tokens, server secrets, or encryption material.

## Workstream I: Tests

### Unit tests

Cover:

- diagnostic ordering;
- severity mapping;
- stable codes;
- span/excerpt generation;
- strict-mode classification;
- JSON serialization;
- redaction/privacy helpers;
- recommendation generation.

### Integration tests

Cover:

- valid file exit 0;
- malformed TOML operational/compatibility status;
- compatibility errors;
- warnings-only behavior;
- strict mode;
- human report content;
- JSON-only stdout;
- no mutation of source/config/libraries;
- doctor/import dry-run equivalence;
- installed compatibility audit;
- shell syntax checks when available.

### Golden reports

Use stable snapshots or structured assertions for representative reports. Avoid brittle assertions on timestamps, absolute temporary paths, or nondeterministic IDs.

### PTY tests

Where Release 3A uses an interactive choice selector, retain PTY tests for selection and cancellation. Doctor itself should remain noninteractive unless explicitly requested.

## Workstream J: Documentation

Update:

- README migration quick start;
- USER_GUIDE migration and doctor sections;
- `docs/PET_COMPATIBILITY.md`;
- architecture inventory;
- CLI exit/stream policy;
- import and diagnostic architecture docs;
- CHANGELOG;
- roadmap status.

Include a recommended migration workflow:

```bash
snp doctor --pet-file ~/.config/pet/snippet.toml
snp import pet ~/.config/pet/snippet.toml --dry-run
snp import pet ~/.config/pet/snippet.toml --library pet-import
snp shell init zsh   # inspect and source manually if desired
```

## Workstream K: Full Validation

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration -- pet_import
cargo test --test integration -- doctor
cargo test --test integration -- variable_choice
cargo test --test pty_integration -- --test-threads=1
```

Confirm normal CI across supported platforms and account for all ignored tests.

## Release 3 Closure Criteria

Release 3 is complete only when:

- valid Pet choice variables work across shared expansion paths;
- existing variable semantics remain unchanged;
- explicit import is atomic, reportable, and source-preserving;
- doctor is read-only and shares analysis with import;
- human and JSON contracts are stable and tested;
- diagnostics are deterministic and actionable;
- imported libraries work with native list/select/run/clip/export/backup/sync workflows;
- security and privacy invariants hold;
- documentation and roadmap status match tested behavior;
- full validation and CI are green.

# Release 1A Plan: Pet Compatibility Contract and Regression Baseline

## Purpose

Establish the behavioral and test foundation required before adding shell-facing compatibility features.

This phase does not add user-facing selection or shell integration. Its purpose is to make the current behavior of `snip-it` explicit and mechanically protected so later refactoring can reuse search, selection, variable expansion, and terminal code without accidentally changing existing commands.

The implementation agent should inspect the repository as it exists at execution time. File and module names in this plan are architectural guidance, not assumptions that should override the current codebase.

## Relationship to the Roadmap

This is Release 1, Track A of `plans/pet-migration-compatibility-roadmap.md`.

Release 1 contains three ordered plans:

1. Release 1A: compatibility contract and regression baseline.
2. Release 1B: stable machine-facing `snp select` primitive.
3. Release 1C: generated Bash, Zsh, and Fish shell-buffer integration.

Release 1B may begin only after the baseline tests and compatibility contract from this phase are in place.

## Goals

1. Document exactly which pet behaviors snip-it supports, intentionally replaces, plans to add, or excludes.
2. Lock down existing CLI semantics that could be affected by selection refactoring.
3. Add representative format and variable fixtures.
4. Define stream, exit-code, terminal, and cancellation conventions for the future `select` command.
5. Identify the internal search/selection/expansion boundaries that Release 1B should reuse.
6. Ensure the project has a repeatable validation command set before implementation work begins.

## Non-Goals

- Add `snp select`.
- Add `snp shell init`.
- Modify existing command behavior.
- Change TUI keybindings or rendering.
- Change fuzzy ranking.
- Add new variable syntax.
- Add multiline creation.
- Add import commands.
- Add history capture.
- Add external selector dependencies.
- Add Gist or GitLab synchronization.
- Perform broad architectural cleanup unrelated to the compatibility track.

## Required Deliverables

### 1. Compatibility matrix

Add a durable document, preferably `docs/PET_COMPATIBILITY.md` or a clearly separated section in `USER_GUIDE.md`.

The matrix should include at least:

| Area | Pet behavior | Current snip-it behavior | Classification | Planned action |
| --- | --- | --- | --- | --- |
| TOML table and fields | Lowercase `[[snippets]]` and standard fields | Compatible | Supported | Preserve |
| Basic variables | `<name>`, `<name=default>` | Compatible | Supported | Preserve |
| Multiple-choice defaults | Pet-specific syntax | Not yet first-class | Planned | Release 3 |
| Interactive search | External selector | Native TUI | Intentional difference | Preserve native TUI |
| Direct execution | `pet exec` | `snp run` | Equivalent workflow | Preserve |
| Clipboard | `pet clip` | `snp clip` | Equivalent workflow | Preserve |
| Shell-buffer insertion | Shell functions around `pet search` | Missing first-class path | Planned | Release 1 |
| Previous-command capture | Shell helper examples | Missing maintained helper | Planned | Release 2 |
| Multiline creation | `pet new --multiline` | Not first-class | Planned | Release 2 |
| Multi-directory loading | Arbitrary TOML directories | Named libraries | Intentional difference | No replacement; optional future external sources |
| Sorting | Configurable sort modes | Native relevance behavior | Partial | Optional Release 4 modes |
| Output field | Displayed metadata | Preserved compatibility field | Partial | Release 4 presentation |
| Sync | Gist/GHE/GitLab file sync | Encrypted self-hosted record sync | Intentional difference | Preserve snip-sync |
| Auto-sync | Post-edit sync | Manual/scheduled behavior | Potential addition | Release 5 |

The document must explicitly state that compatibility does not mean cloning pet's architecture or defaults.

### 2. Existing behavior contract

Document or encode the current behavior of:

- `snp run`;
- `snp clip`;
- `snp search`;
- `snp new`;
- `snp list`;
- library selection and primary-library resolution;
- variable expansion;
- cancellation;
- TUI startup and teardown;
- command execution shell selection;
- clipboard error handling;
- serialization and metadata preservation.

Where possible, make the contract executable through tests instead of prose alone.

### 3. Representative fixtures

Add fixtures under the repository's established test-fixture location. Include:

- canonical pet lowercase TOML;
- older snip-it uppercase table/field aliases;
- tags, empty tags, and multiple tags;
- an `output` field containing multiline text;
- required variables;
- single default variables;
- escaped literal angle brackets;
- duplicate descriptions;
- duplicate commands;
- multiline command data;
- Unicode command and description text;
- snip-it-only metadata such as IDs, folders, favorite state, timestamps, and sync state;
- deleted/tombstoned snippets if represented in local files.

Fixtures should be small enough to understand but broad enough to protect compatibility-sensitive parsing and serialization.

### 4. CLI stream and exit-code policy

Add an internal design note or public contributor document defining the future machine-facing contract.

Recommended conventions:

- Exit `0`: a selection or requested operation completed successfully.
- A dedicated nonzero cancellation code: user cancelled an interactive selection without an operational failure.
- Other nonzero codes: invalid input, configuration/load failure, TTY failure, serialization failure, or internal error.
- Stdout: machine-facing result payload only.
- Stderr: warnings, diagnostics, and human-facing errors.
- TUI/prompts: controlling terminal when needed, not captured stdout.

The exact cancellation code should be chosen after auditing existing exit conventions and dependencies. Avoid selecting a code already used for another stable purpose.

The policy should state that cancellation is not logged as an error and should not print a payload.

### 5. Internal architecture inventory

Produce a concise developer note describing where the current implementation performs:

- command parsing;
- library resolution;
- snippet loading;
- filtering/fuzzy matching;
- TUI selection;
- variable parsing and prompting;
- command execution;
- clipboard output;
- terminal initialization/restoration;
- tracing/logging initialization.

For each boundary, record whether Release 1B should:

- reuse directly;
- extract a shared service;
- wrap without refactoring;
- leave untouched.

This inventory should prevent Release 1B from copying logic into a new command.

## Workstream A: Repository and Test Audit

### A1. Inventory current CLI hierarchy

Inspect the clap command definitions and command dispatch.

Record:

- root command and aliases;
- argument parsing types;
- how `run`, `clip`, and `search` enter the TUI;
- current command return types;
- current error-to-exit-code mapping;
- where stdout and stderr are written;
- how logging is initialized.

Do not refactor during the audit unless a tiny change is required to make behavior testable.

### A2. Inventory current TUI lifecycle

Identify:

- alternate-screen use;
- raw-mode enable/disable;
- cursor state changes;
- event-loop ownership;
- signal handling;
- cleanup guards;
- panic behavior;
- cancellation key behavior;
- selected-result return type.

Add regression coverage around cleanup before changing this code in Release 1B.

### A3. Inventory variable expansion

Identify whether run and clip use one shared path or duplicate logic.

Record:

- parser location;
- prompt implementation;
- default handling;
- escaping rules;
- cancellation behavior;
- multiline behavior;
- whether expanded output can already be returned without execution or clipboard use.

Release 1B should reuse the existing semantics rather than introducing a second expander.

## Workstream B: Regression Test Expansion

### B1. CLI contract tests

Use the repository's existing integration-test style and process-spawning helper.

Cover:

- `snp --help` and existing subcommand discovery;
- `snp new` noninteractive and interactive-compatible paths already supported;
- `snp list` human, JSON, and CSV output where present;
- `snp search` existing result/inspection semantics;
- invalid library handling;
- empty library behavior;
- primary-library resolution;
- explicit `--library` resolution;
- malformed TOML handling;
- config-root environment overrides.

Tests should isolate config/data using a temporary XDG root and must not read the developer's real snippets.

### B2. Serialization tests

Verify:

- canonical writes use lowercase pet-compatible names;
- older aliases remain readable;
- read/write preserves supported fields;
- snip-it metadata is preserved;
- multiline command and output values round-trip;
- Unicode round-trips;
- atomic write/backup behavior remains intact where currently guaranteed.

Do not normalize more data than the current implementation already normalizes.

### B3. Variable behavior tests

Cover:

- required variable substitution;
- default acceptance;
- default replacement;
- repeated variable names if supported;
- escaped angle brackets;
- adjacent variables;
- variables inside quoted shell text;
- cancellation;
- empty user input;
- Unicode values;
- multiline snippets containing variables.

These tests should describe current semantics, not desired future semantics.

### B4. TUI lifecycle tests

Where direct unit testing is impractical, use a pseudo-terminal test harness on Unix.

Verify at least:

- normal cancellation restores terminal mode;
- successful selection restores terminal mode;
- empty results restore terminal mode;
- load error before event loop does not leave raw mode enabled;
- variable-prompt cancellation restores terminal mode;
- no alternate-screen escape state leaks after exit.

If the existing project lacks a pseudo-terminal test dependency, select a small, maintained option after reviewing MSRV and cross-platform impact. Unix-specific tests should be `cfg`-gated; Windows must continue to compile and run its existing suite.

### B5. Existing behavior snapshots

Avoid brittle full-screen snapshots unless the repository already uses them. Prefer semantic assertions over exact terminal-frame matching.

Good snapshot targets include:

- human-readable list row shape;
- JSON field names;
- CSV headers;
- error message categories;
- compatibility-document examples.

Do not lock incidental color codes, terminal dimensions, or layout whitespace unless they are already contractual.

## Workstream C: Compatibility Documentation

### C1. Write the compatibility matrix

The matrix should be concise enough for users but specific enough for maintainers.

Each row should use one of these labels:

- Supported.
- Supported differently.
- Planned.
- Not planned.
- Under consideration.

Avoid vague claims of full compatibility.

### C2. Add migration positioning

Explain that ordinary pet TOML can be loaded directly, but creating a separate snip-it library is recommended when users want to retain snip-it-only metadata.

Explain that `snip-sync` is intentionally not a drop-in replacement for pet's Gist/GitLab backend configuration.

### C3. Add shell roadmap note

Document that shell-buffer insertion is planned and will be opt-in. Do not document commands that do not yet exist as if released.

A clearly marked roadmap/example section is acceptable.

## Workstream D: Release 1 Design Decisions

Before closing this phase, settle and record the following decisions for Release 1B/C.

### D1. Command name

Preferred:

```text
snp select
```

Alternative:

```text
snp search --select
```

Choose a dedicated command unless the current CLI architecture makes it clearly inferior. The dedicated command gives a cleaner stdout and exit-code contract.

### D2. Raw versus expanded terminology

Recommended semantics:

- `--raw`: emit the stored command exactly, including unresolved placeholders.
- `--expanded`: prompt through the existing variable UI and emit the resolved command.

Do not make both flags optional if that creates ambiguity. Choose and document one default. Raw is recommended for shell-buffer insertion because it preserves editability and avoids surprise prompts.

### D3. Cancellation code

Select one stable code and document it for shell adapters.

Shell functions should distinguish cancellation from operational failure so cancellation can silently preserve the existing buffer while errors remain visible.

### D4. Selection replacement semantics

Release 1C needs a default:

- replace the whole current buffer; or
- insert at the cursor.

For pet migration, replacing the whole buffer after using it as the query is recommended. An explicit insert variant may be added, but should not complicate the initial release.

### D5. Keybindings

Generated shell functions should be unbound by default.

An explicit `--bind` mode may install a documented conservative binding. Do not take over `Ctrl-R`, `Ctrl-S`, or other common bindings silently.

## Validation

Run the complete repository validation suite after changes.

Minimum expected commands:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --doc --workspace
cargo build --workspace --all-features
```

Also run targeted tests under isolated temporary configuration roots.

On supported systems, exercise current commands manually with:

- a canonical pet file;
- a legacy snip-it file;
- an empty library;
- variables;
- multiline content;
- cancellation.

If CI is unavailable or unreliable, record the exact local platforms and commands used. Do not claim cross-platform validation that was not performed.

## Acceptance Criteria

This plan is complete when:

1. A pet compatibility matrix exists and clearly separates supported, intentionally different, planned, and non-goal behavior.
2. Existing CLI semantics relevant to Release 1 are covered by integration tests.
3. Canonical pet and legacy snip-it fixtures exist.
4. Parsing and serialization tests protect lowercase canonical output and richer metadata preservation.
5. Variable behavior is covered before extraction or reuse.
6. TUI cancellation and terminal cleanup have regression coverage.
7. Stdout, stderr, cancellation, and exit-code policy is documented.
8. The current selection/expansion architecture has been inventoried for the next implementing agent.
9. Release 1B's command name, raw/expanded semantics, cancellation code, replacement behavior, and keybinding policy are recorded.
10. No existing user-facing behavior has changed.
11. The full validation suite passes, or any environmental limitation is documented with no hidden failures.

## Handoff Notes for Release 1B

The Release 1B agent should begin by reading:

- `plans/pet-migration-compatibility-roadmap.md`;
- this plan;
- the compatibility matrix;
- the architecture inventory;
- the new regression tests.

Release 1B should prefer extracting the smallest shared selection service necessary. It should not broaden scope into shell generation, history capture, parser extensions, or multiline creation.
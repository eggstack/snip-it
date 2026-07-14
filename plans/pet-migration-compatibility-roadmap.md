# Pet Migration Compatibility Roadmap

## Purpose

Extend `snip-it` so established `pet` users can move their existing snippet collections and shell workflows to `snp` with minimal friction, while preserving every intentional product distinction that already exists.

This is not a rewrite, clone, or full behavioral parity project. `snip-it` should remain an opinionated terminal-first snippet manager with its own native TUI, named libraries, Halloy-compatible themes, richer local metadata, encrypted self-hosted synchronization, premade libraries, structured export, recovery behavior, and integrated update path.

The compatibility objective is narrower and more practical:

1. Existing `pet` snippet data should import predictably.
2. The shell-buffer workflow that makes `pet` feel like an enhanced shell history should be available as an opt-in `snp` integration.
3. Common acquisition workflows such as saving the previous command and creating multiline snippets should be first-class.
4. Additional `pet` syntax and metadata should be understood where doing so is low-risk and useful.
5. Existing `snip-it` commands and defaults must not change semantics.

This document is intended for implementation-agent handoff. Each release should be implemented through focused follow-up plans and should begin with inspection of the current repository state rather than assuming file paths or internal APIs remain unchanged.

## Current Product Position

`snip-it` already covers and exceeds the central standalone snippet-manager feature set:

- Pet-compatible lowercase `[[snippets]]` TOML with `description`, `command`, `tag`, and `output` fields.
- Backward-compatible reading of older snip-it capitalization and table names.
- Required and default-valued variables using `<name>` and `<name=default>`.
- Native fuzzy-search TUI with Vim-oriented interaction.
- Run and clipboard workflows.
- Named libraries and a primary-library model.
- Richer local metadata such as IDs, folders, favorites, timestamps, and sync state.
- Client-side encrypted self-hosted synchronization.
- Premade libraries.
- Themes, structured JSON/CSV output, backups, recovery behavior, shell completions, and self-update support.

The most consequential remaining migration gap is not data format or basic search. It is `pet`'s shell-native selection workflow: searching with the current command buffer as the initial query and inserting the selected command back into Bash Readline, Zsh ZLE, or Fish without executing it.

That workflow permits review, editing, completion, redirection, piping, and normal shell-history recording. `snp run` and `snp clip` should remain unchanged; the compatibility layer should add a third, non-executing selection path designed for shell adapters.

## Product Invariants

Every release in this roadmap must preserve the following invariants.

### 1. Existing behavior is frozen unless separately approved

The semantics of these commands must not change as a side effect of compatibility work:

```text
snp new
snp list
snp run
snp clip
snp search
snp edit
snp library
snp premade
snp register
snp sync
snp cron
snp keybindings
snp update
snp completions
```

Existing TUI keybindings, default search behavior, variable prompting, library resolution, clipboard behavior, command execution, sync direction, serialization, configuration paths, and environment-variable behavior must remain compatible.

### 2. Compatibility features are additive and opt-in

Do not silently install shell bindings, rewrite shell startup files, change default keybindings, enable automatic synchronization, or alter a user's library layout.

Generated shell code should be inspectable and explicitly sourced or installed by the user.

### 3. Selection and execution remain distinct

A shell-buffer integration selects and emits command text. It must never execute the selected command.

`run` remains the direct-execution path. `clip` remains the clipboard path. The new selection primitive must not repurpose either command.

### 4. Native snip-it architecture remains primary

Do not introduce `fzf` or `peco` as required runtime dependencies. Do not replace the native TUI with an external selector abstraction. Do not replace named libraries with arbitrary directory scraping as the primary storage model.

### 5. Synchronization remains security-oriented

Do not add GitHub Gist, GitHub Enterprise Gist, or GitLab Snippet synchronization merely for `pet` parity. Those services expose a different trust, encryption, conflict-resolution, and account model.

The existing encrypted `snip-sync` design remains canonical.

### 6. Source compatibility is stronger than round-trip identity

Ordinary `pet` TOML should load without conversion. A dedicated importer should improve diagnostics and migration safety, but snip-it-only metadata cannot be expected to survive editing by software that does not understand it.

The project must document normalization and compatibility boundaries explicitly.

## Non-Goals

- Reproduce every `pet` configuration option.
- Support arbitrary external selector commands as a core abstraction.
- Preserve `pet`'s exact UI rendering or prompt text.
- Implement hosted plaintext sync backends.
- Automatically modify `.bashrc`, `.zshrc`, Fish configuration, or terminal settings.
- Automatically execute selected shell-buffer commands.
- Parse shell history files directly when shell-native APIs can provide safer data.
- Automatically capture command output.
- Replace snip-it libraries with directory scraping.
- Make usage telemetry or ranking metadata remotely synchronized without a separate design decision.
- Guarantee lossless shared editing of one file by both `pet` and `snip-it` when snip-it-only metadata is present.

## Release Sequence

## Release 1: Shell Selection Foundation

Release 1 closes the largest practical migration gap while minimizing changes to persistence and command semantics.

It consists of three implementation tracks:

### R1-A. Compatibility contract and regression baseline

Create a versioned compatibility matrix, document intentional differences, and add regression coverage for all existing command surfaces that the new work could accidentally disturb.

Required outputs:

- `pet` compatibility matrix in project documentation.
- Golden or integration tests for current stdout/stderr/exit-code behavior.
- Fixtures for canonical pet TOML, legacy snip-it TOML, variables, multiline data, and richer metadata.
- Explicit shell-integration invariants.
- A validation command set for implementation agents.

### R1-B. Stable machine-facing selection primitive

Add a new non-executing selection command, preferably:

```text
snp select
```

The command should reuse the existing TUI/search engine but provide a strict machine-facing contract:

- initial query support;
- optional library and tag constraints;
- raw or expanded command output;
- stdout containing only the selected payload;
- diagnostics and prompts kept off stdout;
- distinct cancellation behavior;
- no command execution;
- terminal restoration on success, error, and cancellation.

The exact final CLI surface should be confirmed against the current clap hierarchy, but a dedicated command is preferred over overloading human-facing `search` output.

### R1-C. Generated Bash, Zsh, and Fish integration

Add:

```text
snp shell init bash
snp shell init zsh
snp shell init fish
```

Generated functions/widgets should:

- use the current shell buffer as the initial query;
- invoke `snp select`;
- insert or replace the command buffer without execution;
- preserve the buffer on cancellation;
- place the cursor predictably;
- support raw and expanded variants;
- avoid default keybinding collisions unless `--bind` is explicitly requested.

Release 1 is complete only after pseudo-terminal or shell-level integration tests verify actual buffer behavior rather than testing generated strings alone.

## Release 2: Acquisition Ergonomics

Release 2 makes it easy to save commands discovered during normal shell work and to create short scripts without manually editing TOML.

### R2-A. Shell history capture helpers

Add maintained shell functions/widgets for:

- saving the current buffer;
- saving the previous accepted command;
- optionally choosing a history item through the shell's native history facilities.

The shell should pass command text to `snp` explicitly. The binary should not infer history-file formats.

Add a safe ingestion path such as:

```text
snp new --command-stdin
```

Requirements:

- exact input preservation;
- no evaluation;
- no command execution;
- no normal-level logging of captured command bodies;
- compatibility with existing description and tag prompts.

### R2-B. Multiline, stdin, file, and editor creation

Add coherent additive creation modes:

```text
snp new --multiline
snp new --command-stdin
snp new --from-file <path>
snp new --editor
```

All modes should enter the same validation and persistence pipeline as existing creation.

Temporary editor files must use restrictive permissions and reliable cleanup. Multiline commands must round-trip through TOML, search, run, copy, select, export, import, and sync.

## Release 3: Migration Fidelity

Release 3 makes migration explicit and diagnostic rather than relying only on permissive loading.

### R3-A. Pet multiple-choice parameter compatibility

Teach the variable parser to recognize valid `pet` multiple-default syntax and represent it internally as a variable with choices.

Existing `<name>` and `<name=default>` semantics must remain unchanged.

The native snip-it prompt may present a cleaner list UI; exact pet prompt behavior is not required.

Malformed syntax should generate actionable diagnostics rather than silent reinterpretation.

### R3-B. Explicit pet import workflow

Add:

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
--report json
```

Default behavior should create a new named library, leave the source untouched, and fail atomically.

The import report should include counts, duplicates, malformed entries, normalization, detected choice variables, preserved output fields, and unsupported concepts.

### R3-C. Compatibility diagnostics

Add a diagnostic surface such as:

```text
snp doctor --pet-file <path>
snp doctor --compatibility
```

Diagnostics should cover malformed TOML, duplicate commands/descriptions, invalid tags, unsupported placeholder syntax, output-field handling, and metadata round-trip caveats.

## Release 4: Retrieval and Metadata Polish

Release 4 improves behavior for users importing large long-lived collections.

### R4-A. Optional sorting and usage-aware ranking

Preserve fuzzy relevance as the default. Add explicitly requested modes such as:

```text
--sort relevance
--sort recent
--sort last-used
--sort most-used
--sort description
--sort command
--favorites-first
```

Last-used and use-count metadata should remain local-only unless separately approved.

Default ranking should not shift unexpectedly. If usage signals are used as tie-breakers, the effect must be bounded and covered by deterministic tests.

### R4-B. Output and notes presentation

Expose preserved `output` fields in preview, editing, structured export, and optional search.

Do not automatically capture command output.

A future `notes` abstraction may map to the pet-compatible `output` field, but serialization semantics must be specified before introducing it.

### R4-C. Optional externally managed libraries

Consider read-only indexing of externally managed pet-compatible TOML directories or files for users who keep snippets in dotfiles or project repositories.

This remains subordinate to named libraries and should not be implemented unless the migration evidence shows substantial demand.

A possible model:

```toml
[[external_libraries]]
name = "project"
path = "/path/to/repo/.snippets"
recursive = true
writable = false
```

This feature must not be required to complete the core migration roadmap.

**Status (2026-07-14): Deferred.** Gate 0 evaluation found zero user demand, a sufficient existing workflow (`snp import pet --merge`), and high implementation cost touching every mutation path. External library support remains on the roadmap as a future option if demand materializes. See `plans/pet-compat-release-4c-external-libraries.md` for the full deferral rationale and future interface design.

## Release 5: Synchronization Convenience

Add optional post-mutation synchronization for users accustomed to pet's `auto_sync` convenience.

Suggested configuration:

```toml
[settings.sync]
auto_sync = true
auto_sync_debounce_seconds = 2
auto_sync_failure = "warn"
```

Requirements:

- local writes commit first;
- remote failure never rolls back a successful local mutation;
- rapid mutations coalesce;
- interactive commands do not block indefinitely;
- existing encrypted protocol and conflict model are reused;
- manual and scheduled sync continue to behave unchanged;
- feature remains disabled unless explicitly enabled or current documented configuration already establishes otherwise.

## Cross-Cutting Architecture Requirements

### Shared selection service

Do not duplicate search, library resolution, filtering, or variable expansion logic inside shell-specific paths.

Extract or expose a shared service that can be used by:

- `run`;
- `clip`;
- `search`;
- the new `select` command;
- future import preview or history workflows.

The extraction must preserve existing behavior and should be guarded by regression tests before refactoring.

### Stdout discipline

Machine-facing commands must define strict stream ownership:

- stdout: selected payload or structured result only;
- stderr: diagnostics, warnings, and human-readable errors;
- controlling terminal: TUI rendering and interactive prompts where necessary.

Logs must not contaminate stdout.

Cancellation should have a stable exit code and no payload.

### Terminal lifecycle

All interactive paths must restore terminal state after:

- normal selection;
- cancellation;
- empty result;
- variable-prompt cancellation;
- I/O error;
- panic boundary where recoverable;
- signal handling where currently supported.

Pseudo-terminal tests should detect echo, alternate-screen, cursor, and raw-mode leakage.

### Exact text preservation

Commands may contain:

- single and double quotes;
- backslashes;
- pipes and redirects;
- semicolons and ampersands;
- command substitution syntax;
- Unicode;
- tabs;
- newlines;
- leading hyphens;
- trailing whitespace where representable.

Selection and shell insertion must treat command text as data, not code. Avoid `eval`, shell interpolation, and lossy escaping.

### Cross-platform scope

Release 1 shell-buffer integration targets Bash, Zsh, and Fish on Unix-like systems.

The core `snp select` command should remain portable and compile on Windows. PowerShell or Nushell buffer integrations may follow later through separate plans; their absence must not degrade current Windows behavior.

### Security

Compatibility workflows may surface commands containing credentials, tokens, or sensitive paths.

Requirements:

- do not log full selected or captured commands at normal log levels;
- do not persist temporary command files longer than necessary;
- use restrictive temporary-file permissions;
- do not execute imported, selected, or captured commands during migration;
- retain existing warning that snip-it is not a sandbox or secrets manager.

## Documentation Strategy

Documentation should distinguish three layers:

1. Native snip-it workflows.
2. Pet migration/import compatibility.
3. Optional shell-buffer integration.

Recommended additions:

- `docs/PET_MIGRATION.md` or a dedicated section in `USER_GUIDE.md`.
- Shell setup examples for Bash, Zsh, and Fish.
- Explanation of raw versus expanded insertion.
- Keybinding collision guidance.
- Data-format compatibility and metadata caveats.
- Explicit non-goals and intentional differences.
- Troubleshooting for cancellation, terminal restoration, shell startup, and missing TTYs.

README changes should remain concise and point to the full guide.

## Validation Strategy

Every release must run the existing repository validation suite plus release-specific tests.

Baseline validation should include, as applicable:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --doc --workspace
cargo build --workspace --all-features
```

Release 1 additionally requires:

- CLI contract tests for `snp select`;
- stdout/stderr/exit-code assertions;
- pseudo-terminal tests;
- Bash syntax and behavior tests;
- Zsh syntax and behavior tests;
- Fish syntax and behavior tests;
- cancellation and terminal-restoration tests;
- quoting and multiline fixtures;
- Windows compile and existing-test validation.

Later releases require parser fuzz/property tests, atomic import tests, temporary-file security checks, and sync-failure tests as appropriate.

## Roadmap Exit Criteria

The compatibility roadmap is complete when a representative pet user can:

1. Load or explicitly import an existing pet TOML collection.
2. Receive a clear compatibility report for anything ambiguous or unsupported.
3. Generate and source a Bash, Zsh, or Fish integration.
4. Start typing a query, invoke the integration, and insert a selected snippet into the current shell buffer without execution.
5. Choose raw placeholders or snip-it-expanded values.
6. Save the current or previous command as a snippet.
7. Create multiline snippets without manually editing TOML.
8. Use pet multiple-choice placeholders where present.
9. Continue using all existing snip-it libraries, TUI behavior, themes, encrypted sync, premade collections, export, recovery, and update features unchanged.

## Recommended Immediate Work

Implement Release 1 in three separate handoff plans:

1. Compatibility contract and regression baseline.
2. Stable machine-facing `snp select` primitive.
3. Generated Bash, Zsh, and Fish shell-buffer integration.

Do not begin Release 2 until Release 1 has been validated end to end in real shells and the machine-facing output contract is considered stable.
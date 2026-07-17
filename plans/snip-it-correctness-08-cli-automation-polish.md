# Phase 08: CLI and Automation Polish

## Purpose

Improve deterministic, noninteractive snippet retrieval and machine composition while preserving the product’s narrow role as a terminal snippet manager.

The existing interactive TUI, `select`, `search`, `run`, and `clip` workflows are useful for humans. Scripts, shell widgets, and agent harnesses also need exact lookup, stable output, and unambiguous exit behavior without opening a TUI.

## Preconditions

Begin after:

- storage and identity behavior are stable;
- public API boundaries are intentional;
- status/diagnostic JSON conventions exist;
- end-to-end command tests protect stdout/stderr behavior.

## Product constraints

This phase must not add:

- a workflow engine;
- arbitrary scheduling;
- remote command execution;
- plugin execution;
- command dependency graphs;
- shell emulation;
- general secrets management;
- implicit execution from machine-readable lookup.

The tool stores text and deliberately lets users select, print, copy, or execute it.

## Workstream A: Define command contracts

Audit and document the distinct purpose of:

- `list`;
- `search`;
- `select`;
- `get` or equivalent new deterministic command;
- `run`;
- `clip`;
- `edit`;
- `new`;
- import/export commands.

Recommended contract:

- `list`: enumerate many snippets, optionally filtered/sorted, no TUI.
- `search`: interactive fuzzy search/preview, no execution unless explicitly designed.
- `select`: interactive selection and command emission, used by shell integration.
- `get`: deterministic noninteractive retrieval by ID/exact field/query policy.
- `run`: interactive or exact selection followed by explicit shell execution.
- `clip`: interactive or exact selection followed by clipboard write.

Avoid aliases that make materially different behaviors indistinguishable.

## Workstream B: Add deterministic retrieval

Recommended surface:

```bash
snp get --id <uuid>
snp get --description-exact "Deploy service"
snp get --command-exact "kubectl get pods"
snp get --query kubectl --first
snp get --query kubectl --unique
snp get --library work --id <uuid>
```

Output modes:

```bash
snp get ... --raw
snp get ... --expanded
snp get ... --json
snp get ... --field command
snp get ... --field description
```

Rules:

- no TUI;
- no clipboard access;
- no execution;
- no network sync unless a separate explicit flag with foreground semantics is already supported and justified;
- exact match is exact under documented normalization;
- ambiguous lookup returns a distinct nonzero result and no arbitrary selection;
- `--first` is explicit and uses documented deterministic ordering;
- variable expansion prompts only when attached to a terminal and requested; noninteractive missing values fail clearly unless provided by flags/stdin mapping;
- raw output preserves stored command bytes according to the established contract;
- machine modes emit only requested data to stdout.

## Workstream C: Variable values for noninteractive expansion

Provide safe explicit input for variables, for example:

```bash
snp get --id ... --expanded --var host=example.com --var branch=main
snp run --id ... --var env=prod
```

Requirements:

- repeated `--var key=value`;
- reject duplicate conflicting assignments or define last-wins explicitly;
- distinguish missing required variable from defaulted variable;
- validate choice variables against allowed values unless an override policy exists;
- do not read arbitrary environment variables implicitly;
- optional explicit environment mapping must be named and documented;
- values are not logged;
- values containing shell metacharacters are inserted according to existing textual expansion semantics, not silently shell-escaped unless a separate mode is designed;
- help text warns that snippets execute through the configured shell exactly as expanded.

Machine-readable lookup must not prompt unexpectedly when stdin/stdout are part of a pipeline.

## Workstream D: Standardize exit codes

Define a public CLI exit-code policy distinct from hidden executor codes.

Recommended categories:

```text
0  success
1  general operational failure
2  CLI usage/argument error
3  not found
4  ambiguous match
5  user cancelled interactive selection
6  local persistence/validation failure
7  synchronization failure
8  execution failure, if run propagates child failure distinctly
```

Review existing behavior before assigning numbers to preserve compatibility where users may rely on it.

Requirements:

- document stable codes;
- all commands map typed outcomes consistently;
- cancellation is not reported as internal failure;
- shell integration can distinguish cancellation from error;
- child process exit behavior for `run` is explicit: either propagate child code where possible or map through a stable wrapper policy;
- hidden worker/executor codes remain internal and undocumented as public CLI contract.

## Workstream E: Audit stdout/stderr purity

Create a command-by-command output contract.

Rules:

- requested data goes to stdout;
- diagnostics go to stderr;
- JSON/CSV stdout contains no tracing, warnings, update notices, status warnings, or prompts;
- `select --output-file` does not also print the command unless documented;
- raw command output is not followed by an automatic newline unless the contract says so;
- errors never echo sensitive command content unnecessarily;
- cancellation emits no command bytes;
- detached auto-sync diagnostics never contaminate parent stdout;
- logging initializes in a way that respects machine modes.

Add a global or per-command machine-output guard if needed to suppress incidental presentation.

## Workstream F: Deterministic sorting and ambiguity

For lookup modes, define deterministic ordering using stable keys such as:

1. exact ID match;
2. exact normalized description;
3. exact command;
4. fuzzy score;
5. favorite preference if requested;
6. configured sort mode;
7. library name;
8. stable ID tie-breaker.

Do not rely on filesystem enumeration or hash-map iteration order.

`--unique` should succeed only if one result remains after documented matching. `--first` should make arbitrary selection intentional and reproducible.

## Workstream G: Library scoping

Clarify behavior when IDs are globally unique versus library-scoped.

- if IDs are globally unique, `--library` narrows but is not required;
- if IDs are library-scoped, require library when collision is possible;
- exact description/command may be ambiguous across libraries;
- machine JSON should include library identity;
- primary-library defaults should be explicit;
- `--all-libraries` should be named rather than implicit where behavior differs.

Use the identity contract from Phase 07.

## Workstream H: Shell integration

Review Bash, Zsh, and Fish integrations against the new deterministic surfaces.

Goals:

- continue using interactive `select` where a user expects a picker;
- use output files or safe command substitution according to shell constraints;
- preserve multiline commands;
- preserve cancellation semantics;
- avoid `eval` where insertion widgets can place text directly;
- clearly separate insert-at-prompt from execute-now behavior;
- quote temporary output paths safely;
- clean temp files;
- no secret logging;
- test syntax and behavior in each supported shell.

Do not automatically migrate shell integrations to `get` if interactive selection is their purpose.

## Workstream I: Exact run/clip/edit targeting

Where useful, allow deterministic selectors on commands that currently require TUI interaction:

```bash
snp run --id <uuid>
snp clip --id <uuid>
snp edit --id <uuid>
```

Requirements:

- selector options share one matching implementation;
- conflicting selectors are rejected by Clap;
- execution still requires explicit `run`;
- `clip` writes exactly the selected/expanded bytes;
- `edit` preserves ID and follows atomic persistence;
- usage tracking increments only after successful run/clip;
- sync pending generation records exactly once after edit;
- no TUI is opened when an exact selector resolves uniquely.

## Workstream J: JSON schemas and compatibility

Define JSON output for:

- list;
- get;
- status;
- validate/doctor;
- import/export reports.

Prefer versioned top-level objects for complex reports:

```json
{
  "schema": 1,
  "items": []
}
```

For simple `--field command`, plain text remains appropriate.

Rules:

- stable field names;
- explicit nullability;
- timestamps and IDs use stable formats;
- unknown future fields are additive;
- breaking schema changes require a version increment;
- no ANSI escapes;
- no human prose mixed into machine fields;
- errors may use a stable structured stderr mode if justified, but do not overcomplicate initial scope.

## Workstream K: Completion and help audit

Update shell completions and help text for:

- deterministic selectors;
- output modes;
- exit semantics;
- variable assignment;
- library scoping;
- ambiguity behavior;
- raw versus expanded meaning;
- commands that may execute shell text;
- commands that are guaranteed non-executing.

Help should make safe composition obvious without requiring the full user guide.

## Required tests

### Deterministic lookup

- ID exact success/not-found;
- exact description and command;
- ambiguity across libraries;
- `--unique` failure;
- `--first` deterministic result;
- stable ordering across repeated runs;
- raw byte preservation;
- expanded defaults/required/choice values;
- no prompt in noninteractive mode;
- JSON schema snapshots.

### Exit codes

- success;
- usage error;
- not found;
- ambiguous;
- cancellation;
- persistence failure;
- sync failure;
- child execution failure policy;
- shell integration handling.

### Output purity

- stdout exactly expected bytes;
- stderr contains diagnostics only;
- JSON/CSV no contamination;
- no ANSI in machine modes;
- no auto-sync warnings in stdout;
- multiline commands preserved;
- no unexpected trailing newline;
- cancellation emits zero bytes.

### Exact target operations

- run/clip/edit by ID;
- edit preserves ID;
- usage increments correctly;
- failed run does not count if that is current policy;
- pending generation increments once after edit;
- exact selectors avoid TUI.

### Shells

- generated Bash/Zsh/Fish syntax validation;
- selection insertion;
- cancellation;
- multiline content;
- spaces/Unicode in paths;
- no execute-on-insert regression.

## Documentation

Update:

- command reference;
- shell scripting examples;
- JSON schema/compatibility policy;
- exit-code table;
- raw versus expanded semantics;
- variable assignment safety;
- ambiguity and deterministic ordering;
- run/clip execution warnings;
- shell integration guide.

## Recommended commit sequence

1. Codify selector/match API and deterministic ordering.
2. Add `get` command with raw/plain output.
3. Add JSON and field output modes.
4. Add noninteractive variable assignment/expanded output.
5. Standardize CLI outcomes and exit mapping.
6. Audit stdout/stderr and machine-output guards.
7. Add exact selectors to run/clip/edit where justified.
8. Update shell integration and completions.
9. Add complete cross-platform command tests.
10. Reconcile documentation and compatibility notes.

## Exit criteria

Phase 08 is complete only when:

- deterministic non-TUI retrieval exists;
- ambiguous lookup never silently chooses unless `--first` is explicit;
- output ordering is stable;
- machine stdout is uncontaminated;
- raw and expanded behavior is precise and tested;
- noninteractive expansion never unexpectedly prompts;
- public exit-code policy is documented and consistently implemented;
- exact run/clip/edit targeting uses the shared selector logic;
- shell integration remains safe and cross-platform tested;
- JSON schemas have an explicit compatibility policy;
- no workflow-engine or remote-execution scope is introduced.

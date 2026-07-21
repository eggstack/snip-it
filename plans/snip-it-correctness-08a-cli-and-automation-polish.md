# Phase 08A: CLI and Automation Polish

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-08-cli-automation-polish.md
```

Begin after Phase 07A closes identity, validation, persistence, and transaction behavior. Phase 04A must have stable machine diagnostics and Phase 05A must protect output/process contracts. Baseline implementation commit: `ff506f5934957c4fd989224a6f0e0cf10f907567`.

## Purpose

Make `snp` predictable for scripts, shell integrations, and agent harnesses without weakening its terminal-first interactive workflows or expanding it into a workflow engine.

The phase adds deterministic noninteractive retrieval, shared exact selectors, stable output and exit contracts, explicit variable assignment, and safe composition. Retrieval never executes snippets. Execution remains an explicit `run` action.

## Required outcomes

1. A snippet can be retrieved deterministically without opening a TUI.
2. Ambiguous matches never silently select an item unless an explicit deterministic policy is requested.
3. `get`, exact `run`, exact `clip`, and exact `edit` share one selector implementation.
4. Raw and expanded output semantics are precise and byte-tested.
5. Noninteractive variable expansion never unexpectedly prompts.
6. Machine stdout is uncontaminated and stable.
7. Public CLI exit categories are documented and consistent.
8. JSON schemas use explicit compatibility rules.
9. Bash, Zsh, and Fish integrations preserve cancellation, multiline content, and insertion-versus-execution semantics.
10. No workflow engine, remote execution, plugin behavior, or implicit command execution is introduced.

## Non-goals

Do not add:

- multi-step command workflows;
- task scheduling;
- dependency graphs;
- remote shell execution;
- plugin execution;
- automatic shell escaping that changes stored snippet semantics;
- implicit environment-variable ingestion;
- machine lookup that contacts the sync server by default;
- execution from `get`, `list`, `status`, `validate`, `backup`, `restore --dry-run`, or import preview.

---

## Workstream A — Command contract inventory

Document and enforce the distinct purpose of existing and new commands:

```text
list     enumerate multiple records noninteractively
search   interactive fuzzy discovery/preview
select   interactive selection and text emission for shell integration
get      deterministic noninteractive retrieval; never executes
run      explicit shell execution
clip     explicit clipboard write
edit     explicit mutation through editor or exact fields
new      explicit creation
status   read-only local state projection
validate read-only local data validation
backup   read-only snapshot creation
restore  explicit local mutation after preview/validation
```

Create a command contract table containing:

- interactive/noninteractive;
- reads stdin;
- writes stdout;
- writes stderr;
- may prompt;
- may mutate local data;
- may access clipboard;
- may execute shell;
- may access network;
- may schedule auto-sync;
- machine-output modes;
- exit categories.

Commit the table to the user guide and use it as a test matrix.

---

## Workstream B — Shared selector and match model

Create one selector model used by all deterministic targeting:

```rust
pub struct SnippetSelector {
    pub id: Option<SnippetId>,
    pub description_exact: Option<String>,
    pub command_exact: Option<String>,
    pub query: Option<String>,
    pub library: LibraryScope,
    pub resolution: ResolutionPolicy,
}

pub enum ResolutionPolicy {
    Unique,
    First,
    All,
}

pub enum SelectionResult {
    One(SnippetMatch),
    Many(Vec<SnippetMatch>),
    NotFound,
    Ambiguous(Vec<SnippetIdentity>),
}
```

Rules:

- Clap rejects conflicting exact selectors unless a documented intersection mode exists;
- ID lookup is exact and does not use fuzzy scoring;
- exact description/command normalization is explicit;
- query matching uses the existing canonical ranking implementation;
- ordering is deterministic across process runs and platforms;
- library identity is included in all match results;
- ambiguity is typed;
- `First` is explicit and stable;
- `All` is allowed only for commands that can safely process multiple results;
- selector code has no presentation, TUI, clipboard, execution, or persistence side effects.

### Deterministic tie-break chain

Recommended order for query results:

1. exact ID;
2. exact normalized description;
3. exact command;
4. match score;
5. explicit favorite preference only when requested/current sort contract includes it;
6. configured sort key;
7. library stable identity/name;
8. snippet ID.

Never rely on filesystem enumeration, hash-map order, locale-dependent collation without policy, or unstable floating-point ties.

---

## Workstream C — Add deterministic `snp get`

Preferred command surface:

```bash
snp get --id <uuid>
snp get --description-exact <text>
snp get --command-exact <text>
snp get --query <text> --unique
snp get --query <text> --first
snp get --library <name> ...
snp get --all-libraries ...
```

Output controls:

```bash
snp get ... --field command
snp get ... --field description
snp get ... --field id
snp get ... --raw
snp get ... --expanded
snp get ... --json
```

Semantics:

- no TUI;
- no shell execution;
- no clipboard access;
- no local mutation;
- no sync/network access by default;
- exact selectors return one result or typed not-found/ambiguous outcome;
- query without `--first`, `--unique`, or multi-result output must follow one explicit default; recommended default is `--unique` behavior;
- `--first` uses stable ordering and is clearly named as intentional selection;
- raw output is stored value before variable substitution;
- expanded output uses explicit variable resolution rules;
- JSON includes library and stable identity;
- field output emits only the field bytes according to newline policy;
- no update notice, sync attention warning, tracing, or prompt contaminates stdout.

### Raw newline policy

Choose and document one policy:

- exact stored bytes, no added newline; or
- text-line output with one added newline.

For composability, prefer exact bytes under `--raw`/`--field command` and human newline behavior only in default display. Tests must cover empty final line and multiline commands.

---

## Workstream D — Explicit noninteractive variable assignment

Support repeated assignments:

```bash
snp get --id ... --expanded --var host=example.com --var env=prod
snp run --id ... --var host=example.com
snp clip --id ... --var host=example.com
```

Recommended parser result:

```rust
pub struct VariableAssignments(BTreeMap<String, String>);
```

Rules:

- parse `key=value` without shell evaluation;
- duplicate assignment policy is explicit; prefer rejection for conflicting duplicates;
- required variables without a value fail in noninteractive mode;
- defaults apply deterministically;
- choices validate values against allowed set;
- prompts occur only for commands/modes that explicitly permit prompting and only on a TTY;
- pipeline/machine modes never prompt;
- no implicit import of arbitrary environment variables;
- optional environment mapping, if added, requires explicit `--var-from-env NAME` and is documented;
- values are never logged or persisted in status;
- expansion remains textual; it does not perform shell escaping/evaluation;
- shell safety warning remains attached to explicit execution.

Add a way to distinguish raw, default-expanded, and fully supplied expanded output in JSON metadata without exposing secret assignment values unnecessarily.

---

## Workstream E — Exact targeting for run, clip, and edit

Add shared selectors where useful:

```bash
snp run --id <uuid>
snp run --description-exact <text>
snp clip --id <uuid>
snp edit --id <uuid>
```

Requirements:

- all use `SnippetSelector`;
- no TUI when one exact result resolves;
- ambiguous selectors return the same typed category;
- `run` remains the only command that executes;
- `clip` writes exactly the selected/expanded bytes;
- `edit` preserves ID and uses Phase 07A persistence/transaction primitives;
- edit records exactly one pending generation after commit;
- usage increments only under the documented successful run/clip policy;
- failed execution exit behavior is explicit;
- selectors and variables are not logged;
- exact operations support library scoping consistently.

Do not add `--first` to `run` by default unless the user has explicitly requested it; accidental execution from an ambiguous query is unacceptable.

---

## Workstream F — Public CLI outcome and exit policy

Separate public CLI exits from hidden executor exits.

Recommended stable categories:

```text
0  success
1  general operational failure
2  CLI usage/argument error
3  not found
4  ambiguous match
5  user cancelled interactive action
6  validation or local persistence failure
7  synchronization failure
8  snippet execution failure wrapper
9  destructive action refused or generation changed
```

Before fixing numbers, inventory current observable behavior and preserve compatibility where practical.

Recommended typed application outcome:

```rust
pub enum CliOutcome {
    Success,
    NotFound,
    Ambiguous,
    Cancelled,
    ValidationFailed,
    PersistenceFailed,
    SyncFailed,
    ExecutionFailed { child_code: Option<i32> },
    ConflictOrRefused,
}
```

Rules:

- one centralized exit mapper;
- usage errors remain Clap-controlled/documented;
- cancellation is not internal failure;
- shell integrations can distinguish cancellation from error;
- `run` child exit policy is explicit: either propagate valid child code or map to stable wrapper code while printing child code to stderr/JSON report;
- hidden worker/executor codes remain internal;
- machine error mode does not print human prose to stdout.

---

## Workstream G — Machine-output guard

Introduce one application-level guard/context for machine modes:

```rust
pub struct OutputContext {
    pub mode: OutputMode,
    pub color: ColorPolicy,
    pub interactive: bool,
}
```

Machine modes include JSON, CSV, raw field output, shell completion, and selection output consumed by shell integration.

Rules:

- data only on stdout;
- diagnostics on stderr;
- no ANSI unless explicitly requested in a human mode;
- no update notices;
- no auto-sync attention advisory;
- no prompts;
- no progress spinners;
- no tracing subscriber writing stdout;
- no extra newline in exact-byte modes;
- broken pipe handled gracefully without backtrace/noise;
- serialization failure returns nonzero and no partial invalid JSON where practical.

Audit every command against the contract table.

---

## Workstream H — JSON schema family

Define schemas for:

```text
list
get
status
doctor/validate
backup/restore/repair reports
import/export reports
```

For complex outputs use:

```json
{
  "schema": 1,
  "items": []
}
```

Common identity object:

```json
{
  "id": "uuid",
  "library_id": "...",
  "library": "work"
}
```

Rules:

- snake_case;
- explicit nullability;
- stable timestamp and UUID formats;
- deterministic item/diagnostic ordering;
- additive fields allowed in same schema;
- breaking changes increment schema;
- no ANSI or mixed human prose;
- errors may use a stable JSON stderr/report mode only if it remains simple and consistent;
- secret-bearing values excluded;
- command text appears only when the requested output includes it.

Commit JSON fixtures and compatibility documentation.

---

## Workstream I — Library scope and identity

Use the Phase 07A identity contract.

Required scope modes:

```text
primary library
named library
all libraries
stable library ID if exposed
```

Rules:

- ID global/library-scoped behavior is documented;
- exact description/command can be ambiguous across libraries;
- machine results always include library identity;
- primary-library default is explicit in help;
- `--all-libraries` is explicit when behavior differs;
- missing library uses typed not-found/validation category;
- case/canonicalization policy is stable across platforms.

---

## Workstream J — Shell integration audit

Review generated Bash, Zsh, and Fish integrations.

Separate actions:

```text
select and insert at prompt
select and print
select and copy
select and execute
```

Requirements:

- insertion does not execute;
- execute-now behavior, if present, is explicit and documented;
- avoid `eval` for insertion;
- preserve multiline commands;
- preserve exact cancellation behavior;
- safe temporary output paths;
- cleanup temp files;
- paths with spaces/Unicode work;
- shell function stdout remains clean;
- `snp select` machine-output contract is stable;
- no sync/status warning contaminates command substitution;
- generated scripts pass shell-native syntax checks.

Use real shell process tests where CI provides the shell. Do not assume POSIX quoting for Fish or Windows paths.

---

## Workstream K — Help, completions, and discoverability

Update:

- `--help` descriptions for interactive versus deterministic commands;
- selectors and conflicts;
- raw versus expanded output;
- variable assignment;
- library scope;
- ambiguity and `--first`;
- execution warning for `run`;
- guaranteed non-execution for `get`/list/status/validate;
- public exit-code reference;
- JSON schemas;
- shell examples;
- generated completions.

Help examples must not contain unsafe command substitution or encourage storing secrets in snippets/CLI history.

---

## Workstream L — Compatibility and deprecation policy

Before changing existing aliases/output:

- identify scripts likely relying on current behavior;
- preserve old aliases where harmless;
- warn/deprecate before removal;
- changelog output/exit changes;
- do not silently repurpose an existing flag;
- maintain `select` behavior expected by shell integration;
- provide migration examples for deterministic `get`.

If current commands already offer overlapping exact behavior, consolidate behind shared selector without gratuitous CLI churn.

---

## Test plan

### Selector tests

- ID success/not-found;
- exact description/command;
- ambiguity within/across libraries;
- deterministic first result;
- unique failure;
- repeated-run stable ordering;
- Unicode/case policy;
- invalid/conflicting selector flags.

### Output tests

- exact raw bytes;
- multiline/no-final-newline;
- field output;
- JSON fixtures;
- no ANSI/log/update/status contamination;
- broken pipe;
- stderr-only diagnostics;
- cancellation emits zero command bytes.

### Variable tests

- required/default/choice;
- explicit repeated vars;
- duplicate conflict;
- metacharacters/multiline values;
- no prompt in pipeline;
- TTY prompt only where allowed;
- values absent from logs/status.

### Exact operation tests

- run/clip/edit by ID and exact description;
- no TUI;
- ambiguous run refuses execution;
- edit preserves identity and one pending generation;
- clip exact bytes;
- usage policy;
- child execution exit mapping.

### Shell tests

- Bash/Zsh/Fish syntax;
- insertion without execution;
- cancellation;
- multiline;
- spaces/Unicode paths;
- machine stdout purity;
- temp cleanup.

### Non-execution canaries

For `get`, list, status, validate, backup, restore dry-run, import preview, search preview, and select-print modes, use a snippet command that would create a sentinel file if executed. Assert the file remains absent.

---

## Recommended implementation sequence

1. Commit command contract and selector API.
2. Add deterministic `get` raw/field output.
3. Add ambiguity policies and stable ordering.
4. Add JSON output.
5. Add explicit variable assignments/expanded output.
6. Add centralized CLI outcomes/exit mapping.
7. Add machine-output guard and audit existing commands.
8. Add exact selectors to run/clip/edit.
9. Reconcile shell integration and completions.
10. Add compatibility/deprecation docs and write `plans/snip-it-correctness-08a-status.md`.

## Required verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration
cargo test --test pty_integration -- --test-threads=1
```

Run supported shell integration tests and Linux/macOS/Windows command-output suites.

## Exit criteria

Phase 08A is complete only when:

- deterministic non-TUI retrieval exists;
- ambiguity never causes silent selection unless explicitly requested;
- one selector implementation serves get/run/clip/edit;
- output ordering and JSON schemas are stable;
- raw/expanded byte behavior is precise;
- noninteractive expansion never unexpectedly prompts;
- machine stdout is uncontaminated;
- public exit categories are documented and centralized;
- exact run refuses ambiguity and remains explicit execution;
- shell insertion does not execute and preserves multiline/cancellation semantics;
- non-executing commands pass canary tests;
- compatibility changes are documented;
- no workflow engine, remote execution, plugin runtime, or implicit environment ingestion was introduced.
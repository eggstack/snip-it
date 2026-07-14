# Release 4C Plan: Optional Read-Only External Libraries

## Purpose

Design and, only if justified by repository evidence, implement optional read-only indexing of externally managed Pet-compatible TOML files or directories.

This track is intentionally subordinate to snip-it’s native named-library model. It exists for users who keep snippet collections in dotfiles repositories, project directories, or other externally managed locations and want them searchable without importing or copying them into snip-it’s writable library directory.

This document is intended for implementation-agent handoff. Do not treat external libraries as required for Release 4 closure unless usage evidence or an explicit product decision confirms the feature should ship now.

## Gate 0: Evidence and Decision

Before implementing runtime behavior, inspect:

- existing issues, docs, and migration feedback;
- whether users already symlink files into the libraries directory;
- whether current config supports arbitrary library paths;
- how much complexity read-only sources add to selection, edit, delete, sync, backup, and usage metadata;
- whether a documented import workflow already satisfies most demand.

Produce a short design decision in the implementation commit or architecture docs:

```text
Implement now
Defer with documented rationale
Prototype behind an experimental flag
```

If deferred, this plan is complete when the decision, risks, and future interface are documented. Do not add dead configuration fields.

## Proposed Configuration

If implemented, prefer an explicit section such as:

```toml
[[external_libraries]]
name = "project"
path = "/path/to/repo/.snippets"
recursive = true
writable = false
```

Potential optional fields:

```toml
pattern = "*.toml"
follow_symlinks = false
include_hidden = false
```

Do not support `writable = true` in this release. If the field exists, reject true values with an actionable error.

## Product Invariants

1. Native named libraries remain canonical and primary.
2. External sources are read-only.
3. No automatic modification, formatting, migration, backup, sync, or metadata injection occurs in external files.
4. Existing commands behave unchanged when no external configuration exists.
5. External source failures do not corrupt native library state.
6. Selection from an external source never grants mutation rights.
7. Source provenance is visible where needed.
8. Recursive indexing is bounded and safe.

## Workstream A: Configuration Model

Add a versioned configuration type with strict validation:

- unique external library names;
- no collision with native library names unless a namespace policy is explicitly defined;
- absolute or well-defined relative paths;
- regular-file or directory validation;
- bounded pattern syntax;
- explicit recursion;
- explicit symlink policy;
- no writable mode.

Prefer resolving relative paths against the config file directory rather than process current working directory. Document the rule.

Invalid entries should produce diagnostics without preventing unrelated native libraries from loading unless strict configuration validation already requires fail-closed behavior.

## Workstream B: Source Discovery

For a file source:

- load exactly that file;
- validate regular-file target policy;
- enforce the existing maximum source size;
- parse with the shared Pet/snippet compatibility path.

For a directory source:

- honor recursion only when explicitly enabled;
- apply a bounded filename pattern;
- sort discovered paths deterministically;
- reject or skip symlink loops;
- cap file count and traversal depth;
- avoid following device, socket, FIFO, or special files;
- avoid unbounded network filesystem traversal;
- produce per-file diagnostics without exposing command bodies.

Recommended initial caps:

```text
max depth: 16
max files per external library: 10,000
max individual file: existing 16 MiB source limit
```

Make caps configurable only if real usage requires it.

## Workstream C: Unified Read-Only View

Represent loaded snippets with provenance metadata outside the serialized snippet record:

```text
source kind: native | external
source name
source path
read-only flag
stable source-local identity
```

Do not write provenance into Pet-compatible source files.

Ensure retrieval surfaces can combine native and external candidates without losing identity:

- `run`;
- `clip`;
- `search`;
- `select`;
- `list`;
- Release 4 sorting/ranking.

The UI should distinguish duplicate descriptions or commands from different sources when necessary.

## Workstream D: Mutation Guardrails

Every mutation path must reject external snippets before touching disk:

- edit;
- delete;
- favorite toggle;
- folder/tag mutation;
- sync mutation;
- import replacement;
- usage metadata embedded in source files;
- any TUI delete key.

Provide a clear diagnostic such as:

```text
Snippet comes from read-only external library 'project'. Import it into a native library before editing.
```

Offer an explicit copy/import workflow rather than silently mutating or materializing the source.

Potential future command:

```text
snp import external project --library project-local
```

Do not add it unless it cleanly reuses the Release 3 importer.

## Workstream E: Usage Metadata and Ranking

External snippets may participate in local usage-aware ranking, but usage records must remain in the local sidecar/index.

Choose a stable key based on:

- canonical source path;
- external library name;
- source file path within directory;
- snippet ID when present;
- deterministic content-derived fallback when no ID exists.

Document behavior when a source file changes and IDs disappear or commands are edited externally.

Never write use counts or last-used timestamps into external files.

## Workstream F: Reload and Cache Semantics

Define when external sources are reread:

- each command invocation;
- bounded metadata cache with mtime/size validation;
- explicit refresh command.

Preferred first implementation: load per invocation using existing config/TOML caches where safe. Avoid background filesystem watchers in this release.

Cache invalidation must account for:

- file modification;
- file replacement;
- directory additions/removals;
- config changes;
- symlink target changes if symlinks are allowed.

A stale cache must never cause writes to an external path.

## Workstream G: Security

Treat external paths and content as untrusted.

Required controls:

- no execution during indexing;
- no variable expansion during indexing;
- no shell glob execution;
- use Rust filesystem traversal, not shell commands;
- bounded recursion and file counts;
- clear symlink policy;
- terminal control-sequence neutralization in diagnostics/previews;
- no command-body logging;
- no inclusion in sync unless explicitly imported;
- no automatic trust of repo-local config discovered during traversal.

Do not recursively discover additional configuration files from external sources.

## Workstream H: CLI and UX

Possible additive surfaces:

```text
snp library external list
snp library external check <name>
snp doctor --external-libraries
```

Avoid a broad new command hierarchy if configuration plus existing doctor output is sufficient.

Structured listing should expose source/read-only status additively without breaking existing schemas. Human output may show a marker such as `[external:project]` only when ambiguity exists or a verbose mode is requested.

## Workstream I: Tests

Required tests if implemented:

1. no configuration means zero behavior change;
2. single external file loads and participates in search/select/list;
3. directory discovery is deterministic;
4. recursion disabled/enabled behavior;
5. pattern filtering;
6. file-count and depth caps;
7. broken symlink and symlink-loop handling;
8. special files rejected or skipped safely;
9. malformed one-file diagnostics do not corrupt native state;
10. edit/delete/favorite mutations are rejected;
11. run/clip/select remain command-only;
12. external snippets are excluded from sync payloads;
13. backups do not copy external files;
14. local usage metadata works without source mutation;
15. duplicate names/provenance are represented correctly;
16. source modification is observed on the next invocation;
17. doctor reports configuration and load failures without command leakage;
18. path traversal and relative-path rules are deterministic;
19. platform tests cover Unix and Windows path behavior where supported.

Add PTY coverage for selecting an external snippet and attempting a prohibited TUI mutation.

## Documentation

Update, if implemented:

- README;
- USER_GUIDE;
- PET_COMPATIBILITY;
- architecture inventory;
- configuration reference;
- doctor documentation;
- CHANGELOG.

Document:

- read-only guarantee;
- no sync/backup/write behavior;
- path resolution;
- symlink and recursion policy;
- source limits;
- provenance display;
- import/copy path for editing.

If deferred, record the decision and why native import remains the recommended workflow.

## Validation Commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration -- external_library read_only provenance traversal
cargo test --test pty_integration -- --test-threads=1
```

## Completion Criteria

Release 4C is complete in one of two states.

### Implemented

- external sources are explicitly configured and read-only;
- native behavior is unchanged by default;
- traversal is bounded and deterministic;
- all mutation paths reject external snippets;
- sync and backups exclude external content;
- provenance and local usage identity are correct;
- full tests and documentation pass.

### Deferred

- repository evidence and complexity were evaluated;
- the decision and future interface are documented;
- no inactive or misleading configuration surface was added;
- Release 4A and 4B can close independently.

## Decision: Deferred

**Date**: 2026-07-14
**Decision**: Defer with documented rationale

### Rationale

1. **Zero user demand**: No GitHub issues, PRs, or user feedback request external library support. The pet migration roadmap lists R4-C as optional and subordinate to named libraries.
2. **Existing workflow is sufficient**: `snp import pet --merge` handles the use case (dotfiles repos, project directories) with full edit/delete/sync/backup/usage support and zero architectural complexity. Re-importing is idempotent.
3. **High implementation cost**: Would touch every mutation path (edit, delete, favorite, sync, backup, usage), the TUI, the `SnippetData`/`Snippet` types, and the `LibraryManager`. The plan identifies 9 workstreams and 19 required tests.
4. **No inactive config surface**: Deferring means no dead `external_libraries` field in `libraries.toml`.

### Risks of Deferral

- Users who symlink pet files into `~/.config/snp/libraries/` already get read-only indexing as a side effect (standard file I/O follows symlinks). This undocumented behavior is not a compatibility risk since snp already handles symlinked files via `load_library()`.
- Future demand may materialize if the user base grows and users maintain snippet collections outside snp's config directory.

### Future Interface (When Revisited)

If demand materializes, the recommended entry point is:

```toml
[[external_libraries]]
name = "project"
path = "/path/to/repo/.snippets"
recursive = true
writable = false
```

Key design constraints for a future implementation:
- External sources are always read-only; `writable = true` is rejected.
- Native named libraries remain canonical and primary.
- External failures never corrupt native library state.
- Provenance metadata lives outside the snippet record.
- All mutation paths (edit, delete, favorite, sync, backup, usage) reject external snippets with an actionable diagnostic.
- Traversal is bounded (max depth 16, max files 10,000 per external library).
- No execution, variable expansion, or shell glob during indexing.
- Source provenance is visible where needed (e.g., `[external:project]` marker).
- A documented `snp import external <name> --library <target>` workflow for copying external snippets into native libraries for editing.

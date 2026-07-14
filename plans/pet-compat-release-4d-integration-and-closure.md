# Release 4D Plan: Retrieval and Metadata Integration, Hardening, and Closure

## Purpose

Integrate and close Release 4 after optional sorting/ranking and output presentation have landed, and after the external-library track has either shipped behind an explicit read-only contract or been formally deferred.

This is not a new feature phase. It is a correctness, consistency, security, and release-readiness pass across the retrieval and metadata surfaces introduced in Release 4A–4C.

Release 4 should be considered complete only when large imported collections remain predictable under sorting, preserved output metadata is usable without changing execution semantics, and any external-source support cannot mutate or leak into native storage or sync.

## Entry Conditions

Before starting this pass:

- Release 4A sorting/ranking is implemented or explicitly scoped down;
- Release 4B output presentation/edit/search behavior is implemented;
- Release 4C has a documented implemented/deferred decision;
- Releases 1–3 regression suites remain green.

If any feature diverged from its plan, document the final shipped contract before validating it.

## Product Invariants

1. Default fuzzy relevance remains unchanged.
2. New sort modes are explicit and deterministic.
3. Usage metadata remains local-only.
4. `output` remains metadata and is never automatically captured.
5. Run, clip, and select continue operating on command text only.
6. External sources, if present, are read-only and excluded from sync and backup mutation.
7. Existing Pet and snip-it TOML files round-trip exactly.
8. Machine-facing stdout remains clean.
9. Human rendering treats metadata as untrusted text.
10. No Release 5 automatic-sync behavior is introduced.

## Workstream A: Cross-Feature Architecture Audit

Inspect the final implementation for duplicated logic across:

- candidate loading;
- source provenance;
- filtering;
- fuzzy scoring;
- explicit sorting;
- favorites grouping;
- local usage lookup/update;
- output field rendering;
- structured export;
- external-source mutation checks.

Consolidate shared primitives where duplication could create divergent behavior, but avoid broad refactors after behavior is stable.

Preferred conceptual pipeline:

```text
native/external sources
    ↓
source-aware candidate view
    ↓
filters and fuzzy matching
    ↓
explicit deterministic ordering
    ↓
command-specific rendering/action
```

Output metadata should remain attached to the candidate view but must not contaminate command payload paths.

## Workstream B: Default-Behavior Regression Audit

Prove that invocations without Release 4 flags preserve pre-Release-4 behavior.

Required comparisons:

- `snp run` default candidate order;
- `snp clip` default candidate order;
- `snp search` default filtering and ordering;
- `snp select` stdout and cancellation contract;
- `snp list` text/JSON/CSV defaults;
- TUI preview for snippets without output;
- existing edit behavior;
- import and doctor reports;
- shell integration;
- sync payload content.

Use pinned fixtures and exact assertions rather than visual inspection alone.

## Workstream C: Ranking and Identity Correctness

Validate every explicit sort mode across:

- one library;
- multiple native libraries;
- imported Pet libraries;
- external sources if implemented;
- duplicate descriptions/commands;
- missing metadata;
- equal scores and equal timestamps;
- favorite and non-favorite groups;
- Unicode and case differences.

Critical identity checks:

- selecting a sorted row returns the same snippet shown in preview;
- delete/edit/favorite operations affect the displayed native snippet;
- external snippets cannot be mutated;
- usage updates attach to stable identity rather than list index;
- reordering does not rewrite library TOML order unless separately intended.

Add PTY tests that would fail if displayed index and source index diverge.

## Workstream D: Usage Metadata Closure

Audit local usage storage for:

- atomicity;
- permissions;
- corruption recovery;
- write amplification;
- concurrency behavior;
- stable identity across import, rename, and external source changes;
- pruning of deleted entries;
- privacy.

Required behavioral matrix:

| Action | Count update | Last-used update |
|---|---:|---:|
| successful run | yes | yes |
| failed run | no | no |
| cancelled run | no | no |
| successful clip | per final policy | per final policy |
| cancelled clip | no | no |
| select | per final policy | per final policy |
| search/list/preview | no | no |
| edit/import/doctor | no | no |

Document the exact shipped policy and ensure sync, export, and Pet TOML do not receive local usage data.

## Workstream E: Output Metadata Closure

Validate output metadata through:

- direct Pet loading;
- explicit import;
- native creation/editing;
- backup and restore;
- JSON and CSV;
- sync round trip;
- TUI preview;
- optional output search;
- repeated save/load cycles.

Verify exact preservation for:

- tabs;
- trailing spaces;
- CRLF;
- multiline values;
- Unicode;
- ANSI/OSC sequences;
- quotes/backslashes;
- very large but allowed values.

Ensure human rendering neutralizes terminal control sequences while structured output preserves the raw string.

## Workstream F: External-Library Decision Closure

### If implemented

Audit all mutation boundaries and prove:

- edit/delete/favorite/tag/folder operations fail before writes;
- sync excludes external snippets;
- backups do not copy or modify external files;
- usage remains local sidecar metadata;
- source changes are observed predictably;
- provenance remains stable and visible;
- traversal limits are enforced;
- malformed external files do not break native libraries.

### If deferred

Confirm:

- no inactive config schema was added;
- docs identify explicit import as the supported migration path;
- Release 4 completion does not claim external indexing support;
- the future design and deferral rationale are recorded.

## Workstream G: Structured Output and Schema Stability

Review all JSON/CSV surfaces for additive compatibility.

Required checks:

- existing fields retain names and types;
- new fields are optional/additive;
- output metadata is exact;
- local usage fields, if exposed, are clearly local and optional;
- source/provenance fields do not break existing consumers;
- JSON stdout contains JSON only;
- CSV is parseable with multiline metadata;
- errors and warnings stay on stderr.

Add schema fixtures or snapshot tests where appropriate.

## Workstream H: Security and Privacy Review

Use sentinel values to prove that untrusted metadata does not leak or execute.

Test inputs should include:

```text
SUPER_SECRET_RELEASE4_SENTINEL
\x1b]8;;https://example.com\x07link\x1b]8;;\x07
$(touch /tmp/should-not-run)
`touch /tmp/should-not-run`
https://user:password@example.com/path?token=abc
```

Verify:

- no shell execution during ranking, preview, import, doctor, or indexing;
- no terminal escape execution in human views;
- no command/output bodies in usage logs;
- no secret sentinel in diagnostics unless explicitly displaying the user-requested raw field;
- report and sidecar files use private permissions;
- external traversal does not interpret shell patterns.

## Workstream I: Performance and Scale

Create deterministic scale fixtures representative of large migrated collections:

- 1,000 snippets;
- 10,000 snippets;
- many duplicate prefixes;
- multiline output metadata;
- usage sidecar entries;
- multiple libraries.

Measure and guard against severe regressions in:

- initial load;
- fuzzy filtering;
- sort recomputation;
- usage lookup;
- TUI responsiveness;
- JSON/CSV export;
- external traversal if implemented.

Do not add fragile wall-clock assertions to normal CI. Prefer benchmark targets, generous regression thresholds, operation counts, or bounded algorithmic tests.

## Workstream J: Test Organization

The integration suite has grown substantially. During closure, split Release 4 tests into focused files if the current monolithic structure impedes review or isolation.

Suggested organization:

```text
tests/ranking.rs
tests/usage_metadata.rs
tests/output_metadata.rs
tests/external_libraries.rs
tests/release4_regression.rs
```

Preserve shared helpers in a dedicated test-support module. Avoid duplicating environment setup.

## Workstream K: Documentation Reconciliation

Review and reconcile:

- README;
- USER_GUIDE;
- PET_COMPATIBILITY;
- CLI exit/stream policy;
- architecture inventory;
- configuration reference;
- CHANGELOG;
- help output;
- doctor compatibility report.

Documentation must state:

- default sort remains relevance;
- exact sort and tie-break semantics;
- usage metadata locality and update policy;
- output is metadata, not captured execution output;
- optional output-search behavior;
- external-library status and read-only guarantees or deferral;
- machine-output schema additions.

Remove stale planned behavior that did not ship.

## Workstream L: Full Validation

Run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
```

Run focused suites for:

```bash
cargo test --test integration -- sort ranking usage favorites
cargo test --test integration -- output notes csv json search_output
cargo test --test integration -- external_library read_only provenance
cargo test --test integration -- release4 default_behavior
```

Inventory ignored tests and document why each is safe to exclude from closure.

Obtain visible CI results for the supported platform matrix, especially Unix PTY/shell paths and Windows filesystem/path behavior.

## Release 4 Exit Criteria

Release 4 is closed only when:

1. default retrieval behavior is unchanged;
2. every explicit sort is deterministic and documented;
3. usage metadata is local-only, private, and correctly updated;
4. output metadata is safely presented and exactly preserved;
5. command execution/selection payloads remain output-free;
6. external libraries are either safely read-only or formally deferred;
7. structured output remains additive and parseable;
8. security sentinel tests prove no execution or terminal-control leakage;
9. scale behavior is acceptable for large imported collections;
10. full workspace, PTY, and hosted CI validation pass;
11. documentation matches the shipped implementation;
12. no unresolved Release 4 blocker remains before Release 5.

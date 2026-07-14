# Release 4A Plan: Optional Sorting and Usage-Aware Ranking

## Purpose

Add explicit retrieval ordering controls for users with large, long-lived snippet collections while preserving fuzzy relevance as the default behavior.

This track must improve discoverability without silently changing the ranking behavior established by Releases 1–3. Every non-default ordering mode must be opt-in, deterministic, and shared across compatible retrieval surfaces rather than reimplemented independently in each command.

This document is intended for implementation-agent handoff. Inspect the current search/TUI pipeline, snippet metadata model, CLI hierarchy, persistence format, sync model, and test fixtures before editing.

## Product Invariants

1. Existing invocations without new flags preserve current ordering.
2. Fuzzy relevance remains the default.
3. Usage metadata is local-only unless a later design explicitly approves synchronization.
4. Selecting, running, copying, or searching a snippet must not reorder persistent library storage merely to display a sorted view.
5. Sorting must be deterministic, including ties.
6. Machine-facing output must remain stable and free of TUI diagnostics.
7. No ranking mode may execute commands or expand variables merely to score results.

## Proposed CLI Surface

Evaluate the current clap hierarchy and add additive options where applicable:

```text
--sort relevance
--sort recent
--sort last-used
--sort most-used
--sort description
--sort command
--favorites-first
```

Candidate command coverage:

```text
snp run
snp clip
snp search
snp select
snp list
```

Do not force identical flags onto a command where the semantics are nonsensical, but prefer one shared sort enum and one shared ranking implementation.

## Workstream A: Audit Current Retrieval Ordering

Document the current path from library loading through filtering and display:

- library resolution;
- tag and folder filters;
- fuzzy score calculation;
- candidate ordering;
- TUI selection index mapping;
- JSON/CSV/text list ordering;
- favorites handling, if any;
- updated/created timestamp use;
- any hidden tie-breakers.

Add regression tests that pin the current default ordering before refactoring.

Acceptance criteria:

- default ordering is explicitly documented;
- ties are reproducible;
- tests fail if default relevance ordering drifts.

## Workstream B: Shared Sort and Rank Model

Introduce a shared type such as:

```rust
pub enum SnippetSort {
    Relevance,
    Recent,
    LastUsed,
    MostUsed,
    Description,
    Command,
}
```

Keep sorting separate from persistence and UI rendering. A preferred design is a ranked view model containing:

- stable snippet identity;
- original library/source identity;
- fuzzy score where applicable;
- favorite flag;
- created/updated timestamps;
- local usage data;
- deterministic original index.

Define explicit tie-breakers for every mode. Suggested final tie-break chain:

1. requested primary key;
2. optional favorites-first grouping;
3. fuzzy relevance where meaningful;
4. normalized description;
5. stable snippet ID or original source order.

Do not rely on unstable sort behavior.

## Workstream C: Usage Metadata Design

Before adding use count or last-used behavior, inspect whether equivalent fields already exist.

If new metadata is needed, keep it outside Pet-compatible synchronized snippet data unless the current model already has clearly local fields. Preferred approaches, in order:

1. existing local-only metadata fields;
2. a local usage index keyed by stable snippet ID and library identity;
3. a versioned sidecar file under the snip-it config directory.

Required fields:

```text
use_count
last_used_at
```

Define which actions count as use:

- successful `run`;
- successful `clip`;
- successful `select` only if explicitly approved;
- search preview should not count;
- cancellation and failure must not count.

Recommended default: count successful `run` and `clip`; do not count raw browsing, cancellation, or failed execution/copy.

Required properties:

- atomic writes;
- bounded write amplification;
- missing/corrupt metadata fails open to zero usage;
- stale entries do not break library loading;
- deleted snippet records can be lazily pruned;
- imported IDs and regenerated IDs are handled predictably;
- no command bodies in usage logs;
- no remote sync by default.

## Workstream D: Sorting Semantics

### Relevance

Preserve current fuzzy score behavior exactly. Additional usage signals must not alter this mode unless used only as documented bounded tie-breakers.

### Recent

Define whether this means `updated_at` or `created_at`. Preferred: `updated_at` descending, then `created_at` descending.

Handle absent or malformed legacy timestamps deterministically by placing them after valid timestamps.

### Last used

Sort descending by local `last_used_at`. Never-used snippets follow used snippets and use deterministic fallback ordering.

### Most used

Sort descending by local count, then last-used time, then deterministic fallback.

### Description and command

Use a documented normalization policy. Prefer Unicode-aware case-insensitive comparison where available without adding excessive dependency weight. Preserve original text for display and storage.

### Favorites first

Treat this as an orthogonal stable grouping modifier. Within favorite and non-favorite groups, apply the selected sort mode unchanged.

## Workstream E: CLI and Configuration

Add clap value enums and help text. Reject invalid modes through normal clap validation.

Do not change the default configuration. If persistent defaults are supported, introduce them only after command-level behavior is stable, for example:

```toml
[settings.search]
default_sort = "relevance"
favorites_first = false
```

Persistent defaults must be optional and must not alter existing users' behavior after upgrade.

## Workstream F: TUI Integration

Ensure the ranked candidate list and selection index refer to the same stable snippet identity.

Required validation:

- deleting a snippet deletes the displayed item, not an item from pre-sort source order;
- variable expansion operates on the selected ranked item;
- preview corresponds to the highlighted item;
- changing filter text recomputes relevance correctly;
- non-relevance modes have documented interaction with active fuzzy filters;
- cancellation and terminal restoration remain unchanged.

Recommended behavior with an active query:

- filter candidates using fuzzy matching;
- sort matched candidates by the explicitly requested mode;
- preserve fuzzy relevance as a deterministic secondary key where useful.

## Workstream G: Machine Output

For `list --json` and `list --csv`, decide whether sort flags affect emitted order. Preferred: yes, when explicitly supplied.

Do not add local usage fields to existing output by default if that would break schemas. Consider explicit opt-in fields or a versioned structured-report mode.

If usage fields are exposed, document their local-only nature.

## Workstream H: Tests

Add deterministic fixtures with:

- equal fuzzy scores;
- equal timestamps;
- missing timestamps;
- favorites mixed with non-favorites;
- never-used snippets;
- equal counts with different last-used values;
- Unicode descriptions;
- commands differing only by case;
- snippets from multiple libraries.

Required tests:

1. current default relevance ordering is unchanged;
2. each sort mode has exact expected order;
3. favorites-first preserves the selected intra-group sort;
4. successful run/clip updates usage exactly once;
5. cancellation/failure does not update usage;
6. usage metadata corruption fails open;
7. usage data is not written into Pet-compatible library TOML;
8. sync payloads do not include local usage metadata;
9. TUI selection maps to the correct sorted snippet;
10. JSON/CSV ordering follows explicit sort flags;
11. repeated runs are deterministic;
12. no command text appears in local usage records.

Add PTY coverage for at least one non-default sort through the real selector.

## Documentation

Update:

- README;
- USER_GUIDE;
- CLI exit/stream policy where relevant;
- PET_COMPATIBILITY;
- architecture inventory;
- command architecture documents;
- CHANGELOG.

Document:

- default remains relevance;
- exact semantics and tie-breakers;
- which actions update usage;
- local-only metadata policy;
- favorites-first interaction;
- missing usage/timestamp behavior.

## Validation Commands

At minimum run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration -- sort ranking usage favorites
cargo test --test pty_integration -- --test-threads=1
```

## Completion Criteria

Release 4A is complete only when:

- default relevance ordering is proven unchanged;
- all explicit sort modes are deterministic;
- usage tracking is local-only and atomic;
- TUI identity mapping remains correct after sorting;
- machine outputs obey documented ordering;
- cancellation and failures do not mutate usage;
- full workspace and PTY validation pass.

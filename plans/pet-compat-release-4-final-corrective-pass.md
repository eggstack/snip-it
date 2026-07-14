# Release 4 Final Corrective Pass: Real TUI Usage Ranking, Compatibility-Tie Semantics, and Output Sync Contract

## Purpose

Close the remaining Release 4 correctness and documentation gaps before beginning Release 5.

Release 4 is substantially implemented:

- Explicit deterministic sorting exists for relevance, recent, last-used, most-used, description, and command.
- `--favorites-first` is available across retrieval surfaces.
- Local-only usage metadata is stored separately in `usage.toml` and is recorded after successful run and clip operations.
- Output/notes metadata is presented safely in the TUI, can be edited explicitly, and can optionally participate in search.
- Optional external libraries were formally deferred.
- Security, regression, schema, scale, and PTY suites were expanded substantially.

One material defect remains: TUI-backed commands advertise `last-used` and `most-used`, but the TUI sorter currently substitutes `updated_at` because the usage index is not available inside the selector. This means the CLI contract and actual selected identity can diverge. A small number of related compatibility and documentation questions also need closure.

This pass is narrowly corrective. Do not add Release 5 synchronization triggers, implement external libraries, redesign the snippet model, or broaden output capture semantics.

## Required Outcomes

After this pass:

1. `run`, `clip`, `search`, and `select` use real local usage metadata for `--sort last-used` and `--sort most-used`.
2. Interactive TUI sort cycling uses the same ranking semantics as initial CLI-selected sorting.
3. The list surface and all TUI surfaces share one canonical ranking policy rather than maintaining proxy implementations.
4. Selection identity, preview identity, processed snippet identity, and usage-record identity remain the same after every sort/filter transition.
5. Unflagged relevance behavior is explicitly pinned, including equal-score tie behavior.
6. The project documents whether `output` synchronizes across devices or is only preserved locally during merge.
7. PTY tests prove actual ordering with deliberately divergent `updated_at`, `last_used_at`, and `use_count` values.
8. Release 4 receives explicit closure evidence from the full workspace, serialized PTY suite, lint/format checks, and normal CI matrix.

## Current Defect

The current TUI sorting path contains explicit proxy behavior:

```rust
SortMode::LastUsed => {
    // Proxy: sort by updated_at descending (usage data not available in TUI)
}

SortMode::MostUsed => {
    // Proxy: sort by updated_at descending (usage data not available in TUI)
}
```

This violates the documented meanings of the modes:

- `last-used` must order by local `last_used_at` descending, with never-used snippets after used snippets.
- `most-used` must order by local `use_count` descending, then `last_used_at` descending.

A test that merely proves a flag is accepted is insufficient. The selected command must prove that the advertised ordering was applied.

## Product Invariants

1. Relevance remains the default mode.
2. Usage metadata remains local-only and never enters snippet TOML, protocol payloads, backups, imports, or exported snippet schemas unless explicitly documented as a separate local sidecar.
3. Sorting never mutates persistent snippet order.
4. Sorting never executes commands, expands variables, or reads output text beyond the configured search budget.
5. Cancellation and failed operations never increment usage.
6. Successful run and clip operations record exactly one use against the selected snippet ID.
7. Search, select, preview, and navigation do not record usage.
8. Corrupt or missing usage metadata fails open to zero usage without changing snippet data.
9. Equal inputs produce deterministic ordering.
10. The selected row, preview, output-file command, executed/copied snippet, deletion target, and usage entry must all refer to the same stable snippet identity.

## Workstream A: Canonical Ranked Candidate Model

### A1. Audit current ranking paths

Inspect all ranking and filtering paths, including:

- `src/sort.rs`
- `src/usage.rs`
- `src/ui/mod.rs`
- `src/ui/state.rs`
- `src/commands/mod.rs`
- `src/commands/list_cmd.rs`
- `src/commands/run_cmd.rs`
- `src/commands/clip_cmd.rs`
- `src/commands/search_cmd.rs`
- `src/commands/select_cmd.rs`

Document where ranking is currently performed for:

- `list` with and without a filter;
- initial TUI ordering from CLI flags;
- interactive TUI sort cycling;
- favorites-first toggling;
- re-filtering after query edits;
- deletion followed by selector rebuild.

Identify every duplicated comparator or mode translation.

### A2. Introduce a stable candidate representation

Prefer one ranked-candidate abstraction shared by list and TUI code. The exact type may vary, but it should carry enough data to rank without reaching back through fragile parallel arrays.

Suggested shape:

```rust
pub struct RankedCandidate {
    pub original_index: usize,
    pub snippet_id: String,
    pub fuzzy_score: Option<i64>,
    pub use_count: u64,
    pub last_used_at: Option<i64>,
}
```

Avoid copying full command/output bodies into the ranking structure unless necessary.

The canonical stable identity should be snippet ID plus original library index. If duplicate or missing IDs can exist during legacy loading, define the fallback identity explicitly and test it.

### A3. One ranking function

Refactor so one policy function implements:

- relevance;
- recent;
- last-used;
- most-used;
- description;
- command;
- favorites-first grouping;
- deterministic fallback ordering.

The list command and TUI should call the same ranking policy or a thin adapter over the same comparator/key builder.

Do not maintain a second TUI-only interpretation of `last-used` or `most-used`.

## Workstream B: Load and Thread Usage Metadata into TUI

### B1. Load usage once per selection session

At the command/session boundary, load `UsageIndex` once before opening the selector.

Requirements:

- missing file yields an empty index;
- malformed file emits only bounded diagnostics/logging and yields an empty index;
- command text and output text never enter usage logs;
- loading does not rewrite or normalize the usage file;
- usage lookup is by stable snippet ID.

Avoid re-reading `usage.toml` on every keypress.

### B2. Build parallel usage data safely

For every non-deleted snippet candidate, resolve:

```text
use_count
last_used_at
```

Unknown IDs receive zero/None.

Pass this data into the selector through an explicit parameter or candidate model. Do not infer usage from `updated_at`, `created_at`, current row index, description, or command text.

### B3. Refresh after successful operation only when needed

Most selection flows close after a successful run or clip, so no in-session refresh is required. If a flow can continue after recording use, define whether the selector reloads usage before redisplay.

Do not introduce duplicate writes merely to keep the current session visually refreshed.

## Workstream C: Correct Sort Semantics

### C1. Last-used

Implement exact semantics:

1. snippets with `last_used_at = Some` before snippets with `None`;
2. timestamp descending;
3. use count descending as a bounded secondary key if desired, but document it;
4. normalized description;
5. stable original index or stable ID fallback.

Malformed/future timestamps should remain sortable without panic. Decide whether future timestamps are accepted as ordinary values or clamped for presentation only; do not silently rewrite them during sorting.

### C2. Most-used

Implement exact semantics:

1. `use_count` descending;
2. `last_used_at` descending, with `Some` before `None`;
3. normalized description;
4. stable original index or stable ID fallback.

Counter overflow must use saturating behavior or return an explicit error in `record_use`; sorting itself must not panic.

### C3. Recent

Keep the corrected contract using `updated_at` descending. Add a regression test proving TUI and list agree.

### C4. Favorites-first

Favorites-first remains an orthogonal grouping modifier. Within each favorite/non-favorite partition, the selected mode must apply unchanged.

Add usage-mode combinations:

```text
--sort last-used --favorites-first
--sort most-used --favorites-first
```

### C5. Filtered relevance

When a fuzzy query is active, define whether explicit non-relevance sort modes:

- fully replace fuzzy score ordering among matched candidates; or
- retain fuzzy score as a secondary key.

Use one documented rule across list and TUI.

## Workstream D: Default Relevance Tie Compatibility

### D1. Determine historical behavior

Using a pre-Release-4 fixture or the previous implementation, determine how equal fuzzy scores were ordered.

Possible historical behavior includes:

- original insertion order;
- stable matcher return order;
- description order;
- original index.

Do not assume usage metadata was part of the default contract before Release 4.

### D2. Decide bounded usage tie-breakers explicitly

Choose one of these contracts:

#### Compatibility-first

Unflagged relevance uses only fuzzy score plus historical stable fallback. Usage metadata has no effect unless `--sort last-used` or `--sort most-used` is selected.

#### Bounded usage-aware relevance

Usage metadata breaks only exact fuzzy-score ties. This is a deliberate behavior change and must be documented, deterministic, and tested.

The plan preference is compatibility-first unless existing shipped documentation and tests already establish usage-aware relevance as intentional.

### D3. Pin with tests

Create fixtures where:

- two snippets have equal fuzzy scores;
- insertion order differs from use-count order;
- insertion order differs from last-used order.

Assert unflagged behavior exactly.

## Workstream E: TUI Interactive Cycling

### E1. Map modes without semantic loss

Review the mapping between CLI `SnippetSort` and TUI `SortMode`.

Avoid separate pseudo-modes whose names imply unsupported semantics. If `Oldest`, `AlphaDesc`, or other TUI-only modes remain, document them as interactive-only and ensure they do not interfere with CLI mode restoration.

### E2. Re-rank with real usage data

When the user presses the sort-cycle key, rebuild candidate order using the same loaded usage data and canonical ranking policy.

Do not substitute `updated_at` for usage.

### E3. Preserve selection identity

When re-sorting:

- preserve the currently selected snippet by stable identity when it remains visible;
- otherwise select the nearest valid row deterministically;
- ensure preview updates to the same identity as the selected row;
- ensure Enter processes that identity;
- ensure delete targets that identity.

Add tests for multiple snippets with identical descriptions and commands but distinct IDs.

### E4. Favorites toggle

Toggling favorites-first must also preserve selection by identity where possible.

## Workstream F: Usage Recording Integrity

### F1. Run and clip only

Reconfirm usage is recorded exactly once after successful:

- `run` command dispatch;
- clipboard copy.

Do not record on:

- select raw/expanded;
- search preview;
- cancellation;
- variable prompt cancellation;
- command spawn failure;
- clipboard failure;
- delete;
- output editing.

### F2. Correct ID after sorting

Construct a test where display order differs from library order, select a nonzero sorted row, and assert the usage entry belongs to the selected snippet ID rather than the row index or original neighboring snippet.

### F3. Concurrent writes

Audit whether two simultaneous `snp` processes can lose usage increments.

At minimum, document current last-writer-wins behavior. Prefer a lock/read-modify-write strategy if the repository already has a suitable lock primitive and the change remains narrow.

Do not corrupt `usage.toml` under concurrent writes.

## Workstream G: Output Synchronization Contract

### G1. Inspect protocol and merge behavior

Determine the actual current contract:

- whether `output` is represented in `ProtoSnippet`;
- whether uploads include output;
- whether downloads can update output;
- whether merge deliberately preserves local output when remote data wins;
- whether imported output is device-local after import.

### G2. Choose and document one contract

#### Option 1: Local-only output metadata

Document clearly:

- output is preserved in local libraries, backups, import, export, and edits;
- output is not uploaded or downloaded;
- sync merge preserves the local value rather than deleting it;
- another device will not receive the value automatically.

#### Option 2: Synchronized output metadata

Only choose this if protocol compatibility and server rollout are included. Additive protobuf/schema evolution, old-client compatibility, encrypted payload handling, and merge semantics must be implemented and tested.

This corrective pass should prefer documenting local-only behavior unless the protocol already synchronizes output.

### G3. Correct ambiguous language

Replace phrases such as “preserved during sync” with precise wording that distinguishes:

- local preservation during a merge; and
- actual cross-device synchronization.

Update:

- README;
- USER_GUIDE;
- architecture/output.md;
- sync architecture docs;
- PET compatibility matrix;
- CHANGELOG if the shipped contract was previously misstated.

## Workstream H: Tests

### H1. Unit tests for canonical ranking

Cover at least:

1. `last-used` ignores `updated_at` when usage values disagree.
2. `most-used` ignores `updated_at` when counts disagree.
3. never-used snippets sort after used snippets.
4. equal counts use `last_used_at`.
5. favorites-first partitions correctly in usage modes.
6. deterministic ties.
7. malformed/missing usage entries fail open.
8. deleted snippets are absent.
9. duplicate descriptions/commands remain distinct by ID/index.
10. CLI/TUI mode mappings are exhaustive.

### H2. Integration tests

Create a fixture with deliberately divergent metadata:

| Snippet | updated_at | use_count | last_used_at |
| --- | ---: | ---: | ---: |
| A | 300 | 1 | 100 |
| B | 100 | 9 | 200 |
| C | 200 | 2 | 900 |

Expected:

- recent: A, C, B;
- most-used: B, C, A;
- last-used: C, B, A.

Assert `list --json` and `list --csv` identities.

### H3. PTY identity tests

For each TUI-backed command where practical:

- `select --sort last-used` selects C first;
- `select --sort most-used` selects B first;
- `run --sort most-used` executes or records B;
- `clip --sort last-used` records C after successful copy where the platform supports clipboard tests;
- preview text and output-file command match the same snippet;
- interactive cycling from recent to last-used changes order according to real usage;
- favorites-first plus usage sort selects the correct favorite partition and usage order.

Tests must assert selected command or snippet ID, not only exit status or flag acceptance.

### H4. Default relevance regression tests

Pin equal-score behavior with and without usage data present.

### H5. Output sync contract tests

If local-only:

- local output survives server-wins merge;
- output is absent from protocol payload;
- remote-only device does not receive output;
- docs and tests use “local-only” terminology.

If synchronized:

- add protocol compatibility tests across old/new representations.

### H6. Scale tests

Re-run 1,000-snippet tests with populated usage data. Ensure no per-comparison disk access and no pathological slowdown.

## Workstream I: Documentation and Architecture

Update the architecture to show:

```text
library load
    + usage index load
        ↓
ranked candidate model
        ↓
shared rank/filter policy
        ├── list text/json/csv
        └── TUI run/clip/search/select
```

Document:

- exact ranking key order for every mode;
- default relevance tie policy;
- usage file location and local-only status;
- failure-open behavior;
- whether usage writes are concurrency-safe;
- output synchronization contract;
- interactive TUI mode behavior;
- why external libraries remain deferred.

Remove every comment or test note that describes `updated_at` as a proxy for usage.

## Workstream J: Validation and Closure

Run and record:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test pty_integration -- --test-threads=1
cargo test --test release4_regression
cargo test --test schema
cargo test --test security
cargo test --test scale
```

Also run targeted suites for:

```bash
cargo test --lib sort
cargo test --lib usage
cargo test --test integration -- sort usage output sync
```

Record ignored tests and justify each one. Clipboard/display-server skips are acceptable only when documented and covered on a suitable hosted runner.

Confirm CI on supported Linux, macOS, and Windows targets where applicable. Platform-specific PTY and clipboard jobs may use conditional execution, but skipped behavior must be visible.

## Recommended Implementation Sequence

1. Audit duplicated ranking paths and define the stable candidate model.
2. Load `UsageIndex` at the selection-session boundary.
3. Refactor list and TUI to use the canonical ranking policy.
4. Implement exact last-used and most-used semantics.
5. Preserve selected identity across interactive sort/favorites changes.
6. Decide and pin default relevance tie behavior.
7. Audit usage write identity and concurrency.
8. Determine and document the output sync contract.
9. Replace flag-acceptance tests with divergent-metadata identity tests.
10. Reconcile documentation and run the full closure matrix.

## Exit Criteria

Release 4 may be marked complete only when all of the following are true:

- No TUI code uses `updated_at` as a proxy for usage.
- `last-used` and `most-used` produce the same identity ordering in list and TUI surfaces.
- Interactive sort cycling uses real usage data.
- Selected, previewed, processed, deleted, and usage-recorded snippet identities remain consistent after sorting/filtering.
- Default relevance tie behavior is deliberate, documented, and regression-tested.
- Usage remains local-only and absent from snippet/protocol schemas.
- Output synchronization semantics are precise and tested.
- Security, schema, regression, scale, integration, workspace, PTY, lint, and format checks pass.
- Ignored tests are inventoried and none mask ranking, usage, output, or identity defects.
- Documentation contains no proxy behavior or ambiguous sync claims.
- No Release 5 or external-library scope is introduced.

## Handoff Notes

The implementation agent should inspect the current repository rather than assuming signatures in this plan are exact. Preserve the successful Release 4 modular split (`sort.rs`, `usage.rs`, `output.rs`) and reduce duplicated ranking logic rather than adding another adapter layer.

The key acceptance test is semantic, not syntactic: a fixture whose `updated_at`, `use_count`, and `last_used_at` disagree must yield different and correct first selections for recent, most-used, and last-used across both list and TUI commands.
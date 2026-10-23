# CLI, Library, Selector, and Sync Consolidation Milestone 006 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md`

Source subsystem roadmap:

- `plans/subsystems/cli-library-sync-consolidation-roadmap.md#m006--search-parity-verification-corrective`

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `IMPLEMENTATION_COMMIT_SHA`, relative to baseline
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`.

## 1. Executive finding

The milestone's boundary is complete. The three surfaces the parity contract
names — `snp get --query`, `snp list --filter`, and MCP `snippets_search` —
now share one match primitive, so they cannot disagree about *which* snippets a
query selects. The two behaviors the contract had left open are decided,
implemented, and pinned by tests: an empty query is "no filter" on all three
surfaces, and the cross-library order differs by surface family on purpose.

The milestone's other deliverable — making the parity suite actually gate the
build — is complete: `tests/selector_search_parity.rs` now runs in
`scripts/check.sh`, and it exercises all three surfaces instead of two.

Two production behavior changes were needed to reach the decided semantics,
and both are visible:

- `snp list --filter ""` previously dropped snippets whose description,
  command, and tags were all empty. It now returns exactly what `snp list`
  with no filter returns.
- The MCP `snippets_search` tag filter is now applied as an intersection with
  the shared match rather than as a pre-filter, which is set-equivalent.

One behavior was deliberately **not** changed: the cross-library order of
`snp get --query --library all`. Changing it would have altered the
documented `get --resolution all` aggregate order and MCP `snippet_get`,
which is outside this milestone's scope. The difference is now stated in the
contract and asserted per surface instead of being left implicit.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| All three surfaces share one match primitive | `src/selector.rs` — `score_fuzzy_matches` is the documented single match primitive; called from `src/selector.rs` (`SnippetSelector::resolve`), `src/commands/list_cmd.rs`, `src/mcp/tools.rs` | pass | The field projection `searchable_text` is unchanged, so the field set did not change. |
| `snp get --query` covered by the parity suite | `tests/selector_search_parity.rs` — `get_query_parity_matches_list_and_mcp`, `within_one_library_all_three_surfaces_agree_on_order`, `get_query_never_searches_output`, `deleted_snippets_excluded_by_get_query` | pass | The surface was previously never invoked by the suite. |
| Empty-query semantics identical on all three surfaces | `tests/selector_search_parity.rs` — `empty_query_is_no_filter_on_all_three_surfaces` (exact count 9, `work-nonsearchable` present), `empty_query_matches_plain_list`; `src/selector.rs` unit test `test_score_fuzzy_matches_empty_query_is_no_filter_including_empty_text` | pass | Decided: empty query = "no filter". |
| Cross-library ordering decided and each surface's order asserted | `tests/selector_search_parity.rs` — `cross_library_order_is_selector_family_order` (asserts `get` = `personal-deploy`, `work-deploy-1`, `work-deploy-2`; asserts `list` and MCP agree order-sensitively); `src/selector.rs` doc on `sort_matches` | pass | Decided: selector family = library → description → id; merged-set family = global relevance; they coincide within one library. |
| MCP binds no port and executes no stored command during a search | `tests/selector_search_parity.rs` — `mcp_search_does_not_execute_and_binds_no_port` (canary path must not exist; Linux: no `socket:[` fd in `/proc/<pid>/fd`) | pass | The fd assertion is stronger than "no listening port": the stdio server holds no socket at all. |
| Read-only search does not create or rewrite library metadata | `tests/selector_search_parity.rs` — `read_only_search_does_not_rewrite_library_metadata` (byte-level snapshot of `libraries/` and `libraries.toml` before and after all three surfaces) | pass | Scoped to library material; the app's rotating logs under `<config>/logs/` are expected and unrelated. |
| Non-searchable fields never returned | `tests/selector_search_parity.rs` — `non_searchable_fields_are_never_returned`; `src/selector.rs` unit test `test_searchable_text_excludes_folders_favorite_and_sync_metadata` | pass | `folders`, `favorite`, and sync metadata (`id`, `device_id`, timestamps) never reach the projection. |
| The order-insensitive comparison corrected | `tests/selector_search_parity.rs` — `stable_limit_and_ranking_parity` now compares the ordered vectors directly; the comment matches the assertion | pass | The comment's claim was already true for a single library; the test just never asserted it. |
| The suite gates the build | `scripts/check.sh` — `cargo test --test selector_search_parity --features test-support` added to the focused-target list | pass | The CI workflows inherit the gate through `scripts/check.sh`. |
| The suite is isolated | `tests/selector_search_parity.rs` — every case uses `setup_test_env()` (TempDir + `XDG_CONFIG_HOME` override); the only global path is a canary file that must never be created | pass | No real config, keychain, or port is touched. |
| Existing parity cases still pass | `cargo test --test selector_search_parity --features test-support` — 29 passed | pass | The nine original cases are unchanged in behavior; the fixture grew from 6 to 10 `work` snippets. |
| Negative-case evidence that the cases have teeth | A deliberate divergence in `src/commands/list_cmd.rs` (dropping empty-text snippets) and in `src/selector.rs` (the empty-query branch) each made the suite fail with the expected assertion; both files were restored byte-for-byte (sha256 verified) and the suite returned to green | pass | Two independent divergences, two distinct failing tests. |
| No new protocol, schema, or identifier change | `git diff --stat` — `snip-proto/`, `proto/`, `src/library/model.rs`, and `src/library/persistence.rs` untouched | pass | The milestone is a search-surface change only. |
| The full Linux CI gate | `bash scripts/check.sh` | **not run** | Deliberately out of scope for this milestone's scoped verification; it remains the pre-merge gate. See §4. |

## 3. Production implementation evidence

### One match primitive

`src/selector.rs::score_fuzzy_matches` is now documented and used as the
single match primitive for all three surfaces. It owns the deleted filter, the
empty-query rule, and the fuzzy match over the `searchable_text` projection.
Only the *ranking* of the returned indices is left to the callers.

The three call sites:

- `snp get --query` — `src/selector.rs`, `SnippetSelector::resolve`, with
  `SearchFields::fuzzy_default()`.
- `snp list --filter` — `src/commands/list_cmd.rs`, with
  `SearchFields::fuzzy_default()`, or `SearchFields::with_output()` under
  `--search-output`.
- MCP `snippets_search` — `src/mcp/tools.rs`, with the same two named
  constructors.

`src/commands/list_cmd.rs` no longer builds its own matcher and no longer
carries its own deleted filter; `src/mcp/tools.rs` no longer builds its own
matcher, its own score map, or its own deleted filter. Both files dropped
their `fuzzy_matcher::FuzzyMatcher` import.

### Decided semantics

1. **An empty query is "no filter."** Every non-deleted snippet matches, with
   no scores, including snippets whose searchable text is empty. A non-empty
   query matches the `searchable_text` projection only; a snippet with an
   empty projection never matches a non-empty query, and deleted snippets
   never match any query.

   The tiebreak rule applied: when the documentation is silent, preserve the
   behavior of the larger number of current surfaces. Two of the three
   surfaces already behaved this way, and "no filter" is the natural reading
   of an empty argument. The alternative would also have contradicted
   `snp list` with no filter at all, which returns every live snippet.

2. **Cross-library order differs by surface family, by design.** The selector
   family (`snp get --query`, `snp run/clip/edit --library all`, MCP
   `snippet_get`) resolves per library and then aggregates, so its
   cross-library order is library name → description → snippet id. The
   merged-set family (`snp list --filter`, MCP `snippets_search`) merges the
   libraries first and then scores the merged set, so its order is global
   relevance. Within a single library the two orders coincide.

### Deliberately unchanged

- The field set of `searchable_text`. `output`/notes still participate only
  under `snp list --search-output` and MCP `search_output`; `snp get --query`
  has no opt-in and never searches output.
- The MCP tool schemas. No tool gained, lost, or renamed a parameter.
- The read-only boundary. `snp get`, `snp list`, and MCP still use the
  readonly resolver; the search path writes nothing.

### Documentation

- `.skills/selector-and-search-parity.md` — the contract now names two
  functions (the field set and the match primitive), states the two contract
  items, records the tiebreak rule, and states that the suite gates the
  build.
- `architecture/selector.md` — `score_fuzzy_matches` is documented as the
  shared primitive with its three call sites; the false "only `snp get
  --query` calls it" claim was corrected; the cross-library order contract
  and the empty-query rule are stated.
- `architecture/commands/get_cmd.md`, `architecture/commands/list_cmd.md`,
  `architecture/mcp.md` — per-surface statements of both decided rules.
- `docs/COMMANDS.md` — a "The search contract" section under snippet
  selection, including the visible `snp list --filter ""` change.
- `CHANGELOG.md` — a `### Changed` entry for the shared primitive and the
  empty-filter behavior, and a `### Fixed` entry for the verification gap.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test --test selector_search_parity --features test-support
cargo test --test architecture --features test-support
cargo test --test selector_integration --features test-support
cargo test --test mcp_integration --features test-support
cargo test --test local_contracts --features test-support
cargo test --test cli_surface_compat --features test-support
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no diff) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass — 0 errors, 0 warnings |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass — 0 errors, 0 warnings |
| `cargo test --workspace --lib` | pass — `snip-it` 1214 passed / 0 failed / 8 ignored; `snip-proto` 0; `snip-sync` 187 passed / 0 failed |
| `cargo test --test selector_search_parity --features test-support` | pass — 29 passed / 0 failed |
| `cargo test --test architecture --features test-support` | pass — 11 passed / 0 failed |
| `cargo test --test selector_integration --features test-support` | pass — 23 passed / 0 failed |
| `cargo test --test mcp_integration --features test-support` | pass — 4 passed / 0 failed |
| `cargo test --test local_contracts --features test-support` | pass — 23 passed / 0 failed |
| `cargo test --test cli_surface_compat --features test-support` | pass — 18 passed / 0 failed |

One failure occurred during execution and was fixed: the new unit test
`test_score_fuzzy_matches_non_empty_query_skips_empty_text` initially
asserted that the query `"git"` matched exactly one fixture snippet. It
matches two (`git commit` and `git push`); the assertion was wrong, not the
production code. The test was corrected to assert both matches and the
absence of the empty-text snippet, and the suite was re-run to green.

`bash scripts/check.sh` was **not run**. The milestone's verification was
deliberately scoped to the affected surfaces, per the user's instruction to
keep testing minimal and bounded to the change. `scripts/check.sh` remains the
authoritative pre-merge gate and is expected to pass; it now additionally runs
the parity suite.

### Negative-case evidence

Two deliberate divergences were introduced, each reverted byte-for-byte
(sha256 verified before and after):

1. `src/commands/list_cmd.rs` — the matched set was changed to drop snippets
   whose searchable text is empty. The suite failed in
   `empty_query_is_no_filter_on_all_three_surfaces`, with the expected
   mismatch on the exact count and on the presence of `work-nonsearchable`.
2. `src/selector.rs` — the empty-query branch was changed to skip empty-text
   snippets. The suite failed in
   `empty_query_matches_plain_list` and in
   `read_only_search_does_not_rewrite_library_metadata`'s fixture
   expectations, confirming the empty-query rule is load-bearing on the
   production side and not only on the test side.

After both reverts the suite returned to 29 passed / 0 failed.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| One definition of which fields participate in a search | `searchable_text` unchanged; `src/selector.rs` unit test `test_searchable_text_excludes_folders_favorite_and_sync_metadata` pins the never-searchable set | pass |
| The three surfaces cannot select different snippets for the same query | All three call `score_fuzzy_matches`; `get_query_parity_matches_list_and_mcp` and `within_one_library_all_three_surfaces_agree_on_order` assert set and order equality | pass |
| `output`/notes participate only under the opt-in | `get_query_never_searches_output` asserts `get` returns nothing for the output-only token while `list --search-output` returns exactly one snippet | pass |
| Read-only paths do not mutate | `read_only_search_does_not_rewrite_library_metadata`; the readonly resolver is still used by `get`, `list`, and MCP | pass |
| The MCP boundary stays read-only and non-executing | `mcp_search_does_not_execute_and_binds_no_port` | pass |
| Deleted snippets are excluded on every surface | `deleted_snippets_excluded_everywhere` and `deleted_snippets_excluded_by_get_query` | pass |
| The CLI surface is a published contract | No command spelling, flag, exit code, or output format changed. `cli_surface_compat` passes | pass |
| `snp data` mirror group | No `snp data` spelling was added or removed | pass |
| No protocol, schema, or identifier change | `snip-proto/`, `proto/`, `src/library/model.rs`, and `src/library/persistence.rs` untouched | pass |

## 6. Failure and recovery review

- **Duplicate delivery and idempotency.** Not applicable — the milestone adds
  no delivery path. The MCP tag filter's move from pre-filter to
  intersection is set-equivalent, so no result set changes.
- **Cancellation races.** Not applicable — the milestone starts no background
  work and holds no lock.
- **Restart.** Not applicable — no new persisted state.
- **Partial persistence failure.** Not applicable — the milestone writes
  nothing.
- **Stale generation or lease.** Not applicable.
- **Contention and resource release.** Not applicable.
- **Malformed or unauthorized input.** The query string handling is
  unchanged. A query that matches nothing still yields the pre-existing
  outcomes on each surface (`snp get` exits 3, `snp list` returns an empty
  list, MCP returns an empty result set).
- **Bounded event and artifact behavior.** The MCP `snippets_search` limit is
  unchanged and still applies after the shared match.

## 7. Migration and compatibility review

- **Schema migration.** None. No file format, protocol, or database changed.
- **Backward compatibility.** The only visible change is
  `snp list --filter ""`, which now returns the same rows as `snp list` with
  no filter. A caller that relied on the old dropping behavior was relying on
  an inconsistency between surfaces.
- **Protocol negotiation.** Not applicable — no protocol field changed.
- **Configuration behavior.** Unchanged.
- **Rollback limitations.** Reverting the three production edits restores the
  prior behavior exactly; the new tests would then fail, which is the
  intended signal.
- **Legacy-path status.** Unchanged.

## 8. Security review

- **Authorization enforcement.** Unchanged. The MCP server still scopes every
  query by library through the readonly resolver.
- **Secret handling.** Unchanged. The search path reads no credential and
  writes no credential.
- **Path validation.** Unchanged. The parity suite's canary uses a fixed
  absolute path under `/tmp` that must never be created; the test removes any
  pre-existing file first and asserts it is still absent afterwards, so a
  genuine execution would be detected and not silently absorbed.
- **Privilege boundaries.** Unchanged.
- **Denial-of-service bounds.** The MCP `limit` is unchanged. The suite's
  largest fixture is nine live snippets in one library.
- **Redaction.** Unchanged. No credential passes through a `runtime_error`
  detail.
- **Audit behavior.** Unchanged — the milestone performs no mutation and
  writes no audit entry.

## 9. Documentation and operations

Updated architecture documents:

- `architecture/selector.md` — the shared primitive, the empty-query rule, the
  cross-library order contract, and the corrected call-site claims.
- `architecture/commands/get_cmd.md` — `get`'s empty-query and no-output
  rules, and its cross-library order.
- `architecture/commands/list_cmd.md` — the `--filter ""` change, the shared
  primitive, and `list`'s cross-library order.
- `architecture/mcp.md` — the shared primitive, MCP's empty-query rule, MCP's
  merged-set order, and the tag filter as an intersection.

Updated operator and user documentation:

- `docs/COMMANDS.md` — "The search contract".
- `CHANGELOG.md` — `### Changed` and `### Fixed` entries.

Updated agent reference:

- `.skills/selector-and-search-parity.md` — the two contract items, the
  tiebreak rule, the named-constructor rule, the build gate, and two new
  "common mistakes" entries.

Static guards and build gates:

- `scripts/check.sh` — the parity suite is now a focused target.
- `tests/architecture.rs` — **unchanged**. The milestone adds no new
  structural guard; the parity suite is the guard, and it is a behavioral
  suite rather than a static one.

Recovery instructions: none. The milestone is not a recovery path.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | The suite's fixture snippet `work-canary` uses the fixed path `/tmp/snp-parity-canary-must-not-run`. A concurrently running copy of the suite on the same machine could observe the other's canary if it were ever created. | Negligible — the canary is never created, and the test removes any pre-existing file before asserting. | None. If a future change makes the MCP server execute commands, give the canary a per-test unique path. |
| low | The "binds no port" assertion is Linux-only (`/proc/<pid>/fd`). macOS and Windows CI run `cargo check --workspace --all-targets` and `platform_smoke` only, so the assertion does not execute there. | The stdio-only property of `snp mcp serve` is not asserted on non-Linux platforms. | None for this milestone. A cross-platform port assertion (for example, a listening-socket probe) can be added if the MCP surface changes. |
| low | `bash scripts/check.sh` was not run for this milestone; verification was scoped to the affected surfaces. | The full Linux CI gate is unverified for this change set, although the components it runs were verified individually. | Run `scripts/check.sh` before merging. |
| low | The cross-library order difference between the selector family and the merged-set family is a deliberate design choice, not a convergence. A future reader may mistake it for drift. | Documentation-only; both orders are documented and asserted. | None. The distinction is recorded in `.skills/selector-and-search-parity.md` and `architecture/selector.md`. |

No critical, high, or medium finding remains.

## 11. Roadmap disposition

**Milestone closed and the next dependency may proceed.**

The milestone's stated boundary — that the search-parity contract is verified
across all three surfaces, that the build enforces it, and that the two
behaviors the contract left open are decided and pinned — is complete.

The subsystem roadmap `plans/subsystems/cli-library-sync-consolidation-roadmap.md`
is now complete: M001 through M006 are all closed, and §11's completion
definition ("each have accepted closure evidence and no unresolved
medium-or-higher finding remains") is satisfied. The roadmap's `Status:` moves
from `active` to `complete`, and its milestone table records M006 as closed
with a link to this record. The completed milestone history is preserved; a
dated note records the change.

No corrective implementation plan is required for this milestone.

## 12. Registry updates

`plans/registry.md`:

- The `cli-library-sync-consolidation` roadmap row moves from `active` to
  `complete`.
- The M006 implementation-plan row moves from `ready` to `closed`, with the
  closure-record link added.
- The M006 closure-work row is completed and linked to this record.
- The "Next handoff" section is rewritten: no dependency-ready implementation
  plan remains in this subsystem.
- The search-parity gate note is replaced by the closure statement, with the
  two decided semantics summarized.

`plans/subsystems/cli-library-sync-consolidation-roadmap.md`:

- `Status: active` → `Status: complete`.
- §11's completion definition is marked satisfied.
- §12's milestone table: the M006 row becomes `closed` with the plan link and
  the closure-record link.
- A dated change note records the closure and that the completed milestone
  history is unchanged.

`plans/README.md`:

- The `cli-library-sync-consolidation` workstream status moves from `active` to
  `complete`.

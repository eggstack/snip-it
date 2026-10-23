# CLI, Library, Selector, and Sync-Policy Consolidation Milestone 006 — Search parity verification corrective

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/cli-library-sync-consolidation-roadmap.md#m006--search-parity-verification-corrective`

Long-term requirements:

- `plans/000-long-term-specification.md#7-search-selector-and-mcp-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#6-selection-and-outcome-terms`

Applicable ADRs: none.

Primary class: invariant

This is a corrective plan. It references the original milestone and its
closure record, enumerates the unclosed requirements and newly discovered
defects, explains why the original verification did not catch them, and adds
regression evidence that would have caught them.

Original plan:

- `plans/implementation/cli-library-sync-consolidation/004-selector-search-and-mcp-read-parity.md`

Original closure record:

- `plans/closure/cli-library-sync-consolidation/004-status.md`

## 1. Objective

Close the verification gap in the search-parity contract: prove that the
three surfaces the contract names — `snp get --query`, `snp list --filter`,
and MCP `snippets_search` — share one search definition, and make that
proof gate the build.

## 2. Why this milestone is ready

- All hard dependencies are closed. M004's hard dependencies, M002 and M003,
  are closed, and their closure records exist.
- The interface dependency is stable: `selector::searchable_text` and
  `selector::SearchFields` are the canonical definitions and are not
  expected to change in this milestone.
- The milestone is a verification and correction pass over an existing
  surface, not a redesign. It changes behavior only where the parity
  contract is currently violated.
- The evidence needed to decide the open questions is cheap to gather: the
  three surfaces and their shared helper are all reachable from the CLI and
  the MCP server.

## 3. Current implementation evidence

### The contract

`AGENTS.md` states that "`snp get --query`, `snp list --filter`, MCP
`snippets_search` share `selector::searchable_text` (description + command
always, tags by default, output/notes only with `list --search-output` /
`search_output`)". `.skills/selector-and-search-parity.md` repeats the
contract and warns that "adding a field changes three surfaces with no
compiler error".

### What the code does

`src/selector.rs` holds the shared definition. `SearchFields` carries the
`include_tags` and `include_output` switches, and `searchable_text` builds
the string that is matched.

`snp get --query` reaches the shared matcher through
`src/commands/get_cmd.rs`, which builds a selector with
`selector.with_query(q)` and resolves it. `snp list --filter` reaches it
through `src/commands/list_cmd.rs`, which calls
`crate::selector::searchable_text(s, fields)` directly and then applies
`matcher.fuzzy_match(&display, filter_str)`. MCP `snippets_search` reaches
it through `src/mcp/tools.rs`, which builds a `SearchFields` inline and
calls `crate::selector::shared_fuzzy_matcher()`.

The three surfaces therefore share the projection and the matcher. The
question this milestone must answer is whether they share the *semantics*
of applying them.

### Unclosed requirement 1 — `snp get --query` is never compared

`tests/selector_search_parity.rs` contains nine tests. Each compares the
CLI's `list --json` output against the MCP `snippets_search` result: the
helper `cli_list_ids` runs `snp list --json` and re-derives identity from
the description and command pair, and the helper `mcp_search_ids` calls
the `snippets_search` tool. The tests cover exact identifier agreement,
exact-description ambiguity, exact-command agreement, fuzzy
description-and-command ordering, tag matching and the explicit tag filter,
the output-and-notes opt-in, deleted-snippet exclusion, cross-library
scope, and stable limit and ranking.

No test in the file invokes `snp get --query`. The `query` strings that
appear in the file are MCP `snippets_search` arguments, not the CLI's
`--query` flag. A repository-wide search for `--query` in the test tree
finds it only in `tests/selector_integration.rs` and
`tests/tui_requires_terminal.rs`, neither of which compares a `get`
result against a `list` or MCP result.

So the parity suite verifies two of the three surfaces the contract names.
The one surface it does not verify is the one the contract lists first.

### Unclosed requirement 2 — the parity suite does not gate the build

`tests/selector_search_parity.rs` is not listed in `scripts/check.sh`, and
it does not appear in the CI workflow. `scripts/check.sh` runs the
workspace library tests and seven focused integration targets —
`platform_smoke`, `destination_permissions`, `auto_sync_closure`,
`auto_sync_concurrency`, `sync_multibatch`, `restore_manifest_gate`, and
`snip_sync_lifetime`. The parity suite is not among them. Its nine tests
pass today, but nothing in the build requires them to.

This is the second-order failure: the suite that exists to catch a
cross-surface divergence is itself not run by the gate.

### Discovered defect 1 — empty-query semantics diverge across the three surfaces

`snp get --query ""` and MCP `snippets_search` with an empty query both
return every non-deleted snippet. `snp list --filter ""` does not.

`src/selector.rs` returns early when the query is empty: it collects every
non-deleted snippet and returns before the searchable-text projection is
consulted at all. `src/commands/list_cmd.rs` filters differently: it
computes `searchable_text` for each snippet and drops any snippet whose
searchable text is empty, with a comment explaining that such a snippet
"can never fuzzy-match a non-empty filter". The two paths therefore
disagree about a snippet whose description, command, and tags are all
empty.

The divergence is reachable in practice: `snp list --filter "$UNSET_VAR"`
expands to an empty filter, and `snp get --query "$UNSET_VAR"` expands to
an empty query.

Whether this is a defect or an intentional difference is not settled by the
current documentation. `.skills/selector-and-search-parity.md` describes
the shared field set but does not state the empty-query behavior, and the
existing comment in `list_cmd.rs` justifies the drop in terms of a
non-empty filter, which does not address the empty case. This milestone
must decide, and whatever it decides must be documented and pinned by a
test.

### Discovered defect 2 — the cross-library ordering claim is not asserted

The parity suite's fuzzy-ordering test compares sets of identifiers, and
the helper it uses sorts both sides before comparing. The comment above
that comparison states that "CLI list order for the same filter matches MCP
order", but the comparison itself is order-insensitive, so the comment
describes a property the test does not assert.

The underlying divergence is real and structural. `snp get --query`
resolves per library and then re-sorts the merged result by library, then
description, then identifier. `snp list --filter` and MCP `snippets_search`
merge the libraries first and then score the merged set globally, so their
order is global relevance order. Across more than one library the two
orders can differ, and there is no test that either order is the intended
one.

### Why the original verification did not catch these

The original milestone's closure record lists a single requirement — the
shared search projection with its opt-in and its exclusions — and lists
"implementation file + parity fixtures" as its evidence, with the result
"pass". That requirement was, in fact, met: the three surfaces do share
`searchable_text`.

Two things went wrong in the verification, and both are structural rather
than careless.

First, the requirement was written at the level of "the projection is
shared", and the evidence was "the fixtures exist and pass". Nothing in the
requirement or the evidence asked which surfaces the fixtures compared, so
a suite that compared two of the three surfaces satisfied it. The
requirement did not name the surfaces, so the evidence did not have to.

Second, the suite was never connected to the gate. `scripts/check.sh` runs
a curated set of focused targets, and the parity suite was not among the
curated set. Even the coverage that did exist was not enforced, so a
regression that broke parity would not have failed the build.

The same gap is described structurally in `.skills/selector-and-search-parity.md`,
which warns that "adding a field changes three surfaces with no compiler
error". The warning is correct about the compiler, but the repository has
no non-compiler mechanism that catches it either — which is the defect this
corrective milestone removes.

## 4. Invariants that must not regress

- `selector::searchable_text` remains the single definition of which
  snippet fields participate in a search.
- Description and command always participate. Tags participate by default.
  Output and notes participate only under `list --search-output` and MCP
  `search_output`.
- Folders, favorite, sync metadata, and credentials never participate.
- The MCP surface remains read-only, stdio-only, non-executing, and
  noninteractive.
- The deterministic identifier derivation and the fingerprint field set are
  unchanged.
- The TUI keeps its own documented matcher; this milestone does not unify
  the TUI with the CLI and MCP.
- No command spelling, exit code, or output format changes unless the
  empty-query decision requires it, in which case the change is recorded
  as a contract change in `docs/COMMANDS.md` and `CHANGELOG.md`.

## 5. Scope

### In scope

- Add parity coverage for `snp get --query` against `snp list --filter`
  and MCP `snippets_search`, for the same fixture and the same cases the
  existing suite covers.
- Add the parity suite to `scripts/check.sh` so it gates the build.
- Decide the empty-query semantics for the three surfaces, make them
  agree, implement the decision, and pin it with a test.
- Decide the cross-library ordering semantics, make the surfaces agree or
  document the difference explicitly, and pin the decision with a test.
- Fix the order-insensitive comparison and its misleading comment so that
  the property the comment claims is the property the test asserts.
- Update `.skills/selector-and-search-parity.md` and
  `architecture/selector.md` with the decided semantics, including the
  empty-query and cross-library cases.
- Add the `snp get --query` parity cases to the suite that the build now
  runs.

### Explicitly out of scope

- Unifying the TUI's matcher with `selector::shared_fuzzy_matcher()`. The
  TUI's independent matcher is documented as intentional at
  `architecture/tui.md`, and this milestone does not change it.
- Changing the field set of `searchable_text`. No field is added or
  removed.
- Changing the search algorithm or the fuzzy matcher.
- Changing the MCP tool schemas.
- Changing the deterministic identifier derivation, the fingerprint, or the
  sync merge order.
- The `folders` and `favorite` write path, the non-interactive delete, and
  the other milestones of the newer roadmaps.
- Rewriting M004's implementation plan or closure record. This is a
  corrective pass with its own plan and its own closure record.

## 6. Required production changes

### Core/domain

The empty-query and cross-library ordering decisions may require a change
in `src/selector.rs`, `src/commands/get_cmd.rs`,
`src/commands/list_cmd.rs`, or `src/mcp/tools.rs`. The preferred shape is
that the shared behavior lives in `src/selector.rs` and the three surfaces
call it, so that agreement is structural rather than coincidental.

The decision on empty query is one of two:

- Treat an empty query as "no filter": every non-deleted snippet matches,
  on all three surfaces. This makes `get --query ""` and
  `snippets_search` with an empty query the model, and requires
  `list --filter ""` to stop dropping empty-text snippets.
- Treat an empty query as "match nothing searchable": the three surfaces
  agree, and the empty-text snippets are dropped everywhere.

The first is the smaller change, because two of the three surfaces already
behave that way, and because "no filter" is the more common expectation
for an empty argument. The milestone's decision must be recorded in
`docs/COMMANDS.md` and in the skill, whichever is chosen.

The decision on cross-library ordering is one of two:

- All three surfaces order by global relevance, which requires `get` to
  stop re-sorting by library after resolution.
- `get` keeps its library-then-description-then-identifier order, and the
  difference is documented as intentional, with a test that asserts each
  surface's own order rather than asserting that they agree.

The second is the smaller change and matches the current state of the
code, which the milestone must not change without reason. The milestone's
decision must be recorded either way.

### Storage and migrations

No schema change. No migration.

### Protocol and DTOs

No protocol change. The MCP tool schemas are unchanged; only their
behavior under an empty query may change, and that is covered by the
decision above.

### Runtime and concurrency

No runtime change. Search is a read-only path and uses
`resolve_selector_readonly`, not `resolve_selector`. The milestone must
not change that, and must not introduce a path that mutates.

### Frontend or operator surface

If the empty-query decision changes `snp list --filter ""`, that is a
visible behavior change and is recorded in `docs/COMMANDS.md` and
`CHANGELOG.md`. `snp get --query ""` is currently documented as returning
all snippets; if the decision changes that, the same applies.

### Security and authorization

No credential handling change. Folders, favorite, sync metadata, and
credentials must remain non-searchable, and the milestone's tests must
assert that a snippet whose only matchable difference is in one of those
fields is not returned by any surface.

The read-only boundary holds: the search surfaces use the readonly
resolver, and the MCP server binds no port and never executes a stored
command.

### Documentation and static guards

- `.skills/selector-and-search-parity.md` — the decided empty-query and
  cross-library semantics, and the statement that the parity suite gates
  the build.
- `architecture/selector.md` — the same semantics.
- `architecture/commands/get_cmd.md` — if the `get` behavior changes.
- `architecture/commands/list_cmd.md` — if the `list` behavior changes.
- `architecture/mcp.md` — if the MCP behavior changes.
- `docs/COMMANDS.md` — if a visible behavior changes.
- `CHANGELOG.md` — any visible behavior change.
- `scripts/check.sh` — the parity suite is added to the gate.
- `docs/EXIT_CODES.md` — only if a new outcome is introduced.

## 7. Ordered work packages

### Work package A — Decide the open semantics

Intent: settle the two questions the parity contract leaves open, before
writing the tests, because the tests encode the decisions.

Required changes: determine the intended empty-query semantics and the
intended cross-library ordering semantics from the product documentation,
the skill, and the existing surfaces; record each decision and its
reasoning in the milestone's own documentation updates; where the
documentation is silent, choose the option that preserves the larger
number of current surfaces' behavior, and record that as the tiebreak
rule.

Acceptance evidence: both decisions are written down with their
reasoning before any production change is made; each decision names the
surfaces it affects and the behavior it changes, if any.

### Work package B — Add the `get --query` parity coverage

Intent: cover the third surface the contract names.

Required changes: extend `tests/selector_search_parity.rs` with a helper
that runs `snp get --query` against the fixture and extracts the
identifiers; add cases mirroring the existing suite — exact agreement on
the fuzzy query, tag participation, the output-and-notes opt-in,
deleted-snippet exclusion, and cross-library scope; assert that `get`
returns the same identifiers as `list --filter` and as MCP
`snippets_search` for the same query.

Acceptance evidence: the new cases pass; the suite now exercises all three
surfaces; a deliberate divergence introduced in one surface fails the
suite, proving the cases have teeth.

### Work package C — Implement the decided semantics

Intent: make the three surfaces agree, per the decisions.

Required changes: implement the empty-query decision and the cross-library
ordering decision in the shared layer where possible; update the surface
that changes; update the existing comment in `src/commands/list_cmd.rs`
that justifies dropping empty-text snippets, so that it describes the
decided behavior rather than the pre-decision behavior.

Acceptance evidence: the new parity cases pass with the decided semantics;
the existing nine cases still pass; no surface regresses.

### Work package D — Fix the order-insensitive comparison

Intent: make the suite assert the property its comment claims.

Required changes: either change the comparison to be order-sensitive, or
change the comment to describe what is actually asserted; if the
cross-library decision makes the two orders legitimately different, split
the assertion so that the same-library case asserts order equality and the
cross-library case asserts each surface's documented order.

Acceptance evidence: the suite's comment and its assertion describe the
same property; an intentional reordering of one surface's results fails the
suite.

### Work package E — Gate the suite

Intent: make the parity contract enforceable.

Required changes: add `tests/selector_search_parity.rs` to
`scripts/check.sh` with the `--features test-support` flag that
`check.sh` passes to its other focused targets, and to the appropriate
section of the script; confirm the suite's isolation requirements — it
must not touch the developer's real configuration directory, and it must
not require serial execution unless it does.

Acceptance evidence: `bash scripts/check.sh` runs the parity suite; a
deliberate parity break fails `scripts/check.sh`; the CI workflow inherits
the gate through the same script.

### Work package F — Document the decided semantics

Intent: make the contract complete.

Required changes: update `.skills/selector-and-search-parity.md` and
`architecture/selector.md` with the decided empty-query and cross-library
semantics; note in the skill that the parity suite gates the build and
that the third surface is now covered; update `docs/COMMANDS.md` and
`CHANGELOG.md` for any visible behavior change.

Acceptance evidence: the skill and the architecture document state the
empty-query and cross-library semantics; a reader can determine the
behavior of each surface from the documentation alone.

## 8. Failure, cancellation, restart, and contention semantics

This milestone changes read-only search behavior and a build gate. It
introduces no state machine and no lock.

The properties that must hold are about test isolation rather than
concurrency: the parity suite must use a `TempDir` with an
`XDG_CONFIG_HOME` override, must not read or write the developer's real
configuration, keychain, or ports, and must not leave state behind that
affects another test.

The build-gate change introduces one operational property: the parity suite
now runs on every `scripts/check.sh` invocation, so it must be fast enough
to keep the gate usable. If it turns out to require serial execution, it
must be added with `--test-threads=1` and noted in
`.skills/testing-and-seams.md`.

## 9. Compatibility and migration

No data migration. No stored artifact changes. No protocol change.

No configuration fallback is needed.

The empty-query decision may change a visible behavior. If it does, it is
recorded in `docs/COMMANDS.md` and `CHANGELOG.md` under the unreleased
section, and the affected command's entry in
`docs/COMMAND_CONTRACTS.md` is updated if its documented behavior changes.

The build-gate change is additive: `scripts/check.sh` runs one more
target. No CI workflow change is needed, because the workflows call the
script.

## 10. Required tests

### Focused unit tests

- A unit test for the shared empty-query path in `src/selector.rs` pinning
  the decided behavior.
- A unit test for the shared cross-library ordering path, if the decision
  introduces one.

### Integration tests

- The extended `tests/selector_search_parity.rs`: `get --query` against
  `list --filter` and MCP `snippets_search`, for the fuzzy, tag,
  output-opt-in, deleted-exclusion, and cross-library cases.
- A test asserting that an empty query behaves identically on all three
  surfaces, and that it does or does not include an empty-text snippet
  according to the decision.
- A test asserting the decided cross-library ordering on each surface.
- A test asserting that a snippet whose only matchable difference is in
  its folders, favorite flag, or sync metadata is returned by no surface.
- A test asserting that the MCP server binds no port and executes no
  stored command during a search.

### Restart and recovery tests

None. This milestone introduces no state machine.

### Contention and cancellation tests

None. This milestone touches no lock.

### Security and negative tests

- A test asserting that credentials and sync metadata never appear in a
  search result and never participate in matching.
- A test asserting that the read-only search path does not create or
  rewrite library metadata — that is, that it uses the readonly resolver.

### Migration and compatibility tests

- A test asserting that the existing nine parity cases still pass with the
  decided semantics, which is the compatibility evidence for the changed
  behavior.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test selector_search_parity --features test-support

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

## 12. Documentation updates

- `.skills/selector-and-search-parity.md` — the decided semantics and the
  statement that the suite gates the build.
- `architecture/selector.md` — the same semantics.
- `architecture/commands/get_cmd.md` — if the `get` behavior changes.
- `architecture/commands/list_cmd.md` — if the `list` behavior changes.
- `architecture/mcp.md` — if the MCP behavior changes.
- `architecture/overview.md` — only if a deep dive is added or removed.
- `docs/COMMANDS.md` — any visible behavior change.
- `docs/COMMAND_CONTRACTS.md` — only if a documented command behavior
  changes.
- `CHANGELOG.md` — any visible behavior change.
- `scripts/check.sh` — the parity suite joins the gate.

## 13. Acceptance criteria

1. `tests/selector_search_parity.rs` exercises `snp get --query`,
   `snp list --filter`, and MCP `snippets_search` against the same
   fixture.
2. The suite is listed in `scripts/check.sh`, and a deliberate parity break
   fails that script.
3. The empty-query semantics are identical on all three surfaces, and the
   decision and its reasoning are recorded.
4. The cross-library ordering semantics are decided and recorded, and each
   surface's order is asserted by a test.
5. The suite's order-insensitive comparison no longer carries a comment
   claiming an order-sensitive property.
6. `selector::searchable_text` remains the single field projection, and
   the field set is unchanged.
7. Folders, favorite, sync metadata, and credentials remain non-searchable
   and non-exposed, proven by a test.
8. The MCP surface remains read-only, stdio-only, non-executing, and
   noninteractive.
9. The existing nine parity cases still pass.
10. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
    pass.
11. Any visible behavior change is recorded in `docs/COMMANDS.md` and
    `CHANGELOG.md`.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the empty-query or cross-library decision would require a change to the
  MCP tool schemas, which is out of scope for this milestone;
- the parity cannot be made structural because the surfaces genuinely need
  different semantics, in which case the difference must be documented
  rather than papered over;
- the existing nine cases fail under either decision for a reason that
  indicates a deeper defect than the two open questions;
- a hard dependency is absent;
- the scope would expand into the TUI matcher, the field set, the sync
  merge, or the identifier derivation;
- the suite cannot be added to `scripts/check.sh` without breaking the
  script's isolation or its runtime budget.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh` results, showing the parity
  suite running.
- The parity suite's negative-case evidence: a deliberate divergence
  introduced in one surface, and the resulting failure.
- The empty-query decision, its reasoning, and the test that pins it.
- The cross-library ordering decision, its reasoning, and the test that
  pins it.
- Evidence that the field set is unchanged and that the non-searchable
  fields remain non-searchable.
- Evidence that the MCP surface remains read-only and non-executing.
- The nine original cases' results.

## 16. Handoff notes

- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes
  that appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another
  test run is in flight. The integration tests execute
  `target/debug/snp`; a rebuild mid-run replaces that binary and produces
  false failures.
- `scripts/check.sh` passes `--features test-support` to its focused
  targets. The parity suite must be added with the same flag if it needs
  it.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- Read-only paths use `resolve_selector_readonly`, never `resolve_selector`.
  The latter runs `ensure_library_mode()` and can migrate or rewrite
  metadata. The search surfaces must stay on the readonly path.
- `resolve_exact_target` is in the mutating family. It calls
  `resolve_selector`, so `snp run/clip/edit --id` can rewrite metadata
  while `snp get` and MCP cannot. Do not widen the mutating family in this
  milestone.
- The TUI's matcher is separate and documented as intentional. Do not
  unify it here.
- `.skills/testing-and-seams.md` lists the serial targets and what CI
  skips. If the parity suite needs serial execution, update that skill.

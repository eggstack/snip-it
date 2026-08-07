# Phase 13 Post-Closure Cleanup — Test Isolation and Record Hygiene

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Reviewed baseline: `d3f16be99ebf420fd483fc689260bee870a9610c`

Date: 2026-08-07

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Phase 13J fixed the remaining production correctness issues and the Phase 13 roadmap is correctly closed from a product/runtime perspective.

A post-closure review of `d3f16be` found three small cleanup items that do not justify a new numbered implementation phase:

1. `src/sync.rs::tests::test_all_encryption_failed_accounting` sets the process-global environment variable `SNIP_ALLOW_PLAINTEXT_API_KEY=true` and never restores it, even though Rust unit tests may run concurrently and the test appears not to require that variable for its direct loopback client construction;
2. the Phase 13J and roadmap final acceptance checklists remain unchecked even though the same files claim `Status: COMPLETE`, `Release disposition: CLEARED`, and contain verification evidence for those items;
3. the final record attribution identifies `f6df933` as the Phase 13J record commit, while the actual final closure commit that changed Phase 13J and the roadmap to `COMPLETE`/`CLEARED` is `d3f16be`; Phase 13J also still contains a literal `<this commit>` placeholder.

This plan is a post-closure hygiene pass. It must not reopen Phase 13 architecture or create Phase 13K.

## 2. Disposition and scope

The production/runtime line remains closed unless this cleanup reproduces a new product defect.

Treat this work as:

- recommended repository hygiene before the next release or development line;
- test-isolation cleanup;
- planning-record normalization;
- no feature work;
- no architecture work;
- no CI redesign.

Do not change Phase 13 roadmap status back to open merely because this plan exists.

If removing the environment variable causes the focused encryption-failure regression to fail, stop and diagnose that concrete failure before modifying production code. Do not assume the environment variable is required.

## 3. Small-model execution rules

These rules are mandatory for handoff reliability.

1. Execute the passes in Section 8 in order.
2. Do not modify a file unless the current pass explicitly allows it.
3. Make the smallest edit that satisfies the pass.
4. Do not refactor production sync behavior while touching the unit test.
5. Do not add an environment lock, serial-test framework, mutex, new dependency, or generalized environment-variable helper for this one test unless removal of the variable is proven impossible.
6. Do not add Phase 13K or reopen completed Phase 13 work.
7. Do not change runtime configuration semantics for `SNIP_ALLOW_PLAINTEXT_API_KEY`.
8. Do not change `SyncClient`, server, encryption, batching, persistence, or orchestration behavior unless a focused test reproduces an actual defect.
9. Do not mark checklist items complete without checking the existing evidence in the same plan/roadmap.
10. Preserve the distinction between implementation commits, intermediate record commits, final closure commits, and this post-closure cleanup commit.
11. Run the exact focused command after the test edit before touching records.
12. If the focused test fails after removing the environment mutation, stop and record why; do not silently restore a global mutation without proving it is needed.

## 4. Expected final diff boundary

Expected files:

```text
src/sync.rs
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
```

Optional only if needed for truthful historical attribution:

```text
plans/snip-it-phase-13i-drain-and-regression-closure.md
plans/snip-it-phase-13h-final-correctness-closure.md
```

Do not modify these without a reproduced need:

```text
Cargo.toml
Cargo.lock
snip-sync/Cargo.toml
snip-sync/src/**
snip-proto/**
tests/**
.github/**
scripts/**
architecture/**
.skills/**
AGENTS.md
src/auto_sync/**
src/transaction/**
src/ui/**
```

The expected implementation is one deleted test-side `set_var` block plus record edits.

## 5. Cleanup item A — Remove unnecessary process-global test mutation

Primary file:

```text
src/sync.rs
```

### 5.1 Baseline issue

`test_all_encryption_failed_accounting` currently contains a block equivalent to:

```rust
unsafe {
    std::env::set_var("SNIP_ALLOW_PLAINTEXT_API_KEY", "true");
}
```

The comment asserts that no concurrent threads observe the variable. That is not guaranteed by the normal Rust unit-test harness.

The test then:

1. starts an in-process plaintext loopback sync server;
2. calls `SyncClient::register(server_url)`;
3. constructs `SyncSettings` directly with the returned API key;
4. creates `SyncClient` directly from those settings;
5. seeds remote state;
6. runs the private test-only encryption failure seam;
7. asserts skipped IDs/counts and remote pull behavior.

The configuration serialization/keychain paths controlled by `SNIP_ALLOW_PLAINTEXT_API_KEY` are not obviously used by this sequence.

### 5.2 Required first edit

Delete only the test-local environment mutation and its now-inaccurate safety comment.

Preferred change:

```text
remove:
    unsafe { std::env::set_var("SNIP_ALLOW_PLAINTEXT_API_KEY", "true"); }

retain:
    all server setup
    registration
    direct SyncSettings construction
    direct SyncClient creation
    seed sync
    injected encryption failure
    all assertions
```

Do not replace it with:

- `remove_var` cleanup;
- a global mutex;
- a serial-test annotation;
- an environment guard abstraction;
- a new `test-support` feature path;
- a production configuration change.

The smallest correct outcome is no environment mutation at all.

### 5.3 Focused verification

Immediately run:

```text
cargo test -p snip-it --lib test_all_encryption_failed_accounting -- --test-threads=1
```

Expected result: PASS.

Then run the sync unit tests using the normal default test-thread behavior:

```text
cargo test -p snip-it --lib sync
```

Expected result: PASS.

The second command matters because the point of this cleanup is to avoid process-global contamination under ordinary parallel unit-test execution.

### 5.4 Stop condition

If the focused test fails only because plaintext API-key storage is unexpectedly required:

1. stop;
2. identify the exact call requiring the variable;
3. record the failing error;
4. prefer a narrower direct construction/test-helper path that avoids config serialization;
5. do not introduce synchronization infrastructure until the direct path is shown impossible.

If fixing the failure would require changing production authentication semantics, stop and amend this plan rather than broadening scope.

## 6. Cleanup item B — Normalize Phase 13J and roadmap checklists

Primary files:

```text
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
```

### 6.1 Rule for checklist edits

Do not mechanically replace every `[ ]` with `[x]`.

For each final closure item:

1. read the criterion;
2. identify the implementation or verification evidence already recorded;
3. check the box only when that evidence actually supports it;
4. leave the item unchecked and add a bounded note if evidence is indirect or absent.

The existing Phase 13J completion record already distinguishes two indirect-evidence items:

- high-level pull direction is indirectly covered through real zero-batch client integration;
- failed-sync cursor behavior is supported by inspected caller control flow.

Those items may be checked if the checklist criterion asks for the behavior to be established rather than requiring a direct named test. Preserve the evidence note so the record does not imply a nonexistent direct test.

### 6.2 Phase 13J checklist

Review all items in the final acceptance section covering:

- production shutdown result;
- orchestration proof quality;
- single sync implementation;
- helper visibility;
- scope control;
- records and verification.

Expected outcome at the reviewed baseline is that all listed Phase 13J criteria can be checked, because the file already records:

- production use of `ensure_clean_requested_shutdown()`;
- direct decision-method tests;
- helper-owned shutdown broadcast tests;
- no-pre-signal timeout proof;
- single prepared sync transport;
- private unit-test-only encryption seam;
- private `add_batch_context`;
- retained real integration tests;
- no dependency/protocol/schema/CI topology changes;
- focused/routine/process verification;
- 5/5 short SIGTERM verification;
- clean-tree `release-check.sh verify` PASS;
- truthful indirect-evidence notes.

Do not alter the historical baseline-defect descriptions merely because the final checklist is being normalized.

### 6.3 Roadmap final checklist

Review the roadmap's final Phase 13 closure checklist.

Expected outcome at `d3f16be` is that the criteria are satisfied by the completed 13A–13J line and can be checked.

Preserve:

```text
Status: COMPLETE
Release-blocking phase: None
Current release disposition: CLEARED
Next plan: none — Phase 13 is closed.
```

Do not reopen the roadmap to register this housekeeping plan as a release-blocking phase.

### 6.4 Record consistency scan

Run:

```text
rg -n "Status:|Release disposition:|Release-blocking phase:|\[ \]|<this commit>|record commit" \
  plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md \
  plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
```

Inspect every match manually.

This is a record-consistency review, not a correctness test.

## 7. Cleanup item C — Correct final commit attribution

Primary files:

```text
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
```

### 7.1 Historical commit roles

Preserve these distinct roles:

```text
6092d5b  Phase 13J implementation commit
f6df933  intermediate record commit that records the implementation SHA

d3f16be  final Phase 13J/Phase 13 closure record commit; changes status to
         COMPLETE/CLEARED and writes final verification results
```

Do not call `f6df933` the sole or final Phase 13J closure record commit.

### 7.2 Phase 13J header/completion record

Replace literal placeholders such as:

```text
<this commit>
```

with the actual historical SHA:

```text
d3f16be
```

The preferred attribution is:

```text
Implementation commit:
- `6092d5b` phase-13j: wire shutdown outcomes and consolidate sync test seams

Intermediate record commit:
- `f6df933` phase-13j: record implementation commit SHA in plans

Final closure record commit:
- `d3f16be` phase-13: record verified phase 13j closure
```

Do not rewrite history to make the later post-closure cleanup commit the original closure commit.

### 7.3 Roadmap header/history

Normalize the roadmap's Phase 13J metadata similarly.

Preferred shape:

```text
Phase 13J implementation commit: `6092d5b` ...
Phase 13J intermediate record commit: `f6df933` ...
Phase 13J final closure record commit: `d3f16be` ...
```

The post-closure cleanup commit may be recorded separately as housekeeping after it exists, but it must not replace `d3f16be` as the historical final Phase 13 closure commit.

### 7.4 Optional historical-plan inspection

Inspect Phase 13H and Phase 13I only to ensure the new wording does not contradict them.

Do not edit them unless a concrete attribution error remains after Phase 13J/roadmap normalization.

## 8. Sequential execution passes

### Pass 1 — Baseline confirmation

Allowed modifications: none.

Confirm:

```text
HEAD descends from d3f16be99ebf420fd483fc689260bee870a9610c
```

Inspect:

```text
src/sync.rs
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
```

Confirm the three cleanup items still exist before editing.

If another commit already fixed an item, do not reimplement it. Adjust the remaining work downward.

### Pass 2 — Remove the environment mutation

Allowed file:

```text
src/sync.rs
```

Action:

- remove only the `SNIP_ALLOW_PLAINTEXT_API_KEY` mutation and its inaccurate comment from `test_all_encryption_failed_accounting`.

Run:

```text
cargo fmt --all -- --check
cargo test -p snip-it --lib test_all_encryption_failed_accounting -- --test-threads=1
cargo test -p snip-it --lib sync
```

All must pass before Pass 3.

### Pass 3 — Normalize closure records

Allowed files:

```text
plans/snip-it-phase-13j-production-outcome-and-test-seam-closure.md
plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md
```

Actions:

1. check only acceptance boxes supported by existing evidence;
2. replace `<this commit>` with `d3f16be`;
3. distinguish `f6df933` as intermediate record commit;
4. distinguish `d3f16be` as final closure record commit;
5. preserve Phase 13 status as COMPLETE/CLEARED;
6. preserve indirect-evidence notes;
7. optionally add one short post-closure cleanup note without making this plan a release-blocking phase.

Run the record consistency scan from Section 6.4.

### Pass 4 — Routine verification

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p snip-it --lib sync
cargo test --test sync_multibatch -- --test-threads=1
bash scripts/check.sh
```

Do not rerun the long ignored lifetime test solely because one unit-test environment mutation was removed unless `scripts/check.sh` or another focused result indicates a server-lifetime regression.

Do not add new CI coverage.

### Pass 5 — Cleanup commit

Commit the test-isolation and record normalization together.

Preferred message:

```text
phase-13: clean test isolation and closure records
```

Record the resulting SHA in a short `Post-closure cleanup` note in Phase 13J or the roadmap only if the project convention expects such historical breadcrumbs.

Do not modify the original `d3f16be` attribution.

### Pass 6 — Clean-tree verification

After the cleanup commit exists and the worktree is clean, run:

```text
bash scripts/release-check.sh verify
```

Expected result: PASS.

Because this command requires a clean tree, do not edit the plan before running it.

If it fails:

- identify whether the failure is code/test related or infrastructure related;
- do not mark the cleanup complete on a code-related failure;
- do not broaden into CI redesign for an external runner issue.

### Pass 7 — Optional record-only verification commit

If the repository convention requires the cleanup verification result to be recorded, make one final record-only commit updating the post-closure cleanup note with:

- cleanup implementation SHA;
- `release-check.sh verify: PASS` or exact failure;
- no residual deviation, if true.

Preferred message:

```text
plans: record phase 13 post-closure cleanup verification
```

This second commit is optional. Do not create it merely to replace a literal self-SHA placeholder; avoid creating another recursive attribution problem.

## 9. Acceptance criteria

### 9.1 Test isolation

- [ ] `test_all_encryption_failed_accounting` no longer calls `std::env::set_var("SNIP_ALLOW_PLAINTEXT_API_KEY", ...)`.
- [ ] The test retains its real in-process server and private injected-encryption path.
- [ ] The test still verifies all failed local IDs in `skipped_ids`.
- [ ] The test still verifies `skipped_count`.
- [ ] The test still verifies seeded remote snippets are returned.
- [ ] The focused test passes without the environment mutation.
- [ ] `cargo test -p snip-it --lib sync` passes under the normal test harness.
- [ ] No environment lock, serial-test dependency, global mutex, or replacement process-global mutation is introduced.

### 9.2 Record consistency

- [ ] Phase 13J final acceptance items are checked only where supported by recorded evidence.
- [ ] The roadmap final closure items are checked only where supported by the completed 13A–13J evidence.
- [ ] Indirect high-level pull evidence remains labeled indirect.
- [ ] Failed-sync cursor evidence remains labeled as inspected caller control flow unless a direct test already exists.
- [ ] No literal `<this commit>` placeholder remains in the Phase 13J final record.
- [ ] `6092d5b` is identified as the Phase 13J implementation commit.
- [ ] `f6df933` is identified as an intermediate record commit, not the final closure commit.
- [ ] `d3f16be` is identified as the final Phase 13J/Phase 13 closure record commit.
- [ ] The post-closure cleanup commit is not retroactively mislabeled as the original closure commit.
- [ ] Roadmap header and footer remain consistent.

### 9.3 Scope control

- [ ] No new numbered Phase 13 phase is added.
- [ ] No production sync behavior changes.
- [ ] No server lifecycle/orchestration changes.
- [ ] No dependency or Cargo feature changes.
- [ ] No protocol, schema, migration, persistence, API, CLI, TUI, updater, or packaging changes.
- [ ] No workflow or CI topology changes.
- [ ] No generalized environment/test infrastructure is introduced.

### 9.4 Verification

- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.
- [ ] focused all-encryption-failed unit test passes.
- [ ] `cargo test -p snip-it --lib sync` passes.
- [ ] `cargo test --test sync_multibatch -- --test-threads=1` passes.
- [ ] `bash scripts/check.sh` passes.
- [ ] clean-tree `bash scripts/release-check.sh verify` passes, or any external-infrastructure exception is recorded accurately.

## 10. Non-goals

Do not use this cleanup to:

- revisit sync semantics;
- alter the success flag behavior for partial encryption failures;
- redesign keychain/plaintext configuration;
- remove the existing explicit plaintext opt-in from production configuration;
- refactor test helpers broadly;
- add test serialization infrastructure;
- rewrite Phase 13H or 13I history;
- add more release ceremony;
- rerun every expensive historical verification merely because documentation changed;
- add a new roadmap phase;
- reopen Phase 13 after all acceptance criteria pass.

## 11. Stop conditions

Stop and request plan amendment instead of broadening scope if:

- the all-encryption-failed regression genuinely requires global plaintext configuration and no direct helper path exists;
- removal of the environment variable exposes a production authentication defect;
- record normalization reveals that a checked Phase 13J criterion lacks actual evidence;
- source changes beyond `src/sync.rs` become necessary;
- a new dependency or test framework is proposed;
- protocol, database, persistence, API, or orchestration changes appear necessary;
- routine verification exposes an unrelated regression that is not caused by this cleanup.

## 12. Completion record template

Fill only after implementation and verification.

```text
Status: COMPLETE | PARTIAL

Reviewed baseline:
- d3f16be99ebf420fd483fc689260bee870a9610c

Cleanup implementation/record commit:
- <sha> phase-13: clean test isolation and closure records

Optional verification-record commit:
- <sha> plans: record phase 13 post-closure cleanup verification

Verification:
- cargo fmt --all -- --check: PASS/FAIL
- cargo clippy --workspace --all-targets -- -D warnings: PASS/FAIL
- cargo test -p snip-it --lib test_all_encryption_failed_accounting -- --test-threads=1: PASS/FAIL
- cargo test -p snip-it --lib sync: PASS/FAIL
- cargo test --test sync_multibatch -- --test-threads=1: PASS/FAIL
- bash scripts/check.sh: PASS/FAIL
- bash scripts/release-check.sh verify from clean cleanup commit: PASS/FAIL

Record corrections:
- Phase 13J checkboxes normalized: YES/NO
- roadmap checkboxes normalized: YES/NO
- d3f16be recorded as final historical closure commit: YES/NO
- literal self-commit placeholders removed: YES/NO

Residual deviations:
- none | <bounded deviation>

Phase 13 disposition:
- remains COMPLETE / CLEARED
```

## 13. Expected end state

When this plan is complete:

- the Phase 13 production line remains closed;
- the encryption-failure test no longer mutates process-global plaintext configuration;
- normal parallel unit tests cannot inherit that mutation from this test;
- Phase 13J and roadmap checklists match their completion status;
- historical commit roles are accurately attributed;
- `d3f16be` remains the original final Phase 13 closure record commit;
- any later cleanup SHA is recorded only as post-closure housekeeping;
- no new architecture, dependency, CI surface, or numbered phase has been added.

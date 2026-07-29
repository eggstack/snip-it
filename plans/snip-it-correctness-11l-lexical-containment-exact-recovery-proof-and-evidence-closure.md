# Phase 11L — Lexical Containment, Exact Recovery Proof, and Evidence Closure

Status: READY FOR IMPLEMENTATION

Authoritative predecessor: `plans/snip-it-correctness-11k-literal-safety-and-proof-closure.md`

Corrective baseline: `9427a5766c70624a49f14682d3c68d55a6faa93c`

This plan is the authoritative remaining-work plan for Phase 11 correctness closure. It is intentionally narrow. It must not restore the previously removed CI, release, evidence-registry, or orchestration complexity.

---

## 1. Why Phase 11 must be reopened

Phase 11K corrected most of the transaction-recovery and sync-proof defects, but the final source review found three unresolved closure problems.

### 1.1 Lexical containment does not reject `ParentDir`

`src/transaction.rs::lexically_within` currently:

1. requires absolute paths;
2. collects root and child components;
3. checks that the child component sequence starts with the root component sequence;
4. returns `true` without rejecting a later `Component::ParentDir`.

Therefore a missing path such as:

```text
<artifact-root>/../../outside.bin
```

can pass the lexical prefix check. Because the final path does not exist, canonical containment is skipped. This violates the Phase 11K rule that missing out-of-root references and all paths containing `..` must fail closed before existence checks.

There is a related path-safety gap for a missing child below an existing symlinked intermediate directory. `path.is_symlink()` checks only the final path. Canonicalization is skipped when the final path is missing. A path such as:

```text
<artifact-root>/link-to-outside/missing.bin
```

must be rejected even though `missing.bin` does not exist.

### 1.2 Restore crash tests still use permissive proof

`tests/restore_crash_failpoints.rs` currently accepts the following substitutions:

```rust
if let Ok(result) = serde_json::from_str::<serde_json::Value>(&stdout) {
    assert!(result["applied"].as_u64().unwrap_or(0) > 0);
} else {
    assert!(exit <= 1);
}
```

The idempotent second invocation also makes JSON parsing and all JSON assertions optional.

These patterns do not prove the CLI contract. They permit malformed output, an unrelated successful repair, partial failure, or a missing status field. Phase 11K explicitly prohibited optional parsing, `> 0`/`>= 1` substitutes for exact counts, and multiple acceptable exit codes.

### 1.3 Closure evidence is attached to the wrong commit identity

The closure record names `ec87344dac409dd0a4ef75eba9f51c42f520c78e` as the final implementation commit, but records CI for the later status commit `a94ec9f4cbaedec8a9f89b56fd8315a081894200`.

The final Phase 11L implementation commit must be identified first. Linux correctness, macOS smoke, and Windows smoke must then be observed passing for that exact SHA. A later documentation-only commit may record the evidence, but it may not replace the implementation SHA as the verified subject.

---

## 2. Required outcome

Phase 11L is complete only when all of the following are true:

1. lexical path validation rejects every `ParentDir` component;
2. lexical containment remains component-based and cross-platform;
3. a missing path outside the transaction artifact root is rejected;
4. a missing path below an existing symlinked intermediate component is rejected on Unix;
5. all unsafe-path failures preserve the journal and transaction artifacts;
6. rollback revalidation uses the corrected path-safety helper immediately before reading a backup;
7. restore crash recovery tests require valid JSON and exact exit/status/counter values;
8. first recovery proves exactly one transaction recovery action was applied;
9. repeated recovery proves an exact clean no-op;
10. no test accepts exit code `0` or `1`, parse success or parse failure, or any positive applied count as equivalent;
11. focused tests, the normal local check, and the release verification pass;
12. publish dry-runs pass for each changed crate;
13. the three intentionally small CI instances pass for the exact final implementation SHA;
14. the closure status names that exact SHA and records only observed evidence;
15. the repository retains manual crates.io publishing and no automated release workflow.

---

## 3. Non-negotiable interpretation rules

These rules override permissive existing tests and prior completion claims.

1. **Reject means `Err` plus state preservation.** Logging or canonicalizing through an unsafe reference is not rejection.
2. **Every `ParentDir` is unsafe.** Do not resolve `..` and then accept the normalized result. Reject the component when encountered.
3. **Missing paths are still validated.** `exists()` may classify presence only after lexical and existing-prefix safety checks pass.
4. **Component comparison is required.** Do not use string prefix matching.
5. **Existing symlinked prefixes are unsafe.** Checking only the final path is insufficient.
6. **Exact means `assert_eq!`.** `> 0`, `>= 1`, `<= 1`, nonempty, and multiple acceptable outcomes are not substitutes.
7. **Required JSON must parse.** Use `expect` or an equivalent hard failure.
8. **A successful recovery fixture must exit `0`.** If unrelated repairs make it exit `1`, correct the fixture or isolate the transaction action. Do not broaden the accepted exit codes.
9. **Idempotent recovery must be an exact no-op.** It must exit `0`, report zero applied and zero failed actions, and leave no journal or pending marker.
10. **CI evidence must identify the exact implementation SHA.** A descendant commit is not equivalent evidence for status accounting.
11. **Passing tests do not override source review.** The source checklist must contain no unresolved negative answer.
12. **Do not add process.** No new workflow, matrix, evidence service, registry, daemon, or release automation is permitted.

### Prohibited patterns in closure tests

Do not retain or introduce these forms in required Phase 11L proof:

```rust
assert!(exit == 0 || exit == 1);
assert!(exit <= 1);
assert!(applied > 0);
assert!(applied >= 1);
if let Ok(report) = serde_json::from_slice(...) {
    // required assertions only here
}
serde_json::from_slice(...).ok();
```

Required shape:

```rust
assert_eq!(output.status.code(), Some(0));
let report: serde_json::Value =
    serde_json::from_slice(&output.stdout).expect("repair JSON must parse");
assert_eq!(report["exit_status"], "repaired");
assert_eq!(report["applied"], 1);
assert_eq!(report["failed"], 0);
```

The exact JSON representation may be deserialized into a typed test struct instead. The values may not be weakened.

---

## 4. Preserved architecture and scope boundaries

Preserve all of the following:

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- typed restartable transaction cleanup;
- generation-conditional executor-owned pending clear;
- one Linux correctness CI job;
- one macOS smoke instance;
- one Windows smoke instance;
- deep crash and protocol verification performed locally;
- manual dependency-ordered crates.io publishing;
- no automated crates.io publishing;
- no GitHub Release automation;
- no new evidence registry;
- no new database, queue, daemon, or orchestration layer.

Do not refactor unrelated restore, sync, command, storage, packaging, or UI code.

---

## 5. Execution protocol for a smaller implementation model

Complete workstreams in order. Keep commits small enough that each can be reviewed independently.

For every workstream:

1. inspect the named current functions and tests;
2. add or tighten the focused failing test first;
3. run the focused test and confirm it fails for the intended reason;
4. implement the smallest production or fixture correction;
5. rerun the focused test;
6. run formatting and focused clippy for touched targets;
7. commit before beginning the next workstream;
8. do not mark Phase 11 complete during implementation.

The closure status remains `INCOMPLETE / REOPENED` until Workstream E is complete.

Suggested commit sequence:

1. `transaction: reject lexical parent traversal and symlinked prefixes`
2. `tests: require exact restore recovery reports`
3. `verification: close Phase 11L on exact implementation SHA`

The first two commits may be combined only if the implementation agent cannot keep the repository compiling between them.

---

# Workstream A — Pin the lexical containment defect with exact tests

## Goal

Create tests that fail against the current `lexically_within` implementation and prove the complete missing-path safety contract.

## Files to inspect

- `src/transaction.rs`
  - `validate_artifact_containment`
  - `validate_contained_path`
  - `lexically_within`
  - `journal_owns_artifacts`
  - `classify_journal_recovery`
  - `rollback_transaction`
  - transaction module tests
- relevant integration tests in `tests/repair_transactions.rs` or the existing transaction recovery test module

## Required direct helper tests

Add unit tests close to the helper. Use absolute temporary paths rather than host-specific literal roots.

Required cases:

| Root and child relationship | Required result |
|---|---|
| child is a normal missing file directly below root | accepted as lexically contained |
| child is a normal existing file below root | accepted |
| child equals a sibling path such as `<parent>/artifacts-other/file` | rejected |
| child is a relative path | rejected |
| root is a relative path | rejected |
| child contains `<root>/../outside` | rejected |
| child contains `<root>/sub/../../outside` | rejected |
| child contains `<root>/sub/./file` | accepted only after `CurDir` normalization |
| child is lexically shorter than root | rejected |
| child uses a different Windows prefix/drive when compiled on Windows | rejected |

Required test names may differ, but they must communicate the literal contract. Recommended names:

```rust
#[test]
fn lexical_containment_rejects_parent_dir_after_matching_root_prefix() { ... }

#[test]
fn lexical_containment_rejects_nested_parent_escape() { ... }

#[test]
fn lexical_containment_accepts_missing_normal_child() { ... }
```

The first regression must reproduce the current defect: the child begins with all root components and then contains `ParentDir`.

## Required classification tests

Direct helper tests are not sufficient. Add journal-level tests proving unsafe references fail before state classification.

At minimum:

1. `Prepared` journal with a missing `backup_path` containing `../../outside.bin` returns `Err`;
2. `RollingBack` journal with that reference returns `Err`;
3. `CommittedLocal` journal with a missing `durable_staged_path` containing `../outside.bin` returns `Err`;
4. `CleaningUp` journal with the unsafe reference returns `Err`;
5. terminal `Committed` journal with the unsafe reference returns `Err` rather than becoming `RemoveTerminalJournal`;
6. terminal `RolledBack` journal with the unsafe reference returns `Err`;
7. a safe missing in-root reference remains classifiable;
8. every error leaves the journal file and artifact directory untouched.

Use exact assertions on the error category/message where the project has a stable error contract. At minimum, assert that the result is `Err`, the journal still exists, and no referenced external path was created, read, removed, or modified.

## Required existing-prefix symlink test

On Unix, create:

```text
artifact-root/
  link -> outside-directory/
outside-directory/
```

Reference:

```text
artifact-root/link/missing.bin
```

The final path is missing, but `link` exists and is a symlink. Classification must return `Err` and preserve the journal, symlink, outside directory, and all artifacts.

Use `#[cfg(unix)]`. Do not make Windows CI depend on Unix symlink privileges.

## Focused commands

Use the actual test names after implementation. Expected shape:

```bash
cargo test --lib transaction --all-features lexical_containment -- --test-threads=1
cargo test --lib transaction --all-features missing_parent_traversal -- --test-threads=1
cargo test --lib transaction --all-features symlinked_existing_prefix -- --test-threads=1
```

## Acceptance criteria

- at least one new test fails against baseline `9427a576...` because `ParentDir` is currently accepted;
- all listed helper cases have exact expected results;
- all transaction state families reject unsafe missing references;
- the Unix missing-child-under-symlink test rejects the reference;
- unsafe-path errors preserve journals and artifacts;
- tests do not rely on the unsafe external target existing.

---

# Workstream B — Implement component-safe containment and existing-prefix validation

## Goal

Make `validate_contained_path` fail closed for lexical traversal and symlinked existing prefixes without changing the transaction architecture.

## Required production behavior

### B1. Reject `ParentDir` during normalization

Replace the current raw component-prefix check with normalization that explicitly rejects `Component::ParentDir`.

One acceptable shape:

```rust
use std::path::{Component, Path, PathBuf};

fn normalize_absolute_without_parent(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => return None,
        }
    }
    Some(normalized)
}

fn lexically_within(root: &Path, child: &Path) -> bool {
    let Some(root) = normalize_absolute_without_parent(root) else {
        return false;
    };
    let Some(child) = normalize_absolute_without_parent(child) else {
        return false;
    };
    child.starts_with(root)
}
```

Equivalent code is acceptable. The behavior is not negotiable:

- `ParentDir` returns rejection immediately;
- `CurDir` may be ignored;
- comparison is by path components;
- absolute prefix/root semantics are preserved;
- do not lowercase paths or compare lossy strings;
- do not resolve `ParentDir` and accept the resulting path.

### B2. Reject symlinked existing prefixes

After lexical containment passes, inspect existing components from the artifact root toward the child.

Required behavior:

1. use `symlink_metadata`, not `metadata`, so the check does not follow the link;
2. reject a symlinked artifact root;
3. reject any existing child component that is a symlink;
4. stop walking only at the first genuinely missing component;
5. propagate filesystem errors other than `NotFound`;
6. do not access the external symlink target;
7. preserve all state on rejection.

Recommended shape:

```rust
fn reject_symlinked_existing_prefixes(root: &Path, child: &Path) -> SnipResult<()> {
    let relative = child.strip_prefix(root).map_err(...)?;
    let mut current = root.to_path_buf();

    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(unsafe_symlink_error(&current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(io_error_for_prefix_check(current, error)),
        }
    }

    Ok(())
}
```

The root itself must be checked separately because `strip_prefix` does not include it.

### B3. Retain canonical containment for existing paths

For existing paths, retain canonical containment as defense in depth.

For missing final paths with existing ancestors:

- the component walk must reject symlinked ancestors;
- if the root exists, canonicalize the deepest existing ancestor and verify it remains under the canonical root;
- do not silently substitute the uncanonicalized path after a canonicalization error other than absence;
- propagate unexpected canonicalization errors.

This catches junction/mount/reparse behavior where supported without making the lexical contract dependent on canonicalization.

### B4. Apply one helper everywhere

The corrected helper must remain authoritative for:

- `validate_artifact_containment`;
- `journal_owns_artifacts` before classification;
- rollback revalidation immediately before `fs::read(backup)`;
- cleanup validation before removing artifacts.

Do not create separate helpers with divergent semantics for classification and execution.

### B5. Preserve failure atomicity

On any unsafe-path error:

- do not delete the journal;
- do not remove the artifact root;
- do not read or overwrite the external target;
- do not advance cleanup or rollback progress;
- return `Err` to the caller;
- leave `snp repair` able to report the item as unsafe/manual.

## Anti-examples

Not acceptable:

```rust
// Resolves traversal rather than rejecting it.
Component::ParentDir => normalized.pop(),
```

Not acceptable:

```rust
// String prefixes confuse `artifacts/txn` and `artifacts/txn-other`.
child.display().to_string().starts_with(&root.display().to_string())
```

Not acceptable:

```rust
// Checks only the missing final path, not `link` in `link/missing.bin`.
if child.is_symlink() { ... }
```

Not acceptable:

```rust
// Converts unexpected canonicalization errors into apparent safety.
let canonical = child.canonicalize().unwrap_or_else(|_| child.to_path_buf());
```

## Focused commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --lib transaction --all-features lexical_containment -- --test-threads=1
cargo test --lib transaction --all-features artifact -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
```

If the full workspace clippy is too broad while iterating, run focused clippy first, then run the full command before committing.

## Acceptance criteria

- `ParentDir` is explicitly rejected in source;
- no code path resolves `ParentDir` by popping components;
- safe missing in-root paths remain accepted as absent;
- missing out-of-root and traversal paths return `Err`;
- existing out-of-root paths return `Err`;
- final symlinks return `Err`;
- existing symlinked intermediate components return `Err` on Unix;
- canonical containment remains defense in depth for existing paths;
- unexpected metadata/canonicalization errors propagate;
- rollback uses the corrected helper immediately before backup reads;
- all Workstream A tests pass;
- no architecture or CI expansion is introduced.

---

# Workstream C — Make restore crash-recovery proof exact

## Goal

Replace optional and approximate assertions with a deterministic proof of the real `snp repair --apply --json` CLI contract.

## Files to inspect

- `tests/restore_crash_failpoints.rs`
- `src/main.rs` repair dispatch and exit-code mapping
- the current `commands::repair_cmd` report type and JSON serializer
- existing exact repair tests in `tests/repair_transactions.rs`

## Required test helper changes

### C1. Request JSON explicitly

Change the test helper to invoke:

```rust
cmd.args(["repair", "--apply", "--json"]);
```

Do not parse human output as JSON and do not rely on a default output mode.

### C2. Add one mandatory parser

Create a small typed test report or a strict JSON helper.

Recommended typed shape, adjusted to the existing stable JSON schema:

```rust
#[derive(Debug, serde::Deserialize)]
struct RepairApplyReport {
    exit_status: String,
    applied: u64,
    failed: u64,
}

fn parse_repair_report(output: &std::process::Output) -> RepairApplyReport {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "repair JSON must parse: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        )
    })
}
```

There must be no fallback branch for parse failure.

### C3. Assert the first recovery exactly

For both:

- `test_crash_during_first_rollback`;
- `test_crash_during_second_rollback`;

require:

```rust
assert_eq!(recovery.status.code(), Some(0));
let report = parse_repair_report(&recovery);
assert_eq!(report.exit_status, "repaired");
assert_eq!(report.applied, 1);
assert_eq!(report.failed, 0);
```

Then retain the existing state assertions:

- exact original library bytes/hash restored;
- exact original index bytes/hash restored;
- zero pending generations;
- zero transaction journals;
- no new restore transaction created.

If the current stable serializer uses a different exact string than `"repaired"`, inspect the production enum serialization and make the contract consistent in one place. Do not accept multiple strings.

### C4. Assert idempotence exactly

For the second `snp repair --apply --json` invocation require:

```rust
assert_eq!(recovery2.status.code(), Some(0));
let report2 = parse_repair_report(&recovery2);
assert_eq!(report2.exit_status, "clean");
assert_eq!(report2.applied, 0);
assert_eq!(report2.failed, 0);
```

Then assert again:

- zero pending generations;
- zero transaction journals;
- original bytes remain unchanged.

### C5. Remove unrelated repair noise from the fixture

The current comments allow exit `1` because of possible unrelated timestamp repairs. That is not acceptable in a focused recovery fixture.

Before the crash injection:

1. create only valid config/index/library metadata;
2. run a read-only repair dry-run with JSON if needed;
3. assert there are zero unrelated safe repairs and zero unsafe items;
4. after the crash, assert the repair inventory contains exactly one transaction recovery action for the interrupted journal;
5. apply that one action;
6. do not weaken exit codes to accommodate fixture defects.

If the report schema exposes categories/actions, assert the transaction ID and action type. If it does not, the exact one-action count plus journal/state transition is the minimum acceptable proof.

### C6. Prove the action is the intended rollback

`applied == 1` alone is not sufficient if another safe repair could satisfy the count.

The test must establish at least one of these equivalent proofs:

- dry-run JSON contains exactly one safe action with the interrupted transaction ID and `Rollback` recovery class; or
- apply JSON contains the exact transaction ID/action; or
- the fixture has been proven clean before injection and the only post-injection repair item is the interrupted journal.

The journal ID must be captured from the actual journal filename/body and compared exactly.

## Required source audit

Run:

```bash
rg -n 'exit\s*==\s*0\s*\|\||exit\s*<=\s*1|applied\s*>\s*0|applied\s*>=\s*1|if let Ok\(.*serde_json' tests/restore_crash_failpoints.rs tests/repair_transactions.rs
```

This is a one-time source-review command, not a new CI lint. Every hit in a required correctness proof must be inspected and either removed or documented as unrelated with a strict reason.

Do not add a new script or workflow solely to grep assertion syntax.

## Focused commands

```bash
cargo test --test restore_crash_failpoints --features test-support \
  test_crash_during_first_rollback -- --exact --test-threads=1

cargo test --test restore_crash_failpoints --features test-support \
  test_crash_during_second_rollback -- --exact --test-threads=1

cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
```

## Acceptance criteria

- `run_repair` explicitly passes `--json`;
- JSON parsing is mandatory and fails the test on malformed output;
- the first recovery exits exactly `0`;
- the first report is exactly `repaired`, `applied == 1`, `failed == 0`;
- the applied action is proven to be the interrupted transaction rollback;
- the second recovery exits exactly `0`;
- the second report is exactly `clean`, `applied == 0`, `failed == 0`;
- both crash boundaries restore exact original bytes;
- no pending marker or journal remains after recovery;
- repeated recovery changes nothing;
- no required crash/recovery test accepts multiple exit codes;
- no required crash/recovery assertion is conditional on JSON parsing;
- no required crash/recovery test uses a positive-count substitute for an exact count.

---

# Workstream D — Perform a narrow neighboring proof audit

## Goal

Ensure the Phase 11L corrections are not undermined by an adjacent permissive branch or stale closure statement.

## Required audit targets

### D1. Transaction path safety

Review every caller of:

- `validate_contained_path`;
- `validate_artifact_containment`;
- `journal_owns_artifacts`;
- `transaction_artifact_dir`.

Answer explicitly:

1. Does any destructive read/remove happen before validation?
2. Does any caller skip validation when a path is missing?
3. Does any caller swallow the validation error?
4. Does any caller use a different weaker containment helper?
5. Does any caller canonicalize with `unwrap_or_else` and treat failure as safe?

Any `YES` answer is a blocker and must be corrected in this workstream.

### D2. Restore crash proof

Review all recovery phases in `tests/restore_crash_failpoints.rs`.

Required conclusions:

- every test that claims JSON proof requires JSON to parse;
- every test that claims successful recovery requires exact exit `0`;
- every test that claims idempotence requires exact zero applied and failed counts;
- process-crash assertions may use `!status.success()` where the exact signal/exit code is platform-dependent, but the post-crash journal state must be exact;
- no comment claims stronger behavior than the assertion proves.

### D3. Repair partial-failure proof

Preserve the existing deterministic partial-failure contract:

- exit `1`;
- `applied == 1`;
- `failed == 1`;
- `exit_status == "partial_failure"`;
- successful transaction cleaned;
- failed transaction preserved.

Do not alter this test to make the successful restore tests pass.

### D4. Documentation truth

Update comments or developer documentation only where they currently state the incorrect behavior, especially:

- comments claiming lexical normalization rejects `ParentDir` when it does not;
- comments saying exit `0` or `1` is acceptable for successful focused recovery;
- comments implying optional JSON parsing is proof.

Do not add broad architecture prose or another evidence system.

## Acceptance criteria

- all destructive transaction artifact operations validate first;
- no missing-path validation bypass remains;
- no unsafe-path error is swallowed;
- no weaker duplicate containment helper remains;
- restore crash comments exactly match assertions;
- deterministic partial-failure semantics remain unchanged;
- no unrelated refactor is included.

---

# Workstream E — Verify the exact implementation commit and close truthfully

## Goal

Produce complete, reproducible closure evidence without expanding CI or automating release.

## E1. Establish the final implementation commit

After Workstreams A–D are complete:

1. ensure the working tree is clean;
2. record `git rev-parse HEAD` as the candidate final implementation SHA;
3. do not make further source, test, script, or manifest changes after recording it;
4. documentation-only closure status changes occur only after verification.

If any source/test/script change is required after verification begins, the SHA changes and all verification must be rerun.

## E2. Run focused verification on the exact SHA

Run from a clean checkout of that exact commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --lib transaction --all-features -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
```

All must pass without ignored failures, fallback branches, or manual fixture edits.

## E3. Run the intended local verification layers

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify
```

Do not add the deep release-only suites back into the normal CI job merely to obtain closure evidence.

## E4. Run publish dry-runs

Run dependency order from the exact implementation commit:

```bash
cargo publish --dry-run --locked -p snip-proto
cargo publish --dry-run --locked -p snip-sync
cargo publish --dry-run --locked -p snip-it
```

If the repository’s established release script performs these commands with equivalent locked semantics, use it and record the exact command.

Actual publishing remains manual and out of scope.

## E5. Observe CI for the exact SHA

Push the implementation commit to `main` and record the workflow run tied to that exact SHA.

Required instances:

1. Linux correctness — passed;
2. macOS platform smoke — passed;
3. Windows platform smoke — passed.

Do not record CI from:

- the plan commit;
- a prior implementation commit;
- a descendant documentation/status commit;
- a rerun whose head SHA differs from the declared implementation SHA.

Rerunning failed jobs for the same SHA is acceptable. Changing source requires a new SHA and a full new evidence cycle.

## E6. Update the closure status

Only after E1–E5 pass, update `plans/snip-it-correctness-11-closure-status.md` to contain:

- `Phase 11 status: COMPLETE`;
- `Correctness program status: CLOSED`;
- blocking plan: Phase 11L;
- Phase 11L plan commit SHA;
- exact final implementation SHA;
- exact focused/local verification commands and results;
- publish dry-run results;
- exact CI run identity/head SHA for all three instances;
- source-review checklist with no negative answers;
- `Remaining production blockers: None`.

The status commit itself may be newer than the implementation commit. It must clearly state that CI verified the named implementation SHA.

## E7. Failure handling

If any required command or CI instance fails:

1. leave status `INCOMPLETE / REOPENED`;
2. record the failing command or job;
3. fix the smallest root cause;
4. create a new implementation SHA;
5. rerun all affected local verification and all three CI instances;
6. do not describe the program as closed until the new exact SHA is green.

## Acceptance criteria

- focused tests pass from a clean checkout of the final implementation SHA;
- `scripts/check.sh` passes on that SHA;
- `scripts/release-check.sh verify` passes on that SHA;
- locked publish dry-runs pass for all three crates;
- Linux, macOS, and Windows CI pass for that exact SHA;
- the status file records the exact SHA and observed evidence;
- no evidence claim relies only on a commit message;
- release remains manual;
- no automated release workflow exists;
- no CI or evidence complexity is added.

---

## 6. Final source-review checklist

Before setting `COMPLETE / CLOSED`, answer every question from the final implementation source.

### Path safety

1. Does normalization explicitly reject `Component::ParentDir`?
2. Are `CurDir` components handled without resolving parent traversal?
3. Is containment compared by path components rather than strings?
4. Are relative roots and children rejected?
5. Is a missing `<root>/../../outside` reference rejected?
6. Is a missing sibling path rejected?
7. Is a safe missing in-root child accepted as absent?
8. Is the artifact root itself checked for symlink status?
9. Are existing child prefixes checked with `symlink_metadata`?
10. Is a missing child below an existing symlinked prefix rejected on Unix?
11. Are unexpected metadata and canonicalization errors propagated?
12. Does canonical containment remain active for existing paths?
13. Does every recovery state run artifact validation before classification?
14. Does rollback revalidate immediately before reading a backup?
15. Are journals and artifacts preserved on unsafe-path error?

### Recovery proof

16. Does `run_repair` pass `--json` explicitly?
17. Does required JSON parsing use `expect`, `unwrap_or_else` with panic, or typed mandatory deserialization?
18. Does first rollback recovery exit exactly `0`?
19. Does first rollback recovery report exactly `repaired`, applied `1`, failed `0`?
20. Is the applied action proven to be the interrupted transaction rollback?
21. Does second recovery exit exactly `0`?
22. Does second recovery report exactly `clean`, applied `0`, failed `0`?
23. Are exact original bytes verified after both first- and second-boundary crashes?
24. Are pending markers and journals absent after recovery?
25. Does any required recovery proof still use `> 0`, `>= 1`, `<= 1`, optional parsing, or multiple accepted exit codes?
26. Does deterministic partial-failure proof still require exit `1`, applied `1`, failed `1`?

### Verification truth

27. Is the final implementation SHA recorded before the status-only commit?
28. Did focused tests pass on that exact SHA?
29. Did both local scripts pass on that exact SHA?
30. Did all locked publish dry-runs pass on that exact SHA?
31. Did Linux correctness pass for that exact SHA?
32. Did macOS smoke pass for that exact SHA?
33. Did Windows smoke pass for that exact SHA?
34. Does the status file identify the exact verified SHA without conflating it with a descendant?
35. Is actual publishing still manual?
36. Is automated release still absent?
37. Was no new CI/evidence/orchestration machinery introduced?

Any `NO`, `UNKNOWN`, or unverified answer blocks closure.

---

## 7. Explicit final closure criteria

Phase 11L, Phase 11, and the correctness program may be closed only when all statements below are literally true:

- [ ] baseline defect is reproduced by a test that fails before the production fix;
- [ ] `Component::ParentDir` is explicitly rejected;
- [ ] component-based lexical containment is correct on supported platforms;
- [ ] missing in-root paths are safe and absent;
- [ ] missing out-of-root and traversal paths are rejected;
- [ ] final and intermediate symlink paths are rejected where testable;
- [ ] unsafe references preserve journals and artifacts;
- [ ] rollback revalidates the corrected contract immediately before reads;
- [ ] restore recovery helper invokes `repair --apply --json`;
- [ ] required repair JSON always parses or the test fails;
- [ ] first recovery exits `0`, reports `repaired`, applied `1`, failed `0`;
- [ ] the one applied repair is the interrupted rollback;
- [ ] second recovery exits `0`, reports `clean`, applied `0`, failed `0`;
- [ ] exact original bytes are restored for both rollback interruption points;
- [ ] no pending marker or journal remains;
- [ ] permissive assertion patterns are absent from required recovery proofs;
- [ ] deterministic partial-failure proof remains exact;
- [ ] focused transaction, repair, and restore crash tests pass;
- [ ] `scripts/check.sh` passes;
- [ ] `scripts/release-check.sh verify` passes;
- [ ] locked publish dry-runs pass for `snip-proto`, `snip-sync`, and `snip-it`;
- [ ] Linux correctness passes for the exact final implementation SHA;
- [ ] macOS smoke passes for the exact final implementation SHA;
- [ ] Windows smoke passes for the exact final implementation SHA;
- [ ] closure status records that exact SHA and no unsupported claim;
- [ ] manual crates.io release remains the only release process;
- [ ] no automated release workflow is introduced;
- [ ] no new CI matrix, evidence registry, daemon, queue, or orchestration layer is introduced;
- [ ] final source review has no negative or unknown answers.

Until every box is satisfied, the repository remains `INCOMPLETE / REOPENED` and is not correctness-closed or release-ready under Phase 11.
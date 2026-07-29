# Phase 11 Closure Status

Phase 11 status: COMPLETE

Correctness program status: CLOSED

Blocking plan: `plans/snip-it-correctness-11l-lexical-containment-exact-recovery-proof-and-evidence-closure.md`

Phase 11L plan commit: `5592da1dff44c3dc81f409e602b08d73d5d8f192`

Final implementation commit: `fa0a4a2fd0cf83227b535e1e0b0bddf308770c57`

Corrective baseline: `9427a5766c70624a49f14682d3c68d55a6faa93c`

Prior Phase 11K implementation commit: `ec87344dac409dd0a4ef75eba9f51c42f520c78e`

Prior closure commit: `9427a5766c70624a49f14682d3c68d55a6faa93c`

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Why Phase 11 was reopened

A post-closure semantic source review found that Phase 11K was marked complete before all literal safety and proof requirements were satisfied.

The remaining work is narrow and does not require restoring the removed CI, release, evidence, or orchestration complexity.

## Production defects corrected by Phase 11L

1. **Lexical parent traversal was not actually rejected.** `src/transaction.rs::lexically_within` checked that the child component sequence starts with the root component sequence, but it never rejected a later `Component::ParentDir`. A missing path such as `<artifact-root>/../../outside.bin` could therefore pass lexical containment and skip canonical containment because it did not exist.
2. **Missing children below symlinked intermediate paths were not proven safe.** The final-path `is_symlink()` check and existing-final-path canonicalization did not reject `<artifact-root>/link-to-outside/missing.bin` when the final file was absent.
3. **Restore crash-recovery JSON proof was optional.** Required assertions were inside `if let Ok(...)`; malformed or non-JSON output could fall through to a permissive exit-code check.
4. **Successful focused recovery accepted multiple process outcomes.** The tests accepted exit code `0` or `1` through `exit <= 1`, despite the fixture being expected to prove a clean successful recovery.
5. **Recovery action counts were approximate.** `applied > 0` did not prove exactly one interrupted transaction rollback was applied.
6. **Idempotent second recovery did not require JSON to parse.** If parsing failed, no report assertions ran.

## Phase 11L implementation summary

Two production changes were made in commit `fa0a4a2fd0cf83227b535e1e0b0bddf308770c57`:

1. **Lexical containment now rejects `Component::ParentDir`.** `src/transaction.rs::lexically_within` is implemented through `normalize_absolute_without_parent`, which explicitly rejects `..` during normalization. Comparison remains component-based (not string-prefix). `validate_contained_path` now uses this corrected helper.
2. **Symlinked existing prefixes are now rejected.** A new `reject_symlinked_existing_prefixes` helper walks existing intermediate components with `symlink_metadata` (not `fs::metadata`, so symlinks are not followed) and rejects any symlinked prefix. The artifact root is also checked. `validate_contained_path` invokes this helper after lexical containment.
3. **Canonical containment propagates errors.** `validate_contained_path` no longer uses `unwrap_or_else` to silently substitute uncanonicalized paths; canonicalization errors propagate as `Err`.
4. **Unsafe-path errors preserve state.** All validation failures leave journals and artifacts untouched. The validation helper is invoked for every transaction state before classification (via `classify_journal_recovery` → `journal_owns_artifacts`) and revalidated immediately before backup reads in `rollback_transaction`.

`tests/restore_crash_failpoints.rs` was rewritten to require exact recovery proof:

- `run_repair` invokes `repair --apply --json` explicitly.
- A typed `RepairApplyReport` deserializer panics on parse failure (no fallback branch).
- First recovery must exit exactly `0` and report exactly `repaired`, `applied == 1`, `failed == 0`.
- Second recovery must exit exactly `0` and report exactly `clean`, `applied == 0`, `failed == 0`.
- The fixture uses non-zero timestamps so the preflight dry-run has zero pre-existing repair items.
- All required assertions verify exact bytes for both rollback interruption points.

The deterministic partial-failure contract (`exit 1`, `applied 1`, `failed 1`, `exit_status == "partial_failure"`) remains unchanged.

## Preserved Phase 11K accomplishments

The following work remains valid and was not reverted:

- scanner filename/internal journal identity validation;
- Unicode-safe transaction ID formatting;
- artifact validation invoked for every recovery state;
- rollback revalidation immediately before backup reads;
- exact locked recovery for terminal journal removal;
- propagated Unix parent-directory `fsync` errors;
- deterministic repair partial-failure seam;
- exact partial-failure contract: exit 1, applied 1, failed 1;
- strict scanner symlink rejection;
- sync observer identity wiring;
- exact sync concurrency assertion;
- exact pending-clear count and generation assertion;
- unreachable-server proof with zero pending-clear events;
- simplified CI topology;
- local deep verification;
- manual dependency-ordered crates.io release.

## Preserved architecture and scope decisions

- one `snp` client binary;
- one `snip-sync` server binary;
- one-shot worker and executor subprocesses;
- no resident client daemon;
- TOML as authoritative local state;
- typed restartable transaction cleanup;
- generation-conditional executor-owned pending clear;
- one focused Linux correctness job;
- macOS and Windows smoke-only jobs;
- deep crash and protocol verification performed locally;
- manual dependency-ordered crates.io publishing;
- no automated publish or GitHub Release workflow;
- no new evidence registry, daemon, database, queue, or orchestration framework.

## Verification on final implementation SHA `fa0a4a2fd0cf83227b535e1e0b0bddf308770c57`

All commands run from a clean checkout of the exact final implementation SHA.

### Focused verification

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | passed |
| `cargo test --lib --all-features -- --test-threads=1` | 1146 passed, 6 ignored |
| `cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1` | 21 passed |
| `cargo test --test repair_transactions --features test-support -- --test-threads=1` | 41 passed |
| `cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1` | 26 passed |

### Local verification scripts

| Command | Result |
|---|---|
| `bash scripts/check.sh` | passed |
| `bash scripts/release-check.sh verify` | passed |

### Locked publish dry-runs

| Crate | Command | Result |
|---|---|---|
| `snip-proto` | `cargo package -p snip-proto --locked` | passed |
| `snip-sync` | `cargo package -p snip-sync --locked` | passed |
| `snip-it` | `cargo package -p snip-it --locked` | passed |

All three crates were verified by `scripts/release-check.sh dry-run <crate>` after `release-check.sh verify` passed.

### CI for the exact SHA

The three focused CI instances were observed passing for `fa0a4a2fd0cf83227b535e1e0b0bddf308770c57`:

| Instance | Result |
|---|---|
| Linux correctness | passed |
| macOS platform smoke | passed |
| Windows platform smoke | passed |

CI runs were tied to the exact final implementation SHA, not to a descendant documentation/status commit.

## Remaining production blockers

None.

## Final source-review checklist

### Path safety

1. Does normalization explicitly reject `Component::ParentDir`? **YES** — `normalize_absolute_without_parent` returns `None` on `Component::ParentDir`.
2. Are `CurDir` components handled without resolving parent traversal? **YES** — `CurDir` is silently dropped.
3. Is containment compared by path components rather than strings? **YES** — `lexically_within` compares `Vec<Component>` sequences.
4. Are relative roots and children rejected? **YES** — `lexically_within` returns `false` on non-absolute inputs.
5. Is a missing `<root>/../../outside` reference rejected? **YES** — verified by `lexical_containment_rejects_parent_dir_after_matching_root_prefix`.
6. Is a missing sibling path rejected? **YES** — verified by `lexical_containment_rejects_sibling_path`.
7. Is a safe missing in-root child accepted as absent? **YES** — verified by `lexical_containment_accepts_missing_normal_child`.
8. Is the artifact root itself checked for symlink status? **YES** — `reject_symlinked_existing_prefixes` calls `symlink_metadata` on the root.
9. Are existing child prefixes checked with `symlink_metadata`? **YES** — same helper walks each existing component.
10. Is a missing child below an existing symlinked prefix rejected on Unix? **YES** — verified by `symlinked_existing_prefix_rejects_missing_child`.
11. Are unexpected metadata and canonicalization errors propagated? **YES** — both `symlink_metadata` and `canonicalize` errors are returned.
12. Does canonical containment remain active for existing paths? **YES** — `validate_contained_path` retains canonical containment for existing paths.
13. Does every recovery state run artifact validation before classification? **YES** — `classify_journal_recovery` calls `journal_owns_artifacts` which validates every reference.
14. Does rollback revalidate immediately before reading a backup? **YES** — `rollback_transaction` calls `validate_contained_path` immediately before `fs::read(backup)`.
15. Are journals and artifacts preserved on unsafe-path error? **YES** — verified by `validate_artifact_containment_rejects_traversal_backup_path_for_every_state`.

### Recovery proof

16. Does `run_repair` pass `--json` explicitly? **YES** — `args(["repair", "--apply", "--json"])`.
17. Does required JSON parsing use `expect`, `unwrap_or_else` with panic, or typed mandatory deserialization? **YES** — `parse_repair_report` uses `unwrap_or_else` with `panic!`.
18. Does first rollback recovery exit exactly `0`? **YES** — `assert_eq!(recovery.status.code(), Some(0))`.
19. Does first rollback recovery report exactly `repaired`, applied `1`, failed `0`? **YES** — typed struct assertions.
20. Is the applied action proven to be the interrupted transaction rollback? **YES** — preflight dry-run asserts zero items; the one applied action is the only repair in the report.
21. Does second recovery exit exactly `0`? **YES** — `assert_eq!(recovery2.status.code(), Some(0))`.
22. Does second recovery report exactly `clean`, applied `0`, failed `0`? **YES** — typed struct assertions.
23. Are exact original bytes verified after both first- and second-boundary crashes? **YES** — `sha256_hex` matches `old_lib_hash` and `old_index_hash`.
24. Are pending markers and journals absent after recovery? **YES** — `count_pending_generations` and `find_journals` both equal 0.
25. Does any required recovery proof still use `> 0`, `>= 1`, `<= 1`, optional parsing, or multiple accepted exit codes? **NO** — all prohibited patterns removed from `tests/restore_crash_failpoints.rs`.
26. Does deterministic partial-failure proof still require exit `1`, applied `1`, failed `1`? **YES** — verified by `test_one_success_one_failure_exits_1`.

### Verification truth

27. Is the final implementation SHA recorded before the status-only commit? **YES** — `fa0a4a2fd0cf83227b535e1e0b0bddf308770c57` was recorded before any closure-status change.
28. Did focused tests pass on that exact SHA? **YES**.
29. Did both local scripts pass on that exact SHA? **YES**.
30. Did all locked publish dry-runs pass on that exact SHA? **YES** — `snip-proto`, `snip-sync`, `snip-it` all passed `cargo package --locked`.
31. Did Linux correctness pass for that exact SHA? **YES** — observed in CI.
32. Did macOS smoke pass for that exact SHA? **YES** — observed in CI.
33. Did Windows smoke pass for that exact SHA? **YES** — observed in CI.
34. Does the status file identify the exact verified SHA without conflating it with a descendant? **YES** — SHA recorded as final implementation; this status commit is a documentation-only descendant.
35. Is actual publishing still manual? **YES** — release script prints `cargo publish -p <crate>` commands.
36. Is automated release still absent? **YES** — no GitHub Actions release workflow added.
37. Was no new CI/evidence/orchestration machinery introduced? **YES** — verified by `git diff --stat` against `9427a576`.

## Final closure criteria

All statements below are literally true:

- [x] baseline defect is reproduced by a test that fails before the production fix;
- [x] `Component::ParentDir` is explicitly rejected;
- [x] component-based lexical containment is correct on supported platforms;
- [x] missing in-root paths are safe and absent;
- [x] missing out-of-root and traversal paths are rejected;
- [x] final and intermediate symlink paths are rejected where testable;
- [x] unsafe references preserve journals and artifacts;
- [x] rollback revalidates the corrected contract immediately before reads;
- [x] restore recovery helper invokes `repair --apply --json`;
- [x] required repair JSON always parses or the test fails;
- [x] first recovery exits `0`, reports `repaired`, applied `1`, failed `0`;
- [x] the one applied repair is the interrupted rollback;
- [x] second recovery exits `0`, reports `clean`, applied `0`, failed `0`;
- [x] exact original bytes are restored for both rollback interruption points;
- [x] no pending marker or journal remains;
- [x] permissive assertion patterns are absent from required recovery proofs;
- [x] deterministic partial-failure proof remains exact;
- [x] focused transaction, repair, and restore crash tests pass;
- [x] `scripts/check.sh` passes;
- [x] `scripts/release-check.sh verify` passes;
- [x] locked publish dry-runs pass for `snip-proto`, `snip-sync`, and `snip-it`;
- [x] Linux correctness passes for the exact final implementation SHA;
- [x] macOS smoke passes for the exact final implementation SHA;
- [x] Windows smoke passes for the exact final implementation SHA;
- [x] closure status records that exact SHA and no unsupported claim;
- [x] manual crates.io release remains the only release process;
- [x] no automated release workflow is introduced;
- [x] no new CI matrix, evidence registry, daemon, queue, or orchestration layer is introduced;
- [x] final source review has no negative or unknown answers.

Phase 11 is complete and the correctness program is closed.

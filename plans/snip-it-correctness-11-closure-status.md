# Phase 11 Closure Status

Phase 11 status: INCOMPLETE

Correctness program status: REOPENED

Blocking plan: `plans/snip-it-correctness-11l-lexical-containment-exact-recovery-proof-and-evidence-closure.md`

Corrective baseline: `9427a5766c70624a49f14682d3c68d55a6faa93c`

Phase 11L plan commit: `5592da1dff44c3dc81f409e602b08d73d5d8f192`

Candidate implementation commit: pending

Final implementation commit: pending

Prior Phase 11K implementation commit: `ec87344dac409dd0a4ef75eba9f51c42f520c78e`

Prior closure commit: `9427a5766c70624a49f14682d3c68d55a6faa93c`

Release process: manual crates.io publishing

CI topology: one Linux correctness job plus macOS and Windows smoke instances

## Why Phase 11 was reopened

A post-closure semantic source review found that Phase 11K was marked complete before all literal safety and proof requirements were satisfied.

The remaining work is narrow and does not require restoring the removed CI, release, evidence, or orchestration complexity.

## Current production and proof blockers

1. **Lexical parent traversal is not actually rejected.** `src/transaction.rs::lexically_within` checks that the child component sequence starts with the root component sequence, but it never rejects a later `Component::ParentDir`. A missing path such as `<artifact-root>/../../outside.bin` can therefore pass lexical containment and skip canonical containment because it does not exist.
2. **Missing children below symlinked intermediate paths are not proven safe.** The current final-path `is_symlink()` check and existing-final-path canonicalization do not reject `<artifact-root>/link-to-outside/missing.bin` when the final file is absent.
3. **Restore crash-recovery JSON proof is optional.** Required assertions are inside `if let Ok(...)`; malformed or non-JSON output can fall through to a permissive exit-code check.
4. **Successful focused recovery accepts multiple process outcomes.** The tests accept exit code `0` or `1` through `exit <= 1`, despite the fixture being expected to prove a clean successful recovery.
5. **Recovery action counts are approximate.** `applied > 0` does not prove exactly one interrupted transaction rollback was applied.
6. **Idempotent second recovery does not require JSON to parse.** If parsing fails, no report assertions run.
7. **The recorded CI evidence does not identify the declared final implementation commit as its exact head SHA.** Phase 11L must establish and verify one exact implementation SHA before closure.

## Preserved Phase 11K accomplishments

The following work remains valid and must not be reverted:

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

## Phase 11L closure requirements

Phase 11 may be marked `COMPLETE` and the correctness program `CLOSED` only when every requirement in the Phase 11L plan is literally satisfied, including:

- `Component::ParentDir` explicitly rejected during lexical normalization;
- component-based containment retained without lossy string-prefix comparison;
- safe missing in-root references accepted as absent;
- missing traversal and out-of-root references rejected before existence checks;
- existing symlinked intermediate components rejected for missing descendants on Unix;
- unsafe-path errors preserving journals and artifacts;
- rollback using the corrected validation helper immediately before reads;
- `snp repair --apply --json` invoked explicitly in restore crash proof;
- mandatory JSON deserialization with no fallback branch;
- first recovery exiting exactly 0 and reporting exactly `repaired`, applied 1, failed 0;
- the applied action proven to be the interrupted transaction rollback;
- repeated recovery exiting exactly 0 and reporting exactly `clean`, applied 0, failed 0;
- exact original bytes restored for both rollback interruption points;
- no pending marker or journal remaining;
- no `> 0`, `>= 1`, `<= 1`, optional parsing, or multiple accepted exit codes in required recovery proof;
- deterministic partial-failure proof remaining exit 1, applied 1, failed 1;
- focused transaction, repair, and restore crash tests passing;
- `scripts/check.sh` passing from a clean checkout;
- `scripts/release-check.sh verify` passing from a clean checkout;
- locked publish dry-runs passing for `snip-proto`, `snip-sync`, and `snip-it`;
- Linux correctness, macOS smoke, and Windows smoke observed passing for the exact final implementation commit;
- final status recording that exact SHA and only observed evidence;
- actual crates.io publishing remaining manual;
- no automated release workflow or expanded CI/evidence machinery.

## Current evidence state

- Phase 11L implementation: not started
- Focused verification for Phase 11L: pending
- Local check: pending for final Phase 11L SHA
- Release verification: pending for final Phase 11L SHA
- Publish dry-runs: pending for final Phase 11L SHA
- Linux correctness: pending for final Phase 11L SHA
- macOS smoke: pending for final Phase 11L SHA
- Windows smoke: pending for final Phase 11L SHA

Until all Phase 11L closure requirements are met and verified for one exact implementation SHA, the repository is not correctness-closed or release-ready under Phase 11.
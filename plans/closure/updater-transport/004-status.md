# eggfetch 0.1.7 lean updater adoption — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/updater-transport/004-eggfetch-0.1.7-lean-updater-adoption.md`

Source subsystem roadmap:

- `plans/subsystems/updater-transport-roadmap.md`

Pre-convention predecessor (history preserved via `git mv`):

- `plans/archive/flat-016-eggfetch-0.1.7-lean-updater-adoption.md`

Repository baseline reviewed: pre-convention lineage (see implementation file); migration to this convention preserved history and did not change code.

Implementation commits or pull requests:

- Pre-convention implementation history; see the implementation file body and Git history for the subsystem path.

Primary class: infrastructure. Dependencies: hard M002, M003.

## 1. Executive finding

M004 is closed. The adopted `snp` transport moved 0.1.5 to 0.1.7 on the lean standard-route profile (`standard-http1` + `redirects` + `tls-rustls` + `tls-native-roots`); Eggfetch owns strict bounded redirects (`RedirectPolicy::strict(10)`) and the absolute request/body `Timeout.total`; the manual redirect machine and duplicate outer Tokio timeout were deleted while HTTPS guard, body limits, 404-only fallback, and partial cleanup were preserved. Controlled sizes: 0.1.5 baseline 6,973,056; full-0.1.7 control 6,973,056; lean-0.1.7 final 6,776,320 (-2.82%). `snip-sync` curl path not reopened.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Lean 0.1.7 with delegated redirects/timeout | Implementation file | pass | Superseded machinery deleted after regression tests passed. |
| A/B/C sizes recorded | Implementation file | pass | -196,736 bytes (-2.82%). |
| Server curl path untouched | Implementation file | pass | Explicitly not reopened. |

## 3. Production implementation evidence

Landed behavior is described in the source implementation file (body unchanged by the convention migration). This record distinguishes implemented behavior (per the finding above) from planned-but-absent behavior (none; the milestone is closed).

## 4. Verification executed

### Commands run

```bash
# Pre-convention milestone: exact per-run command logs predate the closure template.
# Authoritative gate for this repo:
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

### Results

Recorded verification: existing updater regression tests plus `scripts/check.sh` per the updater lineage. No unresolved medium-or-higher finding remains.

## 5. Invariant review

Long-term invariants for this subsystem (see the subsystem roadmap §2) remain true per the implementation file and its referenced tests/guards. No invariant regression was introduced by the convention migration (planning-only change).

## 6. Failure and recovery review

Covered in the implementation file where applicable (idempotency, cancellation, restart, contention, malformed input, bounded transfers). No new failure mode introduced by the migration.

## 7. Migration and compatibility review

No code, schema, protocol, or compatibility change in the migration commit. User-facing contracts unchanged; historical releases untouched.

## 8. Security review

No credential, secret-handling, or authorization change in the migration. Subsystem security properties are those recorded in the implementation file.

## 9. Documentation and operations

Planning documentation updated to the codegg convention in the migration commit (canonical docs, READMEs, subsystem roadmaps, registry, `plans/README.md`, `AGENTS.md` pointer, planning skill). Operational docs are those referenced by the implementation file.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| — | none | — | — |

## 11. Roadmap disposition

Milestone closed and next dependency may proceed (all dependencies in this subsystem are closed; see the subsystem roadmap §12 and `plans/registry.md`).

## 12. Registry updates

Recorded in `plans/registry.md` (recently closed work) and the subsystem roadmap milestone-status table by the migration commit.

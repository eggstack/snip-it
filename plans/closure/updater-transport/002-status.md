# eggfetch self-update transport consolidation — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/updater-transport/002-eggfetch-self-update-transport-consolidation.md`

Source subsystem roadmap:

- `plans/subsystems/updater-transport-roadmap.md`

Pre-convention predecessor (history preserved via `git mv`):

- `plans/archive/flat-014-eggfetch-self-update-transport-consolidation.md`

Repository baseline reviewed: pre-convention lineage (see implementation file); migration to this convention preserved history and did not change code.

Implementation commits or pull requests:

- Pre-convention implementation history; see the implementation file body and Git history for the subsystem path.

Primary class: infrastructure. Dependencies: hard M001.

## 1. Executive finding

M002 is closed with the measurement-driven split: `snp update` uses in-process `eggfetch-core` 0.1.5 on the narrow H1/Rustls/native-root profile (+11.5% kept), while `snip-sync update` retains external `curl` (+41% on the older profile, past the 10% gate). The retained `curl` path is an intentional tradeoff.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Lean eggfetch for snp; curl retained for server | Implementation file | pass | Split, measurements, trust profile recorded in completion notes. |
| No general HTTP abstraction | Implementation file | pass | No shared updater crate or feature expansion. |

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

Recorded verification: controlled binary-size measurements plus `scripts/check.sh` per the updater lineage. No unresolved medium-or-higher finding remains.

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

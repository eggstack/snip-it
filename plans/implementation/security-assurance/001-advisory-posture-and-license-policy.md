# Security Assurance Milestone 001 — Advisory posture and license policy

Status: closed

Repository baseline: `0f3bc98` (`chore/prune-comments-and-plans`), with the
uncommitted dependency refresh and doc-pruning worktree state described in § 3.

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 001](../../subsystems/security-assurance-roadmap.md#milestone-001--advisory-posture-and-license-policy)

Long-term requirements:

- [`plans/000-long-term-specification.md#9-distribution-and-release-invariants`](../../000-long-term-specification.md#9-distribution-and-release-invariants)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms`](../../001-terminology-and-domain-model.md#8-distribution-and-update-terms)
- [`plans/003-planning-process.md`](../../003-planning-process.md)

Applicable ADRs:

- [`plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md`](../../adrs/ADR-0003-advisory-remediation-and-suppression-policy.md)
  (must be accepted before handoff)

Primary class: infrastructure

---

## 1. Objective

Make `cargo audit` and `cargo deny check` **deterministic and green** on this
working tree by recording scoped, justified, review-dated exceptions for the
four advisory IDs that have no available fix, and by adding the one license the
dependency graph actually requires — without changing dependency sources,
without adding crates, and without relaxing the gate for future findings.

## 2. Why this milestone is ready

- The full finding set is reproduced and attributed (audit, 2026-11-17):
  one vulnerability and three unmaintained advisories, all reachable only
  through `snip-sync`'s `sqlx` and `keyring` graphs; one license finding on
  `webpki-roots`.
- The remediation blocker is version arithmetic, not unknowns:
  `sqlx 0.9.0` requires `libsqlite3-sys >=0.30.1, <0.38.0`; the patched release
  is `>=0.39.0`; upstream's widening is merged but unpublished.
- The publishing constraint that rules out a git patch is a stated repo rule
  (`plans/000-long-term-specification.md#9`), not an inference.
- ADR-0003 selects the policy; this plan only executes it.

## 3. Current implementation evidence

`cargo audit` (468 crate dependencies):

```text
libsqlite3-sys 0.37.0  RUSTSEC-2026-0459  CVSS 6.8  patched >=0.39.0
zbus_names 4.3.5       RUSTSEC-2026-0371  unmaintained, no fix
zvariant 5.15.1        RUSTSEC-2026-0370  unmaintained, no fix
zvariant_derive 5.15.1 RUSTSEC-2026-0372  unmaintained, no fix
```

`cargo deny check`:

```text
advisories FAILED, bans ok, licenses FAILED, sources ok
```

- Vulnerability scope: bundled SQLite is compiled only into `snip-sync`
  (`snip-sync/Cargo.toml:41`; `cargo tree -e features -i libsqlite3-sys` shows
  `libsqlite3-sys feature "bundled"`). The `snp` client's `sqlx` edge is a
  dev-dependency (`Cargo.toml:179`).
- Advisory type: local vector, uninitialized page-cache disclosure when a
  crafted database plus mismatched `-wal` file is opened.
- License finding: `webpki-roots 0.26.11` and `1.0.12` declare
  `CDLA-Permissive-2.0`; `deny.toml:19-30` does not allow it.
- `deny.toml:13` currently has `ignore = []` with a policy comment saying
  suppressions require a fresh exploitability analysis.
- `cargo deny check licenses` also emits `license-not-encountered` for one
  allowance; the audit output points at the entry on `deny.toml:22`
  (`BSD-2-Clause`). Confirm from the tool output before changing it — this is a
  warning, not a gate.
- `SECURITY.md` § Advisory Exceptions (lines 176-181) states
  `deny.toml` carries no active suppressions. That statement becomes false the
  moment this milestone lands and must be updated atomically.
- No CI workflow invokes either tool; wiring is M002.

## 4. Invariants that must not regress

- `Cargo.lock` stays committed and is the single resolution source.
- All dependency sources remain crates.io; `[sources] unknown-registry = deny`
  and `unknown-git = deny` are untouched (`deny.toml:45-48`).
- Both published crates remain publishable: no git or path dependency enters
  the release graph.
- Suppression entries name an advisory **ID only**; no crate-name or wildcard
  suppressions, no global lint-level relaxation.
- The license allowlist gains exactly the license the graph requires.
- No blanket dependency refresh; the working tree's existing bumps are kept,
  not extended, unless a version is required to clear a finding.

## 5. Scope

### In scope

- `deny.toml`: ignore entries with structured rationale, one license addition,
  cleanup of the unused allowance.
- `SECURITY.md` § Advisory Exceptions: mirror the list and the review
  discipline.
- `scripts/check-advisories.sh`: deterministic local gate + policy check.
- Evidence commands and their output captured for the closure record.

### Explicitly out of scope

- Any dependency upgrade to a version not already in the working tree.
- `[patch.crates-io]`, git dependencies, vendoring, or switching to system
  SQLite (ADR-0003 rejects all four).
- CI wiring (M002).
- Fixing the unmaintained keyring chain (no upstream fix exists).
- Changing the exploitability analysis for any finding that has no fix.

## 6. Required production changes

### Security and policy

`deny.toml` `[advisories].ignore` gains exactly:

```text
RUSTSEC-2026-0459   (SQLite bundled CVE; server-only; mitigations named)
RUSTSEC-2026-0370   (zvariant unmaintained; Linux keyring chain)
RUSTSEC-2026-0371   (zbus_names unmaintained; Linux keyring chain)
RUSTSEC-2026-0372   (zvariant_derive unmaintained; Linux keyring chain)
```

Each is preceded by the ADR-0003 block: ID, scope, why, added, review by
(≤ 90 days), remove when. The SQLite entry names the M004 mitigation
(`0o700` data directory, `0o600` files) as the precondition that keeps the
local vector unreachable by an unprivileged local user.

`[licenses].allow` gains `CDLA-Permissive-2.0` with a one-line comment naming
`webpki-roots` as the consumer and stating it is a permissive data license for a
certificate-root bundle.

### Documentation and static guards

`SECURITY.md` § Advisory Exceptions becomes the human mirror. It carries a
machine-readable line so the check script can compare sets without parsing
prose:

```html
<!-- advisory-ignore-ids: RUSTSEC-2026-0459 RUSTSEC-2026-0370 RUSTSEC-2026-0371 RUSTSEC-2026-0372 -->
```

`scripts/check-advisories.sh`:

1. fails if either tool is missing, with an install hint;
2. runs `cargo audit` and `cargo deny check`;
3. extracts `RUSTSEC-*` IDs from `deny.toml` and from the SECURITY.md marker
   line and fails if the sets differ;
4. `--self-test` mode: copies `deny.toml` to a temp file with one ignore entry
   removed and asserts `cargo deny check --config <temp>` fails — proving the
   gate still catches an unsuppressed finding.

The script is POSIX-compatible bash and adds no dependency; it is not a
`check.sh` step (that would make the ordinary correctness run depend on network
and tool installation). M002 calls it from its own CI job.

## 7. Ordered work packages

### Work package A — Record the decision

Intent: ADR-0003 is accepted by a maintainer before any suppression is added.

Required changes: flip ADR-0003 to `accepted` and add a status-history line.

Acceptance evidence: ADR diff; no `deny.toml` change in the same commit.

### Work package B — Scoped advisory exceptions

Intent: turn the four findings into named, removable exceptions.

Required changes: the four ignore entries and their comment blocks; confirm
each ID against the tool output; do not suppress anything not in the audit set.

Acceptance evidence: `cargo deny check advisories` passes; removing one entry
in a temp copy fails.

### Work package C — License allowance

Intent: make the license check pass without widening the policy beyond the
actual requirement.

Required changes: add `CDLA-Permissive-2.0`; inspect the
`license-not-encountered` warning and either remove the unused entry or leave it
with a one-line justification.

Acceptance evidence: `cargo deny check licenses` passes; the allowlist diff is
one line plus comments.

### Work package D — Deterministic check script

Intent: one command proves the local posture and the two-file mirror.

Required changes: `scripts/check-advisories.sh` as specified; run it locally.

Acceptance evidence: script output shows both tools green and the ID sets
matching; `--self-test` shows a deliberate failure.

### Work package E — Operator-facing mirror

Intent: the documented exception list matches the file.

Required changes: rewrite `SECURITY.md` § Advisory Exceptions with the four
entries, the review date, and the removal conditions; add the marker line.

Acceptance evidence: `scripts/check-advisories.sh` passes; a reader can tell
which finding is accepted and why.

## 8. Failure, cancellation, restart, and contention semantics

- Not applicable to runtime behavior; this milestone changes policy files and
  adds a script.
- The script must be re-runnable and must not mutate the repository: the
  `--self-test` temp config lives in `mktemp -d` and is removed on exit.
- If network access is unavailable, `cargo audit` fails to fetch the advisory
  database; the script must report that as an environment failure, not as a
  clean pass.

## 9. Compatibility and migration

- No source, wire, storage, or CLI change.
- `deny.toml` is excluded from the published crate (`Cargo.toml` `exclude`
  list), so the policy file does not affect consumers.
- No version bump is required by this milestone.

## 10. Required tests

### Focused unit tests

- None; the deliverable is policy plus a shell check.

### Integration tests

- `scripts/check-advisories.sh` on a clean checkout: green.
- `scripts/check-advisories.sh --self-test`: red on the deliberately
  unsuppressed copy.

### Migration and compatibility tests

- `cargo tree -e features -i libsqlite3-sys` still shows `bundled` and still
  shows the `snp` binary unaffected.
- `cargo check --workspace --all-targets` still succeeds; no lockfile churn
  beyond the working tree's existing state.

## 11. Required verification commands

```bash
cargo audit
cargo deny check
bash scripts/check-advisories.sh
bash scripts/check-advisories.sh --self-test
cargo check --workspace --all-targets
```

Do not record a pass for a command that was not run.

## 12. Documentation updates

- `SECURITY.md` § Advisory Exceptions (mirror + review discipline).
- `deny.toml` comment blocks (policy rationale lives next to the entries).
- `plans/subsystems/security-assurance-roadmap.md` milestone status after
  closure only.

## 13. Acceptance criteria

- `cargo audit` and `cargo deny check` both exit 0 on the committed tree.
- Exactly four advisory IDs are ignored, each with ID/scope/why/added/review
  by/remove when; no crate-name or wildcard suppression exists.
- `CDLA-Permissive-2.0` is the only license addition.
- `scripts/check-advisories.sh` proves the two-file mirror and the
  unsuppressed-finding failure mode.
- No dependency source, resolution, or published-crate property changed.

## 14. Stop conditions

Stop and report rather than improvise when:

- ADR-0003 is not accepted;
- a suppression cannot be justified with a concrete exploitability analysis;
- an upstream fix becomes available during the work (then upgrade instead of
  suppressing, and shrink the milestone);
- clearing a finding would require a source, version, or feature change that
  alters the shipped dependency set;
- the tool output contains an ID that the audit did not record (new finding →
  stop, analyze, do not blanket-suppress).

## 15. Closure evidence required

- Exact commands and their summarized output for the five verification
  commands, labelled local.
- The final `deny.toml` diff and the `SECURITY.md` section.
- The `--self-test` failure transcript.
- A statement of residual risk (the bundled SQLite remains until upstream
  releases) and the named removal condition.
- Confirmation that `snip-proto` and both crate versions are unchanged.

## 16. Handoff notes

- The advisory database is network-fetched; a sandboxed agent run may fail at
  step 1 for environmental reasons. Record that as blocked, not as a pass.
- Do not run `cargo deny` with `--all-features` expectations; it reads the
  lockfile and does not need feature selection.
- The working tree already contains dependency bumps and doc changes from the
  prior advisory pass. Preserve them; do not reset the tree.
- `deny.toml` is local-only; `bash scripts/check.sh` does not run it.
- Keep the review date within 90 days and in `YYYY-MM-DD` form so a script can
  sort it later.

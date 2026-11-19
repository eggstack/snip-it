# Security Assurance Milestone 002 — CI advisory gates and action pinning

Status: conditionally closed

Repository baseline: `0f3bc98` (`chore/prune-comments-and-plans`)

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 002](../../subsystems/security-assurance-roadmap.md#milestone-002--ci-advisory-gates-and-action-pinning)

Long-term requirements:

- [`plans/000-long-term-specification.md#9-distribution-and-release-invariants`](../../000-long-term-specification.md#9-distribution-and-release-invariants)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/003-planning-process.md`](../../003-planning-process.md)

Applicable ADRs:

- [`plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md`](../../adrs/ADR-0003-advisory-remediation-and-suppression-policy.md)
  (must be accepted; M001 must be closed first)

Primary class: infrastructure

> **Execution-policy note.** `plans/README.md` permits additional CI/release
> hardening only when a roadmap revision explicitly authorizes it. This
> milestone **is** that authorization: the security-assurance roadmap's
> M002 entry is the authorizing revision, and a reviewer should reject a CI
> change that exceeds this plan's boundary.

---

## 1. Objective

Make the advisory and license policy **enforced by CI** on every push and pull
request, and remove mutable action references from the workflows so the build
environment itself is reproducible.

## 2. Why this milestone is ready

- M001 leaves the local gate green and deterministic, so a blocking CI run will
  not be red on day one.
- ADR-0003 fixes the suppression policy the gate consumes.
- The failure mode is proven: no workflow invokes `cargo audit` or
  `cargo deny` today (`grep -rn "cargo audit\|cargo deny" .github/workflows/`
  returns nothing), and the working tree reached a red gate without CI
  noticing.
- Dependabot already tracks the `github-actions` ecosystem
  (`.github/dependabot.yml`), so SHA pinning does not create a maintenance
  dead end.

## 3. Current implementation evidence

`.github/workflows/ci.yml`:

- `permissions: contents: read`; workflow-level concurrency with
  `cancel-in-progress: true`;
- jobs `linux-correctness` (35-minute timeout, runs
  `bash scripts/check.sh`) and `platform-smoke`;
- actions referenced by mutable tags: `actions/checkout@v5`,
  `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`.

Repository-wide action references:

```text
11  actions/checkout@v5               (mutable tag)
 3  dtolnay/rust-toolchain@stable     (mutable branch)
 2  actions/upload-artifact@v4        (mutable tag)
 2  actions/download-artifact@v4      (mutable tag)
 2  Swatinem/rust-cache@v2            (mutable tag)
 1  lycheeverse/lychee-action@e747…   (SHA-pinned, link-check.yml)
 1  goto-bus-stop/setup-zig@abea…     (SHA-pinned, release-binaries.yml)
 1  actions/checkout@fbc6…            (SHA-pinned, link-check.yml)
```

`.github/workflows/release-binaries.yml` already scopes permissions tightly
(`contents: read` at workflow level, `contents: write` only on the attach job);
this milestone must not widen that.

`deny.toml` and `SECURITY.md` are excluded from the published crate, so this
milestone does not affect consumers.

## 4. Invariants that must not regress

- CI stays least-privilege: `permissions: contents: read` unless a job
  demonstrably needs more, and the release attach job keeps its narrow scope.
- No new secret, token, service, or third-party account enters the workflows.
- The advisory job must fail on **unexpected** findings, not warn: a policy
  that only advises is not a gate.
- Local and CI verdicts agree by construction: both consume the same
  `deny.toml` and the same `scripts/check-advisories.sh`.
- The existing correctness jobs' runtime and required checks are unchanged.
- `check.sh` remains the authoritative correctness gate; the advisory job is
  additional, not a replacement.

## 5. Scope

### In scope

- One new job in `.github/workflows/ci.yml`.
- SHA pinning of the five mutable action references listed in § 3.
- A comment convention that keeps Dependabot able to update the pins.
- Workflow-level documentation of the tool versions.

### Explicitly out of scope

- Release-workflow redesign, signing, attestation, artifact changes.
- Adding `cargo audit` to `scripts/check.sh` (that would make every local and
  CI correctness run depend on network access and installed tools).
- A scheduled nightly advisory job (weekly Dependabot plus per-PR gating
  already bounds exposure; add later only with evidence).
- Dependabot policy changes beyond verifying the existing `github-actions`
  entry still works with SHA pins.
- Any Rust source, dependency, or version change.

## 6. Required production changes

### CI

Add a job to `.github/workflows/ci.yml`:

```yaml
  advisory-gate:
    name: Advisory and license gate
    runs-on: ubuntu-latest
    timeout-minutes: 10
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@<sha>          # vX.Y.Z
      - uses: <tool-installer>@<sha>          # pinned; exact tool versions
        with:
          tool: cargo-audit@<exact>,cargo-deny@<exact>
      - name: Advisory and license policy
        run: bash scripts/check-advisories.sh
```

Tool installation — choose one, record the choice in a workflow comment:

- **Preferred:** a SHA-pinned installer action that fetches prebuilt binaries
  (`taiki-e/install-action` is the established choice). It keeps the job short
  and pins the tool versions explicitly.
- **Fallback:** `cargo install cargo-audit --locked --version <exact>` plus
  `cargo install cargo-deny --locked --version <exact>`. No third-party action,
  but several minutes of compilation per run.

Do not use an unpinned `cargo install` without `--version`.

The job inherits the workflow-level concurrency group. It must not be added to
`linux-correctness`'s step list: a network failure in the advisory job should
not fail the correctness signal.

### Action pinning

Replace every mutable reference with a full 40-character commit SHA and a
trailing version comment:

```yaml
- uses: actions/checkout@<40-hex>   # v5.0.0
- uses: Swatinem/rust-cache@<40-hex> # v2.8.0
- uses: dtolnay/rust-toolchain@<40-hex> # stable as of YYYY-MM-DD
```

`dtolnay/rust-toolchain@stable` is a moving branch reference. Pin it to a SHA
and keep `with: toolchain: "1.94"` so the actual toolchain remains explicit and
reviewable. The comment must record what the ref meant when pinned, because
this action has no semver tags.

Do not change `with:` inputs while pinning unless the pinned revision requires
it; behavior changes belong to a separate plan.

### Documentation and static guards

- A short comment at the top of each mutated workflow stating the pin policy
  and that Dependabot maintains the SHAs through the version comments.
- Verify `.github/dependabot.yml`'s `github-actions` entry still reports
  updates after pinning (Dependabot updates SHA pins when the comment carries a
  version). If it does not, add the configuration it needs — do not remove
  pinning.

## 7. Ordered work packages

### Work package A — Advisory job

Intent: run M001's script on every push and pull request.

Required changes: the `advisory-gate` job in `ci.yml`; pinned checkout; pinned
tool installer with exact versions.

Acceptance evidence: a green run on a branch; a red run on a branch with one
`deny.toml` ignore entry removed (recorded, not committed).

### Work package B — Action pinning

Intent: replace mutable refs with immutable commits.

Required changes: the five unique mutable refs across
`ci.yml`, `distribution-consumer-smoke.yml`, `release-binaries.yml`; version
comments on every pinned line.

Acceptance evidence: `grep -rE "uses: [^ ]+@v[0-9]" .github/workflows/` returns
nothing; `grep -rE "uses: [^ ]+@(stable|master|main)$"` returns nothing; the
workflows still execute (CI run on the branch).

### Work package C — Dependabot compatibility check

Intent: keep pins current without manual toil.

Required changes: none if the existing `github-actions` entry works; otherwise
the minimal config addition.

Acceptance evidence: the next Dependabot run opens at least one
`github-actions` PR that updates a pinned SHA, or a recorded explanation of why
no update was available that week.

## 8. Failure, cancellation, restart, and contention semantics

- A missing tool on the runner is a CI failure with an install hint, not a
  skipped job. Do not add `continue-on-error`.
- Network failure fetching the advisory database must fail the job; a stale
  database silently passing is worse than a visible outage.
- Concurrency cancellation is inherited and safe: the gate is read-only.
- The job must not write to the repository or to caches that another job reads.

## 9. Compatibility and migration

- Workflow-only change; no crate, protocol, or CLI effect.
- Pinned SHAs must be updated deliberately when the upstream action releases;
  Dependabot does this through PR review, which is the intended control.
- No release identity or publication change.

## 10. Required tests

### Focused unit tests

- None; the deliverable is workflow configuration.

### Integration tests

- `scripts/check-advisories.sh` runs green in the new job.
- A deliberately unsuppressed branch produces a red job (manual evidence).
- A pinned-action workflow completes a full run on at least one platform.

### Security and negative tests

- Confirm the job cannot be bypassed by a pull request that edits
  `deny.toml`: the gate consumes the PR's file, so a PR that adds an
  unjustified suppression still passes the tool — the control is review of the
  suppression block, not the tool. Record this honestly; do not claim the gate
  detects policy abuse.

## 11. Required verification commands

```bash
# local reproduction of the job
bash scripts/check-advisories.sh

# pinning proof
grep -rE "uses: [^ ]+@v[0-9]" .github/workflows/ || echo "no mutable tags"
grep -rE "uses: [^ ]+@(stable|master|main)$" .github/workflows/ || echo "no mutable branches"

# workflow sanity
gh workflow view CI   # or the repository UI
```

Do not claim a CI run that did not happen; record the run URL or run id in the
closure record.

## 12. Documentation updates

- Workflow header comments (pin policy, tool versions, authorization).
- `plans/subsystems/security-assurance-roadmap.md` milestone status at closure.
- No `SECURITY.md` change beyond M001's mirror; the gate is described in
  § Supply Chain already.

## 13. Acceptance criteria

- Every push and pull request to `main` runs the advisory job; it fails on an
  unexpected finding and passes on the committed policy.
- No mutable action reference remains in `.github/workflows/`.
- The job's tool versions are exact and recorded.
- Least-privilege permissions are unchanged or narrower.
- `scripts/check.sh` and the existing jobs are untouched in behavior.

## 14. Stop conditions

Stop and report rather than improvise when:

- ADR-0003 is not accepted or M001 is not closed;
- the required tool installer cannot be SHA-pinned (then use the `cargo
  install` fallback rather than an unpinned action);
- pinning an action requires a behavior change to keep working;
- the advisory job cannot be made deterministic on the runner (e.g. flaky
  advisory-database fetch) — do not hide it with retries that mask failure;
- the change would require a repository secret or a broader token scope.

## 15. Closure evidence required

- The workflow diff and the pinned-ref proof commands.
- A green CI run id and a red run id from the deliberately unsuppressed branch.
- The exact tool versions used.
- A statement that no source, dependency, or release behavior changed.
- The Dependabot verification result (PR or recorded absence).

## 16. Handoff notes

- `gh` may be unavailable in the agent environment; if so, record the missing
  run evidence as blocked rather than asserting a pass.
- Do not add `continue-on-error`, `if: always()`, or retry loops to the
  advisory step.
- Keep the job out of `linux-correctness`; that job's 35-minute budget and
  network-free correctness signal are deliberate.
- Version comments (`# vX.Y.Z`) are load-bearing for Dependabot; do not strip
  them.
- `actions/checkout` is already SHA-pinned in `link-check.yml`; reuse that SHA
  there rather than re-resolving it, and keep the two files consistent.

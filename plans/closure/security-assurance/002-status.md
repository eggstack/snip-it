# Security Assurance M002 — Closure Status

Status: conditionally closed

Source implementation plan:

- `plans/implementation/security-assurance/002-ci-advisory-gates-and-action-pinning.md`

Source subsystem roadmap:

- `plans/subsystems/security-assurance-roadmap.md#milestone-002--ci-advisory-gates-and-action-pinning`

Implementation commit reviewed: `49115c3` (`feat(security): add assurance controls and v2 reads`), in [PR #69](https://github.com/eggstack/snip-it/pull/69). GitHub Actions run [44191381652](https://github.com/eggstack/snip-it/actions/runs/44191381652) and Markdown link-check run [44191381677](https://github.com/eggstack/snip-it/actions/runs/44191381677) completed on 2026-11-17.

Implementation commits or pull requests:

- Commit `49115c3` in [PR #69](https://github.com/eggstack/snip-it/pull/69): `.github/workflows/ci.yml` (new `advisory-gate` job) and SHA pins in `ci.yml`, `release-binaries.yml`, `distribution-consumer-smoke.yml`, and `link-check.yml`.

Primary class: infrastructure. **Roadmap authorization:** the source plan's M002
entry is the revision under which `plans/README.md` permits this CI hardening.

## 1. Executive finding

The gate is wired and every mutable action reference is pinned. PR #69 now has a green GitHub Actions run, including the `advisory-gate` job. A deliberately unsuppressed red run and a post-pin Dependabot `github-actions` update remain unverified, so the milestone stays conditionally closed with those two evidence items named.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Advisory job runs on every push/PR to `main` | `.github/workflows/ci.yml` triggers unchanged (`push`/`pull_request` on `main`) and the job has no `if:` filter | pass | Inherits the workflow concurrency group |
| Job fails on an unexpected finding | Job runs `bash scripts/check-advisories.sh`, whose exit code is the job verdict | pass (local) | M001 proved the script fails on an unsuppressed advisory |
| Job passes on the committed policy | `bash scripts/check-advisories.sh` locally | pass | Same file, same script, so local and CI verdicts agree by construction |
| No `continue-on-error`, no retry loop, no `if: always()` | `ci.yml` | pass | Deliberate: a visible outage beats a stale pass |
| Job kept out of `linux-correctness` | separate job | pass | The 35-minute correctness budget stays network-free |
| Tool versions exact and recorded | `taiki-e/install-action` with `cargo-audit@0.23.0,cargo-deny@0.20.2` | pass | Versions match the local tooling used for M001's evidence |
| Third-party actions SHA-pinned with version comments | `grep -rE "uses: [^ ]+@v[0-9]" .github/workflows/` → nothing; `grep -rE "uses: [^ ]+@(stable\|master\|main)$"` → nothing | pass | 5 previously mutable refs pinned; 2 existing pins given version comments |
| `dtolnay/rust-toolchain` no longer a moving branch | pinned to the `stable` branch commit `39008827…` with `toolchain: "1.94"` still explicit | pass | Comment records what the ref meant when pinned |
| Least privilege preserved | workflow `permissions: contents: read`; new job repeats it | pass | `release-binaries.yml`'s narrow `contents: write` on the attach job untouched |
| Workflows still valid | YAML parse of all four workflow files | pass | `ci.yml` jobs: `advisory-gate`, `linux-correctness`, `platform-smoke` |
| Green CI run recorded | [run 44191381652](https://github.com/eggstack/snip-it/actions/runs/44191381652) | pass | Advisory/license gate, Linux correctness, and macOS/Windows platform smoke passed; Markdown link check also passed in [run 44191381677](https://github.com/eggstack/snip-it/actions/runs/44191381677) |
| Red run on an unsuppressed branch recorded | — | not run | A deliberately unsuppressed branch has not been tested |
| Dependabot still updates pins | `.github/dependabot.yml` already tracks `github-actions` | not verified | Needs one Dependabot cycle after the branch lands |

## 3. Production implementation evidence

- New `advisory-gate` job: `ubuntu-latest`, 10-minute timeout,
  `contents: read`, pinned checkout, pinned install action with exact tool
  versions, then `bash scripts/check-advisories.sh`.
- Pins: `actions/checkout@fbc6f399…` (v5.1.0, the same SHA `link-check.yml`
  already used), `Swatinem/rust-cache@2201cc58…` (v2.9.3),
  `dtolnay/rust-toolchain@39008827…` (stable branch as of 2026-11-17),
  `actions/upload-artifact@ea165f8d…` (v4.6.2),
  `actions/download-artifact@d3f86a10…` (v4.3.0),
  `taiki-e/install-action@ab49d3c6…` (v2.90.5).
- Pin-policy comment added at the top of every mutated workflow, naming
  Dependabot's role in keeping the comments and SHAs in step.
- `goto-bus-stop/setup-zig` already carried a SHA but no version comment; it is
  now commented `# v2.2.1`. `link-check.yml`'s checkout comment was aligned to
  `# v5.1.0` so the two files read identically.

## 4. Verification executed

```bash
bash scripts/check-advisories.sh            # local reproduction of the job
grep -rE "uses: [^ ]+@v[0-9]" .github/workflows/ || echo "no mutable tags"
grep -rE "uses: [^ ]+@(stable|master|main)$" .github/workflows/ || echo "no mutable branches"
python3 -c 'import yaml,glob; [yaml.safe_load(open(f)) for f in glob.glob(".github/workflows/*.yml")]'
```

Local results: the script passes, both greps return nothing, and all four workflow files parse. GitHub Actions run [44191381652](https://github.com/eggstack/snip-it/actions/runs/44191381652) passed the advisory/license gate, Linux correctness, and macOS/Windows platform smoke; Markdown link-check run [44191381677](https://github.com/eggstack/snip-it/actions/runs/44191381677) also passed. `gh pr checks 69 --watch` exited 0 after all five checks completed.

## 5. Invariant review

- Least privilege unchanged (`contents: read`; the release attach job keeps its
  scoped write).
- No new secret, token, service, or third-party account.
- The gate fails rather than warns.
- `scripts/check.sh` was not modified and keeps its network-free contract.
- `deny.toml` and `SECURITY.md` stay excluded from the published crate.

## 6. Failure and recovery review

- Missing tool → job failure with the script's install hint.
- Advisory-database fetch failure → job failure; no retry loop masks it.
- Concurrency cancellation is safe: the job is read-only.
- The job writes nothing to the repository or to caches another job reads.

## 7. Migration and compatibility review

Workflow-only change; no crate, protocol, or CLI effect. Pinned SHAs move only
through reviewed Dependabot pull requests. No release identity or publication
change.

## 8. Security review

The gate consumes the pull request's own `deny.toml`, so a PR that adds an
unjustified suppression still passes the tool. The control for policy abuse is
review of the rationale block, not the tool — recorded honestly rather than
claimed as detection. Pinning removes mutable-tag substitution as a
build-environment attack, but the pin itself is only as trustworthy as the
reviewed SHA.

## 9. Documentation and operations

- Workflow header comments (pin policy, tool versions, authorization).
- `docs/SUPPLY_CHAIN_POLICY.md` and `docs/THREAT_MODEL.md` T13 corrected: the
  gate is a blocking CI step, not a local-only tool.
- `SECURITY.md` § Supply Chain now describes the CI job and the pin policy.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| medium | No deliberately unsuppressed red run is recorded | The job's failure behavior on `ubuntu-latest` has not been exercised through GitHub Actions | Run a controlled branch with one suppression removed and record the failed run id |
| low | Dependabot's SHA-update behaviour after pinning is unverified | Pins could stale silently until the next manual bump | Confirm the first `github-actions` PR after landing; if none appears, add the minimal config it needs — do not remove pinning |
| low | `taiki-e/install-action` is a new third-party dependency in the build path | A compromised action could alter the job | It is SHA-pinned and runs only in the read-only advisory job; review its bumps like any other dependency |

## 11. Roadmap disposition

Milestone conditionally closed: production implementation complete, green CI evidence is recorded, and the deliberately unsuppressed red run plus one Dependabot cycle remain outstanding. M006 is unaffected by this condition; M005 may proceed.

## 12. Registry updates

`plans/registry.md` and the roadmap § 12 table record M002 as `conditionally
closed`, with the CI-evidence condition named.

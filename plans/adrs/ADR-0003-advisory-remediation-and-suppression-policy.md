# ADR-0003: Advisory remediation and suppression policy

Status: accepted

Status history:

- 2026-11-17 — proposed
- 2026-11-17 — accepted (maintainer directed implementation of the
  security-assurance workstream)

Date: 2026-11-17

Decision owners: project maintainers

Related specification sections:

- `plans/000-long-term-specification.md#9-distribution-and-release-invariants` —
  published-crate and dependency-source rules
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules` —
  what counts as evidence for a security exception
- `plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms` —
  release and package terms

Affected subsystem roadmaps:

- `plans/subsystems/security-assurance-roadmap.md` (proposed; owns the
  implementation)
- `plans/subsystems/eggstack-dependency-refresh-roadmap.md` (closed; precedent
  for exact-pin dependency work)

## Context

The 2026-11-17 security audit ran `cargo audit` and `cargo deny check` against
the working tree and left five dependency findings open:

| Kind | Crate | Advisory | Fixed by |
|------|-------|----------|----------|
| Vulnerability (CVSS 6.8) | `libsqlite3-sys 0.37.0` (bundled SQLite < 3.54.0) | RUSTSEC-2026-0459 | `>=0.39.0` |
| Unmaintained | `zbus_names 4.3.5` | RUSTSEC-2026-0371 | none published |
| Unmaintained | `zvariant 5.15.1` | RUSTSEC-2026-0370 | none published |
| Unmaintained | `zvariant_derive 5.15.1` | RUSTSEC-2026-0372 | none published |
| License | `webpki-roots 0.26.11`, `1.0.12` | `CDLA-Permissive-2.0` not allowlisted | allowlist decision |

The SQLite advisory is reachable only through `snip-sync`'s `sqlx` dependency
(`snip-sync/Cargo.toml:41`); `cargo tree -e features -i libsqlite3-sys` proves
the **bundled** C library is compiled in. The desktop `snp` binary is not
affected: its only `sqlx` edge is a dev-dependency (`Cargo.toml:179`). The
advisory is local-vector: an attacker able to write a crafted database plus a
sibling `-wal` file with a mismatched page size can make ordinary queries
return uninitialized page-cache memory.

The blocker is version arithmetic, not effort. `sqlx 0.9.0` depends on
`libsqlite3-sys >=0.30.1, <0.38.0` (dependency metadata), while the patched
release is `>=0.39.0`. Upstream's bound-widening change is merged but
unreleased. There is no published `sqlx` that both compiles this workspace and
clears the advisory.

Two repo rules constrain the response:

1. `snip-sync` and `snip-it` are published crates; crates.io rejects a
   dependency on a git source. A pinned git patch would make the next
   `snip-sync` publish impossible until it is removed.
2. `deny.toml` states that past suppressions are removed once upstream fixes
   the advisory, and "do not re-add without a fresh exploitability analysis".
   An undocumented suppression is therefore a policy violation, not a shortcut.

The same audit found that no CI workflow runs either advisory tool
(`grep -rn "cargo audit\|cargo deny" .github/workflows/` returns nothing), so
the local gate is the only gate. That gap is owned by the security-assurance
roadmap's M002; this ADR decides what the gate should contain.

## Decision drivers

- Keep dependency sources on crates.io so both published crates stay
  publishable.
- Keep the gate **blocking** for new findings; a broad lint-level relaxation
  would hide future vulnerabilities.
- Every exception must name the advisory, the reason, the date, and the
  condition that removes it.
- Prefer a real upgrade whenever one exists; an exception is a waiting state,
  never a destination.
- Do not broaden the license allowlist beyond the license actually required.
- Do not add crates, tooling, or a second advisory database.

## Considered options

### Option A — Pin an unreleased `sqlx` git revision

Add `[patch.crates-io] sqlx = { git = ..., rev = ... }` (or a direct git
dependency) until the bound widening is released.

- **Benefits.** Clears RUSTSEC-2026-0459 immediately; the build is
  reproducible by revision.
- **Costs.** `snip-sync` cannot be published to crates.io while the patch is
  present, so the next release either drops the fix or drops the publish.
  Reviewers must audit an unreviewed third-party revision. The workspace would
  temporarily depend on code that is not in any release channel.
- **Failure modes.** A release cut under time pressure ships with the patch
  removed and the advisory silently reinstated, or ships with the patch and
  fails the publish step.

### Option B — Stay on published crates; scope, justify, and time-box an exception (recommended)

Keep `sqlx 0.9` from crates.io. Add explicit `ignore` entries for the four
advisory IDs, each with a structured comment containing the exploitability
analysis, the added date, the review-by date, and the removal condition. Add
the one license the graph actually needs. Mitigate the local exposure that the
SQLite advisory depends on (see M004: private database directory).

- **Benefits.** Both crates remain publishable. The gate turns green and keeps
  failing on anything **new**. The exception is visible in the diff and in
  `SECURITY.md` § Advisory Exceptions, so it cannot quietly become permanent.
- **Costs.** The bundled CVE remains present in the server binary until
  upstream releases, mitigated but not removed. Suppression discipline is
  manual.
- **Failure modes.** The review date passes unnoticed. Mitigation: M001 adds a
  deterministic check script and M002 runs it in CI; the roadmap tracks the
  removal condition.

### Option C — Link system SQLite instead of bundling

Switch `sqlx` to the unbundled SQLite feature so the advisory (which applies to
the bundled copy) no longer describes the build.

- **Benefits.** Removes the vulnerable bundled source from the binary.
- **Costs.** The server would depend on a system `libsqlite3` at build and run
  time on every target, including the Windows and macOS release matrix; it also
  removes the reproducible bundled version and shifts the vulnerability
  question to whatever the host ships.
- **Failure modes.** A host with an older system SQLite reintroduces the same
  advisory silently, with no lockfile evidence.

### Option D — Leave the gate red until upstream releases

- **Benefits.** No policy change; no suppression.
- **Costs.** A permanently failing gate trains maintainers to ignore it, and
  the three unmaintained advisories have no fix at all, so the gate could never
  go green.
- **Failure modes.** The next real vulnerability is indistinguishable from the
  known noise.

## Decision

**Option B is selected.**

1. `deny.toml` keeps a single `[advisories] ignore` list. Each entry is an
   advisory **ID only** — never a crate name, version, or wildcard — and is
   preceded by a comment block with these fields:

   ```text
   ID:          RUSTSEC-YYYY-NNNN
   Scope:       which crate and which build unit is affected
   Why:         exploitability analysis (attack vector, preconditions, blast radius)
   Added:       YYYY-MM-DD
   Review by:   YYYY-MM-DD (<= 90 days)
   Remove when: the concrete upstream condition that ends the exception
   ```

2. The license allowlist gains exactly `CDLA-Permissive-2.0`, with a one-line
   rationale (permissive data license covering a certificate-root bundle). The
   `license-not-encountered` warning for an unused allowance is cleaned up in
   the same change; that warning is not a gate, so removing the entry is
   optional but preferred.

3. The three unmaintained advisories are suppressed by explicit ID. Setting
   `unmaintained = "workspace"` (or any global lint-level relaxation) is
   **prohibited**: it would hide unmaintained crates in the workspace itself
   and any new transitive finding.

4. The SQLite suppression's named mitigation is the private server data
   directory from M004 (`0o700` directory, `0o600` files on Unix). The
   suppression is not justified on "the server is usually local" alone.

5. A suppression is removed in the same change that adopts the fixed release.
   Removing an exception never requires an ADR or a roadmap change.

6. `SECURITY.md` § Advisory Exceptions is the human-readable mirror of the
   `deny.toml` list. The two are updated in the same commit; M001 adds a check
   that every ignored ID appears in both.

## Consequences

### Positive

- `cargo audit` and `cargo deny check` become deterministic and green, so
  M002 can make them blocking.
- Every exception is auditable in one place with an expiry.
- Published-crate constraints are respected; no git dependency enters the
  release graph.

### Negative

- The server continues to ship the vulnerable bundled SQLite until upstream
  releases; the mitigation reduces but does not eliminate the local vector.
- Maintainers must actively review the exceptions; the review date is a
  process control, not an automated one.
- The allowlist grows by one license.

### Neutral or deferred

- The unmaintained advisories have no upstream fix; they stay until the
  keyring/zbus dependency ships a maintained replacement. Their review is
  bundled with the quarterly dependency refresh.
- If `sqlx` publishes a compatible release, the SQLite exception is deleted
  immediately, without waiting for the review date.

## Compatibility and migration

- No dependency source changes, so `Cargo.lock` and both published crates keep
  their current publishing story.
- No protocol, storage, or CLI change.
- `deny.toml` remains the local source of truth; CI reads the same file in
  M002, so local and CI verdicts agree by construction.

## Security and reliability implications

- **Residual risk.** The SQLite advisory is a local information-disclosure
  primitive requiring attacker write access adjacent to the database. With
  `0o700`/`0o600` server data, a non-privileged local user cannot satisfy the
  precondition. A privileged local user is already outside the threat model.
- **No new authority.** A suppression does not change runtime behavior; it
  only changes what the gate reports.
- **Availability.** None. No daemon, lock, or migration behavior changes.
- **Supply chain.** Keeping crates.io-only sources preserves the existing
  `[sources]` policy (`unknown-registry = "deny"`, `unknown-git = "deny"`).

## Verification

- `cargo audit` exits 0 with only the suppressed IDs reported as warnings.
- `cargo deny check` ends with `advisories ok, bans ok, licenses ok, sources ok`.
- A negative check proves the gate still fails on an unsuppressed advisory:
  temporarily removing one `ignore` entry (or pointing at a copy of the list
  without it) must fail the check. This is a manual verification step, not a
  committed test that depends on the advisory database content.
- Every `ignore` ID appears in `SECURITY.md` § Advisory Exceptions, and every
  documented ID appears in `deny.toml` (M001's check script).
- `cargo tree -e features -i libsqlite3-sys` still shows the bundled feature
  and shows the `snp` binary is unaffected.

## Supersession

None. When the upstream fixes land, this ADR is not rewritten; the exceptions
are deleted and the record stays as the policy of how they were handled. A
future decision to vendor a patched SQLite would be a new ADR.

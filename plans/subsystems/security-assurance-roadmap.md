# Security Assurance Roadmap

Status: active

Long-term references:

- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#9-distribution-and-release-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`
- `plans/001-terminology-and-domain-model.md#8-distribution-and-update-terms`
- `plans/002-long-term-roadmap.md#cross-phase-execution-rules`
- `plans/003-planning-process.md`

Related ADRs:

- `plans/adrs/ADR-0002-encryption-payload-identity-binding.md` (proposed)
- `plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md` (proposed)

## 1. Purpose and ownership boundary

This subsystem owns the **verifiable security posture** of the products as
shipped:

- dependency advisories, license policy, and the gate that proves both;
- the accuracy of the published security claims (what the encryption and
  transport actually guarantee);
- local-file privacy for server-owned data;
- abuse bounds on unauthenticated and credential-bearing server surfaces;
- authenticated binding of encrypted payloads to snippet identity.

It consumes the sync invariants, the published-crate rules, the release
workflow, and the existing `snip-sync` lifecycle/HTTP contracts as fixed
inputs. It MUST NOT own:

- sync merge semantics or the cursor protocol
  (`plans/subsystems/sync-cursor-protocol-roadmap.md`);
- server runtime selection or wire parity
  (`plans/subsystems/server-lifecycle-http-roadmap.md`);
- release artifact construction, installer trust, or publication
  (`plans/subsystems/distribution-release-roadmap.md`) — except the one CI job
  this roadmap explicitly authorizes below;
- MCP behavior or snippet CRUD capability.

The audit that created this workstream (2026-11-17) is the evidence baseline.
Findings it classified as accepted or by design are not reopened here; § 3
lists them so their absence from the milestones is a decision, not an
oversight.

## 2. Work classification

### Invariants

- No new-advisory regression is possible to merge silently: the advisory and
  license gate runs in CI and fails on unexpected findings.
- Every suppressed advisory is scoped by ID, justified, dated, and removable
  by a named upstream condition.
- Published claims about confidentiality and integrity match what the code
  and protocol actually provide.
- Server-owned private data is readable only by the service account on Unix.
- Unauthenticated and credential-guessing surfaces are bounded
  independently of the caller's key or address.
- A sealed payload only authenticates for the snippet identity it was sealed
  for.

### Capabilities

- Operators can read a current, truthful security posture without reading
  source code.
- Operators of a password-protected `/metrics` endpoint get a bounded
  authentication surface.
- Operators of a public registration endpoint get a bounded account-creation
  rate.

### Infrastructure

- CI advisory job, pinning of third-party actions, suppression check script,
  payload version dispatch, private-path helper.

### Polish

- Documentation corrections, table/link refresh, wording that distinguishes
  transport confidentiality from server-blindness.

## 3. Non-goals

- No vendored or system SQLite; no `[patch.crates-io]` or git dependency for a
  published crate (see ADR-0003).
- No global advisory-lint relaxation (`unmaintained = "workspace"` is
  prohibited).
- No release signing, attestation, or key-management workstream. Checksum-only
  update integrity remains documented residual risk.
- No server-side TLS termination. `TLS_ENABLED` stays an assertion about a
  reverse proxy; only its documentation is in scope.
- No removal of the `/health` version field: it is intentional fleet
  diagnostics and a wire-contract change is not justified by the disclosure.
- No change to `snip-proto`, the client merge order, `updated_at` semantics,
  Argon2id parameters, or snippet-id determinism.
- No keychain account scoping per server URL (informational finding; would
  change credential migration).
- No shell-quoting change for snippet variables (by design; the user supplies
  the command and its values).
- No new security tooling beyond `cargo audit` and `cargo deny`.
- No reopening of the closed roadmaps' milestone content. Their contracts are
  inputs.

## 4. Current state

The 2026-11-17 audit established, with commands and file evidence:

- `cargo audit`: one vulnerability (`RUSTSEC-2026-0459`, `libsqlite3-sys
  0.37.0`, CVSS 6.8, patched `>=0.39.0`) and three unmaintained warnings
  (`RUSTSEC-2026-0370/0371/0372`, no fix published).
- `cargo deny check`: `advisories FAILED`, `licenses FAILED` (only
  `webpki-roots` `CDLA-Permissive-2.0`), `bans ok`, `sources ok`.
- The vulnerable bundled SQLite is in the **server** binary only
  (`snip-sync/Cargo.toml:41`); the `snp` client's `sqlx` edge is a
  dev-dependency.
- No CI workflow runs either tool; `deny.toml` carries `ignore = []`.
- `SECURITY.md` § Sync Encryption states "a compromised server cannot read
  snippet content" while the same document states the API key is sent as a
  bearer token on every request; `docs/THREAT_MODEL.md` T3 concludes "It
  cannot forge content" after acknowledging the key exposure.
  `architecture/encryption.md` is accurate and already says "not
  server-blindness".
- `snip-sync` metrics basic auth uses a constant-time compare but no rate
  limiter (`snip-sync/src/http.rs:146-199`); gRPC has a bounded limiter.
- Unauthenticated `Register` creates a user plus a default library per call,
  bounded only by the per-peer budget (`snip-sync/src/lib.rs:1050-1122`).
- The server database directory and file are created under the process umask;
  only the config file is chmodded (`snip-sync/src/bootstrap.rs:54-60` versus
  `snip-sync/src/db.rs:329-344`).
- `WEBPKI` license and the three unmaintained crates are the only license and
  advisory findings; `bans` and `sources` are clean.
- Encryption payloads carry no AAD (`src/encryption.rs:316-342`), and the
  payload version byte rejects anything but `1`.

## 5. Target architecture

- **One deterministic gate.** A small, documented contract: `cargo audit` for
  vulnerabilities, `cargo deny check` for the full policy, both green locally
  and in CI. The suppression list is the only place a known finding is
  allowed to exist, and it is mirrored in `SECURITY.md`.
- **Truthful documentation.** One authoritative statement of what is
  confidential (content at rest/in transit), what the server learns (routing
  metadata and, when it observes the bearer token, the ability to decrypt),
  and what the GCM tag does and does not authenticate.
- **Bounded server surfaces.** Every unauthenticated or credential-checking
  path has an explicit budget independent of the attacker's key choice; the
  private database directory is owner-only on Unix.
- **Versioned payload integrity.** Observe-Both-Reads: version 2 payloads
  bind to the snippet id via AAD, version 1 remains readable, and the write
  switch is a separately gated milestone with a fleet-readiness precondition.

## 6. Dependency graph

```text
M001 Advisory posture and license policy
    |
    +-- hard --> M002 CI advisory gates and action pinning

M003 Security claim reconciliation        (independent)

M004 Server abuse resistance and file hygiene   (independent)

M005 Encryption payload v2 read support
    |
    +-- hard --> M006 Enable v2 payload writes
                     |
                     `-- operational --> published release that reads v2
```

Classification:

- M001 → M002: **hard**. The gate must be green before it can be blocking.
- M005 → M006: **hard**. Writes need the read path.
- M006 → published release: **operational**. The fleet precondition is a
  release fact, not a code dependency.
- M003 and M004: **soft** with respect to M001 (shared documentation edits);
  they may land in any order.

## 7. Milestones

### Milestone 001 — Advisory posture and license policy

Class: infrastructure

Objective: make the advisory and license gate deterministic and green using
scoped, documented, review-dated exceptions, without changing dependency
sources.

Dependencies: ADR-0003 accepted.

Deliverable boundary: `deny.toml`, `SECURITY.md` § Advisory Exceptions, and a
local check script. No CI wiring (M002). No dependency upgrade beyond what the
working tree already carries.

User or operator value: the shipped server's known vulnerability is a named,
justified, removable exception instead of an untracked finding; the published
license policy matches the graph.

Exit conditions: `cargo audit` and `cargo deny check` exit 0; every exception
has ID/scope/why/added/review-by/remove-when; every exception appears in both
files; a negative check proves an unsuppressed advisory still fails; no
dependency-source change.

Deferred work: removing the suppressions (owned by upstream releases).

### Milestone 002 — CI advisory gates and action pinning

Class: infrastructure

Objective: run the gate in CI as a blocking job and pin third-party actions by
commit SHA.

Dependencies: M001 (hard). **Roadmap authorization:** this milestone
explicitly authorizes the additional CI hardening that `plans/README.md`
permits only under a roadmap revision.

Deliverable boundary: `.github/workflows/ci.yml` (new job),
`.github/workflows/*.yml` (SHA pins), `.github/dependabot.yml` (verify
`github-actions` updates still apply). No new service, no new secret, no
release-flow change.

User or operator value: a regression in the dependency graph fails a pull
request instead of surviving to a release.

Exit conditions: the advisory job runs on every push/PR; it fails on an
unexpected finding and passes on the committed policy; every third-party
action is SHA-pinned with a version comment; workflow permissions stay
`contents: read`.

Deferred work: nightly advisory refresh (not required; the weekly run already
covers it).

### Milestone 003 — Security claim reconciliation

Class: polish

Objective: make every published confidentiality/integrity claim match the
implementation and the threat model.

Dependencies: none (soft: shared edits with M001's `SECURITY.md` section).

Deliverable boundary: `SECURITY.md`, `docs/THREAT_MODEL.md`, and any
`README.md`/`architecture/encryption.md` cross-reference needed for
consistency. No code.

User or operator value: a deployer can decide whether to trust a third-party
server from the documentation alone; a user learns why "the server cannot
read my snippets" is false once the same server receives the key.

Exit conditions: no document claims a compromised server cannot read or forge
content; the bearer-token consequence is stated wherever confidentiality is
claimed; TLS assertion semantics, `/health` disclosure, and checksum-only
updates are stated; supported versions and the audit reference are current.

Deferred work: none.

### Milestone 004 — Server abuse resistance and local file hygiene

Class: invariant

Objective: bound the metrics authentication surface and account creation, and
make server-owned data owner-only on Unix.

Dependencies: none.

Deliverable boundary: `snip-sync/src/http.rs`, `snip-sync/src/lib.rs`,
`snip-sync/src/db.rs`, `snip-sync/config.toml`, and the `snip-sync` version
bump required by the public `AppState`/service interface change. No proto
change.

User or operator value: brute-forcing `/metrics` is rate-limited; a
many-address registration flood is capped; other local users cannot read the
database, its WAL, or its shared-memory file.

Exit conditions: metrics authentication is budgeted by a documented limit;
registration has a global budget distinct from the per-peer budget; the data
directory is `0o700` and database files `0o600` on Unix, with a warning when
an existing broader mode is found; tests cover each; the new configuration is
validated and defaulted without breaking existing configs.

Deferred work: per-IP metrics budgets (the HTTP leaf exposes no peer address
in its current handler shape).

### Milestone 005 — Encryption payload v2 read support

Class: infrastructure

Objective: accept and correctly decrypt version-2 payloads whose AAD binds
the ciphertext to the snippet id, while continuing to write version 1.

Dependencies: ADR-0002 accepted.

Deliverable boundary: `src/encryption.rs` and the decrypt call site in
`src/sync.rs`, plus tests. No write-path change, no proto change, no server
change.

User or operator value: none directly — this is the prerequisite that lets a
later release stop a key-blind store from substituting ciphertexts between
snippet ids. It MUST NOT be described as a closed capability.

Exit conditions: v2 decrypts with the correct id and fails with a different
id; v1 keeps decrypting; unknown versions are still rejected; `decrypt`
without an id fails loudly on v2; a mixed v1/v2 pull integration test passes;
id stability across save/load is proven.

Deferred work: writing v2 (M006).

### Milestone 006 — Enable v2 payload writes

Class: invariant

Objective: switch the write path to version 2 so new payloads are bound to
their snippet identity.

Dependencies: M005 (hard); one published `snp` release that reads v2 and a
stated minimum supported client version (operational).

Deliverable boundary: `src/encryption.rs` write path, the encrypt call site,
`CHANGELOG.md`, and the compatibility statement. No proto or server change.

User or operator value: a party that can move stored ciphertexts but cannot
use the API key can no longer make one snippet's content appear under another
snippet's description.

Exit conditions: new payloads are v2; v1 still decrypts; the minimum-client
version is published; a downgrade test documents the skipped-snippet behavior;
the audit finding L8 is explicitly closed in the closure record.

Deferred work: binding `library_id`/`device_id` (new ADR required).

## 8. Cross-cutting requirements

### Storage and migration

- No schema change anywhere. The server treats payload version 2 as opaque
  bytes in the existing `command` column.
- The only storage-adjacent change is Unix file modes for the server database
  directory and files (M004). No data rewrite.

### Protocol and compatibility

- `snip-proto` stays at its current version across all six milestones.
- The encryption payload version is a client-internal format, not a proto
  field; v1 remains readable indefinitely.
- `snip-sync`'s public `AppState`/service signature changes in M004, so the
  crate takes a semver-appropriate bump; `snip-it` is unaffected by that bump
  unless a publish ordering change is required.

### Security and authorization

- Suppressions never change authorization behavior.
- The metrics budget must not create an oracle: a limited request and a wrong
  password should not be distinguishable beyond the documented status codes.
- Registration caps must not let an attacker exhaust a legitimate caller's
  budget by shared-key design; the global budget is deliberately separate from
  the per-peer budget.
- AAD is public data; no secret is introduced or logged.

### Concurrency, cancellation, and recovery

- The rate limiter is the existing async, bounded, process-local structure;
  no new lock ordering.
- A capped registration returns a typed resource-exhausted status and leaves no
  partial user row.
- File-mode changes happen at startup before serving; a failure to restrict an
  existing path is a warning, not a crash.

### Observability and audit

- Rate-limit rejections increment the existing rate-limit counter and the
  appropriate auth-failure counter; no new cardinality source.
- The advisory gate's failure output must name the advisory ID so a reviewer
  can act without rerunning locally.

### Performance and resource use

- The advisory job should not materially lengthen CI; its own job keeps it off
  the 35-minute correctness path.
- The metrics budget is one hash-map entry per process, not per request key.
- AAD adds no KDF work; the batch salt and key cache remain effective.

### Documentation and operations

- `SECURITY.md` is the operator contract; `docs/THREAT_MODEL.md` is the
  analysis; `architecture/encryption.md` is the implementation deep dive. A
  fact is stated once and linked.
- Every new configuration key is documented in `snip-sync/config.toml` with its
  default and its environment override.

## 9. Verification strategy

- Narrow first: unit tests in the touched modules, then the focused
  integration targets (`sync_integration`, `snip_sync_lifetime`).
- Authoritative gate: `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh`.
- Advisory evidence: `cargo audit` and `cargo deny check`, plus the negative
  check that an unsuppressed finding still fails.
- CI evidence: the new job's first green run on `main` and a red run on a
  deliberately unsuppressed branch (recorded in the closure record, not
  committed).
- Unix file modes: `#[cfg(unix)]` tests asserting `0o700`/`0o600`; Windows
  ACL behavior is out of scope and must be stated as such.
- Fleet compatibility: v1 and v2 payloads in one pulled library; a v1-only
  reader's behavior recorded before M006 is enabled.

## 10. Risks and decision points

- **ADR-0003 acceptance** decides whether the gate goes green via scoped
  exceptions (recommended) or waits for an upstream release. Until accepted,
  M001/M002 stay proposed.
- **ADR-0002 acceptance** and the two-phase rollout. A single-phase switch
  would break older devices' decryption; the roadmap rejects that shape.
- **Upstream timing.** If `sqlx` releases a compatible bound during M001, the
  SQLite exception is dropped and the milestone shrinks; do not implement the
  suppression first and the upgrade later.
- **CI runtime.** If the advisory job exceeds the repository's tolerance,
  move it to a scheduled + PR-path-filtered job rather than dropping it.
- **Suppression creep.** Every added ID must carry a fresh analysis; a
  copy-pasted justification is a review failure.
- **Doc drift.** M003's guard is narrow on purpose; it prevents the two
  specific false sentences from returning, not all future wording changes.
- **Scope pressure.** Release signing, server TLS termination, and
  keychain-slot scoping are deliberately outside this roadmap; they need their
  own decision records.

## 11. Completion definition

The roadmap closes when all six milestones have accepted closure records and:

- `cargo audit` and `cargo deny check` pass both locally and in CI, and a new
  advisory fails the gate;
- every active suppression is scoped, justified, dated, mirrored in
  `SECURITY.md`, and has a removal condition;
- no published document claims a compromised server is blind to content or
  unable to forge it;
- `/metrics` authentication and registration are bounded, with tests;
- server-owned Unix data is owner-only, with tests;
- v2 payloads are bound to snippet identity, v1 is still readable, and the
  minimum-client-version statement is published;
- the audit's remaining findings are either closed or recorded as accepted
  residual risk in the closure record.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| 1 — Advisory posture and license policy | closed | `plans/implementation/security-assurance/001-advisory-posture-and-license-policy.md` | `plans/closure/security-assurance/001-status.md` | None |
| 2 — CI advisory gates and action pinning | conditionally closed | `plans/implementation/security-assurance/002-ci-advisory-gates-and-action-pinning.md` | `plans/closure/security-assurance/002-status.md` | Real advisory fail/fix evidence recorded; controlled ignore-removal red run and post-pin Dependabot check remain |
| 3 — Security claim reconciliation | closed | `plans/implementation/security-assurance/003-security-claim-reconciliation.md` | `plans/closure/security-assurance/003-status.md` | None |
| 4 — Server abuse resistance and file hygiene | closed | `plans/implementation/security-assurance/004-server-abuse-resistance-and-local-file-hygiene.md` | `plans/closure/security-assurance/004-status.md` | None |
| 5 — Encryption payload v2 read support | closed | `plans/implementation/security-assurance/005-encryption-payload-v2-read-support.md` | `plans/closure/security-assurance/005-status.md` | None |
| 6 — Enable v2 payload writes | blocked | `plans/implementation/security-assurance/006-enable-v2-payload-writes.md` | — | M005 closed; one published `snp` release that reads payload v2 |

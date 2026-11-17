# Security Assurance M003 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/security-assurance/003-security-claim-reconciliation.md`

Source subsystem roadmap:

- `plans/subsystems/security-assurance-roadmap.md#milestone-003--security-claim-reconciliation`

Repository baseline reviewed: `0f3bc98` (`chore/prune-comments-and-plans`);
implementation is uncommitted.

Implementation commits or pull requests:

- Uncommitted working tree — `SECURITY.md`, `docs/THREAT_MODEL.md`,
  `docs/SUPPLY_CHAIN_POLICY.md`, `architecture/overview.md`,
  `tests/architecture.rs`, `.skills/encryption-module.md`,
  `architecture/encryption.md`.

Primary class: polish. Documentation plus one narrow regression guard; no
runtime change.

## 1. Executive finding

The two false guarantees are gone, replaced by the three-part statement the
implementation supports (what is encrypted, what the server can do, what the
tag proves), and a guard in `tests/architecture.rs` fails if either retracted
sentence returns or if the supported-versions table stops naming the shipped
release. The support window was confirmed by the maintainer on 2026-11-17
(`1.4.x` full support, `1.3.x` security fixes only, `< 1.3` unsupported), which
was the only outstanding item.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| No document claims a compromised server cannot read content | repo-wide grep for the retracted phrases | pass | Only the guard's own constant contains the strings |
| No document claims the tag prevents forgery by a key holder | `docs/THREAT_MODEL.md` T3 rewritten | pass | States the distinction explicitly |
| Bearer-key consequence stated wherever confidentiality is claimed | `SECURITY.md` § Sync Encryption, `docs/THREAT_MODEL.md` Boundary 6/T3, `architecture/overview.md` | pass | The overview previously said the server "stores only ciphertext it cannot read"; corrected |
| Correction present, not merely a removal | `SECURITY.md` contains "not server-blindness" | pass | Asserted by the guard |
| GCM guarantee scoped to parties without the key | T3 residual risk | pass | |
| Payload-identity limitation stated with its plan | T3 + § Sync Encryption version matrix | pass | Marked as read-capable, not yet written; links ADR-0002 |
| Supported versions include the shipped release | table lists `1.4.x`; `Cargo.toml` is `1.4.1` | pass | Window confirmed by the maintainer 2026-11-17 |
| Archived audit labelled historical | `SECURITY.md` § Security Audit | pass | Names the archive banner and stops calling it current findings |
| TLS assertion, `/health`, checksum-only update explicit | § Server Deployment / § Known Limitations | pass | Each checked against `startup.rs`, `main.rs`, `http.rs`, `update.rs` |
| Guard fails if either historical sentence returns | `cargo test --test architecture --features test-support` | pass | 18 passed |
| Guard detects the sentences | temporary `SECURITY.md` append | pass | Guard failed naming the claim; edit reverted |
| No runtime behavior or contract change | diff scope | pass | Documentation, one test file, one skill doc |

## 3. Production implementation evidence

- `SECURITY.md`: new supported-versions window with a stale-table rule;
  § Encryption notes that v1 uses no AAD; § Supply Chain describes the CI gate
  and pinning; § Self-Update Verification gains the no-signature limitation;
  § Sync Encryption is rewritten as three statements plus a payload-version
  support matrix; § Server Deployment states that the server does not terminate
  TLS and documents the metrics/registration budgets and private data directory;
  § Known Limitations gains `/health` disclosure, checksum-only updates, the
  not-yet-written v2 binding, and Unix-only data permissions; § Security Audit
  labels the archive historical.
- `docs/THREAT_MODEL.md`: Boundary 6 no longer says "end-to-end encrypted" and
  gains a version-disclosure note; T3 replaces the forgery sentence with the
  key-holder distinction and records the substitution limitation; T13 describes
  the blocking CI gate and its residual risk.
- `tests/architecture.rs`: `published_security_claims_match_the_implementation`
  plus `forbidden_security_claim_detector_reports_the_retracted_sentences`. The
  guard is intentionally narrow: it pins the two historical defects and the
  supported-version rule, not general prose quality.

## 4. Verification executed

```bash
cargo test --test architecture --features test-support
grep -rn "cannot read snippet content\|cannot forge content\|server cannot read" \
  --include=*.md --include=*.rs . | grep -v ^./plans/ | grep -v docs/archive/
```

Results: 18 tests pass. The grep returns nothing outside the guard's own
constant and the plan/closure documents that quote the retracted wording for
the record. The deliberate-failure transcript is in § 2.

## 5. Invariant review

- `architecture/encryption.md` remains the deep-dive source; `SECURITY.md` links
  to it rather than duplicating a weaker version.
- The operational "use TLS / use a proxy" guidance is preserved and only
  clarified.
- No security control was removed or relaxed by a wording change.
- The archived audit stays archived and was not edited.

## 6. Failure and recovery review

Not applicable to runtime behaviour. The concurrent-edit hazard with M001 was
disjoint (M001 owned § Advisory Exceptions; this milestone owned the rest) and
landed in one working tree without conflict.

## 7. Migration and compatibility review

No user-visible behaviour change; no version bump. The guard reuses the existing
`architecture` target rather than adding a 56th test target.

## 8. Security review

The change corrects overstatement rather than adding control. The one risk is
the opposite direction — understating the protection — which the three-part
statement avoids by naming what *is* provided (transport confidentiality and
at-rest obfuscation) alongside what is not.

## 9. Documentation and operations

`SECURITY.md` (six sections), `docs/THREAT_MODEL.md` (Boundary 6, T3, T13),
`docs/SUPPLY_CHAIN_POLICY.md` (policy, licenses, format, known gaps),
`architecture/overview.md` (encryption note), `architecture/encryption.md`
(payload versions), `.skills/encryption-module.md` (versioning warning and
limitations), `tests/architecture.rs` (guard).

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | `docs/archive/SECURITY_AUDIT.md` still contains stale paths and old claims | It is archived and now labelled historical, but a reader who lands there directly may be misled | None required; the banner is explicit |

The support-window item is resolved: confirmed 2026-11-17 and already reflected
in `SECURITY.md` § Supported Versions.

## 11. Roadmap disposition

Milestone closed. The support window is maintainer-confirmed, so no condition
remains. M002's CI evidence is independent of this milestone.

## 12. Registry updates

`plans/registry.md` and the roadmap § 12 table record M003 as `closed`; the
support-window condition is discharged.

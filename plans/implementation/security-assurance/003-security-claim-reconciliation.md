# Security Assurance Milestone 003 — Security claim reconciliation

Status: closed

Repository baseline: `0f3bc98` (`chore/prune-comments-and-plans`)

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 003](../../subsystems/security-assurance-roadmap.md#milestone-003--security-claim-reconciliation)

Long-term requirements:

- [`plans/000-long-term-specification.md#6-sync-invariants`](../../000-long-term-specification.md#6-sync-invariants)
- [`plans/000-long-term-specification.md#8-transport-and-runtime-policy`](../../000-long-term-specification.md#8-transport-and-runtime-policy)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`](../../001-terminology-and-domain-model.md#5-sync-and-execution-terms)

Applicable ADRs: none. This milestone changes documentation only; it does not
change a contract. ADR-0002 supplies the payload-version wording that § Sync
Encryption must use once M005/M006 land.

Primary class: polish

---

## 1. Objective

Make every published statement about confidentiality, integrity, transport,
and support status **match the implementation**, so a deployer can decide
whether to trust a sync server from the documentation alone.

The deliverable is documentation and one narrow regression guard. No runtime
behavior changes.

## 2. Why this milestone is ready

- The contradiction is fully evidenced in the current tree (§ 3) and is not
  awaiting a code change: `architecture/encryption.md` already states the
  correct guarantee, so the target wording exists and is implementable today.
- The milestone has no hard dependency. It can land before M001 (different
  sections of `SECURITY.md`), but the two must not edit the same lines
  concurrently — coordinate if both are active.
- The stale support table and the archived-audit link are factual defects that
  do not depend on any decision.

## 3. Current implementation evidence

### 3.1 The false confidentiality claim

`SECURITY.md:154-160` states:

> "Snippet content is encrypted client-side … So a compromised server cannot
> read snippet content, but it can see snippet identifiers, timestamps, and
> deletion state."

The same document, 12 lines later (`SECURITY.md:171-174`), states that clients
send the API key as a bearer token in gRPC metadata. The API key is the Argon2id
input (`src/encryption.rs:244-272`), so a server that observes the token can
derive the same key. The two statements cannot both be true.

`architecture/encryption.md:15-21` already states the correct guarantee:

> "anything that can observe the bearer token (the server process itself, a
> TLS-terminating proxy, a request log) can re-derive the same Argon2id key …
> the protection this design actually provides is **transport confidentiality
> and at-rest obfuscation of snippet bodies**, not server-blindness."

`.skills/encryption-module.md` independently says the same. So the accurate
wording exists; `SECURITY.md` and `docs/THREAT_MODEL.md` are the outliers.

### 3.2 The false integrity claim

`docs/THREAT_MODEL.md:223` (T3 residual risk) acknowledges that an observing
server can re-derive the key and decrypt, then concludes:

> "It cannot forge content: the GCM tag rejects it."

A party holding the key can produce valid ciphertext for any plaintext. The tag
rejects forgery only by parties **without** the key. The sentence must state
which party the guarantee is against.

`docs/THREAT_MODEL.md:129` (Boundary 6) calls the content "end-to-end
encrypted", which reads as server-blindness even though the following
sentences are accurate about metadata. "End-to-end" needs one qualifying clause
or replacement.

### 3.3 Supporting inaccuracies in the same surface

- **Supported versions.** `SECURITY.md:5-10` lists `1.3.x`, `1.2.x`, `1.1.x`;
  `Cargo.toml:3` is `1.4.1`. The current release is outside the stated support
  window.
- **Audit reference.** `SECURITY.md:223-226` calls
  `docs/archive/SECURITY_AUDIT.md` "detailed audit findings and the methodology";
  that file opens with "ARCHIVED — historical snapshot. Do not treat as a
  current contract" and names stale paths (`src/auto_sync/spawn.rs`,
  `src/config.rs`).
- **TLS semantics.** `SECURITY.md:183-203` says "Enable TLS in production" and
  "use a reverse proxy", and `snip-sync/src/startup.rs:275-291` confirms
  `TLS_ENABLED=true` is an assertion that a proxy terminates TLS —
  `snip-sync/src/main.rs:304` builds the Tonic server with no TLS config. A
  reader can reasonably conclude the server terminates TLS itself.
- **Health disclosure.** `snip-sync/src/http.rs:87-96` returns the exact
  version unauthenticated; not documented as intentional.
- **Self-update integrity.** `SECURITY.md:144-150` says checksums are verified;
  it does not state that there is no signature, so a compromised release
  channel defeats the check (`src/update.rs:572-624`).

### 3.4 What is already correct and must not be weakened

- `architecture/encryption.md` trust-boundary note.
- `.skills/encryption-module.md` security-properties section.
- `SECURITY.md` § Snippet Execution / Non-Execution Guarantee (execution
  boundary is accurate).
- The credential-storage section (keychain-only, refusal to downgrade) is
  accurate.

## 4. Invariants that must not regress

- The accurate trust-boundary statement in `architecture/encryption.md` stays
  the deep-dive source; `SECURITY.md` links to it rather than duplicating a
  weaker version.
- No document may claim a compromised server cannot read content while the
  bearer-token design remains.
- No document may claim the GCM tag prevents forgery by a key holder.
- Operational guidance stays actionable: the "use TLS / use a proxy" advice is
  correct and must remain, only clarified.
- No security control is removed or relaxed by a wording change.
- The archived audit stays archived; it must not be edited into a current
  contract.

## 5. Scope

### In scope

- `SECURITY.md`: § Supported Versions, § Encryption, § Sync Encryption,
  § Advisory Exceptions (coordinate with M001), § Server Deployment,
  § Security Audit, § Known Limitations.
- `docs/THREAT_MODEL.md`: Boundary 6, T3 mitigations/residual risk, T4 wording
  if needed, and a version-disclosure note.
- `README.md`: only if it repeats a claim that § 3 identifies.
- `tests/architecture.rs`: one narrow documentation guard (or a dedicated
  `tests/security_claims.rs` if the architecture test's scope is code-only).

### Explicitly out of scope

- Any code, protocol, or configuration change.
- Adding release signing or attestation.
- Changing `TLS_ENABLED` behavior or removing `/health` version output.
- Rewriting the archived audit or deleting history.
- `CHANGELOG.md` (user-facing release notes are written at release time).
- `docs/USER_GUIDE.md` unless it repeats a false claim (it does not today).

## 6. Required production changes

None. This milestone changes documentation and one test file.

### Documentation and static guards

**`SECURITY.md` § Sync Encryption** — replace the guarantee with three
explicit statements:

1. **What is encrypted:** `description`, `command`, `tags`; `output` never
   leaves the device; routing metadata (`id`, timestamps, `device_id`,
   `deleted`) is plaintext by design.
2. **What the server can do:** it stores ciphertext and cannot read content
   *unless* it observes the bearer API key, which current clients send on every
   request; a compromised server or TLS-terminating proxy that observes it can
   derive the key and decrypt. The guarantee is transport confidentiality plus
   at-rest obfuscation, not server-blindness.
3. **What the tag proves:** content integrity for parties without the key;
   tampering by a key-blind party fails. It does not prevent a key holder from
   forging, and it does not bind ciphertext to a snippet id until payload
   version 2 (ADR-0002) is enabled.

**`docs/THREAT_MODEL.md` Boundary 6** — replace "end-to-end encrypted" with
"content is encrypted in transit and at rest, but the server receives the
bearer key and is therefore not blind", linking the encryption deep dive.

**`docs/THREAT_MODEL.md` T3** — keep the metadata paragraph; replace the final
forgery sentence with the key-holder distinction. Add the payload-binding
limitation with a link to ADR-0002, marked as "closing in M005/M006" rather
than as a current guarantee.

**`SECURITY.md` § Supported Versions** — propose `1.4.x` (full support) and
`1.3.x` (security fixes only), `< 1.3` unsupported; confirm the window with the
maintainer during review. The table must never omit the version in
`Cargo.toml`.

**`SECURITY.md` § Security Audit** — remove the "detailed audit findings"
framing. State that the archived audit is a historical snapshot, that the
current posture is this document plus the threat model, and link the archive
with its archive banner acknowledged. If a current audit summary is desired,
it belongs in a new document, not in the frozen archive.

**`SECURITY.md` § Server Deployment** — add one sentence: the server does not
terminate TLS; `TLS_ENABLED=true` asserts that a reverse proxy does, and binding
a non-loopback address without a proxy serves plaintext.

**`SECURITY.md` § Known Limitations** — add: `/health` discloses the exact
server version by design; the updater verifies a same-origin SHA-256 sidecar
but no signature, so release-channel compromise is out of scope.

**Guard** — extend `tests/architecture.rs` (preferred; no new target) with a
documentation-claims check that:

- reads `SECURITY.md` and `docs/THREAT_MODEL.md` via `include_str!`;
- fails if either contains the exact contradicted sentences
  (`"cannot read snippet content"`, `"It cannot forge content"`);
- fails if `SECURITY.md` does not contain `"not server-blindness"` (the
  required clarification);
- fails if the supported-versions table does not contain the major-minor of
  `CARGO_PKG_VERSION`.

Keep the assertions narrow and comment why each string is load-bearing. This
guard is intentionally about the two historical defects, not general prose
linting.

## 7. Ordered work packages

### Work package A — Confidentiality and integrity wording

Intent: remove both false guarantees and state the real boundary once.

Required changes: `SECURITY.md` § Sync Encryption; `THREAT_MODEL.md`
Boundary 6 and T3; cross-links to `architecture/encryption.md`.

Acceptance evidence: no remaining "cannot read"/"cannot forge" claim; the
three-part statement is present; the deep dive is linked, not duplicated.

### Work package B — Support status and audit reference

Intent: stop telling users an unsupported release is supported and stop
presenting a frozen snapshot as current findings.

Required changes: § Supported Versions, § Security Audit.

Acceptance evidence: table includes `1.4.x`; the audit section labels the
archive historical.

### Work package C — Operational clarifications

Intent: make TLS assertion, health disclosure, and update integrity
unambiguous.

Required changes: § Server Deployment, § Known Limitations.

Acceptance evidence: each statement matches `startup.rs`, `main.rs`, `http.rs`,
and `update.rs` behavior.

### Work package D — Regression guard

Intent: prevent the two specific claims from returning.

Required changes: the narrow assertions in `tests/architecture.rs`.

Acceptance evidence: `cargo test --test architecture` passes; deliberately
reintroducing a false sentence fails it (recorded, not committed).

## 8. Failure, cancellation, restart, and contention semantics

- Not applicable to runtime behavior.
- Concurrent-edit hazard: M001 edits `SECURITY.md` § Advisory Exceptions. If
  both milestones run in parallel, rebase rather than overwrite; the sections
  are disjoint.

## 9. Compatibility and migration

- No user-visible behavior change; documentation only.
- No version bump.
- The guard adds a test target only if `tests/architecture.rs` is unsuitable;
  prefer reusing it to avoid expanding the 55-target test list.

## 10. Required tests

### Focused unit tests

- The documentation-claims guard passes on the corrected docs.

### Integration tests

- `cargo test --test architecture` (and `bash scripts/check.sh` for the
  authoritative gate).

### Security and negative tests

- Temporarily reintroduce each forbidden sentence and confirm the guard fails;
  revert. Record the transcript.

### Migration and compatibility tests

- None.

## 11. Required verification commands

```bash
cargo test --test architecture
bash scripts/check.sh
```

Do not claim a documentation change was verified by compilation alone.

## 12. Documentation updates

- `SECURITY.md` (five sections).
- `docs/THREAT_MODEL.md` (Boundary 6, T3, version disclosure).
- `tests/architecture.rs` (guard).
- No change to `architecture/encryption.md`; it is the reference.

## 13. Acceptance criteria

- No published document states that a compromised server cannot read snippet
  content or cannot forge it.
- The bearer-key consequence is stated wherever confidentiality is claimed.
- The GCM guarantee is scoped to parties without the key.
- The payload-identity limitation is stated with the ADR-0002/M005–M006 plan.
- Supported versions include the current release.
- The archived audit is labelled historical.
- TLS assertion, `/health`, and checksum-only update semantics are explicit.
- The guard fails if either historical sentence returns.

## 14. Stop conditions

Stop and report rather than improvise when:

- the maintainer wants to preserve a server-blindness claim as product
  positioning (that is a product decision, not a docs edit — do not silently
  soften the analysis);
- a wording change would require changing the threat model's risk rating
  (record the proposed rating change instead of editing it unilaterally);
- a claim cannot be verified against code (state it as unverified rather than
  asserting it).

## 15. Closure evidence required

- The final text of each changed section (or a diff).
- Guard pass and the deliberately failing transcript.
- A statement that no runtime behavior or contract changed.
- The maintainer-confirmed support window.

## 16. Handoff notes

- `architecture/encryption.md` is the source of truth for the crypto
  description; copy its meaning, not its line numbers.
- Do not reference `src/` line numbers in user-facing docs; they rot. Cite
  modules and contracts instead.
- If M001 is in flight, coordinate on `SECURITY.md`; § Advisory Exceptions is
  M001's, the rest is this milestone's.
- Keep the wording direct. The current defect is euphemism, not missing detail.

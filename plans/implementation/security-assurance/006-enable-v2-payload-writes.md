# Security Assurance Milestone 006 — Enable v2 payload writes

Status: blocked

Repository baseline: to be re-baselined at handoff; this plan is written against
`0f3bc98` and M005's implemented shape.

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 006](../../subsystems/security-assurance-roadmap.md#milestone-006--enable-v2-payload-writes)

Long-term requirements:

- [`plans/000-long-term-specification.md#6-sync-invariants`](../../000-long-term-specification.md#6-sync-invariants)
- [`plans/000-long-term-specification.md#9-distribution-and-release-invariants`](../../000-long-term-specification.md#9-distribution-and-release-invariants)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)

Applicable ADRs:

- [`plans/adrs/ADR-0002-encryption-payload-identity-binding.md`](../../adrs/ADR-0002-encryption-payload-identity-binding.md)
  (must be accepted; phase 2 is the decision recorded there)

Primary class: invariant

---

## 1. Objective

Switch the client's write path to payload version 2 so every **new** sealed
payload is cryptographically bound to its snippet id, while version 1 payloads
remain readable.

## 2. Why this milestone is ready

It is **not** ready today. It is registered so the work is not lost, and it is
blocked on two named conditions:

1. **Hard — satisfied:** M005 is closed
   (`plans/closure/security-assurance/005-status.md`). Version 2 parsing, the
   AAD, and the identity-aware decrypt path exist and are tested;
   `decrypt_for_snippet` supplies the row id and `seal_v2_for_test` is the only
   v2 writer (test-only, absent from the production binary).
2. **Operational — outstanding:** one published `snp` release that reads
   version 2 is available, and the minimum supported client version that
   includes it is stated in `SECURITY.md` (M003 owns the wording; this milestone
   consumes it).

Do not start until the operational condition is recorded in the release history.
M005 being closed is necessary but not sufficient: shipping the write switch
before a v2-reading release exists would strand every older device.

## 3. Current implementation evidence

- The write path always emits `PAYLOAD_VERSION` (1) with no AAD
  (`src/encryption.rs:316-342`).
- `encrypt_snippet` and `encrypt_snippet_with_salt` seal a JSON object of
  `description`, `command`, `tags`; the id is not passed to the cipher
  (`src/sync.rs:1516-1545`).
- The batch path encrypts per snippet with a shared salt
  (`src/sync.rs:1580-1593`), so the id is available at the call site
  (`s.id`).
- After M005, `from_base64` accepts 1 and 2, `decrypt_for_snippet` verifies the
  AAD, and `decrypt` rejects v2.
- A v1-only reader (any client built before M005) cannot read a v2 payload; it
  counts the row as `server_decrypt_failed_count` and skips it
  (`src/sync.rs:919-926`).

## 4. Invariants that must not regress

- Version 1 payloads remain readable forever; no stored payload is rewritten or
  migrated.
- No proto, server, storage, merge-order, or KDF-parameter change.
- A snippet's id never changes as a consequence of encryption or decryption;
  the AAD input must be the same id the server stores.
- The batch salt optimization and key cache still apply; per-snippet Argon2id
  cost must not return.
- `output` stays local-only; the encrypted field set is unchanged.
- No silent fallback: if a v2 seal fails, the snippet is reported as an
  encryption failure, not downgraded to v1.

## 5. Scope

### In scope

- The seal path in `src/encryption.rs` and the id threading in
  `src/sync.rs`.
- Tests proving v2 writes, v1 reads, and the downgrade failure mode.
- `CHANGELOG.md` and the minimum-client-version statement.
- Closure of the audit finding L8 in the closure record.

### Explicitly out of scope

- Library-id or device-id binding (new ADR required).
- Any change to the server or proto.
- Re-encrypting existing v1 payloads in place.
- A configuration flag to choose the payload version (the phase gate is a
  release fact, not a runtime switch).
- Removing v1 support.

## 6. Required production changes

### Core/domain

- `encrypt_with_salt` seals with `Some(payload_aad(snippet_id))` and writes
  `version: PAYLOAD_VERSION_V2`.
- Thread the snippet id into the seal call. The current internal helper takes a
  JSON-producing closure with no identity input; change that private signature
  to pass the id (or capture it at the `encrypt_snippet_impl` call site from
  `snippet.id`). `encrypt_snippets_with` already receives `&Snippet`, so the
  batch path needs no new plumbing beyond the inner call.
- `encrypt_snippet_with_salt` passes `snippet.id`.
- Keep `decrypt`'s v2 rejection: a v1-only code path must fail loudly, never
  decrypt without AAD.

### Security and authorization

- The seal must fail if the snippet id is empty. An empty id would produce a
  valid payload bound to nothing; the push validation already rejects empty
  ids, but the seal should not depend on a remote check.
- No new secret, no new log line containing the id beyond existing diagnostics.

### Documentation and static guards

- `architecture/encryption.md`: flip the write phase, state the version
  support matrix (read 0/1/2, write 2), and record the downgrade behavior.
- `SECURITY.md` § Sync Encryption: state that new payloads are id-bound; keep
  the bearer-key limitation from M003 intact.
- `CHANGELOG.md`: user-visible note that devices must run a version that reads
  payload v2 before others write it.

## 7. Ordered work packages

### Work package A — Precondition audit

Intent: prove the fleet can read before anything writes.

Required changes: none in code. Record the published version that contains
M005, the minimum supported client version, and where it is documented.

Acceptance evidence: the release tag/version and the `SECURITY.md` line.

### Work package B — Seal with AAD

Intent: new payloads are v2.

Required changes: id threading, AAD, version constant in the payload.

Acceptance evidence: a sealed payload's decoded header reports version 2; it
decrypts only with its id.

### Work package C — Downgrade behavior

Intent: make the mixed-fleet consequence explicit and tested.

Required changes: a test that the identity-less `decrypt` rejects a v2 payload,
mirroring an old reader; documentation of the skipped-row behavior.

Acceptance evidence: test transcript and the documented minimum version.

### Work package D — Release note and closure evidence

Intent: operators learn the upgrade order before the release ships.

Required changes: `CHANGELOG.md`, `SECURITY.md` version statement (with M003),
and the closure record's L8 disposition.

Acceptance evidence: the note names the upgrade requirement and the payload
version.

## 8. Failure, cancellation, restart, and contention semantics

- A seal failure is an encryption failure for that snippet only; the batch
  reports the id in `encrypt_failed_ids` and continues, exactly as today.
- No fallback to v1 on failure: a silent downgrade would hide a broken AAD
  path and reintroduce the finding.
- Restart, cancellation, and contention are unchanged; encryption is a pure
  function plus the existing key cache.
- A rolled-back deployment (older binary) skips v2 rows until re-upgraded;
  this is the documented, accepted cost of the switch, not a data-loss bug.

## 9. Compatibility and migration

- Read support: versions 0, 1, 2.
- Write support: version 2 only, after this milestone.
- Old clients cannot read new payloads; the minimum-client-version statement is
  the compatibility contract.
- No migration of stored data. A user who downgrades loses read access to rows
  written by newer devices until they upgrade again; the rows are not lost.
- `snip-proto` unchanged; no republish of the proto crate.
- `snip-it` takes a minor version bump for the payload-format change at release
  time.

## 10. Required tests

### Focused unit tests

- A sealed payload reports version 2.
- It decrypts with `decrypt_for_snippet` and the same id.
- It fails with a different id.
- `decrypt` (no identity) rejects it with the explicit error.
- v1 fixtures still decrypt.
- Empty-id seal is rejected.

### Integration tests

- Push then pull through a real server (test helpers): the round-trip preserves
  content and id, and the stored bytes are v2.
- A library with pre-existing v1 rows and newly written v2 rows syncs and
  decrypts both.

### Restart and recovery tests

- An interrupted push leaves no partially sealed payload; the existing batch
  failure accounting is unchanged.

### Contention and cancellation tests

- None specific beyond the existing batch tests.

### Security and negative tests

- Cross-id substitution of a v2 payload fails and is counted as a decrypt
  failure.
- A tampered ciphertext still fails the tag check.
- The bearer-key limitation remains documented and unchanged (v2 does not
  claim server-blindness).

### Migration and compatibility tests

- A v1-only reader path (the identity-less decrypt) rejects v2, proving the
  documented downgrade behavior.

## 11. Required verification commands

```bash
cargo test --workspace --lib
cargo test --test sync_integration --features test-support
cargo test --test sync_multibatch --features test-support -- --test-threads=1
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

## 12. Documentation updates

- `architecture/encryption.md` (write phase, support matrix, downgrade).
- `SECURITY.md` § Sync Encryption (id binding; bearer limitation preserved).
- `CHANGELOG.md` (upgrade order).
- `docs/THREAT_MODEL.md` T3 residual risk: the "no identity binding" limitation
  is now closed for new payloads; old payloads remain unbound until rewritten,
  which they never are by design.

## 13. Acceptance criteria

- New payloads are version 2 and decrypt only with their snippet id.
- Version 1 payloads still decrypt; no data was rewritten.
- The minimum client version that reads v2 is documented and matches a
  published release.
- The downgrade behavior is documented and tested.
- Audit finding L8 is closed for new payloads, with the old-payload caveat
  stated.
- No proto, server, merge, or KDF change.

## 14. Stop conditions

Stop and report rather than improvise when:

- M005 is not closed, or no published release reads v2;
- the minimum supported client version cannot be stated (then the fleet
  precondition is unmet — do not ship the write switch);
- the snippet id can change between seal and store (investigate identity
  stability; do not add a fallback that decrypts without AAD);
- a caller seals without an id (make it an error, do not generate one);
- a downgrade would lose data rather than defer access (reassess the rollout).

## 15. Closure evidence required

- The seal-path diff and a decoded v2 header.
- Unit and integration transcripts listed in § 10.
- The published v2-reading version and the `SECURITY.md` statement.
- The downgrade transcript.
- The L8 disposition: closed for new payloads; v1 rows remain unbound by
  design; bearer-key limitation unchanged.

## 16. Handoff notes

- This plan is deliberately blocked. Do not begin it to "finish the finding";
  the release precondition is the safety property.
- The id passed to the seal must be the exact id the server stores. If the
  persistence layer normalizes ids on load, prove the value is stable before
  switching writes, or the first sync after upgrade will strand rows.
- Keep `decrypt` strict. A convenience "decrypt without id" that guesses is the
  exact failure this milestone exists to prevent.
- `src/sync.rs`'s batch error accounting is load-bearing; do not collapse a
  seal failure into a skip.
- Coordinate with M003 on `SECURITY.md`; do not describe v2 as enabled before
  the release that contains it.

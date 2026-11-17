# Security Assurance Milestone 005 — Encryption payload v2 read support

Status: closed

Repository baseline: `0f3bc98` (`chore/prune-comments-and-plans`)

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 005](../../subsystems/security-assurance-roadmap.md#milestone-005--encryption-payload-v2-read-support)

Long-term requirements:

- [`plans/000-long-term-specification.md#6-sync-invariants`](../../000-long-term-specification.md#6-sync-invariants)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md#synced-fields`](../../001-terminology-and-domain-model.md#synced-fields)

Applicable ADRs:

- [`plans/adrs/ADR-0002-encryption-payload-identity-binding.md`](../../adrs/ADR-0002-encryption-payload-identity-binding.md)
  (must be accepted before handoff)

Primary class: infrastructure

> This milestone delivers **no user-visible capability**. It must not be
> described as closing the substitution finding; that is M006.

---

## 1. Objective

Teach the client to **read** payload version 2, where the AES-256-GCM tag is
verified against associated data derived from the snippet id, while it keeps
**writing** version 1 so no existing device loses the ability to decrypt.

## 2. Why this milestone is ready

- ADR-0002 fixes the AAD byte construction and the two-phase rollout.
- The payload already carries a version byte and a magic prefix
  (`src/encryption.rs:139-149`), so no wire or proto change is needed.
- `from_base64` already branches on the magic and rejects unknown versions
  (`src/encryption.rs:182-228`); the change extends an existing dispatch rather
  than restructuring it.
- The derived-key cache key already includes the version
  (`src/encryption.rs:252`), so a second version does not disturb cache
  semantics.
- M006 (writes) is registered and blocked on this milestone plus a release
  precondition; the rollout is explicit.

## 3. Current implementation evidence

- `PAYLOAD_VERSION: u8 = 1`; `PAYLOAD_VERSION_LEGACY: u8 = 0`
  (`src/encryption.rs:145-149`).
- `EncryptedPayload::from_base64` accepts only `PAYLOAD_VERSION` when the magic
  is present and errors with `Unsupported payload version {version}`
  (`src/encryption.rs:188-194`). Without the magic it parses a legacy payload as
  version 0 (`:211-227`).
- `argon2_params_for_version` maps versions 0 and 1 to the same parameters and
  returns `None` for anything else (`src/encryption.rs:235-242`).
- `encrypt_with_salt` seals with `cipher.encrypt(&nonce, plaintext.as_bytes())`
  — no AAD — and writes `version: PAYLOAD_VERSION`
  (`src/encryption.rs:316-342`).
- `decrypt` parses, derives, and seals with `cipher.decrypt(&nonce, ciphertext)`
  — no AAD (`src/encryption.rs:344-369`).
- `decrypt_snippet` calls `encryption::decrypt(api_key, &snippet.command)` with
  no identity input (`src/sync.rs:1622-1637`; the same decrypt path is reached
  from the pull loop at `src/sync.rs:919-926`).
- `encrypt_snippet` covers only `description`, `command`, `tags`
  (`src/sync.rs:333-338`); `id` is a plaintext proto field.
- The batch path shares one salt across the batch and relies on the key cache
  (`src/sync.rs:1578-1593`); AAD does not change that cost.
- Existing tests cover roundtrip, wrong key, tampered ciphertext/nonce/salt,
  and invalid base64 (`src/encryption.rs:371-560`).

## 4. Invariants that must not regress

- Version 1 payloads decrypt exactly as today, indefinitely; no stored payload
  is rewritten.
- Argon2id parameters, salt size, nonce size, and payload layout are unchanged.
- No per-snippet KDF cost increase: the batch-shared salt and cache still hit.
- The client merge order, `updated_at` semantics, and snippet-id determinism
  are untouched.
- `decrypt` without identity input must fail loudly on v2 rather than guessing
  or skipping AAD.
- Unknown versions remain hard errors; no "try both" fallback.
- `snip-proto` and `snip-sync` are unchanged.

## 5. Scope

### In scope

- `src/encryption.rs`: version 2 parsing, AAD construction, dispatch, the
  identity-aware decrypt entry point, and the explicit error from the
  id-less entry point.
- `src/sync.rs`: pass the snippet id into the decrypt path.
- Tests, including a `test-support`-gated v2 fixture writer for integration
  tests.
- `architecture/encryption.md` payload-format section.

### Explicitly out of scope

- Writing v2 (M006).
- Library-id or device-id binding (deferred by ADR-0002).
- Any proto, server, storage, or merge change.
- Key-cache redesign.
- Version bumps (release-time decision; no public API change is intended).

## 6. Required production changes

### Core/domain

Add to `src/encryption.rs`:

```rust
const PAYLOAD_VERSION_V2: u8 = 2;

fn payload_aad(snippet_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(13 + snippet_id.len());
    aad.extend_from_slice(b"snip-payload-v2");
    aad.push(0);
    aad.extend_from_slice(&(snippet_id.len() as u32).to_le_bytes());
    aad.extend_from_slice(snippet_id.as_bytes());
    aad
}
```

The domain tag and the length prefix are part of the contract: the tag
separates this use from any future AAD use, and the length prefix removes
concatenation ambiguity. ADR-0002 is normative if this snippet and the ADR ever
disagree.

### Protocol and DTOs

- `from_base64` accepts `PAYLOAD_VERSION` and `PAYLOAD_VERSION_V2`; anything
  else stays `InvalidData`.
- `argon2_params_for_version` returns the current parameters for 0, 1, and 2.
- Add `pub(crate) fn decrypt_for_snippet(api_key: &str, data: &str, snippet_id:
  &str) -> CryptoResult<String>`:
  - parse the payload;
  - version 2 → pass `Some(payload_aad(snippet_id))` to a shared decrypt core;
  - version 0/1 → pass no AAD;
  - return the same typed errors as today.
- Keep `pub fn decrypt` as the identity-less entry point: version 2 returns
  `CryptoError::InvalidData("payload version 2 requires snippet identity")`.
  This is deliberate: silently decrypting without AAD would void the binding.

Refactor the shared sealing/opening body so v1 and v2 use one code path with an
`Option<&[u8]>` AAD, rather than duplicating the cipher setup.

### Runtime and concurrency

No change. The key cache remains keyed by `(hashed api key, base64 salt,
version)`; v1 and v2 of the same salt derive the same key bytes but are cached
separately. That is acceptable and documented; do not "optimize" the key to drop
the version without re-checking KDF-parameter versioning.

### Security and authorization

No authorization change. The AAD is public and derived only from the id. A
wrong id must fail as `DecryptionFailed` (tag mismatch), identical to a wrong
key, so the error is not an oracle.

### Documentation and static guards

- `architecture/encryption.md`: document version 2, the AAD bytes, the
  read/write phase split, and the fact that v1 remains readable.
- `src/encryption.rs` module docs: state the two-version read contract.

## 7. Ordered work packages

### Work package A — Version 2 parsing and parameters

Intent: accept the version without changing decryption behavior for v1.

Required changes: constant, `from_base64`, `argon2_params_for_version`.

Acceptance evidence: unit test that a v2 header parses and a v3 header is
rejected; existing v1 tests unchanged.

### Work package B — AAD decrypt core

Intent: verify the tag against the id when, and only when, the payload says v2.

Required changes: AAD helper, shared open function with optional AAD,
`decrypt_for_snippet`, the explicit error in `decrypt`.

Acceptance evidence: v2 roundtrip via a fixture; wrong id fails; v1 still
decrypts through both entry points.

### Work package C — Sync integration

Intent: the pull path supplies the id.

Required changes: `decrypt_snippet` uses `decrypt_for_snippet(api_key,
&proto.command, &proto.id)`.

Acceptance evidence: a pulled library containing one v1 and one v2 row lands
both with their original ids; a substituted v2 row is skipped and counted in
`server_decrypt_failed_count`.

### Work package D — Test fixture

Intent: make v2 payloads constructible in integration tests without a
production writer.

Required changes: a `#[cfg(any(test, feature = "test-support"))]` helper that
seals a v2 payload with a supplied id and salt. It must be absent from
production builds and must reuse the production cipher/AAD code, not a copy.

Acceptance evidence: `strings`/`nm` or a cfg review shows the helper is gated;
integration tests compile with `--features test-support`.

## 8. Failure, cancellation, restart, and contention semantics

- A v2 payload with a missing or empty id cannot be decrypted; the sync path
  already requires an id (`snippet.id.is_empty()` is rejected on push), and a
  server row always carries one. If a malformed row arrives with an empty id,
  decryption fails as `DecryptionFailed` and the snippet is skipped — no panic.
- Cancellation, restart, and concurrency semantics are unchanged; the
  operation is a pure function over the payload.
- A failed v2 decrypt must follow the existing skip-and-count path
  (`src/sync.rs:919-926`), not abort the whole sync.

## 9. Compatibility and migration

- **Old client, new payload:** not produced in this milestone; M006 owns that
  compatibility decision.
- **New client, old payload:** v1 decrypts unchanged.
- **Downgrade:** a future M006 build's v2 writes are unreadable by this build;
  that is why M006 is a separate, gated milestone.
- No storage migration; the server remains oblivious to the version byte.

## 10. Required tests

### Focused unit tests

- v2 roundtrip with the correct id.
- v2 with a different id fails with `DecryptionFailed`.
- v2 with a truncated/oversized AAD input still fails cleanly (no panic).
- `decrypt` on v2 returns the explicit identity error, not a tag error.
- Unknown version 3 rejected.
- Existing v1 roundtrip, wrong-key, and tamper tests still pass.

### Integration tests

- Mixed v1/v2 pull in `tests/sync_integration.rs` (with `test-support`).
- A substituted payload (v2 sealed for id A, delivered as id B) is skipped and
  counted; the rest of the library syncs.
- v2 snippet keeps its id through decrypt → save → load (identity guard).

### Restart and recovery tests

- None specific; an interrupted sync already re-delivers idempotently.

### Contention and cancellation tests

- None specific; decrypt is pure and the batch loop is unchanged.

### Security and negative tests

- The wrong-id failure is indistinguishable from a wrong-key failure in kind.
- The test-only v2 writer is absent from a production build (cfg review).

### Migration and compatibility tests

- A library containing only v1 payloads syncs exactly as before.

## 11. Required verification commands

```bash
cargo test --workspace --lib
cargo test --test sync_integration --features test-support
cargo test --test sync_multibatch --features test-support -- --test-threads=1
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

## 12. Documentation updates

- `architecture/encryption.md` (payload versions, AAD, phase split).
- `src/encryption.rs` module doc (read contract).
- No `SECURITY.md` change here; M003 owns the claim wording and should be
  coordinated so it does not describe v2 as already written.

## 13. Acceptance criteria

- A v2 payload decrypts only with the id it was sealed for.
- Version 1 decrypts unchanged through both entry points.
- `decrypt` without an id fails loudly on v2.
- No proto, server, storage, KDF-parameter, or merge-order change.
- The v2 writer exists only under test/test-support.
- The sync pull path supplies the id and preserves it.

## 14. Stop conditions

Stop and report rather than improvise when:

- ADR-0002 is not accepted;
- the AAD construction in this plan and the ADR disagree (the ADR wins; stop
  and reconcile);
- a v2 payload can be produced by any production path (that is M006, and
  shipping it here would break old devices);
- the id supplied by the pull path is not the id the writer used (investigate
  identity stability before proceeding);
- making `decrypt` reject v2 breaks an existing caller that cannot supply an id
  (enumerate callers first; do not add a silent default).

## 15. Closure evidence required

- The encryption diff and the AAD byte layout as implemented.
- Unit transcript: correct id, wrong id, v1 unaffected, unknown version.
- Mixed-pull integration transcript with counters.
- Confirmation that no production v2 writer exists (`grep` on the cfg gates).
- Statement that `snip-proto`, `snip-sync`, merge order, and KDF parameters are
  unchanged.

## 16. Handoff notes

- `src/encryption.rs` is heavily unit-tested; follow the existing test naming
  and keep the tamper tests.
- Do not change `ARGON2_*` constants. ADR-0002 and the module doc both state
  that a parameter change requires a new version.
- `decrypt` is used by tests and by `decrypt_snippet`; enumerate every caller
  before changing its behavior.
- The `test-support` helper must live behind
  `#[cfg(any(test, feature = "test-support"))]`; `scripts/ci/test-production-seams.sh`
  will not catch this one (it checks runtime seams, not symbols), so review the
  cfg gate explicitly.
- Integration targets need `--features test-support`; follow `scripts/check.sh`
  flags rather than inventing them.

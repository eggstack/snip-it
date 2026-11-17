# ADR-0002: Bind encrypted sync payloads to snippet identity

Status: accepted

Status history:

- 2026-11-17 — proposed
- 2026-11-17 — accepted (maintainer directed implementation of the
  security-assurance workstream)

Date: 2026-11-17

Decision owners: project maintainers

Related specification sections:

- `plans/000-long-term-specification.md#6-sync-invariants` — encrypted fields,
  merge identity, and the local-only field split
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules` —
  evidence required for a payload-format change
- `plans/001-terminology-and-domain-model.md#synced-fields` — the synced /
  local-only field boundary

Affected subsystem roadmaps:

- `plans/subsystems/security-assurance-roadmap.md` (proposed; owns the
  implementation)
- `plans/subsystems/sync-cursor-protocol-roadmap.md` (closed; context only —
  its versioning convention for additive sync changes is reused)

## Context

`src/encryption.rs` seals payloads with AES-256-GCM and no associated data:
`cipher.encrypt(&nonce, plaintext.as_bytes())` (`src/encryption.rs:328-330`)
and `cipher.decrypt(&nonce, payload.ciphertext.as_ref())`
(`src/encryption.rs:355-357`). GCM authenticates the ciphertext, but nothing
cryptographically ties that ciphertext to the row it belongs to.

The payload that `encrypt_snippet` produces covers only three fields —
`description`, `command`, `tags` (`src/sync.rs:333-338`) — while the routing
fields `id`, `created_at`, `updated_at`, `device_id`, and `deleted` travel as
plaintext proto fields (`src/sync.rs:1516-1535`; `architecture/encryption.md`
"Scope boundary"). The ciphertext is carried in the proto `command` field and
the record is marked `encrypted = true`.

Consequences of the missing binding:

1. **Cross-row substitution.** A server, a database operator, or anyone able
   to reorder stored rows can copy a valid ciphertext from one of a user's
   snippets into another snippet row with a different `id`. The client
   decrypts successfully and accepts the content as the target row's
   content. The substitution cannot invent new plaintext, but it can
   rearrange the user's existing content across descriptions and ids.
2. **No fail-loud signal.** Because the tag still verifies, the client
   cannot distinguish "this is the content that was sealed for this id" from
   "this is content sealed for a different id".
3. **Boundary with the documented limitation.** The same server also
   observes the bearer API key on every request (`src/sync.rs:323-343`), and
   any party holding that key can decrypt and re-encrypt arbitrary content.
   This ADR does **not** close that — see `SECURITY.md` § Sync Encryption and
   `architecture/encryption.md`'s trust-boundary note. It closes the
   narrower, key-blind case (a store that can move ciphertext but cannot use
   the key), and it makes the two failure modes distinguishable in the
   payload format.

The defect is recorded as finding L8 in the 2026-11-17 security audit.

## Decision drivers

- A ciphertext must only decrypt for the snippet identity it was sealed for.
- Existing v1 payloads must keep decrypting indefinitely; history cannot be
  rewritten.
- `snip-proto` must not change: the ciphertext already travels opaquely in
  `command`, and no new field is needed.
- Old clients must not be forced to understand the new format before the
  fleet is ready, because a failed decrypt currently surfaces as a skipped
  snippet and a retried sync.
- Argon2id cost must not grow per snippet: the batch-shared salt and the
  derived-key cache stay valid, because AAD does not affect key derivation.
- The change must not touch the client merge order, `updated_at` semantics,
  or snippet-id determinism (`docs/IDENTITY_CONTRACT.md`).

## Considered options

### Option A — Payload version 2 with AAD over the snippet id (recommended)

Keep `magic‖version‖salt‖nonce‖ciphertext`. Version `2` means the ciphertext
was sealed with associated data derived from the snippet identity:

```text
AAD = b"snip-payload-v2\0" ‖ u32_le(id.len()) ‖ id.as_bytes()
```

The length prefix removes concatenation ambiguity. `id` is the same
content-addressable id the client already stores, sends, and merges on; the
server treats `id` as an ordinary primary-key column and does not need to
interpret it.

- **Benefits.** Substitution across ids fails authentication and surfaces as
  a normal `DecryptionFailed`. No protocol change, no server change, no new
  KDF work. The version byte already exists in the header, so the format is
  self-describing.
- **Costs.** A mixed-version fleet: a client that only understands v1 cannot
  decrypt a v2 payload written by another device. `from_base64` currently
  rejects any version other than 1 with `Unsupported payload version`
  (`src/encryption.rs:190-194`), and `decrypt_snippet` counts the failure as
  `server_decrypt_failed_count` (`src/sync.rs:919-926`).
- **Failure modes.** If writes switch to v2 before every device can read it,
  older devices skip those snippets on every sync until upgraded.

### Option B — AAD over id and device_id

Same as Option A plus `device_id`.

- **Benefits.** Also binds the writing device.
- **Costs.** `device_id` is client-supplied and can legitimately change on a
  re-registered device; a re-registered device would then fail to decrypt its
  own earlier payloads after a restore. The id already identifies the row, so
  the extra binding buys little against substitution.
- **Failure modes.** Restore/re-registration turns a routine operation into
  mass decryption failure.

### Option C — AAD over id and library id

- **Benefits.** Prevents cross-library replay as well as cross-id replay.
- **Costs.** The library identifier is not currently threaded into
  `encrypt_snippets` (`src/sync.rs:1580-1593`); doing so touches the batch
  API and every caller. It also couples the payload to a server-assigned
  library id whose lifetime is managed by the server.
- **Failure modes.** Library rename/relink semantics (missing-library
  recovery) would have to guarantee the library id is stable across relink —
  not something this ADR can assert today.

### Option D — No format change; document the residual risk

- **Benefits.** Zero compatibility cost.
- **Costs.** Leaves the key-blind substitution path open, and the threat model
  cannot truthfully state that the tag authenticates *which* snippet the
  content belongs to.
- **Failure modes.** The gap is indistinguishable from "content integrity is
  complete" to future maintainers.

## Decision

**Option A is selected, introduced in two phases.**

- **Phase 1 — read support (this workstream's M005).** Accept
  `PAYLOAD_VERSION_V2`, implement the AAD construction and the v1/v2
  decryption dispatch, and keep **writing v1**. No behavior change for
  existing fleets; v2 payloads decrypt correctly and substitutions fail.
- **Phase 2 — write support (M006).** Switch `encrypt_with_salt` to write v2
  with AAD after the named release-adoption precondition is satisfied: one
  published `snp` release that reads v2 must be available, and the minimum
  supported client version in `SECURITY.md` must include it.

Phase 2 is deliberately not bundled with Phase 1. The audited risk is low and
local; a fleet-wide decryption regression is a worse outcome than a delayed
switch. The roadmap records Phase 2 as blocked on Phase 1 plus the release
precondition rather than leaving it as an unstated follow-up.

Library-id binding (Option C) is deferred: it is a strictly larger change
whose identity-stability precondition (missing-library recovery and relink)
must be established first.

## Consequences

### Positive

- A ciphertext is cryptographically bound to its snippet id; cross-row
  substitution fails with `DecryptionFailed` instead of silently succeeding.
- No proto, server, storage, merge-order, or KDF-parameter change.
- v1 remains readable forever; existing libraries need no migration.
- The threat model can state precisely what the tag guarantees: content
  integrity for the bound identity, not confidentiality against a party that
  observes the bearer key.

### Negative

- Two payload versions must be maintained, tested, and documented.
- The write switch needs a coordinated minimum-client-version statement; until
  M006 lands, the implemented protection is read-side only.
- Old clients cannot read v2 payloads, which is why Phase 2 is gated.

### Neutral or deferred

- AAD coverage of `library_id` and `device_id` is deferred (Options B/C),
  with reasons recorded above.
- Per-snippet random salts remain unnecessary for security given unique
  nonces; the batch-shared salt perf optimization stands.

## Compatibility and migration

- `EncryptedPayload::from_base64` accepts version `1` and `2`; any other
  version remains a hard `InvalidData` error.
- `argon2_params_for_version` returns the same parameters for versions 0, 1,
  and 2, so the derived-key cache keyed by `(api_key, salt, version)`
  (`src/encryption.rs:249-260`) hits for all of them; no re-derivation cost is
  introduced by the version byte.
- Decryption of a v2 payload requires the snippet id. The public
  `decrypt(api_key, data)` cannot supply it, so it must keep rejecting v2 with
  an explicit error; a new `decrypt_for_snippet(api_key, data, id)` is used by
  `decrypt_snippet`. This is a fail-loud API split, not a silent default.
- No database or wire migration. A v2 payload stored by a future client in an
  existing database is simply opaque bytes to the server.

## Security and reliability implications

- **Integrity.** Substitution of ciphertext between ids of the same key fails
  the GCM tag check because the AAD differs.
- **Confidentiality.** Unchanged: the key still derives from the API key and
  the per-payload salt; a bearer-token observer can still decrypt.
- **Denial of service.** A malicious server can still withhold or replay
  payloads; AAD does not address availability. Replaying an identical payload
  for the same id is indistinguishable from a legitimate re-send and remains
  covered by the merge rules, not by the tag.
- **Migration.** No destructive step; v1 payloads are never rewritten in
  place. A user who downgrades after M006 will skip v2 payloads, which is the
  documented cost of the write switch.
- **No new secret.** AAD is public data; it adds no key material and no
  storage.

## Verification

- Unit: v2 roundtrip with the correct id; v2 payload decrypted with a
  different id fails with `DecryptionFailed`.
- Unit: v1 payload still decrypts with `decrypt` and with
  `decrypt_for_snippet`; `decrypt` on a v2 payload returns the explicit
  "requires identity binding" error rather than a generic tag failure.
- Unit: unknown version (`3`) is still rejected as `InvalidData`.
- Integration: `sync` pulls a library containing one v1 and one v2 row, and
  both land with their original ids (`tests/sync_integration.rs`).
- Guard: a test asserting a decrypted v2 snippet keeps the id it was sealed
  under after save/load id normalization (`docs/IDENTITY_CONTRACT.md`).
- Phase 2 additionally: mixed-version negative test proving an old-format-only
  decrypt path cannot read a v2 write, so the release precondition is
  documented rather than assumed.

## Supersession

None. If library-id binding is later adopted, it must arrive as a new payload
version under a new ADR; version 2 is not redefined in place.

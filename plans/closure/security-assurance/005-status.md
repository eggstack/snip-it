# Security Assurance M005 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/security-assurance/005-encryption-payload-v2-read-support.md`

Source subsystem roadmap:

- `plans/subsystems/security-assurance-roadmap.md#milestone-005--encryption-payload-v2-read-support`

Repository baseline reviewed: `0f3bc98` (`chore/prune-comments-and-plans`);
implementation is uncommitted.

Implementation commits or pull requests:

- Uncommitted working tree — `src/encryption.rs`, `src/sync.rs`,
  `src/sync.rs` test re-export, `tests/sync_integration.rs`,
  `architecture/encryption.md`, `.skills/encryption-module.md`.

Primary class: infrastructure. **This milestone delivers no user-visible
capability** and must not be described as closing the substitution finding.

## 1. Executive finding

M005 is complete. The client reads payload version 2, verifying an AES-GCM tag
computed over associated data derived from the snippet id, while still writing
version 1. Version 0 and 1 payloads decrypt unchanged, an unknown version is
still a hard error, and the only version-2 writer is a `test-support`-gated
helper that is provably absent from the production binary.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| v2 decrypts with the correct id | `v2_roundtrip_with_the_correct_id` | pass | |
| v2 fails with a different id | `v2_fails_with_a_different_id` | pass | Fails as `DecryptionFailed`, the same class as a wrong key |
| v1 decrypts unchanged through both entry points | `v1_payloads_still_decrypt_through_both_entry_points` | pass | The id is ignored for v1 |
| Legacy version 0 decrypts unchanged | `legacy_unversioned_payloads_still_decrypt_through_both_entry_points` | pass | |
| Unknown version rejected | `test_unknown_payload_version_is_rejected` | pass | Version 3 rejected at parse and at decrypt |
| `decrypt` without an id fails loudly on v2 | `v2_without_identity_is_rejected_loudly` | pass | Explicit "requires snippet identity" error, not a tag failure |
| Empty id cannot seal a v2 payload | `v2_seal_rejects_an_empty_id` | pass | |
| Tampered v2 ciphertext still fails | `v2_tampered_ciphertext_still_fails_the_tag` | pass | |
| AAD construction is unambiguous | `aad_construction_is_unambiguous` | pass | Length prefix kills concatenation collisions |
| Mixed v1/v2 pull lands both rows with their ids | `mixed_v1_and_v2_payloads_sync_and_a_substituted_v2_row_is_skipped` | pass | Integration, real server |
| A substituted v2 row is skipped and counted | same test | pass | Appears in `skipped_ids`, absent from `snippets`, `success` stays true |
| v1-only library syncs exactly as before | `v1_only_library_still_syncs_unchanged` | pass | |
| No per-snippet KDF cost added | `argon2_params_for_version` returns the same parameters for 0/1/2; cache key unchanged in shape | pass | AAD does not affect derivation |
| v2 writer absent from production | `nm -a target/production-seam/release/snp \| grep -c seal_v2_for_test` → `0` | pass | The AAD domain string appears once because the production read path builds it |
| No proto, server, storage, or merge change | diff scope | pass | `snip-proto/`, `snip-sync/`, `src/sync_commands.rs` untouched |
| Argon2 parameters unchanged | constants untouched | pass | |

## 3. Production implementation evidence

- `PAYLOAD_VERSION_V2 = 2`; `from_base64` accepts 1 and 2 and rejects anything
  else; `argon2_params_for_version` maps 0/1/2 to the current parameters.
- `payload_aad(id) = b"snip-payload-v2\0" ‖ u32_le(id.len()) ‖ id`.
- `seal_with_salt` and `open_payload` are the single sealing/opening paths, with
  the AAD as a parameter; `encrypt_with_salt` passes an empty AAD and version 1,
  so version-1 output is byte-identical.
- `decrypt_for_snippet(api_key, data, id)` is the production read entry point;
  `decrypt_snippet` passes the proto row's `id`.
- `seal_v2_for_test` is `#[cfg(any(test, feature = "test-support"))]` and reuses
  the production cipher and AAD helper; `src/sync.rs` re-exports it under the
  same gate for integration tests.

## 4. Verification executed

```bash
cargo test --lib encryption                                   # 29 passed (filtered)
XDG_CONFIG_HOME=$(mktemp -d) cargo test --workspace --lib      # 1248 passed, 8 ignored; snip-sync 197 passed
cargo test --test sync_integration --features test-support     # 11 passed
cargo test --test sync_multibatch --features test-support -- --test-threads=1  # 6 passed
nm -a target/production-seam/release/snp | grep -c seal_v2_for_test            # 0
```

Results: all pass. `cargo test --workspace --lib` required an isolated
`XDG_CONFIG_HOME` in this environment because unrelated auto-sync tests read the
developer's real config directory; that is a pre-existing condition, recorded in
§ 10.

## 5. Invariant review

- Version 1 payloads decrypt exactly as before; no stored payload is rewritten.
- Argon2id parameters, salt size, nonce size, and payload layout are unchanged.
- The batch-shared salt and derived-key cache still apply; no per-snippet KDF
  cost returned.
- Merge order, `updated_at`, and snippet-id determinism are untouched.
- Unknown versions remain hard errors; there is no "try both" fallback.
- `snip-proto` and `snip-sync` are unchanged.

## 6. Failure and recovery review

- A v2 payload with an empty id cannot be decrypted; it fails as
  `DecryptionFailed` and follows the existing skip-and-count path rather than
  aborting the sync.
- Decryption is a pure function over the payload; cancellation, restart, and
  concurrency semantics are unchanged.
- A failed v2 decrypt increments `server_decrypt_failed_count` and adds the id
  to `skipped_ids`, proved by the substitution test.

## 7. Migration and compatibility review

- Old payloads, new client: v1 decrypts unchanged.
- New payloads, old client: not produced by this milestone. A build that writes
  v2 would be unreadable by a v1-only reader, which is why the write switch is
  M006 and gated on a published v2-reading release.
- No storage migration; the server treats version 2 as opaque bytes.
- No version bump: no public API change is intended, and none occurred (the
  module is crate-internal).

## 8. Security review

- A wrong id and a wrong key are indistinguishable in kind (both
  `DecryptionFailed`), so the error is not an oracle.
- The AAD is public data and is never logged.
- `decrypt` is compiled only for tests. This is a deliberate deviation from the
  plan's letter (see § 10): no production caller can hold a payload without its
  identity, so the identity-less entry point is a compile-time error in
  production rather than a runtime rejection. The runtime rejection is still
  proven by unit tests, and it removes dead code from the shipped binary.
- The test-only writer cannot reach a production build, proven by symbol count.

## 9. Documentation and operations

- `architecture/encryption.md` — payload version table, AAD bytes, phase split,
  write path, and the key-cache interaction.
- `src/encryption.rs` module docs — the three-version read contract and why
  `decrypt` rejects v2.
- `.skills/encryption-module.md` — corrected the stale "no parameter versioning"
  limitation and recorded that v2 writes are not active.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | Deviation: `decrypt` is test-only rather than a shipped entry point that rejects v2 at runtime | None functionally; the compile-time boundary is stricter than the plan's runtime one | Recorded here; ADR-0002's intent (fail loudly rather than silently decrypt without AAD) is preserved |
| low | `cargo test --workspace --lib` hangs in this environment without an isolated config dir | Verification only; unrelated to this milestone | Pre-existing: several auto-sync tests read the developer's real config dir. Run with `XDG_CONFIG_HOME=$(mktemp -d)` until those tests adopt the thread-local override |
| low | The id binding protects only new payloads, and no new payload exists yet | The audited substitution path is still open for every stored row | M006, blocked on a published v2-reading release |

## 11. Roadmap disposition

Milestone closed; M006 remains blocked on its operational precondition (one
published `snp` release that reads v2 plus the stated minimum client version),
not on any code gap here.

## 12. Registry updates

`plans/registry.md` and the roadmap § 12 table record M005 as `closed`; M006
stays `blocked`.

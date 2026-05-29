# Encryption Module Review

**Module:** `src/encryption.rs` (325 lines)
**Architecture Doc:** `architecture/encryption.md` (110 lines)
**Date:** 2026-05-29

---

## 1. Document Accuracy

### Verified Correct

| Claim | Status | Evidence |
|-------|--------|----------|
| AES-256-GCM encryption | Correct | `encryption.rs:23-26` imports `Aes256Gcm`; `encryption.rs:159` creates cipher |
| Argon2id key derivation | Correct | `encryption.rs:121-122` uses `Algorithm::Argon2id` |
| Random 12-byte nonce | Correct | `encryption.rs:74` `NONCE_SIZE: usize = 12`; `encryption.rs:163` `OsRng.fill_bytes` |
| Random 16-byte salt | Correct | `encryption.rs:75` `SALT_SIZE: usize = 16`; `encryption.rs:154-155` `OsRng.fill_bytes` |
| Argon2 time cost = 3 | Correct | `encryption.rs:33` `ARGON2_TIME_COST: u32 = 3` |
| Argon2 parallelism = 4 | Correct | `encryption.rs:34` `ARGON2_PARALLELISM: u32 = 4` |
| Argon2 memory cost = 64 KiB | Correct | `encryption.rs:32` `1 << 6` = 64 KiB |
| 32-byte key output | Correct | `encryption.rs:128` `Some(32)` |
| Payload format: Salt + Nonce + Ciphertext | Correct | `encryption.rs:88-93` `to_base64` |
| `EncryptedPayload` struct | Correct | `encryption.rs:81-85` |
| `CryptoError` enum variants | Correct | `encryption.rs:60-70` |
| `DerivedKey` implements `Zeroize` + `Drop` | Correct | `encryption.rs:48-58` |
| Roundtrip test | Correct | `encryption.rs:205-214` |
| Different encryptions differ | Correct | `encryption.rs:216-225` |
| Wrong key fails | Correct | `encryption.rs:227-237` |
| Empty/unicode/large payload tests | Correct | `encryption.rs:239-263` |
| Invalid base64 / truncated payload tests | Correct | `encryption.rs:265-280` |
| Tampered ciphertext/nonce/salt tests | Correct | `encryption.rs:282-324` |

### Discrepancies

None found. The architecture document accurately describes the implementation.

---

## 2. Bugs & Issues

### CRITICAL: Argon2 Memory Cost Too Low

**Location:** `encryption.rs:32`
```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 6;  // 64 KiB
```

OWASP recommends a **minimum of 19 MiB (19,456 KiB)** for Argon2id. The current value of 64 KiB is **300x below** the recommended minimum. This defeats the memory-hard property of Argon2 — an attacker with a GPU can test billions of key candidates per second because memory is negligible.

**Impact:** Brute-force attacks on the API key become practical. An attacker with commodity hardware could derive keys at rates of ~10^9 attempts/second.

**Recommendation:** Change to `1 << 14` (16 MiB) at minimum, or `1 << 18` (256 KiB) if startup latency is a concern — but see design note below about key derivation purpose.

### HIGH: `hash_password` Misused for Key Derivation

**Location:** `encryption.rs:117-151`

`PasswordHasher::hash_password` is designed for **password storage**, not key derivation. The code uses it to extract raw key material from the PHC string output:

```rust
let hash = argon2.hash_password(api_key.as_bytes(), &salt_string)?;  // line 134
let hash_output = hash.hash.ok_or_else(|| ...)?;                     // line 137-139
let hash_bytes = hash_output.as_bytes();                             // line 141
```

This works functionally (`hash_password` calls `hash_raw` internally with identical output), but:

1. The PHC string output embeds algorithm parameters that are never verified — a silent parameter change would produce different keys with no error.
2. The API contract is misleading: `hash_password` implies password verification, not deterministic key derivation.
3. If the argon2 crate changes `hash_password` behavior, the derived key could silently change.

**Recommendation:** Use `Argon2::new().hash_raw(password, salt, params)` which returns raw hash bytes directly and is the intended API for key derivation.

### MEDIUM: `drop(key)` is Redundant and Incomplete

**Location:** `encryption.rs:176` (encrypt), `encryption.rs:195` (decrypt)

```rust
drop(key);  // Only zeros DerivedKey wrapper, not the copy inside Aes256Gcm
```

`DerivedKey::Drop` zeros the 32-byte array, but the AES-GCM cipher has already copied the key material into its internal key schedule (`Aes256Gcm` stores `Aes256` which stores the expanded key). The `aes-gcm` crate does not implement `Zeroize` for its cipher type, so key material persists in memory after the cipher is dropped.

The explicit `drop(key)` gives a false sense of security. The key material survives in two locations: the (now zeroed) `DerivedKey` and the (not zeroed) cipher's key schedule. When `cipher` drops at end of function, its key schedule is also not zeroed.

**Impact:** Key material may persist in heap memory until overwritten by the allocator. On systems with swap or core dumps, this could be exposed.

**Mitigation:** This is a `aes-gcm` crate limitation. Consider documenting this limitation. For stronger guarantees, consider `aes-gcm-siv` with `Zeroize` support or use `ring::aead` which handles key zeroization internally.

### MEDIUM: No Parameter Versioning / No Key Rotation Support

**Location:** `encryption.rs:32-34`

The Argon2 parameters are compile-time constants with no version header in the encrypted payload. If parameters ever change (e.g., increasing memory cost), all previously encrypted data becomes undecryptable because the same salt + different parameters produces a different derived key.

The `EncryptedPayload` contains `salt` but not the Argon2 parameters used. A future parameter change would silently break decryption of all stored data.

**Recommendation:** Either:
1. Include Argon2 parameters in the payload header (adds ~4 bytes), or
2. Treat these parameters as a breaking protocol change requiring re-encryption of all data, or
3. At minimum, document this constraint prominently.

### LOW: No API Key Validation

**Location:** `encryption.rs:153`

`encrypt` accepts any `&str` as `api_key` including empty strings. While this is technically a valid input to Argon2, an empty API key provides no security. There's no minimum length check.

**Recommendation:** Add a minimum length validation (e.g., 16 characters) at the API boundary, or document that the caller is responsible for key quality.

---

## 3. Design Issues

### Tight Coupling to Argon2 PHC Format

The `derive_key` function uses `SaltString::encode_b64` from the `argon2` crate's `password_hash` module. This couples the key derivation to the PHC string format. If we switch to `hash_raw`, we'd use raw `&[u8]` salt directly, removing this dependency.

### `EncryptedPayload` Uses `Vec<u8>` Instead of Fixed Arrays

`encryption.rs:81-85`: The struct uses `Vec<u8>` for `salt`, `nonce`, and `ciphertext`. Since salt is always 16 bytes and nonce is always 12 bytes, these could be `[u8; 16]` and `[u8; 12]` for stack allocation and compile-time size guarantees. The ciphertext varies so `Vec<u8>` is appropriate there.

### `CryptoError` Not Integrated with `SnipError`

`encryption.rs:60-70`: `CryptoError` is a standalone error type. Callers in `sync.rs:331-332` and `sync.rs:355-356` convert it to `SnipError::runtime_error` with `map_err`. A `From<CryptoError> for SnipError` impl would reduce boilerplate and provide better error context.

### No Streaming API

`encrypt`/`decrypt` require the entire plaintext/ciphertext in memory. For large snippets this is fine, but the architecture doesn't account for future use cases where streaming might be needed (e.g., encrypting large file contents during sync).

---

## 4. Security Concerns

| # | Severity | Issue | Location |
|---|----------|-------|----------|
| S1 | **CRITICAL** | Argon2 memory cost 64 KiB (OWASP min: 19 MiB) | `encryption.rs:32` |
| S2 | **HIGH** | `hash_password` API misuse for key derivation | `encryption.rs:117-151` |
| S3 | **MEDIUM** | Key material not zeroized in AES-GCM key schedule | `encryption.rs:159,176,186,195` |
| S4 | **MEDIUM** | No parameter versioning — parameter changes break all stored data | `encryption.rs:32-34` |
| S5 | **LOW** | No minimum API key length validation | `encryption.rs:153` |
| S6 | **INFO** | OsRng used correctly for CSPRNG | `encryption.rs:155,163` — verified secure |

### Security Properties Verified

- **Confidentiality**: AES-256-GCM — correct
- **Integrity**: GCM auth tag — correct, tested at `encryption.rs:282-324`
- **Key isolation**: API key never sent to server — correct (verified in `sync.rs:318-345`)
- **Nonce uniqueness**: Random per-encryption via OsRng — correct
- **Salt uniqueness**: Random per-encryption via OsRng — correct
- **Forward secrecy claim**: Architecture doc says "identical plaintexts produce different ciphertexts" — this is correct due to random salt/nonce, but is technically **semantic security** not **forward secrecy** (which requires ephemeral key exchange). The doc uses the term loosely.

---

## 5. Performance Issues

### Argon2 is Slow in Debug Builds

All 11 tests pass but run in the unoptimized test profile. The `ARGON2_TIME_COST: u32 = 3` with 64 KiB memory means each encryption takes ~3 Argon2 iterations. In debug mode this is noticeable but acceptable. In release mode with LTO (enabled in Cargo.toml:50) it's fast.

### No Performance Issues in Production Use

The encrypt/decrypt functions are called per-snippet during sync. With release optimization, the Argon2 cost is ~10-50ms per operation. For typical sync operations (hundreds of snippets), this adds seconds of latency. This is acceptable for security but could be optimized if needed (e.g., key caching, PBKDF2 for hot path).

---

## 6. Test Coverage

### Covered

- [x] Roundtrip encrypt/decrypt
- [x] Different encryptions produce different output (randomness)
- [x] Wrong key fails
- [x] Empty string
- [x] Unicode
- [x] Large payload (10KB)
- [x] Invalid base64
- [x] Truncated payload
- [x] Tampered ciphertext
- [x] Tampered nonce
- [x] Tampered salt

### Missing

- [ ] Zeroize correctness test (verify key material is actually cleared)
- [ ] Parameter versioning test (verify behavior with different Argon2 params)
- [ ] Concurrent encrypt/decrypt test (thread safety)
- [ ] Known-answer test (KAT) with fixed salt/nonce for regression detection
- [ ] Benchmarks for Argon2 cost measurement
- [ ] Test with API key of various lengths (empty, 1 char, max)
- [ ] Integration test verifying encrypt/decrypt in sync flow end-to-end

---

## 7. Priority Ranking

| Priority | Issue | Action |
|----------|-------|--------|
| **P0** | S1: Argon2 memory cost 64 KiB | Increase to ≥16 MiB. Breaking change for existing encrypted data — requires re-encryption or parameter versioning. |
| **P1** | S2: `hash_password` API misuse | Refactor to use `hash_raw` or `hash_password_into`. Functionally equivalent but semantically correct. |
| **P1** | S4: No parameter versioning | Add version byte to payload header or document constraint. Must ship with P0 fix. |
| **P2** | S3: Key material in cipher schedule | Document limitation. Consider `ring::aead` for better zeroization. |
| **P3** | S5: No API key validation | Add minimum length check at API boundary. |
| **P3** | Design: `CryptoError` not integrated | Add `From<CryptoError> for SnipError` impl. |
| **P4** | Design: `Vec<u8>` vs fixed arrays | Use `[u8; 16]` for salt, `[u8; 12]` for nonce in struct. |

---

## 8. Recommendations

### Immediate (Before Next Release)

1. **Increase Argon2 memory cost** to `1 << 14` (16 MiB) minimum. This is a breaking change — all existing encrypted snippets will fail to decrypt. Implement alongside parameter versioning (P1) or accept a one-time re-encryption migration.

2. **Add parameter versioning** to `EncryptedPayload` — prepend a 1-byte version header encoding the Argon2 parameter set. This allows future parameter changes without breaking existing data.

3. **Refactor `derive_key`** to use `Argon2::new().hash_raw()` instead of `hash_password`. This is a non-breaking internal change.

### Short-Term

4. **Add `From<CryptoError> for SnipError`** to reduce boilerplate in `sync.rs`.

5. **Use fixed-size arrays** for salt (`[u8; 16]`) and nonce (`[u8; 12]`) in `EncryptedPayload` to prevent accidental size mismatches.

6. **Document key material zeroization limitation** — the `aes-gcm` crate does not zeroize its internal key schedule.

### Long-Term

7. **Consider `ring::aead`** for better memory safety guarantees (ring zeros keys on drop).

8. **Add KAT (Known-Answer Test) vectors** with fixed salt/nonce for regression detection when parameters change.

9. **Benchmark Argon2 cost** across platforms to ensure acceptable sync latency.

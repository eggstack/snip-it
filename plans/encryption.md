# Encryption Module Review & Improvement Plan

## Architecture Document Verification

| Claim | Documented | Actual | Status |
|-------|-----------|--------|--------|
| File location | `src/encryption.rs` | `src/encryption.rs` | ✓ Match |
| Line count | 325 lines | 325 lines | ✓ Match |
| Encryption algorithm | AES-256-GCM | AES-256-GCM | ✓ Match |
| Key derivation | Argon2id | Argon2id | ✓ Match |
| Nonce size | 12 bytes | 12 bytes | ✓ Match |
| Salt size | 16 bytes | 16 bytes | ✓ Match |
| Argon2 time cost | 3 | 3 | ✓ Match |
| Argon2 parallelism | 4 | 4 | ✓ Match |
| Argon2 memory cost | `1 << 6` (64 KiB) | `1 << 14` (16 MiB) | ✗ MISMATCH |

## Discrepancy Details

### 1. Argon2 Memory Cost Mismatch

**Architecture doc (line 23):**
```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 6;  // 64 KiB
```

**Actual code (line 32):**
```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum for Argon2id
```

The documentation is outdated. The code uses 16 MiB which is the OWASP recommended minimum for Argon2id, while the doc still shows the old 64 KiB value. This discrepancy is actually a security improvement in the code that was never reflected in the documentation.

---

## Bugs & Edge Cases

### 1. `drop(key)` in `encrypt()` is Ineffective (Line 176)

```rust
let payload = EncryptedPayload {
    salt: salt.to_vec(),
    nonce: nonce_bytes.to_vec(),
    ciphertext,
};

drop(key);  // <-- This drops the DerivedKey, but key was moved into cipher
```

**Issue:** `key` was already moved into `cipher` at line 159-160. The `drop(key)` call here does nothing — it attempts to drop a value that was already moved. The `cipher` struct will clean up the key material when it goes out of scope, but this `drop` call creates confusion and has no effect.

**Fix:** Remove the ineffective `drop(key)` call. If explicit cleanup is needed, the cipher should be dropped explicitly before constructing the payload.

### 2. `drop(key)` in `decrypt()` has Same Issue (Line 195)

Same problem as above — `key` was moved into `cipher` at line 186-187. The `drop(key)` call is a no-op.

---

## Potential Improvements

### 1. Use `std::mem::take` for Explicit Key Cleanup

Instead of the ineffective `drop(key)`, if we need guaranteed cleanup before payload construction:

```rust
let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())?;
// cipher now contains the key
// Drop cipher explicitly to ensure key zeroization happens now
drop(cipher);
```

### 2. Consider ZeroizeDerive for DerivedKey

The `Zeroize` implementation manually zeroes the array. Could use `zeroize::ZeroizeFrom` or `ZeroizeDefault` for cleaner implementation.

### 3. Add Constant-Time Comparison for Salt/Nonce Extraction

The `from_base64` function uses slice operations that could theoretically leak timing information about payload structure. Minor concern for this use case, but worth noting.

### 4. Missing Test: Wrong API Key Produces Different Error Than Corrupted Data

Currently all decryption failures produce similar errors regardless of cause (wrong key vs. corrupted ciphertext). This is actually good for security (no oracle), but could be documented.

---

## Security Analysis

### Strengths

1. **OWASP-compliant Argon2id params** — The 16 MiB memory cost exceeds OWASP minimum (9 MiB for Argon2id at security level 2)
2. **Authenticated encryption** — AES-256-GCM provides confidentiality + integrity
3. **Per-encryption randomness** — Salt and nonce are generated per encryption using `OsRng`
4. **Memory safety** — `DerivedKey` implements `Zeroize` + `Drop` for key cleanup
5. **No oracle** — Decryption failures don't reveal which byte was wrong

### Observations

1. The `drop(key)` calls are ineffective but harmless (the key will still be cleaned up when `cipher` is dropped)
2. AES-256-GCM with Argon2id is a solid choice for this use case
3. Key isolation is maintained — server never sees plaintext or derived key

---

## Error Handling Assessment

| Error Case | Handled? | Notes |
|------------|----------|-------|
| Base64 decode failure | ✓ | Returns `InvalidData` |
| Payload too short | ✓ | Returns `InvalidData` |
| Argon2 param invalid | ✓ | Returns `KeyDerivationFailed` |
| Argon2 hashing fails | ✓ | Returns `KeyDerivationFailed` |
| AES key init fails | ✓ | Returns `EncryptionFailed`/`DecryptionFailed` |
| AES encryption fails | ✓ | Returns `EncryptionFailed` |
| AES decryption fails (tamper) | ✓ | Returns `DecryptionFailed` (GCM auth tag catches this) |
| UTF-8 conversion fails | ✓ | Returns `DecryptionFailed` |

---

## Recommended Actions

1. **Update architecture doc** — Change memory cost from `1 << 6` to `1 << 14` and note OWASP compliance
2. **Remove ineffective `drop(key)` calls** — Lines 176 and 195 serve no purpose
3. **Add comment explaining why `cipher` is dropped before payload construction** — If explicit ordering is desired for cleanup timing

---

## Test Coverage Assessment

All documented test cases are implemented:
- ✓ Roundtrip encrypt/decrypt (`test_encrypt_decrypt_roundtrip`)
- ✓ Different output per encryption (`test_different_encryptions_produce_different_output`)
- ✓ Wrong key fails (`test_wrong_key_fails`)
- ✓ Empty string (`test_encrypt_empty_string`)
- ✓ Unicode (`test_encrypt_unicode`)
- ✓ Large payload (`test_encrypt_large_payload`)
- ✓ Invalid base64 (`test_invalid_base64_decrypt`)
- ✓ Truncated payload (`test_truncated_payload_decrypt`)
- ✓ Tampered ciphertext (`test_tampered_ciphertext_detected`)
- ✓ Tampered nonce (`test_tampered_nonce_detected`)
- ✓ Tampered salt (`test_tampered_salt_detected`)
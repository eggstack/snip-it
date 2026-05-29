# Encryption Module Skill

## Purpose
Guide agents through working with the encryption module (`src/encryption.rs`).

## Argon2 Memory Cost

**Location**: `src/encryption.rs:32`
```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14;  // 16 MiB
```

Memory cost is set to `1 << 14` (16 MiB). OWASP recommends a minimum of 19 MiB (19,456 KiB).

**WARNING**: Changing Argon2 parameters is a **breaking change**. All existing encrypted snippets will fail to decrypt because the same salt + different parameters produces a different derived key. If changing, add parameter versioning to `EncryptedPayload` (1-byte version header) and support decrypting with old parameters for backward compatibility.

## Key Derivation

**Current (misused API)**:
```rust
// Uses hash_password (designed for password storage, not key derivation)
let hash = argon2.hash_password(api_key.as_bytes(), &salt_string)?;
let hash_output = hash.hash.ok_or_else(...)?;
let hash_bytes = hash_output.as_bytes();
```

**Recommended (correct API)**:
```rust
// Use hash_raw for direct key derivation
let mut hash_bytes = [0u8; 32];
argon2.hash_password_into(api_key.as_bytes(), salt, &mut hash_bytes)?;
```

## Payload Format

```
Base64(Salt[16] + Nonce[12] + Ciphertext[...])
```

- Salt: 16 random bytes (OsRng)
- Nonce: 12 random bytes (OsRng)
- Ciphertext: AES-256-GCM encrypted JSON of `{description, command, tags}`

## API

```rust
pub fn encrypt(api_key: &str, plaintext: &str) -> SnipResult<String>
pub fn decrypt(api_key: &str, encrypted: &str) -> SnipResult<String>

// Internal (used by sync module)
pub fn encrypt_snippet(api_key: &str, snippet: &Snippet) -> SnipResult<ProtoSnippet>
pub fn decrypt_snippet(api_key: &str, proto: &ProtoSnippet) -> SnipResult<Snippet>
```

## Error Handling

`CryptoError` enum:
- `InvalidBase64` — corrupted payload
- `InvalidPayload` — wrong length or format
- `EncryptionFailed` — AES-GCM error
- `DecryptionFailed` — authentication tag mismatch (wrong key or tampered data)
- `KeyDerivationFailed` — Argon2 error

**Note**: `CryptoError` is not integrated with `SnipError`. Callers use `map_err(|e| SnipError::runtime_error(...))`.

## Security Properties

- **Confidentiality**: AES-256-GCM
- **Integrity**: GCM auth tag (tested with tampered ciphertext/nonce/salt tests)
- **Key isolation**: API key never sent to server (used locally for encryption only)
- **Nonce uniqueness**: Random per-encryption via OsRng
- **Salt uniqueness**: Random per-encryption via OsRng

## Known Limitations

1. Key material persists in AES-GCM cipher key schedule (not zeroized)
2. No parameter versioning — changing Argon2 constants breaks all stored data
3. `DerivedKey` implements `Zeroize` but the copy inside `Aes256Gcm` is not zeroized
4. No streaming API — entire plaintext/ciphertext must fit in memory

## Tests

11 tests in `encryption.rs:197-324`:
- Roundtrip encrypt/decrypt
- Different encryptions produce different output
- Wrong key fails
- Empty string, unicode, large payload (10KB)
- Invalid base64, truncated payload
- Tampered ciphertext, nonce, salt

Missing: Zeroize correctness, parameter versioning, concurrent encrypt/decrypt, benchmarks.

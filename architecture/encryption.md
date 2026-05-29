# Encryption

[← Back to Overview](overview.md)

## File

**`src/encryption.rs`** (325 lines)

End-to-end encryption for snippet data in transit. The server never sees plaintext snippet content.

## Security Model

| Component | Algorithm | Details |
|-----------|-----------|---------|
| Encryption | AES-256-GCM | Authenticated encryption, 256-bit key |
| Key Derivation | Argon2id | From API key + random salt |
| Nonce | Random 12 bytes | Per-encryption, stored with ciphertext |
| Salt | Random 16 bytes | Per-encryption, stored with ciphertext |

### Argon2id Parameters

```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 6;  // 64 KiB
const ARGON2_TIME_COST: u32 = 3;               // 3 iterations
const ARGON2_PARALLELISM: u32 = 4;             // 4 threads
```

## Encrypted Payload Format

```
┌──────────┬──────────┬────────────┐
│ Salt(16) │ Nonce(12)│ Ciphertext │
└──────────┴──────────┴────────────┘
         Base64 encoded
```

### `EncryptedPayload` struct

```rust
pub struct EncryptedPayload {
    pub salt: Vec<u8>,      // 16 bytes, random
    pub nonce: Vec<u8>,     // 12 bytes, random
    pub ciphertext: Vec<u8>, // AES-GCM encrypted data + auth tag
}
```

- `to_base64()` — Concatenate salt + nonce + ciphertext, base64 encode
- `from_base64()` — Decode and split components

## Key Derivation

```rust
fn derive_key(api_key: &str, salt: &[u8]) -> CryptoResult<DerivedKey>
```

1. Encode salt as base64 `SaltString`
2. Create Argon2id instance with fixed params
3. Hash API key with salt → 32-byte output
4. Return as `DerivedKey` (zeroized on drop)

## Encrypt / Decrypt

### `encrypt(api_key, plaintext) -> String`

1. Generate random 16-byte salt
2. Derive key from API key + salt
3. Generate random 12-byte nonce
4. Encrypt with AES-256-GCM
5. Build payload (salt + nonce + ciphertext)
6. Return base64 string

### `decrypt(api_key, encrypted_data) -> String`

1. Parse base64 → extract salt, nonce, ciphertext
2. Derive key from API key + salt
3. Decrypt with AES-256-GCM (verifies auth tag)
4. Return plaintext string

## Security Properties

- **Confidentiality** — AES-256-GCM encryption
- **Integrity** — GCM auth tag detects tampering (ciphertext, nonce, salt)
- **Key isolation** — API key never sent to server; derived key is ephemeral
- **Forward secrecy** — Random salt/nonce per encryption means identical plaintexts produce different ciphertexts
- **Memory safety** — `DerivedKey` implements `Zeroize` + `Drop` to clear key material

## Error Types

```rust
pub enum CryptoError {
    EncryptionFailed(String),
    DecryptionFailed(String),
    KeyDerivationFailed(String),
    InvalidData(String),
}
```

## Test Coverage

- Roundtrip encrypt/decrypt
- Different encryptions produce different output
- Wrong key fails
- Empty string, unicode, large payloads
- Invalid base64, truncated payload
- Tampered ciphertext/nonce/salt all detected

## Key Files

- `src/encryption.rs` — Core encrypt/decrypt, key derivation, payload format
- `src/sync.rs` — Uses encrypt/decrypt for snippet sync

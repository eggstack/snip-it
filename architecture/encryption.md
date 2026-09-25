# Encryption

[← Back to Overview](overview.md)

## Overview

End-to-end encryption for snippet sync (`src/encryption.rs`, ~418 lines,
sync-client layer). Snippet bodies are encrypted with the user's API key
before transmission; the server stores and relays only ciphertext and
never sees plaintext. Key derivation uses Argon2id; bulk encryption uses
AES-256-GCM.

## Security Model

| Component | Algorithm | Details |
|-----------|-----------|---------|
| Encryption | AES-256-GCM | Authenticated encryption, 256-bit key, 16-byte tag |
| Key Derivation | Argon2id v0x13 | From API key + random 16-byte salt |
| Nonce | Random 12 bytes | Per-encryption via `Nonce::generate()` (OsRng) |
| Salt | Random 16 bytes | Per-encryption via `OsRng` |

### Argon2id Parameters — `src/encryption.rs:37-39`

```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum
const ARGON2_TIME_COST: u32 = 3;             // 3 iterations — OWASP minimum
const ARGON2_PARALLELISM: u32 = 4;           // 4 lanes
```

Output is always 32 bytes (`Params::new(..., Some(32))`,
`hash_password_into` with raw salt bytes, `:190-205`).

## Encrypted Payload Format

```
┌──────────┬──────────┬─────────────────────────┐
│ Salt(16) │ Nonce(12)│ Ciphertext (+ 16B tag)  │
└──────────┴──────────┴─────────────────────────┘
          Base64 encoded (STANDARD)
```

```rust
pub struct EncryptedPayload {          // encryption.rs:142-146
    pub salt: Vec<u8>,                 // 16 bytes
    pub nonce: Vec<u8>,                // 12 bytes
    pub ciphertext: Vec<u8>,           // AES-GCM output + auth tag
}
impl EncryptedPayload {
    pub fn to_base64(&self) -> String;              // encryption.rs:149
    pub fn from_base64(data: &str) -> CryptoResult<Self>; // encryption.rs:157
}
```

`from_base64` rejects payloads shorter than 28 bytes
(`InvalidData("Data too short")`).

## Key Derivation — `src/encryption.rs:178-227`

```rust
fn derive_key(api_key: &str, salt: &[u8]) -> CryptoResult<DerivedKey>
```

1. Build cache key `(SHA-256(api_key) hex, base64(salt))`; return clone
   on hit (`hash_api_key`, `:49-58`).
2. Run Argon2id with the fixed params above into `[u8; 32]`.
3. Insert into the session cache (10K cap, evict-half on overflow) and
   return a `DerivedKey`, zeroizing the stack buffer.

`DerivedKey` (`:102-113`) is `Zeroize + ZeroizeOnDrop + Default`; callers
explicitly `zeroize()` / `drop(mem::take(&mut key))` after use
(`:243, :269`). Cache keys hash the API key with full SHA-256 so one
user's entry can never collide with another's.

## Encrypt / Decrypt — `src/encryption.rs:229-273`

```rust
pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String>
pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String>
pub type CryptoResult<T> = Result<T, CryptoError>;
```

**Encrypt**: random salt → `derive_key` → `Aes256Gcm::new_from_slice` →
random nonce → `encrypt` → zeroize key → `salt‖nonce‖ciphertext` base64.
Identical plaintexts produce different ciphertexts (fresh salt+nonce).

**Decrypt**: base64 split → `derive_key(salt)` → nonce length check →
`decrypt` (GCM tag verified) → zeroize key → UTF-8 conversion. Any
tampering of salt (wrong key), nonce, or ciphertext fails; truncated or
non-base64 input fails as `InvalidData`.

## Key Cache

```rust
static KEY_CACHE: LazyLock<Mutex<HashMap<(String, String), DerivedKey>>> // :63-64
pub fn clear_key_cache()                       // :95 — call at end of sync
pub(crate) fn key_cache_guard() -> KeyCacheGuard // :90 — Drop clears cache (RAII)
```

Session-local only: avoids re-running Argon2id for the same
`(api_key, salt)` pair within one sync. Poison recovery keeps surviving
entries (still valid derived keys) instead of disabling the cache.
Evicted/cleared values are `DerivedKey`, so eviction wipes key material.

## Error Types — `src/encryption.rs:115-126`

```rust
pub enum CryptoError {
    EncryptionFailed(String),
    DecryptionFailed(String),
    KeyDerivationFailed(String),
    InvalidData(String),
}
```

Blanket `From<CryptoError> for SnipError` (`src/error.rs:316-331`):
`EncryptionFailed` → `SyncFailureKind::EncryptionFailed`, everything
else → `DecryptionFailed` (both classify as `FailureClass::Internal`,
so retry policy is preserved). `Display` never includes key material.

## Failure Flow vs `last_sync`

Decryption/encryption failures do **not** advance the sync cursor.
`sync_commands.rs:851-896` checks `response.skipped_count > 0` and skips
`update_last_sync()` so failed snippets are retried on the next sync;
Push-mode records a `conflicts += 1` + "will retry" note instead of
`pushed += 1`. No local data is rolled back — sync failure never undoes
a committed local mutation.

## Invariants / gotchas (from AGENTS.md)

- **Argon2 parameter changes break all existing encrypted payloads** —
  version the format first; never silently retune memory/time/parallelism.
- API key never leaves the client: server sees ciphertext only; errors
  and logs must never contain key material (`SyncSettings::drop`
  zeroizes; `Debug` prints `[REDACTED]`).
- No AAD binds ciphertext to context (accepted for current use);
  integrity comes from the GCM tag over ciphertext+nonce.
- Nonce birthday bound ~2⁴⁸ encryptions with random 12-byte nonces —
  fine for per-snippet sync payloads.
- Layer rule: `encryption.rs` is sync-client; core (`library`, `sort`,
  `selector`) must not import it (`tests/architecture.rs`).

## Test Coverage — `src/encryption.rs:275-418`

Round-trip, distinct-ciphertext per encryption, wrong-key failure, empty /
Unicode / 10 KB payloads, invalid base64, truncated payload, tampered
ciphertext/nonce/salt detection, constant-time helper, cache-key
uniqueness (same key → same hash, different keys → different hashes).

## Key Files

- `src/encryption.rs` — params, payload, `derive_key`, `encrypt`/`decrypt`,
  `KEY_CACHE`, `CryptoError`, unit tests.
- `src/sync.rs` — encrypts uploads / decrypts downloads via these helpers.
- `src/sync_commands.rs:846-896` — `skipped_count` gate on `last_sync`.
- `src/error.rs:316-331` — `CryptoError → SyncFailureKind` mapping.

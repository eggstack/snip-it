# Encryption

[← Back to Overview](overview.md)

## Overview

Encrypted sync for snippet bodies (`src/encryption.rs`, 418 lines,
sync-client layer). Snippet **bodies** are encrypted with the user's API key
before transmission; the server stores and relays only ciphertext for those
fields. Key derivation uses Argon2id; bulk encryption uses AES-256-GCM.

> **Trust boundary — read this before calling it "end-to-end".** The API key
> *does* leave the client: it is sent as `Bearer <api_key>` in the gRPC
> `authorization` metadata on every request (`src/sync.rs:306-314`), and the
> server reads it from there (`snip-sync/src/lib.rs:1079`). The server stores
> only an Argon2 hash of the key in `users.api_key`, so it cannot recover the
> key from the database — but anything that can observe the bearer token (the
> server process itself, a TLS-terminating proxy, a request log) can re-derive
> the same Argon2id key and decrypt any payload it holds or relays. The
> protection this design actually provides is **transport confidentiality and
> at-rest obfuscation of snippet bodies**, not server-blindness.

**Scope boundary — what is *not* encrypted.** `encrypt_snippet`
(`src/sync.rs:1389-1419`) encrypts only
`EncryptedSnippetData { description, command, tags }`
(`src/sync.rs:316-321`), serialized as JSON and placed in the proto `command`
field. The routing and bookkeeping fields `id`, `created_at`, `updated_at`,
`device_id`, and `deleted` are sent as **plaintext** proto fields — the server
needs them for merge ordering and deletion tombstones. `output` is not in
`ProtoSnippet` at all (local-only). So "the server never sees plaintext" means
snippet *content* is not stored or relayed in the clear, not snippet
*metadata*; do not assume `device_id` or `updated_at` are confidential.

The merge, conflict resolution, and library-orchestration logic live in
[sync.md](sync.md); the config and keychain plumbing live in
[config.md](config.md).

## Security Model

| Component | Algorithm | Details |
|-----------|-----------|---------|
| Encryption | AES-256-GCM | Authenticated encryption, 256-bit key, 16-byte tag |
| Key Derivation | Argon2id v0x13 | From API key + random 16-byte salt |
| Nonce | Random 12 bytes | Per-encryption via `Nonce::generate()` (OsRng) |
| Salt | Random 16 bytes | Per-encryption via `OsRng` |
| Payload encoding | Base64 (STANDARD) | `salt‖nonce‖ciphertext` concatenated |

### Argon2id Parameters — `src/encryption.rs:37-39`

```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum for Argon2id
const ARGON2_TIME_COST: u32 = 3; // 3 iterations — OWASP minimum recommendation
const ARGON2_PARALLELISM: u32 = 4; // 4 threads — matches typical desktop CPU core count
```

Output is always 32 bytes: `Params::new(ARGON2_MEMORY_COST_KIB,
ARGON2_TIME_COST, ARGON2_PARALLELISM, Some(32))` with
`hash_password_into` over the raw salt bytes
(`src/encryption.rs:190-205`).

## Encrypted Payload Format

```
┌──────────┬──────────┬─────────────────────────┐
│ Salt(16) │ Nonce(12)│ Ciphertext (+ 16B tag)  │
└──────────┴──────────┴─────────────────────────┘
          Base64 encoded (STANDARD)
```

```rust
// src/encryption.rs:142-146
pub struct EncryptedPayload {
    pub salt: Vec<u8>,       // 16 bytes
    pub nonce: Vec<u8>,      // 12 bytes
    pub ciphertext: Vec<u8>, // AES-GCM output + auth tag
}
impl EncryptedPayload {
    pub fn to_base64(&self) -> String;                        // src/encryption.rs:149
    pub fn from_base64(data: &str) -> CryptoResult<Self>;     // src/encryption.rs:157
}
```

`from_base64` rejects payloads shorter than 28 bytes
(`SALT_SIZE + NONCE_SIZE`, `src/encryption.rs:162-164`) with
`InvalidData("Data too short")`, and non-base64 input with
`InvalidData("Failed to decode base64: …")`.

## Key Derivation — `src/encryption.rs:178-227`

```rust
fn derive_key(api_key: &str, salt: &[u8]) -> CryptoResult<DerivedKey>
```

1. Build the cache key `(SHA-256(api_key) hex, base64(salt))`; return a clone
   on a hit (`hash_api_key`, `src/encryption.rs:49-58`).
2. Run Argon2id with the fixed parameters above into `[u8; 32]`.
3. Insert into the session cache (10 000-entry cap, evict-half on overflow)
   and return a `DerivedKey`, zeroizing the stack buffer
   (`src/encryption.rs:207-226`).

`DerivedKey` (`src/encryption.rs:102-113`) is `Zeroize + ZeroizeOnDrop +
Default`; callers explicitly `zeroize()` / `drop(mem::take(&mut key))` after
use (`src/encryption.rs:243`, `:269`). Cache keys hash the API key with the
full SHA-256 digest so one user's entry can never collide with another's.

### Versioning Constraint

The payload format carries **no version byte and no KDF identifier**. The
salt is stored in the clear but the parameters are not. Therefore **changing
any Argon2 parameter silently breaks every existing encrypted payload**: the
server keeps serving ciphertext that the client can no longer derive a key
for. Version the format (add a parameter/KDF id to the payload) *before*
retuning memory, time, or parallelism — never retune in place.

The same applies to the AES-256-GCM construction and the 12-byte nonce size
(`src/encryption.rs:130-131`): changing either invalidates stored data.

## Encrypt / Decrypt — `src/encryption.rs:229-273`

```rust
pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String>   // :229
pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String> // :254
pub type CryptoResult<T> = Result<T, CryptoError>;                       // :128
```

**Encrypt** (`:229-252`): random salt → `derive_key` →
`Aes256Gcm::new_from_slice` → random nonce → `encrypt` → `key.zeroize()` →
`salt‖nonce‖ciphertext` base64. Identical plaintexts produce different
ciphertexts (fresh salt and nonce each time).

**Decrypt** (`:254-273`): base64 split → `derive_key(salt)` → nonce length
check (`InvalidData("Invalid nonce length")`) → `decrypt` (GCM tag verified) →
`drop(std::mem::take(&mut key))` → UTF-8 conversion. Any tampering of salt
(wrong key), nonce, or ciphertext fails; truncated or non-base64 input fails
as `InvalidData`.

## What Sync Actually Encrypts

| Path | Behaviour |
|------|-----------|
| Upload | `encrypt_snippets` (`src/sync.rs:1428-1434`) encrypts each local snippet via `encrypt_snippet`; records that fail encryption are collected into `failed_ids` (`src/sync.rs:1439-1460`) and surface as `skipped_count`. |
| Upload shape | `encrypt_snippet` (`src/sync.rs:1389-1419`) returns a `ProtoSnippet` with `description = ""`, `tags = []`, `command = <base64 ciphertext>`, and `encrypted = true`. |
| Download | `decrypt_snippet` (`src/sync.rs:1463-1491`) returns the snippet unchanged when `encrypted == false` (plaintext records are passed through as-is), otherwise decrypts `command`, JSON-parses it back into `EncryptedSnippetData`, and restores `description`, `command`, and `tags`. |
| Download failures | A decryption failure increments the page's decrypt-failure counter and the snippet's id is appended to the skipped-id list (`src/sync.rs:849-857`). |

The server enforces a **1 MiB cap on encrypted payloads**
(`DEFAULT_MAX_ENCRYPTED_PAYLOAD_LENGTH`, `snip-sync/src/lib.rs:102`, enforced at
`:880-885`) so a single record can always be transported. Records above the
cap are refused, and the refusal is reported to the client in `skipped_ids`.

## Key Cache

```rust
static KEY_CACHE: LazyLock<Mutex<HashMap<(String, String), DerivedKey>>> // :63-64
pub fn clear_key_cache()                                                // :95
pub(crate) fn key_cache_guard() -> KeyCacheGuard                         // :90
```

Session-local only: it avoids re-running Argon2id for the same
`(api_key, salt)` pair within one sync. `KeyCacheGuard` is an RAII guard whose
`Drop` clears the whole cache (`src/encryption.rs:82-88`); it is taken at the
top of `sync_encrypted_inner` (`src/sync.rs:504`) and of
`run_sync_with_limits` (`src/sync_commands.rs:675`), so the cache never
outlives the sync operation.

Poison recovery keeps the surviving entries (still valid derived keys) instead
of disabling the cache (`lock_key_cache`, `src/encryption.rs:72-80`). The cap
is `MAX_KEY_CACHE_SIZE = 10_000` (`src/encryption.rs:43`); on overflow half
the entries are evicted. Evicted and cleared values are `DerivedKey`, so
eviction wipes the key material.

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
`EncryptionFailed` → `SyncFailureKind::EncryptionFailed`; `DecryptionFailed`,
`KeyDerivationFailed`, and `InvalidData` all → `SyncFailureKind::DecryptionFailed`.
Both target kinds classify as `FailureClass::Internal`, so the retry policy is
preserved (see [sync.md](sync.md#failure-classification)). `Display` never
includes key material.

## Failure Flow vs `last_sync`

Decryption/encryption failures do **not** advance the sync cursor.
`src/sync_commands.rs:908` computes `has_failures = response.skipped_count > 0`,
and `last_sync` is only written when `!has_failures`
(`src/sync_commands.rs:917-920`, `:939-943`, `:1025-1029`). A skipped record
counts as `failed`, never as a conflict, and the per-library result is `false`
(`src/sync_commands.rs:921-931`, `:957-974`, `:1045-1058`), so the run exits
non-zero and the records are retried on the next sync. No local data is rolled
back — a sync failure never undoes a committed local mutation.

## Test Coverage — `src/encryption.rs:275-418`

Round-trip, distinct-ciphertext per encryption, wrong-key failure, empty /
Unicode / 10 KB payloads, invalid base64, truncated payload, tampered
ciphertext/nonce/salt detection, constant-time helper (`ct_eq`,
`src/encryption.rs:133-136`), and cache-key uniqueness (same key → same hash,
different keys → different hashes).

## Key Files

- `src/encryption.rs` — params, payload, `derive_key`, `encrypt`/`decrypt`,
  `KEY_CACHE`, `CryptoError`, unit tests.
- `src/sync.rs:1389-1491` — `encrypt_snippet` / `decrypt_snippet`, the
  client-side crypto seam.
- `src/sync_commands.rs:908-974` — the `skipped_count` gate on `last_sync`.
- `src/config/sync_settings.rs:236-334` — keychain-backed API-key storage.
- `src/error.rs:316-331` — `CryptoError → SyncFailureKind` mapping.

## Invariants and Gotchas

- **Argon2 parameter changes break all existing encrypted payloads** — version
  the format first; never silently retune memory/time/parallelism.
- **The API key does leave the client.** It travels as a `Bearer <api_key>`
  gRPC metadata header on every request (`src/sync.rs:306-314`), so the server
  can re-derive the payload key; the confidentiality this module provides is
  transport + at-rest, not server-blindness (see the trust-boundary note in the
  Overview). Errors and logs must never contain key material.
  `SyncSettings::drop` zeroizes the key and `Debug` prints `[REDACTED]`
  (`src/config/sync_settings.rs:134-169`).
- **No AAD binds the ciphertext to context** (accepted for current use);
  integrity comes from the GCM tag over ciphertext+nonce.
- **Nonce birthday bound** ~2⁴⁸ encryptions with random 12-byte nonces —
  fine for per-snippet sync payloads.
- **Metadata is not confidential.** `id`, `created_at`, `updated_at`,
  `device_id`, and `deleted` travel in plaintext; the server needs them.
- **Layer rule**: `encryption.rs` is sync-client; core (`library`, `sort`,
  `selector`) must not import it (`tests/architecture.rs`).
- **`decrypt_snippet` passes plaintext records through unchanged.** A record
  with `encrypted == false` is returned as-is, so mixing plaintext and
  encrypted records in one library is tolerated but means some content is
  never encrypted.
- **An encryption or decryption failure is a *skip*, not a conflict.** It
  increments `failed`, blocks the `last_sync` advance, and is retried on the
  next sync — see [sync.md](sync.md#the-skipped_count-rule).

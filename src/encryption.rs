//! **Layer: Sync-Client**
//!
//! Encryption utilities for secure sync.
//!
//! Provides end-to-end encryption for snippet data using AES-256-GCM with
//! Argon2 key derivation. All snippets are encrypted before transmission
//! and decrypted upon receipt.
//!
//! # Security Model
//!
//! - **Encryption**: AES-256-GCM (authenticated encryption)
//! - **Key Derivation**: Argon2id from API key + random salt
//! - **Nonce**: Random 12-byte nonce per encryption (stored with ciphertext)
//!
//! # Example
//!
//! ```text
//! // Internal API — not available to external consumers.
//! // use snip_it::encryption::{decrypt, encrypt};
//! ```

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, Generate, KeyInit},
};
use argon2::Argon2;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[cfg(test)]
use subtle::ConstantTimeEq;

const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum for Argon2id
const ARGON2_TIME_COST: u32 = 3; // 3 iterations — OWASP minimum recommendation
const ARGON2_PARALLELISM: u32 = 4; // 4 threads — matches typical desktop CPU core count

/// Maximum number of derived keys to cache. Each entry is ~100 bytes (32-byte key
/// + string keys + HashMap overhead), so 10K entries ≈ 1 MB.
const MAX_KEY_CACHE_SIZE: usize = 10_000;

/// Cryptographic hash for API key cache keys.
///
/// Uses the full SHA-256 digest to avoid cache-key collisions that could
/// cause one user's derived key to be served from another user's cache entry.
fn hash_api_key(api_key: &str) -> String {
    let hash = Sha256::digest(api_key.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(hash.len() * 2);
    for byte in hash {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

/// Session-local cache for derived keys to avoid re-running Argon2id
/// for the same (api_key, salt) pair during a sync operation.
/// Key: (hashed_api_key, base64(salt)), Value: derived key bytes
static KEY_CACHE: LazyLock<Mutex<HashMap<(String, String, u8), DerivedKey>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Lock the session key cache, recovering entries after a poison.
///
/// A poisoned mutex means a previous holder panicked mid-update. The
/// surviving entries are still valid derived keys, so recover them instead
/// of silently disabling the cache (which would re-run Argon2id on every
/// subsequent call).
fn lock_key_cache() -> std::sync::MutexGuard<'static, HashMap<(String, String, u8), DerivedKey>> {
    match KEY_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("KEY_CACHE mutex poisoned; recovering cached derived keys");
            poisoned.into_inner()
        }
    }
}

pub(crate) struct KeyCacheGuard;

impl Drop for KeyCacheGuard {
    fn drop(&mut self) {
        clear_key_cache();
    }
}

pub(crate) fn key_cache_guard() -> KeyCacheGuard {
    KeyCacheGuard
}

/// Clear the session key cache. Should be called at the end of a sync operation.
pub fn clear_key_cache() {
    let mut cache = KEY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    // Every cached value is a `DerivedKey` (ZeroizeOnDrop), so clearing
    // the map wipes the key material even when this is never called.
    cache.clear();
}

#[derive(Clone, Zeroize, ZeroizeOnDrop, Default)]
struct DerivedKey([u8; 32]);

impl DerivedKey {
    fn new(key: &[u8; 32]) -> Self {
        Self(*key)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("Key derivation failed: {0}")]
    KeyDerivationFailed(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

pub type CryptoResult<T> = Result<T, CryptoError>;

const NONCE_SIZE: usize = 12;
/// Size of the KDF salt, in bytes.
pub const SALT_SIZE: usize = 16;

#[cfg(test)]
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Magic prefix that identifies the versioned payload layout.
///
/// The prefix is 8 bytes long so that a legacy payload — whose first bytes
/// are random salt — cannot be mistaken for a versioned one.
const PAYLOAD_MAGIC: [u8; 8] = *b"SNIP-PAY";

/// Version of the payload layout written by this build.
const PAYLOAD_VERSION: u8 = 1;

/// Version of the pre-magic (unversioned) payload layout.
const PAYLOAD_VERSION_LEGACY: u8 = 0;

/// Encrypted data container with version, salt, nonce, and ciphertext.
///
/// The salt and nonce are stored alongside the ciphertext to enable
/// decryption without separate key exchange.
///
/// The layout version identifies the key-derivation parameters the payload
/// was produced with. Changing the Argon2id parameters in place would make
/// every previously stored payload undecryptable, and the failure would be
/// indistinguishable from "wrong API key" — so a parameter change has to
/// introduce a new version instead.
pub struct EncryptedPayload {
    /// Payload layout version. `0` for the legacy unversioned layout.
    pub version: u8,
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl EncryptedPayload {
    pub fn to_base64(&self) -> String {
        let mut combined = Vec::with_capacity(
            PAYLOAD_MAGIC.len() + 1 + SALT_SIZE + NONCE_SIZE + self.ciphertext.len(),
        );
        combined.extend_from_slice(&PAYLOAD_MAGIC);
        combined.push(self.version);
        combined.extend_from_slice(&self.salt);
        combined.extend_from_slice(&self.nonce);
        combined.extend_from_slice(&self.ciphertext);
        BASE64.encode(&combined)
    }

    pub fn from_base64(data: &str) -> CryptoResult<Self> {
        let combined = BASE64
            .decode(data)
            .map_err(|e| CryptoError::InvalidData(format!("Failed to decode base64: {e}")))?;

        let header = PAYLOAD_MAGIC.len() + 1;
        if combined.len() >= header && combined[..PAYLOAD_MAGIC.len()] == PAYLOAD_MAGIC {
            let version = combined[PAYLOAD_MAGIC.len()];
            if version != PAYLOAD_VERSION {
                return Err(CryptoError::InvalidData(format!(
                    "Unsupported payload version {version}"
                )));
            }
            if combined.len() < header + SALT_SIZE + NONCE_SIZE {
                return Err(CryptoError::InvalidData("Data too short".to_string()));
            }

            let salt = combined[header..header + SALT_SIZE].to_vec();
            let nonce = combined[header + SALT_SIZE..header + SALT_SIZE + NONCE_SIZE].to_vec();
            let ciphertext = combined[header + SALT_SIZE + NONCE_SIZE..].to_vec();

            return Ok(Self {
                version,
                salt,
                nonce,
                ciphertext,
            });
        }

        // Legacy layout: no magic, no version byte. The parameters are the
        // ones this build still uses, so these payloads stay readable.
        if combined.len() < SALT_SIZE + NONCE_SIZE {
            return Err(CryptoError::InvalidData("Data too short".to_string()));
        }

        let salt = combined[..SALT_SIZE].to_vec();
        let nonce = combined[SALT_SIZE..SALT_SIZE + NONCE_SIZE].to_vec();
        let ciphertext = combined[SALT_SIZE + NONCE_SIZE..].to_vec();

        Ok(Self {
            version: PAYLOAD_VERSION_LEGACY,
            salt,
            nonce,
            ciphertext,
        })
    }
}

/// Argon2id parameters for a payload version.
///
/// Version 0 (the legacy unversioned layout) and version 1 use the same
/// parameters. A parameter change must introduce a new version: changing
/// these in place would make every stored payload undecryptable.
fn argon2_params_for_version(version: u8) -> Option<(u32, u32, u32)> {
    match version {
        PAYLOAD_VERSION_LEGACY | PAYLOAD_VERSION => {
            Some((ARGON2_MEMORY_COST_KIB, ARGON2_TIME_COST, ARGON2_PARALLELISM))
        }
        _ => None,
    }
}

fn derive_key(api_key: &str, salt: &[u8], version: u8) -> CryptoResult<DerivedKey> {
    let (memory_cost, time_cost, parallelism) =
        argon2_params_for_version(version).ok_or_else(|| {
            CryptoError::KeyDerivationFailed(format!("Unsupported payload version {version}"))
        })?;
    let salt_b64 = BASE64.encode(salt);
    // The version is part of the cache key: two versions can use the same
    // salt with different KDF parameters.
    let cache_key = (hash_api_key(api_key), salt_b64, version);

    // Check cache first
    {
        let cache = lock_key_cache();
        if let Some(cached) = cache.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(memory_cost, time_cost, parallelism, Some(32))
            .map_err(|e| CryptoError::KeyDerivationFailed(format!("Invalid Argon2 params: {e}")))?,
    );

    let mut key_bytes = [0u8; 32];
    argon2
        .hash_password_into(api_key.as_bytes(), salt, &mut key_bytes)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Hashing failed: {e}")))?;

    // Cache the derived key for future use with the same (api_key, salt)
    {
        let mut cache = lock_key_cache();
        // Evict half the entries when cache is full. HashMap iteration order is
        // arbitrary, but this is acceptable for a session-local cache — re-deriving
        // a key costs less than the initial Argon2id computation. Removed values
        // are `DerivedKey` (ZeroizeOnDrop), so eviction wipes their bytes.
        if cache.len() >= MAX_KEY_CACHE_SIZE {
            let keys_to_remove: Vec<_> =
                cache.keys().take(MAX_KEY_CACHE_SIZE / 2).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }
        cache.insert(cache_key, DerivedKey::new(&key_bytes));
    }

    let derived = DerivedKey::new(&key_bytes);
    key_bytes.zeroize();
    Ok(derived)
}

/// Generates a fresh random salt for a batch of payloads.
pub fn generate_salt() -> [u8; SALT_SIZE] {
    <[u8; SALT_SIZE]>::generate()
}

/// Encrypts a plaintext with a fresh random salt.
pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String> {
    encrypt_with_salt(api_key, plaintext, &generate_salt())
}

/// Encrypts a plaintext with a caller-provided salt.
///
/// Callers that encrypt a batch share one salt so the derived-key cache hits
/// for every payload after the first — the Argon2id derivation is the
/// expensive part, and deriving it per snippet made a batch cost one full
/// derivation per snippet. Every payload still carries its own random nonce,
/// so the ciphertexts remain independent under the shared key.
///
/// The payload layout is unchanged, so payloads written by other versions (or
/// by a single-snippet call) still decrypt.
pub fn encrypt_with_salt(
    api_key: &str,
    plaintext: &str,
    salt: &[u8; SALT_SIZE],
) -> CryptoResult<String> {
    let mut key = derive_key(api_key, salt, PAYLOAD_VERSION)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Key init failed: {e}")))?;

    let nonce = Nonce::generate();

    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Encryption failed: {e}")))?;

    key.zeroize();

    let payload = EncryptedPayload {
        version: PAYLOAD_VERSION,
        salt: salt.to_vec(),
        nonce: nonce.to_vec(),
        ciphertext,
    };

    Ok(payload.to_base64())
}

pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String> {
    let payload = EncryptedPayload::from_base64(encrypted_data)?;

    let mut key = derive_key(api_key, &payload.salt, payload.version)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Key init failed: {e}")))?;

    let nonce = Nonce::try_from(payload.nonce.as_slice())
        .map_err(|_| CryptoError::InvalidData("Invalid nonce length".to_string()))?;

    let plaintext = cipher
        .decrypt(&nonce, payload.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Decryption failed: {e}")))?;

    drop(std::mem::take(&mut key));

    String::from_utf8(plaintext)
        .map_err(|e| CryptoError::DecryptionFailed(format!("UTF-8 conversion failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let api_key = "test-api-key-12345";
        let plaintext = "echo 'hello world'";

        let encrypted = encrypt(api_key, plaintext).unwrap();
        let decrypted = decrypt(api_key, &encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_different_encryptions_produce_different_output() {
        let api_key = "test-api-key-12345";
        let plaintext = "echo 'hello world'";

        let encrypted1 = encrypt(api_key, plaintext).unwrap();
        let encrypted2 = encrypt(api_key, plaintext).unwrap();

        assert_ne!(encrypted1, encrypted2);
    }

    #[test]
    fn test_wrong_key_fails() {
        let api_key = "test-api-key-12345";
        let wrong_key = "wrong-key-67890";
        let plaintext = "echo 'hello world'";

        let encrypted = encrypt(api_key, plaintext).unwrap();
        let result = decrypt(wrong_key, &encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_empty_string() {
        let api_key = "test-key";
        let encrypted = encrypt(api_key, "").unwrap();
        let decrypted = decrypt(api_key, &encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_encrypt_unicode() {
        let api_key = "test-key";
        let plaintext = "echo 'héllo wörld'";
        let encrypted = encrypt(api_key, plaintext).unwrap();
        let decrypted = decrypt(api_key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_large_payload() {
        let api_key = "test-key";
        let plaintext = "x".repeat(10000);
        let encrypted = encrypt(api_key, &plaintext).unwrap();
        let decrypted = decrypt(api_key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_invalid_base64_decrypt() {
        let api_key = "test-key";
        let result = decrypt(api_key, "not-valid-base64!!!@#");
        assert!(result.is_err());
    }

    #[test]
    fn test_truncated_payload_decrypt() {
        let api_key = "test-key";
        let encrypted = encrypt(api_key, "test").unwrap();
        // Truncate the encrypted data
        let truncated = &encrypted[..10];
        let result = decrypt(api_key, truncated);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_ciphertext_detected() {
        let api_key = "test-key";
        let plaintext = "sensitive data";
        let encrypted = encrypt(api_key, plaintext).unwrap();

        let mut payload = EncryptedPayload::from_base64(&encrypted).unwrap();
        // Flip a byte in the ciphertext (AES-GCM should detect this)
        if payload.ciphertext.len() > 10 {
            payload.ciphertext[10] ^= 0xFF;
        }
        let tampered = payload.to_base64();
        let result = decrypt(api_key, &tampered);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_nonce_detected() {
        let api_key = "test-key";
        let plaintext = "sensitive data";
        let encrypted = encrypt(api_key, plaintext).unwrap();

        let mut payload = EncryptedPayload::from_base64(&encrypted).unwrap();
        // Flip a byte in the nonce
        payload.nonce[0] ^= 0xFF;
        let tampered = payload.to_base64();
        let result = decrypt(api_key, &tampered);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_salt_detected() {
        let api_key = "test-key";
        let plaintext = "sensitive data";
        let encrypted = encrypt(api_key, plaintext).unwrap();

        let mut payload = EncryptedPayload::from_base64(&encrypted).unwrap();
        // Flip a byte in the salt (different key derivation => decryption fails)
        payload.salt[0] ^= 0xFF;
        let tampered = payload.to_base64();
        let result = decrypt(api_key, &tampered);
        assert!(result.is_err());
    }

    #[test]
    fn test_payload_carries_a_version_marker() {
        let api_key = "test-key";
        let encrypted = encrypt(api_key, "sensitive data").unwrap();
        let payload = EncryptedPayload::from_base64(&encrypted).unwrap();
        assert_eq!(payload.version, PAYLOAD_VERSION);
        // The marker is part of the serialized payload, so a reader can tell
        // which KDF parameters produced it.
        let raw = BASE64.decode(&encrypted).unwrap();
        assert_eq!(&raw[..PAYLOAD_MAGIC.len()], &PAYLOAD_MAGIC);
        assert_eq!(raw[PAYLOAD_MAGIC.len()], PAYLOAD_VERSION);
    }

    #[test]
    fn test_legacy_payload_without_marker_still_decrypts() {
        let api_key = "test-key";
        let plaintext = "sensitive data";
        let encrypted = encrypt(api_key, plaintext).unwrap();
        let payload = EncryptedPayload::from_base64(&encrypted).unwrap();

        // Rebuild the legacy layout: salt | nonce | ciphertext, no marker.
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&payload.salt);
        legacy.extend_from_slice(&payload.nonce);
        legacy.extend_from_slice(&payload.ciphertext);
        let legacy_b64 = BASE64.encode(&legacy);

        let parsed = EncryptedPayload::from_base64(&legacy_b64).unwrap();
        assert_eq!(parsed.version, PAYLOAD_VERSION_LEGACY);
        assert_eq!(decrypt(api_key, &legacy_b64).unwrap(), plaintext);
    }

    #[test]
    fn test_unknown_payload_version_is_rejected() {
        let api_key = "test-key";
        let encrypted = encrypt(api_key, "sensitive data").unwrap();
        let mut payload = EncryptedPayload::from_base64(&encrypted).unwrap();
        payload.version = PAYLOAD_VERSION + 1;
        let future = payload.to_base64();
        let err = decrypt(api_key, &future).unwrap_err();
        assert!(
            err.to_string().contains("Unsupported payload version"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_ct_eq() {
        let a = b"hello world";
        let b = b"hello world";
        let c = b"hello worlx";
        assert!(ct_eq(a, b));
        assert!(!ct_eq(a, c));
        assert!(!ct_eq(a, b""));
    }

    #[test]
    fn test_cache_keys_unique() {
        let key1 = hash_api_key("test-api-key-12345");
        let key2 = hash_api_key("test-api-key-12346");
        let key3 = hash_api_key("test-api-key-12345");
        assert_ne!(key1, key2, "different keys must produce different hashes");
        assert_eq!(key1, key3, "same key must produce same hash");
    }
}

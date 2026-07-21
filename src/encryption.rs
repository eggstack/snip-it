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
//! ```ignore
//! // Internal API — not available to external consumers.
//! // use snip_it::encryption::{decrypt, encrypt};
//! ```

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
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
/// Uses SHA-256 to avoid cache-key collisions that could cause one user's
/// derived key to be served from another user's cache entry.
fn hash_api_key(api_key: &str) -> String {
    let hash = Sha256::digest(api_key.as_bytes());
    format!("{:016x}", u64::from_le_bytes(hash[..8].try_into().unwrap()))
}

/// Session-local cache for derived keys to avoid re-running Argon2id
/// for the same (api_key, salt) pair during a sync operation.
/// Key: (hashed_api_key, base64(salt)), Value: derived key bytes
static KEY_CACHE: LazyLock<Mutex<HashMap<(String, String), [u8; 32]>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Clear the session key cache. Should be called at the end of a sync operation.
pub fn clear_key_cache() {
    if let Ok(mut cache) = KEY_CACHE.lock() {
        for mut key in cache.drain().map(|(_, v)| v) {
            key.zeroize();
        }
    }
}

#[derive(Zeroize, ZeroizeOnDrop, Default)]
struct DerivedKey([u8; 32]);

impl DerivedKey {
    fn new(key: [u8; 32]) -> Self {
        Self(key)
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
const SALT_SIZE: usize = 16;

#[cfg(test)]
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Encrypted data container with salt, nonce, and ciphertext.
///
/// The salt and nonce are stored alongside the ciphertext to enable
/// decryption without separate key exchange.
pub struct EncryptedPayload {
    pub salt: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl EncryptedPayload {
    pub fn to_base64(&self) -> String {
        let mut combined = Vec::with_capacity(SALT_SIZE + NONCE_SIZE + self.ciphertext.len());
        combined.extend_from_slice(&self.salt);
        combined.extend_from_slice(&self.nonce);
        combined.extend_from_slice(&self.ciphertext);
        BASE64.encode(&combined)
    }

    pub fn from_base64(data: &str) -> CryptoResult<Self> {
        let combined = BASE64
            .decode(data)
            .map_err(|e| CryptoError::InvalidData(format!("Failed to decode base64: {e}")))?;

        if combined.len() < SALT_SIZE + NONCE_SIZE {
            return Err(CryptoError::InvalidData("Data too short".to_string()));
        }

        let salt = combined[..SALT_SIZE].to_vec();
        let nonce = combined[SALT_SIZE..SALT_SIZE + NONCE_SIZE].to_vec();
        let ciphertext = combined[SALT_SIZE + NONCE_SIZE..].to_vec();

        Ok(Self {
            salt,
            nonce,
            ciphertext,
        })
    }
}

fn derive_key(api_key: &str, salt: &[u8]) -> CryptoResult<DerivedKey> {
    let salt_b64 = BASE64.encode(salt);
    let cache_key = (hash_api_key(api_key), salt_b64);

    // Check cache first
    {
        if let Ok(cache) = KEY_CACHE.lock()
            && let Some(cached) = cache.get(&cache_key)
        {
            return Ok(DerivedKey::new(*cached));
        }
    }

    let salt_string = SaltString::encode_b64(salt)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Salt encoding failed: {e}")))?;

    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(
            ARGON2_MEMORY_COST_KIB,
            ARGON2_TIME_COST,
            ARGON2_PARALLELISM,
            Some(32),
        )
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Invalid Argon2 params: {e}")))?,
    );

    let hash = argon2
        .hash_password(api_key.as_bytes(), &salt_string)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Hashing failed: {e}")))?;

    let hash_output = hash
        .hash
        .ok_or_else(|| CryptoError::KeyDerivationFailed("No hash output".to_string()))?;

    let hash_bytes = hash_output.as_bytes();
    if hash_bytes.len() < 32 {
        return Err(CryptoError::KeyDerivationFailed(
            "Argon2 output too short for AES-256 key".to_string(),
        ));
    }
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&hash_bytes[..32]);

    // Cache the derived key for future use with the same (api_key, salt)
    if let Ok(mut cache) = KEY_CACHE.lock() {
        // Evict half the entries when cache is full. HashMap iteration order is
        // arbitrary, but this is acceptable for a session-local cache — re-deriving
        // a key costs less than the initial Argon2id computation.
        if cache.len() >= MAX_KEY_CACHE_SIZE {
            let keys_to_remove: Vec<_> =
                cache.keys().take(MAX_KEY_CACHE_SIZE / 2).cloned().collect();
            for key in keys_to_remove {
                if let Some(mut old_key) = cache.remove(&key) {
                    old_key.zeroize();
                }
            }
        }
        cache.insert(cache_key, key_bytes);
    }

    Ok(DerivedKey::new(key_bytes))
}

pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String> {
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let mut key = derive_key(api_key, &salt)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Key init failed: {e}")))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Encryption failed: {e}")))?;

    drop(std::mem::take(&mut key));

    let payload = EncryptedPayload {
        salt: salt.to_vec(),
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    };

    Ok(payload.to_base64())
}

pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String> {
    let payload = EncryptedPayload::from_base64(encrypted_data)?;

    let mut key = derive_key(api_key, &payload.salt)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Key init failed: {e}")))?;

    let nonce = Nonce::from_slice(&payload.nonce);

    let plaintext = cipher
        .decrypt(nonce, payload.ciphertext.as_ref())
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

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
//! ```rust,ignore
//! use snp::encryption::{encrypt, decrypt};
//!
//! let api_key = "your-api-key";
//! let encrypted = encrypt(api_key, "sensitive data")?;
//! let decrypted = decrypt(api_key, &encrypted)?;
//! ```

use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use thiserror::Error;
use zeroize::Zeroize;

const ARGON2_MEMORY_COST_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum for Argon2id
const ARGON2_TIME_COST: u32 = 3;
const ARGON2_PARALLELISM: u32 = 4;

struct DerivedKey([u8; 32]);

impl DerivedKey {
    fn new(key: [u8; 32]) -> Self {
        Self(key)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Zeroize for DerivedKey {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for DerivedKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

#[derive(Error, Debug)]
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
            .map_err(|e| CryptoError::InvalidData(format!("Failed to decode base64: {}", e)))?;

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
    let salt_string = SaltString::encode_b64(salt)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Salt encoding failed: {}", e)))?;

    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(
            ARGON2_MEMORY_COST_KIB,
            ARGON2_TIME_COST,
            ARGON2_PARALLELISM,
            Some(32),
        )
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Invalid Argon2 params: {}", e)))?,
    );

    let hash = argon2
        .hash_password(api_key.as_bytes(), &salt_string)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Hashing failed: {}", e)))?;

    let hash_output = hash
        .hash
        .ok_or_else(|| CryptoError::KeyDerivationFailed("No hash output".to_string()))?;

    let hash_bytes = hash_output.as_bytes();
    if hash_bytes.len() < 32 {
        return Err(CryptoError::KeyDerivationFailed(
            "Argon2 output too short for AES-256 key".to_string(),
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash_bytes[..32]);

    Ok(DerivedKey::new(key))
}

pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String> {
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let key = derive_key(api_key, &salt)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Key init failed: {}", e)))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| CryptoError::EncryptionFailed(format!("Encryption failed: {}", e)))?;

    let payload = EncryptedPayload {
        salt: salt.to_vec(),
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    };

    drop(key);

    Ok(payload.to_base64())
}

pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String> {
    let payload = EncryptedPayload::from_base64(encrypted_data)?;

    let key = derive_key(api_key, &payload.salt)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_slice())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Key init failed: {}", e)))?;

    let nonce = Nonce::from_slice(&payload.nonce);

    let plaintext = cipher
        .decrypt(nonce, payload.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Decryption failed: {}", e)))?;

    drop(key);

    String::from_utf8(plaintext)
        .map_err(|e| CryptoError::DecryptionFailed(format!("UTF-8 conversion failed: {}", e)))
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
}

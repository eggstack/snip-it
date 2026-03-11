use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use thiserror::Error;

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

fn derive_key(api_key: &str, salt: &[u8]) -> CryptoResult<[u8; 32]> {
    let salt_string = SaltString::encode_b64(salt)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Salt encoding failed: {}", e)))?;

    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(api_key.as_bytes(), &salt_string)
        .map_err(|e| CryptoError::KeyDerivationFailed(format!("Hashing failed: {}", e)))?;

    let hash_output = hash
        .hash
        .ok_or_else(|| CryptoError::KeyDerivationFailed("No hash output".to_string()))?;

    let hash_bytes = hash_output.as_bytes();
    let mut key = [0u8; 32];
    let copy_len = hash_bytes.len().min(32);
    key[..copy_len].copy_from_slice(&hash_bytes[..copy_len]);

    Ok(key)
}

pub fn encrypt(api_key: &str, plaintext: &str) -> CryptoResult<String> {
    let mut salt = [0u8; SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let key = derive_key(api_key, &salt)?;

    let cipher = Aes256Gcm::new_from_slice(&key)
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

    Ok(payload.to_base64())
}

pub fn decrypt(api_key: &str, encrypted_data: &str) -> CryptoResult<String> {
    let payload = EncryptedPayload::from_base64(encrypted_data)?;

    let key = derive_key(api_key, &payload.salt)?;

    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| CryptoError::DecryptionFailed(format!("Key init failed: {}", e)))?;

    let nonce = Nonce::from_slice(&payload.nonce);

    let plaintext = cipher
        .decrypt(nonce, payload.ciphertext.as_ref())
        .map_err(|e| CryptoError::DecryptionFailed(format!("Decryption failed: {}", e)))?;

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
}

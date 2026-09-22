//! Envelope encryption primitives (research.md §5, Constitution Principle
//! VI). A project master key is derived from a user passphrase via
//! Argon2id; every individual blob gets its own random data-encryption key
//! (DEK), which is itself wrapped (AES-256-GCM) under the master key. This
//! lets a single blob be re-keyed without touching the rest of the project.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use thiserror::Error;

pub const MASTER_KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 12;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("key derivation failed")]
    Derivation,
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed: credential invalid or content tampered")]
    Decrypt,
}

/// A derived, in-memory-only project master key. Never persisted; re-derived
/// from the passphrase + stored salt on every service start.
pub struct MasterKey([u8; MASTER_KEY_LEN]);

impl MasterKey {
    /// Derives the master key from a user passphrase and a project-specific
    /// salt (persisted alongside the project, not secret on its own).
    pub fn derive(passphrase: &str, salt: &[u8]) -> Result<Self, CryptoError> {
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
        let mut out = [0u8; MASTER_KEY_LEN];
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut out)
            .map_err(|_| CryptoError::Derivation)?;
        Ok(Self(out))
    }

    pub fn as_bytes(&self) -> &[u8; MASTER_KEY_LEN] {
        &self.0
    }

    /// Wraps a fresh data-encryption key under this master key.
    pub fn wrap_key(&self, dek: &[u8; 32]) -> Result<WrappedKey, CryptoError> {
        encrypt_with_key(&self.0, dek)
    }

    pub fn unwrap_key(&self, wrapped: &WrappedKey) -> Result<[u8; 32], CryptoError> {
        let plain = decrypt_with_key(&self.0, wrapped)?;
        plain
            .try_into()
            .map_err(|_| CryptoError::Decrypt)
            .map(|arr: [u8; 32]| arr)
    }
}

/// Generates a random salt suitable for `MasterKey::derive`.
pub fn generate_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// A random 256-bit data-encryption key, generated per blob/row.
pub fn generate_dek() -> [u8; 32] {
    let mut dek = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut dek);
    dek
}

#[derive(Debug, Clone)]
pub struct WrappedKey {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// AES-256-GCM encrypt with an explicit key (used for both key-wrapping and
/// direct content encryption, keeping the primitive shared).
pub fn encrypt_with_key(key_bytes: &[u8; 32], plaintext: &[u8]) -> Result<WrappedKey, CryptoError> {
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::Encrypt)?;
    Ok(WrappedKey {
        nonce: nonce_bytes,
        ciphertext,
    })
}

pub fn decrypt_with_key(
    key_bytes: &[u8; 32],
    wrapped: &WrappedKey,
) -> Result<Vec<u8>, CryptoError> {
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(&wrapped.nonce);
    cipher
        .decrypt(nonce, wrapped.ciphertext.as_ref())
        .map_err(|_| CryptoError::Decrypt)
}

/// Envelope-encrypts `plaintext` under a fresh DEK, itself wrapped by
/// `master`. Returns the wrapped DEK and the encrypted content — the pair
/// that a blob store persists together for one blob (data-model.md, blob
/// store).
pub fn envelope_encrypt(
    master: &MasterKey,
    plaintext: &[u8],
) -> Result<(WrappedKey, WrappedKey), CryptoError> {
    let dek = generate_dek();
    let wrapped_dek = master.wrap_key(&dek)?;
    let wrapped_content = encrypt_with_key(&dek, plaintext)?;
    Ok((wrapped_dek, wrapped_content))
}

pub fn envelope_decrypt(
    master: &MasterKey,
    wrapped_dek: &WrappedKey,
    wrapped_content: &WrappedKey,
) -> Result<Vec<u8>, CryptoError> {
    let dek = master.unwrap_key(wrapped_dek)?;
    decrypt_with_key(&dek, wrapped_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let salt = generate_salt();
        let master = MasterKey::derive("correct horse battery staple", &salt).unwrap();
        let (wrapped_dek, wrapped_content) = envelope_encrypt(&master, b"pixel bytes").unwrap();
        let plain = envelope_decrypt(&master, &wrapped_dek, &wrapped_content).unwrap();
        assert_eq!(plain, b"pixel bytes");
    }

    #[test]
    fn wrong_passphrase_fails_to_decrypt() {
        let salt = generate_salt();
        let master = MasterKey::derive("correct horse battery staple", &salt).unwrap();
        let (wrapped_dek, wrapped_content) = envelope_encrypt(&master, b"pixel bytes").unwrap();

        let wrong_master = MasterKey::derive("wrong passphrase", &salt).unwrap();
        assert!(envelope_decrypt(&wrong_master, &wrapped_dek, &wrapped_content).is_err());
    }
}

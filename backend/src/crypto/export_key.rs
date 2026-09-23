//! Export Bundle key hierarchy (research.md §5, FR-035). Deliberately
//! shares no state with the project `MasterKey`: the key comes only from an
//! export credential plus a per-bundle random salt, so a bundle can be
//! opened without (and can never reveal) the project passphrase, and the
//! credential is structurally unable to live inside the bundle it protects.
//!
//! Container layout (all offsets fixed, so structure can be checked before
//! any decryption is attempted):
//!
//! ```text
//! MAGIC(8) | salt(16) | dek_nonce(12) | wrapped_dek(48) | body_nonce(12) | ciphertext | blake3(everything before)(32)
//! ```
//!
//! The trailing digest covers the header and ciphertext, so a flipped bit is
//! reported as `Integrity` without ever consulting a credential.

use argon2::{Algorithm, Argon2, Params, Version};
use thiserror::Error;

use super::{
    decrypt_with_key, encrypt_with_key, generate_dek, generate_salt, CryptoError, WrappedKey,
    NONCE_LEN,
};

const MAGIC: &[u8; 8] = b"RSYBNDL1";
const SALT_LEN: usize = 16;
const WRAPPED_DEK_LEN: usize = 32 + 16;
const DIGEST_LEN: usize = 32;
const HEADER_LEN: usize = 8 + SALT_LEN + NONCE_LEN + WRAPPED_DEK_LEN + NONCE_LEN;
const KDF_CONTEXT: &[u8] = b"rosaray/export-bundle/v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BundleCryptoError {
    #[error("not a Rosaray export bundle")]
    Malformed,
    #[error("bundle integrity check failed")]
    Integrity,
    #[error("export credential does not open this bundle")]
    Credential,
    #[error("encryption failed")]
    Encrypt,
}

/// Derived, in-memory-only key for one Export Bundle.
pub struct ExportKey([u8; 32]);

impl ExportKey {
    pub fn derive(credential: &str, salt: &[u8]) -> Result<Self, CryptoError> {
        let mut domain_salt = KDF_CONTEXT.to_vec();
        domain_salt.extend_from_slice(salt);
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
        let mut out = [0u8; 32];
        argon2
            .hash_password_into(credential.as_bytes(), &domain_salt, &mut out)
            .map_err(|_| CryptoError::Derivation)?;
        Ok(Self(out))
    }

    /// Opaque, non-invertible reference to this key (`ExportBundle.credential_key_id`).
    pub fn key_id(&self) -> String {
        let derived = blake3::derive_key("rosaray export key id v1", &self.0);
        derived[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Encrypts `plaintext` into a self-contained bundle container, returning
/// the container bytes and the opaque key id.
pub fn seal(credential: &str, plaintext: &[u8]) -> Result<(Vec<u8>, String), BundleCryptoError> {
    let salt = generate_salt();
    let key = ExportKey::derive(credential, &salt).map_err(|_| BundleCryptoError::Encrypt)?;
    let dek = generate_dek();
    let wrapped_dek = encrypt_with_key(&key.0, &dek).map_err(|_| BundleCryptoError::Encrypt)?;
    let body = encrypt_with_key(&dek, plaintext).map_err(|_| BundleCryptoError::Encrypt)?;

    let mut out = Vec::with_capacity(HEADER_LEN + body.ciphertext.len() + DIGEST_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&wrapped_dek.nonce);
    out.extend_from_slice(&wrapped_dek.ciphertext);
    out.extend_from_slice(&body.nonce);
    out.extend_from_slice(&body.ciphertext);
    let digest = blake3::hash(&out);
    out.extend_from_slice(digest.as_bytes());
    Ok((out, key.key_id()))
}

/// Verifies the container's integrity digest, then opens it with the
/// credential. Nothing about the payload is revealed unless both succeed.
pub fn open(credential: &str, container: &[u8]) -> Result<Vec<u8>, BundleCryptoError> {
    if container.len() < HEADER_LEN + 16 + DIGEST_LEN || &container[..8] != MAGIC {
        return Err(BundleCryptoError::Malformed);
    }
    let (signed, digest) = container.split_at(container.len() - DIGEST_LEN);
    if blake3::hash(signed).as_bytes() != digest {
        return Err(BundleCryptoError::Integrity);
    }

    let salt = &signed[8..8 + SALT_LEN];
    let mut at = 8 + SALT_LEN;
    let dek_nonce: [u8; NONCE_LEN] = signed[at..at + NONCE_LEN].try_into().unwrap();
    at += NONCE_LEN;
    let wrapped_dek = WrappedKey {
        nonce: dek_nonce,
        ciphertext: signed[at..at + WRAPPED_DEK_LEN].to_vec(),
    };
    at += WRAPPED_DEK_LEN;
    let body_nonce: [u8; NONCE_LEN] = signed[at..at + NONCE_LEN].try_into().unwrap();
    at += NONCE_LEN;
    let body = WrappedKey {
        nonce: body_nonce,
        ciphertext: signed[at..].to_vec(),
    };

    let key = ExportKey::derive(credential, salt).map_err(|_| BundleCryptoError::Credential)?;
    let dek: [u8; 32] = decrypt_with_key(&key.0, &wrapped_dek)
        .map_err(|_| BundleCryptoError::Credential)?
        .try_into()
        .map_err(|_| BundleCryptoError::Credential)?;
    // The digest already vouched for the bytes, so a body failure with a
    // valid DEK can only mean a deliberately re-signed forgery.
    decrypt_with_key(&dek, &body).map_err(|_| BundleCryptoError::Integrity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (bundle, _) = seal("export-secret", b"research content").unwrap();
        assert_eq!(open("export-secret", &bundle).unwrap(), b"research content");
    }

    #[test]
    fn wrong_credential_is_rejected() {
        let (bundle, _) = seal("export-secret", b"research content").unwrap();
        assert_eq!(open("nope", &bundle), Err(BundleCryptoError::Credential));
    }

    #[test]
    fn flipped_bit_is_integrity_failure() {
        let (mut bundle, _) = seal("export-secret", b"research content").unwrap();
        bundle[HEADER_LEN + 2] ^= 1;
        assert_eq!(
            open("export-secret", &bundle),
            Err(BundleCryptoError::Integrity)
        );
    }

    #[test]
    fn plaintext_and_credential_are_absent_from_bundle() {
        let (bundle, _) = seal("export-secret", b"research content").unwrap();
        let has = |needle: &[u8]| bundle.windows(needle.len()).any(|w| w == needle);
        assert!(!has(b"research content"));
        assert!(!has(b"export-secret"));
    }

    #[test]
    fn garbage_is_malformed() {
        assert_eq!(
            open("x", b"short"),
            Err(BundleCryptoError::Malformed)
        );
    }
}

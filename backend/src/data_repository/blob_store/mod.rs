//! Content-addressed, encrypted-at-rest blob store (data-model.md Artifact,
//! research.md §3/§5). Every blob is stored under two files keyed by its
//! content identity: a wrapped data-encryption key and the AES-256-GCM
//! ciphertext of the content itself, so a single blob can be re-keyed
//! without touching any other blob or the relational metadata.

use crate::crypto::{self, CryptoError, MasterKey, WrappedKey, NONCE_LEN};
use crate::domain::content_identity::content_identity;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BlobStoreError {
    #[error("blob not found for this content identity")]
    NotFound,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("decrypted content identity does not match the requested identity")]
    Integrity,
}

pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn shard_dir(&self, content_identity: &str) -> PathBuf {
        self.root.join(&content_identity[0..2])
    }

    fn key_path(&self, content_identity: &str) -> PathBuf {
        self.shard_dir(content_identity)
            .join(format!("{content_identity}.key"))
    }

    fn bin_path(&self, content_identity: &str) -> PathBuf {
        self.shard_dir(content_identity)
            .join(format!("{content_identity}.bin"))
    }

    pub fn exists(&self, content_identity: &str) -> bool {
        self.bin_path(content_identity).is_file()
    }

    /// Writes `bytes` under their own content identity. Idempotent: writing
    /// the same content identity again is a no-op (the store is
    /// content-addressed, so identical bytes never need re-encryption).
    pub fn write(&self, master: &MasterKey, bytes: &[u8]) -> Result<String, BlobStoreError> {
        let identity = content_identity(bytes);
        if self.exists(&identity) {
            return Ok(identity);
        }
        let dir = self.shard_dir(&identity);
        std::fs::create_dir_all(&dir)?;

        let (wrapped_dek, wrapped_content) = crypto::envelope_encrypt(master, bytes)?;

        write_wrapped(&self.key_path(&identity), &wrapped_dek)?;
        write_wrapped(&self.bin_path(&identity), &wrapped_content)?;
        Ok(identity)
    }

    /// Reads and decrypts the blob for `content_identity`, re-verifying that
    /// the decrypted bytes still hash to the requested identity (FR-023:
    /// integrity check before disclosure).
    pub fn read(
        &self,
        master: &MasterKey,
        requested_identity: &str,
    ) -> Result<Vec<u8>, BlobStoreError> {
        if !self.exists(requested_identity) {
            return Err(BlobStoreError::NotFound);
        }
        let wrapped_dek = read_wrapped(&self.key_path(requested_identity))?;
        let wrapped_content = read_wrapped(&self.bin_path(requested_identity))?;
        let plaintext = crypto::envelope_decrypt(master, &wrapped_dek, &wrapped_content)?;
        if content_identity(&plaintext) != requested_identity {
            return Err(BlobStoreError::Integrity);
        }
        Ok(plaintext)
    }
}

fn write_wrapped(path: &Path, wrapped: &WrappedKey) -> io::Result<()> {
    let mut out = Vec::with_capacity(NONCE_LEN + wrapped.ciphertext.len());
    out.extend_from_slice(&wrapped.nonce);
    out.extend_from_slice(&wrapped.ciphertext);
    std::fs::write(path, out)
}

fn read_wrapped(path: &Path) -> io::Result<WrappedKey> {
    let raw = std::fs::read(path)?;
    if raw.len() < NONCE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated blob file",
        ));
    }
    let (nonce, ciphertext) = raw.split_at(NONCE_LEN);
    Ok(WrappedKey {
        nonce: nonce.try_into().expect("checked length above"),
        ciphertext: ciphertext.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path()).unwrap();
        let master = MasterKey::derive("passphrase", &crypto::generate_salt()).unwrap();

        let identity = store.write(&master, b"grayscale pixels").unwrap();
        assert_eq!(identity, content_identity(b"grayscale pixels"));

        let read_back = store.read(&master, &identity).unwrap();
        assert_eq!(read_back, b"grayscale pixels");
    }

    #[test]
    fn missing_blob_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path()).unwrap();
        let master = MasterKey::derive("passphrase", &crypto::generate_salt()).unwrap();
        let missing = content_identity(b"never written");
        assert!(matches!(
            store.read(&master, &missing),
            Err(BlobStoreError::NotFound)
        ));
    }
}

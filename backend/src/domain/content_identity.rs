/// BLAKE3 content-identity hashing for arbitrary byte content (research.md §4).
/// Used both for source-image de-duplication and for content-addressed blob
/// storage keys — the same identity must be reproducible from the same bytes
/// on any machine, with no dependency on filesystem metadata.
pub fn content_identity(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_identity() {
        assert_eq!(content_identity(b"hello"), content_identity(b"hello"));
    }

    #[test]
    fn different_bytes_different_identity() {
        assert_ne!(content_identity(b"hello"), content_identity(b"world"));
    }
}

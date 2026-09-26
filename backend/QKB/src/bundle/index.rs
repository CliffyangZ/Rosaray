//! Published-version index read straight from the folders (the KB files, not
//! the catalog, are authoritative).

use std::path::{Path, PathBuf};

use super::model::{parse_yaml, BundleLock, Kind};
use super::read::LOCK_FILE;
use crate::catalog::scan::discover_bundle_dirs;

#[derive(Debug, Clone)]
pub struct PublishedRef {
    pub kind: Kind,
    pub id: String,
    pub version: String,
    pub content_id: String,
    pub dir: PathBuf,
}

/// Every locked bundle on disk, by lightly reading identity and `bundle.lock`.
pub fn published_index(root: &Path) -> Vec<PublishedRef> {
    let mut out = Vec::new();
    for dir in discover_bundle_dirs(root) {
        let Ok(text) = std::fs::read_to_string(dir.join(LOCK_FILE)) else { continue };
        let Ok(lock) = parse_yaml::<BundleLock>(&text) else { continue };
        let kind = if dir.join("ALGOPIPE.md").is_file() { Kind::Algopipe } else { Kind::Algonode };
        out.push(PublishedRef { kind, id: lock.id, version: lock.version, content_id: lock.content_id, dir });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    None,
    /// The same `id@version` with the same content is already present.
    Identical,
    /// The same `id@version` exists with different content (`version_exists`).
    Conflict,
}

pub fn detect_existing(index: &[PublishedRef], id: &str, version: &str, content_id: &str) -> Existing {
    match index.iter().find(|p| p.id == id && p.version == version) {
        None => Existing::None,
        Some(p) if p.content_id == content_id => Existing::Identical,
        Some(_) => Existing::Conflict,
    }
}

/// Reads a bundle directory as `(relative path, bytes)` pairs.
pub fn read_files(dir: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for (rel, _) in crate::identity::file_hashes(dir)? {
        out.push((rel.clone(), std::fs::read(dir.join(&rel))?));
    }
    Ok(out)
}

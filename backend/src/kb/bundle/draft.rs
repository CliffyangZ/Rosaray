//! Draft revision hashing and compare-and-swap save (FR-044): a draft may be
//! edited by external tools, so every write names the revision it was based
//! on and is refused if the directory has moved on.

use std::collections::BTreeMap;
use std::path::Path;

use super::read::{read_bundle, Bundle};
use super::write::{check_relative, ensure_writable, write_bundle_atomic, WriteError};
use crate::designer::validate::finding::Finding;
use crate::kb::identity::{content_id_of, file_hashes};

/// Opaque revision string of a draft directory: a hash over every file.
pub fn revision_of(dir: &Path) -> std::io::Result<String> {
    if !dir.exists() {
        return Ok(content_id_of([]));
    }
    let hashes = file_hashes(dir)?;
    Ok(content_id_of(hashes.iter().map(|(p, h)| (p.as_str(), h.as_str()))))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DraftConflict {
    pub on_disk_revision: String,
    /// Files whose on-disk content differs from what the caller tried to
    /// save (names only — the API layer fetches contents for the banner).
    pub changed_files: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("draft_conflict: the draft changed on disk since revision was read")]
    Conflict(DraftConflict),
    #[error(transparent)]
    Write(#[from] WriteError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Opens a draft, returning the parsed bundle (if it parses), its findings,
/// and the current revision.
pub fn open_draft(dir: &Path) -> std::io::Result<(Option<Bundle>, Vec<Finding>, String)> {
    let revision = revision_of(dir)?;
    let (bundle, findings) = read_bundle(dir);
    Ok((bundle, findings, revision))
}

/// Saves `files` (relative path → bytes) over the draft, keeping every other
/// existing file, and deleting `remove`. Fails with `Conflict` if the draft's
/// current revision is not `base_revision`. Returns the new revision.
pub fn save_draft(
    dir: &Path,
    base_revision: &str,
    files: &[(String, Vec<u8>)],
    remove: &[String],
) -> Result<String, DraftError> {
    ensure_writable(dir)?;
    for (rel, _) in files {
        check_relative(rel)?;
    }
    for rel in remove {
        check_relative(rel)?;
    }
    let on_disk = revision_of(dir)?;
    if on_disk != base_revision {
        return Err(DraftError::Conflict(DraftConflict {
            on_disk_revision: on_disk,
            changed_files: differing_files(dir, files)?,
        }));
    }

    let mut merged: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    if dir.exists() {
        for (rel, _) in file_hashes(dir)? {
            merged.insert(rel.clone(), std::fs::read(dir.join(&rel))?);
        }
    }
    for rel in remove {
        merged.remove(rel);
    }
    for (rel, bytes) in files {
        merged.insert(rel.clone(), bytes.clone());
    }
    let merged: Vec<(String, Vec<u8>)> = merged.into_iter().collect();
    write_bundle_atomic(dir, &merged)?;
    Ok(revision_of(dir)?)
}

fn differing_files(dir: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    for (rel, bytes) in files {
        let path = dir.join(rel);
        let same = path.is_file() && std::fs::read(&path)? == *bytes;
        if !same {
            out.push(rel.clone());
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(rel: &str, c: &str) -> (String, Vec<u8>) {
        (rel.to_string(), c.as_bytes().to_vec())
    }

    #[test]
    fn save_then_conflict_on_stale_revision() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("d");
        let base = revision_of(&dir).unwrap();
        let r1 = save_draft(&dir, &base, &[f("ALGONODE.md", "one")], &[]).unwrap();
        assert_ne!(r1, base);

        // An external tool edits the file behind our back.
        std::fs::write(dir.join("ALGONODE.md"), "external").unwrap();

        let err = save_draft(&dir, &r1, &[f("ALGONODE.md", "mine")], &[]).unwrap_err();
        match err {
            DraftError::Conflict(c) => {
                assert_eq!(c.changed_files, vec!["ALGONODE.md"]);
                assert_ne!(c.on_disk_revision, r1);
            }
            other => panic!("expected conflict, got {other:?}"),
        }
        assert_eq!(std::fs::read_to_string(dir.join("ALGONODE.md")).unwrap(), "external");
    }

    #[test]
    fn keeps_unlisted_files_and_honours_remove() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("d");
        let r0 = revision_of(&dir).unwrap();
        let r1 = save_draft(&dir, &r0, &[f("a", "1"), f("b", "2")], &[]).unwrap();
        let r2 = save_draft(&dir, &r1, &[f("a", "3")], &["b".to_string()]).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "3");
        assert!(!dir.join("b").exists());
        assert_ne!(r1, r2);
    }

    #[test]
    fn refuses_published_bundle() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("d");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bundle.lock"), "x").unwrap();
        let base = revision_of(&dir).unwrap();
        let err = save_draft(&dir, &base, &[f("a", "1")], &[]).unwrap_err();
        assert!(matches!(err, DraftError::Write(WriteError::PublishedImmutable(_))));
    }
}

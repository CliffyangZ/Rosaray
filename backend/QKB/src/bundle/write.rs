//! Atomic bundle writer (research §6–7): stage in `<dest>.tmp-<uuid>/`, fsync
//! files and directory, then `rename` into place. A path that holds a
//! `bundle.lock` is never opened for write (Constitution II, FR-043).

use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use uuid::Uuid;

use super::read::LOCK_FILE;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The destination holds a published (hash-locked) bundle.
    #[error("published_immutable: {0} is a published bundle and cannot be modified")]
    PublishedImmutable(PathBuf),
    #[error("unsafe bundle file path: {0}")]
    UnsafePath(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Fails with `PublishedImmutable` if `dest` holds a `bundle.lock`.
pub fn ensure_writable(dest: &Path) -> Result<(), WriteError> {
    if dest.join(LOCK_FILE).exists() {
        return Err(WriteError::PublishedImmutable(dest.to_path_buf()));
    }
    Ok(())
}

/// Accepts only relative, forward-slash paths with no `..` components.
pub fn check_relative(rel: &str) -> Result<PathBuf, WriteError> {
    let unsafe_path = || WriteError::UnsafePath(rel.to_string());
    if rel.is_empty() || rel.contains('\\') || rel.contains('\0') {
        return Err(unsafe_path());
    }
    let path = Path::new(rel);
    for c in path.components() {
        match c {
            Component::Normal(_) => {}
            _ => return Err(unsafe_path()),
        }
    }
    Ok(path.to_path_buf())
}

fn sibling(dest: &Path, tag: &str) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dest.with_file_name(format!("{name}.{tag}-{}", Uuid::new_v4()))
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    // Directory fsync is not supported on every platform (e.g. Windows).
    match File::open(dir).and_then(|d| d.sync_all()) {
        Ok(()) => Ok(()),
        Err(_) if cfg!(windows) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Writes the complete file set `files` as the bundle at `dest`. A missing
/// `dest` is created; an existing unlocked `dest` is replaced as a whole
/// (callers merge before calling — see `draft::save_draft`).
pub fn write_bundle_atomic(dest: &Path, files: &[(String, Vec<u8>)]) -> Result<(), WriteError> {
    ensure_writable(dest)?;
    let tmp = sibling(dest, "tmp");
    let result = stage_and_swap(dest, &tmp, files);
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

fn stage_and_swap(dest: &Path, tmp: &Path, files: &[(String, Vec<u8>)]) -> Result<(), WriteError> {
    fs::create_dir_all(tmp)?;
    for (rel, bytes) in files {
        let path = tmp.join(check_relative(rel)?);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = File::create(&path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // fsync every directory we created, deepest first is unnecessary for
    // correctness of the final rename; syncing each is cheap for bundles.
    sync_tree(tmp)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        // POSIX rename cannot replace a non-empty directory: move the old
        // copy aside first, then swap, then discard it.
        let old = sibling(dest, "old");
        fs::rename(dest, &old)?;
        if let Err(e) = fs::rename(tmp, dest) {
            let _ = fs::rename(&old, dest);
            return Err(e.into());
        }
        let _ = fs::remove_dir_all(&old);
    } else {
        fs::rename(tmp, dest)?;
    }
    if let Some(parent) = dest.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

fn sync_tree(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            sync_tree(&path)?;
        }
    }
    sync_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        pairs.iter().map(|(p, c)| (p.to_string(), c.as_bytes().to_vec())).collect()
    }

    #[test]
    fn writes_new_and_replaces_existing_without_leftovers() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("b");
        write_bundle_atomic(&dest, &files(&[("a.md", "1"), ("sub/x.yaml", "2")])).unwrap();
        assert_eq!(fs::read_to_string(dest.join("sub/x.yaml")).unwrap(), "2");
        write_bundle_atomic(&dest, &files(&[("a.md", "3")])).unwrap();
        assert_eq!(fs::read_to_string(dest.join("a.md")).unwrap(), "3");
        assert!(!dest.join("sub").exists());
        let names: Vec<_> = fs::read_dir(root.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "no tmp/old siblings remain: {names:?}");
    }

    #[test]
    fn refuses_locked_destination() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("b");
        write_bundle_atomic(&dest, &files(&[("a.md", "1"), (LOCK_FILE, "lock")])).unwrap();
        let err = write_bundle_atomic(&dest, &files(&[("a.md", "2")])).unwrap_err();
        assert!(matches!(err, WriteError::PublishedImmutable(_)));
        assert_eq!(fs::read_to_string(dest.join("a.md")).unwrap(), "1");
    }

    #[test]
    fn rejects_unsafe_paths_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("b");
        for bad in ["../evil", "/abs", "a\\b", ""] {
            let err = write_bundle_atomic(&dest, &files(&[(bad, "x")])).unwrap_err();
            assert!(matches!(err, WriteError::UnsafePath(_)), "{bad}");
        }
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}

/// Best-effort read-only marking of a published bundle's files (research §7):
/// the guarantee is hash *detection*, not the permission bit.
pub fn mark_read_only(dir: &Path) {
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                mark_read_only(&path);
            } else if let Ok(meta) = fs::metadata(&path) {
                let mut perms = meta.permissions();
                perms.set_readonly(true);
                let _ = fs::set_permissions(&path, perms);
            }
        }
    }
}

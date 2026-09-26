//! Built-in seed AlgoNode bundles (research §10): the six nodes ported from the
//! prototype's registry. They are embedded in the binary and installed into an
//! empty knowledge base as *published* versions — idempotently, and never
//! overwriting anything already there. Their implementations are trusted
//! built-ins; they start `implemented` and become `technically_verified` only
//! when their `tests/` pass and a verification record is written (US4).

use std::path::Path;

use rusqlite::Connection;


pub struct SeedBundle {
    pub id: &'static str,
    pub version: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

macro_rules! seed {
    ($short:literal, $id:literal, [$(($rel:literal, $path:literal)),* $(,)?]) => {
        SeedBundle {
            id: $id,
            version: "1.0.0",
            files: &[
                ("ALGONODE.md", include_str!(concat!("bundles/", $short, "/ALGONODE.md"))),
                ("contract.yaml", include_str!(concat!("bundles/", $short, "/contract.yaml"))),
                ("implementation.yaml", include_str!(concat!("bundles/", $short, "/implementation.yaml"))),
                $(($rel, include_str!(concat!("bundles/", $short, "/", $path)))),*
            ],
        }
    };
}

pub fn seed_bundles() -> Vec<SeedBundle> {
    vec![
        seed!("image-source", "rosaray.image-source", [("tests/cases.yaml", "tests/cases.yaml")]),
        seed!("normalize", "rosaray.normalize", [("tests/cases.yaml", "tests/cases.yaml")]),
        seed!("gaussian-blur", "rosaray.gaussian-blur", [("tests/cases.yaml", "tests/cases.yaml")]),
        seed!("threshold", "rosaray.threshold", [("tests/cases.yaml", "tests/cases.yaml")]),
        seed!("morphology", "rosaray.morphology", [("tests/cases.yaml", "tests/cases.yaml")]),
        seed!("area", "rosaray.area", [("tests/cases.yaml", "tests/cases.yaml")]),
    ]
}

/// Installs any seed bundle not already present, through the same submission
/// path as any author: validated, technically verified by running its own
/// tests, then stored. Returns how many were installed. Never overwrites an
/// existing `id@version`.
pub fn install(conn: &Connection, root: &Path) -> std::io::Result<usize> {
    use crate::bundle::index::published_index;
    use crate::submit::{submit, Submission};
    let existing = published_index(root);
    let mut installed = 0;
    for seed in seed_bundles() {
        if existing.iter().any(|p| p.id == seed.id && p.version == seed.version) {
            continue;
        }
        let files = seed.files.iter().map(|(rel, text)| (rel.to_string(), text.as_bytes().to_vec())).collect();
        match submit(conn, root, Submission { files }) {
            Ok(a) if !a.already_present => installed += 1,
            Ok(_) => {}
            Err(e) => return Err(std::io::Error::other(format!("seed {}@{} rejected: {e:?}", seed.id, seed.version))),
        }
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::model::Trust;
    use crate::bundle::read::read_bundle;

    fn db(dir: &Path) -> Connection {
        crate::store::open(&dir.join("t.sqlite3")).unwrap()
    }

    #[test]
    fn installs_six_valid_published_seeds_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kb");
        crate::ensure_layout(&root).unwrap();
        let conn = db(dir.path());
        assert_eq!(install(&conn, &root).unwrap(), 6);
        assert_eq!(install(&conn, &root).unwrap(), 0, "second run installs nothing");

        for seed in seed_bundles() {
            let (bundle, findings) = read_bundle(&root.join("nodes").join(seed.id).join(seed.version));
            let bundle = bundle.expect("seed reads");
            assert!(findings.is_empty(), "{}: {findings:?}", seed.id);
            assert!(bundle.is_published());
            let advisory = crate::contract::bundle::validate_bundle(
                &bundle,
                &crate::contract::bundle::BundleCheck::files_only(),
            );
            let errors: Vec<_> = advisory.iter().filter(|f| f.is_error()).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", seed.id);
            assert_eq!(crate::trust::effective_trust(&root, &bundle), Trust::Builtin);
        }
        let page = crate::catalog::query::query_entries(&conn, &Default::default()).unwrap();
        assert_eq!(page.entries.len(), 6);
        assert!(page.entries.iter().all(|e| e.maturity.as_deref() == Some("technically_verified")));
    }

    #[test]
    fn never_rewrites_an_installed_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kb");
        crate::ensure_layout(&root).unwrap();
        let conn = db(dir.path());
        install(&conn, &root).unwrap();
        let before = crate::identity::file_hashes(&root.join("nodes/rosaray.threshold/1.0.0")).unwrap();
        assert_eq!(install(&conn, &root).unwrap(), 0);
        assert_eq!(crate::identity::file_hashes(&root.join("nodes/rosaray.threshold/1.0.0")).unwrap(), before);
    }
}

//! Built-in seed AlgoNode bundles (research §10): the six nodes ported from the
//! prototype's registry. They are embedded in the binary and installed into an
//! empty knowledge base as *published* versions — idempotently, and never
//! overwriting anything already there. Their implementations are trusted
//! built-ins; they start `implemented` and become `technically_verified` only
//! when their `tests/` pass and a verification record is written (US4).

use std::path::Path;

use rusqlite::Connection;
use serde_json::json;

use crate::data_repository::sqlite::kb_event_repo;
use crate::kb::bundle::frontmatter::set_field;
use crate::kb::bundle::model::{to_yaml, BundleLock, Kind, LockFile, Trust};
use crate::kb::bundle::read::LOCK_FILE;
use crate::kb::bundle::service::{published_index, sync_catalog};
use crate::kb::bundle::write::{mark_read_only, write_bundle_atomic};
use crate::kb::identity::content_id_of;
use crate::kb::trust;

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

/// Installs any seed bundle not already present as a published version.
/// Returns how many were installed. Never overwrites an existing `id@version`,
/// even one with different content (that is the researcher's data now).
pub fn install(conn: &Connection, root: &Path) -> std::io::Result<usize> {
    let existing = published_index(root);
    let mut installed = 0;
    for seed in seed_bundles() {
        if existing.iter().any(|p| p.id == seed.id && p.version == seed.version) {
            continue;
        }
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for (rel, text) in seed.files {
            let bytes = if *rel == "ALGONODE.md" {
                set_field(text, "status", &json!("published")).map_err(|e| std::io::Error::other(e.message))?.into_bytes()
            } else {
                text.as_bytes().to_vec()
            };
            files.push((rel.to_string(), bytes));
        }
        let hashed: Vec<(String, String)> =
            files.iter().map(|(p, b)| (p.clone(), format!("b3:{}", blake3::hash(b).to_hex()))).collect();
        let content_id = content_id_of(hashed.iter().map(|(p, h)| (p.as_str(), h.as_str())));
        let lock = BundleLock {
            schema: "quantify-kb/1".into(),
            id: seed.id.into(),
            version: seed.version.into(),
            content_id: content_id.clone(),
            computational_identity: None,
            release_kind: Some("knowledge".into()),
            disclosures: Vec::new(),
            dependency_summary: None,
            verification_summary: None,
            files: hashed
                .iter()
                .map(|(p, h)| LockFile { path: p.clone(), blake3: h.trim_start_matches("b3:").to_string() })
                .collect(),
        };
        files.push((LOCK_FILE.into(), to_yaml(&lock).map_err(std::io::Error::other)?.into_bytes()));
        let dest = root.join("nodes").join(seed.id).join(seed.version);
        write_bundle_atomic(&dest, &files).map_err(|e| std::io::Error::other(e.to_string()))?;
        mark_read_only(&dest);
        trust::record(root, seed.id, seed.version, &content_id, Trust::Builtin, "seed")?;
        kb_event_repo::append(
            conn,
            "published",
            &format!("{}@{}", seed.id, seed.version),
            &json!({ "kind": Kind::Algonode.as_str(), "release": "knowledge", "seed": true, "content_id": content_id }),
        )
        .map_err(std::io::Error::other)?;
        installed += 1;
    }
    if installed > 0 {
        sync_catalog(conn, root).map_err(std::io::Error::other)?;
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;
    use crate::kb::bundle::read::read_bundle;

    fn db(dir: &Path) -> Connection {
        let key = MasterKey::derive("pw", &crate::crypto::generate_salt()).unwrap();
        crate::data_repository::sqlite::open(&dir.join("t.sqlite3"), &key).unwrap()
    }

    #[test]
    fn installs_six_valid_published_seeds_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kb");
        crate::kb::ensure_layout(&root).unwrap();
        let conn = db(dir.path());
        assert_eq!(install(&conn, &root).unwrap(), 6);
        assert_eq!(install(&conn, &root).unwrap(), 0, "second run installs nothing");

        for seed in seed_bundles() {
            let (bundle, findings) = read_bundle(&root.join("nodes").join(seed.id).join(seed.version));
            let bundle = bundle.expect("seed reads");
            assert!(findings.is_empty(), "{}: {findings:?}", seed.id);
            assert!(bundle.is_published());
            let advisory = crate::designer::validate::bundle::validate_bundle(
                &bundle,
                &crate::designer::validate::bundle::BundleCheck::files_only(),
            );
            let errors: Vec<_> = advisory.iter().filter(|f| f.is_error()).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", seed.id);
            assert_eq!(crate::kb::trust::effective_trust(&root, &bundle), Trust::Builtin);
        }
        let page = crate::kb::catalog::query::query_entries(&conn, &Default::default()).unwrap();
        assert_eq!(page.entries.len(), 6);
        assert!(page.entries.iter().all(|e| e.maturity.as_deref() == Some("implemented")));
    }

    #[test]
    fn never_overwrites_an_existing_version() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kb");
        crate::kb::ensure_layout(&root).unwrap();
        let conn = db(dir.path());
        let custom = root.join("nodes/rosaray.threshold/1.0.0");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("bundle.lock"), "schema: quantify-kb/1\nid: rosaray.threshold\nversion: 1.0.0\ncontent_id: \"b3:x\"\nfiles: []\n").unwrap();
        std::fs::write(custom.join("marker.txt"), "mine").unwrap();
        assert_eq!(install(&conn, &root).unwrap(), 5);
        assert_eq!(std::fs::read_to_string(custom.join("marker.txt")).unwrap(), "mine");
    }
}

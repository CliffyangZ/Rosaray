//! FR-018 / SC-007: an old project directory is upgraded without touching any
//! legacy Dataset/Run file; only `knowledge-base/` is cleaned.

mod common;

use common::*;
use serde_json::json;

/// Builds a legacy-shaped project: Dataset/Run files plus a knowledge base that
/// holds executable versions, a draft, a knowledge-only pipe and a corrupt bundle.
fn legacy_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("rosaray.sqlite3"), b"SQLite format 3\0legacy-encrypted-bytes").unwrap();
    std::fs::write(p.join("rosaray.sqlite3-wal"), b"wal bytes").unwrap();
    std::fs::write(p.join("project.salt"), b"salt-salt-salt-salt").unwrap();
    std::fs::write(p.join("project.verifier"), b"verifier").unwrap();
    std::fs::create_dir_all(p.join("blobs/ab")).unwrap();
    std::fs::write(p.join("blobs/ab/abcdef"), b"\x89PNG\r\n\x1a\nlegacy image").unwrap();
    std::fs::create_dir_all(p.join("exports")).unwrap();
    std::fs::write(p.join("exports/run-1.rosaray"), b"ciphertext").unwrap();

    // A knowledge base as the legacy app left it.
    let kb = p.join("knowledge-base");
    rosaray_qkb::ensure_layout(&kb).unwrap();
    let conn = rosaray_qkb::store::open(&p.join("scratch.sqlite")).unwrap();
    assert_eq!(rosaray_qkb::seed::install(&conn, &kb).unwrap(), 6);
    drop(conn);
    std::fs::remove_file(p.join("scratch.sqlite")).unwrap();
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(p.join(format!("scratch.sqlite{ext}")));
    }
    std::fs::create_dir_all(kb.join("drafts/pipes/old.draft")).unwrap();
    std::fs::write(
        kb.join("drafts/pipes/old.draft/ALGOPIPE.md"),
        "---\nschema: quantify-kb/1\nkind: algopipe\nid: old.draft\nversion: 0.1.0\nname: Old draft\nstatus: draft\nresearch_use_only: true\n---\nBody\n",
    )
    .unwrap();
    std::fs::write(kb.join("drafts/pipes/old.draft/graph.yaml"), "schema: quantify-kb/1\nnodes: []\nedges: []\n").unwrap();
    std::fs::create_dir_all(kb.join("nodes/old.corrupt/1.0.0")).unwrap();
    std::fs::write(kb.join("nodes/old.corrupt/1.0.0/ALGONODE.md"), "garbage").unwrap();
    dir
}

fn legacy_hashes(dir: &std::path::Path) -> Vec<(String, String)> {
    ["rosaray.sqlite3", "rosaray.sqlite3-wal", "project.salt", "project.verifier"]
        .iter()
        .map(|f| (f.to_string(), blake3::hash(&std::fs::read(dir.join(f)).unwrap()).to_hex().to_string()))
        .chain(hash_tree(&dir.join("blobs")))
        .chain(hash_tree(&dir.join("exports")))
        .collect()
}

#[tokio::test]
async fn upgrading_leaves_legacy_files_byte_identical_and_cleans_only_the_knowledge_base() {
    let dir = legacy_project();
    let before = legacy_hashes(dir.path());
    let kb_nodes_before = hash_tree(&dir.path().join("knowledge-base/nodes/rosaray.area"));

    let path = dir.path().to_path_buf();
    let svc = spawn_in(&path, Some(dir)).await;
    assert!(svc.report.legacy_files_present);
    assert_eq!(svc.report.deleted_versions, 2, "the draft and the corrupt bundle");

    assert_eq!(legacy_hashes(&svc.project_dir), before, "Dataset/Run files were not touched");
    assert!(!svc.project_dir.join("knowledge-base/drafts").exists());
    assert!(!svc.project_dir.join("knowledge-base/nodes/old.corrupt").exists());
    assert_eq!(hash_tree(&svc.project_dir.join("knowledge-base/nodes/rosaray.area")), kb_nodes_before, "executable versions keep their content");

    // Everything executable is still readable; the deletions are explained.
    let (_, b) = svc.get(As::Frontend, "/qkb/v1/catalog/entries").await;
    assert_eq!(b["entries"].as_array().unwrap().len(), 6);
    let (_, d) = svc.get(As::Author, "/qkb/v1/deletions").await;
    let reasons: std::collections::BTreeSet<_> = d["deletions"].as_array().unwrap().iter().map(|r| r["reason_code"].as_str().unwrap().to_string()).collect();
    assert_eq!(reasons, ["draft_removed", "unrecognized_format"].iter().map(|s| s.to_string()).collect());
    // The QKB never opened the legacy database: it used its own file.
    assert!(svc.project_dir.join("qkb.sqlite").exists());
    let _ = json!({});
}

#[tokio::test]
async fn a_second_start_changes_nothing() {
    let dir = legacy_project();
    let first = spawn_in(dir.path(), None).await;
    let after_first = (hash_tree(&first.state.kb_root), legacy_hashes(dir.path()));
    let second = spawn_in(dir.path(), None).await;
    assert_eq!(second.report.deleted_versions, 0);
    assert_eq!(second.report.seeded, 0);
    assert_eq!((hash_tree(&second.state.kb_root), legacy_hashes(dir.path())), after_first);
}

#[tokio::test]
async fn a_fresh_install_needs_no_dataset_image_run_or_model() {
    let svc = spawn().await;
    assert!(!svc.report.legacy_files_present);
    assert_eq!(svc.report.seeded, 6);
    for legacy in ["rosaray.sqlite3", "blobs", "project.salt"] {
        assert!(!svc.project_dir.join(legacy).exists(), "{legacy} must not be created");
    }
    let (s, b) = svc.get(As::Frontend, "/qkb/v1/catalog/entries?q=threshold").await;
    assert_eq!(s, reqwest::StatusCode::OK);
    let ids: Vec<&str> = b["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"rosaray.threshold"), "{ids:?}");
    // System One being absent affects nothing: queries are simply never made.
    let (s, _) = svc.get(As::Frontend, "/qkb/v1/catalog/algonode/rosaray.threshold/1.0.0").await;
    assert_eq!(s, reqwest::StatusCode::OK);
}

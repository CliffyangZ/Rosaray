//! Quickstart §1 steps 5–6: the catalog is a derived index. Dropping it and
//! rebuilding loses nothing; a relocated bundle still resolves; invalid
//! bundles are indexed with findings rather than skipped.

mod common;

use reqwest::Method;
use rosaray_service::kb::catalog::query::{query_entries, EntryFilter};
use rosaray_service::kb::catalog::repo;
use serde_json::json;

fn entries_json(svc: &common::TestService) -> serde_json::Value {
    let db = svc.state.db.lock().unwrap();
    let page = query_entries(&db, &EntryFilter { limit: Some(500), ..Default::default() }).unwrap();
    serde_json::to_value(page.entries).unwrap()
}

#[tokio::test]
async fn dropping_the_derived_tables_and_rebuilding_loses_nothing() {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc.create_draft_from_fixture("algopipe", "acme.demo-pipe", "draft-pipe").await;
    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;
    // The corrupt fixture sits in the tree but never went through the service.
    common::copy_dir(&common::fixture_dir("corrupt-bundle"), &svc.kb_root.join("drafts/nodes/acme.corrupt"));
    svc.refresh_catalog();
    let before = entries_json(&svc);
    assert_eq!(before.as_array().unwrap().len(), 9);

    {
        let db = svc.state.db.lock().unwrap();
        repo::clear_derived(&db).unwrap();
        let n: i64 = db.query_row("SELECT count(*) FROM kb_catalog_entry", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "derived tables really are empty");
    }
    let (status, rebuilt) = svc.json(Method::POST, "/kb/rebuild", None).await;
    assert_eq!(status, 200, "{rebuilt}");
    assert_eq!(rebuilt["indexed"], 9);
    assert_eq!(rebuilt["invalid"], 1);
    assert_eq!(entries_json(&svc), before, "identical entry set after rebuild");
}

#[tokio::test]
async fn an_invalid_bundle_is_indexed_as_invalid_with_findings_never_skipped() {
    let svc = common::spawn().await;
    common::copy_dir(&common::fixture_dir("corrupt-bundle"), &svc.kb_root.join("drafts/nodes/acme.corrupt"));
    svc.refresh_catalog();

    let entries = svc.entries("").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["status"], "invalid");
    assert!(entries[0]["finding_count"].as_i64().unwrap() >= 1);
    // Filtering can exclude it only explicitly.
    assert!(svc.entries("?status=valid").await.is_empty());
    assert_eq!(svc.entries("?status=invalid").await.len(), 1);

    let (_, findings) = svc.json(Method::GET, "/kb/findings?status=invalid", None).await;
    let f = &findings["bundles"][0]["findings"][0];
    assert_eq!(f["code"], "frontmatter_invalid");
    assert!(!f["explanation"].as_str().unwrap().is_empty());
    assert!(!f["action"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn a_folder_with_no_description_file_is_still_reported() {
    let svc = common::spawn().await;
    let dir = svc.kb_root.join("nodes/mystery/1.0.0");
    common::write_bundle(&dir, &[("contract.yaml", "schema: quantify-kb/1\n")]);
    svc.refresh_catalog();
    let entries = svc.entries("").await;
    assert_eq!(entries[0]["status"], "invalid");
    let (_, findings) = svc.json(Method::GET, "/kb/findings?status=invalid", None).await;
    assert_eq!(findings["bundles"][0]["findings"][0]["code"], "bundle_file_missing");
}

/// Step 6 at the index level: a moved bundle keeps its identity and
/// content_id; the pipe that references it still resolves.
#[tokio::test]
async fn relocated_bundle_still_resolves_for_the_pipe_that_uses_it() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let thr = svc.content_id_of_published("rosaray.threshold", "1.0.0");
    let src = svc.content_id_of_published("rosaray.image-source", "1.0.0");
    let graph = format!(
        "schema: quantify-kb/1\nnodes:\n  - {{ instance_id: src, ref: {{ id: rosaray.image-source, version: 1.0.0, content_id: \"{src}\" }} }}\n  - {{ instance_id: th, ref: {{ id: rosaray.threshold, version: 1.0.0, content_id: \"{thr}\" }} }}\nedges:\n  - {{ from: src.image, to: th.image }}\n"
    );
    common::make_published_bundle(
        &svc.kb_root,
        "algopipe",
        "acme.pinned",
        "1.0.0",
        "knowledge",
        &[
            ("ALGOPIPE.md", "---\nschema: quantify-kb/1\nkind: algopipe\nid: acme.pinned\nversion: 1.0.0\nname: Pinned\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n"),
            ("graph.yaml", &graph),
        ],
    );
    svc.refresh_catalog();
    let pipe = svc.entries("?kind=algopipe").await.remove(0);
    assert_eq!(pipe["dependency_summary"], json!({ "total": 2, "unresolved": 0 }));

    std::fs::create_dir_all(svc.kb_root.join("elsewhere")).unwrap();
    std::fs::rename(
        svc.kb_root.join("nodes/rosaray.threshold/1.0.0"),
        svc.kb_root.join("elsewhere/threshold-moved"),
    )
    .unwrap();
    svc.refresh_catalog();
    let pipe = svc.entries("?kind=algopipe").await.remove(0);
    assert_eq!(pipe["dependency_summary"], json!({ "total": 2, "unresolved": 0 }), "still resolves");
    assert_eq!(svc.content_id_of_published("rosaray.threshold", "1.0.0"), thr, "same content_id");
}

/// A reference is never silently re-pointed: content drift is `unresolved`.
#[tokio::test]
async fn a_reference_whose_content_changed_is_unresolved_not_repointed() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let graph = "schema: quantify-kb/1\nnodes:\n  - { instance_id: th, ref: { id: rosaray.threshold, version: 1.0.0, content_id: \"b3:0000\" } }\nedges: []\n";
    common::make_published_bundle(
        &svc.kb_root,
        "algopipe",
        "acme.stale",
        "1.0.0",
        "knowledge",
        &[
            ("ALGOPIPE.md", "---\nschema: quantify-kb/1\nkind: algopipe\nid: acme.stale\nversion: 1.0.0\nname: Stale\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n"),
            ("graph.yaml", graph),
        ],
    );
    svc.refresh_catalog();
    let pipe = svc.entries("?kind=algopipe").await.remove(0);
    assert_eq!(pipe["dependency_summary"], json!({ "total": 1, "unresolved": 1 }));
    let (_, findings) = svc.json(Method::GET, "/kb/findings?id=acme.stale", None).await;
    let codes: Vec<_> = findings["bundles"][0]["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect();
    assert!(codes.contains(&"dependency_content_mismatch".to_string()), "{codes:?}");
}

/// A published bundle whose files drift from `bundle.lock` is `unavailable`,
/// never silently used (research §7).
#[tokio::test]
async fn a_modified_published_bundle_is_flagged_unavailable() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let file = svc.kb_root.join("nodes/rosaray.threshold/1.0.0/contract.yaml");
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();
    std::fs::write(&file, "tampered: true\n").unwrap();
    svc.refresh_catalog();

    let entries = svc.entries("?kind=algonode&status=published").await;
    let entry = entries.iter().find(|e| e["id"] == "rosaray.threshold").unwrap();
    assert_eq!(entry["availability"], "unavailable");
    let (_, findings) = svc.json(Method::GET, "/kb/findings?id=rosaray.threshold", None).await;
    assert!(findings["bundles"][0]["findings"].as_array().unwrap().iter().any(|f| f["code"] == "published_bundle_modified"));
}

/// Incremental refresh re-reads only what changed, and drops what vanished.
#[tokio::test]
async fn refresh_is_incremental_and_forgets_deleted_bundles() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let db_indexed_at = |svc: &common::TestService, id: &str| -> String {
        let db = svc.state.db.lock().unwrap();
        db.query_row("SELECT indexed_at FROM kb_catalog_entry WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };
    let untouched = db_indexed_at(&svc, "rosaray.area");
    std::thread::sleep(std::time::Duration::from_millis(20));

    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;
    assert_eq!(db_indexed_at(&svc, "rosaray.area"), untouched, "unchanged bundles are not re-read");

    std::fs::remove_dir_all(svc.kb_root.join("drafts/nodes/acme.spec-only")).unwrap();
    svc.refresh_catalog();
    assert!(svc.entries("?q=sharpness").await.is_empty(), "search index forgot it too");
    assert_eq!(svc.entries("").await.len(), 6);
}

/// Catalog scanning must never write to bundle files (SC-013).
#[tokio::test]
async fn scanning_never_modifies_bundle_files() {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;
    let before = common::hash_tree(&svc.kb_root);
    for _ in 0..2 {
        let (status, _) = svc.json(Method::POST, "/kb/rebuild", None).await;
        assert_eq!(status, 200);
    }
    assert_eq!(common::hash_tree(&svc.kb_root), before);
}

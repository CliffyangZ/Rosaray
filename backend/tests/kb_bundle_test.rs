//! Quickstart §1 (US1) — steps 1–3, 6, 7 and 10: authoring, publishing,
//! relocating and conflict-checking portable bundles, with no paper import and
//! no execution.

mod common;

use reqwest::Method;
use serde_json::json;

fn find<'a>(entries: &'a [serde_json::Value], id: &str) -> &'a serde_json::Value {
    entries.iter().find(|e| e["id"] == id).unwrap_or_else(|| panic!("{id} not in {entries:?}"))
}

/// Step 1: a valid spec-only draft is listed as `draft` / `specification_only`.
#[tokio::test]
async fn spec_only_node_draft_is_created_saved_listed_and_reopened() {
    let svc = common::spawn().await;
    let revision = svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;

    let entries = svc.entries("?kind=algonode").await;
    let entry = find(&entries, "acme.spec-only");
    assert_eq!(entry["status"], "draft");
    assert_eq!(entry["maturity"], "specification_only");
    assert_eq!(entry["release_kind"], "draft");
    assert_eq!(entry["availability"], "available");
    assert!(entry.get("path").is_none(), "paths must not be exposed: {entry}");

    // Reopen: same revision, same files.
    let (status, draft) = svc.json(Method::GET, "/kb/algonode/acme.spec-only/draft", None).await;
    assert_eq!(status, 200, "{draft}");
    assert_eq!(draft["revision"], revision);
    assert!(draft["files"]["contract.yaml"].as_str().unwrap().contains("edge-sharpness"));
    assert_eq!(draft["header"]["id"], "acme.spec-only");
    assert!(draft["findings"].as_array().unwrap().iter().all(|f| f["severity"] != "error"), "{draft}");
}

/// FR-034: saving an invalid draft is allowed; the findings are advisory.
#[tokio::test]
async fn saving_an_invalid_draft_is_allowed_and_reports_findings() {
    let svc = common::spawn().await;
    let (_, created) = svc.json(Method::POST, "/kb/algonode/drafts", Some(json!({ "id": "acme.rough" }))).await;
    let bad_contract = "schema: quantify-kb/1\ninputs:\n  - { port_id: x, artifact_kind: image2d }\noutputs: []\n";
    let (status, saved) = svc
        .json(
            Method::PUT,
            "/kb/algonode/drafts/acme.rough",
            Some(json!({ "base_revision": created["revision"], "files": { "contract.yaml": bad_contract } })),
        )
        .await;
    assert_eq!(status, 200, "{saved}");
    let codes: Vec<_> = saved["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"port_unit_undeclared"), "{codes:?}");
    for f in saved["findings"].as_array().unwrap() {
        assert!(!f["explanation"].as_str().unwrap().is_empty());
        assert!(!f["action"].as_str().unwrap().is_empty());
    }
    let entries = svc.entries("?kind=algonode").await;
    assert_eq!(find(&entries, "acme.rough")["status"], "draft");
}

#[tokio::test]
async fn invalid_ids_and_duplicate_drafts_are_rejected() {
    let svc = common::spawn().await;
    let (status, body) = svc.json(Method::POST, "/kb/algonode/drafts", Some(json!({ "id": "NotAnId" }))).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "bundle_invalid");
    assert_eq!(body["error"]["details"]["findings"][0]["code"], "id_invalid");

    let (status, _) = svc.json(Method::POST, "/kb/algonode/drafts", Some(json!({ "id": "acme.one" }))).await;
    assert_eq!(status, 201);
    let (status, body) = svc.json(Method::POST, "/kb/algonode/drafts", Some(json!({ "id": "acme.one" }))).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "identity_conflict");
}

/// Step 2: a pipe draft referencing published nodes lists a dependency summary.
#[tokio::test]
async fn pipe_draft_lists_a_dependency_summary() {
    let svc = common::spawn().await;
    assert_eq!(svc.install_seeds(), 6);
    svc.create_draft_from_fixture("algopipe", "acme.demo-pipe", "draft-pipe").await;

    let entries = svc.entries("?kind=algopipe").await;
    let pipe = find(&entries, "acme.demo-pipe");
    assert_eq!(pipe["status"], "draft");
    assert_eq!(pipe["dependency_summary"], json!({ "total": 2, "unresolved": 0 }));
}

/// Step 3: knowledge publication is immutable, hash-locked and stays spec-only.
#[tokio::test]
async fn publishing_a_spec_only_node_is_immutable_and_stays_specification_only() {
    let svc = common::spawn().await;
    let revision = svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;

    let (status, published) = svc
        .json(
            Method::POST,
            "/kb/algonode/drafts/acme.spec-only/publish",
            Some(json!({ "release": "knowledge", "base_revision": revision, "version_description": "first" })),
        )
        .await;
    assert_eq!(status, 200, "{published}");
    assert_eq!(published["id"], "acme.spec-only");
    assert_eq!(published["version"], "0.1.0");
    assert_eq!(published["release_kind"], "knowledge");
    assert!(published["content_id"].as_str().unwrap().starts_with("b3:"));

    // Published bundle on disk carries a valid lock; nothing else was touched.
    let dir = svc.kb_root.join("nodes/acme.spec-only/0.1.0");
    assert!(dir.join("bundle.lock").is_file());
    let published_hash = common::hash_tree(&dir);

    let entries = svc.entries("?kind=algonode&status=published").await;
    let entry = find(&entries, "acme.spec-only");
    assert_eq!(entry["status"], "published");
    assert_eq!(entry["maturity"], "specification_only");
    assert_eq!(entry["release_kind"], "knowledge");
    assert_eq!(entry["content_id"], published["content_id"]);

    // The draft continued as the next version.
    let (_, draft) = svc.json(Method::GET, "/kb/algonode/acme.spec-only/draft", None).await;
    assert_eq!(draft["header"]["version"], "0.1.1");
    assert_eq!(draft["revision"], published["next_draft_revision"]);

    // With the draft discarded, writing to the id targets an immutable version.
    let (status, _) = svc.json(Method::DELETE, "/kb/algonode/drafts/acme.spec-only", None).await;
    assert_eq!(status, 204);
    let (status, body) = svc
        .json(
            Method::PUT,
            "/kb/algonode/drafts/acme.spec-only",
            Some(json!({ "base_revision": "x", "files": { "contract.yaml": "changed" } })),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "published_immutable");
    assert_eq!(common::hash_tree(&dir), published_hash, "published bundle unchanged");

    // The service refuses to open a locked folder for writing at all.
    let err = rosaray_service::kb::bundle::write::write_bundle_atomic(&dir, &[("x".into(), b"y".to_vec())]).unwrap_err();
    assert!(matches!(err, rosaray_service::kb::bundle::write::WriteError::PublishedImmutable(_)));
}

/// Publishing reports *every* blocking finding and writes nothing.
#[tokio::test]
async fn publication_lists_all_blocking_findings_and_writes_nothing() {
    let svc = common::spawn().await;
    let (_, created) = svc.json(Method::POST, "/kb/algonode/drafts", Some(json!({ "id": "acme.blank" }))).await;
    let (status, body) = svc
        .json(
            Method::POST,
            "/kb/algonode/drafts/acme.blank/publish",
            Some(json!({ "release": "knowledge", "base_revision": created["revision"] })),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "not_publishable");
    let codes: Vec<_> = body["error"]["details"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes.iter().filter(|c| **c == "publication_field_missing").count(), 2, "{codes:?}");
    assert!(!svc.kb_root.join("nodes/acme.blank").exists(), "nothing written");
}

/// Step 6: relocating a bundle folder does not break references; identity
/// comes from frontmatter, so only a warning is raised.
#[tokio::test]
async fn relocated_bundle_still_resolves_with_only_a_warning() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let content_id = svc.content_id_of_published("rosaray.threshold", "1.0.0");

    let old = svc.kb_root.join("nodes/rosaray.threshold/1.0.0");
    let new = svc.kb_root.join("nodes/archive/2026/thresholds");
    std::fs::create_dir_all(new.parent().unwrap()).unwrap();
    std::fs::rename(&old, &new).unwrap();
    let (status, _) = svc.json(Method::POST, "/kb/refresh", None).await;
    assert_eq!(status, 200);

    let entries = svc.entries("?kind=algonode&status=published").await;
    let entry = find(&entries, "rosaray.threshold");
    assert_eq!(entry["content_id"], content_id);
    assert_eq!(entry["availability"], "available");

    let (_, findings) = svc.json(Method::GET, "/kb/findings?id=rosaray.threshold", None).await;
    let f = findings["bundles"][0]["findings"].as_array().unwrap();
    assert!(f.iter().any(|f| f["code"] == "path_identity_mismatch" && f["severity"] == "warning"), "{f:?}");
    assert!(f.iter().all(|f| f["severity"] != "error"), "{f:?}");
}

/// Step 7: an external edit after the draft was opened → `draft_conflict`,
/// neither version changed.
#[tokio::test]
async fn stale_base_revision_yields_draft_conflict_and_writes_nothing() {
    let svc = common::spawn().await;
    let base = svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;

    let contract = svc.kb_root.join("drafts/nodes/acme.spec-only/contract.yaml");
    let external = format!("{}# edited externally\n", std::fs::read_to_string(&contract).unwrap());
    std::fs::write(&contract, &external).unwrap();

    let mine = "schema: quantify-kb/1\ninputs: []\noutputs: []\n";
    let (status, body) = svc
        .json(
            Method::PUT,
            "/kb/algonode/drafts/acme.spec-only",
            Some(json!({ "base_revision": base, "files": { "contract.yaml": mine } })),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "draft_conflict");
    let details = &body["error"]["details"];
    assert_eq!(details["your_base"], base);
    assert_ne!(details["on_disk_revision"], base);
    assert_eq!(details["changed_files"], json!(["contract.yaml"]));
    assert_eq!(std::fs::read_to_string(&contract).unwrap(), external, "external edit preserved");

    // Re-reading yields the new revision, which then saves cleanly.
    let (_, draft) = svc.json(Method::GET, "/kb/algonode/acme.spec-only/draft", None).await;
    let (status, _) = svc
        .json(
            Method::PUT,
            "/kb/algonode/drafts/acme.spec-only",
            Some(json!({ "base_revision": draft["revision"], "files": { "contract.yaml": mine } })),
        )
        .await;
    assert_eq!(status, 200);
}

/// Step 10: a non-dental contract is browsable and demands no dental fields.
#[tokio::test]
async fn non_dental_contract_is_a_first_class_citizen() {
    let svc = common::spawn().await;
    let revision = svc.create_draft_from_fixture("algonode", "acme.smooth-signal", "timeseries-node").await;
    let entries = svc.entries("?kind=algonode&domain=time-series").await;
    let entry = find(&entries, "acme.smooth-signal");
    assert_eq!(entry["domain"], "time-series");
    assert_eq!(entry["finding_count"], 0, "no dental-specific findings: {entry}");

    let by_kind = svc.entries("?data_kind=timeseries").await;
    assert_eq!(by_kind.len(), 1);
    let (status, body) = svc
        .json(
            Method::POST,
            "/kb/algonode/drafts/acme.smooth-signal/publish",
            Some(json!({ "release": "knowledge", "base_revision": revision })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
}

/// Free-text search runs over name/summary/purpose/intended use, safely.
#[tokio::test]
async fn catalog_search_filters_and_paginates() {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;

    let hits = svc.entries("?q=gaussian").await;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["id"], "rosaray.gaussian-blur");
    // Hyphens and other punctuation never break the query.
    assert_eq!(svc.entries("?q=gaussian-blur").await.len(), 1);
    assert!(svc.entries("?q=%22%28%29%2A").await.is_empty());
    assert_eq!(svc.entries("?maturity=implemented").await.len(), 6);
    assert_eq!(svc.entries("?maturity=specification_only").await.len(), 1);
    assert_eq!(svc.entries("?release=draft").await.len(), 1);

    let (_, page1) = svc.json(Method::GET, "/kb/entries?limit=3", None).await;
    assert_eq!(page1["entries"].as_array().unwrap().len(), 3);
    let after = page1["next"].as_str().unwrap();
    let (_, page2) = svc.json(Method::GET, &format!("/kb/entries?limit=10&after={after}"), None).await;
    assert_eq!(page2["entries"].as_array().unwrap().len(), 4);
    assert!(page2["next"].is_null());
}

/// Seeding the store must be idempotent and never touch existing bundles.
#[tokio::test]
async fn seed_install_is_idempotent_through_the_service() {
    let svc = common::spawn().await;
    assert_eq!(svc.install_seeds(), 6);
    let before = common::hash_tree(&svc.kb_root.join("nodes"));
    assert_eq!(svc.install_seeds(), 0);
    assert_eq!(common::hash_tree(&svc.kb_root.join("nodes")), before);
}

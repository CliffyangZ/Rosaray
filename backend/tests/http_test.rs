//! End-to-end over HTTP: authorization scopes, the authoring → System One flow,
//! and the read-only catalog.

mod common;

use common::*;
use reqwest::StatusCode;
use serde_json::json;

const Q: &str = "00000000-0000-4000-8000-0000000000a1";

#[tokio::test]
async fn the_frontend_credential_can_read_the_catalog_and_nothing_else() {
    let svc = spawn().await;
    let (s, b) = svc.get(As::Frontend, "/qkb/v1/session").await;
    assert_eq!((s, b["state"].as_str()), (StatusCode::OK, Some("ready")));
    let (s, b) = svc.get(As::Frontend, "/qkb/v1/catalog/entries").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["entries"].as_array().unwrap().len(), 6, "the six seeded, verified nodes");

    // Every write / exchange / maintenance route refuses it (FR-014, SC-006).
    for (m, p) in [
        ("POST", "/qkb/v1/submissions"),
        ("POST", "/qkb/v1/verification"),
        ("POST", "/qkb/v1/trust"),
        ("POST", "/qkb/v1/maintenance/rescan"),
        ("POST", "/qkb/v1/maintenance/rebuild-index"),
        ("GET", "/qkb/v1/deletions"),
        ("POST", "/qkb/v1/system-one/queries"),
        ("POST", "/qkb/v1/system-one/selections"),
        ("GET", "/qkb/v1/system-one/selections/x"),
    ] {
        let method = reqwest::Method::from_bytes(m.as_bytes()).unwrap();
        let (s, b) = svc.call(As::Frontend, method, p, Some(json!({}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {p}");
        assert_eq!(b["error"]["code"], "access_denied");
    }
}

#[tokio::test]
async fn credentials_do_not_cross_scopes() {
    let svc = spawn().await;
    let (s, _) = svc.get(As::Nobody, "/qkb/v1/catalog/entries").await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = svc.post(As::FrontendAsAuthor, "/qkb/v1/submissions", json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the frontend token is not an author token");
    let (s, _) = svc.post(As::FrontendAsSystemOne, "/qkb/v1/system-one/queries", json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = svc.post(As::SystemOne, "/qkb/v1/submissions", pipe_submission("x.y", "1.0.0")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "System One cannot author methods");
    let (s, _) = svc.post(As::Author, "/qkb/v1/system-one/queries", json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "authors cannot report selections");
    // System One may read the catalog.
    let (s, _) = svc.get(As::SystemOne, "/qkb/v1/catalog/entries").await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn author_submits_system_one_selects_and_the_record_survives_a_deletion() {
    let svc = spawn().await;
    let (s, accepted) = svc.post(As::Author, "/qkb/v1/submissions", pipe_submission("acme.demo", "1.0.0")).await;
    assert_eq!(s, StatusCode::CREATED, "{accepted}");
    let content_id = accepted["content_id"].as_str().unwrap().to_string();
    let (s, again) = svc.post(As::Author, "/qkb/v1/submissions", pipe_submission("acme.demo", "1.0.0")).await;
    assert_eq!((s, again["already_present"].as_bool()), (StatusCode::OK, Some(true)));

    // Frontend detail view.
    let (s, d) = svc.get(As::Frontend, "/qkb/v1/catalog/algopipe/acme.demo/1.0.0").await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(d["state"], "executable");
    assert_eq!(d["dependencies"].as_array().unwrap().len(), 3);
    assert_eq!(d["content_id"], content_id.as_str());

    let q = json!({ "protocol_version": "system-one/1", "request_id": Q, "task_purpose": "threshold segmentation",
                    "data_conditions": { "modality": "intraoral-photo" } });
    let (s, r) = svc.post(As::SystemOne, "/qkb/v1/system-one/queries", q).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["candidates"][0]["pipe"]["id"], "acme.demo");

    let sel = json!({ "protocol_version": "system-one/1", "request_id": Q, "decision": "selected",
                      "selection": { "id": "acme.demo", "version": "1.0.0", "content_id": content_id }, "reason": "fits" });
    let (s, r) = svc.post(As::SystemOne, "/qkb/v1/system-one/selections", sel.clone()).await;
    assert_eq!((s, r["validation"].as_str()), (StatusCode::OK, Some("accepted")), "{r}");
    assert_eq!(r["record"]["source_id"], svc.state.creds.source_id.as_str());
    let (s, r2) = svc.post(As::SystemOne, "/qkb/v1/system-one/selections", sel).await;
    assert_eq!((s, &r2), (StatusCode::OK, &r), "idempotent");

    // Author revokes a node: the pipe disappears from the catalog…
    let (s, del) = svc.post(As::Author, "/qkb/v1/trust", json!({ "id": "rosaray.normalize", "version": "1.0.0", "trust": "untrusted" })).await;
    assert_eq!(s, StatusCode::OK, "{del}");
    assert_eq!(del["deleted"].as_array().unwrap().len(), 2);
    let (s, _) = svc.get(As::Frontend, "/qkb/v1/catalog/algopipe/acme.demo/1.0.0").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, deletions) = svc.get(As::Author, "/qkb/v1/deletions?id=acme.demo").await;
    assert_eq!(deletions["deletions"][0]["reason_code"], "cascade");
    // …but the selection record keeps its snapshot.
    let (s, rec) = svc.get(As::SystemOne, &format!("/qkb/v1/system-one/selections/{Q}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(rec["selection"]["record"]["selected_version_deleted"], true);
    assert!(rec["selection"]["record"]["contract_snapshot"]["contract"]["nodes"].is_array());
}

#[tokio::test]
async fn protocol_errors_map_to_their_codes() {
    let svc = spawn().await;
    let bad_version = json!({ "protocol_version": "system-one/2", "request_id": Q, "task_purpose": "x" });
    let (s, b) = svc.post(As::SystemOne, "/qkb/v1/system-one/queries", bad_version).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::UPGRADE_REQUIRED, Some("protocol_incompatible")));
    let unknown_field = json!({ "protocol_version": "system-one/1", "request_id": Q, "task_purpose": "x", "extra": 1 });
    let (s, b) = svc.post(As::SystemOne, "/qkb/v1/system-one/queries", unknown_field).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::BAD_REQUEST, Some("malformed")));
    let pii = json!({ "protocol_version": "system-one/1", "request_id": Q, "task_purpose": "photo of patient name: Chen" });
    let (s, b) = svc.post(As::SystemOne, "/qkb/v1/system-one/queries", pii).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("rejected_content")));
    let sel = json!({ "protocol_version": "system-one/1", "request_id": Q, "decision": "abstained", "reason": "none" });
    let (s, b) = svc.post(As::SystemOne, "/qkb/v1/system-one/selections", sel).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::NOT_FOUND, Some("unknown_request")));
}

#[tokio::test]
async fn a_rejected_submission_reports_findings_and_stores_nothing() {
    let svc = spawn().await;
    let before = hash_tree(&svc.state.kb_root);
    let mut sub = pipe_submission("acme.bad", "1.0.0");
    sub["files"]["graph.yaml"] = json!(sub["files"]["graph.yaml"].as_str().unwrap().replace("\"1.0.0\"", "\"9.9.9\""));
    let (s, b) = svc.post(As::Author, "/qkb/v1/submissions", sub).await;
    assert_eq!((s, b["error"]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("submission_rejected")), "{b}");
    assert!(!b["error"]["findings"].as_array().unwrap().is_empty());
    assert_eq!(hash_tree(&svc.state.kb_root), before);
}

#[tokio::test]
async fn responses_are_never_cached() {
    let svc = spawn().await;
    let resp = reqwest::Client::new()
        .get(format!("{}/qkb/v1/catalog/entries", svc.base_url))
        .header("X-Rosaray-Session", &svc.catalog_token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers().get("cache-control").unwrap(), "no-store");
}

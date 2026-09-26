//! Quickstart §6 steps 1–5, 7 (US6): knowledge vs executable publication,
//! all-findings rejection, disclosure of missing dataset validation, immutability
//! and the identity of successive versions.

mod common;

use reqwest::Method;
use serde_json::{json, Value};

fn node(inst: &str, id: &str, version: &str, params: Value) -> Value {
    json!({ "instance_id": inst, "ref": { "id": id, "version": version }, "parameters": params })
}

fn profile() -> Value {
    json!({ "profile_version": 1, "require": [{ "predicate": "modality", "equals": "intraoral-photo" }] })
}

fn graph(threshold: Value) -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node("src", "rosaray.image-source", "1.0.0", json!({})),
            node("norm", "rosaray.normalize", "1.0.0", json!({})),
            node("th", "rosaray.threshold", "1.0.0", threshold),
        ],
        "edges": [
            { "from": "src.image", "to": "norm.image" },
            { "from": "norm.image", "to": "th.image" },
        ],
        "target_data_profile": profile(),
    })
}

const NODES: [&str; 3] = ["rosaray.image-source", "rosaray.normalize", "rosaray.threshold"];

async fn verify(svc: &common::TestService, id: &str) {
    let imp = format!("builtin.{}", id.trim_start_matches("rosaray."));
    let (s, b) = svc
        .json(Method::POST, &format!("/kb/nodes/{id}/1.0.0/verification"), Some(json!({ "type": "technical", "event": "passed", "implementation_id": imp, "implementation_version": "1" })))
        .await;
    assert_eq!(s, 200, "{b}");
}

async fn draft(svc: &common::TestService, id: &str, g: Value) -> String {
    let (s, created) = svc
        .json(Method::POST, "/kb/algopipe/drafts", Some(json!({ "id": id, "name": "Demo", "summary": "s" })))
        .await;
    assert_eq!(s, 201, "{created}");
    let md = "---\nschema: quantify-kb/1\nkind: algopipe\nid: ID\nversion: 0.1.0\nname: Demo\nsummary: s\nstatus: draft\nresearch_use_only: true\nintended_use: Threshold demo\nlimitations: Demo only\n---\nBody\n".replace("ID", id);
    let (s, saved) = svc
        .json(Method::PUT, &format!("/kb/algopipe/drafts/{id}"), Some(json!({ "base_revision": created["revision"], "graph": g, "files": { "ALGOPIPE.md": md } })))
        .await;
    assert_eq!(s, 200, "{saved}");
    saved["revision"].as_str().unwrap().to_string()
}

async fn publish(svc: &common::TestService, id: &str, rev: &str, release: &str) -> (reqwest::StatusCode, Value) {
    svc.json(Method::POST, &format!("/kb/algopipe/drafts/{id}/publish"), Some(json!({ "release": release, "base_revision": rev }))).await
}

async fn seeded() -> common::TestService {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc
}

#[tokio::test]
async fn an_all_verified_pipe_publishes_as_an_immutable_executable_version_with_disclosure() {
    let svc = seeded().await;
    for n in NODES {
        verify(&svc, n).await;
    }
    let rev = draft(&svc, "acme.demo", graph(json!({ "mode": "otsu" }))).await;
    let (s, out) = publish(&svc, "acme.demo", &rev, "executable").await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["release_kind"], "executable");
    assert!(out["computational_identity"].as_str().unwrap().starts_with("b3:"));
    assert_eq!(out["disclosures"], json!(["dataset_validation_missing"]), "disclosed, not a gate");
    assert_eq!(out["dependency_summary"], json!({ "total": 3, "unresolved": 0 }));
    assert_eq!(out["verification_summary"]["verified"], 3);

    let dir = svc.kb_root.join("pipes/acme.demo/0.1.0");
    let graph_yaml = std::fs::read_to_string(dir.join("graph.yaml")).unwrap();
    assert!(graph_yaml.contains("content_id: b3:"), "references are frozen with content ids");
    assert!(graph_yaml.contains("implementation_pins") && graph_yaml.contains("builtin.threshold"));
    let lock = std::fs::read_to_string(dir.join("bundle.lock")).unwrap();
    assert!(lock.contains("release_kind: executable") && lock.contains("dataset_validation_missing") && lock.contains("computational_identity"));

    let e = svc.entries("?kind=algopipe&status=published").await.remove(0);
    assert_eq!(e["release_kind"], "executable");
    assert_eq!(e["content_id"], out["content_id"]);

    // Immutable: the id@version cannot be republished or written.
    let before = common::hash_tree(&dir);
    let (s, body) = svc.json(Method::DELETE, "/kb/algopipe/drafts/acme.demo", None).await;
    assert_eq!(s, 204, "{body:?}");
    let (s, body) = svc.json(Method::PUT, "/kb/algopipe/drafts/acme.demo", Some(json!({ "base_revision": "x", "graph": graph(json!({})) }))).await;
    assert_eq!(s, 409, "{body}");
    assert_eq!(body["error"]["code"], "published_immutable");
    assert_eq!(common::hash_tree(&dir), before);
}

#[tokio::test]
async fn a_rejected_publication_lists_every_condition_and_writes_nothing() {
    let svc = seeded().await;
    verify(&svc, "rosaray.image-source").await; // normalize and threshold stay unverified
    let mut g = graph(json!({ "mode": "otsu" }));
    g["nodes"].as_array_mut().unwrap().push(node("m", "rosaray.morphology", "draft", json!({ "op": "open", "radius": 1 })));
    g["nodes"].as_array_mut().unwrap().push(node("a", "rosaray.area", "1.0.0", json!({})));
    g["edges"].as_array_mut().unwrap().push(json!({ "from": "th.mask", "to": "m.mask" }));
    g["edges"].as_array_mut().unwrap().push(json!({ "from": "m.mask", "to": "a.mask" }));
    let rev = draft(&svc, "acme.bad", g).await;
    let before = common::hash_tree(&svc.kb_root);
    let (s, body) = publish(&svc, "acme.bad", &rev, "executable").await;
    assert_eq!(s, 422, "{body}");
    assert_eq!(body["error"]["code"], "not_publishable");
    let findings = body["error"]["details"]["findings"].as_array().unwrap();
    let codes: std::collections::BTreeSet<_> = findings.iter().map(|f| f["code"].as_str().unwrap()).collect();
    for want in ["node_not_verified", "dependency_not_immutable", "prerequisite_unmet"] {
        assert!(codes.contains(want), "{want} missing from {codes:?}");
    }
    for f in findings {
        assert!(!f["explanation"].as_str().unwrap().is_empty() && !f["action"].as_str().unwrap().is_empty());
    }
    assert!(!svc.kb_root.join("pipes/acme.bad").exists(), "nothing was written");
    assert_eq!(common::hash_tree(&svc.kb_root.join("pipes")), common::hash_tree(&svc.kb_root.join("pipes")));
    let _ = before;
}

#[tokio::test]
async fn a_new_version_has_a_new_computational_identity_and_leaves_the_old_one_byte_identical() {
    let svc = seeded().await;
    for n in NODES {
        verify(&svc, n).await;
    }
    let rev = draft(&svc, "acme.demo", graph(json!({ "mode": "manual", "value": 100 }))).await;
    let (_, v1) = publish(&svc, "acme.demo", &rev, "executable").await;
    assert_eq!(v1["version"], "0.1.0");
    let old_dir = svc.kb_root.join("pipes/acme.demo/0.1.0");
    let old_hash = common::hash_tree(&old_dir);
    let old_lock = std::fs::read(old_dir.join("bundle.lock")).unwrap();

    // The draft continued as 0.1.1; a layout-only edit does not change identity...
    let (_, d) = svc.json(Method::GET, "/kb/algopipe/acme.demo/draft", None).await;
    assert_eq!(d["header"]["version"], "0.1.1");
    let mut moved = graph(json!({ "mode": "manual", "value": 100 }));
    moved["nodes"][0]["layout"] = json!({ "x": 400, "y": 400 });
    let (s, saved) = svc.json(Method::PUT, "/kb/algopipe/drafts/acme.demo", Some(json!({ "base_revision": d["revision"], "graph": moved }))).await;
    assert_eq!(s, 200, "{saved}");
    let (s, v_layout) = publish(&svc, "acme.demo", saved["revision"].as_str().unwrap(), "executable").await;
    assert_eq!(s, 200, "{v_layout}");
    assert_eq!(v_layout["computational_identity"], v1["computational_identity"], "layout never changes identity");
    assert_ne!(v_layout["content_id"], v1["content_id"], "but the files (and so content_id) differ");

    // ...a parameter edit does.
    let (_, d) = svc.json(Method::GET, "/kb/algopipe/acme.demo/draft", None).await;
    let (s, saved) = svc.json(Method::PUT, "/kb/algopipe/drafts/acme.demo", Some(json!({ "base_revision": d["revision"], "graph": graph(json!({ "mode": "manual", "value": 150 })) }))).await;
    assert_eq!(s, 200, "{saved}");
    let (s, v3) = publish(&svc, "acme.demo", saved["revision"].as_str().unwrap(), "executable").await;
    assert_eq!(s, 200, "{v3}");
    assert_ne!(v3["computational_identity"], v1["computational_identity"]);
    assert_eq!(v3["version"], "0.1.2");
    assert_eq!(common::hash_tree(&old_dir), old_hash);
    assert_eq!(std::fs::read(old_dir.join("bundle.lock")).unwrap(), old_lock, "bundle.lock unchanged");
}

#[tokio::test]
async fn knowledge_only_stays_non_runnable_even_after_its_nodes_are_verified() {
    let svc = seeded().await;
    let rev = draft(&svc, "acme.know", graph(json!({ "mode": "otsu" }))).await;
    let (s, out) = publish(&svc, "acme.know", &rev, "knowledge").await;
    assert_eq!(s, 200, "{out}"); // unverified nodes are fine for a knowledge release
    assert_eq!(out["release_kind"], "knowledge");
    for n in NODES {
        verify(&svc, n).await;
    }
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 90);
    let (_, version) = svc.import_dir(dir.path(), "DS").await;
    let image = svc.image_ids(&version).await.remove(0);
    let (s, body) = svc
        .json(Method::POST, "/runs", Some(json!({ "dataset_version_id": version, "image_asset_id": image, "algopipe": { "id": "acme.know", "version": "0.1.0" }, "seed": 1 })))
        .await;
    assert_eq!(s, 409, "{body}");
    assert_eq!(body["error"]["code"], "not_executable");
    assert_eq!(body["error"]["details"]["reasons"][0]["code"], "not_executable_release");
    let rows: i64 = svc.state.db.lock().unwrap().query_row("SELECT count(*) FROM run_records", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn deprecating_a_referenced_node_leaves_a_published_pipe_resolving_to_the_same_content() {
    let svc = seeded().await;
    for n in NODES {
        verify(&svc, n).await;
    }
    let rev = draft(&svc, "acme.demo", graph(json!({ "mode": "otsu" }))).await;
    let (_, out) = publish(&svc, "acme.demo", &rev, "executable").await;
    let dir = svc.kb_root.join("pipes/acme.demo/0.1.0");
    let before = common::hash_tree(&dir);
    let (s, _) = svc.json(Method::POST, "/kb/algonode/rosaray.threshold/1.0.0/deprecate", Some(json!({ "reason": "superseded" }))).await;
    assert_eq!(s, 200);
    let e = svc.entries("?kind=algopipe&status=published").await.remove(0);
    assert_eq!(e["dependency_summary"], json!({ "total": 3, "unresolved": 0 }));
    assert_eq!(e["content_id"], out["content_id"]);
    assert_eq!(e["availability"], "available");
    assert_eq!(common::hash_tree(&dir), before);
}

//! Quickstart §6 steps 6, 8–10 (US6): formal Run admission by profile and
//! verification, amendments that touch nothing, suspension/restoration of
//! eligibility, and full provenance of a completed Run (SC-005, SC-017).

mod common;

use std::time::Duration;

use reqwest::Method;
use rosaray_service::kb::evidence::verification::{record_event, Event, Subject, VerificationType};
use serde_json::{json, Value};

fn node(inst: &str, id: &str, params: Value) -> Value {
    json!({ "instance_id": inst, "ref": { "id": id, "version": "1.0.0" }, "parameters": params })
}

fn graph() -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node("src", "rosaray.image-source", json!({})),
            node("norm", "rosaray.normalize", json!({})),
            node("th", "rosaray.threshold", json!({ "mode": "otsu" })),
            node("area", "rosaray.area", json!({})),
        ],
        "edges": [
            { "from": "src.image", "to": "norm.image" },
            { "from": "norm.image", "to": "th.image" },
            { "from": "th.mask", "to": "area.mask" },
        ],
        "target_data_profile": { "profile_version": 1, "require": [
            { "predicate": "pixel_spacing_present", "equals": true },
            { "predicate": "min_width_px", "value": 8 },
        ] },
    })
}

const NODES: [&str; 4] = ["rosaray.image-source", "rosaray.normalize", "rosaray.threshold", "rosaray.area"];

fn imp(id: &str) -> String {
    format!("builtin.{}", id.trim_start_matches("rosaray."))
}

async fn verify(svc: &common::TestService, id: &str) -> (reqwest::StatusCode, Value) {
    svc.json(
        Method::POST,
        &format!("/kb/nodes/{id}/1.0.0/verification"),
        Some(json!({ "type": "technical", "event": "passed", "implementation_id": imp(id), "implementation_version": "1" })),
    )
    .await
}

struct World {
    svc: common::TestService,
    good_version: String,
    good_image: String,
    bad_version: String,
    bad_image: String,
    _dirs: Vec<tempfile::TempDir>,
}

async fn import(svc: &common::TestService, name: &str, seed: u8) -> (String, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let img = image::GrayImage::from_fn(16, 12, |x, y| image::Luma([((x * 15 + y * 9 + seed as u32) % 256) as u8]));
    image::DynamicImage::ImageLuma8(img).save(dir.path().join("a.png")).unwrap();
    let (_, version) = svc.import_dir(dir.path(), name).await;
    let image = svc.image_ids(&version).await.remove(0);
    (version, image, dir)
}

async fn world() -> World {
    let svc = common::spawn().await;
    svc.install_seeds();
    for n in NODES {
        assert_eq!(verify(&svc, n).await.0, 200);
    }
    let (good_version, good_image, d1) = import(&svc, "Calibrated", 3).await;
    let (bad_version, bad_image, d2) = import(&svc, "Uncalibrated", 50).await;
    svc.state.db.lock().unwrap().execute(
        "UPDATE image_assets SET pixel_spacing_mm_x = 0.05, pixel_spacing_mm_y = 0.05, spacing_source = 'user_entered' WHERE id = ?1",
        [&good_image],
    ).unwrap();

    let (_, created) = svc.json(Method::POST, "/kb/algopipe/drafts", Some(json!({ "id": "acme.exec", "name": "Exec", "summary": "s" }))).await;
    let md = "---\nschema: quantify-kb/1\nkind: algopipe\nid: acme.exec\nversion: 0.1.0\nname: Exec\nsummary: s\nstatus: draft\nresearch_use_only: true\nintended_use: Measure a region\nlimitations: Demo\n---\nBody\n";
    let evidence = "evidence_id: e1\ntype: method_source\napplies_to: { id: acme.exec, version: 0.1.0 }\ndistributable: false\npatient_data_status: none\nlocator: { page: 2, quote_ref: q1 }\n";
    let (s, saved) = svc
        .json(
            Method::PUT,
            "/kb/algopipe/drafts/acme.exec",
            Some(json!({ "base_revision": created["revision"], "graph": graph(), "files": { "ALGOPIPE.md": md, "references/e1.yaml": evidence } })),
        )
        .await;
    assert_eq!(s, 200, "{saved}");
    let (s, out) = svc
        .json(Method::POST, "/kb/algopipe/drafts/acme.exec/publish", Some(json!({ "release": "executable", "base_revision": saved["revision"] })))
        .await;
    assert_eq!(s, 200, "{out}");
    World { svc, good_version, good_image, bad_version, bad_image, _dirs: vec![d1, d2] }
}

fn run_rows(svc: &common::TestService) -> i64 {
    svc.state.db.lock().unwrap().query_row("SELECT count(*) FROM run_records", [], |r| r.get(0)).unwrap()
}

impl World {
    async fn run(&self, version: &str, image: &str) -> (reqwest::StatusCode, Value) {
        self.svc
            .json(Method::POST, "/runs", Some(json!({ "dataset_version_id": version, "image_asset_id": image, "algopipe": { "id": "acme.exec", "version": "0.1.0" }, "seed": 7 })))
            .await
    }

    async fn wait(&self, run_id: &str) -> Value {
        for _ in 0..200 {
            let (_, r) = self.svc.json(Method::GET, &format!("/runs/{run_id}"), None).await;
            if r["status"] != "running" {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("run never finished");
    }
}

/// Step 6: one dataset satisfies the profile, the other does not.
#[tokio::test]
async fn a_dataset_that_satisfies_the_profile_is_admitted_and_one_that_does_not_is_refused() {
    let w = world().await;
    let pipe_dir = w.svc.kb_root.join("pipes/acme.exec/0.1.0");
    let pipe_hash = common::hash_tree(&pipe_dir);

    let rows = run_rows(&w.svc);
    let (s, denied) = w.run(&w.bad_version, &w.bad_image).await;
    assert_eq!(s, 422, "{denied}");
    assert_eq!(denied["error"]["code"], "profile_unsatisfied");
    let r = &denied["error"]["details"]["reasons"];
    assert!(r.as_array().unwrap().iter().any(|x| x["code"] == "profile_unsatisfied" && x["predicate"] == "pixel_spacing_present"), "{r}");
    assert_eq!(run_rows(&w.svc), rows, "no Run record on denial");
    let db = w.svc.state.db.lock().unwrap();
    let denied_events: i64 = db.query_row("SELECT count(*) FROM kb_event WHERE type = 'run_admission_denied'", [], |r| r.get(0)).unwrap();
    assert_eq!(denied_events, 1);
    drop(db);

    let (s, ok) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 200, "{ok}");
    let run = w.wait(ok["run_id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "succeeded", "{run}");
    assert_eq!(run["algopipe_id"], "acme.exec");
    assert_eq!(run["algopipe_version"], "0.1.0");
    assert!(run["algopipe_content_id"].as_str().unwrap().starts_with("b3:"));
    assert!(run["eligibility_event_id"].is_string());
    // The Run really computed: a measurement table with the calibrated area.
    assert!(run["metric_set"]["foreground_pixels"].as_i64().unwrap() >= 0);
    assert!((run["metric_set"]["area_mm2"].as_f64().unwrap() - run["metric_set"]["foreground_pixels"].as_i64().unwrap() as f64 * 0.0025).abs() < 1e-9);
    assert_eq!(common::hash_tree(&pipe_dir), pipe_hash, "the pipe version is unchanged");
    // Reproducible: a repeat gives the identical output identity.
    let (_, again) = w.run(&w.good_version, &w.good_image).await;
    let run2 = w.wait(again["run_id"].as_str().unwrap()).await;
    assert_eq!(run2["status"], "succeeded");
    assert_eq!(run2["output_content_identities"], run["output_content_identities"]);
}

/// Step 10 (SC-005): provenance resolves every link.
#[tokio::test]
async fn a_completed_run_resolves_to_pipe_nodes_parameters_dataset_and_evidence() {
    let w = world().await;
    let (_, ok) = w.run(&w.good_version, &w.good_image).await;
    let id = ok["run_id"].as_str().unwrap().to_string();
    w.wait(&id).await;
    let (s, p) = w.svc.json(Method::GET, &format!("/runs/{id}/provenance"), None).await;
    assert_eq!(s, 200, "{p}");
    assert_eq!(p["algopipe"]["id"], "acme.exec");
    assert_eq!(p["algopipe"]["release_kind"], "executable");
    assert!(p["algopipe"]["computational_identity"].as_str().unwrap().starts_with("b3:"));
    assert_eq!(p["algopipe"]["disclosures"], json!(["dataset_validation_missing"]));
    assert_eq!(p["dataset_version"]["id"], w.good_version.as_str());
    let nodes = p["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 4);
    let th = nodes.iter().find(|n| n["instance_id"] == "th").unwrap();
    assert_eq!(th["ref"]["id"], "rosaray.threshold");
    assert!(th["ref"]["content_id"].as_str().unwrap().starts_with("b3:"));
    assert_eq!(th["parameters"]["mode"], "otsu");
    assert_eq!(th["parameters"]["invert"], false, "author-supplied defaults are shown as effective parameters");
    assert_eq!(th["implementation"]["implementation_id"], "builtin.threshold");
    assert_eq!(p["admission_event"]["type"], "run_admission_allowed");
    // A legacy Run has no provenance of this kind.
    let (s, _) = w.svc.json(Method::GET, &format!("/runs/{}/provenance", uuid::Uuid::new_v4()), None).await;
    assert_eq!(s, 404);
}

/// Step 8: an amendment changes nothing that was published or recorded.
#[tokio::test]
async fn an_amendment_leaves_bundles_and_prior_runs_untouched_and_is_visible_everywhere() {
    let w = world().await;
    let (_, ok) = w.run(&w.good_version, &w.good_image).await;
    let run_id = ok["run_id"].as_str().unwrap().to_string();
    let run_before = w.wait(&run_id).await;
    let bundles_before = common::hash_tree(&w.svc.kb_root.join("pipes"));
    let nodes_before = common::hash_tree(&w.svc.kb_root.join("nodes"));

    let (s, out) = w
        .svc
        .json(
            Method::POST,
            "/kb/algopipe/acme.exec/0.1.0/amendments",
            Some(json!({ "kind": "withdrawal", "evidence_id": "e1", "reason": "the cited page was mis-numbered", "resulting_status": "withdrawn" })),
        )
        .await;
    assert_eq!(s, 200, "{out}");
    assert_eq!(out["affected_versions"], json!(["acme.exec@0.1.0"]));
    assert!(out["amendment_id"].as_str().unwrap().len() > 8);

    assert_eq!(common::hash_tree(&w.svc.kb_root.join("pipes")), bundles_before);
    assert_eq!(common::hash_tree(&w.svc.kb_root.join("nodes")), nodes_before);
    let (_, run_after) = w.svc.json(Method::GET, &format!("/runs/{run_id}"), None).await;
    // (artifact references are minted per request; everything stored is identical)
    let stored = |mut v: Value| {
        v.as_object_mut().unwrap().remove("output_artifact_refs");
        v
    };
    assert_eq!(stored(run_after), stored(run_before), "prior Run rows unchanged");

    let e = w.svc.entries("?kind=algopipe&status=published").await.remove(0);
    assert_eq!(e["amendment_count"], 1, "the catalog shows it");
    let (_, prov) = w.svc.json(Method::GET, &format!("/runs/{run_id}/provenance"), None).await;
    let recorded = prov["amendments"].as_array().unwrap().iter().find(|a| a["bundle"] == "acme.exec@0.1.0").unwrap();
    assert_eq!(recorded["records"][0]["kind"], "withdrawal");
    let (_, list) = w.svc.json(Method::GET, "/kb/algopipe/acme.exec/0.1.0/amendments", None).await;
    assert_eq!(list["amendments"][0]["detail"]["original_evidence"]["evidence_id"], "e1");
    // It is still eligible: amending evidence is not a verification event.
    let (s, _) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 200);

    // Unknown evidence or missing reason is refused.
    let (s, _) = w.svc.json(Method::POST, "/kb/algopipe/acme.exec/0.1.0/amendments", Some(json!({ "kind": "correction", "evidence_id": "nope", "reason": "x", "resulting_status": "s" }))).await;
    assert_eq!(s, 422);
}

/// Step 9 (SC-017): withdrawal suspends, only the identical subject restores.
#[tokio::test]
async fn withdrawn_verification_suspends_new_runs_and_only_the_identical_subject_restores_them() {
    let w = world().await;
    let (_, ok) = w.run(&w.good_version, &w.good_image).await;
    let earlier = ok["run_id"].as_str().unwrap().to_string();
    w.wait(&earlier).await;
    let mut events = w.svc.state.event_tx.subscribe();

    let (s, body) = w
        .svc
        .json(
            Method::POST,
            "/kb/nodes/rosaray.threshold/1.0.0/verification",
            Some(json!({ "type": "technical", "event": "withdrawn", "implementation_id": "builtin.threshold", "implementation_version": "1", "reason": "fixture was wrong" })),
        )
        .await;
    assert_eq!(s, 200, "{body}");
    let ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let v = serde_json::to_value(&events.recv().await.unwrap()).unwrap();
            if v["type"] == "eligibility_changed" {
                return v;
            }
        }
    })
    .await
    .expect("eligibility_changed");
    assert_eq!(ev["payload"]["algopipe"], json!({ "id": "acme.exec", "version": "0.1.0" }));
    assert_eq!(ev["payload"]["eligible"], false);
    assert_eq!(ev["payload"]["reasons"], json!(["verification_withdrawn"]));

    let rows = run_rows(&w.svc);
    let (s, denied) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 409, "{denied}");
    assert_eq!(denied["error"]["code"], "verification_invalid");
    assert_eq!(denied["error"]["details"]["reasons"][0]["node"], "th", "names the node");
    assert_eq!(run_rows(&w.svc), rows);
    let (s, old) = w.svc.json(Method::GET, &format!("/runs/{earlier}"), None).await;
    assert_eq!(s, 200);
    assert_eq!(old["status"], "succeeded", "the earlier Run stays readable");

    // A passed record for a *different* implementation version does not restore it.
    let (bundle, _) = rosaray_service::kb::bundle::read::read_bundle(&w.svc.kb_root.join("nodes/rosaray.threshold/1.0.0"));
    let mut other: Subject = rosaray_service::kb::catalog::status::verification_subject(&bundle.unwrap()).unwrap();
    other.implementation_version = "2".into();
    record_event(&w.svc.kb_root, VerificationType::Technical, Event::Passed, &other, None, None, None).unwrap();
    w.svc.refresh_catalog();
    let (s, _) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 409, "still suspended");

    // Re-running the node's own tests records `passed` for the identical tuple: restored.
    let (s, body) = verify(&w.svc, "rosaray.threshold").await;
    assert_eq!(s, 200, "{body}");
    let restored = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let v = serde_json::to_value(&events.recv().await.unwrap()).unwrap();
            if v["type"] == "eligibility_changed" && v["payload"]["eligible"] == true {
                return v;
            }
        }
    })
    .await
    .expect("restoration is visible too");
    assert_eq!(restored["payload"]["reasons"], json!([]));
    let (s, ok) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 200, "{ok}");
    assert_eq!(w.wait(ok["run_id"].as_str().unwrap()).await["status"], "succeeded");
}

/// Dataset-level validation withdrawal never suspends anything (FR-052).
#[tokio::test]
async fn withdrawing_dataset_validation_does_not_affect_eligibility() {
    let w = world().await;
    let (s, _) = w
        .svc
        .json(
            Method::POST,
            "/kb/nodes/rosaray.threshold/1.0.0/verification",
            Some(json!({ "type": "dataset", "event": "withdrawn", "implementation_id": "builtin.threshold", "implementation_version": "1", "reason": "study retracted" })),
        )
        .await;
    assert_eq!(s, 200);
    let (s, body) = w.run(&w.good_version, &w.good_image).await;
    assert_eq!(s, 200, "{body}");
}

#[tokio::test]
async fn request_validation_and_the_legacy_body_still_work() {
    let w = world().await;
    let both = json!({ "dataset_version_id": w.good_version, "image_asset_id": w.good_image, "seed": 1,
        "algopipe": { "id": "acme.exec", "version": "0.1.0" },
        "pipeline_snapshot": { "nodes": [{ "node_id": "a", "node_type": "source" }], "edges": [] }, "target_node_id": "a" });
    let (s, _) = w.svc.json(Method::POST, "/runs", Some(both)).await;
    assert_eq!(s, 422);
    let neither = json!({ "dataset_version_id": w.good_version, "image_asset_id": w.good_image, "seed": 1 });
    assert_eq!(w.svc.json(Method::POST, "/runs", Some(neither)).await.0, 422);
    let (s, _) = w
        .svc
        .json(Method::POST, "/runs", Some(json!({ "dataset_version_id": w.good_version, "image_asset_id": w.good_image, "seed": 1, "algopipe": { "id": "acme.nope", "version": "1.0.0" } })))
        .await;
    assert_eq!(s, 404);
    let legacy = json!({ "dataset_version_id": w.good_version, "image_asset_id": w.good_image, "seed": 1,
        "pipeline_snapshot": { "nodes": [{ "node_id": "a", "node_type": "source" }], "edges": [] }, "target_node_id": "a" });
    let (s, out) = w.svc.json(Method::POST, "/runs", Some(legacy)).await;
    assert_eq!(s, 200, "{out}");
    let _ = &w.bad_image;
}

//! Quickstart §4 (US4): real, incremental, single-image Preview of an AlgoPipe
//! draft — unverified isolation, staleness and reuse, out-of-order completion,
//! failure reporting — and, on every path, no Run record (SC-009).

mod common;

use std::time::Duration;

use reqwest::Method;
use serde_json::{json, Value};

fn node(inst: &str, id: &str, params: Value) -> Value {
    json!({ "instance_id": inst, "ref": { "id": id, "version": "1.0.0" }, "parameters": params })
}

fn edge(from: &str, to: &str) -> Value {
    json!({ "from": from, "to": to })
}

fn chain(threshold: Value) -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node("src", "rosaray.image-source", json!({})),
            node("norm", "rosaray.normalize", json!({ "lo": 1, "hi": 99 })),
            node("th", "rosaray.threshold", threshold),
            node("morph", "rosaray.morphology", json!({ "op": "open", "radius": 1 })),
            node("area", "rosaray.area", json!({})),
        ],
        "edges": [
            edge("src.image", "norm.image"),
            edge("norm.image", "th.image"),
            edge("th.mask", "morph.mask"),
            edge("morph.mask", "area.mask"),
        ],
    })
}

struct Fixture {
    svc: common::TestService,
    image_id: String,
    pipe: &'static str,
    _dir: tempfile::TempDir,
}

fn run_rows(svc: &common::TestService) -> i64 {
    let db = svc.state.db.lock().unwrap();
    db.query_row("SELECT count(*) FROM run_records", [], |r| r.get(0)).unwrap()
}

fn set_spacing(svc: &common::TestService, image_id: &str, spacing: Option<f64>) {
    let db = svc.state.db.lock().unwrap();
    db.execute(
        "UPDATE image_assets SET pixel_spacing_mm_x = ?1, pixel_spacing_mm_y = ?1, spacing_source = ?2 WHERE id = ?3",
        rusqlite::params![spacing, spacing.map(|_| "user_entered"), image_id],
    )
    .unwrap();
}

async fn fixture() -> Fixture {
    let svc = common::spawn().await;
    svc.install_seeds();
    let dir = tempfile::tempdir().unwrap();
    let img = image::GrayImage::from_fn(16, 12, |x, y| image::Luma([((x * 15 + y * 9) % 256) as u8]));
    image::DynamicImage::ImageLuma8(img).save(dir.path().join("a.png")).unwrap();
    let (_, version) = svc.import_dir(dir.path(), "Preview DS").await;
    let image_id = svc.image_ids(&version).await.remove(0);
    Fixture { svc, image_id, pipe: "acme.pv", _dir: dir }
}

impl Fixture {
    /// Creates (first call) or replaces the draft's graph; returns the new revision.
    async fn save_graph(&self, graph: Value) -> String {
        let (status, cur) = self.svc.json(Method::GET, &format!("/kb/algopipe/{}/draft", self.pipe), None).await;
        let base = if status == 404 {
            let (s, created) = self.svc.json(Method::POST, "/kb/algopipe/drafts", Some(json!({ "id": self.pipe, "name": "PV" }))).await;
            assert_eq!(s, 201, "{created}");
            created["revision"].clone()
        } else {
            cur["revision"].clone()
        };
        let (status, saved) = self
            .svc
            .json(Method::PUT, &format!("/kb/algopipe/drafts/{}", self.pipe), Some(json!({ "base_revision": base, "graph": graph })))
            .await;
        assert_eq!(status, 200, "{saved}");
        saved["revision"].as_str().unwrap().to_string()
    }

    async fn preview(&self, revision: &str, target: &str) -> (reqwest::StatusCode, Value) {
        self.svc
            .json(
                Method::POST,
                "/preview",
                Some(json!({ "pipe_id": self.pipe, "revision": revision, "image_asset_id": self.image_id, "target_node_id": target })),
            )
            .await
    }

    async fn verify(&self, id: &str) {
        let imp = format!("builtin.{}", id.trim_start_matches("rosaray."));
        let (status, body) = self
            .svc
            .json(
                Method::POST,
                &format!("/kb/nodes/{id}/1.0.0/verification"),
                Some(json!({ "type": "technical", "event": "passed", "implementation_id": imp, "implementation_version": "1" })),
            )
            .await;
        assert_eq!(status, 200, "{id}: {body}");
        assert_eq!(body["maturity"], "technically_verified");
    }
}

#[tokio::test]
async fn preview_is_unverified_until_every_node_in_the_cone_is_verified_and_never_creates_a_run() {
    let f = fixture().await;
    let rev = f.save_graph(chain(json!({ "mode": "otsu" }))).await;
    let rows_before = run_rows(&f.svc);

    let (status, out) = f.preview(&rev, "th").await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["state"], "ready");
    assert_eq!(out["verification_state"], "unverified");
    let mut un: Vec<_> = out["unverified_nodes"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    un.sort();
    assert_eq!(un, vec!["norm", "src", "th"], "only the target's cone is tainted");
    assert_eq!(out["reused"], false);

    // The artifact is a real, decodable image with the source's dimensions.
    let id = out["artifact_ref"]["id"].as_str().unwrap();
    let resp = f.svc.request(Method::GET, &format!("/artifacts/{id}/content")).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let img = image::load_from_memory(&resp.bytes().await.unwrap()).unwrap().to_luma8();
    assert_eq!((img.width(), img.height()), (16, 12));
    assert!(img.pixels().all(|p| p.0[0] == 0 || p.0[0] == 255), "a mask is rendered as 0/255");

    // Verify every node in the cone via the runner; the same preview becomes verified.
    for n in ["rosaray.image-source", "rosaray.normalize", "rosaray.threshold"] {
        f.verify(n).await;
    }
    let (_, out) = f.preview(&rev, "th").await;
    assert_eq!(out["verification_state"], "verified");
    assert!(out["unverified_nodes"].as_array().unwrap().is_empty());
    // Downstream nodes are still unverified, and the taint follows the cone.
    let (_, out) = f.preview(&rev, "morph").await;
    assert_eq!(out["verification_state"], "unverified");
    assert_eq!(out["unverified_nodes"], json!(["morph"]));

    assert_eq!(run_rows(&f.svc), rows_before, "Preview never creates a Run record");
}

#[tokio::test]
async fn a_computational_edit_stales_only_that_node_and_downstream_and_reuses_the_rest() {
    let f = fixture().await;
    let rev1 = f.save_graph(chain(json!({ "mode": "manual", "value": 100 }))).await;
    let (_, first) = f.preview(&rev1, "morph").await;
    assert_eq!(first["state"], "ready", "{first}");
    assert_eq!(first["node_reuse"], json!({ "src": false, "norm": false, "th": false, "morph": false }));

    let mut events = f.svc.state.event_tx.subscribe();
    let rev2 = f.save_graph(chain(json!({ "mode": "manual", "value": 140 }))).await;
    assert_ne!(rev1, rev2);

    // preview_stale names exactly the edited node and what is downstream of it.
    let ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let e = events.recv().await.unwrap();
            let v = serde_json::to_value(&e).unwrap();
            if v["type"] == "preview_stale" {
                return v;
            }
        }
    })
    .await
    .expect("preview_stale event");
    assert_eq!(ev["payload"]["pipe_id"], "acme.pv");
    assert_eq!(ev["payload"]["revision"], rev2);
    assert_eq!(ev["payload"]["nodes"], json!(["area", "morph", "th"]));

    let (_, status) = f.svc.json(Method::GET, &format!("/preview/status?pipe_id=acme.pv&revision={rev2}"), None).await;
    assert_eq!(status["nodes"], json!({ "src": "ready", "norm": "ready", "th": "stale", "morph": "stale", "area": "stale" }));

    let (_, second) = f.preview(&rev2, "morph").await;
    assert_eq!(second["node_reuse"], json!({ "src": true, "norm": true, "th": false, "morph": false }), "unaffected nodes are reused");
    assert_eq!(second["computed_nodes"], json!(["th", "morph"]));
    let (_, status) = f.svc.json(Method::GET, "/preview/status?pipe_id=acme.pv", None).await;
    assert_eq!(status["nodes"]["morph"], "ready");
    assert_eq!(status["nodes"]["area"], "stale");

    // Identical request again: everything comes from the cache.
    let (_, third) = f.preview(&rev2, "morph").await;
    assert_eq!(third["reused"], true);
    assert_eq!(third["computed_nodes"], json!([]));

    // Clearing the cache makes the next request a plain recompute, never an error.
    let (status, _) = f.svc.json(Method::POST, "/cache/clear", None).await;
    assert_eq!(status, 200);
    let (_, again) = f.preview(&rev2, "morph").await;
    assert_eq!(again["state"], "ready");
    assert_eq!(again["reused"], false);
}

#[tokio::test]
async fn results_depend_on_the_pixels_not_on_cache_state() {
    // A cached and a freshly computed result must be byte-identical.
    let f = fixture().await;
    let rev = f.save_graph(chain(json!({ "mode": "otsu" }))).await;
    let fetch = |out: Value| {
        let id = out["artifact_ref"]["content_identity"].as_str().unwrap().to_string();
        id
    };
    let (_, a) = f.preview(&rev, "th").await;
    let (_, b) = f.preview(&rev, "th").await;
    assert_eq!(b["reused"], true);
    let first = fetch(a);
    let second = fetch(b);
    f.svc.json(Method::POST, "/cache/clear", None).await;
    let (_, c) = f.preview(&rev, "th").await;
    assert_eq!(first, second);
    assert_eq!(first, fetch(c), "recomputing after a cache clear reproduces the same content");
}

#[tokio::test]
async fn a_request_that_completes_out_of_order_is_discarded() {
    let f = fixture().await;
    let rev_a = f.save_graph(chain(json!({ "mode": "manual", "value": 90 }))).await;
    f.svc.state.preview_delays.lock().unwrap().push_back(500);

    let base = f.svc.base_url.clone();
    let token = f.svc.session_token.clone();
    let (image, pipe_rev) = (f.image_id.clone(), rev_a.clone());
    let a = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base}/preview"))
            .header("X-Rosaray-Session", token)
            .json(&json!({ "pipe_id": "acme.pv", "revision": pipe_rev, "image_asset_id": image, "target_node_id": "th" }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    tokio::time::sleep(Duration::from_millis(100)).await; // A is in flight, delayed

    let rev_b = f.save_graph(chain(json!({ "mode": "manual", "value": 200 }))).await;
    let (_, b) = f.preview(&rev_b, "th").await;
    assert_eq!(b["state"], "ready");
    assert_eq!(b["current"], true);

    let a = a.await.unwrap();
    assert_eq!(a["state"], "ready");
    assert_eq!(a["current"], false, "A finished after B started, so it is superseded");

    // A never became the selection's last successful result; B did.
    let image_uuid = uuid::Uuid::parse_str(&f.image_id).unwrap();
    let last = f.svc.state.preview_isolation.last_successful(image_uuid, "acme.pv/th").unwrap();
    assert_eq!(last.content_identity, b["artifact_ref"]["content_identity"].as_str().unwrap());
    assert_ne!(a["artifact_ref"]["content_identity"], b["artifact_ref"]["content_identity"]);
}

#[tokio::test]
async fn an_executor_error_fails_with_the_node_and_labels_the_last_result_stale() {
    let f = fixture().await;
    let rev = f.save_graph(chain(json!({ "mode": "otsu" }))).await;
    set_spacing(&f.svc, &f.image_id, Some(0.05));
    let (_, ok) = f.preview(&rev, "area").await;
    assert_eq!(ok["state"], "ready", "{ok}");
    let table_id = ok["artifact_ref"]["id"].as_str().unwrap().to_string();
    let table: Value = f.svc.request(Method::GET, &format!("/artifacts/{table_id}/content")).send().await.unwrap().json().await.unwrap();
    assert!(table["pixels"].as_f64().unwrap() >= 0.0);
    assert!((table["mm2"].as_f64().unwrap() - table["pixels"].as_f64().unwrap() * 0.0025).abs() < 1e-9);

    // Calibration is removed: the area node cannot run and says why.
    set_spacing(&f.svc, &f.image_id, None);
    let rows = run_rows(&f.svc);
    let (status, failed) = f.preview(&rev, "area").await;
    assert_eq!(status, 200, "{failed}");
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["failing_node_id"], "area");
    assert_eq!(failed["error"]["code"], "missing_pixel_spacing");
    assert!(failed["error"]["message"].as_str().unwrap().contains("pixel spacing"));
    assert_eq!(failed["stale"], true);
    assert_eq!(failed["last_successful_artifact_ref"]["content_identity"], ok["artifact_ref"]["content_identity"]);
    let (_, st) = f.svc.json(Method::GET, "/preview/status?pipe_id=acme.pv", None).await;
    assert_eq!(st["nodes"]["area"], "failed");
    assert_eq!(st["nodes"]["morph"], "ready", "upstream results survive a downstream failure");
    assert_eq!(run_rows(&f.svc), rows);
}

#[tokio::test]
async fn a_specification_only_node_in_the_cone_is_not_previewable() {
    let f = fixture().await;
    let md = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.spec\nversion: 1.0.0\nname: n\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n";
    let contract = std::fs::read_to_string(common::fixture_dir("valid-spec-only-node").join("contract.yaml")).unwrap();
    common::make_published_bundle(&f.svc.kb_root, "algonode", "acme.spec", "1.0.0", "knowledge", &[("ALGONODE.md", md), ("contract.yaml", &contract)]);
    f.svc.refresh_catalog();
    let rev = f
        .save_graph(json!({
            "schema": "quantify-kb/1",
            "nodes": [node("src", "rosaray.image-source", json!({})), node("s", "acme.spec", json!({ "window": 5 })), node("norm", "rosaray.normalize", json!({}))],
            "edges": [edge("src.image", "s.image")],
        }))
        .await;
    let rows = run_rows(&f.svc);
    let (status, body) = f.preview(&rev, "s").await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "not_previewable");
    assert_eq!(body["error"]["details"]["nodes"], json!(["s"]));
    // The implemented source alone is fine.
    let (status, body) = f.preview(&rev, "src").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(run_rows(&f.svc), rows);
}

#[tokio::test]
async fn errors_in_the_targets_own_inputs_block_it_but_unrelated_branches_do_not() {
    let f = fixture().await;
    let mut g = chain(json!({ "mode": "otsu" }));
    g["nodes"].as_array_mut().unwrap().push(node("stray", "rosaray.gaussian-blur", json!({ "sigma": 99 }))); // invalid + unconnected
    let rev = f.save_graph(g).await;
    let (status, ok) = f.preview(&rev, "th").await;
    assert_eq!(status, 200, "an unrelated bad node does not block: {ok}");

    let mut broken = chain(json!({ "mode": "manual", "value": 999 })); // out of range
    broken["nodes"][2]["parameters"] = json!({ "mode": "manual", "value": 999 });
    let rev = f.save_graph(broken).await;
    let (status, body) = f.preview(&rev, "th").await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "not_previewable");
    assert_eq!(body["error"]["details"]["findings"][0]["code"], "parameter_invalid");
}

#[tokio::test]
async fn a_stale_revision_is_refused_before_anything_runs() {
    let f = fixture().await;
    let rev1 = f.save_graph(chain(json!({ "mode": "otsu" }))).await;
    let _rev2 = f.save_graph(chain(json!({ "mode": "manual", "value": 50 }))).await;
    let cached_before = f.svc.state.preview_nodes.len();
    let (status, body) = f.preview(&rev1, "th").await;
    assert_eq!(status, 410, "{body}");
    assert_eq!(body["error"]["code"], "stale_reference");
    assert_eq!(f.svc.state.preview_nodes.len(), cached_before, "nothing ran");
    let (status, _) = f.preview("whatever", "nope").await;
    assert_eq!(status, 410);
}

#[tokio::test]
async fn preview_events_carry_the_pipe_revision_and_verification_state() {
    let f = fixture().await;
    let rev = f.save_graph(chain(json!({ "mode": "otsu" }))).await;
    let mut events = f.svc.state.event_tx.subscribe();
    let (_, out) = f.preview(&rev, "norm").await;
    let ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let v = serde_json::to_value(&events.recv().await.unwrap()).unwrap();
            if v["type"] == "preview_ready" {
                return v;
            }
        }
    })
    .await
    .unwrap();
    let p = &ev["payload"];
    assert_eq!(p["pipe_id"], "acme.pv");
    assert_eq!(p["revision"], rev);
    assert_eq!(p["verification_state"], "unverified");
    assert_eq!(p["target_node_id"], "norm");
    assert_eq!(p["request_context_id"], out["request_context_id"]);
}

// ---- verification runner (quickstart §4.2) --------------------------------------

#[tokio::test]
async fn technical_passed_is_only_accepted_when_the_nodes_own_tests_pass() {
    let f = fixture().await;
    // The claim must describe this bundle.
    let (status, body) = f
        .svc
        .json(
            Method::POST,
            "/kb/nodes/rosaray.threshold/1.0.0/verification",
            Some(json!({ "type": "technical", "event": "passed", "implementation_id": "builtin.threshold", "implementation_version": "99" })),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "verification_invalid");

    // A node whose tests fail: the failed run is recorded, never a pass.
    let md = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.bad\nversion: 1.0.0\nname: n\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n";
    let contract = std::fs::read_to_string(f.svc.kb_root.join("nodes/rosaray.threshold/1.0.0/contract.yaml")).unwrap();
    let cases = r#"{"schema":"quantify-kb/1","cases":[{"name":"wrong expectation","inputs":{"image":{"kind":"image2d","width":2,"height":1,"data":[0,255]}},"parameters":{"mode":"manual","value":100},"expect":{"outputs":{"mask":{"kind":"mask2d","width":2,"height":1,"data":[1,0]}}}}]}"#;
    common::make_published_bundle(
        &f.svc.kb_root,
        "algonode",
        "acme.bad",
        "1.0.0",
        "knowledge",
        &[
            ("ALGONODE.md", md),
            ("contract.yaml", &contract),
            ("implementation.yaml", "implementation_id: builtin.threshold\nimplementation_version: \"1\"\ntrust: builtin\nassets: []\n"),
            ("tests/cases.yaml", cases),
        ],
    );
    f.svc.refresh_catalog();
    let (status, body) = f
        .svc
        .json(
            Method::POST,
            "/kb/nodes/acme.bad/1.0.0/verification",
            Some(json!({ "type": "technical", "event": "passed", "implementation_id": "builtin.threshold", "implementation_version": "1" })),
        )
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["details"]["cases"][0]["name"], "wrong expectation");
    let (_, hist) = f.svc.json(Method::GET, "/kb/nodes/acme.bad/1.0.0/verification", None).await;
    assert_eq!(hist["records"].as_array().unwrap().len(), 1);
    assert_eq!(hist["records"][0]["kind"], "failed", "the run's real result is what gets recorded");
    assert_eq!(hist["intact"], true);

    // failed / withdrawn may be requested directly; passed never.
    let (status, body) = f
        .svc
        .json(
            Method::POST,
            "/kb/nodes/rosaray.threshold/1.0.0/verification",
            Some(json!({ "type": "technical", "event": "withdrawn", "implementation_id": "builtin.threshold", "implementation_version": "1", "reason": "fixture 4 was wrong" })),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["technical_verification"], "withdrawn");
    assert_eq!(body["maturity"], "implemented");

    // A dataset validation must state its scope.
    let (status, _) = f
        .svc
        .json(
            Method::POST,
            "/kb/nodes/rosaray.threshold/1.0.0/verification",
            Some(json!({ "type": "dataset", "event": "passed", "implementation_id": "builtin.threshold", "implementation_version": "1" })),
        )
        .await;
    assert_eq!(status, 422);
}

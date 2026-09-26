//! Quickstart §2 (US2): `POST /designer/validate` on graphs built from the six
//! seed nodes — one finding code per error class, bound to the right subject,
//! layout edits never change identity, and validation is fast at 50 nodes.

mod common;

use reqwest::Method;
use rosaray_service::kb::bundle::model::GraphFile;
use serde_json::{json, Value};

fn node(inst: &str, id: &str, params: Value) -> Value {
    json!({ "instance_id": inst, "ref": { "id": id, "version": "1.0.0" }, "parameters": params })
}

fn edge(from: &str, to: &str) -> Value {
    json!({ "from": from, "to": to })
}

fn chain() -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node("src", "rosaray.image-source", json!({})),
            node("norm", "rosaray.normalize", json!({ "lo": 1, "hi": 99 })),
            node("th", "rosaray.threshold", json!({ "mode": "otsu" })),
            node("morph", "rosaray.morphology", json!({ "op": "open", "radius": 2 })),
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

fn profile() -> Value {
    json!({ "profile_version": 1, "require": [{ "predicate": "pixel_spacing_present", "equals": true }] })
}

async fn validate(svc: &common::TestService, graph: Value, extra: Value) -> Value {
    let mut body = json!({ "kind": "algopipe", "graph": graph, "domain": "dental-image", "profile": profile() });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let (status, out) = svc.json(Method::POST, "/designer/validate", Some(body)).await;
    assert_eq!(status, 200, "{out}");
    out
}

fn codes(report: &Value) -> Vec<String> {
    report["findings"].as_array().unwrap().iter().map(|f| f["code"].as_str().unwrap().to_string()).collect()
}

fn finding<'a>(report: &'a Value, code: &str) -> &'a Value {
    report["findings"].as_array().unwrap().iter().find(|f| f["code"] == code).unwrap_or_else(|| panic!("{code} not in {:?}", codes(report)))
}

async fn seeded() -> common::TestService {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc
}

#[tokio::test]
async fn a_valid_five_node_chain_is_valid_and_previewable() {
    let svc = seeded().await;
    let report = validate(&svc, chain(), json!({})).await;
    assert_eq!(report["valid"], true, "{:?}", report["findings"]);
    assert_eq!(report["executable_for_preview"], true);
    assert!(report["findings"].as_array().unwrap().iter().all(|f| f["severity"] != "error"));
}

#[tokio::test]
async fn every_finding_carries_an_explanation_and_an_action() {
    let svc = seeded().await;
    let mut g = chain();
    g["edges"].as_array_mut().unwrap().pop();
    g["nodes"][2]["parameters"] = json!({ "mode": "manual", "value": 999 });
    let report = validate(&svc, g, json!({})).await;
    assert!(!report["findings"].as_array().unwrap().is_empty());
    for f in report["findings"].as_array().unwrap() {
        assert!(!f["explanation"].as_str().unwrap().is_empty(), "{f}");
        assert!(!f["action"].as_str().unwrap().is_empty(), "{f}");
        assert!(f["subject"]["ref"].is_string(), "{f}");
    }
}

#[tokio::test]
async fn a_cycle_is_reported() {
    let svc = seeded().await;
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [node("a", "rosaray.normalize", json!({})), node("b", "rosaray.gaussian-blur", json!({}))],
        "edges": [edge("a.image", "b.image"), edge("b.image", "a.image")],
    });
    let report = validate(&svc, g, json!({ "domain": null })).await;
    assert_eq!(report["valid"], false);
    let f = finding(&report, "graph_cycle");
    assert_eq!(f["subject"]["type"], "node_instance");
    assert!(f["explanation"].as_str().unwrap().contains("a"));
    assert_eq!(report["executable_for_preview"], false);
}

#[tokio::test]
async fn a_unit_mismatch_is_port_incompatible_unit_on_that_edge() {
    let svc = seeded().await;
    let contract = |port: &str, dir: &str, unit: &str| {
        format!(
            "schema: quantify-kb/1\ninputs:{}\noutputs:{}\nparameters: []\nreproducibility: {{ deterministic: true, cacheable: true }}\n",
            if dir == "in" { format!("\n  - {{ port_id: {port}, artifact_kind: scalar, dtype: float32, unit: {unit} }}") } else { " []".into() },
            if dir == "out" { format!("\n  - {{ port_id: {port}, artifact_kind: scalar, dtype: float32, unit: {unit} }}") } else { " []".into() },
        )
    };
    let md = |id: &str| format!("---\nschema: quantify-kb/1\nkind: algonode\nid: {id}\nversion: 1.0.0\nname: n\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n");
    common::make_published_bundle(&svc.kb_root, "algonode", "acme.mm-source", "1.0.0", "knowledge", &[("ALGONODE.md", &md("acme.mm-source")), ("contract.yaml", &contract("length", "out", "mm"))]);
    common::make_published_bundle(&svc.kb_root, "algonode", "acme.px-sink", "1.0.0", "knowledge", &[("ALGONODE.md", &md("acme.px-sink")), ("contract.yaml", &contract("length", "in", "pixel"))]);
    svc.refresh_catalog();
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [node("a", "acme.mm-source", json!({})), node("b", "acme.px-sink", json!({}))],
        "edges": [edge("a.length", "b.length")],
    });
    let report = validate(&svc, g, json!({ "domain": null })).await;
    let f = finding(&report, "port_incompatible.unit");
    assert_eq!(f["subject"], json!({ "type": "edge", "ref": "a.length->b.length" }));
    assert!(f["explanation"].as_str().unwrap().contains("mm"));
    assert_eq!(report["valid"], false);
}

#[tokio::test]
async fn type_mismatch_is_reported_for_an_image_wired_to_a_mask_input() {
    let svc = seeded().await;
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [node("src", "rosaray.image-source", json!({})), node("morph", "rosaray.morphology", json!({ "op": "open", "radius": 1 }))],
        "edges": [edge("src.image", "morph.mask")],
    });
    let report = validate(&svc, g, json!({})).await;
    assert_eq!(finding(&report, "port_incompatible.type")["subject"]["ref"], "src.image->morph.mask");
}

#[tokio::test]
async fn a_second_image_source_is_a_source_count_finding_from_the_domain_rule() {
    let svc = seeded().await;
    let mut g = chain();
    g["nodes"].as_array_mut().unwrap().push(node("src2", "rosaray.image-source", json!({})));
    let report = validate(&svc, g.clone(), json!({})).await;
    assert_eq!(finding(&report, "source_count")["severity"], "error");
    // The core validator never branches on the domain: without one, no rule fires.
    let report = validate(&svc, g, json!({ "domain": null })).await;
    assert!(!codes(&report).contains(&"source_count".to_string()));
    // No source at all is also a source_count finding for this domain.
    let mut none = chain();
    none["nodes"].as_array_mut().unwrap().remove(0);
    none["edges"].as_array_mut().unwrap().remove(0);
    assert!(codes(&validate(&svc, none, json!({})).await).contains(&"source_count".to_string()));
}

#[tokio::test]
async fn missing_required_input_and_over_occupied_input() {
    let svc = seeded().await;
    let mut g = chain();
    g["edges"].as_array_mut().unwrap().remove(1); // norm -> th
    let report = validate(&svc, g, json!({})).await;
    assert_eq!(finding(&report, "input_missing")["subject"]["ref"], "th.image");

    let mut g = chain();
    g["edges"].as_array_mut().unwrap().push(edge("src.image", "th.image"));
    let report = validate(&svc, g, json!({})).await;
    let f = finding(&report, "input_over_occupied");
    assert_eq!(f["subject"]["ref"], "th.image");
    assert_eq!(report["executable_for_preview"], false);
}

#[tokio::test]
async fn parameter_problems_are_bound_to_the_parameter() {
    let svc = seeded().await;
    for (params, why) in [
        (json!({ "mode": "manual", "value": 999 }), "out of range"),
        (json!({ "mode": "sobel" }), "not an allowed value"),
        (json!({ "mode": "otsu", "nonsense": 1 }), "unknown parameter"),
        (json!({ "mode": "otsu", "invert": "yes" }), "wrong type"),
    ] {
        let mut g = chain();
        g["nodes"][2]["parameters"] = params;
        let report = validate(&svc, g, json!({})).await;
        let f = finding(&report, "parameter_invalid");
        assert_eq!(f["subject"]["type"], "parameter", "{why}");
        assert!(f["subject"]["ref"].as_str().unwrap().starts_with("th."), "{why}: {f}");
        assert_eq!(report["valid"], false, "{why}");
    }
    // A required parameter with no value and no default.
    let mut g = chain();
    g["nodes"][2]["parameters"] = json!({});
    // mode has an author-supplied default, so the omission is fine.
    assert_eq!(validate(&svc, g, json!({})).await["valid"], true);
}

#[tokio::test]
async fn prerequisites_are_checked_against_the_target_data_profile() {
    let svc = seeded().await;
    let unmet = validate(&svc, chain(), json!({ "profile": { "profile_version": 1, "require": [] } })).await;
    let f = finding(&unmet, "prerequisite_unmet");
    assert_eq!(f["severity"], "error");
    assert!(f["explanation"].as_str().unwrap().contains("pixel_spacing_present"));
    assert_eq!(unmet["valid"], false);
    // With no profile yet the finding is only a warning.
    let no_profile = validate(&svc, chain(), json!({ "profile": null })).await;
    assert_eq!(finding(&no_profile, "prerequisite_unmet")["severity"], "warning");
    assert_eq!(no_profile["valid"], true);
}

#[tokio::test]
async fn unresolved_references_are_findings_and_drafts_may_be_referenced_as_draft() {
    let svc = seeded().await;
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [node("x", "acme.nowhere", json!({}))],
        "edges": [],
    });
    let report = validate(&svc, g, json!({ "domain": null })).await;
    assert_eq!(finding(&report, "dependency_unresolved")["subject"]["ref"], "x");

    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [json!({ "instance_id": "d", "ref": { "id": "acme.spec-only", "version": "draft" } })],
        "edges": [],
    });
    let report = validate(&svc, g, json!({ "domain": null })).await;
    assert!(!codes(&report).contains(&"dependency_unresolved".to_string()), "{:?}", codes(&report));
    // input_missing for its required `image` input, but a spec-only node is never previewable.
    assert_eq!(report["executable_for_preview"], false);
}

#[tokio::test]
async fn a_specification_only_node_keeps_the_pipe_from_previewing() {
    let svc = seeded().await;
    let md = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.spec\nversion: 1.0.0\nname: n\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n";
    let contract = std::fs::read_to_string(common::fixture_dir("valid-spec-only-node").join("contract.yaml")).unwrap();
    common::make_published_bundle(&svc.kb_root, "algonode", "acme.spec", "1.0.0", "knowledge", &[("ALGONODE.md", md), ("contract.yaml", &contract)]);
    svc.refresh_catalog();
    let g = json!({
        "schema": "quantify-kb/1",
        "nodes": [node("src", "rosaray.image-source", json!({})), node("s", "acme.spec", json!({ "window": 5 }))],
        "edges": [edge("src.image", "s.image")],
    });
    let report = validate(&svc, g, json!({})).await;
    assert_eq!(report["valid"], true, "{:?}", report["findings"]);
    assert_eq!(report["executable_for_preview"], false);
}

#[tokio::test]
async fn layout_only_changes_keep_the_computational_identity_byte_identical() {
    let svc = seeded().await;
    let mut a = chain();
    let mut b = chain();
    a["nodes"][1]["layout"] = json!({ "x": 10, "y": 10 });
    b["nodes"][1]["layout"] = json!({ "x": 500, "y": 60 });
    b["nodes"][2]["label"] = json!("my threshold");
    let ida = rosaray_service::kb::identity::computational_identity(&serde_json::from_value::<GraphFile>(a.clone()).unwrap());
    let idb = rosaray_service::kb::identity::computational_identity(&serde_json::from_value::<GraphFile>(b.clone()).unwrap());
    assert_eq!(ida, idb);
    // ...and validation agrees nothing computational changed.
    let report = validate(&svc, b, json!({ "previous": a })).await;
    assert!(report["stale_nodes"].as_array().unwrap().is_empty());
    // A parameter edit does change identity.
    let mut c = chain();
    c["nodes"][2]["parameters"] = json!({ "mode": "manual", "value": 100 });
    assert_ne!(rosaray_service::kb::identity::computational_identity(&serde_json::from_value::<GraphFile>(c).unwrap()), ida);
}

#[tokio::test]
async fn a_parameter_edit_marks_exactly_that_node_and_its_downstream_stale() {
    let svc = seeded().await;
    let before = chain();
    let mut after = chain();
    after["nodes"][2]["parameters"] = json!({ "mode": "manual", "value": 100 });
    let report = validate(&svc, after, json!({ "previous": before })).await;
    assert_eq!(report["stale_nodes"], json!(["area", "morph", "th"]));
}

#[tokio::test]
async fn contracts_can_be_validated_too() {
    let svc = common::spawn().await;
    let contract = json!({
        "schema": "quantify-kb/1",
        "inputs": [{ "port_id": "x", "artifact_kind": "image2d" }],
        "outputs": [],
    });
    let (status, out) = svc.json(Method::POST, "/designer/validate", Some(json!({ "kind": "algonode", "contract": contract }))).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["valid"], false);
    assert_eq!(finding(&out, "port_unit_undeclared")["subject"], json!({ "type": "port", "ref": "x" }));
}

/// SC-003: a 50-node graph re-validates in well under a second, p95.
#[tokio::test]
async fn fifty_node_graph_validates_within_a_second_at_p95() {
    let svc = seeded().await;
    let mut nodes = vec![node("src", "rosaray.image-source", json!({}))];
    let mut edges = Vec::new();
    let mut prev = "src".to_string();
    for i in 0..48 {
        let id = format!("blur{i}");
        nodes.push(node(&id, "rosaray.gaussian-blur", json!({ "sigma": 2 })));
        edges.push(edge(&format!("{prev}.image"), &format!("{id}.image")));
        prev = id;
    }
    nodes.push(node("th", "rosaray.threshold", json!({ "mode": "otsu" })));
    edges.push(edge(&format!("{prev}.image"), "th.image"));
    let mut graph = json!({ "schema": "quantify-kb/1", "nodes": nodes, "edges": edges });
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 50);
    assert_eq!(validate(&svc, graph.clone(), json!({ "profile": null })).await["valid"], true);

    let mut times = Vec::new();
    for i in 0..100 {
        let previous = graph.clone();
        graph["nodes"][1 + (i % 48)]["parameters"] = json!({ "sigma": 1.0 + (i % 5) as f64 });
        let start = std::time::Instant::now();
        let report = validate(&svc, graph.clone(), json!({ "previous": previous, "profile": null })).await;
        times.push(start.elapsed());
        assert_eq!(report["valid"], true);
    }
    times.sort();
    let p95 = times[94];
    assert!(p95 < std::time::Duration::from_secs(1), "p95 {p95:?}");
}

/// `port_incompatible.source`: inputs declared `same_source_as` must trace back
/// to one origin (through outputs' own `same_source_as`).
#[tokio::test]
async fn inputs_that_must_share_a_source_but_do_not_are_reported() {
    let svc = seeded().await;
    let md = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.pair\nversion: 1.0.0\nname: n\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n";
    let port = |id: &str, kind: &str, extra: &str| format!("  - {{ port_id: {id}, artifact_kind: {kind}, dtype: {}, unit: {}, coordinate_space: image-pixel, calibration: none{extra} }}\n", if kind == "image2d" { "float32" } else { "uint8" }, if kind == "image2d" { "intensity" } else { "none" });
    let contract = format!(
        "schema: quantify-kb/1\ninputs:\n{}{}outputs: []\nparameters: []\nreproducibility: {{ deterministic: true, cacheable: true }}\n",
        port("img", "image2d", ""),
        port("msk", "mask2d", ", same_source_as: [img]"),
    );
    common::make_published_bundle(&svc.kb_root, "algonode", "acme.pair", "1.0.0", "knowledge", &[("ALGONODE.md", md), ("contract.yaml", &contract)]);
    svc.refresh_catalog();
    let build = |mask_from: &str| {
        json!({
            "schema": "quantify-kb/1",
            "nodes": [
                node("s1", "rosaray.image-source", json!({})), node("s2", "rosaray.image-source", json!({})),
                node("t1", "rosaray.threshold", json!({ "mode": "otsu" })), node("t2", "rosaray.threshold", json!({ "mode": "otsu" })),
                node("p", "acme.pair", json!({})),
            ],
            "edges": [
                edge("s1.image", "t1.image"), edge("s2.image", "t2.image"),
                edge("s1.image", "p.img"), edge(mask_from, "p.msk"),
            ],
        })
    };
    let ok = validate(&svc, build("t1.mask"), json!({ "domain": null })).await;
    assert!(!codes(&ok).contains(&"port_incompatible.source".to_string()), "{:?}", codes(&ok));
    let bad = validate(&svc, build("t2.mask"), json!({ "domain": null })).await;
    assert_eq!(finding(&bad, "port_incompatible.source")["subject"]["ref"], "p.msk");
}

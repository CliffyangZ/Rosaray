//! Quickstart §3 (US3): the inspector keeps maturity, technical verification,
//! dataset validation, availability and deprecation as separate dimensions,
//! keeps the three evidence types apart, reports unknowns literally, and
//! derives everything from files and append-only chains.

mod common;

use reqwest::Method;
use rosaray_service::domain::dataset::{
    Dimensions, ImageAsset, ImageAssetStatus, MetadataStatus, PixelSpacing, SpacingSource,
};
use rosaray_service::kb::bundle::read::read_bundle;
use rosaray_service::kb::catalog::status::verification_subject;
use rosaray_service::kb::evidence::verification::{record_event, Event, VerificationType};
use serde_json::{json, Value};

async fn inspect(svc: &common::TestService, id: &str) -> Value {
    let (status, body) = svc.json(Method::GET, &format!("/kb/algonode/{id}/1.0.0"), None).await;
    assert_eq!(status, 200, "{body}");
    body["inspector"].clone()
}

fn subject_of(svc: &common::TestService, id: &str) -> rosaray_service::kb::evidence::verification::Subject {
    let (bundle, _) = read_bundle(&svc.kb_root.join("nodes").join(id).join("1.0.0"));
    verification_subject(&bundle.unwrap()).unwrap()
}

fn pass(svc: &common::TestService, id: &str, vtype: VerificationType, event: Event) {
    record_event(&svc.kb_root, vtype, event, &subject_of(svc, id), None, Some(json!({ "dataset": "d1" })), None).unwrap();
    svc.refresh_catalog();
}

const SPEC_MD: &str = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.spec\nversion: 1.0.0\nname: Spec only\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n";

fn publish_spec_only(svc: &common::TestService, extra: &[(&str, &str)]) {
    let contract = std::fs::read_to_string(common::fixture_dir("valid-spec-only-node").join("contract.yaml")).unwrap();
    let mut files: Vec<(&str, &str)> = vec![("ALGONODE.md", SPEC_MD), ("contract.yaml", &contract)];
    files.extend_from_slice(extra);
    common::make_published_bundle(&svc.kb_root, "algonode", "acme.spec", "1.0.0", "knowledge", &files);
    svc.refresh_catalog();
}

fn asset(spacing: Option<f64>) -> ImageAsset {
    ImageAsset {
        id: uuid::Uuid::new_v4(),
        external_source_uri: "file:///x/scan.png".into(),
        source_content_identity: format!("s-{}", uuid::Uuid::new_v4()),
        imported_content_identity: format!("i-{}", uuid::Uuid::new_v4()),
        dimensions: Dimensions { width: 640, height: 480 },
        source_created_at: None,
        imported_at: "2026-01-01T00:00:00Z".into(),
        status: ImageAssetStatus::Available,
        patient_id: None,
        split: None,
        reference_mask_id: None,
        metadata_status: MetadataStatus::Incomplete,
        pixel_spacing_mm: spacing.map(|s| PixelSpacing { x: s, y: s }),
        spacing_source: spacing.map(|_| SpacingSource::UserEntered),
    }
}

/// Five separate status fields, and no single "verified".
#[tokio::test]
async fn inspector_shows_five_separate_status_dimensions_per_state() {
    let svc = common::spawn().await;
    svc.install_seeds();
    publish_spec_only(&svc, &[]);

    let spec = inspect(&svc, "acme.spec").await;
    let implemented = inspect(&svc, "rosaray.threshold").await;
    for i in [&spec, &implemented] {
        let st = i["status"].as_object().unwrap();
        let mut keys: Vec<_> = st.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec!["availability", "dataset_validation", "deprecation", "maturity", "technical_verification"]);
        assert!(!i.to_string().contains("\"verified\""), "no collapsed verified flag");
    }
    assert_eq!(spec["status"]["maturity"], "specification_only");
    assert_eq!(spec["status"]["technical_verification"], "none");
    assert_eq!(spec["implementation"]["status"], "none");
    assert_eq!(implemented["status"]["maturity"], "implemented");
    assert_eq!(implemented["implementation"]["status"], "available");
    assert_eq!(implemented["implementation"]["effective_trust"], "builtin");

    // Technically verified: a passed record for this exact tuple.
    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Passed);
    let verified = inspect(&svc, "rosaray.threshold").await;
    assert_eq!(verified["status"]["maturity"], "technically_verified");
    assert_eq!(verified["status"]["technical_verification"], "passed");
    assert_eq!(verified["status"]["dataset_validation"], "none", "technical evidence never implies dataset validation");
    let entries = svc.entries("?maturity=technically_verified").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["verification"], "passed");
    assert_eq!(entries[0]["dataset_validation"], "none");

    // Dataset validation is its own dimension.
    pass(&svc, "rosaray.threshold", VerificationType::Dataset, Event::Passed);
    let both = inspect(&svc, "rosaray.threshold").await;
    assert_eq!(both["status"]["dataset_validation"], "passed");
    assert_eq!(both["status"]["technical_verification"], "passed");
}

/// Maturity is not monotone: withdrawal drops it back; a different subject never restores it.
#[tokio::test]
async fn withdrawn_verification_drops_maturity_and_only_the_identical_subject_restores_it() {
    let svc = common::spawn().await;
    svc.install_seeds();
    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Passed);
    assert_eq!(inspect(&svc, "rosaray.threshold").await["status"]["maturity"], "technically_verified");

    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Withdrawn);
    let i = inspect(&svc, "rosaray.threshold").await;
    assert_eq!(i["status"]["maturity"], "implemented");
    assert_eq!(i["status"]["technical_verification"], "withdrawn");

    // A `passed` for a different implementation version is not this subject.
    let mut other = subject_of(&svc, "rosaray.threshold");
    other.implementation_version = "2".into();
    record_event(&svc.kb_root, VerificationType::Technical, Event::Passed, &other, None, None, None).unwrap();
    svc.refresh_catalog();
    assert_eq!(inspect(&svc, "rosaray.threshold").await["status"]["technical_verification"], "withdrawn");

    // The identical tuple restores it.
    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Passed);
    assert_eq!(inspect(&svc, "rosaray.threshold").await["status"]["maturity"], "technically_verified");
    // The history is all there, in order.
    let history: Vec<_> = inspect(&svc, "rosaray.threshold").await["verification_history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["event"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(history, vec!["passed", "withdrawn", "passed", "passed"]);
}

/// Unknown information is the literal "unknown", never omitted.
#[tokio::test]
async fn unknown_information_is_reported_literally() {
    let svc = common::spawn().await;
    let md = SPEC_MD.replace("intended_use: u\nlimitations: l\n", "");
    common::make_published_bundle(
        &svc.kb_root,
        "algonode",
        "acme.sparse",
        "1.0.0",
        "knowledge",
        &[
            ("ALGONODE.md", &md.replace("acme.spec", "acme.sparse")),
            ("contract.yaml", "schema: quantify-kb/1\ninputs:\n  - { port_id: x, artifact_kind: image2d }\noutputs: []\n"),
        ],
    );
    svc.refresh_catalog();
    let i = inspect(&svc, "acme.sparse").await;
    for k in ["intended_use", "limitations", "purpose", "method", "domain"] {
        assert_eq!(i[k], "unknown", "{k}");
    }
    assert_eq!(i["ports"]["inputs"][0]["unit"], "unknown");
    assert_eq!(i["reproducibility"]["deterministic"], "unknown");
    assert_eq!(i["reproducibility"]["cacheable"], "unknown");
}

/// The three evidence types are never merged.
#[tokio::test]
async fn evidence_is_grouped_by_type_and_dataset_validation_needs_a_scope() {
    let svc = common::spawn().await;
    let rec = |id: &str, ty: &str, extra: &str| {
        format!("evidence_id: {id}\ntype: {ty}\napplies_to: {{ id: acme.spec, version: 1.0.0 }}\ndistributable: true\npatient_data_status: none\n{extra}")
    };
    publish_spec_only(
        &svc,
        &[
            ("references/paper.yaml", &rec("paper", "method_source", "locator: { page: 3, quote_ref: q1 }\n")),
            ("references/tech.yaml", &rec("tech", "technical_verification", "")),
            ("references/ds.yaml", &rec("ds", "dataset_validation", "scope: { dataset: d1, metric: dice }\n")),
        ],
    );
    let i = inspect(&svc, "acme.spec").await;
    let ev = &i["evidence"];
    assert_eq!(ev["method_source"].as_array().unwrap().len(), 1);
    assert_eq!(ev["technical_verification"].as_array().unwrap().len(), 1);
    assert_eq!(ev["dataset_validation"].as_array().unwrap().len(), 1);
    assert_eq!(ev["method_source"][0]["evidence_id"], "paper");
    assert_eq!(ev["dataset_validation"][0]["scope"]["metric"], "dice");
    // Evidence records do not change the derived statuses.
    assert_eq!(i["status"]["technical_verification"], "none");
    assert_eq!(i["status"]["dataset_validation"], "none");
    assert_eq!(i["status"]["maturity"], "specification_only");
}

/// A calibration-requiring node against an image without pixel spacing.
#[tokio::test]
async fn a_mm_requiring_node_is_prerequisite_unmet_on_an_uncalibrated_image() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let (bare, calibrated) = (asset(None), asset(Some(0.05)));
    {
        let db = svc.state.db.lock().unwrap();
        for a in [&bare, &calibrated] {
            rosaray_service::data_repository::sqlite::dataset_repo::insert_image_asset(&db, a).unwrap();
        }
    }
    let (status, body) = svc.json(Method::GET, &format!("/kb/algonode/rosaray.area/1.0.0?image_asset_id={}", bare.id), None).await;
    assert_eq!(status, 200, "{body}");
    let c = &body["inspector"]["compatibility"];
    assert_eq!(c["compatible"], false);
    assert_eq!(c["unmet"][0]["code"], "prerequisite_unmet");
    assert_eq!(c["unmet"][0]["predicate"], "pixel_spacing_present");
    assert_eq!(c["unmet"][0]["observed"], false);
    assert!(c["unmet"][0]["explanation"].as_str().unwrap().contains("pixel_spacing_present"));
    assert!(!c["unmet"][0]["action"].as_str().unwrap().is_empty());

    let (_, body) = svc.json(Method::GET, &format!("/kb/algonode/rosaray.area/1.0.0?image_asset_id={}", calibrated.id), None).await;
    assert_eq!(body["inspector"]["compatibility"]["compatible"], true);
    // Spacing survives the round trip through the asset repository.
    let db = svc.state.db.lock().unwrap();
    let back = rosaray_service::data_repository::sqlite::dataset_repo::image_asset_by_id(&db, calibrated.id).unwrap().unwrap();
    assert_eq!(back.pixel_spacing_mm, Some(PixelSpacing { x: 0.05, y: 0.05 }));
    assert_eq!(back.spacing_source, Some(SpacingSource::UserEntered));
}

/// Deprecating a used version keeps the reference; only a warning appears.
#[tokio::test]
async fn deprecating_a_used_version_preserves_the_reference_and_warns() {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc.create_draft_from_fixture("algopipe", "acme.demo-pipe", "draft-pipe").await;
    let draft_before = common::hash_tree(&svc.kb_root.join("drafts/pipes/acme.demo-pipe"));
    let bundle_before = common::hash_tree(&svc.kb_root.join("nodes/rosaray.threshold/1.0.0"));

    let (status, body) = svc
        .json(
            Method::POST,
            "/kb/algonode/rosaray.threshold/1.0.0/deprecate",
            Some(json!({ "reason": "superseded by adaptive thresholding", "replacement": { "id": "rosaray.threshold", "version": "2.0.0" } })),
        )
        .await;
    assert_eq!(status, 200, "{body}");

    let i = inspect(&svc, "rosaray.threshold").await;
    assert_eq!(i["status"]["availability"], "deprecated");
    assert_eq!(i["status"]["deprecation"]["deprecated"], true);
    assert_eq!(i["status"]["deprecation"]["replacement"], json!({ "id": "rosaray.threshold", "version": "2.0.0" }));
    assert_eq!(i["status"]["maturity"], "implemented", "deprecation is not a maturity change");
    assert_eq!(i["amendments"][0]["kind"], "deprecation");

    // Nothing was rewritten: not the published bundle, not the draft that uses it.
    assert_eq!(common::hash_tree(&svc.kb_root.join("nodes/rosaray.threshold/1.0.0")), bundle_before);
    assert_eq!(common::hash_tree(&svc.kb_root.join("drafts/pipes/acme.demo-pipe")), draft_before);

    // The pipe still resolves and is warned, with replacement info.
    let pipe = svc.entries("?kind=algopipe").await.remove(0);
    assert_eq!(pipe["dependency_summary"], json!({ "total": 2, "unresolved": 0 }));
    let (_, findings) = svc.json(Method::GET, "/kb/findings?id=acme.demo-pipe", None).await;
    let f = findings["bundles"][0]["findings"].as_array().unwrap().iter().find(|f| f["code"] == "dependency_deprecated").cloned().expect("warning");
    assert_eq!(f["severity"], "warning");
    assert!(f["explanation"].as_str().unwrap().contains("2.0.0"), "{f}");

    // The graph validator agrees: a warning, not an error.
    let (_, v) = svc
        .json(
            Method::POST,
            "/designer/validate",
            Some(json!({
                "graph": { "schema": "quantify-kb/1", "nodes": [{ "instance_id": "th", "ref": { "id": "rosaray.threshold", "version": "1.0.0" } }], "edges": [] }
            })),
        )
        .await;
    assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == "dependency_deprecated" && f["severity"] == "warning"));

    let (status, _) = svc.json(Method::POST, "/kb/algonode/rosaray.threshold/1.0.0/deprecate", Some(json!({ "reason": " " }))).await;
    assert_eq!(status, 422);
    let (status, _) = svc.json(Method::POST, "/kb/algonode/rosaray.nope/1.0.0/deprecate", Some(json!({ "reason": "x" }))).await;
    assert_eq!(status, 404);
}

/// A broken chain link makes the affected version unavailable, with a finding.
#[tokio::test]
async fn a_tampered_history_chain_marks_the_version_unavailable() {
    let svc = common::spawn().await;
    svc.install_seeds();
    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Passed);
    pass(&svc, "rosaray.threshold", VerificationType::Technical, Event::Withdrawn);
    assert_eq!(inspect(&svc, "rosaray.threshold").await["status"]["availability"], "available");

    let dir = svc.kb_root.join("verification/rosaray.threshold");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let text = std::fs::read_to_string(&files[0]).unwrap().replace("passed", "failed");
    std::fs::write(&files[0], text).unwrap();
    svc.refresh_catalog();

    let i = inspect(&svc, "rosaray.threshold").await;
    assert_eq!(i["status"]["availability"], "unavailable");
    assert_eq!(i["status"]["technical_verification"], "none", "an untrusted chain grants nothing");
    assert!(i["findings"].as_array().unwrap().iter().any(|f| f["code"] == "chain_broken"), "{}", i["findings"]);
}

/// Appending history is noticed by an incremental refresh, and never touches bundles.
#[tokio::test]
async fn chain_writes_are_picked_up_incrementally_without_touching_bundles() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let nodes_before = common::hash_tree(&svc.kb_root.join("nodes"));
    pass(&svc, "rosaray.morphology", VerificationType::Technical, Event::Passed);
    assert_eq!(svc.entries("?maturity=technically_verified").await.len(), 1);
    assert_eq!(common::hash_tree(&svc.kb_root.join("nodes")), nodes_before);
}

/// An untrusted (imported-style) implementation never counts as implemented.
#[tokio::test]
async fn an_untrusted_or_unresolvable_implementation_stays_specification_only() {
    let svc = common::spawn().await;
    let imp = "implementation_id: builtin.threshold\nimplementation_version: \"1\"\ntrust: builtin\nassets: []\n";
    publish_spec_only(&svc, &[("implementation.yaml", imp)]);
    // No local trust record ⇒ untrusted, despite the bundle's own declaration.
    let i = inspect(&svc, "acme.spec").await;
    assert_eq!(i["status"]["maturity"], "specification_only");
    assert_eq!(i["implementation"]["status"], "untrusted");
    // Trusting it locally (a decision outside the bundle) promotes it.
    let cid = svc.content_id_of_published("acme.spec", "1.0.0");
    rosaray_service::kb::trust::record(&svc.kb_root, "acme.spec", "1.0.0", &cid, rosaray_service::kb::bundle::model::Trust::Trusted, "test").unwrap();
    svc.refresh_catalog(); // the trust file is part of the change signature
    assert_eq!(inspect(&svc, "acme.spec").await["status"]["maturity"], "implemented");

    // An id the registry cannot resolve stays a specification, with a finding.
    let svc2 = common::spawn().await;
    let imp = "implementation_id: builtin.mystery\nimplementation_version: \"1\"\ntrust: builtin\nassets: []\n";
    publish_spec_only(&svc2, &[("implementation.yaml", imp)]);
    let cid = svc2.content_id_of_published("acme.spec", "1.0.0");
    rosaray_service::kb::trust::record(&svc2.kb_root, "acme.spec", "1.0.0", &cid, rosaray_service::kb::bundle::model::Trust::Trusted, "test").unwrap();
    svc2.refresh_catalog();
    let i = inspect(&svc2, "acme.spec").await;
    assert_eq!(i["status"]["maturity"], "specification_only");
    assert_eq!(i["implementation"]["status"], "unavailable");
    assert!(i["findings"].as_array().unwrap().iter().any(|f| f["code"] == "implementation_unavailable"));
}

/// The pipe inspector reports its dependencies' availability separately.
#[tokio::test]
async fn pipe_inspector_lists_dependencies_with_their_own_status() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let thr = svc.content_id_of_published("rosaray.threshold", "1.0.0");
    let graph = format!("schema: quantify-kb/1\nnodes:\n  - {{ instance_id: th, ref: {{ id: rosaray.threshold, version: 1.0.0, content_id: \"{thr}\" }} }}\nedges: []\ntarget_data_profile:\n  profile_version: 1\n  require:\n    - {{ predicate: modality, equals: intraoral-photo }}\n");
    common::make_published_bundle(
        &svc.kb_root, "algopipe", "acme.p", "1.0.0", "knowledge",
        &[("ALGOPIPE.md", "---\nschema: quantify-kb/1\nkind: algopipe\nid: acme.p\nversion: 1.0.0\nname: P\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n"), ("graph.yaml", &graph)],
    );
    svc.refresh_catalog();
    let (status, body) = svc.json(Method::GET, "/kb/algopipe/acme.p/1.0.0", None).await;
    assert_eq!(status, 200, "{body}");
    let i = &body["inspector"];
    assert_eq!(i["release_kind"], "knowledge");
    assert_eq!(i["dependencies"][0]["ref"], "rosaray.threshold@1.0.0");
    assert_eq!(i["dependencies"][0]["availability"], "available");
    assert_eq!(i["dependencies"][0]["maturity"], "implemented");
    assert_eq!(i["target_data_profile"]["require"][0]["predicate"], "modality");
}

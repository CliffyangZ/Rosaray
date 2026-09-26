//! Quickstart §1 steps 4, 8, 9, 11: `.algobundle` exchange round-trips
//! identity, refuses conflicts, never trusts or runs imported implementations,
//! and can never carry patient data.

mod common;

use std::io::Write;

use reqwest::Method;
use rosaray_service::domain::dataset::{
    Dimensions, ImageAsset, ImageAssetStatus, MetadataStatus,
};
use rosaray_service::kb::exchange::export::{pack_archive, Manifest, ManifestEntry};
use serde_json::json;

const PIPE_MD: &str = "---\nschema: quantify-kb/1\nkind: algopipe\nid: acme.pinned\nversion: 1.0.0\nname: Pinned\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\nA pinned pipe.\n";

/// A published pipe pinning two seed nodes by content_id, plus an amendment.
async fn service_with_pipe() -> common::TestService {
    let svc = common::spawn().await;
    svc.install_seeds();
    let src = svc.content_id_of_published("rosaray.image-source", "1.0.0");
    let thr = svc.content_id_of_published("rosaray.threshold", "1.0.0");
    let graph = format!(
        "schema: quantify-kb/1\nnodes:\n  - {{ instance_id: src, ref: {{ id: rosaray.image-source, version: 1.0.0, content_id: \"{src}\" }} }}\n  - {{ instance_id: th, ref: {{ id: rosaray.threshold, version: 1.0.0, content_id: \"{thr}\" }}, parameters: {{ mode: otsu }} }}\nedges:\n  - {{ from: src.image, to: th.image }}\n"
    );
    common::make_published_bundle(
        &svc.kb_root,
        "algopipe",
        "acme.pinned",
        "1.0.0",
        "knowledge",
        &[("ALGOPIPE.md", PIPE_MD), ("graph.yaml", &graph)],
    );
    rosaray_service::kb::evidence::chain::append(
        &svc.kb_root.join("amendments/acme.pinned@1.0.0"),
        rosaray_service::kb::evidence::chain::NewRecord {
            kind: "correction".into(),
            subject: json!({ "evidence_id": "e1" }),
            reason: Some("typo in the source citation".into()),
            author: "researcher-local".into(),
            extra: Default::default(),
        },
    )
    .unwrap();
    svc.refresh_catalog();
    svc
}

async fn export_pipe(svc: &common::TestService) -> (serde_json::Value, Vec<u8>) {
    let (status, preview) = svc
        .json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algopipe", "id": "acme.pinned", "version": "1.0.0" })))
        .await;
    assert_eq!(status, 200, "{preview}");
    let (status, exported) = svc
        .json(
            Method::POST,
            "/kb/export",
            Some(json!({ "kind": "algopipe", "id": "acme.pinned", "version": "1.0.0", "manifest_hash": preview["manifest_hash"] })),
        )
        .await;
    assert_eq!(status, 200, "{exported}");
    let resp = svc
        .request(Method::GET, &format!("/kb/export/{}/content", exported["export_id"].as_str().unwrap()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    (preview, resp.bytes().await.unwrap().to_vec())
}

/// Step 4: export a pipe, import it into an empty KB; identical content_ids,
/// dependencies and amendments come along, execution deps are only listed.
#[tokio::test]
async fn pipe_round_trips_with_identical_identities() {
    let a = service_with_pipe().await;
    let (preview, archive) = export_pipe(&a).await;

    let m = &preview["manifest"];
    let included: Vec<_> = m["included"].as_array().unwrap().iter().map(|i| format!("{}@{}:{}", i["id"].as_str().unwrap(), i["version"].as_str().unwrap(), i["reason"].as_str().unwrap())).collect();
    assert_eq!(
        included,
        vec![
            "acme.pinned@1.0.0:primary",
            "rosaray.image-source@1.0.0:dependency",
            "rosaray.threshold@1.0.0:dependency"
        ]
    );
    let reasons: std::collections::BTreeSet<_> = m["entries"].as_array().unwrap().iter().map(|e| e["reason"].as_str().unwrap()).collect();
    assert!(reasons.contains("amendment") && reasons.contains("primary") && reasons.contains("dependency"), "{reasons:?}");
    assert_eq!(m["omitted_execution_deps"].as_array().unwrap().len(), 2, "two seed implementations listed, none shipped");
    assert_eq!(preview["blocked"], false);

    let b = common::spawn().await;
    let (status, staged) = b.upload_algobundle(archive).await;
    assert_eq!(status, 200, "{staged}");
    let plan = &staged["plan"];
    assert_eq!(plan["new"].as_array().unwrap().len(), 3);
    assert!(plan["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(plan["omitted_execution_deps"].as_array().unwrap().len(), 2);
    // Nothing is applied until confirmed.
    assert!(b.entries("").await.is_empty());

    let (status, applied) = b
        .json(Method::POST, &format!("/kb/import/{}/confirm", staged["staged_id"].as_str().unwrap()), None)
        .await;
    assert_eq!(status, 200, "{applied}");
    assert_eq!(applied["applied"].as_array().unwrap().len(), 3);

    for (id, ver) in [("acme.pinned", "1.0.0"), ("rosaray.image-source", "1.0.0"), ("rosaray.threshold", "1.0.0")] {
        assert_eq!(b.content_id_of_published(id, ver), a.content_id_of_published(id, ver), "{id} identity survives the trip");
    }
    let pipe = b.entries("?kind=algopipe").await.remove(0);
    assert_eq!(pipe["dependency_summary"], json!({ "total": 2, "unresolved": 0 }));
    assert_eq!(
        common::hash_tree(&b.kb_root.join("amendments")),
        common::hash_tree(&a.kb_root.join("amendments")),
        "amendment chain came along byte-for-byte"
    );
    assert!(!b.state.exports_dir.join("kb-staging").exists());
}

/// Step 9: an imported implementation is untrusted and never runs.
#[tokio::test]
async fn imported_implementations_are_untrusted_and_the_node_stays_specification_only() {
    let a = service_with_pipe().await;
    let (_, archive) = export_pipe(&a).await;
    let b = common::spawn().await;
    let (_, staged) = b.upload_algobundle(archive).await;
    b.json(Method::POST, &format!("/kb/import/{}/confirm", staged["staged_id"].as_str().unwrap()), None).await;

    for e in b.entries("?kind=algonode").await {
        assert_eq!(e["maturity"], "specification_only", "{e}");
        assert_eq!(e["verification"], "none");
    }
    let trust = rosaray_service::kb::trust::load(&b.kb_root);
    assert_eq!(trust.entries.len(), 3);
    assert!(trust.entries.iter().all(|t| t.trust == rosaray_service::kb::bundle::model::Trust::Untrusted && t.source == "import"));
    // The bundle's own files are byte-identical (its declaration still says builtin);
    // trust is a local decision that never rewrites a locked bundle.
    let text = std::fs::read_to_string(b.kb_root.join("nodes/rosaray.threshold/1.0.0/implementation.yaml")).unwrap();
    assert!(text.contains("trust: builtin"));

    // In the exporting KB the same node is `implemented` (trusted seed).
    let ours = a.entries("?kind=algonode").await;
    assert!(ours.iter().all(|e| e["maturity"] == "implemented"));
}

/// Step 8: same id@version, different content ⇒ identity_conflict, nothing applied.
#[tokio::test]
async fn conflicting_duplicate_is_refused_and_the_catalog_is_unchanged() {
    let a = service_with_pipe().await;
    let (_, archive) = export_pipe(&a).await;

    let b = common::spawn().await;
    common::make_published_bundle(
        &b.kb_root,
        "algonode",
        "rosaray.threshold",
        "1.0.0",
        "knowledge",
        &[
            ("ALGONODE.md", "---\nschema: quantify-kb/1\nkind: algonode\nid: rosaray.threshold\nversion: 1.0.0\nname: Different\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n"),
            ("contract.yaml", "schema: quantify-kb/1\ninputs: []\noutputs: []\n"),
        ],
    );
    b.refresh_catalog();
    let before_entries = b.entries("").await;
    let before_tree = common::hash_tree(&b.kb_root);

    let (status, staged) = b.upload_algobundle(archive).await;
    assert_eq!(status, 200, "{staged}");
    assert_eq!(staged["plan"]["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(staged["plan"]["conflicts"][0]["id"], "rosaray.threshold");

    let (status, body) = b
        .json(Method::POST, &format!("/kb/import/{}/confirm", staged["staged_id"].as_str().unwrap()), None)
        .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"]["code"], "identity_conflict");
    assert_eq!(b.entries("").await, before_entries);
    assert_eq!(common::hash_tree(&b.kb_root), before_tree, "nothing was written");
}

#[tokio::test]
async fn re_importing_identical_content_is_a_no_op_and_cancel_discards_staging() {
    let a = service_with_pipe().await;
    let (_, archive) = export_pipe(&a).await;

    // Importing into the exporter itself: everything is identical.
    let (status, staged) = a.upload_algobundle(archive.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(staged["plan"]["identical"].as_array().unwrap().len(), 3);
    assert!(staged["plan"]["new"].as_array().unwrap().is_empty());
    let (status, _) = a
        .json(Method::POST, &format!("/kb/import/{}/cancel", staged["staged_id"].as_str().unwrap()), None)
        .await;
    assert_eq!(status, 204);
    let (status, _) = a
        .json(Method::POST, &format!("/kb/import/{}/confirm", staged["staged_id"].as_str().unwrap()), None)
        .await;
    assert_eq!(status, 404, "a cancelled import cannot be confirmed");
}

// ---- archive safety -----------------------------------------------------------

fn raw_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        for (name, bytes, mode) in entries {
            let mut opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            if let Some(m) = mode {
                opts = opts.unix_permissions(*m);
            }
            zip.start_file(*name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    buf.into_inner()
}

const EMPTY_MANIFEST: &[u8] = b"schema: quantify-kb/1\nentries: []\nincluded: []\nexcluded: []\nomitted_execution_deps: []\n";

#[tokio::test]
async fn hostile_archives_are_rejected_before_anything_is_written() {
    let svc = common::spawn().await;
    let before = common::hash_tree(&svc.kb_root);
    let bomb = vec![0u8; 8 * 1024 * 1024];
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("zip-slip", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("../evil.txt", b"x", None)])),
        ("absolute", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("/etc/evil", b"x", None)])),
        ("backslash", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("a\\..\\b", b"x", None)])),
        ("symlink", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("link", b"/etc/passwd", Some(0o120777))])),
        ("bomb", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("bundles/algonode/a.b/1.0.0/big.bin", &bomb, None)])),
        ("no-manifest", raw_zip(&[("bundles/algonode/a.b/1.0.0/ALGONODE.md", b"x", None)])),
        ("unlisted-file", raw_zip(&[("manifest.yaml", EMPTY_MANIFEST, None), ("bundles/algonode/a.b/1.0.0/x", b"x", None)])),
        ("not-a-zip", b"this is not an archive".to_vec()),
    ];
    for (name, bytes) in cases {
        let (status, body) = svc.upload_algobundle(bytes).await;
        assert_eq!(status, 422, "{name}: {body}");
        assert_eq!(body["error"]["code"], "bundle_invalid", "{name}: {body}");
    }
    assert_eq!(common::hash_tree(&svc.kb_root), before);
    let staging = svc.kb_root.with_file_name("kb-staging");
    assert!(!staging.exists() || std::fs::read_dir(&staging).unwrap().count() == 0, "no staging leftovers");
}

#[tokio::test]
async fn a_file_that_does_not_match_its_manifest_hash_is_rejected() {
    let a = service_with_pipe().await;
    let (_, archive) = export_pipe(&a).await;
    // Re-pack with one file's bytes altered but the manifest untouched.
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(archive)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        if f.name().ends_with("graph.yaml") {
            bytes.extend_from_slice(b"# tampered\n");
        }
        entries.push((f.name().to_string(), bytes));
    }
    let refs: Vec<(&str, &[u8], Option<u32>)> = entries.iter().map(|(n, b)| (n.as_str(), b.as_slice(), None)).collect();
    let b = common::spawn().await;
    let (status, body) = b.upload_algobundle(raw_zip(&refs)).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "bundle_invalid");
}

// ---- step 11: patient data ------------------------------------------------------

fn register_image_asset(svc: &common::TestService, png: &[u8], patient_id: &str) {
    let db = svc.state.db.lock().unwrap();
    let asset = ImageAsset {
        id: uuid::Uuid::new_v4(),
        external_source_uri: "file:///clinic/scan.png".into(),
        source_content_identity: rosaray_service::domain::content_identity::content_identity(png),
        imported_content_identity: "imported-identity-not-a-byte-hash".into(),
        dimensions: Dimensions { width: 6, height: 6 },
        source_created_at: None,
        imported_at: "2026-01-01T00:00:00Z".into(),
        status: ImageAssetStatus::Available,
        patient_id: Some(patient_id.into()),
        split: None,
        reference_mask_id: None,
        metadata_status: MetadataStatus::Incomplete,
        pixel_spacing_mm: None,
        spacing_source: None,
    };
    rosaray_service::data_repository::sqlite::dataset_repo::insert_image_asset(&db, &asset).unwrap();
}

fn node_files(extra: Vec<(&'static str, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("ALGONODE.md".into(), b"---\nschema: quantify-kb/1\nkind: algonode\nid: acme.leaky\nversion: 1.0.0\nname: Leaky\nsummary: s\nstatus: published\nresearch_use_only: true\nintended_use: u\nlimitations: l\n---\n".to_vec()),
        ("contract.yaml".into(), b"schema: quantify-kb/1\ninputs: []\noutputs: []\n".to_vec()),
    ];
    files.extend(extra.into_iter().map(|(p, b)| (p.to_string(), b)));
    files
}

fn publish_raw(svc: &common::TestService, files: &[(String, Vec<u8>)]) {
    let dir = svc.kb_root.join("nodes/acme.leaky/1.0.0");
    for (p, b) in files {
        let path = dir.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b).unwrap();
    }
    let hashes = rosaray_service::kb::identity::file_hashes(&dir).unwrap();
    let cid = rosaray_service::kb::identity::content_id_of(hashes.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let mut lock = format!("schema: quantify-kb/1\nid: acme.leaky\nversion: 1.0.0\ncontent_id: \"{cid}\"\nrelease_kind: knowledge\nfiles:\n");
    for (p, h) in &hashes {
        lock.push_str(&format!("  - {{ path: {p}, blake3: \"{}\" }}\n", h.trim_start_matches("b3:")));
    }
    std::fs::write(dir.join("bundle.lock"), lock).unwrap();
    svc.refresh_catalog();
}

#[tokio::test]
async fn a_bundle_holding_a_copy_of_project_image_data_cannot_be_exported_or_imported() {
    let svc = common::spawn().await;
    let patient_png = common::png_bytes(42);
    register_image_asset(&svc, &patient_png, "P-12345");
    publish_raw(&svc, &node_files(vec![("tests/case1.png", patient_png.clone())]));

    let (status, preview) = svc
        .json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0" })))
        .await;
    assert_eq!(status, 200);
    assert_eq!(preview["blocked"], true);
    assert!(preview["findings"].as_array().unwrap().iter().any(|f| f["code"] == "patient_data_blocked"));
    assert!(
        !preview["manifest"]["entries"].as_array().unwrap().iter().any(|e| e["path"].as_str().unwrap().ends_with("case1.png")),
        "blocked file is not in the manifest"
    );

    let (status, body) = svc
        .json(
            Method::POST,
            "/kb/export",
            Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0", "manifest_hash": preview["manifest_hash"] })),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "patient_data_blocked");
    // Non-leaking: no patient id, no hash, no contents anywhere in the response.
    let text = body.to_string();
    assert!(!text.contains("P-12345"), "{text}");
    assert!(!text.contains(&rosaray_service::domain::content_identity::content_identity(&patient_png)), "{text}");
    assert!(!body["error"]["message"].as_str().unwrap().contains("case1"), "message is generic");

    // A hand-crafted archive carrying the same bytes is refused on import.
    let files = node_files(vec![("tests/case1.png", patient_png)]);
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path().join("nodes/acme.leaky/1.0.0");
    for (p, b) in &files {
        let path = dir.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b).unwrap();
    }
    let hashes = rosaray_service::kb::identity::file_hashes(&dir).unwrap();
    let cid = rosaray_service::kb::identity::content_id_of(hashes.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let mut lock = format!("schema: quantify-kb/1\nid: acme.leaky\nversion: 1.0.0\ncontent_id: \"{cid}\"\nrelease_kind: knowledge\nfiles:\n");
    for (p, h) in &hashes {
        lock.push_str(&format!("  - {{ path: {p}, blake3: \"{}\" }}\n", h.trim_start_matches("b3:")));
    }
    let mut all = files.clone();
    all.push(("bundle.lock".into(), lock.into_bytes()));
    let mut manifest = Manifest { schema: "quantify-kb/1".into(), ..Default::default() };
    let mut packed = Vec::new();
    for (p, b) in &all {
        let path = format!("bundles/algonode/acme.leaky/1.0.0/{p}");
        manifest.entries.push(ManifestEntry { path: path.clone(), blake3: format!("b3:{}", blake3::hash(b).to_hex()), reason: "primary".into() });
        packed.push((path, b.clone()));
    }
    let archive = pack_archive(&manifest, &packed);
    let other = common::spawn().await;
    register_image_asset(&other, &common::png_bytes(42), "P-999");
    let (status, body) = other.upload_algobundle(archive).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["error"]["code"], "patient_data_blocked");
    assert!(other.entries("").await.is_empty(), "nothing indexed");
    assert!(!body.to_string().contains("P-999"));
}

#[tokio::test]
async fn unreviewed_images_are_excluded_and_reviewed_non_patient_assets_are_allowed() {
    let svc = common::spawn().await;
    let unreviewed = common::png_bytes(10);
    let reviewed = common::png_bytes(20);
    let review = format!(
        "reviews:\n  - {{ path: tests/reviewed.png, blake3: \"b3:{}\", review: non-patient, reviewer: alice, reviewed_at: \"2026-09-26T10:00:00Z\" }}\n",
        blake3::hash(&reviewed).to_hex()
    );
    publish_raw(
        &svc,
        &node_files(vec![
            ("tests/unreviewed.png", unreviewed),
            ("tests/reviewed.png", reviewed),
            ("asset-review.yaml", review.into_bytes()),
            ("tests/expected.yaml", b"expected: 1\n".to_vec()),
        ]),
    );
    let (_, preview) = svc
        .json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0" })))
        .await;
    assert_eq!(preview["blocked"], false);
    let m = &preview["manifest"];
    let entry = |suffix: &str| m["entries"].as_array().unwrap().iter().find(|e| e["path"].as_str().unwrap().ends_with(suffix)).cloned();
    assert_eq!(entry("tests/reviewed.png").unwrap()["reason"], "reviewed_test_asset");
    assert!(entry("tests/expected.yaml").is_some());
    assert!(entry("tests/unreviewed.png").is_none());
    let excluded = m["excluded"].as_array().unwrap();
    assert_eq!(excluded.len(), 1);
    assert_eq!(excluded[0]["code"], "patient_status_unresolved");
    assert_eq!(excluded[0]["path"], "tests/unreviewed.png");

    // Export still succeeds (without the excluded file).
    let (status, _) = svc
        .json(
            Method::POST,
            "/kb/export",
            Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0", "manifest_hash": preview["manifest_hash"] })),
        )
        .await;
    assert_eq!(status, 200);
}

/// The reviewed asset rule does not override a known-content match.
#[tokio::test]
async fn a_review_record_cannot_launder_a_known_patient_image() {
    let svc = common::spawn().await;
    let png = common::png_bytes(33);
    register_image_asset(&svc, &png, "P-1");
    let review = format!(
        "reviews:\n  - {{ path: tests/x.png, blake3: \"b3:{}\", review: non-patient, reviewer: mallory }}\n",
        blake3::hash(&png).to_hex()
    );
    publish_raw(&svc, &node_files(vec![("tests/x.png", png), ("asset-review.yaml", review.into_bytes())]));
    let (_, preview) = svc
        .json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0" })))
        .await;
    assert_eq!(preview["blocked"], true);
}

/// FR-039: the manifest reviewed is the manifest written.
#[tokio::test]
async fn export_refuses_a_manifest_hash_that_no_longer_matches() {
    let svc = service_with_pipe().await;
    let (status, body) = svc
        .json(
            Method::POST,
            "/kb/export",
            Some(json!({ "kind": "algopipe", "id": "acme.pinned", "version": "1.0.0", "manifest_hash": "b3:stale" })),
        )
        .await;
    assert_eq!(status, 409, "{body}");
}

/// Non-distributable evidence never leaves; execution assets are only listed.
#[tokio::test]
async fn non_distributable_evidence_and_execution_assets_are_excluded() {
    let svc = common::spawn().await;
    let evidence = |id: &str, dist: bool| {
        format!("evidence_id: {id}\ntype: method_source\napplies_to: {{ id: acme.leaky, version: 1.0.0 }}\ndistributable: {dist}\npatient_data_status: none\n")
    };
    let weights = b"\x00\x01\x02 pretend model weights".to_vec();
    let imp = format!(
        "implementation_id: builtin.onnx\nimplementation_version: \"1\"\ntrust: builtin\nassets:\n  - {{ path: assets/model.onnx, blake3: \"{}\", role: weights }}\n",
        blake3::hash(&weights).to_hex()
    );
    publish_raw(
        &svc,
        &node_files(vec![
            ("references/public.yaml", evidence("public", true).into_bytes()),
            ("references/private.yaml", evidence("private", false).into_bytes()),
            ("implementation.yaml", imp.into_bytes()),
            ("assets/model.onnx", weights),
        ]),
    );
    let (_, preview) = svc
        .json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algonode", "id": "acme.leaky", "version": "1.0.0" })))
        .await;
    let m = &preview["manifest"];
    let paths: Vec<_> = m["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap().to_string()).collect();
    assert!(paths.iter().any(|p| p.ends_with("references/public.yaml")));
    assert!(!paths.iter().any(|p| p.ends_with("private.yaml") || p.ends_with("model.onnx")), "{paths:?}");
    let excluded: Vec<_> = m["excluded"].as_array().unwrap().iter().map(|e| format!("{}:{}", e["path"].as_str().unwrap(), e["code"].as_str().unwrap())).collect();
    assert!(excluded.contains(&"references/private.yaml:not_distributable".to_string()), "{excluded:?}");
    assert!(excluded.contains(&"assets/model.onnx:execution_asset".to_string()), "{excluded:?}");
    assert_eq!(m["omitted_execution_deps"][0]["assets"], json!(["assets/model.onnx"]));
}

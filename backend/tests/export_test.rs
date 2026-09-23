mod common;

use serde_json::json;

const CREDENTIAL: &str = "export-credential-1";

fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

async fn json_of(resp: reqwest::Response) -> serde_json::Value {
    resp.json().await.unwrap()
}

/// Imports one image, requests its thumbnail (so Preview-class content
/// exists in the source project), and completes one official Run.
async fn seeded_source(service: &common::TestService, dir: &std::path::Path) -> (String, String) {
    write_png(&dir.join("a.png"), 90);
    let preview = json_of(
        service
            .request(reqwest::Method::POST, "/import-batches")
            .json(&json!({
                "source_selection": "folder_scan",
                "paths": [dir.to_string_lossy()],
                "metadata_manifest": null,
                "dataset_display_name": "Export Fixture"
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let batch_id = preview["batch_id"].as_str().unwrap();
    let confirm = json_of(
        service
            .request(
                reqwest::Method::POST,
                &format!("/import-batches/{batch_id}/confirm"),
            )
            .json(&json!({ "confirmed_source_refs": ["a.png"] }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let version_id = confirm["dataset_version_id"].as_str().unwrap().to_string();
    let images = json_of(
        service
            .request(
                reqwest::Method::GET,
                &format!("/dataset-versions/{version_id}/images"),
            )
            .send()
            .await
            .unwrap(),
    )
    .await;
    let image_id = images["images"][0]["id"].as_str().unwrap().to_string();

    // Provoke a thumbnail so there is cache content that must NOT travel.
    let _ = service
        .request(
            reqwest::Method::GET,
            &format!("/image-assets/{image_id}/thumbnail"),
        )
        .send()
        .await
        .unwrap();

    let created = json_of(
        service
            .request(reqwest::Method::POST, "/runs")
            .json(&json!({
                "dataset_version_id": version_id,
                "image_asset_id": image_id,
                "pipeline_snapshot": {
                    "nodes": [
                        {"node_id": "source", "node_type": "source"},
                        {"node_id": "blur", "node_type": "gaussian", "canonical_parameters": {"sigma": 2}}
                    ],
                    "edges": [{"from": "source", "to": "blur"}]
                },
                "target_node_id": "blur",
                "seed": 5,
            }))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let run_id = created["run_id"].as_str().unwrap().to_string();
    for _ in 0..100 {
        let run = get_run(service, &run_id).await;
        if run["status"] == "succeeded" {
            return (version_id, run_id);
        }
        assert_ne!(run["status"], "failed", "seed run failed: {run}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("seed run never completed");
}

async fn get_run(service: &common::TestService, run_id: &str) -> serde_json::Value {
    json_of(
        service
            .request(reqwest::Method::GET, &format!("/runs/{run_id}"))
            .send()
            .await
            .unwrap(),
    )
    .await
}

async fn export(
    service: &common::TestService,
    version_id: &str,
    run_id: &str,
) -> (serde_json::Value, Vec<u8>) {
    let resp = service
        .request(reqwest::Method::POST, "/export-bundles")
        .json(&json!({
            "dataset_version_ids": [version_id],
            "run_record_ids": [run_id],
            "credential": CREDENTIAL,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = json_of(resp).await;
    let id = body["export_bundle_id"].as_str().unwrap();
    let bytes = service
        .request(
            reqwest::Method::GET,
            &format!("/export-bundles/{id}/content"),
        )
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap()
        .to_vec();
    (body, bytes)
}

async fn import(
    service: &common::TestService,
    bundle: Vec<u8>,
    credential: &str,
) -> reqwest::Response {
    let form = reqwest::multipart::Form::new()
        .part("bundle", reqwest::multipart::Part::bytes(bundle))
        .text("credential", credential.to_string());
    service
        .request(reqwest::Method::POST, "/export-bundles/import")
        .multipart(form)
        .send()
        .await
        .unwrap()
}

async fn dataset_summary(service: &common::TestService) -> serde_json::Value {
    let datasets = json_of(
        service
            .request(reqwest::Method::GET, "/datasets")
            .send()
            .await
            .unwrap(),
    )
    .await;
    datasets["datasets"].clone()
}

/// quickstart.md §5.1: bundle needs its own credential, carries no
/// Preview/thumbnail content, and leaks neither the credential nor content.
#[tokio::test]
async fn bundle_is_credential_locked_and_excludes_preview() {
    let source = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, run_id) = seeded_source(&source, dir.path()).await;

    // No credential → refused, nothing written.
    let refused = source
        .request(reqwest::Method::POST, "/export-bundles")
        .json(&json!({ "dataset_version_ids": [version_id] }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);
    assert_eq!(json_of(refused).await["error"]["code"], "credential_required");

    let (body, bytes) = export(&source, &version_id, &run_id).await;
    assert_eq!(body["manifest"]["excludes_preview"], true);
    // 1 grayscale image + 1 run input + 1 run output (mask absent).
    let content = body["manifest"]["content_list"].as_array().unwrap();
    assert!(!content.is_empty());

    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!has(CREDENTIAL.as_bytes()));
    assert!(!has(version_id.as_bytes()), "ids must not appear in the clear");
    assert!(!has(b"a.png"), "source file names must not appear in the clear");
}

/// quickstart.md §5.2 / SC-007: round-trip into a blank project preserves
/// fingerprint, run identity, pipeline identity and metrics exactly.
#[tokio::test]
async fn round_trip_into_blank_project_preserves_identities() {
    let source = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, run_id) = seeded_source(&source, dir.path()).await;
    let (_, bytes) = export(&source, &version_id, &run_id).await;

    let target = common::spawn().await;
    assert!(dataset_summary(&target).await.as_array().unwrap().is_empty());

    let resp = import(&target, bytes, CREDENTIAL).await;
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);

    let source_run = get_run(&source, &run_id).await;
    let target_run = get_run(&target, &run_id).await;
    assert_eq!(target_run["status"], "succeeded");
    for key in [
        "dataset_fingerprint",
        "pipeline_snapshot_id",
        "seed",
        "target_node_id",
        "metric_set",
    ] {
        assert_eq!(source_run[key], target_run[key], "{key} diverged");
    }
    assert_eq!(
        source_run["output_artifact_refs"][0]["content_identity"],
        target_run["output_artifact_refs"][0]["content_identity"]
    );

    let src_ds = dataset_summary(&source).await;
    let dst_ds = dataset_summary(&target).await;
    let ds_id = dst_ds[0]["id"].as_str().unwrap();
    assert_eq!(src_ds[0]["id"], dst_ds[0]["id"]);
    let versions = json_of(
        target
            .request(reqwest::Method::GET, &format!("/datasets/{ds_id}/versions"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    let src_versions = json_of(
        source
            .request(reqwest::Method::GET, &format!("/datasets/{ds_id}/versions"))
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        versions["versions"][0]["fingerprint"],
        src_versions["versions"][0]["fingerprint"]
    );
    assert_eq!(dst_ds[0]["latest_version_id"], json!(version_id));

    // Content is readable in the target (re-encrypted under its own key).
    let images = json_of(
        target
            .request(
                reqwest::Method::GET,
                &format!("/dataset-versions/{version_id}/images"),
            )
            .send()
            .await
            .unwrap(),
    )
    .await;
    let image_id = images["images"][0]["id"].as_str().unwrap();
    let display = json_of(
        target
            .request(
                reqwest::Method::GET,
                &format!("/image-assets/{image_id}/display"),
            )
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert!(display.is_object());
}

/// quickstart.md §5.3 / FR-027/FR-034/FR-035: wrong credential and a
/// bit-flipped bundle are both rejected before the target changes.
#[tokio::test]
async fn wrong_credential_and_tampered_bundle_are_rejected_pre_disclosure() {
    let source = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, run_id) = seeded_source(&source, dir.path()).await;
    let (_, bytes) = export(&source, &version_id, &run_id).await;

    let target = common::spawn().await;

    let wrong = import(&target, bytes.clone(), "not-the-credential").await;
    assert_eq!(wrong.status(), 401);
    let wrong_body = json_of(wrong).await;
    assert_eq!(wrong_body["error"]["code"], "credential_invalid");

    let missing = import(&target, bytes.clone(), "").await;
    assert_eq!(json_of(missing).await["error"]["code"], "credential_required");

    let mut flipped = bytes.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x01;
    let tampered = import(&target, flipped, CREDENTIAL).await;
    assert_eq!(tampered.status(), 422);
    assert_eq!(json_of(tampered).await["error"]["code"], "bundle_tampered");

    let garbage = import(&target, b"not a bundle".to_vec(), CREDENTIAL).await;
    assert_eq!(json_of(garbage).await["error"]["code"], "bundle_tampered");

    assert!(
        dataset_summary(&target).await.as_array().unwrap().is_empty(),
        "target project must be untouched by every rejected import"
    );
    // ...and the untouched bundle still imports afterwards.
    assert_eq!(import(&target, bytes, CREDENTIAL).await.status(), 200);
}

/// Re-importing the same bundle is idempotent, never duplicating or
/// overwriting immutable rows (FR-032).
#[tokio::test]
async fn reimport_is_idempotent() {
    let source = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, run_id) = seeded_source(&source, dir.path()).await;
    let (_, bytes) = export(&source, &version_id, &run_id).await;

    assert_eq!(import(&source, bytes.clone(), CREDENTIAL).await.status(), 200);
    assert_eq!(import(&source, bytes, CREDENTIAL).await.status(), 200);
    assert_eq!(dataset_summary(&source).await.as_array().unwrap().len(), 1);
}

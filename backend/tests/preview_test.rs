mod common;

use serde_json::json;

fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

async fn import_one_image(service: &common::TestService, dir: &std::path::Path) -> String {
    write_png(&dir.join("normal.png"), 77);

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.to_string_lossy()],
            "metadata_manifest": null,
            "dataset_display_name": "Preview Fixture"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let batch_id = preview["batch_id"].as_str().unwrap().to_string();

    let confirm: serde_json::Value = service
        .request(
            reqwest::Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
        )
        .json(&json!({ "confirmed_source_refs": ["normal.png"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let version_id = confirm["dataset_version_id"].as_str().unwrap().to_string();

    let images: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/dataset-versions/{version_id}/images"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    images["images"][0]["id"].as_str().unwrap().to_string()
}

fn simple_pipeline() -> serde_json::Value {
    json!({
        "nodes": [
            {"node_id": "source", "node_type": "source"},
            {"node_id": "blur", "node_type": "gaussian", "canonical_parameters": {"sigma": 2}}
        ],
        "edges": [{"from": "source", "to": "blur"}]
    })
}

/// quickstart.md §3 step 1: repeated identical preview requests reuse
/// without a new Run (FR-013/FR-014).
#[tokio::test]
async fn identical_preview_requests_are_reused() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let image_id = import_one_image(&service, dir.path()).await;

    let first: serde_json::Value = service
        .request(reqwest::Method::POST, "/preview")
        .json(&json!({
            "image_asset_id": image_id,
            "pipeline_snapshot": simple_pipeline(),
            "target_node_id": "blur",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["reused"], false);
    assert!(first["artifact_ref"]["id"].is_string());

    let second: serde_json::Value = service
        .request(reqwest::Method::POST, "/preview")
        .json(&json!({
            "image_asset_id": image_id,
            "pipeline_snapshot": simple_pipeline(),
            "target_node_id": "blur",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(second["reused"], true);
    assert_eq!(
        second["artifact_ref"]["content_identity"],
        first["artifact_ref"]["content_identity"]
    );
    assert_ne!(second["request_context_id"], first["request_context_id"]);
}

/// quickstart.md §3 step 4: a non-reproducible node with no seed fails, but
/// identifies the failing node and falls back to the last successful
/// result for that selection (FR-015/FR-017).
#[tokio::test]
async fn missing_seed_fails_with_last_successful_fallback() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let image_id = import_one_image(&service, dir.path()).await;

    let ok: serde_json::Value = service
        .request(reqwest::Method::POST, "/preview")
        .json(&json!({
            "image_asset_id": image_id,
            "pipeline_snapshot": simple_pipeline(),
            "target_node_id": "blur",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let last_good_identity = ok["artifact_ref"]["content_identity"].clone();

    let mut flaky_pipeline = simple_pipeline();
    flaky_pipeline["nodes"][1]["reproducible"] = json!(false);

    let failed: serde_json::Value = service
        .request(reqwest::Method::POST, "/preview")
        .json(&json!({
            "image_asset_id": image_id,
            "pipeline_snapshot": flaky_pipeline,
            "target_node_id": "blur",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(failed["state"], "stale");
    assert_eq!(failed["failing_node_id"], "blur");
    assert_eq!(failed["error"]["code"], "missing_seed");
    assert_eq!(
        failed["last_successful_artifact_ref"]["content_identity"],
        last_good_identity
    );
}

#[tokio::test]
async fn unknown_target_node_reports_that_node_as_failing() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let image_id = import_one_image(&service, dir.path()).await;

    let failed: serde_json::Value = service
        .request(reqwest::Method::POST, "/preview")
        .json(&json!({
            "image_asset_id": image_id,
            "pipeline_snapshot": simple_pipeline(),
            "target_node_id": "does-not-exist",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(failed["state"], "stale");
    assert_eq!(failed["failing_node_id"], "does-not-exist");
    assert_eq!(failed["error"]["code"], "unknown_node");
    assert!(failed["last_successful_artifact_ref"].is_null());
}

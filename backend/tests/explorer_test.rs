mod common;

use serde_json::json;

fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

async fn import_one_image(
    service: &common::TestService,
    dir: &std::path::Path,
) -> (String, String) {
    write_png(&dir.join("normal.png"), 77);

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.to_string_lossy()],
            "metadata_manifest": null,
            "dataset_display_name": "Explorer Fixture"
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
    let image_id = images["images"][0]["id"].as_str().unwrap().to_string();

    (version_id, image_id)
}

/// quickstart.md §2 steps 1-2: list versions/images without pixel loads,
/// then fetch the display descriptor and the actual image content behind
/// its `ArtifactReference`.
#[tokio::test]
async fn explorer_list_display_and_content_roundtrip() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, image_id) = import_one_image(&service, dir.path()).await;

    let versions: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/datasets/{}/versions", uuid::Uuid::nil()),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Wrong dataset id still returns a well-formed (empty) list, never an error leak.
    assert_eq!(versions["versions"].as_array().unwrap().len(), 0);

    let display: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/image-assets/{image_id}/display"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(display["image_asset_id"], image_id);
    assert_eq!(display["dataset_version_id"], version_id);
    assert_eq!(display["source_status"], "available");
    // No reference mask was ever provided for this fixture image.
    assert!(display["reference_mask_unavailable_reason"].is_string());
    assert!(display["reference_mask_ref"].is_null());

    let artifact_id = display["image_artifact_ref"]["id"].as_str().unwrap();
    let content = service
        .request(
            reqwest::Method::GET,
            &format!("/artifacts/{artifact_id}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), 200);
    assert_eq!(content.headers().get("content-type").unwrap(), "image/png");
    let bytes = content.bytes().await.unwrap();
    assert!(!bytes.is_empty());
}

/// Explorer must be able to discover existing Datasets on startup, before
/// selecting any Dataset Version (US2 acceptance scenario 1) — not just
/// learn the dataset id as the return value of a fresh import.
#[tokio::test]
async fn datasets_are_discoverable_after_import() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    import_one_image(&service, dir.path()).await;

    let datasets: serde_json::Value = service
        .request(reqwest::Method::GET, "/datasets")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = datasets["datasets"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["display_name"], "Explorer Fixture");
    assert!(list[0]["latest_version_id"].is_string());
}

#[tokio::test]
async fn datasets_list_is_empty_before_any_import() {
    let service = common::spawn().await;

    let datasets: serde_json::Value = service
        .request(reqwest::Method::GET, "/datasets")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(datasets["datasets"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn unknown_artifact_id_is_not_found() {
    let service = common::spawn().await;
    let response = service
        .request(
            reqwest::Method::GET,
            &format!("/artifacts/{}/content", uuid::Uuid::new_v4()),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

/// quickstart.md §2: thumbnail generation starts as `generating` and
/// eventually becomes `ready` on a follow-up request.
#[tokio::test]
async fn thumbnail_generates_then_becomes_ready() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (_version_id, image_id) = import_one_image(&service, dir.path()).await;

    let first: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/image-assets/{image_id}/thumbnail"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["state"], "generating");

    // Generation runs on a background task; poll briefly for it to land.
    for _ in 0..20 {
        let poll: serde_json::Value = service
            .request(
                reqwest::Method::GET,
                &format!("/image-assets/{image_id}/thumbnail"),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if poll["state"] == "ready" {
            assert!(poll["data_base64"].is_string());
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("thumbnail never reached the ready state");
}

#[tokio::test]
async fn cancel_request_is_idempotent_on_unknown_id() {
    let service = common::spawn().await;
    let response = service
        .request(
            reqwest::Method::DELETE,
            &format!("/requests/{}", uuid::Uuid::new_v4()),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

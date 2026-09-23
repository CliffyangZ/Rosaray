//! quickstart.md §1 end to end against the full fixture, plus the
//! version-derivation rules (FR-007/FR-032). §2–§5 live beside their
//! features in explorer/preview/runs/export tests.

mod common;

use reqwest::Method;
use serde_json::json;

/// §1.1–§1.4: classification with zero writes, confirm creates exactly one
/// version, leakage is a blocking finding, and an incompatible mask is
/// surfaced as an explicit reason rather than a blank overlay.
#[tokio::test]
async fn full_fixture_import_validates_leakage_and_mask() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("normal.png"), 10);
    common::write_png(&dir.path().join("duplicate.png"), 10);
    common::write_png(&dir.path().join("no_patient.png"), 20);
    common::write_png(&dir.path().join("leak_a.png"), 30);
    common::write_png(&dir.path().join("leak_b.png"), 31);
    common::write_png(&dir.path().join("masked.png"), 40);
    image::GrayImage::from_pixel(4, 4, image::Luma([255]))
        .save(dir.path().join("bad_mask.png"))
        .unwrap();

    let manifest = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "manifest_version": "1.0",
        "entries": [
            {"source_ref": "normal.png", "patient_id": "P001", "split": "train", "reference_mask_ref": null},
            {"source_ref": "leak_a.png", "patient_id": "P002", "split": "train", "reference_mask_ref": null},
            {"source_ref": "leak_b.png", "patient_id": "P002", "split": "test", "reference_mask_ref": null},
            {"source_ref": "masked.png", "patient_id": "P003", "split": "train", "reference_mask_ref": "bad_mask.png"}
        ]
    });
    let (status, preview) = service
        .json(
            Method::POST,
            "/import-batches",
            Some(json!({
                "source_selection": "folder_scan",
                "paths": [dir.path().to_string_lossy()],
                "metadata_manifest": manifest,
                "dataset_display_name": "Quickstart",
            })),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(preview["status"], "previewing");

    // §1.1: nothing exists yet.
    let (_, datasets) = service.json(Method::GET, "/datasets", None).await;
    assert!(datasets["datasets"].as_array().unwrap().is_empty());

    let class = |name: &str| {
        preview["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["source_ref"] == name)
            .unwrap()["classification"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let pair = [class("normal.png"), class("duplicate.png")];
    assert!(pair.contains(&"duplicate".to_string()) && pair.contains(&"importable".to_string()));
    let kept = if class("normal.png") == "importable" { "normal.png" } else { "duplicate.png" };

    // §1.2
    let batch_id = preview["batch_id"].as_str().unwrap();
    let (status, confirm) = service
        .json(
            Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
            Some(json!({ "confirmed_source_refs": [kept, "no_patient.png", "leak_a.png", "leak_b.png", "masked.png"] })),
        )
        .await;
    assert_eq!(status, 200, "{confirm}");
    let version_id = confirm["dataset_version_id"].as_str().unwrap();
    let dataset_id = confirm["dataset_id"].as_str().unwrap();
    let (_, versions) = service
        .json(Method::GET, &format!("/datasets/{dataset_id}/versions"), None)
        .await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 1, "exactly one new version");

    // §1.3: cross-split subject → blocking, not usable for official evaluation.
    assert_eq!(versions["versions"][0]["validation_summary"]["status"], "blocked");
    let (_, images) = service
        .json(Method::GET, &format!("/dataset-versions/{version_id}/images"), None)
        .await;
    let images = images["images"].as_array().unwrap();
    assert_eq!(images.len(), 5);
    assert!(
        images.iter().any(|i| i["patient_id"] == "P002"),
        "leaking subject is listed with its patient id"
    );

    // §1.4: the masked image explains why there is no overlay.
    let masked = images.iter().find(|i| i["patient_id"] == "P003").unwrap();
    let (status, display) = service
        .json(Method::GET, &format!("/image-assets/{}/display", masked["id"].as_str().unwrap()), None)
        .await;
    assert_eq!(status, 200);
    assert!(display["reference_mask_unavailable_reason"].is_string());
}

/// §1.5/§1.6 (FR-007, FR-032): a content change derives a new immutable
/// version with a different fingerprint and leaves the original — and Runs
/// against it — untouched; re-confirming identical content creates nothing.
#[tokio::test]
async fn content_change_derives_new_version_and_identical_content_does_not() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 70);
    let (dataset_id, v1) = service.import_dir(dir.path(), "Derive").await;
    let (_, before) = service
        .json(Method::GET, &format!("/datasets/{dataset_id}/versions"), None)
        .await;
    let fingerprint_v1 = before["versions"][0]["fingerprint"].clone();

    let image_id = service.image_ids(&v1).await.remove(0);
    let run = service.run_to_completion(&v1, &image_id, 2).await;
    assert_eq!(run["status"], "succeeded");

    common::write_png(&dir.path().join("b.png"), 71);
    let (_, v2) = service.import_dir(dir.path(), "Derive").await;
    assert_ne!(v1, v2);

    let (_, after) = service
        .json(Method::GET, &format!("/datasets/{dataset_id}/versions"), None)
        .await;
    let versions = after["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    let original = versions.iter().find(|v| v["id"] == json!(v1)).unwrap();
    let derived = versions.iter().find(|v| v["id"] == json!(v2)).unwrap();
    assert_eq!(original["fingerprint"], fingerprint_v1, "v1 is immutable");
    assert_ne!(derived["fingerprint"], fingerprint_v1);
    assert_eq!(service.image_ids(&v1).await.len(), 1);

    let (_, rerun) = service
        .json(Method::GET, &format!("/runs/{}", run["id"].as_str().unwrap()), None)
        .await;
    assert_eq!(rerun["dataset_version_id"], json!(v1));
    assert_eq!(rerun["dataset_fingerprint"], run["dataset_fingerprint"]);

    // Same content again → every candidate is a duplicate → no new version.
    let (_, preview) = service
        .json(
            Method::POST,
            "/import-batches",
            Some(json!({
                "source_selection": "folder_scan",
                "paths": [dir.path().to_string_lossy()],
                "metadata_manifest": null,
                "dataset_display_name": "Derive",
            })),
        )
        .await;
    let batch_id = preview["batch_id"].as_str().unwrap();
    let (status, _) = service
        .json(
            Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
            Some(json!({ "confirmed_source_refs": [] })),
        )
        .await;
    assert_ne!(status, 200);
    let (_, still) = service
        .json(Method::GET, &format!("/datasets/{dataset_id}/versions"), None)
        .await;
    assert_eq!(still["versions"].as_array().unwrap().len(), 2);
}

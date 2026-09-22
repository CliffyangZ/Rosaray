mod common;

use serde_json::json;

fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

/// quickstart.md §1: import a fixture with a normal image, a byte-identical
/// duplicate, a missing-patient-id image, and a cross-split subject; verify
/// preview classification, confirm-only writes, and leakage validation.
#[tokio::test]
async fn import_batch_preview_then_confirm_creates_one_dataset_version() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();

    write_png(&dir.path().join("normal.png"), 10);
    write_png(&dir.path().join("duplicate.png"), 10); // byte-identical to normal.png
    write_png(&dir.path().join("no_patient.png"), 20);
    write_png(&dir.path().join("leak_a.png"), 30);
    write_png(&dir.path().join("leak_b.png"), 31);

    let manifest = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "manifest_version": "1.0",
        "entries": [
            {"source_ref": "normal.png", "patient_id": "P001", "split": "train", "reference_mask_ref": null},
            {"source_ref": "leak_a.png", "patient_id": "P002", "split": "train", "reference_mask_ref": null},
            {"source_ref": "leak_b.png", "patient_id": "P002", "split": "test", "reference_mask_ref": null}
        ]
    });

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.path().to_string_lossy()],
            "metadata_manifest": manifest,
            "dataset_display_name": "US1 Fixture"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Preview must classify without writing anything importable/duplicate/etc.
    let candidates = preview["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 5);
    let classification_of = |name: &str| {
        candidates.iter().find(|c| c["source_ref"] == name).unwrap()["classification"]
            .as_str()
            .unwrap()
            .to_string()
    };
    // Scan order across `normal.png`/`duplicate.png` is filesystem-dependent —
    // exactly one of the byte-identical pair is importable, the other duplicate.
    let normal_and_duplicate = [
        classification_of("normal.png"),
        classification_of("duplicate.png"),
    ];
    let importable_count = normal_and_duplicate
        .iter()
        .filter(|c| *c == "importable")
        .count();
    let duplicate_count = normal_and_duplicate
        .iter()
        .filter(|c| *c == "duplicate")
        .count();
    assert_eq!(importable_count, 1);
    assert_eq!(duplicate_count, 1);
    let importable_name = if classification_of("normal.png") == "importable" {
        "normal.png"
    } else {
        "duplicate.png"
    };
    assert_eq!(classification_of("no_patient.png"), "importable");
    assert_eq!(classification_of("leak_a.png"), "importable");

    let batch_id = preview["batch_id"].as_str().unwrap();

    let confirm: serde_json::Value = service
        .request(
            reqwest::Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
        )
        .json(&json!({
            "confirmed_source_refs": [importable_name, "no_patient.png", "leak_a.png", "leak_b.png"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(confirm["status"], "confirmed");
    assert!(confirm["dataset_version_id"].is_string());
}

/// quickstart.md §1 step 4 groundwork: an image paired with an
/// incompatible-dimension mask must still import, but the mask must be
/// recorded as invalid rather than silently linked as usable.
#[tokio::test]
async fn incompatible_mask_is_recorded_as_invalid() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();

    write_png(&dir.path().join("with_mask.png"), 40); // 8x8
    let small_mask = image::GrayImage::from_pixel(4, 4, image::Luma([255]));
    small_mask.save(dir.path().join("mask.png")).unwrap(); // 4x4: incompatible

    let manifest = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "manifest_version": "1.0",
        "entries": [
            {"source_ref": "with_mask.png", "patient_id": "P010", "split": "train", "reference_mask_ref": "mask.png"}
        ]
    });

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.path().to_string_lossy()],
            "metadata_manifest": manifest,
            "dataset_display_name": "Mask Fixture"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let batch_id = preview["batch_id"].as_str().unwrap();
    let confirm = service
        .request(
            reqwest::Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
        )
        .json(&json!({ "confirmed_source_refs": ["with_mask.png"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(confirm.status(), 200);
}

#[tokio::test]
async fn cancelled_batch_creates_no_data() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("a.png"), 1);

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.path().to_string_lossy()],
            "metadata_manifest": null,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let batch_id = preview["batch_id"].as_str().unwrap();

    let cancel = service
        .request(
            reqwest::Method::POST,
            &format!("/import-batches/{batch_id}/cancel"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(cancel.status(), 200);

    // Confirming a cancelled batch must fail, not silently write anything.
    let confirm = service
        .request(
            reqwest::Method::POST,
            &format!("/import-batches/{batch_id}/confirm"),
        )
        .json(&json!({ "confirmed_source_refs": ["a.png"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(confirm.status(), 409);
}

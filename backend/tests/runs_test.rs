mod common;

use serde_json::json;

fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

async fn import_one_image(service: &common::TestService, dir: &std::path::Path) -> (String, String) {
    write_png(&dir.join("normal.png"), 77);

    let preview: serde_json::Value = service
        .request(reqwest::Method::POST, "/import-batches")
        .json(&json!({
            "source_selection": "folder_scan",
            "paths": [dir.to_string_lossy()],
            "metadata_manifest": null,
            "dataset_display_name": "Run Fixture"
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

fn simple_pipeline() -> serde_json::Value {
    json!({
        "nodes": [
            {"node_id": "source", "node_type": "source"},
            {"node_id": "blur", "node_type": "gaussian", "canonical_parameters": {"sigma": 2}}
        ],
        "edges": [{"from": "source", "to": "blur"}]
    })
}

/// `POST /runs` returns immediately with `running`; completion is only
/// confirmed via `GET /runs/{id}` (contracts/local-service-api.md §Official
/// Runs) — never inferred from the POST response itself.
async fn post_run(
    service: &common::TestService,
    version_id: &str,
    image_id: &str,
    pipeline: serde_json::Value,
    target_node_id: &str,
    seed: u64,
) -> String {
    let created: serde_json::Value = service
        .request(reqwest::Method::POST, "/runs")
        .json(&json!({
            "dataset_version_id": version_id,
            "image_asset_id": image_id,
            "pipeline_snapshot": pipeline,
            "target_node_id": target_node_id,
            "seed": seed,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["status"], "running");
    created["run_id"].as_str().unwrap().to_string()
}

async fn wait_until_terminal(service: &common::TestService, run_id: &str) -> serde_json::Value {
    for _ in 0..100 {
        let record: serde_json::Value = service
            .request(reqwest::Method::GET, &format!("/runs/{run_id}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if record["status"] != "running" {
            return record;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("run {run_id} never left the `running` state");
}

/// quickstart.md §4.1 / SC-006: two Runs with identical dataset version,
/// image, pipeline, and seed resolve to the same output content identity.
#[tokio::test]
async fn identical_input_runs_resolve_to_same_output_identity() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, image_id) = import_one_image(&service, dir.path()).await;

    let first_id = post_run(
        &service,
        &version_id,
        &image_id,
        simple_pipeline(),
        "blur",
        42,
    )
    .await;
    let first = wait_until_terminal(&service, &first_id).await;
    assert_eq!(first["status"], "succeeded");

    let second_id = post_run(
        &service,
        &version_id,
        &image_id,
        simple_pipeline(),
        "blur",
        42,
    )
    .await;
    let second = wait_until_terminal(&service, &second_id).await;
    assert_eq!(second["status"], "succeeded");

    assert_eq!(
        first["output_artifact_refs"][0]["content_identity"],
        second["output_artifact_refs"][0]["content_identity"]
    );
}

/// quickstart.md §4.3 / FR-022 / SC-004: from a succeeded Run, every link
/// in `MetricSet → RunRecord → {DatasetVersion, RunInputArtifact →
/// ImageAsset, PipelineSnapshot}` resolves.
#[tokio::test]
async fn successful_run_traceability_chain_resolves() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, image_id) = import_one_image(&service, dir.path()).await;

    let run_id = post_run(
        &service,
        &version_id,
        &image_id,
        simple_pipeline(),
        "blur",
        7,
    )
    .await;
    let run = wait_until_terminal(&service, &run_id).await;

    assert_eq!(run["status"], "succeeded");
    assert!(run["metric_set"].is_object());
    assert_eq!(run["metric_set"]["run_record_id"], run_id);
    assert_eq!(run["dataset_version_id"], version_id);
    assert_eq!(run["image_asset_id"], image_id);
    assert!(run["run_input_artifact_id"].is_string());
    assert!(run["pipeline_snapshot_id"].is_string());
    assert!(!run["dataset_fingerprint"].as_str().unwrap().is_empty());

    // DatasetVersion leg of the chain.
    let dataset_id = only_dataset_id(&service).await;
    let versions: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/datasets/{dataset_id}/versions"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(versions["versions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == version_id));

    // ImageAsset leg of the chain.
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
}

/// The only Dataset in this fixture project.
async fn only_dataset_id(service: &common::TestService) -> String {
    let datasets: serde_json::Value = service
        .request(reqwest::Method::GET, "/datasets")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    datasets["datasets"][0]["id"].as_str().unwrap().to_string()
}

/// quickstart.md §4.4: a forced Run failure shows `status: failed`, a
/// `failed_stage`, and a redacted `error_summary` — never a fake success.
#[tokio::test]
async fn failed_run_shows_redacted_error_never_fake_success() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, image_id) = import_one_image(&service, dir.path()).await;

    let run_id = post_run(
        &service,
        &version_id,
        &image_id,
        simple_pipeline(),
        "does-not-exist",
        1,
    )
    .await;
    let run = wait_until_terminal(&service, &run_id).await;

    assert_eq!(run["status"], "failed");
    assert!(run["ended_at"].is_string());
    assert!(run["failed_stage"].is_string());
    let summary = run["error_summary"].as_str().unwrap();
    assert!(summary.contains("does-not-exist"));
    // Redaction: never echoes the image's own content identity or file path.
    assert!(!summary.contains("normal.png"));
    assert!(run["output_artifact_refs"].as_array().unwrap().is_empty());
    assert!(run["metric_set"].is_null());
}

/// `GET /runs?dataset_version_id=&image_asset_id=` filters the history list.
#[tokio::test]
async fn list_runs_filters_by_dataset_version_and_image() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let (version_id, image_id) = import_one_image(&service, dir.path()).await;

    let run_id = post_run(
        &service,
        &version_id,
        &image_id,
        simple_pipeline(),
        "blur",
        3,
    )
    .await;
    wait_until_terminal(&service, &run_id).await;

    let filtered: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/runs?dataset_version_id={version_id}&image_asset_id={image_id}"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let runs = filtered["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["id"], run_id);

    let other_image_id = uuid::Uuid::new_v4().to_string();
    let empty: serde_json::Value = service
        .request(
            reqwest::Method::GET,
            &format!("/runs?dataset_version_id={version_id}&image_asset_id={other_image_id}"),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(empty["runs"].as_array().unwrap().is_empty());
}

mod common;

use reqwest::Method;
use serde_json::json;

/// quickstart.md §3.2 / FR-029: clearing the cache reports what needs
/// recomputation, leaves official data alone, and the next preview is a
/// recoverable miss rather than an error.
#[tokio::test]
async fn cache_clear_leaves_official_data_and_reports_recompute() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 40);
    let (_, version_id) = service.import_dir(dir.path(), "Cache").await;
    let image_id = service.image_ids(&version_id).await.remove(0);
    let run = service.run_to_completion(&version_id, &image_id, 1).await;
    assert_eq!(run["status"], "succeeded");

    let preview_body = json!({
        "image_asset_id": image_id,
        "pipeline_snapshot": common::TestService::blur_pipeline(),
        "target_node_id": "blur",
    });
    let (_, first) = service.json(Method::POST, "/preview", Some(preview_body.clone())).await;
    assert_eq!(first["reused"], false);
    let (_, second) = service.json(Method::POST, "/preview", Some(preview_body.clone())).await;
    assert_eq!(second["reused"], true);
    // Provoke a thumbnail so the thumbnail cache has something to drop.
    service
        .request(Method::GET, &format!("/image-assets/{image_id}/thumbnail"))
        .send()
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let (status, cleared) = service.json(Method::POST, "/cache/clear", None).await;
    assert_eq!(status, 200);
    assert_eq!(cleared["cleared_preview_entries"], 1);
    let recompute = cleared["requires_recomputation"].as_array().unwrap();
    assert!(recompute.iter().any(|r| r["kind"] == "preview"
        && r["image_asset_id"] == json!(image_id)
        && r["target_node_id"] == "blur"));

    // Official data untouched: the Run still resolves with its output.
    let run_id = run["id"].as_str().unwrap();
    let (status, after) = service.json(Method::GET, &format!("/runs/{run_id}"), None).await;
    assert_eq!(status, 200);
    assert_eq!(after["status"], "succeeded");
    let output = after["output_artifact_refs"][0]["id"].as_str().unwrap();
    let content = service
        .request(Method::GET, &format!("/artifacts/{output}/content"))
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), 200);
    assert_eq!(service.image_ids(&version_id).await, vec![image_id.clone()]);

    // Recoverable miss: same request recomputes, and yields the same content.
    let (status, third) = service.json(Method::POST, "/preview", Some(preview_body)).await;
    assert_eq!(status, 200);
    assert_eq!(third["reused"], false);
    assert_eq!(
        third["artifact_ref"]["content_identity"],
        first["artifact_ref"]["content_identity"]
    );
}

/// FR-030: removal is two-step and leaves no resolvable orphan behind.
#[tokio::test]
async fn deleting_a_run_needs_confirmation_and_leaves_no_resolvable_output() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 50);
    let (_, version_id) = service.import_dir(dir.path(), "Runs").await;
    let image_id = service.image_ids(&version_id).await.remove(0);
    let run = service.run_to_completion(&version_id, &image_id, 3).await;
    let run_id = run["id"].as_str().unwrap();
    let output = run["output_artifact_refs"][0]["id"].as_str().unwrap();

    for query in ["", "?dry_run=true"] {
        let (status, body) = service
            .json(Method::DELETE, &format!("/runs/{run_id}{query}"), None)
            .await;
        assert_eq!(status, 200);
        assert_eq!(body["removed"], false);
        assert_eq!(body["would_invalidate"][0]["kind"], "metric_set");
        let (status, _) = service.json(Method::GET, &format!("/runs/{run_id}"), None).await;
        assert_eq!(status, 200, "nothing may be removed without confirm=true");
    }

    let (status, body) = service
        .json(Method::DELETE, &format!("/runs/{run_id}?confirm=true"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["removed"], true);

    let (status, _) = service.json(Method::GET, &format!("/runs/{run_id}"), None).await;
    assert_eq!(status, 404);
    let stale = service
        .request(Method::GET, &format!("/artifacts/{output}/content"))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 404, "a deleted Run's output must not resolve");
    let (status, _) = service
        .json(Method::DELETE, &format!("/runs/{run_id}?confirm=true"), None)
        .await;
    assert_eq!(status, 404);
    // The dataset the Run pointed at is unaffected.
    assert_eq!(service.image_ids(&version_id).await.len(), 1);
}

#[tokio::test]
async fn deleting_a_dataset_version_lists_relations_and_refuses_when_derived_versions_exist() {
    let service = common::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    common::write_png(&dir.path().join("a.png"), 60);
    let (dataset_id, v1) = service.import_dir(dir.path(), "Versions").await;

    common::write_png(&dir.path().join("b.png"), 61);
    let (same_dataset, v2) = service.import_dir(dir.path(), "Versions").await;
    assert_eq!(dataset_id, same_dataset);
    assert_ne!(v1, v2);

    let v1_images = service.image_ids(&v1).await;
    let v2_images = service.image_ids(&v2).await;
    let new_image = v2_images.iter().find(|i| !v1_images.contains(i)).unwrap().clone();
    let run = service.run_to_completion(&v2, &new_image, 9).await;
    let run_id = run["id"].as_str().unwrap().to_string();

    // v1 has a derived version: dry run explains, confirm is refused.
    let (_, dry) = service
        .json(Method::DELETE, &format!("/dataset-versions/{v1}?dry_run=true"), None)
        .await;
    assert_eq!(dry["removed"], false);
    assert_eq!(dry["blocked_by"][0]["id"], json!(v2));
    let (status, refused) = service
        .json(Method::DELETE, &format!("/dataset-versions/{v1}?confirm=true"), None)
        .await;
    assert_eq!(status, 409);
    assert_eq!(refused["error"]["code"], "conflict");

    // v2 (the leaf): the Run and its exclusively-owned image are listed...
    let (_, dry) = service
        .json(Method::DELETE, &format!("/dataset-versions/{v2}"), None)
        .await;
    let listed: Vec<(String, String)> = dry["would_invalidate"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["kind"].as_str().unwrap().into(), a["id"].as_str().unwrap().into()))
        .collect();
    assert!(listed.contains(&("run".into(), run_id.clone())));
    assert!(listed.contains(&("image_asset".into(), new_image.clone())));
    assert!(!listed.iter().any(|(_, id)| v1_images.contains(id)), "v1's images are shared");
    assert_eq!(service.image_ids(&v2).await.len(), 2, "dry run must not remove anything");

    // ...and only on confirm are they removed.
    let (status, done) = service
        .json(Method::DELETE, &format!("/dataset-versions/{v2}?confirm=true"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(done["removed"], true);

    let (_, versions) = service
        .json(Method::GET, &format!("/datasets/{dataset_id}/versions"), None)
        .await;
    assert_eq!(versions["versions"].as_array().unwrap().len(), 1);
    let (_, datasets) = service.json(Method::GET, "/datasets", None).await;
    assert_eq!(datasets["datasets"][0]["latest_version_id"], json!(v1));
    let (status, _) = service.json(Method::GET, &format!("/runs/{run_id}"), None).await;
    assert_eq!(status, 404);
    let (status, _) = service
        .json(Method::GET, &format!("/image-assets/{new_image}/display"), None)
        .await;
    assert_eq!(status, 404);
    // v1 is intact and now deletable.
    assert_eq!(service.image_ids(&v1).await, v1_images);
    let (status, _) = service
        .json(Method::DELETE, &format!("/dataset-versions/{v1}?confirm=true"), None)
        .await;
    assert_eq!(status, 200);
    let (_, datasets) = service.json(Method::GET, "/datasets", None).await;
    assert!(datasets["datasets"][0]["latest_version_id"].is_null());
}

#[tokio::test]
async fn deleting_unknown_entities_is_not_found() {
    let service = common::spawn().await;
    let id = uuid::Uuid::new_v4();
    for path in [format!("/runs/{id}?confirm=true"), format!("/dataset-versions/{id}?confirm=true")] {
        let (status, body) = service.json(Method::DELETE, &path, None).await;
        assert_eq!(status, 404);
        assert_eq!(body["error"]["code"], "not_found");
    }
}

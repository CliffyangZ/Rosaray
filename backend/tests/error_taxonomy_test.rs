//! Every error a handler returns must use the contract's
//! `{ "error": { "code", "message" } }` shape with a documented code.

mod common;

use reqwest::Method;
use rosaray_service::api::errors::ALL_CODES;
use serde_json::json;

fn assert_contract_error(body: &serde_json::Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "{body}");
    assert!(ALL_CODES.contains(&code));
    assert!(
        body["error"]["message"].as_str().is_some_and(|m| !m.is_empty()),
        "missing message: {body}"
    );
}

#[tokio::test]
async fn handler_errors_follow_the_contract_shape() {
    let service = common::spawn().await;
    let id = uuid::Uuid::new_v4();

    // access_denied: no session header, on a real route.
    let denied = service
        .client()
        .get(format!("{}/datasets", service.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    assert_contract_error(&denied.json().await.unwrap(), "access_denied");

    let not_found_paths = [
        (Method::GET, format!("/image-assets/{id}/display")),
        (Method::GET, format!("/runs/{id}")),
        (Method::GET, format!("/artifacts/{id}/content")),
        (Method::DELETE, format!("/runs/{id}?confirm=true")),
        (Method::DELETE, format!("/dataset-versions/{id}?confirm=true")),
        (Method::GET, format!("/export-bundles/{id}/content")),
    ];
    for (method, path) in not_found_paths {
        let (status, body) = service.json(method.clone(), &path, None).await;
        assert_eq!(status, 404, "{method} {path}");
        assert_contract_error(&body, "not_found");
    }

    let (status, body) = service
        .json(Method::POST, "/export-bundles", Some(json!({ "dataset_version_ids": [id] })))
        .await;
    assert_eq!(status, 401);
    assert_contract_error(&body, "credential_required");

    let (status, body) = service
        .json(
            Method::POST,
            "/export-bundles",
            Some(json!({ "dataset_version_ids": [id], "credential": "c" })),
        )
        .await;
    assert_eq!(status, 404);
    assert_contract_error(&body, "not_found");

    let (status, body) = service
        .json(Method::POST, &format!("/import-batches/{id}/cancel"), None)
        .await;
    assert!(status.is_client_error());
    assert_contract_error(&body, body["error"]["code"].as_str().unwrap());
}

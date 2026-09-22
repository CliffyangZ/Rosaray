mod common;

#[tokio::test]
async fn get_session_returns_ready() {
    let service = common::spawn().await;

    let response = service
        .request(reqwest::Method::GET, "/session")
        .send()
        .await
        .expect("request failed");

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["state"], "ready");
}

#[tokio::test]
async fn missing_session_header_is_denied() {
    let service = common::spawn().await;

    let response = service
        .client()
        .get(format!("{}/session", service.base_url))
        .send()
        .await
        .expect("request failed");

    assert_eq!(response.status(), 401);
}

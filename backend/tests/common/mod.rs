use std::sync::{Arc, Mutex, RwLock};

use rosaray_service::api::session::SessionState;
use rosaray_service::api::{AppState, AppStateInner};
use rosaray_service::crypto::MasterKey;
use rosaray_service::data_repository::blob_store::BlobStore;
use rosaray_service::data_repository::sqlite;

/// An in-process service instance backed by a temp SQLite file and temp
/// blob directory, per quickstart.md Prerequisites. Kept alive by holding
/// onto the returned `TestService` (its tempdir is dropped, and cleaned up,
/// when it goes out of scope).
pub struct TestService {
    pub base_url: String,
    pub session_token: String,
    _project_dir: tempfile::TempDir,
    _server: tokio::task::JoinHandle<()>,
}

pub async fn spawn() -> TestService {
    let project_dir = tempfile::tempdir().expect("failed to create temp project dir");

    let salt = rosaray_service::crypto::generate_salt();
    let master_key = MasterKey::derive("test-passphrase", &salt).expect("key derivation failed");

    let db = sqlite::open(&project_dir.path().join("rosaray.sqlite3"), &master_key)
        .expect("failed to open/migrate test database");
    let project_id =
        sqlite::ensure_default_project(&db).expect("failed to bootstrap test project row");
    let blob_store = BlobStore::new(project_dir.path().join("blobs"))
        .expect("failed to initialize test blob store");

    let session_token = "test-session-token".to_string();
    let (event_tx, _rx) = rosaray_service::events::new_channel();

    let state = AppState(Arc::new(AppStateInner {
        session_token: session_token.clone(),
        session_state: RwLock::new(SessionState::Ready),
        db: Mutex::new(db),
        blob_store,
        master_key,
        event_tx,
        project_id,
        exports_dir: project_dir.path().join("exports"),
        pending_batches: Default::default(),
        pending_batch_paths: Default::default(),
        artifact_registry: Default::default(),
        pending_requests: Default::default(),
        preview_cache: Default::default(),
        preview_isolation: Default::default(),
    }));

    let router = rosaray_service::api::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server exited unexpectedly");
    });

    TestService {
        base_url: format!("http://127.0.0.1:{port}"),
        session_token,
        _project_dir: project_dir,
        _server: server,
    }
}

impl TestService {
    pub fn client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    pub fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client()
            .request(method, format!("{}{}", self.base_url, path))
            .header("X-Rosaray-Session", &self.session_token)
    }
}

// ---- shared fixture helpers (used by the quickstart-level test files) ----

#[allow(dead_code)]
pub fn write_png(path: &std::path::Path, seed: u8) {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([seed, seed, seed]));
    image::DynamicImage::ImageRgb8(img).save(path).unwrap();
}

#[allow(dead_code)]
impl TestService {
    pub async fn json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let mut req = self.request(method, path);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let body = resp.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    /// Scans `dir` into `dataset_name` and confirms every importable
    /// candidate; returns `(dataset_id, dataset_version_id)`.
    pub async fn import_dir(&self, dir: &std::path::Path, dataset_name: &str) -> (String, String) {
        let (_, preview) = self
            .json(
                reqwest::Method::POST,
                "/import-batches",
                Some(serde_json::json!({
                    "source_selection": "folder_scan",
                    "paths": [dir.to_string_lossy()],
                    "metadata_manifest": null,
                    "dataset_display_name": dataset_name,
                })),
            )
            .await;
        let refs: Vec<String> = preview["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["classification"] == "importable")
            .map(|c| c["source_ref"].as_str().unwrap().to_string())
            .collect();
        let batch_id = preview["batch_id"].as_str().unwrap();
        let (status, confirm) = self
            .json(
                reqwest::Method::POST,
                &format!("/import-batches/{batch_id}/confirm"),
                Some(serde_json::json!({ "confirmed_source_refs": refs })),
            )
            .await;
        assert_eq!(status, 200, "{confirm}");
        (
            confirm["dataset_id"].as_str().unwrap().to_string(),
            confirm["dataset_version_id"].as_str().unwrap().to_string(),
        )
    }

    pub async fn image_ids(&self, version_id: &str) -> Vec<String> {
        let (_, images) = self
            .json(
                reqwest::Method::GET,
                &format!("/dataset-versions/{version_id}/images"),
                None,
            )
            .await;
        images["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    }

    pub fn blur_pipeline() -> serde_json::Value {
        serde_json::json!({
            "nodes": [
                {"node_id": "source", "node_type": "source"},
                {"node_id": "blur", "node_type": "gaussian", "canonical_parameters": {"sigma": 2}}
            ],
            "edges": [{"from": "source", "to": "blur"}]
        })
    }

    /// Starts an official Run and waits for it to leave `running`.
    pub async fn run_to_completion(
        &self,
        version_id: &str,
        image_id: &str,
        seed: u64,
    ) -> serde_json::Value {
        let (_, created) = self
            .json(
                reqwest::Method::POST,
                "/runs",
                Some(serde_json::json!({
                    "dataset_version_id": version_id,
                    "image_asset_id": image_id,
                    "pipeline_snapshot": Self::blur_pipeline(),
                    "target_node_id": "blur",
                    "seed": seed,
                })),
            )
            .await;
        let run_id = created["run_id"].as_str().unwrap().to_string();
        for _ in 0..200 {
            let (_, run) = self
                .json(reqwest::Method::GET, &format!("/runs/{run_id}"), None)
                .await;
            if run["status"] != "running" {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("run {run_id} never left `running`");
    }
}

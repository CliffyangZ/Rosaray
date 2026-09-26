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
    /// The service's own state, so tests can reach its database and KB folders.
    pub state: AppState,
    pub kb_root: std::path::PathBuf,
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

    let kb_root = project_dir.path().join("knowledge-base");
    rosaray_service::kb::ensure_layout(&kb_root).expect("failed to create knowledge-base folders");

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
        kb_root: kb_root.clone(),
        pending_batches: Default::default(),
        pending_batch_paths: Default::default(),
        artifact_registry: Default::default(),
        pending_requests: Default::default(),
        preview_cache: Default::default(),
        preview_isolation: Default::default(),
        preview_nodes: Default::default(),
        preview_board: Default::default(),
        preview_delays: Default::default(),
        extractions: Default::default(),
        extraction_delay_ms: Default::default(),
    }));

    let router = rosaray_service::api::build_router(state.clone());

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
        state,
        kb_root,
        _project_dir: project_dir,
        _server: server,
    }
}

impl TestService {
    /// The service's temp project directory (blobs, database, knowledge base).
    pub fn _project_dir_path(&self) -> std::path::PathBuf {
        self._project_dir.path().to_path_buf()
    }

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

// ---- Knowledge-base helpers (feature 002, T005) ----

/// A fresh, empty `knowledge-base/` root inside a temp dir. Keep the returned
/// `TempDir` alive for the duration of the test; the root is `<tmp>/knowledge-base`.
#[allow(dead_code)]
pub fn kb_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp kb dir");
    let root = dir.path().join("knowledge-base");
    rosaray_service::kb::ensure_layout(&root).expect("failed to create kb layout");
    (dir, root)
}

/// Writes `files` (relative path → contents) under `dir`, creating parent dirs.
#[allow(dead_code)]
pub fn write_bundle(dir: &std::path::Path, files: &[(&str, &str)]) {
    for (rel, contents) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).expect("failed to create bundle dir");
        std::fs::write(path, contents).expect("failed to write bundle file");
    }
}

/// BLAKE3 over every file under `dir` as sorted `(relative_path, file_hash)`
/// pairs — used for "published bundles unchanged" assertions.
#[allow(dead_code)]
pub fn hash_tree(dir: &std::path::Path) -> String {
    fn walk(base: &std::path::Path, cur: &std::path::Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(cur)
            .expect("failed to read dir")
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                let h = blake3::hash(&std::fs::read(&p).expect("failed to read file"));
                out.push((rel, h.to_hex().to_string()));
            }
        }
    }
    let mut pairs = Vec::new();
    walk(dir, dir, &mut pairs);
    let mut hasher = blake3::Hasher::new();
    for (rel, h) in pairs {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(h.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

// ---- Knowledge-base test support (feature 002) ------------------------------

#[allow(dead_code)]
pub fn fixture_dir(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kb").join(name)
}

#[allow(dead_code)]
pub fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// Writes a *published* bundle (files + a correct `bundle.lock`) directly,
/// bypassing the publish workflow, and returns its `content_id`. For building
/// scenarios the workflow cannot yet (or should not) produce.
#[allow(dead_code)]
pub fn make_published_bundle(
    root: &std::path::Path,
    kind: &str,
    id: &str,
    version: &str,
    release_kind: &str,
    files: &[(&str, &str)],
) -> String {
    let sub = if kind == "algopipe" { "pipes" } else { "nodes" };
    let dir = root.join(sub).join(id).join(version);
    write_bundle(&dir, files);
    let hashes = rosaray_service::kb::identity::file_hashes(&dir).unwrap();
    let content_id = rosaray_service::kb::identity::content_id_of(hashes.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let mut lock = format!(
        "schema: quantify-kb/1\nid: {id}\nversion: {version}\ncontent_id: \"{content_id}\"\nrelease_kind: {release_kind}\nfiles:\n"
    );
    for (p, h) in &hashes {
        lock.push_str(&format!("  - {{ path: {p}, blake3: \"{}\" }}\n", h.trim_start_matches("b3:")));
    }
    std::fs::write(dir.join("bundle.lock"), lock).unwrap();
    content_id
}

/// A tiny grayscale PNG whose pixels depend on `px` (distinct bytes per value).
#[allow(dead_code)]
pub fn png_bytes(px: u8) -> Vec<u8> {
    let img = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(6, 6, image::Luma([px])));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[allow(dead_code)]
impl TestService {
    /// Installs the six seed nodes (as a fresh service would on first start).
    pub fn install_seeds(&self) -> usize {
        let db = self.state.db.lock().unwrap();
        rosaray_service::kb::seed::install(&db, &self.kb_root).unwrap()
    }

    /// Incremental catalog refresh, as `POST /kb/refresh` does.
    pub fn refresh_catalog(&self) {
        let db = self.state.db.lock().unwrap();
        rosaray_service::kb::catalog::repo::refresh(&db, &self.kb_root, &mut |_| {}).unwrap();
    }

    pub async fn entries(&self, query: &str) -> Vec<serde_json::Value> {
        let (status, body) = self.json(reqwest::Method::GET, &format!("/kb/entries{query}"), None).await;
        assert_eq!(status, 200, "{body}");
        body["entries"].as_array().unwrap().clone()
    }

    /// `content_id` of a published bundle, straight from the catalog.
    pub fn content_id_of_published(&self, id: &str, version: &str) -> String {
        let db = self.state.db.lock().unwrap();
        db.query_row(
            "SELECT content_id FROM kb_catalog_entry WHERE id = ?1 AND version = ?2 AND status = 'published'",
            [id, version],
            |r| r.get(0),
        )
        .unwrap()
    }

    pub async fn upload_algobundle(&self, bytes: Vec<u8>) -> (reqwest::StatusCode, serde_json::Value) {
        let part = reqwest::multipart::Part::bytes(bytes).file_name("x.algobundle");
        let form = reqwest::multipart::Form::new().part("bundle", part);
        let resp = self
            .request(reqwest::Method::POST, "/kb/import")
            .multipart(form)
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(serde_json::Value::Null))
    }

    /// Creates a draft from a fixture folder through the API (create + save).
    pub async fn create_draft_from_fixture(&self, kind: &str, id: &str, fixture: &str) -> String {
        let (status, created) = self
            .json(reqwest::Method::POST, &format!("/kb/{kind}/drafts"), Some(serde_json::json!({ "id": id })))
            .await;
        assert_eq!(status, 201, "{created}");
        let mut files = serde_json::Map::new();
        let dir = fixture_dir(fixture);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            files.insert(
                entry.file_name().to_string_lossy().to_string(),
                serde_json::Value::String(std::fs::read_to_string(entry.path()).unwrap()),
            );
        }
        let (status, saved) = self
            .json(
                reqwest::Method::PUT,
                &format!("/kb/{kind}/drafts/{id}"),
                Some(serde_json::json!({ "base_revision": created["revision"], "files": files })),
            )
            .await;
        assert_eq!(status, 200, "{saved}");
        saved["revision"].as_str().unwrap().to_string()
    }
}

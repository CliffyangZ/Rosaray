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

    let db = sqlite::open(&project_dir.path().join("rosaray.sqlite3"))
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

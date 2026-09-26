pub mod algopipe_run;
pub mod artifacts;
pub mod cache;
pub mod deletion;
pub mod designer;
pub mod errors;
pub mod explorer;
pub mod export;
pub mod import;
pub mod kb;
pub mod papers;
pub mod pipe_preview;
pub mod preview;
pub mod runs;
pub mod session;

use axum::extract::{Path, State};
use axum::http::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get};
use axum::Router;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use uuid::Uuid;

use crate::crypto::MasterKey;
use crate::data_engine::preview::RequestIsolation;
use crate::data_repository::blob_store::BlobStore;
use crate::data_repository::memory_cache::PreviewCache;
use crate::domain::{ArtifactKind, ArtifactReference, ServiceError};
use session::SessionState;

/// Per-batch import scratch state: `source_ref` → absolute path, plus the
/// dataset display name the batch was previewed under.
pub type PendingBatchPaths = Mutex<HashMap<Uuid, (HashMap<String, PathBuf>, String)>>;

pub struct AppStateInner {
    pub session_token: String,
    pub session_state: RwLock<SessionState>,
    pub db: Mutex<rusqlite::Connection>,
    pub blob_store: BlobStore,
    pub master_key: MasterKey,
    pub event_tx: tokio::sync::broadcast::Sender<crate::events::Event>,
    pub project_id: Uuid,
    /// Where finished Export Bundles (ciphertext only) are written (FR-026).
    pub exports_dir: PathBuf,
    /// Root of the plain-file Knowledge Base tree, `<project>/knowledge-base/`
    /// (feature 002, research §5). The derived catalog lives in `db`.
    pub kb_root: PathBuf,
    pub pending_batches: import::PendingBatches,
    pub pending_batch_paths: PendingBatchPaths,
    /// Maps a short-lived `ArtifactReference.id` back to the actual content
    /// identity + kind it stands for, so `GET /artifacts/{id}/content` can
    /// resolve it — `ArtifactReference` is composed on request, not a
    /// stored table (data-model.md).
    pub artifact_registry: Mutex<HashMap<Uuid, (String, ArtifactKind)>>,
    /// In-flight cancellable requests (thumbnail/display generation),
    /// keyed by the `request_id` handed back to the caller (FR-048).
    pub pending_requests: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    /// Volatile Preview cache, keyed by content-equivalence (FR-011/FR-013,
    /// User Story 3). Never persisted, never consulted by official Runs.
    pub preview_cache: PreviewCache,
    /// Tracks the latest in-flight Preview request per selection so a
    /// superseded completion never overwrites a fresher result (FR-017).
    pub preview_isolation: RequestIsolation,
    /// Lossless per-node Preview results for AlgoPipe drafts (feature 002).
    /// Memory only, like `preview_cache`; never consulted by official Runs.
    pub preview_nodes: crate::designer::preview::NodeCache,
    /// Per-pipe, per-node Preview status (idle/stale/queued/running/ready/…).
    pub preview_board: crate::designer::preview::PreviewBoard,
    /// Test hook: artificial delays (ms) applied to the next Preview requests,
    /// to exercise out-of-order completion (SC-004). Empty in production.
    pub preview_delays: Mutex<std::collections::VecDeque<u64>>,
    /// In-flight paper extractions by `extraction_id`, so one can be cancelled
    /// cooperatively (US5-6). Volatile.
    pub extractions: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    /// Test hook: per-page delay (ms) in an extraction pass, so tests can cancel
    /// one mid-flight. Zero in production.
    pub extraction_delay_ms: std::sync::atomic::AtomicU64,
}

impl AppStateInner {
    /// Deletes each blob no longer referenced by any official row or
    /// thumbnail, and forgets any `ArtifactReference` that resolved to it —
    /// so a removed result cannot still be fetched by a stale reference
    /// (FR-030). Best-effort: an I/O failure leaves an orphan file, never a
    /// dangling reference.
    pub fn remove_unreferenced_blobs(&self, candidates: Vec<String>) {
        use crate::data_repository::sqlite::housekeeping_repo as hk;
        let mut gone = Vec::new();
        {
            let db = self.db.lock().unwrap();
            for identity in candidates {
                let held = hk::content_is_official(&db, &identity).unwrap_or(true)
                    || hk::content_is_thumbnail(&db, &identity).unwrap_or(true);
                if !held && self.blob_store.delete(&identity).is_ok() {
                    gone.push(identity);
                }
            }
        }
        if !gone.is_empty() {
            self.artifact_registry
                .lock()
                .unwrap()
                .retain(|_, (identity, _)| !gone.contains(identity));
        }
    }

    pub fn register_artifact(
        &self,
        content_identity: String,
        kind: ArtifactKind,
    ) -> ArtifactReference {
        let id = Uuid::new_v4();
        self.artifact_registry
            .lock()
            .unwrap()
            .insert(id, (content_identity.clone(), kind));
        ArtifactReference {
            id,
            content_identity,
            kind,
        }
    }
}

#[derive(Clone)]
pub struct AppState(pub Arc<AppStateInner>);

impl std::ops::Deref for AppState {
    type Target = AppStateInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// `X-Rosaray-Session` auth boundary (contracts/local-service-api.md, FR-040):
/// a missing/invalid token is indistinguishable from "route does not exist"
/// beyond the shared `access_denied` error code — it never leaks whether the
/// underlying resource exists.
async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, ServiceError> {
    let provided = request
        .headers()
        .get("X-Rosaray-Session")
        .and_then(|v| v.to_str().ok());
    match provided {
        Some(token) if token == state.session_token => Ok(next.run(request).await),
        _ => Err(ServiceError::AccessDenied),
    }
}

/// `DELETE /requests/{request_id}` (FR-048): cancels an in-flight
/// thumbnail/display request. Missing/already-completed request ids are
/// treated as already-resolved, not an error — cancellation is inherently
/// racy against completion.
async fn cancel_request(
    State(state): State<AppState>,
    Path(request_id): Path<Uuid>,
) -> axum::Json<serde_json::Value> {
    if let Some(flag) = state.pending_requests.lock().unwrap().remove(&request_id) {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    axum::Json(serde_json::json!({ "request_id": request_id, "cancelled": true }))
}

/// Builds the full axum router: every route defined by any `api::*` submodule
/// merged together, with the session-token auth boundary applied to every
/// typed HTTP route. `/events` authenticates itself via a query parameter
/// instead (browsers cannot set a custom header on a WebSocket handshake),
/// so it is mounted outside this layer.
pub fn build_router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/session", get(session::get_session))
        .route("/requests/:request_id", delete(cancel_request))
        .merge(import::router())
        .merge(explorer::router())
        .merge(artifacts::router())
        .merge(preview::router())
        .merge(runs::router())
        .merge(export::router())
        .merge(cache::router())
        .merge(deletion::router())
        .merge(kb::router())
        .merge(designer::router())
        .merge(papers::router())
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let unprotected = Router::new().route("/events", get(crate::events::events_ws));

    // Permissive CORS: the service is loopback-bound and the real access
    // control is the `X-Rosaray-Session` token, not same-origin — the
    // frontend dev server and this service are on different localhost
    // ports and would otherwise be blocked by the browser's CORS check.
    protected
        .merge(unprotected)
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

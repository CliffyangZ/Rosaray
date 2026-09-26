//! HTTP surface of the QKB: three authorization scopes over one router.
//! `catalog:read` (frontend) → GET catalog; `author` → submissions and
//! maintenance; `system_one` → the `system-one/1` exchange.

pub mod author;
pub mod catalog;
pub mod errors;
pub mod system_one;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use rosaray_qkb::auth::{permits, Credentials, Scope, HEADER_AUTHOR, HEADER_CATALOG, HEADER_SYSTEM_ONE};

pub use errors::ApiError;

pub struct AppStateInner {
    pub creds: Credentials,
    pub db: Mutex<rusqlite::Connection>,
    pub kb_root: PathBuf,
}

#[derive(Clone)]
pub struct AppState(pub Arc<AppStateInner>);

impl std::ops::Deref for AppState {
    type Target = AppStateInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AppState {
    /// Runs blocking QKB work (file + SQLite I/O) off the async runtime.
    pub async fn blocking<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection, &std::path::Path) -> Result<T, ApiError> + Send + 'static,
    {
        let state = self.clone();
        tokio::task::spawn_blocking(move || {
            let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
            f(&db, &state.kb_root)
        })
        .await
        .map_err(|_| ApiError::internal("worker failed"))?
    }
}

/// The authenticated caller, attached by the auth layer.
#[derive(Clone, Debug)]
pub struct Caller {
    pub scope: Scope,
    /// Set for `SystemOne` callers: the source identity recorded with selections.
    pub source_id: Option<String>,
}

#[derive(Clone)]
struct Guard {
    state: AppState,
    need: Scope,
}

/// Each credential is only honoured in its own header (a frontend credential
/// sent as `X-Rosaray-Author` grants nothing). A missing or wrong credential
/// is indistinguishable from any other denial.
async fn auth_mw(State(g): State<Guard>, mut req: Request, next: Next) -> Result<Response, ApiError> {
    let mut granted: Option<Scope> = None;
    for name in [HEADER_CATALOG, HEADER_AUTHOR, HEADER_SYSTEM_ONE] {
        if let Some(v) = req.headers().get(name).and_then(|v| v.to_str().ok()) {
            if let Some(scope) = g.state.creds.scope_of(name, v) {
                // The most specific scope wins over catalog:read.
                if granted.is_none() || scope != Scope::CatalogRead {
                    granted = Some(scope);
                }
            }
        }
    }
    match granted {
        Some(have) if permits(have, g.need) => {
            let source_id = (have == Scope::SystemOne).then(|| g.state.creds.source_id.clone());
            req.extensions_mut().insert(Caller { scope: have, source_id });
            let mut resp = next.run(req).await;
            resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(resp)
        }
        _ => Err(ApiError::access_denied()),
    }
}

fn guarded(router: Router<AppState>, state: &AppState, need: Scope) -> Router<AppState> {
    router.layer(middleware::from_fn_with_state(Guard { state: state.clone(), need }, auth_mw))
}

pub fn build_router(state: AppState) -> Router {
    let read = Router::new()
        .route("/qkb/v1/session", get(catalog::session))
        .route("/qkb/v1/catalog/entries", get(catalog::entries))
        .route("/qkb/v1/catalog/:kind/:id", get(catalog::versions))
        .route("/qkb/v1/catalog/:kind/:id/:version", get(catalog::detail));
    let author = Router::new()
        .route("/qkb/v1/submissions", post(author::submit))
        .route("/qkb/v1/verification", post(author::verification))
        .route("/qkb/v1/trust", post(author::trust))
        .route("/qkb/v1/maintenance/rescan", post(author::rescan))
        .route("/qkb/v1/maintenance/rebuild-index", post(author::rebuild_index))
        .route("/qkb/v1/deletions", get(author::deletions));
    let so = Router::new()
        .route("/qkb/v1/system-one/queries", post(system_one::query))
        .route("/qkb/v1/system-one/selections", post(system_one::selection))
        .route("/qkb/v1/system-one/selections/:request_id", get(system_one::record));

    // Permissive CORS: the service is loopback-bound and access control is the
    // credential, not same-origin; the dev frontend is on another local port.
    guarded(read, &state, Scope::CatalogRead)
        .merge(guarded(author, &state, Scope::Author))
        .merge(guarded(so, &state, Scope::SystemOne))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

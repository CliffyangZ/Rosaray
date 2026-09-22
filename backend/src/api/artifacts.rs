//! `GET /artifacts/{artifact_id}/content` (contracts/local-service-api.md):
//! streamed binary content, re-verifying content identity before disclosure
//! (FR-023) — a mismatch returns `410 stale_reference` rather than stale or
//! wrong-identity bytes.

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::api::AppState;
use crate::domain::{ArtifactKind, ServiceError};

pub async fn get_artifact_content(
    State(state): State<AppState>,
    Path(artifact_id): Path<Uuid>,
) -> Result<impl IntoResponse, ServiceError> {
    let (content_identity, kind) = state
        .artifact_registry
        .lock()
        .unwrap()
        .get(&artifact_id)
        .cloned()
        .ok_or(ServiceError::NotFound)?;

    // `BlobStore::read` itself re-hashes the decrypted plaintext and
    // compares it against `content_identity` — any tamper or drift surfaces
    // here as an integrity error, which we map to the contract's
    // `stale_reference` (FR-023).
    let bytes = state
        .blob_store
        .read(&state.master_key, &content_identity)
        .map_err(|_| ServiceError::StaleReference)?;

    let content_type = match kind {
        ArtifactKind::Image | ArtifactKind::Mask => "image/png",
        ArtifactKind::Measurement | ArtifactKind::Other => "application/octet-stream",
    };

    Ok(([(header::CONTENT_TYPE, content_type)], bytes))
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new().route("/artifacts/:artifact_id/content", get(get_artifact_content))
}

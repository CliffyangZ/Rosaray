//! `POST /cache/clear` (contracts/local-service-api.md §Housekeeping,
//! FR-029): drops Preview and thumbnail cache only. Official data — Dataset
//! Versions, Runs, masks — is never read or written here, and a blob is
//! removed only if no official row still points at it (content addressing
//! means a Preview output can be byte-identical to an official Run output).

use axum::extract::State;
use axum::routing::post;
use axum::Json;
use serde::Serialize;
use uuid::Uuid;

use crate::api::AppState;
use crate::data_repository::sqlite::housekeeping_repo;
use crate::domain::ServiceError;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recompute {
    Preview {
        image_asset_id: Uuid,
        pipeline_snapshot_id: Uuid,
        target_node_id: String,
    },
    Thumbnail {
        image_asset_id: Uuid,
    },
}

#[derive(Serialize)]
pub struct ClearCacheResponse {
    pub cleared_preview_entries: usize,
    pub cleared_thumbnails: usize,
    pub requires_recomputation: Vec<Recompute>,
}

pub async fn post_cache_clear(
    State(state): State<AppState>,
) -> Result<Json<ClearCacheResponse>, ServiceError> {
    let entries = state.preview_cache.drain();
    let (thumb_images, thumb_blobs) = {
        let db = state.db.lock().unwrap();
        housekeeping_repo::clear_thumbnails(&db).map_err(|_| ServiceError::ServiceUnavailable)?
    };

    let mut candidates: Vec<String> = thumb_blobs;
    candidates.extend(entries.iter().map(|e| e.output_content_identity.clone()));
    state.remove_unreferenced_blobs(candidates);

    let mut requires_recomputation: Vec<Recompute> = entries
        .iter()
        .map(|e| Recompute::Preview {
            image_asset_id: e.image_asset_id,
            pipeline_snapshot_id: e.pipeline_snapshot_id,
            target_node_id: e.producing_node_id.clone(),
        })
        .collect();
    requires_recomputation.extend(
        thumb_images
            .iter()
            .map(|id| Recompute::Thumbnail { image_asset_id: *id }),
    );

    Ok(Json(ClearCacheResponse {
        cleared_preview_entries: entries.len(),
        cleared_thumbnails: thumb_images.len(),
        requires_recomputation,
    }))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/cache/clear", post(post_cache_clear))
}

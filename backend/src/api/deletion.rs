//! `DELETE /dataset-versions/{id}` and `DELETE /runs/{id}`
//! (contracts/local-service-api.md §Housekeeping, FR-030). Two-step by
//! design: without `confirm=true` nothing is removed and the caller gets
//! the list of relations the removal would invalidate; `dry_run=true`
//! states that intent explicitly.

use axum::extract::{Path, Query, State};
use axum::routing::delete;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::data_repository::sqlite::housekeeping_repo::{self, Affected};
use crate::domain::ServiceError;

#[derive(Deserialize, Default)]
pub struct DeleteParams {
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Serialize)]
pub struct DeleteResponse {
    pub would_invalidate: Vec<Affected>,
    /// Non-empty means removal is refused: these entities would be left
    /// pointing at something that no longer exists.
    pub blocked_by: Vec<Affected>,
    pub removed: bool,
}

fn unavailable<E>(_: E) -> ServiceError {
    ServiceError::ServiceUnavailable
}

pub async fn delete_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Query(params): Query<DeleteParams>,
) -> Result<Json<DeleteResponse>, ServiceError> {
    let (would_invalidate, candidates) = {
        let mut db = state.db.lock().unwrap();
        let status = housekeeping_repo::run_status(&db, run_id)
            .map_err(unavailable)?
            .ok_or(ServiceError::NotFound)?;
        // A running Run's executor would try to finalize a row that no
        // longer exists; it must finish (or fail) first.
        if status == "running" {
            return Err(ServiceError::Conflict);
        }
        let would_invalidate = housekeeping_repo::run_impact(&db, run_id).map_err(unavailable)?;
        if params.dry_run || !params.confirm {
            return Ok(Json(DeleteResponse {
                would_invalidate,
                blocked_by: vec![],
                removed: false,
            }));
        }
        let tx = db.transaction().map_err(unavailable)?;
        let candidates = housekeeping_repo::delete_run(&tx, run_id).map_err(unavailable)?;
        tx.commit().map_err(unavailable)?;
        (would_invalidate, candidates)
    };
    state.remove_unreferenced_blobs(candidates);
    Ok(Json(DeleteResponse {
        would_invalidate,
        blocked_by: vec![],
        removed: true,
    }))
}

pub async fn delete_dataset_version(
    State(state): State<AppState>,
    Path(version_id): Path<Uuid>,
    Query(params): Query<DeleteParams>,
) -> Result<Json<DeleteResponse>, ServiceError> {
    let (would_invalidate, candidates) = {
        let mut db = state.db.lock().unwrap();
        if !housekeeping_repo::version_exists(&db, version_id).map_err(unavailable)? {
            return Err(ServiceError::NotFound);
        }
        let impact = housekeeping_repo::version_impact(&db, version_id).map_err(unavailable)?;
        let would_invalidate = impact.would_invalidate();
        let blocked_by = impact.blocked_by();

        if params.dry_run || !params.confirm {
            return Ok(Json(DeleteResponse {
                would_invalidate,
                blocked_by,
                removed: false,
            }));
        }
        if !blocked_by.is_empty() {
            return Err(ServiceError::Conflict);
        }
        // Same reasoning as `delete_run`: never yank a Run out from under
        // its executor.
        for run in &impact.runs {
            let run = Uuid::parse_str(run).expect("stored UUID");
            if housekeeping_repo::run_status(&db, run).map_err(unavailable)?.as_deref()
                == Some("running")
            {
                return Err(ServiceError::Conflict);
            }
        }
        let tx = db.transaction().map_err(unavailable)?;
        let candidates =
            housekeeping_repo::delete_version(&tx, version_id, &impact).map_err(unavailable)?;
        tx.commit().map_err(unavailable)?;
        (would_invalidate, candidates)
    };
    state.remove_unreferenced_blobs(candidates);
    Ok(Json(DeleteResponse {
        would_invalidate,
        blocked_by: vec![],
        removed: true,
    }))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/dataset-versions/:version_id", delete(delete_dataset_version))
        .route("/runs/:run_id", delete(delete_run))
}

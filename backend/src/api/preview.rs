//! `POST /preview` (contracts/local-service-api.md §Preview, User Story 3).
//! Content-equivalence cache reuse/miss (FR-014/FR-015), lineage
//! diagnostics (FR-016), and a stale/failed fallback to the selection's
//! last successful result with the failing node identified (FR-017). Never
//! creates a `RunRecord` (FR-013).

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::data_engine::preview::{run_preview, LastSuccessful, PreviewRequest};
use crate::data_engine::validation::check_source_availability;
use crate::data_repository::sqlite::dataset_repo;
use crate::domain::dataset::ImageAssetStatus;
use crate::domain::pipeline_snapshot::PipelineGraph;
use crate::domain::{ArtifactReference, ServiceError};
use crate::events::{Event, EventPayload};

#[derive(Deserialize)]
pub struct PreviewRequestBody {
    pub image_asset_id: Uuid,
    pub pipeline_snapshot: PipelineGraph,
    pub target_node_id: String,
}

#[derive(Serialize)]
pub struct PreviewErrorPayload {
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum PreviewResponse {
    Ready {
        artifact_ref: ArtifactReference,
        reused: bool,
        request_context_id: Uuid,
    },
    Stale {
        state: &'static str,
        error: PreviewErrorPayload,
        failing_node_id: String,
        last_successful_artifact_ref: Option<ArtifactReference>,
    },
}

pub async fn post_preview(
    State(state): State<AppState>,
    Json(body): Json<PreviewRequestBody>,
) -> Result<Json<PreviewResponse>, ServiceError> {
    let asset = {
        let db = state.db.lock().unwrap();
        dataset_repo::image_asset_by_id(&db, body.image_asset_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
            .ok_or(ServiceError::NotFound)?
    };

    // FR-031: a missing/changed external source blocks new Preview creation
    // the same way it blocks a new official Run.
    match check_source_availability(&asset) {
        ImageAssetStatus::Available => {}
        ImageAssetStatus::SourceMissing => return Err(ServiceError::SourceMissing),
        ImageAssetStatus::SourceChanged => return Err(ServiceError::SourceChanged),
    }

    let req = PreviewRequest {
        image_asset_id: body.image_asset_id,
        source_content_identity: &asset.imported_content_identity,
        graph: &body.pipeline_snapshot,
        target_node_id: &body.target_node_id,
    };

    let outcome = run_preview(
        &state.preview_cache,
        &state.preview_isolation,
        |bytes| {
            state
                .blob_store
                .write(&state.master_key, bytes)
                .map_err(|_| ())
        },
        req,
    );

    let response = match outcome {
        Ok(result) => {
            let artifact_ref = state.register_artifact(
                result.artifact_content_identity.clone(),
                result.artifact_kind,
            );

            let _ = state.event_tx.send(Event::new(
                result.request_context_id,
                EventPayload::PreviewReady {
                    request_context_id: result.request_context_id,
                    image_asset_id: body.image_asset_id,
                    pipeline_snapshot_id: result.pipeline_snapshot_id,
                    target_node_id: body.target_node_id.clone(),
                },
            ));

            PreviewResponse::Ready {
                artifact_ref,
                reused: result.reused,
                request_context_id: result.request_context_id,
            }
        }
        Err(err) => {
            let request_context_id = Uuid::new_v4();
            let failing_node_id = err.failing_node_id(&body.target_node_id);

            let _ = state.event_tx.send(Event::new(
                request_context_id,
                EventPayload::PreviewFailed {
                    request_context_id,
                    image_asset_id: body.image_asset_id,
                    pipeline_snapshot_id: crate::domain::pipeline_snapshot::pipeline_snapshot_id(
                        &crate::domain::pipeline_snapshot::compute_graph_identity(
                            &body.pipeline_snapshot,
                        ),
                    ),
                    target_node_id: body.target_node_id.clone(),
                },
            ));

            let last_successful_artifact_ref = state
                .preview_isolation
                .last_successful(body.image_asset_id, &body.target_node_id)
                .map(
                    |LastSuccessful {
                         content_identity,
                         kind,
                     }| { state.register_artifact(content_identity, kind) },
                );

            PreviewResponse::Stale {
                state: "stale",
                error: PreviewErrorPayload {
                    code: err.code(),
                    message: err.message(),
                },
                failing_node_id,
                last_successful_artifact_ref,
            }
        }
    };

    Ok(Json(response))
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::post;
    axum::Router::new().route("/preview", post(post_preview))
}

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
    /// Legacy body: a graph sent inline.
    #[serde(default)]
    pub pipeline_snapshot: Option<PipelineGraph>,
    pub target_node_id: String,
    /// Feature 002 body: an AlgoPipe draft at a revision (the service builds the graph).
    #[serde(default)]
    pub pipe_id: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub dataset_version_id: Option<Uuid>,
    #[serde(default)]
    pub port: Option<String>,
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

/// `POST /preview`: the legacy `pipeline_snapshot` body keeps working; a
/// `pipe_id` body previews an AlgoPipe draft (contracts/kb-api.md §4).
pub async fn post_preview(
    State(state): State<AppState>,
    Json(body): Json<PreviewRequestBody>,
) -> Result<Json<serde_json::Value>, crate::api::kb::ApiError> {
    if body.pipe_id.is_some() {
        return crate::api::pipe_preview::post_pipe_preview(state, body).await;
    }
    let snapshot = body.pipeline_snapshot.clone().ok_or_else(|| {
        crate::api::errors::KbError::new(
            crate::api::errors::KbErrorCode::BundleInvalid,
            "send either `pipeline_snapshot` or `pipe_id` with `revision`",
        )
    })?;
    let response = legacy_preview(state, body, snapshot).await?;
    Ok(Json(serde_json::to_value(response.0).expect("preview response serializes")))
}

async fn legacy_preview(
    state: AppState,
    body: PreviewRequestBody,
    snapshot: PipelineGraph,
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
        graph: &snapshot,
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
                    pipe_id: None,
                    revision: None,
                    verification_state: None,
                    unverified_nodes: None,
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
                            &snapshot,
                        ),
                    ),
                    target_node_id: body.target_node_id.clone(),
                    pipe_id: None,
                    revision: None,
                    verification_state: None,
                    unverified_nodes: None,
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
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/preview", post(post_preview))
        .route("/preview/status", get(crate::api::pipe_preview::get_preview_status))
}

//! `POST /runs`, `GET /runs/{run_id}`, `GET /runs?dataset_version_id=&image_asset_id=`
//! (contracts/local-service-api.md §Official Runs, User Story 4). The
//! endpoint returns immediately with `status: "running"`; completion is
//! reported over the Event Bus (`run_progress`/`run_completed`/
//! `run_failed`, event-bus.md) and confirmed via `GET /runs/{id}` — never
//! inferred from this call's own response.

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::data_engine::run::{compute_run_output, RunError};
use crate::data_engine::validation::check_source_availability;
use crate::data_repository::sqlite::{dataset_repo, run_repo};
use crate::domain::dataset::ImageAssetStatus;
use crate::domain::pipeline_snapshot::{PipelineGraph, PipelineSnapshot};
use crate::domain::run::{MetricSet, RunInputArtifact, RunPolicy, RunRecord, RunStatus};
use crate::domain::{ArtifactKind, ArtifactReference, ServiceError};
use crate::events::{Event, EventPayload};

#[derive(Deserialize)]
pub struct RunPolicyRequest {
    #[serde(default)]
    pub retain_intermediates: bool,
}

#[derive(Deserialize)]
pub struct PostRunRequest {
    pub dataset_version_id: Uuid,
    pub image_asset_id: Uuid,
    pub pipeline_snapshot: PipelineGraph,
    pub target_node_id: String,
    pub seed: u64,
    #[serde(default)]
    pub run_policy: Option<RunPolicyRequest>,
}

#[derive(Serialize)]
pub struct PostRunResponse {
    pub run_id: Uuid,
    pub status: &'static str,
}

/// Creates the immutable identity chain for one Run (dataset version +
/// fingerprint, image identity, input artifact, pipeline snapshot, seed —
/// FR-018) and inserts it as `running` in one transaction, then hands
/// execution to a background task so this call can return immediately.
pub async fn post_runs(
    State(state): State<AppState>,
    Json(req): Json<PostRunRequest>,
) -> Result<Json<PostRunResponse>, ServiceError> {
    let (asset, version) = {
        let db = state.db.lock().unwrap();
        let asset = dataset_repo::image_asset_by_id(&db, req.image_asset_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
            .ok_or(ServiceError::NotFound)?;
        let version = dataset_repo::get_dataset_version(&db, req.dataset_version_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
            .ok_or(ServiceError::NotFound)?;
        (asset, version)
    };

    // FR-031: a missing/changed external source blocks new official Run
    // creation, the same way it blocks a new Preview.
    match check_source_availability(&asset) {
        ImageAssetStatus::Available => {}
        ImageAssetStatus::SourceMissing => return Err(ServiceError::SourceMissing),
        ImageAssetStatus::SourceChanged => return Err(ServiceError::SourceChanged),
    }

    let run_id = Uuid::new_v4();
    let started_at = Utc::now().to_rfc3339();
    let snapshot = PipelineSnapshot::from_graph(&req.pipeline_snapshot);
    let run_input_artifact = RunInputArtifact {
        id: Uuid::new_v4(),
        content_identity: asset.imported_content_identity.clone(),
        source_image_asset_id: asset.id,
        created_at: started_at.clone(),
    };
    let run_policy = RunPolicy {
        retain_intermediates: req
            .run_policy
            .as_ref()
            .map(|p| p.retain_intermediates)
            .unwrap_or(false),
    };

    {
        let mut db = state.db.lock().unwrap();
        let tx = db
            .transaction()
            .map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_pipeline_snapshot(&tx, &snapshot)
            .map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_run_input_artifact(&tx, &run_input_artifact)
            .map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_run_record(
            &tx,
            &RunRecord {
                id: run_id,
                status: RunStatus::Running,
                dataset_version_id: req.dataset_version_id,
                dataset_fingerprint: version.fingerprint.clone(),
                image_asset_id: asset.id,
                image_asset_identity: asset.imported_content_identity.clone(),
                run_input_artifact_id: run_input_artifact.id,
                pipeline_snapshot_id: snapshot.id,
                target_node_id: req.target_node_id.clone(),
                seed: req.seed,
                node_versions: snapshot.node_versions.clone(),
                started_at: started_at.clone(),
                ended_at: None,
                output_content_identities: Vec::new(),
                run_policy,
                metric_set_id: None,
                error_summary: None,
                failed_stage: None,
            },
        )
        .map_err(|_| ServiceError::ServiceUnavailable)?;
        tx.commit().map_err(|_| ServiceError::ServiceUnavailable)?;
    }

    let state_clone = state.clone();
    let dataset_version_id = req.dataset_version_id;
    let image_asset_id = asset.id;
    let seed = req.seed;
    let pipeline_snapshot_id = snapshot.id;
    let graph = req.pipeline_snapshot;
    let target_node_id = req.target_node_id;
    let source_identity = asset.imported_content_identity;
    tokio::spawn(async move {
        execute_run(
            state_clone,
            run_id,
            dataset_version_id,
            image_asset_id,
            source_identity,
            graph,
            target_node_id,
            seed,
            pipeline_snapshot_id,
        )
        .await;
    });

    Ok(Json(PostRunResponse {
        run_id,
        status: "running",
    }))
}

/// Runs the pipeline off the request thread, checks reproducibility against
/// any prior identical-input Run, and — only once every required field is
/// ready — persists success and `metric_set_id` together in one transaction
/// (FR-020). Any failure at any stage instead persists a redacted
/// `error_summary` (FR-021) and leaves no partially-populated `succeeded`
/// row.
#[allow(clippy::too_many_arguments)]
async fn execute_run(
    state: AppState,
    run_id: Uuid,
    dataset_version_id: Uuid,
    image_asset_id: Uuid,
    source_identity: String,
    graph: PipelineGraph,
    target_node_id: String,
    seed: u64,
    pipeline_snapshot_id: Uuid,
) {
    let _ = state.event_tx.send(Event::new(
        run_id,
        EventPayload::RunProgress {
            run_id,
            stage: "importing_input".into(),
            node_id: None,
        },
    ));

    let compute = {
        let graph = graph.clone();
        let target_node_id = target_node_id.clone();
        let source_identity = source_identity.clone();
        tokio::task::spawn_blocking(move || {
            compute_run_output(&graph, &target_node_id, &source_identity)
        })
        .await
    };
    let outcome = match compute {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(err)) => {
            fail_run(&state, run_id, &err, "executing_node").await;
            return;
        }
        Err(_) => {
            fail_run_generic(
                &state,
                run_id,
                "executing_node",
                "Run's compute task did not complete.",
            )
            .await;
            return;
        }
    };

    let _ = state.event_tx.send(Event::new(
        run_id,
        EventPayload::RunProgress {
            run_id,
            stage: "executing_node".into(),
            node_id: Some(target_node_id.clone()),
        },
    ));

    let output_bytes = outcome.output_bytes;
    let write_result = {
        let state = state.clone();
        tokio::task::spawn_blocking(move || state.blob_store.write(&state.master_key, &output_bytes))
            .await
    };
    let output_identity = match write_result {
        Ok(Ok(identity)) => identity,
        _ => {
            fail_run(&state, run_id, &RunError::PersistenceFailed, "persisting").await;
            return;
        }
    };

    // FR-015/SC-006: a repeat of the exact same composite input must
    // resolve to the same output content identity, or be classified
    // non-reproducible rather than silently accepted.
    let prior = {
        let db = state.db.lock().unwrap();
        run_repo::first_successful_output(
            &db,
            dataset_version_id,
            image_asset_id,
            pipeline_snapshot_id,
            seed,
            &target_node_id,
        )
    };
    if let Ok(Some(prior_identity)) = prior {
        if prior_identity != output_identity {
            let err = RunError::NonReproducible {
                failing_node_id: target_node_id.clone(),
            };
            fail_run(&state, run_id, &err, "persisting").await;
            return;
        }
    }

    let _ = state.event_tx.send(Event::new(
        run_id,
        EventPayload::RunProgress {
            run_id,
            stage: "persisting".into(),
            node_id: None,
        },
    ));

    let metric_set_id = Uuid::new_v4();
    let ended_at = Utc::now().to_rfc3339();
    let mut step_timings = std::collections::BTreeMap::new();
    step_timings.insert(target_node_id.clone(), outcome.duration_ms);
    let metric_set = MetricSet {
        id: metric_set_id,
        run_record_id: run_id,
        // Reference-based/pixel metrics belong to the future Algorithm
        // Layer (plan.md Project Structure) — left null rather than
        // fabricated, matching data-model.md's "null when not computed"
        // rule for `dice`.
        dice: None,
        area_mm2: None,
        foreground_pixels: None,
        connected_components: None,
        step_timings,
    };

    let commit_result = {
        let mut db = state.db.lock().unwrap();
        (|| -> rusqlite::Result<()> {
            let tx = db.transaction()?;
            run_repo::insert_metric_set(&tx, &metric_set)?;
            run_repo::finalize_run_success(
                &tx,
                run_id,
                &ended_at,
                &[output_identity],
                metric_set_id,
            )?;
            tx.commit()
        })()
    };

    if commit_result.is_err() {
        fail_run(&state, run_id, &RunError::PersistenceFailed, "persisting").await;
        return;
    }

    let _ = state
        .event_tx
        .send(Event::new(run_id, EventPayload::RunCompleted { run_id }));
}

async fn fail_run(state: &AppState, run_id: Uuid, err: &RunError, stage: &str) {
    fail_run_generic(state, run_id, stage, &err.redacted_summary()).await;
}

async fn fail_run_generic(state: &AppState, run_id: Uuid, stage: &str, message: &str) {
    let ended_at = Utc::now().to_rfc3339();
    {
        let db = state.db.lock().unwrap();
        let _ = run_repo::finalize_run_failure(&db, run_id, &ended_at, message, stage);
    }
    let _ = state.event_tx.send(Event::new(
        run_id,
        EventPayload::RunFailed {
            run_id,
            failed_stage: stage.into(),
            error_summary: message.into(),
        },
    ));
}

#[derive(Serialize)]
pub struct RunRecordResponse {
    #[serde(flatten)]
    pub record: RunRecord,
    pub metric_set: Option<MetricSet>,
    /// Resolved on request, same as every other content path in this
    /// service (data-model.md ImageDisplayDescriptor) — never stored.
    pub output_artifact_refs: Vec<ArtifactReference>,
}

/// FR-022/SC-004: everything needed to walk `MetricSet → RunRecord →
/// {DatasetVersion, RunInputArtifact → ImageAsset, PipelineSnapshot}` is on
/// the returned `RunRecord` itself (`dataset_version_id`,
/// `run_input_artifact_id`, `pipeline_snapshot_id`), resolvable through the
/// existing Explorer/import endpoints.
pub async fn get_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<RunRecordResponse>, ServiceError> {
    let (record, metric_set) = {
        let db = state.db.lock().unwrap();
        let record = run_repo::get_run_record(&db, run_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
            .ok_or(ServiceError::NotFound)?;
        let metric_set = match record.metric_set_id {
            Some(id) => run_repo::get_metric_set(&db, id)
                .map_err(|_| ServiceError::ServiceUnavailable)?,
            None => None,
        };
        (record, metric_set)
    };

    let output_artifact_refs = record
        .output_content_identities
        .iter()
        .cloned()
        .map(|identity| state.register_artifact(identity, ArtifactKind::Image))
        .collect();

    Ok(Json(RunRecordResponse {
        record,
        metric_set,
        output_artifact_refs,
    }))
}

#[derive(Deserialize, Default)]
pub struct ListRunsQuery {
    #[serde(default)]
    pub dataset_version_id: Option<Uuid>,
    #[serde(default)]
    pub image_asset_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct ListRunsResponse {
    pub runs: Vec<RunRecord>,
}

pub async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<ListRunsQuery>,
) -> Result<Json<ListRunsResponse>, ServiceError> {
    let db = state.db.lock().unwrap();
    let runs = run_repo::list_run_records(&db, query.dataset_version_id, query.image_asset_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    Ok(Json(ListRunsResponse { runs }))
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/runs", post(post_runs).get(list_runs))
        .route("/runs/:run_id", get(get_run))
}

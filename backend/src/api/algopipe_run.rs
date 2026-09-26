//! Formal Run of a *published AlgoPipe* (contracts/kb-api.md §8, FR-035/FR-050/
//! FR-052). Admission is `eligibility(pipe, dataset)`: the release must be
//! executable, every dependency intact, every node's technical verification
//! currently valid for its exact subject, the Dataset Version must satisfy the
//! Target Data Profile, and every node prerequisite must hold. On any failure a
//! `run_admission_denied` audit event is written and **no Run record is created**.
//! On success the Run records the AlgoPipe identity and the admission event.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::errors::{KbError, KbErrorCode};
use crate::api::kb::ApiError;
use crate::api::runs::{execute_run, AlgoPipeRef, ComputeFn, Computed, PostRunRequest, PostRunResponse};
use crate::api::AppState;
use crate::data_engine::run::RunError;
use crate::data_engine::validation::check_source_availability;
use crate::data_repository::sqlite::{dataset_repo, kb_event_repo, run_repo};
use crate::designer::official::{default_target, execute_pipe, resolve_run_nodes, snapshot_graph};
use crate::designer::runtime_adapter::{Artifact, ExecContext, ImageF32};
use crate::domain::dataset::ImageAssetStatus;
use crate::domain::pipeline_snapshot::PipelineSnapshot;
use crate::domain::run::{RunInputArtifact, RunPolicy, RunRecord, RunStatus};
use crate::domain::ServiceError;
use crate::kb::eligibility::{eligibility, load_pipe, Eligibility};
use crate::kb::profile::ImageFacts;

/// Maps the first reason an admission failed to the contract's error.
fn admission_error(e: &Eligibility) -> KbError {
    let (code, message) = match e.reasons.first().map(|r| r.code) {
        Some("not_executable_release") => (KbErrorCode::NotExecutable, "this version is a knowledge-only release and cannot be run"),
        Some("dependency_unavailable") | Some("implementation_unavailable") => (KbErrorCode::NotExecutable, "a node this version depends on is not available to run"),
        Some("verification_withdrawn") | Some("verification_missing") => (KbErrorCode::VerificationInvalid, "a node's technical verification is not currently valid"),
        _ => (KbErrorCode::ProfileUnsatisfied, "the selected dataset does not satisfy this version's data requirements"),
    };
    KbError::new(code, message).with_details(json!({ "reasons": e.reasons }))
}

fn load_gray(state: &AppState, identity: &str) -> Result<ImageF32, ()> {
    let bytes = state.blob_store.read(&state.master_key, identity).map_err(|_| ())?;
    let img = image::load_from_memory(&bytes).map_err(|_| ())?.to_luma8();
    Ok(ImageF32 { w: img.width() as usize, h: img.height() as usize, data: img.pixels().map(|p| f32::from(p.0[0])).collect() })
}

pub async fn post_algopipe_run(state: AppState, req: PostRunRequest) -> Result<Json<PostRunResponse>, ApiError> {
    let AlgoPipeRef { id: pipe_id, version: pipe_version } = req.algopipe.expect("dispatched only with an algopipe");
    let admitted = {
        let db = state.db.lock().unwrap();
        let asset = dataset_repo::image_asset_by_id(&db, req.image_asset_id)?.ok_or(ServiceError::NotFound)?;
        let version = dataset_repo::get_dataset_version(&db, req.dataset_version_id)?.ok_or(ServiceError::NotFound)?;
        match check_source_availability(&asset) {
            ImageAssetStatus::Available => {}
            ImageAssetStatus::SourceMissing => return Err(ServiceError::SourceMissing.into()),
            ImageAssetStatus::SourceChanged => return Err(ServiceError::SourceChanged.into()),
        }
        let pipe = load_pipe(&db, &state.kb_root, &pipe_id, &pipe_version).ok_or(ServiceError::NotFound)?;
        let images: Vec<ImageFacts> = dataset_repo::image_assets_for_version(&db, req.dataset_version_id)?.iter().map(ImageFacts::from_asset).collect();
        let verdict = eligibility(&db, &state.kb_root, &pipe, &images);
        let subject = format!("{pipe_id}@{pipe_version}");
        if !verdict.eligible {
            // Denials are audited; nothing else is written.
            kb_event_repo::append(&db, "run_admission_denied", &subject, &json!({ "dataset_version_id": req.dataset_version_id, "reasons": verdict.reasons }))?;
            return Err(admission_error(&verdict).into());
        }
        let event_id = kb_event_repo::append(&db, "run_admission_allowed", &subject, &json!({ "dataset_version_id": req.dataset_version_id }))?;
        (asset, version, pipe, event_id)
    };
    let (asset, version, pipe, event_id) = admitted;

    let graph = pipe.graph.clone().expect("an eligible pipe has a graph");
    let nodes = {
        let db = state.db.lock().unwrap();
        resolve_run_nodes(&db, &state.kb_root, &pipe).map_err(|_| ServiceError::ServiceUnavailable)?
    };
    let target = match req.target_node_id.clone().or_else(|| default_target(&graph)) {
        Some(t) if graph.nodes.iter().any(|n| n.instance_id == t) => t,
        _ => return Err(KbError::new(KbErrorCode::BundleInvalid, "name a `target_node_id`: the pipe has no single output node").into()),
    };

    let pg = snapshot_graph(&graph, &nodes);
    let mut snapshot = PipelineSnapshot::from_graph(&pg);
    let content_id = pipe.lock.as_ref().map(|l| l.content_id.clone()).unwrap_or_default();
    snapshot.source_algopipe_content_id = Some(content_id.clone());

    let run_id = Uuid::new_v4();
    let started_at = Utc::now().to_rfc3339();
    let input = RunInputArtifact { id: Uuid::new_v4(), content_identity: asset.imported_content_identity.clone(), source_image_asset_id: asset.id, created_at: started_at.clone() };
    {
        let mut db = state.db.lock().unwrap();
        let tx = db.transaction().map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_pipeline_snapshot(&tx, &snapshot).map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_run_input_artifact(&tx, &input).map_err(|_| ServiceError::ServiceUnavailable)?;
        run_repo::insert_run_record(
            &tx,
            &RunRecord {
                id: run_id,
                status: RunStatus::Running,
                dataset_version_id: req.dataset_version_id,
                dataset_fingerprint: version.fingerprint.clone(),
                image_asset_id: asset.id,
                image_asset_identity: asset.imported_content_identity.clone(),
                run_input_artifact_id: input.id,
                pipeline_snapshot_id: snapshot.id,
                target_node_id: target.clone(),
                seed: req.seed,
                node_versions: snapshot.node_versions.clone(),
                started_at,
                ended_at: None,
                output_content_identities: Vec::new(),
                run_policy: RunPolicy { retain_intermediates: req.run_policy.as_ref().map(|p| p.retain_intermediates).unwrap_or(false) },
                metric_set_id: None,
                error_summary: None,
                failed_stage: None,
                algopipe_id: Some(pipe_id),
                algopipe_version: Some(pipe_version),
                algopipe_content_id: Some(content_id),
                eligibility_event_id: Some(event_id.to_string()),
            },
        )
        .map_err(|_| ServiceError::ServiceUnavailable)?;
        tx.commit().map_err(|_| ServiceError::ServiceUnavailable)?;
    }

    let compute: ComputeFn = {
        let target = target.clone();
        let spacing = asset.pixel_spacing_mm.map(|p| (p.x, p.y));
        let source = load_gray(&state, &asset.imported_content_identity);
        Box::new(move || {
            let src = source.map_err(|_| RunError::PersistenceFailed)?;
            let ctx = ExecContext { source: Some(Arc::new(src)), pixel_spacing_mm: spacing, seed: None };
            let out = execute_pipe(&graph, &nodes, &pg, &target, None, &ctx).map_err(|(failing_node_id, _)| RunError::ComputeFailed { failing_node_id })?;
            let metrics = match &out.artifact {
                Artifact::Table(t) => Some((t.get("mm2").copied(), t.get("pixels").map(|v| *v as i64), t.get("components").map(|v| *v as i64))),
                _ => None,
            };
            let (bytes, _) = out.artifact.display_bytes();
            Ok(Computed { bytes, duration_ms: out.duration_ms, metrics })
        })
    };
    let (dataset_version_id, image_asset_id, seed, snapshot_id) = (req.dataset_version_id, asset.id, req.seed, snapshot.id);
    tokio::spawn(async move {
        execute_run(state, run_id, dataset_version_id, image_asset_id, target, seed, snapshot_id, compute).await;
    });
    Ok(Json(PostRunResponse { run_id, status: "running" }))
}

/// `GET /runs/{id}/provenance` (SC-005): resolves a completed Run back to the
/// published AlgoPipe version, each node's exact version and effective
/// parameters, the Dataset Version, the evidence references and any amendments —
/// all read from immutable published files and append-only chains.
pub async fn get_provenance(State(state): State<AppState>, Path(run_id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    let db = state.db.lock().unwrap();
    let run = run_repo::get_run_record(&db, run_id)?.ok_or(ServiceError::NotFound)?;
    let (Some(id), Some(version)) = (run.algopipe_id.clone(), run.algopipe_version.clone()) else {
        return Err(ServiceError::NotFound.into());
    };
    let pipe = load_pipe(&db, &state.kb_root, &id, &version).ok_or(ServiceError::NotFound)?;
    let graph = pipe.graph.clone().ok_or(ServiceError::NotFound)?;
    let mut nodes = Vec::new();
    let mut amendments = vec![json!({ "bundle": format!("{id}@{version}"), "records": crate::kb::evidence::amendment::list(&state.kb_root, &id, &version).unwrap_or_default() })];
    for n in &graph.nodes {
        let r = &n.node_ref;
        let dep = crate::kb::catalog::query::path_of(&db, "algonode", &r.id, &r.version, "published")?
            .and_then(|p| crate::kb::bundle::read::read_bundle(&state.kb_root.join(p)).0);
        let params = dep
            .as_ref()
            .and_then(|d| d.contract.as_ref())
            .map(|c| crate::designer::runtime_adapter::effective_parameters(c, n.parameters.as_ref()))
            .unwrap_or_default();
        let pin = graph.implementation_pins.iter().find(|p| p.instance_id == n.instance_id);
        nodes.push(json!({
            "instance_id": n.instance_id,
            "ref": { "id": r.id, "version": r.version, "content_id": r.content_id },
            "parameters": params,
            "implementation": pin.map(|p| json!({ "implementation_id": p.implementation_id, "version": p.version })),
            "evidence": dep.as_ref().map(|d| d.evidence.iter().map(|e| json!({ "evidence_id": e.evidence_id, "type": e.evidence_type, "distributable": e.distributable })).collect::<Vec<_>>()).unwrap_or_default(),
        }));
        amendments.push(json!({ "bundle": format!("{}@{}", r.id, r.version), "records": crate::kb::evidence::amendment::list(&state.kb_root, &r.id, &r.version).unwrap_or_default() }));
    }
    let event = run.eligibility_event_id.as_deref().and_then(|eid| {
        kb_event_repo::list(&db, &kb_event_repo::EventFilter { event_type: Some("run_admission_allowed".into()), ..Default::default() })
            .ok()
            .and_then(|l| l.into_iter().find(|e| e.event_id.to_string() == eid))
    });
    Ok(Json(json!({
        "run_id": run.id,
        "status": run.status,
        "algopipe": {
            "id": id,
            "version": version,
            "content_id": run.algopipe_content_id,
            "computational_identity": pipe.lock.as_ref().and_then(|l| l.computational_identity.clone()),
            "release_kind": pipe.lock.as_ref().and_then(|l| l.release_kind.clone()),
            "disclosures": pipe.lock.as_ref().map(|l| l.disclosures.clone()).unwrap_or_default(),
        },
        "dataset_version": { "id": run.dataset_version_id, "fingerprint": run.dataset_fingerprint },
        "image_asset_id": run.image_asset_id,
        "target_node_id": run.target_node_id,
        "nodes": nodes,
        "amendments": amendments,
        "admission_event": event,
    })))
}

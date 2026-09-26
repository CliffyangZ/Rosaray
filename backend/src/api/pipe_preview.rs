//! Preview of an AlgoPipe draft (contracts/kb-api.md §4). Deliberately has no
//! reference to the run repository: no path through here can create a Run
//! record (Constitution III, SC-009).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::api::errors::{KbError, KbErrorCode};
use crate::api::kb::ApiError;
use crate::api::preview::PreviewRequestBody;
use crate::api::AppState;
use crate::data_engine::preview::LastSuccessful;
use crate::data_engine::validation::check_source_availability;
use crate::data_repository::memory_cache::PreviewCacheEntry;
use crate::data_repository::sqlite::dataset_repo;
use crate::designer::preview::{resolve_node, run_pipe_preview, PipePreviewRequest, PreviewFailure};
use crate::designer::runtime_adapter::{ImageF32, Registry};
use crate::designer::validate::finding::BundleRef;
use crate::designer::validate::graph::{validate_graph, GraphInput};
use crate::domain::dataset::ImageAssetStatus;
use crate::domain::{ArtifactKind, ServiceError};
use crate::events::{Event, EventPayload};
use crate::kb::bundle::draft::revision_of;
use crate::kb::bundle::model::GraphFile;
use crate::kb::bundle::read::read_bundle;
use crate::kb::catalog::query::CatalogDefinitions;
use crate::kb::catalog::repo::draft_dir;
use crate::kb::identity::split_endpoint;

/// The instance ids feeding `target`, inclusive.
fn cone_of(graph: &GraphFile, target: &str) -> Vec<String> {
    let mut seen = vec![target.to_string()];
    let mut i = 0;
    while i < seen.len() {
        let cur = seen[i].clone();
        for e in &graph.edges {
            if split_endpoint(&e.to).0 == cur {
                let from = split_endpoint(&e.from).0;
                if !seen.contains(&from) {
                    seen.push(from);
                }
            }
        }
        i += 1;
    }
    seen
}

fn load_gray(state: &AppState, identity: &str) -> Result<ImageF32, ServiceError> {
    let bytes = state.blob_store.read(&state.master_key, identity).map_err(|_| ServiceError::StaleReference)?;
    let img = image::load_from_memory(&bytes).map_err(|_| ServiceError::StaleReference)?.to_luma8();
    Ok(ImageF32 { w: img.width() as usize, h: img.height() as usize, data: img.pixels().map(|p| f32::from(p.0[0])).collect() })
}

fn extras(pipe_id: &str, revision: &str, verification: Option<&str>, unverified: Option<Vec<String>>) -> (Option<String>, Option<String>, Option<String>, Option<Vec<String>>) {
    (Some(pipe_id.to_string()), Some(revision.to_string()), verification.map(str::to_string), unverified)
}

pub async fn post_pipe_preview(state: AppState, body: PreviewRequestBody) -> Result<Json<Value>, ApiError> {
    let pipe_id = body.pipe_id.clone().unwrap_or_default();
    let revision = body
        .revision
        .clone()
        .ok_or_else(|| KbError::new(KbErrorCode::BundleInvalid, "`revision` is required with `pipe_id`"))?;
    let delay_ms = state.preview_delays.lock().unwrap().pop_front();

    // Everything that touches the database or disk happens before any await.
    let prepared = {
        let db = state.db.lock().unwrap();
        let asset = dataset_repo::image_asset_by_id(&db, body.image_asset_id)?.ok_or(ServiceError::NotFound)?;
        match check_source_availability(&asset) {
            ImageAssetStatus::Available => {}
            ImageAssetStatus::SourceMissing => return Err(ServiceError::SourceMissing.into()),
            ImageAssetStatus::SourceChanged => return Err(ServiceError::SourceChanged.into()),
        }
        let dir = draft_dir(&state.kb_root, "algopipe", &pipe_id);
        if !dir.is_dir() {
            return Err(ServiceError::NotFound.into());
        }
        // A stale revision is refused before anything runs (FR-019).
        if revision_of(&dir).map_err(|_| ServiceError::ServiceUnavailable)? != revision {
            return Err(ServiceError::StaleReference.into());
        }
        let (bundle, _) = read_bundle(&dir);
        let bundle = bundle.ok_or(ServiceError::NotFound)?;
        let graph = bundle.graph.clone().ok_or(ServiceError::NotFound)?;
        let cone = cone_of(&graph, &body.target_node_id);
        if !graph.nodes.iter().any(|n| n.instance_id == body.target_node_id) {
            return Err(ServiceError::NotFound.into());
        }

        // Errors inside the target's cone make it not previewable; unrelated branches don't.
        let defs = CatalogDefinitions { conn: &db, root: &state.kb_root };
        let report = validate_graph(
            &GraphInput {
                bundle: BundleRef::new(pipe_id.clone(), None),
                graph: &graph,
                previous: None,
                profile: graph.target_data_profile.as_ref(),
                domain: bundle.header.domain.as_deref(),
            },
            &defs,
        );
        let blocking: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.is_error())
            .filter(|f| {
                let inst = f.subject.reference.split(['.', '-']).next().unwrap_or("");
                f.subject.reference == pipe_id || cone.iter().any(|c| c == inst || f.subject.reference.contains(&format!("{c}.")))
            })
            .cloned()
            .collect();
        if !blocking.is_empty() {
            return Err(KbError::new(KbErrorCode::NotPreviewable, "the pipe has errors in this node's inputs")
                .with_details(json!({ "findings": blocking, "nodes": [] }))
                .into());
        }

        let mut nodes = BTreeMap::new();
        for n in &graph.nodes {
            if let Some(pn) = resolve_node(&db, &state.kb_root, &n.instance_id, &n.node_ref) {
                nodes.insert(n.instance_id.clone(), pn);
            }
        }
        (asset, graph, nodes, bundle.header.domain.clone())
    };
    let (asset, graph, nodes, _domain) = prepared;

    let source = Arc::new(load_gray(&state, &asset.imported_content_identity)?);
    let selection = format!("{pipe_id}/{}", body.target_node_id);
    let generation = state.preview_isolation.begin(body.image_asset_id, &selection);
    let request_context_id = Uuid::new_v4();

    let outcome = run_pipe_preview(
        &state.preview_nodes,
        &state.preview_board,
        &Registry::builtin(),
        PipePreviewRequest {
            graph: &graph,
            nodes: &nodes,
            target: &body.target_node_id,
            port: body.port.as_deref(),
            source,
            source_identity: &asset.imported_content_identity,
            pixel_spacing_mm: asset.pixel_spacing_mm.map(|p| (p.x, p.y)),
            pipe_id: &pipe_id,
            revision: &revision,
        },
    );

    // Injectable delay so tests can make an older request finish last (SC-004).
    if let Some(ms) = delay_ms {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    match outcome {
        Ok(out) => {
            let (bytes, kind) = out.artifact.display_bytes();
            let identity = state.blob_store.write(&state.master_key, &bytes).map_err(|_| ServiceError::ServiceUnavailable)?;
            let artifact_ref = state.register_artifact(identity.clone(), kind);
            state.preview_cache.insert(
                format!("pipe:{pipe_id}:{}:{}:{}", body.image_asset_id, body.target_node_id, out.port),
                PreviewCacheEntry {
                    output_content_identity: identity.clone(),
                    producing_node_id: body.target_node_id.clone(),
                    pipeline_snapshot_id: out.pipeline_snapshot_id,
                    image_asset_id: body.image_asset_id,
                    duration_ms: out.duration_ms,
                },
            );
            // Only the latest request for this selection may become "last successful".
            let current = state.preview_isolation.is_current(body.image_asset_id, &selection, generation);
            state.preview_isolation.record_success(body.image_asset_id, &selection, generation, LastSuccessful { content_identity: identity, kind });

            let (pipe, rev, vs, un) = extras(&pipe_id, &revision, Some(out.verification_state), Some(out.unverified_nodes.clone()));
            let _ = state.event_tx.send(Event::new(
                request_context_id,
                EventPayload::PreviewReady {
                    request_context_id,
                    image_asset_id: body.image_asset_id,
                    pipeline_snapshot_id: out.pipeline_snapshot_id,
                    target_node_id: body.target_node_id.clone(),
                    pipe_id: pipe,
                    revision: rev,
                    verification_state: vs,
                    unverified_nodes: un,
                },
            ));
            Ok(Json(json!({
                "state": "ready",
                "request_context_id": request_context_id,
                "artifact_ref": artifact_ref,
                "port": out.port,
                "reused": out.reused,
                "node_reuse": out.node_reuse,
                "computed_nodes": out.computed_nodes,
                "duration_ms": out.duration_ms,
                "verification_state": out.verification_state,
                "unverified_nodes": out.unverified_nodes,
                "stale": false,
                "current": current,
            })))
        }
        Err(PreviewFailure::NotPreviewable { nodes }) => Err(KbError::new(KbErrorCode::NotPreviewable, "a node in this node's inputs cannot run")
            .with_details(json!({ "nodes": nodes }))
            .into()),
        Err(PreviewFailure::UnknownTarget(_)) => Err(ServiceError::NotFound.into()),
        Err(PreviewFailure::Graph(msg)) => Err(KbError::new(KbErrorCode::NotPreviewable, msg).into()),
        Err(PreviewFailure::Node { failing_node_id, error }) => {
            let last = state
                .preview_isolation
                .last_successful(body.image_asset_id, &selection)
                .map(|LastSuccessful { content_identity, kind }: LastSuccessful| state.register_artifact(content_identity, kind as ArtifactKind));
            let (pipe, rev, vs, un) = extras(&pipe_id, &revision, None, None);
            let _ = state.event_tx.send(Event::new(
                request_context_id,
                EventPayload::PreviewFailed {
                    request_context_id,
                    image_asset_id: body.image_asset_id,
                    pipeline_snapshot_id: Uuid::nil(),
                    target_node_id: body.target_node_id.clone(),
                    pipe_id: pipe,
                    revision: rev,
                    verification_state: vs,
                    unverified_nodes: un,
                },
            ));
            Ok(Json(json!({
                "state": "failed",
                "request_context_id": request_context_id,
                "error": { "code": error.code, "message": error.message },
                "failing_node_id": failing_node_id,
                "last_successful_artifact_ref": last,
                // The previous result is shown, but labelled as no longer current.
                "stale": last.is_some(),
            })))
        }
    }
}

#[derive(Deserialize)]
pub struct StatusQuery {
    pub pipe_id: String,
    pub revision: Option<String>,
}

/// `GET /preview/status?pipe_id=&revision=` — per-node status (FR-016).
pub async fn get_preview_status(State(state): State<AppState>, Query(q): Query<StatusQuery>) -> Result<Json<Value>, ApiError> {
    let dir = draft_dir(&state.kb_root, "algopipe", &q.pipe_id);
    let (bundle, _) = read_bundle(&dir);
    let all: Vec<String> = bundle.and_then(|b| b.graph).map(|g| g.nodes.into_iter().map(|n| n.instance_id).collect()).unwrap_or_default();
    let nodes = state.preview_board.statuses(&q.pipe_id, &all);
    Ok(Json(json!({ "pipe_id": q.pipe_id, "revision": q.revision, "nodes": nodes })))
}

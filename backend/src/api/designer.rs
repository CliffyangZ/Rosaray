//! `/designer` routes (contracts/kb-api.md §2): stateless validation of the
//! *unsaved* working graph or contract, which is what keeps undo/redo
//! client-side while validation stays server-authoritative.

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::errors::{KbError, KbErrorCode};
use crate::api::kb::ApiError;
use crate::api::AppState;
use crate::designer::validate::contract::validate_contract;
use crate::designer::validate::finding::BundleRef;
use crate::designer::validate::graph::{validate_graph, GraphInput};
use crate::kb::bundle::model::{Contract, GraphFile, TargetDataProfile};
use crate::kb::catalog::query::CatalogDefinitions;

#[derive(Deserialize)]
pub struct ValidateBody {
    /// `algopipe` (default) or `algonode`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub graph: Option<GraphFile>,
    /// The graph before the latest edit, to compute `stale_nodes`.
    #[serde(default)]
    pub previous: Option<GraphFile>,
    #[serde(default)]
    pub profile: Option<TargetDataProfile>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub contract: Option<Contract>,
}

pub async fn post_validate(State(state): State<AppState>, Json(body): Json<ValidateBody>) -> Result<Json<Value>, ApiError> {
    let bundle = BundleRef::new(body.id.clone().unwrap_or_else(|| "(unsaved)".to_string()), None);
    match body.kind.as_deref().unwrap_or("algopipe") {
        "algonode" => {
            let contract = body
                .contract
                .ok_or_else(|| KbError::new(KbErrorCode::BundleInvalid, "send a `contract` to validate"))?;
            let findings = validate_contract(&bundle, &contract);
            let valid = !findings.iter().any(|f| f.is_error());
            Ok(Json(json!({ "valid": valid, "executable_for_preview": false, "findings": findings, "stale_nodes": [] })))
        }
        "algopipe" => {
            let graph = body.graph.ok_or_else(|| KbError::new(KbErrorCode::BundleInvalid, "send a `graph` to validate"))?;
            let db = state.db.lock().unwrap();
            let defs = CatalogDefinitions { conn: &db, root: &state.kb_root };
            let report = validate_graph(
                &GraphInput {
                    bundle,
                    graph: &graph,
                    previous: body.previous.as_ref(),
                    profile: body.profile.as_ref(),
                    domain: body.domain.as_deref(),
                },
                &defs,
            );
            // A computational edit invalidates earlier Preview results downstream (FR-018).
            if let (Some(pipe_id), false) = (body.id.as_deref(), report.stale_nodes.is_empty()) {
                let revision = crate::kb::bundle::draft::revision_of(&crate::kb::catalog::repo::draft_dir(&state.kb_root, "algopipe", pipe_id)).unwrap_or_default();
                state.preview_board.mark_stale(pipe_id, &report.stale_nodes);
                let _ = state.event_tx.send(crate::events::Event::new(
                    uuid::Uuid::nil(),
                    crate::events::EventPayload::PreviewStale { pipe_id: pipe_id.to_string(), revision, nodes: report.stale_nodes.clone() },
                ));
            }
            Ok(Json(json!({
                "valid": report.valid,
                "executable_for_preview": report.executable_for_preview,
                "findings": report.findings,
                "stale_nodes": report.stale_nodes,
            })))
        }
        _ => Err(KbError::new(KbErrorCode::BundleInvalid, "kind must be `algopipe` or `algonode`").into()),
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/designer/validate", post(post_validate))
}

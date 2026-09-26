//! `system-one/1` transport. All protocol semantics live in `rosaray_qkb::system_one`.

use axum::extract::{Path, State};
use axum::{Extension, Json};
use rosaray_qkb::system_one::{query, selection};
use serde_json::Value;

use super::{ApiError, AppState, Caller};

fn source(caller: &Caller) -> Result<String, ApiError> {
    caller.source_id.clone().ok_or_else(ApiError::access_denied)
}

pub async fn query(State(state): State<AppState>, Extension(caller): Extension<Caller>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let source_id = source(&caller)?;
    let out = state.blocking(move |conn, root| Ok(query::answer(conn, root, &source_id, &body)?)).await?;
    Ok(Json(out))
}

pub async fn selection(State(state): State<AppState>, Extension(caller): Extension<Caller>, Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let source_id = source(&caller)?;
    let out = state.blocking(move |conn, root| Ok(selection::report(conn, root, &source_id, &body)?)).await?;
    Ok(Json(out))
}

pub async fn record(State(state): State<AppState>, Extension(caller): Extension<Caller>, Path(request_id): Path<String>) -> Result<Json<Value>, ApiError> {
    source(&caller)?;
    let out = state.blocking(move |conn, _| Ok(selection::read_record(conn, &request_id)?)).await?;
    Ok(Json(out))
}

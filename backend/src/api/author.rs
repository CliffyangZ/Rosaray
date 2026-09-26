//! Author-scope operations: submissions and maintenance. Frontend and System
//! One credentials never reach these routes.

use axum::extract::{Query, State};
use axum::Json;
use base64::Engine as _;
use rosaray_qkb::bundle::model::Trust;
use rosaray_qkb::submit::Submission;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ApiError, AppState};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmissionBody {
    /// Relative path → UTF-8 text.
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, String>,
    /// Relative path → base64 bytes (for non-text files).
    #[serde(default)]
    pub files_base64: std::collections::BTreeMap<String, String>,
}

pub async fn submit(State(state): State<AppState>, Json(body): Json<SubmissionBody>) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let mut files: Vec<(String, Vec<u8>)> = body.files.into_iter().map(|(p, t)| (p, t.into_bytes())).collect();
    for (p, b64) in body.files_base64 {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| ApiError::bad_request("files_base64 must be base64"))?;
        files.push((p, bytes));
    }
    let accepted = state
        .blocking(move |conn, root| Ok(rosaray_qkb::submit::submit(conn, root, Submission { files })?))
        .await?;
    let status = if accepted.already_present { axum::http::StatusCode::OK } else { axum::http::StatusCode::CREATED };
    Ok((status, Json(json!({ "kind": accepted.kind, "id": accepted.id, "version": accepted.version, "content_id": accepted.content_id, "already_present": accepted.already_present }))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationBody {
    pub id: String,
    pub version: String,
    /// `reverify` (re-run the node's own tests) or `withdraw`.
    pub action: String,
    #[serde(default)]
    pub reason: Option<String>,
}

pub async fn verification(State(state): State<AppState>, Json(b): Json<VerificationBody>) -> Result<Json<Value>, ApiError> {
    let deleted = state
        .blocking(move |conn, root| match b.action.as_str() {
            "reverify" => {
                let (passed, deleted) = rosaray_qkb::author::reverify(conn, root, &b.id, &b.version).map_err(ApiError::from)?;
                Ok(json!({ "passed": passed, "deleted": deleted }))
            }
            "withdraw" => {
                let reason = b.reason.filter(|r| !r.trim().is_empty()).ok_or_else(|| ApiError::bad_request("a reason is required to withdraw"))?;
                let deleted = rosaray_qkb::author::withdraw_verification(conn, root, &b.id, &b.version, &reason).map_err(ApiError::from)?;
                Ok(json!({ "deleted": deleted }))
            }
            _ => Err(ApiError::bad_request("action must be `reverify` or `withdraw`")),
        })
        .await?;
    Ok(Json(deleted))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustBody {
    pub id: String,
    pub version: String,
    /// `builtin | trusted | untrusted`
    pub trust: Trust,
}

pub async fn trust(State(state): State<AppState>, Json(b): Json<TrustBody>) -> Result<Json<Value>, ApiError> {
    let deleted = state
        .blocking(move |conn, root| Ok(rosaray_qkb::author::set_trust(conn, root, &b.id, &b.version, b.trust)?))
        .await?;
    Ok(Json(json!({ "deleted": deleted })))
}

pub async fn rescan(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let deleted = state.blocking(|conn, root| Ok(rosaray_qkb::cleanup::recompute_and_purge(conn, root, "rescan")?)).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

pub async fn rebuild_index(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let out = state.blocking(|conn, root| Ok(rosaray_qkb::catalog::repo::rebuild(conn, root, &mut |_| {})?)).await?;
    Ok(Json(json!({ "scanned": out.scanned, "indexed": out.indexed })))
}

#[derive(Deserialize)]
pub struct DeletionsQuery {
    pub id: Option<String>,
    pub version: Option<String>,
}

pub async fn deletions(State(state): State<AppState>, Query(q): Query<DeletionsQuery>) -> Result<Json<Value>, ApiError> {
    let records = state
        .blocking(move |conn, _| {
            let ident = q.id.as_deref().map(|id| (id, q.version.as_deref().unwrap_or("")));
            Ok(rosaray_qkb::cleanup::list_deletions(conn, ident)?)
        })
        .await?;
    Ok(Json(json!({ "deletions": records })))
}

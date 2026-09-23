//! `POST /export-bundles`, `GET /export-bundles/{id}/content`,
//! `POST /export-bundles/import` (contracts/local-service-api.md §Export /
//! Import Bundle, User Story 5). The credential is only ever a request
//! field: it is never persisted, logged, or written next to the bundle
//! (FR-035) — only an opaque key id is recorded.

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use chrono::Utc;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::data_engine::export::{self, Manifest};
use crate::domain::ServiceError;

#[derive(Deserialize)]
pub struct PostExportRequest {
    #[serde(default)]
    pub dataset_version_ids: Vec<Uuid>,
    #[serde(default)]
    pub run_record_ids: Vec<Uuid>,
    #[serde(default)]
    pub credential: Option<String>,
}

#[derive(Serialize)]
pub struct PostExportResponse {
    pub export_bundle_id: Uuid,
    pub manifest: Manifest,
    pub credential_key_id: String,
    pub size_bytes: usize,
}

pub async fn post_export_bundle(
    State(state): State<AppState>,
    Json(req): Json<PostExportRequest>,
) -> Result<Json<PostExportResponse>, ServiceError> {
    let credential = req.credential.unwrap_or_default();
    let bundle_id = Uuid::new_v4();

    let response = tokio::task::spawn_blocking(move || -> Result<_, ServiceError> {
        let built = {
            let db = state.db.lock().unwrap();
            export::build_bundle(
                &db,
                &state.blob_store,
                &state.master_key,
                &req.dataset_version_ids,
                &req.run_record_ids,
                &credential,
            )?
        };

        let file_name = format!("{bundle_id}.rsybundle");
        std::fs::create_dir_all(&state.exports_dir).map_err(|_| ServiceError::ServiceUnavailable)?;
        // Write-then-rename so a crash never leaves a truncated bundle
        // that looks complete.
        let final_path = state.exports_dir.join(&file_name);
        let tmp_path = state.exports_dir.join(format!("{file_name}.tmp"));
        std::fs::write(&tmp_path, &built.bytes).map_err(|_| ServiceError::ServiceUnavailable)?;
        std::fs::rename(&tmp_path, &final_path).map_err(|_| ServiceError::ServiceUnavailable)?;

        let db = state.db.lock().unwrap();
        db.execute(
            "INSERT INTO export_bundles (id, manifest_json, credential_key_id, file_name, size_bytes, created_at, excludes_preview)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            rusqlite::params![
                bundle_id.to_string(),
                serde_json::to_string(&built.manifest).unwrap(),
                built.key_id,
                file_name,
                built.bytes.len() as i64,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|_| ServiceError::ServiceUnavailable)?;

        Ok(PostExportResponse {
            export_bundle_id: bundle_id,
            manifest: built.manifest,
            credential_key_id: built.key_id,
            size_bytes: built.bytes.len(),
        })
    })
    .await
    .map_err(|_| ServiceError::ServiceUnavailable)??;

    Ok(Json(response))
}

/// Streams the encrypted bundle file (ciphertext only — safe to hand out
/// without the credential).
pub async fn get_export_bundle_content(
    State(state): State<AppState>,
    Path(bundle_id): Path<Uuid>,
) -> Result<Response, ServiceError> {
    let file_name: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT file_name FROM export_bundles WHERE id = ?1",
            [bundle_id.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| ServiceError::ServiceUnavailable)?
    };
    let file_name = file_name.ok_or(ServiceError::NotFound)?;
    let bytes = tokio::fs::read(state.exports_dir.join(&file_name))
        .await
        .map_err(|_| ServiceError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}

#[derive(Serialize)]
pub struct ImportResponse {
    pub imported_dataset_version_ids: Vec<Uuid>,
    pub imported_run_record_ids: Vec<Uuid>,
}

pub async fn post_import_bundle(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ImportResponse>, ServiceError> {
    let mut bundle: Option<Vec<u8>> = None;
    let mut credential: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ServiceError::BundleTampered)?
    {
        match field.name() {
            Some("bundle") => {
                bundle = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|_| ServiceError::BundleTampered)?
                        .to_vec(),
                )
            }
            Some("credential") => {
                credential = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ServiceError::CredentialRequired)?,
                )
            }
            _ => {}
        }
    }
    let bundle = bundle.ok_or(ServiceError::BundleTampered)?;
    let credential = credential.unwrap_or_default();

    let summary = tokio::task::spawn_blocking(move || -> Result<_, ServiceError> {
        let mut db = state.db.lock().unwrap();
        Ok(export::import_bundle(
            &mut db,
            &state.blob_store,
            &state.master_key,
            state.project_id,
            &credential,
            &bundle,
        )?)
    })
    .await
    .map_err(|_| ServiceError::ServiceUnavailable)??;

    Ok(Json(ImportResponse {
        imported_dataset_version_ids: summary.dataset_version_ids,
        imported_run_record_ids: summary.run_record_ids,
    }))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/export-bundles", post(post_export_bundle))
        .route("/export-bundles/:bundle_id/content", get(get_export_bundle_content))
        .route(
            "/export-bundles/import",
            // Bundles carry image content; axum's 2 MB default would reject
            // any realistic one.
            post(post_import_bundle).layer(DefaultBodyLimit::disable()),
        )
}

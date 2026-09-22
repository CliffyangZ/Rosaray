//! Dataset/Explorer reads (User Story 2, contracts/local-service-api.md
//! §Dataset / Explorer reads). Lightweight list queries only — no pixel
//! data (FR-036) — plus the single `/display` call the frontend uses to
//! get `ArtifactReference`s for actual content, and on-demand thumbnails.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use base64::Engine;
use serde::Serialize;
use uuid::Uuid;

use crate::api::AppState;
use crate::data_engine::thumbnail::render_thumbnail_png;
use crate::data_engine::validation::check_source_availability;
use crate::data_repository::sqlite::thumbnail_repo::ThumbnailState;
use crate::data_repository::sqlite::{dataset_repo, thumbnail_repo};
use crate::domain::dataset::MaskValidity;
use crate::domain::display::{
    DatasetVersionListEntry, DisplayMetadata, ImageDisplayDescriptor, ImageListEntry,
};
use crate::domain::{ArtifactKind, ServiceError};
use crate::events::{Event, EventPayload};

#[derive(Serialize)]
pub struct DatasetListEntry {
    id: Uuid,
    display_name: String,
    latest_version_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct DatasetsResponse {
    datasets: Vec<DatasetListEntry>,
}

/// Dataset discovery (US2 acceptance scenario 1): lets Explorer list
/// existing Datasets on startup without requiring a fresh import to learn
/// their identity first.
pub async fn list_datasets(State(state): State<AppState>) -> Result<Json<DatasetsResponse>, ServiceError> {
    let db = state.db.lock().unwrap();
    let datasets = dataset_repo::list_datasets_for_project(&db, state.project_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?
        .into_iter()
        .map(|d| DatasetListEntry {
            id: d.id,
            display_name: d.display_name,
            latest_version_id: d.latest_version_id,
        })
        .collect();
    Ok(Json(DatasetsResponse { datasets }))
}

#[derive(Serialize)]
pub struct VersionsResponse {
    versions: Vec<DatasetVersionListEntry>,
}

pub async fn list_dataset_versions(
    State(state): State<AppState>,
    Path(dataset_id): Path<Uuid>,
) -> Result<Json<VersionsResponse>, ServiceError> {
    let db = state.db.lock().unwrap();
    let versions = dataset_repo::list_versions_for_dataset(&db, dataset_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    let versions = versions
        .into_iter()
        .map(|v| DatasetVersionListEntry {
            id: v.id,
            fingerprint: v.fingerprint,
            created_at: v.created_at,
            image_count: v.image_asset_ids.len(),
            validation_summary: v.validation_summary,
        })
        .collect();
    Ok(Json(VersionsResponse { versions }))
}

#[derive(Serialize)]
pub struct ImagesResponse {
    images: Vec<ImageListEntry>,
}

pub async fn list_version_images(
    State(state): State<AppState>,
    Path(version_id): Path<Uuid>,
) -> Result<Json<ImagesResponse>, ServiceError> {
    let db = state.db.lock().unwrap();
    dataset_repo::get_dataset_version(&db, version_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?
        .ok_or(ServiceError::NotFound)?;
    let assets = dataset_repo::image_assets_for_version(&db, version_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?;

    let images = assets
        .into_iter()
        .map(|asset| {
            let reference_mask_status = match asset.reference_mask_id {
                None => "missing",
                Some(mask_id) => match dataset_repo::reference_mask_by_id(&db, mask_id) {
                    Ok(Some(mask)) if mask.validity == MaskValidity::Valid => "valid",
                    _ => "invalid",
                },
            };
            let display_name = asset
                .external_source_uri
                .rsplit('/')
                .next()
                .unwrap_or(&asset.external_source_uri)
                .to_string();
            ImageListEntry {
                id: asset.id,
                display_name,
                patient_id: asset.patient_id,
                split: asset.split,
                dimensions: asset.dimensions,
                source_status: asset.status,
                reference_mask_status,
            }
        })
        .collect();

    Ok(Json(ImagesResponse { images }))
}

pub async fn get_image_display(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
) -> Result<Json<ImageDisplayDescriptor>, ServiceError> {
    let db = state.db.lock().unwrap();
    let asset = dataset_repo::image_asset_by_id(&db, image_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?
        .ok_or(ServiceError::NotFound)?;
    let dataset_version_id = dataset_repo::latest_version_containing_image(&db, image_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?
        .ok_or(ServiceError::NotFound)?;
    let version = dataset_repo::get_dataset_version(&db, dataset_version_id)
        .map_err(|_| ServiceError::ServiceUnavailable)?
        .ok_or(ServiceError::NotFound)?;

    let source_status = check_source_availability(&asset);

    let (reference_mask_ref, reference_mask_unavailable_reason) = match asset.reference_mask_id {
        None => (
            None,
            Some("No reference mask available for this image.".to_string()),
        ),
        Some(mask_id) => match dataset_repo::reference_mask_by_id(&db, mask_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
        {
            Some(mask) if mask.validity == MaskValidity::Valid => (
                Some(state.register_artifact(mask.content_identity, ArtifactKind::Mask)),
                None,
            ),
            Some(mask) => (
                None,
                Some(match mask.validity {
                    MaskValidity::IncompatibleDimensions => {
                        "Reference mask dimensions do not match this image.".to_string()
                    }
                    MaskValidity::Unreadable => {
                        "Reference mask file could not be read.".to_string()
                    }
                    MaskValidity::Valid => unreachable!(),
                }),
            ),
            None => (None, Some("Reference mask record is missing.".to_string())),
        },
    };

    let image_artifact_ref =
        state.register_artifact(asset.imported_content_identity, ArtifactKind::Image);

    Ok(Json(ImageDisplayDescriptor {
        dataset_version_id,
        image_asset_id: image_id,
        display_metadata: DisplayMetadata {
            patient_id: asset.patient_id,
            split: asset.split,
            dimensions: asset.dimensions,
        },
        source_status,
        validation_summary: version.validation_summary,
        image_artifact_ref,
        reference_mask_ref,
        reference_mask_unavailable_reason,
    }))
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ThumbnailResponse {
    Ready {
        content_identity: String,
        data_base64: String,
    },
    Generating {
        request_id: Uuid,
    },
    Placeholder,
}

pub async fn get_thumbnail(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
) -> Result<impl IntoResponse, ServiceError> {
    let source_identity = {
        let db = state.db.lock().unwrap();
        let asset = dataset_repo::image_asset_by_id(&db, image_id)
            .map_err(|_| ServiceError::ServiceUnavailable)?
            .ok_or(ServiceError::NotFound)?;
        asset.imported_content_identity
    };

    let cached = {
        let db = state.db.lock().unwrap();
        thumbnail_repo::get(&db, &source_identity).map_err(|_| ServiceError::ServiceUnavailable)?
    };

    match cached {
        Some(row) if row.state == ThumbnailState::Ready => {
            let identity = row
                .content_identity
                .ok_or(ServiceError::ServiceUnavailable)?;
            let bytes = state
                .blob_store
                .read(&state.master_key, &identity)
                .map_err(|_| ServiceError::StaleReference)?;
            Ok((
                StatusCode::OK,
                Json(ThumbnailResponse::Ready {
                    content_identity: identity,
                    data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                }),
            ))
        }
        Some(row) if row.state == ThumbnailState::Generating => Ok((
            StatusCode::ACCEPTED,
            Json(ThumbnailResponse::Generating {
                request_id: Uuid::nil(),
            }),
        )),
        Some(row) if row.state == ThumbnailState::Placeholder => {
            Ok((StatusCode::OK, Json(ThumbnailResponse::Placeholder)))
        }
        _ => {
            // Cache miss or `stale`: kick off generation in the background
            // and return immediately (FR-047/FR-049); the caller polls or
            // waits for `thumbnail_ready` on the Event Bus.
            {
                let db = state.db.lock().unwrap();
                thumbnail_repo::upsert(&db, &source_identity, None, ThumbnailState::Generating)
                    .map_err(|_| ServiceError::ServiceUnavailable)?;
            }

            let request_id = Uuid::new_v4();
            let cancelled = Arc::new(AtomicBool::new(false));
            state
                .pending_requests
                .lock()
                .unwrap()
                .insert(request_id, cancelled.clone());

            let state_clone = state.clone();
            let source_identity_clone = source_identity.clone();
            tokio::spawn(async move {
                generate_thumbnail(state_clone, image_id, source_identity_clone, cancelled).await;
            });

            Ok((
                StatusCode::ACCEPTED,
                Json(ThumbnailResponse::Generating { request_id }),
            ))
        }
    }
}

async fn generate_thumbnail(
    state: AppState,
    image_id: Uuid,
    source_identity: String,
    cancelled: Arc<AtomicBool>,
) {
    let result = tokio::task::spawn_blocking({
        let state = state.clone();
        let source_identity = source_identity.clone();
        move || -> Result<String, ()> {
            let source_bytes = state
                .blob_store
                .read(&state.master_key, &source_identity)
                .map_err(|_| ())?;
            let decoded = image::load_from_memory(&source_bytes).map_err(|_| ())?;
            let thumb_bytes = render_thumbnail_png(&decoded);
            state
                .blob_store
                .write(&state.master_key, &thumb_bytes)
                .map_err(|_| ())
        }
    })
    .await;

    if cancelled.load(Ordering::SeqCst) {
        return;
    }

    let db = state.db.lock().unwrap();
    match result {
        Ok(Ok(thumb_identity)) => {
            let _ = thumbnail_repo::upsert(
                &db,
                &source_identity,
                Some(&thumb_identity),
                ThumbnailState::Ready,
            );
            drop(db);
            let _ = state.event_tx.send(Event::new(
                image_id,
                EventPayload::ThumbnailReady {
                    image_asset_id: image_id,
                    source_content_identity: source_identity,
                },
            ));
        }
        _ => {
            let _ =
                thumbnail_repo::upsert(&db, &source_identity, None, ThumbnailState::Placeholder);
        }
    }
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/datasets", get(list_datasets))
        .route("/datasets/:dataset_id/versions", get(list_dataset_versions))
        .route(
            "/dataset-versions/:version_id/images",
            get(list_version_images),
        )
        .route("/image-assets/:image_id/display", get(get_image_display))
        .route("/image-assets/:image_id/thumbnail", get(get_thumbnail))
}

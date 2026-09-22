//! `POST /import-batches`, `/confirm`, `/cancel` (contracts/local-service-api.md
//! §Import, User Story 1). Preview performs zero writes; confirm atomically
//! creates exactly one new `DatasetVersion` + its `ImageAsset` rows, or
//! none at all on any failure (FR-044, SC-019).

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::AppState;
use crate::data_engine::dataset_version::{compute_fingerprint, FingerprintInput};
use crate::data_engine::import::{
    candidate_counts, classify_candidates, parse_manifest, render_grayscale_png, scan_folder,
    validate_readable, ScannedFile, DEFAULT_MAX_SCAN_DEPTH, DEFAULT_MAX_SCAN_FILES,
};
use crate::data_engine::validation::validate_dataset_version;
use crate::data_repository::sqlite::dataset_repo;
use crate::domain::dataset::{
    CandidateClassification, Dimensions, ImageAsset, ImageAssetStatus, ImportBatch,
    ImportBatchStatus, MaskValidity, MetadataStatus, ReferenceMask, SourceSelection,
    ValidationStatus,
};
use crate::domain::ServiceError;

/// In-memory pending batches (previewing/confirmed/cancelled) — the batch
/// itself is not durable data; only a `confirmed` batch's resulting
/// `DatasetVersion`/`ImageAsset` rows are (data-model.md ImportBatch).
pub type PendingBatches = Mutex<HashMap<Uuid, ImportBatch>>;

#[derive(Deserialize)]
pub struct CreateImportBatchRequest {
    pub source_selection: SourceSelection,
    pub paths: Vec<String>,
    pub metadata_manifest: Option<serde_json::Value>,
    #[serde(default)]
    pub dataset_display_name: Option<String>,
}

#[derive(Serialize)]
pub struct ImportBatchResponse {
    pub batch_id: Uuid,
    pub status: &'static str,
    pub candidates: Vec<crate::domain::dataset::ImportCandidate>,
    pub counts: HashMap<&'static str, usize>,
}

pub async fn create_import_batch(
    State(state): State<AppState>,
    Json(req): Json<CreateImportBatchRequest>,
) -> Result<Json<ImportBatchResponse>, ServiceError> {
    let manifest = match &req.metadata_manifest {
        Some(value) => {
            let json = serde_json::to_string(value).map_err(|_| ServiceError::InvalidMask)?;
            Some(parse_manifest(&json).map_err(|_| ServiceError::InvalidMask)?)
        }
        None => None,
    };

    let files: Vec<ScannedFile> = match req.source_selection {
        SourceSelection::SingleFile | SourceSelection::MultiFile => req
            .paths
            .iter()
            .map(|p| ScannedFile {
                source_ref: std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone()),
                path: p.into(),
            })
            .collect(),
        SourceSelection::FolderScan => {
            let root = req.paths.first().ok_or(ServiceError::InvalidMask)?;
            let root_path = std::path::PathBuf::from(root);
            scan_folder(&root_path, DEFAULT_MAX_SCAN_FILES, DEFAULT_MAX_SCAN_DEPTH)
                .unwrap_or_default()
                .into_iter()
                .map(|path| {
                    let source_ref = path
                        .strip_prefix(&root_path)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();
                    ScannedFile { source_ref, path }
                })
                .collect()
        }
    };

    let existing_identities = {
        let db = state.db.lock().unwrap();
        dataset_repo::all_known_content_identities(&db)
            .map_err(|_| ServiceError::ServiceUnavailable)?
    };

    let candidates = classify_candidates(&files, &existing_identities, manifest.as_ref());
    let counts = candidate_counts(&candidates);

    let batch_id = Uuid::new_v4();
    let batch = ImportBatch {
        id: batch_id,
        source_selection: req.source_selection,
        candidates: candidates.clone(),
        metadata_manifest_id: None,
        status: ImportBatchStatus::Previewing,
    };

    // Stash the resolved file paths + dataset name alongside the batch so
    // `confirm` can re-run without re-scanning; keyed by source_ref.
    let path_by_ref: HashMap<String, std::path::PathBuf> =
        files.into_iter().map(|f| (f.source_ref, f.path)).collect();
    state.pending_batch_paths.lock().unwrap().insert(
        batch_id,
        (
            path_by_ref,
            req.dataset_display_name
                .unwrap_or_else(|| "Default Dataset".into()),
        ),
    );

    state
        .pending_batches
        .lock()
        .unwrap()
        .insert(batch_id, batch);

    Ok(Json(ImportBatchResponse {
        batch_id,
        status: "previewing",
        candidates,
        counts,
    }))
}

#[derive(Deserialize)]
pub struct ConfirmImportBatchRequest {
    pub confirmed_source_refs: Vec<String>,
}

#[derive(Serialize)]
pub struct ConfirmImportBatchResponse {
    pub batch_id: Uuid,
    pub status: &'static str,
    pub dataset_version_id: Option<Uuid>,
    pub dataset_id: Option<Uuid>,
}

pub async fn confirm_import_batch(
    State(state): State<AppState>,
    Path(batch_id): Path<Uuid>,
    Json(req): Json<ConfirmImportBatchRequest>,
) -> Result<Json<ConfirmImportBatchResponse>, ServiceError> {
    let batch = {
        let mut batches = state.pending_batches.lock().unwrap();
        let batch = batches
            .get(&batch_id)
            .cloned()
            .ok_or(ServiceError::NotFound)?;
        if batch.status != ImportBatchStatus::Previewing {
            return Err(ServiceError::Conflict);
        }
        batches.get_mut(&batch_id).unwrap().status = ImportBatchStatus::Confirmed;
        batch
    };

    let (path_by_ref, dataset_name) = state
        .pending_batch_paths
        .lock()
        .unwrap()
        .get(&batch_id)
        .cloned()
        .ok_or(ServiceError::NotFound)?;

    let result = try_confirm(
        &state,
        &batch,
        &req.confirmed_source_refs,
        &path_by_ref,
        &dataset_name,
    );

    match result {
        Ok((dataset_id, dataset_version_id)) => Ok(Json(ConfirmImportBatchResponse {
            batch_id,
            status: "confirmed",
            dataset_version_id: Some(dataset_version_id),
            dataset_id: Some(dataset_id),
        })),
        Err(e) => {
            state
                .pending_batches
                .lock()
                .unwrap()
                .get_mut(&batch_id)
                .unwrap()
                .status = ImportBatchStatus::Failed;
            Err(e)
        }
    }
}

/// All writes for one confirm happen inside a single SQLite transaction:
/// either the new `DatasetVersion` + all its `ImageAsset` rows land
/// together, or (on any error) nothing is committed at all (FR-044,
/// SC-019).
fn try_confirm(
    state: &AppState,
    batch: &ImportBatch,
    confirmed_source_refs: &[String],
    path_by_ref: &HashMap<String, std::path::PathBuf>,
    dataset_name: &str,
) -> Result<(Uuid, Uuid), ServiceError> {
    let mut db = state.db.lock().unwrap();
    let tx = db
        .transaction()
        .map_err(|_| ServiceError::ServiceUnavailable)?;

    let dataset = dataset_repo::get_or_create_dataset(&tx, state.project_id, dataset_name)
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    let previous_version = dataset_repo::get_latest_dataset_version(&tx, dataset.id)
        .map_err(|_| ServiceError::ServiceUnavailable)?;

    let mut new_assets: Vec<ImageAsset> = Vec::new();

    for candidate in &batch.candidates {
        if candidate.classification != CandidateClassification::Importable {
            continue;
        }
        if !confirmed_source_refs.contains(&candidate.source_ref) {
            continue;
        }
        let path = path_by_ref
            .get(&candidate.source_ref)
            .ok_or(ServiceError::NotFound)?;
        let (image, source_bytes) =
            validate_readable(path).map_err(|_| ServiceError::SourceMissing)?;
        let source_identity = crate::domain::content_identity::content_identity(&source_bytes);
        let grayscale = render_grayscale_png(&image);
        let imported_identity = state
            .blob_store
            .write(&state.master_key, &grayscale)
            .map_err(|_| ServiceError::ServiceUnavailable)?;

        if let Some(patient_id) = &candidate.resolved_patient_id {
            dataset_repo::ensure_research_subject(&tx, dataset.id, patient_id)
                .map_err(|_| ServiceError::ServiceUnavailable)?;
        }

        let metadata_status =
            if candidate.resolved_patient_id.is_some() && candidate.resolved_split.is_some() {
                MetadataStatus::Complete
            } else {
                MetadataStatus::Incomplete
            };

        let image_id = Uuid::new_v4();
        let reference_mask_id = match &candidate.resolved_mask_ref {
            Some(mask_ref) => Some(
                import_reference_mask(
                    state,
                    &tx,
                    path,
                    mask_ref,
                    image.width(),
                    image.height(),
                    image_id,
                )
                .map_err(|_| ServiceError::InvalidMask)?,
            ),
            None => None,
        };

        let asset = ImageAsset {
            id: image_id,
            external_source_uri: format!("file://{}", path.display()),
            source_content_identity: source_identity,
            imported_content_identity: imported_identity,
            dimensions: Dimensions {
                width: image.width(),
                height: image.height(),
            },
            source_created_at: None,
            imported_at: Utc::now().to_rfc3339(),
            status: ImageAssetStatus::Available,
            patient_id: candidate.resolved_patient_id.clone(),
            split: candidate.resolved_split,
            reference_mask_id,
            metadata_status,
        };
        dataset_repo::insert_image_asset(&tx, &asset)
            .map_err(|_| ServiceError::ServiceUnavailable)?;
        new_assets.push(asset);
    }

    if new_assets.is_empty() {
        return Err(ServiceError::Conflict);
    }

    let mut all_assets = dataset_repo::image_assets_for_version(
        &tx,
        previous_version
            .as_ref()
            .map(|v| v.id)
            .unwrap_or(Uuid::nil()),
    )
    .unwrap_or_default();
    all_assets.extend(new_assets.iter().cloned());

    let fingerprint_inputs: Vec<FingerprintInput> = all_assets
        .iter()
        .map(|a| FingerprintInput::from_image_asset(a, None))
        .collect();
    let fingerprint = compute_fingerprint(fingerprint_inputs);

    if let Some(prev) = &previous_version {
        if prev.fingerprint == fingerprint {
            return Err(ServiceError::Conflict);
        }
    }

    let version_id = Uuid::new_v4();
    let findings = validate_dataset_version(version_id, &all_assets, &[]);
    let has_blocking = findings
        .iter()
        .any(|f| f.severity == crate::domain::dataset::Severity::Blocking);
    let status = if has_blocking {
        ValidationStatus::Blocked
    } else if !findings.is_empty() {
        ValidationStatus::Warned
    } else {
        ValidationStatus::Ok
    };

    let version = crate::domain::dataset::DatasetVersion {
        id: version_id,
        dataset_id: dataset.id,
        fingerprint,
        derived_from_version_id: previous_version.map(|v| v.id),
        created_at: Utc::now().to_rfc3339(),
        image_asset_ids: all_assets.iter().map(|a| a.id).collect(),
        validation_summary: crate::domain::dataset::ValidationSummary {
            status,
            finding_ids: findings.iter().map(|f| f.id).collect(),
        },
    };
    dataset_repo::insert_dataset_version(&tx, &version)
        .map_err(|_| ServiceError::ServiceUnavailable)?;
    for finding in &findings {
        dataset_repo::insert_validation_finding(&tx, finding)
            .map_err(|_| ServiceError::ServiceUnavailable)?;
    }

    tx.commit().map_err(|_| ServiceError::ServiceUnavailable)?;
    Ok((dataset.id, version_id))
}

pub async fn cancel_import_batch(
    State(state): State<AppState>,
    Path(batch_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ServiceError> {
    let mut batches = state.pending_batches.lock().unwrap();
    let batch = batches.get_mut(&batch_id).ok_or(ServiceError::NotFound)?;
    batch.status = ImportBatchStatus::Cancelled;
    Ok(Json(
        serde_json::json!({ "batch_id": batch_id, "status": "cancelled" }),
    ))
}

/// Resolves a manifest `reference_mask_ref` relative to its image's own
/// directory, checks dimension compatibility (FR-038), and persists a
/// `ReferenceMask` row regardless of validity — so an invalid/missing mask
/// is recorded as such (surfaced later as `reference_mask_unavailable_reason`)
/// rather than silently left unlinked.
fn import_reference_mask(
    state: &AppState,
    conn: &rusqlite::Connection,
    image_path: &std::path::Path,
    mask_ref: &str,
    image_width: u32,
    image_height: u32,
    image_id: Uuid,
) -> Result<Uuid, crate::data_engine::import::ImportError> {
    let mask_path = image_path
        .parent()
        .map(|dir| dir.join(mask_ref))
        .unwrap_or_else(|| std::path::PathBuf::from(mask_ref));

    let mask_id = Uuid::new_v4();
    let (content_identity, dimensions, validity) = match validate_readable(&mask_path) {
        Ok((mask_image, mask_bytes)) => {
            let identity =
                if mask_image.width() == image_width && mask_image.height() == image_height {
                    state
                        .blob_store
                        .write(&state.master_key, &mask_bytes)
                        .unwrap_or_else(|_| {
                            crate::domain::content_identity::content_identity(&mask_bytes)
                        })
                } else {
                    crate::domain::content_identity::content_identity(&mask_bytes)
                };
            let validity =
                if mask_image.width() == image_width && mask_image.height() == image_height {
                    MaskValidity::Valid
                } else {
                    MaskValidity::IncompatibleDimensions
                };
            (
                identity,
                Dimensions {
                    width: mask_image.width(),
                    height: mask_image.height(),
                },
                validity,
            )
        }
        Err(_) => (
            String::new(),
            Dimensions {
                width: 0,
                height: 0,
            },
            MaskValidity::Unreadable,
        ),
    };

    let mask = ReferenceMask {
        id: mask_id,
        content_identity,
        dimensions,
        compatible_with_image_id: image_id,
        validity,
    };
    dataset_repo::insert_reference_mask(conn, &mask)
        .map_err(|e| crate::data_engine::import::ImportError::Unreadable(e.to_string()))?;
    Ok(mask_id)
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/import-batches", post(create_import_batch))
        .route(
            "/import-batches/:batch_id/confirm",
            post(confirm_import_batch),
        )
        .route(
            "/import-batches/:batch_id/cancel",
            post(cancel_import_batch),
        )
}

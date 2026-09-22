//! `ImageDisplayDescriptor` (data-model.md) — read-only, composed on
//! request from `ImageAsset` + `DatasetVersion` + `ValidationFinding` +
//! `ArtifactReference`s. Never includes full pixel data (FR-036).

use serde::Serialize;
use uuid::Uuid;

use super::dataset::{Dimensions, ImageAssetStatus, Split, ValidationSummary};
use super::ArtifactReference;

#[derive(Debug, Clone, Serialize)]
pub struct DisplayMetadata {
    pub patient_id: Option<String>,
    pub split: Option<Split>,
    pub dimensions: Dimensions,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageDisplayDescriptor {
    pub dataset_version_id: Uuid,
    pub image_asset_id: Uuid,
    pub display_metadata: DisplayMetadata,
    pub source_status: ImageAssetStatus,
    pub validation_summary: ValidationSummary,
    pub image_artifact_ref: ArtifactReference,
    pub reference_mask_ref: Option<ArtifactReference>,
    pub reference_mask_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetVersionListEntry {
    pub id: Uuid,
    pub fingerprint: String,
    pub created_at: String,
    pub image_count: usize,
    pub validation_summary: ValidationSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageListEntry {
    pub id: Uuid,
    pub display_name: String,
    pub patient_id: Option<String>,
    pub split: Option<Split>,
    pub dimensions: Dimensions,
    pub source_status: ImageAssetStatus,
    pub reference_mask_status: &'static str,
}

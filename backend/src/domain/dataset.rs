//! Entities for User Story 1 (data-model.md §Dataset…MetadataManifest).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dataset {
    pub id: Uuid,
    pub project_id: Uuid,
    pub display_name: String,
    pub latest_version_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Ok,
    Blocked,
    Warned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub status: ValidationStatus,
    pub finding_ids: Vec<Uuid>,
}

/// Immutable once created — a "change" is always a new row with
/// `derived_from_version_id` set (FR-007, FR-032).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetVersion {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub fingerprint: String,
    pub derived_from_version_id: Option<Uuid>,
    pub created_at: String,
    pub image_asset_ids: Vec<Uuid>,
    pub validation_summary: ValidationSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageAssetStatus {
    Available,
    SourceMissing,
    SourceChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Validation,
    Test,
}

impl Split {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "train" => Some(Self::Train),
            "validation" => Some(Self::Validation),
            "test" => Some(Self::Test),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Validation => "validation",
            Split::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataStatus {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

/// Stable identity independent of any `DatasetVersion` membership
/// (data-model.md ImageAsset). The `external_source_uri` file is never
/// copied — only linked (FR-002).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAsset {
    pub id: Uuid,
    pub external_source_uri: String,
    pub source_content_identity: String,
    pub imported_content_identity: String,
    pub dimensions: Dimensions,
    pub source_created_at: Option<String>,
    pub imported_at: String,
    pub status: ImageAssetStatus,
    pub patient_id: Option<String>,
    pub split: Option<Split>,
    pub reference_mask_id: Option<Uuid>,
    pub metadata_status: MetadataStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSubject {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub deidentified_patient_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskValidity {
    Valid,
    IncompatibleDimensions,
    Unreadable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceMask {
    pub id: Uuid,
    pub content_identity: String,
    pub dimensions: Dimensions,
    pub compatible_with_image_id: Uuid,
    pub validity: MaskValidity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSelection {
    SingleFile,
    MultiFile,
    FolderScan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportBatchStatus {
    Previewing,
    Confirmed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateClassification {
    Importable,
    Skipped,
    Duplicate,
    Unreadable,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestMatch {
    Matched,
    Unmatched,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub source_ref: String,
    pub classification: CandidateClassification,
    pub resolved_patient_id: Option<String>,
    pub resolved_split: Option<Split>,
    pub resolved_mask_ref: Option<String>,
    pub manifest_match: ManifestMatch,
    /// Human-readable reason for `unreadable`/`unsupported`/`duplicate`
    /// classifications — surfaced to the frontend Import Batch Preview.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatch {
    pub id: Uuid,
    pub source_selection: SourceSelection,
    pub candidates: Vec<ImportCandidate>,
    pub metadata_manifest_id: Option<Uuid>,
    pub status: ImportBatchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataManifestEntry {
    pub source_ref: String,
    pub patient_id: Option<String>,
    pub split: Option<String>,
    pub reference_mask_ref: Option<String>,
}

/// A versioned JSON mapping (research.md §6) — never inferred from
/// filenames or folder names (FR-046).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataManifest {
    pub id: Uuid,
    pub manifest_version: String,
    pub entries: Vec<MetadataManifestEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    PatientSplitLeakage,
    MissingPatientId,
    MissingOrIncompatibleMask,
    DuplicateContent,
    IncompleteMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationFinding {
    pub id: Uuid,
    pub dataset_version_id: Uuid,
    pub category: FindingCategory,
    pub severity: Severity,
    pub affected_image_asset_ids: Vec<Uuid>,
}

//! Entities for User Story 4 (data-model.md §PipelineSnapshot…RunRecord…
//! MetricSet). A `RunRecord` is the durable, fully-traceable evidence unit
//! for an official experiment — never confused with the volatile Preview
//! path (Constitution Principle III, FR-013).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "succeeded" => RunStatus::Succeeded,
            "failed" => RunStatus::Failed,
            "cancelled" => RunStatus::Cancelled,
            _ => RunStatus::Running,
        }
    }
}

/// Part of the Run Record itself (FR-019): whether intermediate artifacts
/// are retained is an explicit, recorded policy, not an implementation
/// detail.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RunPolicy {
    pub retain_intermediates: bool,
}

/// The actual grayscale bytes fed to the pipeline for one Run (data-model.md
/// RunInputArtifact). The link to `source_image_asset_id` is retained even
/// if that `ImageAsset` later becomes `source_missing` — an existing Run
/// must stay reproducible from its own recorded input (FR-019, Edge Cases).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInputArtifact {
    pub id: Uuid,
    pub content_identity: String,
    pub source_image_asset_id: Uuid,
    pub created_at: String,
}

/// data-model.md RunRecord. Transitions to `succeeded` only inside the same
/// storage transaction that persists `output_content_identities` and
/// `metric_set_id` (FR-020) — see `data_repository::sqlite::run_repo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: Uuid,
    pub status: RunStatus,
    pub dataset_version_id: Uuid,
    pub dataset_fingerprint: String,
    pub image_asset_id: Uuid,
    pub image_asset_identity: String,
    pub run_input_artifact_id: Uuid,
    pub pipeline_snapshot_id: Uuid,
    pub target_node_id: String,
    pub seed: u64,
    pub node_versions: BTreeMap<String, String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// Final output artifacts only, by content identity (an `ArtifactReference`
    /// is composed on request from these, same as every other read path in
    /// this service — see data-model.md ImageDisplayDescriptor). Empty while
    /// `status == running`.
    pub output_content_identities: Vec<String>,
    pub run_policy: RunPolicy,
    pub metric_set_id: Option<Uuid>,
    /// Present only when `status == failed`; MUST NOT include raw image
    /// content or non-essential subject info (FR-021).
    pub error_summary: Option<String>,
    pub failed_stage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSet {
    pub id: Uuid,
    pub run_record_id: Uuid,
    /// Null when no valid reference mask (FR-038) — also null for now
    /// because reference-based scoring belongs to the future Algorithm
    /// Layer, out of this Data Layer feature's scope (plan.md Project
    /// Structure).
    pub dice: Option<f64>,
    pub area_mm2: Option<f64>,
    pub foreground_pixels: Option<i64>,
    pub connected_components: Option<i64>,
    pub step_timings: BTreeMap<String, u64>,
}

//! Extraction candidates (data-model.md ExtractionCandidate). Every proposed
//! field is either **anchored** to a source span or absent with an `Ambiguity`
//! recorded — there is no code path that fills a default (FR-026, SC-007).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::pdf::PageText;

/// FR-023's nine classes, plus the auto-tag for unsafe clinical claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    DataEligibility,
    Preprocessing,
    ModelInference,
    Postprocessing,
    LandmarkExtraction,
    Measurement,
    DecisionRule,
    ValidationOnly,
    RejectedContent,
    /// A diagnosis / treatment statement, not a reproducible analysis step.
    /// May only be marked non-executable or rejected (FR-033).
    ClinicalClaim,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::DataEligibility => "data_eligibility",
            Category::Preprocessing => "preprocessing",
            Category::ModelInference => "model_inference",
            Category::Postprocessing => "postprocessing",
            Category::LandmarkExtraction => "landmark_extraction",
            Category::Measurement => "measurement",
            Category::DecisionRule => "decision_rule",
            Category::ValidationOnly => "validation_only",
            Category::RejectedContent => "rejected_content",
            Category::ClinicalClaim => "clinical_claim",
        }
    }
}

/// Where a statement came from. Offsets index the page's extracted text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSpan {
    pub page: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub quote: String,
    pub start: usize,
    pub end: usize,
}

/// One proposed input / output / parameter / unit / assumption / formula.
/// `value` and `unit` are only ever present when `source_span` is, and both
/// literally occur in its quote.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub name: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub source_span: Option<SourceSpan>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Proposed {
    #[serde(default)]
    pub inputs: Vec<Item>,
    #[serde(default)]
    pub outputs: Vec<Item>,
    #[serde(default)]
    pub parameters: Vec<Item>,
    #[serde(default)]
    pub units: Vec<Item>,
    #[serde(default)]
    pub assumptions: Vec<Item>,
    #[serde(default)]
    pub formula: Option<Item>,
}

/// Something the source does not settle. Never resolved by guessing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ambiguity {
    /// `missing_unit | unstated_preprocessing | formula_unparsed |
    /// parameter_value_absent | conflicting_values | category_uncertain | unstated_io`
    pub code: String,
    pub field: String,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Proposed,
    Accepted,
    Edited,
    Mapped,
    NonExecutable,
    Rejected,
    Deferred,
}

impl CandidateState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateState::Proposed => "proposed",
            CandidateState::Accepted => "accepted",
            CandidateState::Edited => "edited",
            CandidateState::Mapped => "mapped",
            CandidateState::NonExecutable => "non_executable",
            CandidateState::Rejected => "rejected",
            CandidateState::Deferred => "deferred",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "proposed" => CandidateState::Proposed,
            "accepted" => CandidateState::Accepted,
            "edited" => CandidateState::Edited,
            "mapped" => CandidateState::Mapped,
            "non_executable" => CandidateState::NonExecutable,
            "rejected" => CandidateState::Rejected,
            "deferred" => CandidateState::Deferred,
            _ => return None,
        })
    }
}

/// A proposal before it has an identity or state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposedCandidate {
    pub category: Category,
    /// ≥ 1; several when multiple passages support one candidate (US5-4).
    pub sources: Vec<SourceSpan>,
    pub proposed: Proposed,
    pub ambiguities: Vec<Ambiguity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionCandidate {
    pub candidate_id: Uuid,
    pub paper_id: Uuid,
    /// The extraction pass that produced it.
    pub run_id: Uuid,
    pub category: Category,
    pub sources: Vec<SourceSpan>,
    pub proposed: Proposed,
    pub ambiguities: Vec<Ambiguity>,
    pub state: CandidateState,
    /// Fixed `paper_derived`; carried onto any draft made from it (FR-027).
    pub origin: String,
}

/// Offline, deterministic proposal of candidates from paper text. The trait
/// boundary lets a local model-based proposer be added later without touching
/// storage or the review flow; nothing here may use the network.
pub trait CandidateProposer {
    fn propose(&self, pages: &[PageText]) -> Vec<ProposedCandidate>;
}

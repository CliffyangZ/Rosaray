//! Wire types, limits and shared helpers of `system-one/1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::profile::ImageFacts;

pub const PROTOCOL_VERSION: &str = "system-one/1";
pub const MAX_PURPOSE_CHARS: usize = 200;
pub const MAX_REASON_CHARS: usize = 2000;
pub const DEFAULT_MAX_CANDIDATES: u32 = 10;
pub const HARD_MAX_CANDIDATES: u32 = 50;

/// Failures of the exchange. `code()` is the stable wire code.
#[derive(Debug, thiserror::Error)]
pub enum SoError {
    #[error("malformed: {0}")]
    Malformed(String),
    #[error("protocol_incompatible")]
    ProtocolIncompatible,
    #[error("rejected_content: {0}")]
    RejectedContent(String),
    #[error("unknown_request")]
    UnknownRequest,
    #[error("access_denied")]
    AccessDenied,
    #[error("conflicting_request")]
    ConflictingRequest,
    #[error("conflicting_report")]
    ConflictingReport,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl SoError {
    pub fn code(&self) -> &'static str {
        match self {
            SoError::Malformed(_) => "malformed",
            SoError::ProtocolIncompatible => "protocol_incompatible",
            SoError::RejectedContent(_) => "rejected_content",
            SoError::UnknownRequest => "unknown_request",
            SoError::AccessDenied => "access_denied",
            SoError::ConflictingRequest => "conflicting_request",
            SoError::ConflictingReport => "conflicting_report",
            SoError::Db(_) | SoError::Io(_) => "internal",
        }
    }
}

/// Only the major part must match (`system-one/1`, `system-one/1.2`, …).
pub fn check_version(value: &Value) -> Result<(), SoError> {
    let v = value
        .get("protocol_version")
        .and_then(Value::as_str)
        .ok_or_else(|| SoError::Malformed("protocol_version is required".into()))?;
    let Some(rest) = v.strip_prefix("system-one/") else { return Err(SoError::ProtocolIncompatible) };
    let major = rest.split('.').next().unwrap_or("");
    if major == "1" {
        Ok(())
    } else {
        Err(SoError::ProtocolIncompatible)
    }
}

/// Runs the privacy guard on the whole payload.
pub fn guard(value: &Value) -> Result<(), SoError> {
    crate::guard::check_payload(value).map_err(|r| SoError::RejectedContent(r.describe()))
}

/// Canonical hash of a request/report body (idempotency).
pub fn canonical_hash(value: &Value) -> String {
    format!("b3:{}", blake3::hash(&crate::identity::canonical_json(value)).to_hex())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spacing {
    pub x: f64,
    pub y: f64,
}

/// Declared facts about the data a method would be applied to. Closed
/// vocabulary, all optional; an unstated fact is *unknown* and never
/// satisfies a requirement (nothing is assumed).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DataConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixel_spacing_mm: Option<Spacing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_present: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_space: Option<String>,
}

impl DataConditions {
    pub fn to_facts(&self) -> ImageFacts {
        ImageFacts {
            format: self.format.clone(),
            width: self.width_px.unwrap_or(0),
            height: self.height_px.unwrap_or(0),
            pixel_spacing_mm: self.pixel_spacing_mm.as_ref().map(|s| (s.x, s.y)),
            mask_present: self.mask_present.unwrap_or(false),
            color_mode: self.color_mode.clone(),
            modality: self.modality.clone(),
            coordinate_space: self.coordinate_space.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    pub protocol_version: String,
    pub request_id: uuid::Uuid,
    pub task_purpose: String,
    #[serde(default)]
    pub data_conditions: DataConditions,
    #[serde(default)]
    pub max_candidates: Option<u32>,
    #[serde(default)]
    pub include_inapplicable: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VersionRef {
    pub id: String,
    pub version: String,
    pub content_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    pub protocol_version: String,
    pub request_id: uuid::Uuid,
    /// `selected | abstained`
    pub decision: String,
    #[serde(default)]
    pub selection: Option<VersionRef>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Applicability {
    /// `applicable | not_applicable`
    pub result: &'static str,
    pub reasons: Vec<Value>,
}

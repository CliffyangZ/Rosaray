pub mod content_identity;
pub mod dataset;
pub mod display;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub display_name: String,
    pub created_at: String,
    pub contract_version: String,
}

/// Metadata used to request artifact content through the API — never a
/// vehicle for embedding raw pixel bytes directly (data-model.md Artifact /
/// ArtifactReference).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactReference {
    pub id: Uuid,
    pub content_identity: String,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Image,
    Mask,
    Measurement,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPersistence {
    Preview,
    Official,
}

/// Error taxonomy matching contracts/local-service-api.md's error codes.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ServiceError {
    #[error("not found")]
    NotFound,
    #[error("stale reference")]
    StaleReference,
    #[error("source missing")]
    SourceMissing,
    #[error("source changed")]
    SourceChanged,
    #[error("invalid mask")]
    InvalidMask,
    #[error("credential required")]
    CredentialRequired,
    #[error("credential invalid")]
    CredentialInvalid,
    #[error("conflict")]
    Conflict,
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("access denied")]
    AccessDenied,
}

impl ServiceError {
    pub fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            ServiceError::NotFound => StatusCode::NOT_FOUND,
            ServiceError::StaleReference => StatusCode::GONE,
            ServiceError::SourceMissing | ServiceError::SourceChanged => StatusCode::CONFLICT,
            ServiceError::InvalidMask => StatusCode::UNPROCESSABLE_ENTITY,
            ServiceError::CredentialRequired | ServiceError::CredentialInvalid => {
                StatusCode::UNAUTHORIZED
            }
            ServiceError::Conflict => StatusCode::CONFLICT,
            ServiceError::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ServiceError::AccessDenied => StatusCode::UNAUTHORIZED,
        }
    }
}

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let status = self.http_status();
        let body = serde_json::json!({ "error": &self });
        (status, axum::Json(body)).into_response()
    }
}

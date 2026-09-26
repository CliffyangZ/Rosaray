//! Wire errors: `{ "error": { "code", "message", "findings"? } }`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rosaray_qkb::author::AuthorError;
use rosaray_qkb::cleanup::CleanupError;
use rosaray_qkb::submit::SubmitError;
use rosaray_qkb::system_one::SoError;
use serde_json::{json, Value};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub findings: Option<Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), findings: None }
    }
    pub fn access_denied() -> Self {
        Self::new(StatusCode::FORBIDDEN, "access_denied", "access denied")
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "not found")
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "malformed", message)
    }
    /// Generic on purpose: internals are never echoed to the caller.
    pub fn internal(_detail: &str) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "internal error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut err = json!({ "code": self.code, "message": self.message });
        if let Some(f) = self.findings {
            err["findings"] = f;
        }
        (self.status, Json(json!({ "error": err }))).into_response()
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(_: rusqlite::Error) -> Self {
        ApiError::internal("database")
    }
}
impl From<std::io::Error> for ApiError {
    fn from(_: std::io::Error) -> Self {
        ApiError::internal("io")
    }
}
impl From<CleanupError> for ApiError {
    fn from(_: CleanupError) -> Self {
        ApiError::internal("cleanup")
    }
}

impl From<SubmitError> for ApiError {
    fn from(e: SubmitError) -> Self {
        match e {
            SubmitError::Rejected(findings) => {
                let mut err = ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "submission_rejected", "the submission was rejected; nothing was stored");
                err.findings = serde_json::to_value(findings).ok();
                err
            }
            SubmitError::VersionExists => ApiError::new(StatusCode::CONFLICT, "version_exists", "this id@version already exists with different content"),
            SubmitError::RejectedContent(why) => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "rejected_content", why),
            SubmitError::Io(_) | SubmitError::Db(_) | SubmitError::Internal(_) => ApiError::internal("submit"),
        }
    }
}

impl From<AuthorError> for ApiError {
    fn from(e: AuthorError) -> Self {
        match e {
            AuthorError::NotFound => ApiError::not_found(),
            AuthorError::VerificationInvalid(m) => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "verification_invalid", m),
            _ => ApiError::internal("author"),
        }
    }
}

impl From<SoError> for ApiError {
    fn from(e: SoError) -> Self {
        let code = e.code();
        match e {
            SoError::Malformed(m) => ApiError::new(StatusCode::BAD_REQUEST, code, m),
            SoError::ProtocolIncompatible => ApiError::new(StatusCode::UPGRADE_REQUIRED, code, "unsupported protocol version; this service speaks system-one/1"),
            SoError::RejectedContent(m) => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, m),
            SoError::UnknownRequest => ApiError::new(StatusCode::NOT_FOUND, code, "unknown request_id"),
            SoError::AccessDenied => ApiError::access_denied(),
            SoError::ConflictingRequest | SoError::ConflictingReport => ApiError::new(StatusCode::CONFLICT, code, "a different body was already recorded under this request_id"),
            SoError::Db(_) | SoError::Io(_) => ApiError::internal("system_one"),
        }
    }
}

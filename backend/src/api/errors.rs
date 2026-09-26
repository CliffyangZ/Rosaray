//! Error taxonomy glue (contracts/local-service-api.md §Conventions): the
//! wire shape `{ "error": { "code", "message" } }` and a machine check that
//! `ServiceError`, its HTTP statuses, and the contract's documented code
//! list cannot drift apart.

use crate::domain::ServiceError;

/// Every code a handler may return, in the contract's spelling.
pub const ALL_CODES: &[&str] = &[
    "not_found",
    "stale_reference",
    "source_missing",
    "source_changed",
    "invalid_mask",
    "credential_required",
    "credential_invalid",
    "conflict",
    "service_unavailable",
    "access_denied",
    "bundle_tampered",
    "bundle_incompatible",
];

impl ServiceError {
    /// Stable wire code, identical to the serde tag.
    pub fn code(&self) -> &'static str {
        match self {
            ServiceError::NotFound => "not_found",
            ServiceError::StaleReference => "stale_reference",
            ServiceError::SourceMissing => "source_missing",
            ServiceError::SourceChanged => "source_changed",
            ServiceError::InvalidMask => "invalid_mask",
            ServiceError::CredentialRequired => "credential_required",
            ServiceError::CredentialInvalid => "credential_invalid",
            ServiceError::Conflict => "conflict",
            ServiceError::ServiceUnavailable => "service_unavailable",
            ServiceError::AccessDenied => "access_denied",
            ServiceError::BundleTampered => "bundle_tampered",
            ServiceError::BundleIncompatible => "bundle_incompatible",
        }
    }

    /// Response body per the contract: `code` plus a human-readable
    /// `message`. Messages are generic on purpose — they never echo
    /// request content, so they cannot leak bundle or research data.
    pub fn body(&self) -> serde_json::Value {
        serde_json::json!({ "error": { "code": self.code(), "message": self.to_string() } })
    }
}

// ---- Feature 002 (contracts/kb-api.md §Additional conventions) -------------

/// Error codes added by the knowledge-base API. Kept apart from `ALL_CODES`,
/// which a test pins to feature 001's documented list.
pub const KB_CODES: &[&str] = &[
    "draft_conflict",
    "identity_conflict",
    "not_previewable",
    "not_publishable",
    "not_executable",
    "verification_invalid",
    "profile_unsatisfied",
    "patient_data_blocked",
    "unsupported_schema",
    "published_immutable",
    "bundle_invalid",
    "authorization_required",
    "clinical_claim_not_executable",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbErrorCode {
    DraftConflict,
    IdentityConflict,
    NotPreviewable,
    NotPublishable,
    NotExecutable,
    VerificationInvalid,
    ProfileUnsatisfied,
    PatientDataBlocked,
    UnsupportedSchema,
    PublishedImmutable,
    BundleInvalid,
    AuthorizationRequired,
    ClinicalClaimNotExecutable,
}

impl KbErrorCode {
    pub fn code(&self) -> &'static str {
        match self {
            KbErrorCode::DraftConflict => "draft_conflict",
            KbErrorCode::IdentityConflict => "identity_conflict",
            KbErrorCode::NotPreviewable => "not_previewable",
            KbErrorCode::NotPublishable => "not_publishable",
            KbErrorCode::NotExecutable => "not_executable",
            KbErrorCode::VerificationInvalid => "verification_invalid",
            KbErrorCode::ProfileUnsatisfied => "profile_unsatisfied",
            KbErrorCode::PatientDataBlocked => "patient_data_blocked",
            KbErrorCode::UnsupportedSchema => "unsupported_schema",
            KbErrorCode::PublishedImmutable => "published_immutable",
            KbErrorCode::BundleInvalid => "bundle_invalid",
            KbErrorCode::AuthorizationRequired => "authorization_required",
            KbErrorCode::ClinicalClaimNotExecutable => "clinical_claim_not_executable",
        }
    }

    pub fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            KbErrorCode::DraftConflict
            | KbErrorCode::IdentityConflict
            | KbErrorCode::NotPreviewable
            | KbErrorCode::NotExecutable
            | KbErrorCode::VerificationInvalid
            | KbErrorCode::PublishedImmutable => StatusCode::CONFLICT,
            KbErrorCode::NotPublishable
            | KbErrorCode::ProfileUnsatisfied
            | KbErrorCode::PatientDataBlocked
            | KbErrorCode::UnsupportedSchema
            | KbErrorCode::AuthorizationRequired
            | KbErrorCode::ClinicalClaimNotExecutable
            | KbErrorCode::BundleInvalid => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }
}

/// A knowledge-base API error: the contract envelope plus optional
/// `details` (e.g. `findings[]` for `not_publishable`/`bundle_invalid`, or
/// the on-disk revision for `draft_conflict`). Messages are generic and
/// never carry file contents or patient details (FR-053).
#[derive(Debug, Clone)]
pub struct KbError {
    pub code: KbErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl KbError {
    pub fn new(code: KbErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn body(&self) -> serde_json::Value {
        let mut error = serde_json::json!({ "code": self.code.code(), "message": self.message });
        if let Some(d) = &self.details {
            error["details"] = d.clone();
        }
        serde_json::json!({ "error": error })
    }
}

impl axum::response::IntoResponse for KbError {
    fn into_response(self) -> axum::response::Response {
        (self.code.http_status(), axum::Json(self.body())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn every_variant() -> Vec<ServiceError> {
        let all = vec![
            ServiceError::NotFound,
            ServiceError::StaleReference,
            ServiceError::SourceMissing,
            ServiceError::SourceChanged,
            ServiceError::InvalidMask,
            ServiceError::CredentialRequired,
            ServiceError::CredentialInvalid,
            ServiceError::Conflict,
            ServiceError::ServiceUnavailable,
            ServiceError::AccessDenied,
            ServiceError::BundleTampered,
            ServiceError::BundleIncompatible,
        ];
        // Adding a variant without listing it above breaks compilation here.
        for e in &all {
            match e {
                ServiceError::NotFound
                | ServiceError::StaleReference
                | ServiceError::SourceMissing
                | ServiceError::SourceChanged
                | ServiceError::InvalidMask
                | ServiceError::CredentialRequired
                | ServiceError::CredentialInvalid
                | ServiceError::Conflict
                | ServiceError::ServiceUnavailable
                | ServiceError::AccessDenied
                | ServiceError::BundleTampered
                | ServiceError::BundleIncompatible => {}
            }
        }
        all
    }

    #[test]
    fn code_matches_serde_tag_and_all_codes() {
        let variants = every_variant();
        assert_eq!(variants.len(), ALL_CODES.len());
        for e in variants {
            let tag = serde_json::to_value(&e).unwrap()["code"].as_str().unwrap().to_string();
            assert_eq!(e.code(), tag);
            assert!(ALL_CODES.contains(&e.code()));
        }
    }

    #[test]
    fn body_has_contract_shape() {
        for e in every_variant() {
            let body = e.body();
            assert_eq!(body["error"]["code"], e.code());
            assert!(!body["error"]["message"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn http_statuses_match_the_contract() {
        let expected = [
            ("not_found", StatusCode::NOT_FOUND),
            ("stale_reference", StatusCode::GONE),
            ("source_missing", StatusCode::CONFLICT),
            ("source_changed", StatusCode::CONFLICT),
            ("invalid_mask", StatusCode::UNPROCESSABLE_ENTITY),
            ("credential_required", StatusCode::UNAUTHORIZED),
            ("credential_invalid", StatusCode::UNAUTHORIZED),
            ("conflict", StatusCode::CONFLICT),
            ("service_unavailable", StatusCode::SERVICE_UNAVAILABLE),
            ("access_denied", StatusCode::UNAUTHORIZED),
            ("bundle_tampered", StatusCode::UNPROCESSABLE_ENTITY),
            ("bundle_incompatible", StatusCode::UNPROCESSABLE_ENTITY),
        ];
        for e in every_variant() {
            let (_, status) = expected.iter().find(|(c, _)| *c == e.code()).unwrap();
            assert_eq!(e.http_status(), *status, "{}", e.code());
        }
    }

    #[test]
    fn kb_codes_have_the_documented_statuses() {
        use KbErrorCode::*;
        let expected = [
            (DraftConflict, StatusCode::CONFLICT),
            (IdentityConflict, StatusCode::CONFLICT),
            (NotPreviewable, StatusCode::CONFLICT),
            (NotPublishable, StatusCode::UNPROCESSABLE_ENTITY),
            (NotExecutable, StatusCode::CONFLICT),
            (VerificationInvalid, StatusCode::CONFLICT),
            (ProfileUnsatisfied, StatusCode::UNPROCESSABLE_ENTITY),
            (PatientDataBlocked, StatusCode::UNPROCESSABLE_ENTITY),
            (UnsupportedSchema, StatusCode::UNPROCESSABLE_ENTITY),
            (PublishedImmutable, StatusCode::CONFLICT),
            (BundleInvalid, StatusCode::UNPROCESSABLE_ENTITY),
            (AuthorizationRequired, StatusCode::UNPROCESSABLE_ENTITY),
            (ClinicalClaimNotExecutable, StatusCode::UNPROCESSABLE_ENTITY),
        ];
        assert_eq!(expected.len(), KB_CODES.len());
        for (code, status) in expected {
            assert!(KB_CODES.contains(&code.code()), "{}", code.code());
            assert_eq!(code.http_status(), status, "{}", code.code());
        }
    }

    #[test]
    fn kb_error_body_carries_details_only_when_given() {
        let plain = KbError::new(KbErrorCode::PublishedImmutable, "published");
        assert!(plain.body()["error"].get("details").is_none());
        let rich = KbError::new(KbErrorCode::NotPublishable, "no")
            .with_details(serde_json::json!({"findings": []}));
        assert_eq!(rich.body()["error"]["details"]["findings"], serde_json::json!([]));
        assert_eq!(rich.body()["error"]["code"], "not_publishable");
    }

    /// Every KB code is documented in contracts/kb-api.md.
    #[test]
    fn kb_contract_lists_every_kb_code() {
        let contract = include_str!("../../../specs/002-paper-pipeline-designer/contracts/kb-api.md");
        for code in KB_CODES {
            assert!(contract.contains(&format!("`{code}`")), "{code} undocumented");
        }
    }

    /// The documented "Common error codes" list and `ALL_CODES` must agree
    /// in both directions.
    #[test]
    fn contract_document_lists_exactly_the_implemented_codes() {
        let contract = include_str!(
            "../../../specs/001-data-layer-design/contracts/local-service-api.md"
        );
        let start = contract.find("Common error codes").expect("contract lists error codes");
        let paragraph = contract[start..].split("\n\n").next().unwrap();
        let documented: std::collections::BTreeSet<&str> = paragraph
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        let implemented: std::collections::BTreeSet<&str> = ALL_CODES.iter().copied().collect();
        assert_eq!(documented, implemented);
    }
}

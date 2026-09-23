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

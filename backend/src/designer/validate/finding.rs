//! `Finding` — the single diagnostic shape used by every validator and API
//! (contracts/kb-api.md). `explanation` and `action` are mandatory (FR-005,
//! FR-042). A Finding names a file, rule or port — never file contents or
//! patient details (FR-053).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectType {
    Bundle,
    NodeInstance,
    Port,
    Edge,
    Parameter,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subject {
    #[serde(rename = "type")]
    pub kind: SubjectType,
    #[serde(rename = "ref")]
    pub reference: String,
}

impl Subject {
    pub fn new(kind: SubjectType, reference: impl Into<String>) -> Self {
        Self {
            kind,
            reference: reference.into(),
        }
    }
    pub fn bundle(reference: impl Into<String>) -> Self {
        Self::new(SubjectType::Bundle, reference)
    }
    pub fn file(reference: impl Into<String>) -> Self {
        Self::new(SubjectType::File, reference)
    }
}

/// Which bundle a finding is about; `version` is absent for drafts whose
/// version is not yet fixed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl BundleRef {
    pub fn new(id: impl Into<String>, version: Option<String>) -> Self {
        Self {
            id: id.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub bundle: BundleRef,
    pub subject: Subject,
    pub explanation: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindingError {
    #[error("a finding must carry a non-empty explanation")]
    EmptyExplanation,
    #[error("a finding must carry a non-empty action")]
    EmptyAction,
}

impl Finding {
    /// Rejects a blank `explanation` or `action` (FR-005): a diagnostic the
    /// researcher cannot act on is not a valid finding.
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        bundle: BundleRef,
        subject: Subject,
        explanation: impl Into<String>,
        action: impl Into<String>,
    ) -> Result<Self, FindingError> {
        let explanation = explanation.into();
        let action = action.into();
        if explanation.trim().is_empty() {
            return Err(FindingError::EmptyExplanation);
        }
        if action.trim().is_empty() {
            return Err(FindingError::EmptyAction);
        }
        Ok(Self {
            severity,
            code: code.into(),
            bundle,
            subject,
            explanation,
            action,
        })
    }

    /// For call sites whose explanation/action text is a compiled-in
    /// constant plus interpolated values, so emptiness is a programmer bug.
    pub fn build(
        severity: Severity,
        code: &str,
        bundle: &BundleRef,
        subject: Subject,
        explanation: String,
        action: &str,
    ) -> Self {
        Self::new(severity, code, bundle.clone(), subject, explanation, action)
            .expect("finding text is always non-empty at call sites")
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b() -> BundleRef {
        BundleRef::new("rosaray.demo", Some("1.0.0".into()))
    }

    #[test]
    fn rejects_empty_explanation_or_action() {
        assert_eq!(
            Finding::new(Severity::Error, "x", b(), Subject::bundle("a"), " ", "fix it").unwrap_err(),
            FindingError::EmptyExplanation
        );
        assert_eq!(
            Finding::new(Severity::Error, "x", b(), Subject::bundle("a"), "bad", "").unwrap_err(),
            FindingError::EmptyAction
        );
    }

    #[test]
    fn serializes_to_contract_shape() {
        let f = Finding::new(Severity::Warning, "c", b(), Subject::file("contract.yaml"), "e", "a").unwrap();
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["severity"], "warning");
        assert_eq!(v["subject"]["type"], "file");
        assert_eq!(v["subject"]["ref"], "contract.yaml");
        assert_eq!(v["bundle"]["version"], "1.0.0");
    }
}

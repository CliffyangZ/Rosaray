//! Evidence records (`references/*.yaml`) grouped by type. The three types —
//! method source, technical verification, dataset validation — are never
//! merged into one notion of "verified" (FR-011).

use serde::Serialize;

use crate::contract::finding::{Finding, Severity, Subject};
use crate::bundle::model::{EvidenceRecord, EvidenceType};
use crate::bundle::read::Bundle;

#[derive(Debug, Clone, Default, Serialize)]
pub struct EvidenceGroups {
    pub method_source: Vec<EvidenceRecord>,
    pub technical_verification: Vec<EvidenceRecord>,
    pub dataset_validation: Vec<EvidenceRecord>,
}

pub fn group(records: &[EvidenceRecord]) -> EvidenceGroups {
    let mut g = EvidenceGroups::default();
    for r in records {
        match r.evidence_type {
            EvidenceType::MethodSource => g.method_source.push(r.clone()),
            EvidenceType::TechnicalVerification => g.technical_verification.push(r.clone()),
            EvidenceType::DatasetValidation => g.dataset_validation.push(r.clone()),
        }
    }
    g
}

/// Structural findings for a bundle's evidence: dataset validation must state
/// its scope, and a record must apply to the bundle that holds it.
pub fn evidence_findings(bundle: &Bundle) -> Vec<Finding> {
    let r = bundle.bundle_ref();
    let mut out = Vec::new();
    for rec in &bundle.evidence {
        if rec.evidence_type == EvidenceType::DatasetValidation && rec.scope.is_none() {
            out.push(Finding::build(
                Severity::Error,
                "evidence_scope_missing",
                &r,
                Subject::file(format!("references/{}", rec.evidence_id)),
                format!("The dataset-validation record {} does not say what dataset, subjects, reference standard and metric it covers.", rec.evidence_id),
                "Add a scope so the result is not read as more general than it is.",
            ));
        }
        if rec.applies_to.id != bundle.header.id || rec.applies_to.version != bundle.header.version {
            out.push(Finding::build(
                Severity::Warning,
                "evidence_applies_elsewhere",
                &r,
                Subject::file(format!("references/{}", rec.evidence_id)),
                format!("The evidence record {} applies to {}@{}, not to this bundle.", rec.evidence_id, rec.applies_to.id, rec.applies_to.version),
                "Correct applies_to, or move the record to the bundle it describes.",
            ));
        }
    }
    out
}

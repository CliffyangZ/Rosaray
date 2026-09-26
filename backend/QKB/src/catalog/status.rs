//! Derived status of a bundle: maturity, availability, technical verification,
//! dataset validation and deprecation — five separate dimensions, never
//! collapsed into one "verified" flag (FR-011) and never hand-written in a
//! bundle (data-model.md).

use std::path::Path;

use crate::designer::validate::finding::{Finding, Severity, Subject as FSubject};
use crate::kb::bundle::model::{Kind, Trust};
use crate::kb::bundle::read::Bundle;
use crate::kb::evidence::deprecation::{self, DeprecationState};
use crate::kb::evidence::verification::{self, Status, Subject, VerificationType};
use crate::kb::trust::effective_trust;

pub const SPECIFICATION_ONLY: &str = "specification_only";
pub const IMPLEMENTED: &str = "implemented";
pub const TECHNICALLY_VERIFIED: &str = "technically_verified";

/// Implementation ids the built-in Runtime Adapter registry resolves. The
/// registry (designer::runtime_adapter) asserts it matches this list, so a
/// node naming anything else is `implementation_unavailable` (FR-045).
pub const BUILTIN_IMPLEMENTATIONS: &[&str] = &[
    "builtin.image-source",
    "builtin.normalize",
    "builtin.gaussian-blur",
    "builtin.threshold",
    "builtin.morphology",
    "builtin.area",
];

pub fn is_resolvable_implementation(id: &str) -> bool {
    BUILTIN_IMPLEMENTATIONS.contains(&id)
}

/// `specification_only` (no usable binding) → `implemented` (a binding whose
/// *effective* trust is not `untrusted` and whose id the built-in registry
/// resolves). Technical verification is layered on top by `derive`. Only nodes
/// have a maturity.
pub fn maturity_of(bundle: &Bundle, effective_trust: Trust) -> Option<&'static str> {
    if bundle.kind != Kind::Algonode {
        return None;
    }
    match &bundle.implementation {
        Some(imp) if effective_trust != Trust::Untrusted && is_resolvable_implementation(&imp.implementation_id) => Some(IMPLEMENTED),
        _ => Some(SPECIFICATION_ONLY),
    }
}

/// The exact identity a verification of this published node applies to.
pub fn verification_subject(bundle: &Bundle) -> Option<Subject> {
    let lock = bundle.lock.as_ref()?;
    let imp = bundle.implementation.as_ref()?;
    let mut assets: Vec<String> = imp
        .assets
        .iter()
        .map(|a| format!("b3:{}", a.blake3.trim_start_matches("b3:")))
        .collect();
    assets.sort();
    Some(Subject {
        node_id: bundle.header.id.clone(),
        definition_content_id: lock.content_id.clone(),
        implementation_id: imp.implementation_id.clone(),
        implementation_version: imp.implementation_version.clone(),
        asset_content_ids: assets,
    })
}

#[derive(Debug, Clone)]
pub struct DerivedStatus {
    pub maturity: Option<&'static str>,
    /// `available | deprecated | unavailable`
    pub availability: &'static str,
    /// `none | passed | failed | withdrawn` (technical verification)
    pub verification: Status,
    /// `none | passed | failed | stale`
    pub dataset_validation: &'static str,
    pub deprecation: DeprecationState,
    pub trust: Trust,
    pub implementation_resolvable: bool,
    pub findings: Vec<Finding>,
}

/// Derives every status dimension for `bundle`. `modified` says the reader
/// already found the published bundle drifting from its `bundle.lock`.
pub fn derive(root: &Path, bundle: &Bundle, modified: bool) -> DerivedStatus {
    let trust = effective_trust(root, bundle);
    let mut findings = Vec::new();
    let r = bundle.bundle_ref();
    let mut unavailable = modified;

    let implementation_resolvable = bundle.implementation.as_ref().is_some_and(|i| is_resolvable_implementation(&i.implementation_id));
    if let Some(imp) = &bundle.implementation {
        if !implementation_resolvable {
            findings.push(Finding::build(
                Severity::Warning,
                "implementation_unavailable",
                &r,
                FSubject::file("implementation.yaml"),
                format!("The implementation \"{}\" is not one this installation can run, so the node stays a specification.", imp.implementation_id),
                "Use a built-in implementation id, or treat the node as knowledge only.",
            ));
        }
    }

    // Deprecation lives in the version's amendment chain (outside the bundle).
    let mut dep = DeprecationState::default();
    if bundle.is_published() {
        match deprecation::read(root, &bundle.header.id, &bundle.header.version) {
            Ok(chain) => {
                if let Some(brk) = &chain.broken {
                    unavailable = true;
                    findings.push(brk.to_finding(&r));
                }
                dep = deprecation::state_of(&chain);
            }
            Err(_) => unavailable = true,
        }
    }

    // Technical / dataset verification, for this exact subject tuple.
    let mut verification = Status::None;
    let mut dataset = "none";
    if bundle.kind == Kind::Algonode && bundle.is_published() {
        if let Some(subject) = verification_subject(bundle) {
            match verification::status_for(root, VerificationType::Technical, &subject) {
                Ok((status, chain)) => {
                    if let Some(brk) = &chain.broken {
                        unavailable = true;
                        findings.push(brk.to_finding(&r));
                    }
                    verification = status;
                    dataset = match verification::current_status(&chain, VerificationType::Dataset, &subject) {
                        Status::Passed => "passed",
                        Status::Failed => "failed",
                        Status::Withdrawn => "stale",
                        Status::None => "none",
                    };
                }
                Err(_) => unavailable = true,
            }
        }
    }

    let mut maturity = maturity_of(bundle, trust);
    if maturity == Some(IMPLEMENTED) && verification.is_valid() {
        maturity = Some(TECHNICALLY_VERIFIED);
    }
    let availability = if unavailable {
        "unavailable"
    } else if dep.deprecated {
        "deprecated"
    } else {
        "available"
    };
    DerivedStatus { maturity, availability, verification, dataset_validation: dataset, deprecation: dep, trust, implementation_resolvable, findings }
}

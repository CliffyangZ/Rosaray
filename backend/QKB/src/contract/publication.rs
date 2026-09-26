//! `check_publication(kind)`: may this AlgoPipe draft become an immutable
//! version, and as which kind of release? Returns *every* unresolved condition
//! at once (US6-2) so nothing is written until all are addressed.
//!
//! | check                                                | Knowledge | Executable |
//! |------------------------------------------------------|-----------|------------|
//! | bundle valid, graph valid                            |     ✔     |     ✔      |
//! | every node ref → immutable published Definition      |     ✔     |     ✔      |
//! | mandatory review items resolved                      |     ✔     |     ✔      |
//! | implementation pinned, resolvable and trusted        |     –     |     ✔      |
//! | every node technically verified                      |     –     |     ✔      |
//! | node prerequisites ⊆ the pipe's target data profile  |     –     |     ✔      |
//! | dataset validation missing                           |  disclose |  disclose  |

use std::path::Path;

use rusqlite::Connection;

use super::bundle::{validate_bundle, BundleCheck};
use super::finding::{Finding, Severity, Subject, SubjectType};
use super::graph::{validate_graph, GraphInput};
use crate::bundle::model::{Pin, Trust};
use crate::bundle::read::{read_bundle, Bundle};
use crate::catalog::query::{path_of, CatalogDefinitions, CatalogResolver};
use crate::catalog::status::{derive, verification_subject};
use crate::evidence::verification::Status;
use crate::profile::is_known_predicate;

/// Whether a pipe version is published as knowledge only or as executable.
/// QKB only retains executable pipes; the knowledge release is rejected on submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    Knowledge,
    Executable,
}

impl Release {
    pub fn as_str(&self) -> &'static str {
        match self {
            Release::Knowledge => "knowledge",
            Release::Executable => "executable",
        }
    }
}

pub const DISCLOSURE_DATASET_VALIDATION_MISSING: &str = "dataset_validation_missing";

/// What publication learned about one node reference.
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub instance_id: String,
    pub id: String,
    pub version: String,
    /// The referenced version's `content_id`, to be frozen into the published graph.
    pub content_id: String,
    pub verified: bool,
    pub dataset_validated: bool,
}

#[derive(Debug, Default)]
pub struct PublicationCheck {
    pub findings: Vec<Finding>,
    pub disclosures: Vec<String>,
    pub nodes: Vec<ResolvedNode>,
    /// Executable releases only.
    pub pins: Vec<Pin>,
}

impl PublicationCheck {
    pub fn blocked(&self) -> bool {
        self.findings.iter().any(|f| f.is_error())
    }
}

fn err(bundle: &Bundle, code: &str, subject: Subject, explanation: String, action: &str) -> Finding {
    Finding::build(Severity::Error, code, &bundle.bundle_ref(), subject, explanation, action)
}

pub fn check_publication(conn: &Connection, root: &Path, bundle: &Bundle, release: Release) -> PublicationCheck {
    let mut check = PublicationCheck::default();
    let Some(graph) = &bundle.graph else {
        check.findings.push(err(bundle, "bundle_file_missing", Subject::file("graph.yaml"), "The pipe has no graph.yaml.".into(), "Add the pipe's graph."));
        return check;
    };

    // Shared: bundle and graph validity, mandatory review items.
    check.findings.extend(validate_bundle(bundle, &BundleCheck::with_resolver(&CatalogResolver { conn })));
    let defs = CatalogDefinitions { conn, root };
    let report = validate_graph(
        &GraphInput {
            bundle: bundle.bundle_ref(),
            graph,
            previous: None,
            profile: graph.target_data_profile.as_ref(),
            domain: bundle.header.domain.as_deref(),
        },
        &defs,
    );
    // Findings validate_bundle already produced (e.g. dependency ones) are not repeated.
    for f in report.findings {
        if !check.findings.iter().any(|g| g.code == f.code && g.subject == f.subject) {
            check.findings.push(f);
        }
    }
    for (field, value) in [("intended_use", &bundle.header.intended_use), ("limitations", &bundle.header.limitations)] {
        if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
            check.findings.push(err(
                bundle,
                "publication_field_missing",
                Subject::bundle(bundle.header.id.clone()),
                format!("`{field}` is empty; a published version must state it."),
                &format!("Fill in `{field}` in the frontmatter before publishing."),
            ));
        }
    }
    if graph.nodes.is_empty() {
        check.findings.push(err(bundle, "graph_empty", Subject::file("graph.yaml"), "The pipe has no nodes.".into(), "Add at least one node before publishing."));
    }

    // Every reference must be an immutable, present, intact published version.
    for n in &graph.nodes {
        let r = &n.node_ref;
        let subject = Subject::new(SubjectType::NodeInstance, n.instance_id.clone());
        if semver::Version::parse(&r.version).is_err() {
            check.findings.push(err(
                bundle,
                "dependency_not_immutable",
                subject,
                format!("{} refers to \"{}\" of {}, which is not an exact published version.", n.instance_id, r.version, r.id),
                "Publish that node first and reference its exact version; `draft`, `latest` and ranges are for drafts only.",
            ));
            continue;
        }
        let Some(path) = path_of(conn, "algonode", &r.id, &r.version, "published").ok().flatten() else {
            check.findings.push(err(
                bundle,
                "dependency_unresolved",
                subject,
                format!("{}@{} is not a published node in this knowledge base.", r.id, r.version),
                "Import or publish that version, or reference one that exists.",
            ));
            continue;
        };
        let (dep, read_findings) = read_bundle(&root.join(&path));
        let Some(dep) = dep else { continue };
        let modified = read_findings.iter().any(|f| f.code == "published_bundle_modified");
        let derived = derive(root, &dep, modified);
        let Some(lock) = &dep.lock else { continue };
        if derived.availability == "unavailable" {
            check.findings.push(err(
                bundle,
                "dependency_unavailable",
                subject.clone(),
                format!("{}@{} is currently unavailable (its files or history no longer verify).", r.id, r.version),
                "Restore the original files or re-import the version.",
            ));
        }
        if r.content_id.as_ref().is_some_and(|c| *c != lock.content_id) {
            check.findings.push(err(
                bundle,
                "dependency_content_mismatch",
                subject.clone(),
                format!("{}@{} exists but its content differs from the one this pipe was built against.", r.id, r.version),
                "Restore the original version, or update the reference deliberately.",
            ));
        }
        let verified = derived.verification == Status::Passed;
        let validated = derived.dataset_validation == "passed";
        check.nodes.push(super::publication::ResolvedNode {
            instance_id: n.instance_id.clone(),
            id: r.id.clone(),
            version: r.version.clone(),
            content_id: lock.content_id.clone(),
            verified,
            dataset_validated: validated,
        });

        if release == Release::Executable {
            match &dep.implementation {
                None => check.findings.push(err(
                    bundle,
                    "implementation_unavailable",
                    subject.clone(),
                    format!("{}@{} has no implementation, so a pipe using it cannot be run.", r.id, r.version),
                    "Publish a knowledge release instead, or use a node that has a built-in implementation.",
                )),
                Some(imp) => {
                    if derived.trust == Trust::Untrusted {
                        check.findings.push(err(
                            bundle,
                            "implementation_untrusted",
                            subject.clone(),
                            format!("The implementation of {}@{} is not trusted on this machine.", r.id, r.version),
                            "Only trusted built-in implementations can be part of an executable release.",
                        ));
                    } else if !derived.implementation_resolvable {
                        check.findings.push(err(
                            bundle,
                            "implementation_unavailable",
                            subject.clone(),
                            format!("The implementation \"{}\" of {}@{} cannot be run by this installation.", imp.implementation_id, r.id, r.version),
                            "Use a node whose implementation is built in.",
                        ));
                    }
                    if let Some(s) = verification_subject(&dep) {
                        check.pins.push(Pin {
                            instance_id: n.instance_id.clone(),
                            implementation_id: s.implementation_id,
                            version: s.implementation_version,
                            asset_content_ids: s.asset_content_ids,
                        });
                    }
                }
            }
            if !verified {
                check.findings.push(err(
                    bundle,
                    "node_not_verified",
                    subject,
                    format!("{}@{} is not technically verified (status: {}), so an executable release is not allowed.", r.id, r.version, derived.verification.as_str()),
                    "Run the node's verification, or publish a knowledge-only release.",
                ));
            }
            // Node contracts ⊆ the pipe's target data profile.
            if let Some(contract) = &dep.contract {
                for pre in &contract.prerequisites {
                    let declared = graph.target_data_profile.as_ref().is_some_and(|p| {
                        p.require.iter().any(|req| {
                            req.predicate == pre.predicate
                                && match (pre.args.get("equals"), req.args.get("equals")) {
                                    (Some(want), Some(have)) => want == have,
                                    (Some(_), None) => false,
                                    _ => true,
                                }
                        })
                    });
                    if !declared {
                        check.findings.push(err(
                            bundle,
                            "prerequisite_unmet",
                            Subject::new(SubjectType::NodeInstance, n.instance_id.clone()),
                            format!("{} requires {} but the pipe's target data profile does not guarantee it.", n.instance_id, pre.predicate),
                            &format!("Add a `{}` requirement to the target data profile.", pre.predicate),
                        ));
                    }
                }
            }
        }
    }

    if release == Release::Executable {
        if let Some(profile) = &graph.target_data_profile {
            for p in &profile.require {
                if !is_known_predicate(&p.predicate) {
                    check.findings.push(err(
                        bundle,
                        "unsupported_profile_predicate",
                        Subject::bundle(p.predicate.clone()),
                        format!("The target data profile uses \"{}\", which cannot be checked.", p.predicate),
                        "Use a supported predicate.",
                    ));
                }
            }
        }
    }

    // Missing dataset-level validation is *disclosed*, never a gate (FR-029).
    if check.nodes.iter().any(|n| !n.dataset_validated) {
        check.disclosures.push(DISCLOSURE_DATASET_VALIDATION_MISSING.to_string());
    }
    check
}

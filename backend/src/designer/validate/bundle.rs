//! `validate_bundle`: bundle completeness and contract shape (FR-042). One
//! call site for edit-time (advisory), import and publish (blocking).

use super::contract::validate_contract;
use super::finding::{BundleRef, Finding, Severity, Subject, SubjectType};
use crate::kb::bundle::model::Kind;
use crate::kb::bundle::read::Bundle;
use crate::kb::bundle::write::check_relative;
use crate::kb::identity::{is_valid_id, is_valid_version, validate_identity};

/// What a resolver knows about a referenced `id@version`.
#[derive(Debug, Clone)]
pub struct ResolvedRef {
    pub content_id: Option<String>,
}

/// Resolves a pipe's node references against the catalog.
pub trait DependencyResolver {
    fn resolve(&self, id: &str, version: &str) -> Option<ResolvedRef>;
}

/// For callers with no catalog (pure-file checks); every reference is
/// reported unresolved only if `check_dependencies` is on.
pub struct NoResolver;

impl DependencyResolver for NoResolver {
    fn resolve(&self, _id: &str, _version: &str) -> Option<ResolvedRef> {
        None
    }
}

pub struct BundleCheck<'a> {
    pub resolver: Option<&'a dyn DependencyResolver>,
}

impl<'a> BundleCheck<'a> {
    pub fn files_only() -> Self {
        Self { resolver: None }
    }
    pub fn with_resolver(resolver: &'a dyn DependencyResolver) -> Self {
        Self { resolver: Some(resolver) }
    }
}

pub fn validate_bundle(bundle: &Bundle, check: &BundleCheck) -> Vec<Finding> {
    let mut out = Vec::new();
    let r = bundle.bundle_ref();
    let h = &bundle.header;
    let published = bundle.is_published();

    if h.id.is_empty() || h.version.is_empty() {
        out.push(Finding::build(
            Severity::Error,
            "identity_missing",
            &r,
            Subject::bundle(h.id.clone()),
            "The frontmatter has no id or no version, so the bundle has no stable identity.".into(),
            "Set both `id` and `version` in the frontmatter header.",
        ));
    } else {
        out.extend(validate_identity(&h.id, &h.version));
    }
    for (field, value) in [("name", &h.name), ("summary", &h.summary)] {
        if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
            out.push(Finding::build(
                Severity::Warning,
                "field_missing",
                &r,
                Subject::bundle(h.id.clone()),
                format!("The frontmatter has no `{field}`."),
                &format!("Add a `{field}` so the bundle is easy to find."),
            ));
        }
    }
    if h.research_use_only == Some(false) {
        out.push(Finding::build(
            Severity::Error,
            "research_use_only_required",
            &r,
            Subject::bundle(h.id.clone()),
            "research_use_only is false; every knowledge bundle is research-use-only.".into(),
            "Set research_use_only: true (or remove the field).",
        ));
    }

    match bundle.kind {
        Kind::Algonode => {
            if let Some(c) = &bundle.contract {
                out.extend(validate_contract(&r, c));
            }
        }
        Kind::Algopipe => validate_graph_refs(bundle, &r, check, published, &mut out),
    }

    if let Some(imp) = &bundle.implementation {
        for asset in &imp.assets {
            match check_relative(&asset.path) {
                Err(_) => out.push(Finding::build(
                    Severity::Error,
                    "asset_path_unsafe",
                    &r,
                    Subject::file("implementation.yaml"),
                    "An implementation asset path is absolute or escapes the bundle folder.".into(),
                    "Use a relative path inside the bundle.",
                )),
                Ok(rel) => match std::fs::read(bundle.dir.join(rel)) {
                    Err(_) => out.push(Finding::build(
                        Severity::Error,
                        "asset_missing",
                        &r,
                        Subject::file(asset.path.clone()),
                        format!("The implementation asset {} is listed but not present in the bundle.", asset.path),
                        "Add the file, or remove it from implementation.yaml.",
                    )),
                    Ok(bytes) => {
                        let actual = blake3::hash(&bytes).to_hex().to_string();
                        if actual != asset.blake3.strip_prefix("b3:").unwrap_or(&asset.blake3) {
                            out.push(Finding::build(
                                Severity::Error,
                                "asset_hash_mismatch",
                                &r,
                                Subject::file(asset.path.clone()),
                                format!("The implementation asset {} does not match the hash recorded for it.", asset.path),
                                "Restore the original file or update the recorded hash deliberately.",
                            ));
                        }
                    }
                },
            }
        }
    }
    out
}

fn validate_graph_refs(
    bundle: &Bundle,
    r: &BundleRef,
    check: &BundleCheck,
    published: bool,
    out: &mut Vec<Finding>,
) {
    let Some(graph) = &bundle.graph else { return };
    let mut seen = std::collections::HashSet::new();
    for node in &graph.nodes {
        let subject = Subject::new(SubjectType::NodeInstance, node.instance_id.clone());
        if !seen.insert(node.instance_id.as_str()) {
            out.push(Finding::build(
                Severity::Error,
                "instance_duplicate",
                r,
                subject.clone(),
                format!("The node instance id \"{}\" is used more than once.", node.instance_id),
                "Give each node instance a unique instance_id.",
            ));
        }
        let nref = &node.node_ref;
        if !is_valid_id(&nref.id) {
            out.push(Finding::build(
                Severity::Error,
                "id_invalid",
                r,
                subject.clone(),
                format!("The node reference \"{}\" is not a valid stable ID.", nref.id),
                "Reference nodes by their stable ID.",
            ));
            continue;
        }
        if !is_valid_version(&nref.version) {
            if published {
                out.push(Finding::build(
                    Severity::Error,
                    "dependency_not_immutable",
                    r,
                    subject.clone(),
                    format!("The reference to {} uses \"{}\", which is not an exact version.", nref.id, nref.version),
                    "Pin an exact published SemVer version; `draft`, `latest` and ranges are for drafts only.",
                ));
            }
            continue;
        }
        if published && nref.content_id.is_none() {
            out.push(Finding::build(
                Severity::Error,
                "dependency_not_immutable",
                r,
                subject.clone(),
                format!("The reference to {}@{} carries no content_id.", nref.id, nref.version),
                "Record the referenced version's content_id.",
            ));
        }
        if let Some(resolver) = check.resolver {
            match resolver.resolve(&nref.id, &nref.version) {
                None => out.push(Finding::build(
                    Severity::Error,
                    "dependency_unresolved",
                    r,
                    subject,
                    format!("{}@{} is not in the knowledge base.", nref.id, nref.version),
                    "Import or create that version, or reference one that exists.",
                )),
                Some(found) => {
                    if let (Some(want), Some(have)) = (&nref.content_id, &found.content_id) {
                        if want != have {
                            out.push(Finding::build(
                                Severity::Error,
                                "dependency_content_mismatch",
                                r,
                                subject,
                                format!("{}@{} exists but its content differs from the one this pipe was built against.", nref.id, nref.version),
                                "Restore the original version, or update the reference deliberately.",
                            ));
                        }
                    }
                }
            }
        }
    }
}

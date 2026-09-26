//! Publication (FR-030, FR-043, FR-048): turns a draft into an immutable,
//! hash-locked version. Validation runs first and every blocking finding is
//! returned together; on any failure nothing is written. The draft is
//! retained and continues as the next version.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use crate::data_repository::sqlite::kb_event_repo;
use crate::designer::validate::bundle::{validate_bundle, BundleCheck};
use crate::designer::validate::finding::{Finding, Severity, Subject};
use crate::designer::validate::publication::check_publication;
use crate::kb::bundle::draft::{revision_of, save_draft, DraftConflict};
use crate::kb::bundle::frontmatter::set_field;
use crate::kb::bundle::model::{to_yaml, BundleLock, Kind, LockFile};
use crate::kb::bundle::read::{read_bundle, Bundle, LOCK_FILE};
use crate::kb::bundle::service::{bump_patch, published_index, read_files, sync_catalog, KbServiceError};
use crate::kb::bundle::write::{mark_read_only, write_bundle_atomic};
use crate::kb::catalog::query::CatalogResolver;
use crate::kb::catalog::repo::{draft_dir, record_draft_session};
use crate::kb::identity::{computational_identity, content_id_of};
use crate::kb::trust;

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

#[derive(Debug, Clone)]
pub struct PublishRequest {
    pub kind: Kind,
    pub id: String,
    pub base_revision: String,
    pub release: Release,
    pub version_description: Option<String>,
    /// Records that the researcher saw the disclosures; never a gate (FR-029).
    pub acknowledge_unvalidated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencySummary {
    pub total: usize,
    pub unresolved: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishOutcome {
    pub id: String,
    pub version: String,
    pub content_id: String,
    pub computational_identity: Option<String>,
    pub release_kind: String,
    pub dependency_summary: DependencySummary,
    pub verification_summary: serde_json::Value,
    pub disclosures: Vec<String>,
    /// Revision of the retained draft, which has moved on to the next version.
    pub next_draft_revision: String,
}

fn publication_findings(bundle: &Bundle, conn: &Connection) -> Vec<Finding> {
    let r = bundle.bundle_ref();
    let mut out = validate_bundle(bundle, &BundleCheck::with_resolver(&CatalogResolver { conn }));
    for (field, value) in [("intended_use", &bundle.header.intended_use), ("limitations", &bundle.header.limitations)] {
        if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
            out.push(Finding::build(
                Severity::Error,
                "publication_field_missing",
                &r,
                Subject::bundle(bundle.header.id.clone()),
                format!("`{field}` is empty; a published version must state it."),
                &format!("Fill in `{field}` in the frontmatter before publishing."),
            ));
        }
    }
    out
}

pub fn publish(conn: &Connection, root: &Path, req: PublishRequest) -> Result<PublishOutcome, KbServiceError> {
    let dir = draft_dir(root, req.kind.as_str(), &req.id);
    if !dir.is_dir() {
        return Err(KbServiceError::NotFound);
    }
    let revision = revision_of(&dir)?;
    if revision != req.base_revision {
        return Err(KbServiceError::DraftConflict(DraftConflict { on_disk_revision: revision, changed_files: Vec::new() }));
    }

    let (bundle, mut findings) = read_bundle(&dir);
    let Some(bundle) = bundle else {
        return Err(KbServiceError::NotPublishable(findings));
    };

    // Nodes only ever have knowledge releases; whether a *pipe* is runnable is decided
    // by which release it is published as.
    let mut check = None;
    if req.kind == Kind::Algonode {
        findings.extend(publication_findings(&bundle, conn));
        if req.release != Release::Knowledge {
            findings.push(Finding::build(
                Severity::Error,
                "release_not_supported",
                &bundle.bundle_ref(),
                Subject::bundle(req.id.clone()),
                "An AlgoNode has a knowledge release only; runnability belongs to a published AlgoPipe.".to_string(),
                "Publish the node as a knowledge release, then reference it from a pipe.",
            ));
        }
    } else {
        let c = check_publication(conn, root, &bundle, req.release);
        findings.extend(c.findings.iter().cloned());
        check = Some(c);
    }
    if findings.iter().any(|f| f.is_error()) {
        // Everything is reported together and nothing has been written.
        return Err(KbServiceError::NotPublishable(findings));
    }

    let version = bundle.header.version.clone();
    let index = published_index(root);
    if index.iter().any(|p| p.id == req.id && p.version == version) {
        return Err(KbServiceError::PublishedImmutable);
    }
    if index.iter().any(|p| p.id == req.id && p.kind != req.kind) {
        return Err(KbServiceError::IdentityConflict(format!("the id {} is used by a bundle of a different kind", req.id)));
    }

    // Freeze: the draft's files, with the header marked published.
    let main = if req.kind == Kind::Algopipe { "ALGOPIPE.md" } else { "ALGONODE.md" };

    // A published pipe pins each reference to the exact version *and* content it was
    // built against, plus (executable releases) the implementations that ran.
    let mut frozen_graph = None;
    if let (Some(graph), Some(c)) = (&bundle.graph, &check) {
        let mut g = graph.clone();
        for node in &mut g.nodes {
            if let Some(resolved) = c.nodes.iter().find(|r| r.instance_id == node.instance_id) {
                node.node_ref.content_id = Some(resolved.content_id.clone());
            }
        }
        g.implementation_pins = if req.release == Release::Executable { c.pins.clone() } else { Vec::new() };
        frozen_graph = Some(g);
    }

    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (rel, bytes) in read_files(&dir)? {
        if rel == LOCK_FILE {
            continue;
        }
        if rel == "graph.yaml" {
            if let Some(g) = &frozen_graph {
                files.push((rel, to_yaml(g).map_err(KbServiceError::UnsafePath)?.into_bytes()));
                continue;
            }
        }
        if rel == main {
            let text = String::from_utf8(bytes).map_err(|_| KbServiceError::UnsafePath(rel.clone()))?;
            let text = set_field(&text, "status", &json!("published"))
                .and_then(|t| set_field(&t, "research_use_only", &json!(true)))
                .and_then(|t| if req.kind == Kind::Algopipe { set_field(&t, "release_kind", &json!(req.release.as_str())) } else { Ok(t) })
                .map_err(|e| KbServiceError::Invalid(vec![Finding::build(
                    Severity::Error,
                    e.code,
                    &bundle.bundle_ref(),
                    Subject::file(main),
                    e.message,
                    "Correct the frontmatter header.",
                )]))?;
            files.push((rel, text.into_bytes()));
        } else {
            files.push((rel, bytes));
        }
    }

    let hashed: Vec<(String, String)> =
        files.iter().map(|(p, b)| (p.clone(), format!("b3:{}", blake3::hash(b).to_hex()))).collect();
    let content_id = content_id_of(hashed.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let lock = BundleLock {
        schema: "quantify-kb/1".to_string(),
        id: req.id.clone(),
        version: version.clone(),
        content_id: content_id.clone(),
        computational_identity: frozen_graph.as_ref().map(computational_identity),
        release_kind: Some(req.release.as_str().to_string()),
        disclosures: check.as_ref().map(|c| c.disclosures.clone()).unwrap_or_default(),
        dependency_summary: check.as_ref().map(|c| json!({ "total": c.nodes.len(), "unresolved": 0 })),
        verification_summary: check.as_ref().map(|c| json!({
            "verified": c.nodes.iter().filter(|n| n.verified).count(),
            "total": c.nodes.len(),
            "unverified": c.nodes.iter().filter(|n| !n.verified).map(|n| n.instance_id.clone()).collect::<Vec<_>>(),
        })),
        files: hashed
            .iter()
            .map(|(p, h)| LockFile { path: p.clone(), blake3: h.trim_start_matches("b3:").to_string() })
            .collect(),
    };
    files.push((LOCK_FILE.to_string(), to_yaml(&lock).map_err(KbServiceError::UnsafePath)?.into_bytes()));

    let sub = if req.kind == Kind::Algopipe { "pipes" } else { "nodes" };
    let dest = root.join(sub).join(&req.id).join(&version);
    if dest.exists() {
        return Err(KbServiceError::PublishedImmutable);
    }
    write_bundle_atomic(&dest, &files)?;
    mark_read_only(&dest);

    if let Some(imp) = &bundle.implementation {
        trust::record(root, &req.id, &version, &content_id, imp.trust, "publish")?;
    }

    // The draft continues as the next version (FR-030).
    let next_version = bump_patch(&version);
    let mut updates = Vec::new();
    let draft_main = std::fs::read_to_string(dir.join(main))?;
    if let Ok(t) = set_field(&draft_main, "version", &json!(next_version)) {
        updates.push((main.to_string(), t.into_bytes()));
    }
    if req.kind == Kind::Algonode {
        if let Ok(text) = std::fs::read_to_string(dir.join("contract.yaml")) {
            if text.lines().any(|l| l.starts_with("definition_version:")) {
                let replaced: Vec<String> = text
                    .lines()
                    .map(|l| if l.starts_with("definition_version:") { format!("definition_version: {}", json!(next_version)) } else { l.to_string() })
                    .collect();
                updates.push(("contract.yaml".to_string(), format!("{}\n", replaced.join("\n")).into_bytes()));
            }
        }
    }
    let next_revision = save_draft(&dir, &revision, &updates, &[]).map_err(KbServiceError::from)?;
    record_draft_session(conn, req.kind.as_str(), &req.id, &next_revision)?;

    kb_event_repo::append(
        conn,
        "published",
        &format!("{}@{version}", req.id),
        &json!({
            "kind": req.kind.as_str(),
            "release": req.release.as_str(),
            "content_id": content_id,
            "version_description": req.version_description,
            "acknowledge_unvalidated": req.acknowledge_unvalidated,
        }),
    )?;
    sync_catalog(conn, root)?;

    let (published, _) = read_bundle(&dest);
    let maturity = published
        .as_ref()
        .and_then(|b| crate::kb::catalog::status::maturity_of(b, trust::effective_trust(root, b)));
    Ok(PublishOutcome {
        id: req.id,
        version,
        content_id,
        computational_identity: lock.computational_identity.clone(),
        release_kind: req.release.as_str().to_string(),
        dependency_summary: DependencySummary { total: check.as_ref().map(|c| c.nodes.len()).unwrap_or(0), unresolved: 0 },
        verification_summary: lock.verification_summary.clone().unwrap_or_else(|| json!({ "maturity": maturity })),
        disclosures: lock.disclosures.clone(),
        next_draft_revision: next_revision,
    })
}

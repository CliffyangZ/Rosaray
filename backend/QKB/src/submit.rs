//! Submission (FR-002, plan R3): validate → (nodes) run technical verification →
//! freeze and write → re-assess. Nothing unexecutable is ever retained and no
//! draft exists: a failed submission leaves the KB byte-for-byte unchanged.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use crate::bundle::frontmatter::set_field;
use crate::bundle::index::{detect_existing, published_index, Existing};
use crate::bundle::model::{to_yaml, BundleLock, Kind, LockFile, Trust};
use crate::bundle::read::{read_bundle, Bundle, LOCK_FILE};
use crate::bundle::write::{check_relative, mark_read_only, write_bundle_atomic};
use crate::catalog::query::CatalogResolver;
use crate::catalog::repo::refresh;
use crate::catalog::status::verification_subject;
use crate::cleanup::{recompute_and_purge, remove_stale_staging};
use crate::contract::bundle::{validate_bundle, BundleCheck};
use crate::contract::finding::{Finding, Severity, Subject};
use crate::contract::publication::{check_publication, Release};
use crate::evidence::tech_verify::{run_bundle_tests, suite_of, TestError};
use crate::evidence::verification::{record_event, Event as VEvent, VerificationType};
use crate::executability::assess_dir;
use crate::guard;
use crate::identity::{computational_identity, content_id_of, is_valid_version};
use crate::trust;

const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

/// One submitted bundle: relative path → bytes. `bundle.lock` is never
/// accepted — the QKB computes identity itself.
#[derive(Debug, Clone, Default)]
pub struct Submission {
    pub files: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Accepted {
    pub kind: String,
    pub id: String,
    pub version: String,
    pub content_id: String,
    /// True when the identical version was already present (idempotent replay).
    pub already_present: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("submission_rejected")]
    Rejected(Vec<Finding>),
    #[error("version_exists")]
    VersionExists,
    #[error("rejected_content: {0}")]
    RejectedContent(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("internal: {0}")]
    Internal(String),
}

fn reject(code: &str, subject: Subject, explanation: impl Into<String>, action: &str) -> SubmitError {
    SubmitError::Rejected(vec![Finding::build(
        Severity::Error,
        code,
        &crate::contract::finding::BundleRef::new("submission", None),
        subject,
        explanation.into(),
        action,
    )])
}

struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required_text(bundle: &Bundle) -> Vec<Finding> {
    let mut out = Vec::new();
    for (field, value) in [("intended_use", &bundle.header.intended_use), ("limitations", &bundle.header.limitations)] {
        if value.as_deref().is_none_or(|v| v.trim().is_empty()) {
            out.push(Finding::build(
                Severity::Error,
                "publication_field_missing",
                &bundle.bundle_ref(),
                Subject::bundle(bundle.header.id.clone()),
                format!("`{field}` is empty; an accepted version must state it."),
                &format!("Fill in `{field}` in the frontmatter."),
            ));
        }
    }
    out
}

/// Validates and, only if everything holds, stores the submission.
pub fn submit(conn: &Connection, root: &Path, sub: Submission) -> Result<Accepted, SubmitError> {
    crate::ensure_layout(root)?;

    // 1. Shape and privacy checks on the raw files.
    if sub.files.is_empty() || sub.files.len() > MAX_FILES {
        return Err(reject("submission_shape", Subject::bundle("submission"), "A submission must contain between 1 and 256 files.", "Send the bundle's files."));
    }
    for (rel, bytes) in &sub.files {
        if check_relative(rel).is_err() || rel == LOCK_FILE || rel.split('/').any(|s| s.starts_with('.')) {
            return Err(reject("unsafe_path", Subject::file(rel.clone()), "The file path is not allowed in a submission.", "Use relative paths without `..`, hidden folders, or bundle.lock."));
        }
        if bytes.len() > MAX_FILE_BYTES {
            return Err(reject("file_too_large", Subject::file(rel.clone()), "The file exceeds the 8 MiB limit.", "Reduce the file size."));
        }
        if let Err(r) = guard::check_bundle_file(rel, bytes) {
            return Err(SubmitError::RejectedContent(format!("{rel}: {}", r.describe())));
        }
    }

    // 2. Stage under a name the catalog scanner skips (`.tmp-`).
    refresh(conn, root, &mut |_| {})?;
    let staging_root = Staging(root.join(format!(".tmp-staging-{}", uuid::Uuid::new_v4())));
    let stage_dir = staging_root.0.join("bundle");
    write_bundle_atomic(&stage_dir, &sub.files).map_err(|e| SubmitError::Internal(e.to_string()))?;

    let (bundle, mut findings) = read_bundle(&stage_dir);
    let Some(bundle) = bundle else { return Err(SubmitError::Rejected(findings)) };
    if bundle.is_published() {
        return Err(reject("lock_not_accepted", Subject::file(LOCK_FILE), "A submission must not carry a bundle.lock.", "Remove it; the QKB computes the identity."));
    }
    findings.retain(|f| f.is_error());

    // 3. Contract, identity and dependency validation (all findings together).
    let id = bundle.header.id.clone();
    let version = bundle.header.version.clone();
    if !is_valid_version(&version) || semver::Version::parse(&version).is_err() {
        findings.push(Finding::build(Severity::Error, "version_invalid", &bundle.bundle_ref(), Subject::bundle(id.clone()), "The version is not a valid SemVer string.".into(), "Use MAJOR.MINOR.PATCH."));
    }
    let mut check = None;
    let mut report = None;
    match bundle.kind {
        Kind::Algonode => {
            findings.extend(validate_bundle(&bundle, &BundleCheck::with_resolver(&CatalogResolver { conn })));
            findings.extend(required_text(&bundle));
            match &bundle.implementation {
                None => findings.push(Finding::build(Severity::Error, "implementation_unavailable", &bundle.bundle_ref(), Subject::file("implementation.yaml"), "The node has no implementation binding, so it is knowledge only and cannot be retained.".into(), "Add implementation.yaml naming an available implementation.")),
                Some(imp) if !crate::catalog::status::is_resolvable_implementation(&imp.implementation_id) => findings.push(Finding::build(Severity::Error, "implementation_unavailable", &bundle.bundle_ref(), Subject::file("implementation.yaml"), format!("The implementation \"{}\" is not available on this installation.", imp.implementation_id), "Name an available implementation id.")),
                Some(imp) if imp.trust == Trust::Untrusted => findings.push(Finding::build(Severity::Error, "implementation_untrusted", &bundle.bundle_ref(), Subject::file("implementation.yaml"), "The implementation is declared untrusted.".into(), "Only trusted implementations can be retained.")),
                Some(_) => match run_bundle_tests(&bundle) {
                    Ok(r) if r.all_passed => report = Some(r),
                    Ok(r) => findings.push(Finding::build(Severity::Error, "verification_failed", &bundle.bundle_ref(), Subject::file("tests/cases.yaml"), format!("{} of the node's own test cases failed.", r.cases.iter().filter(|c| !c.passed).count()), "Fix the implementation or the expected values.")),
                    Err(TestError::NoTests) => findings.push(Finding::build(Severity::Error, "verification_missing", &bundle.bundle_ref(), Subject::file("tests/cases.yaml"), "The node has no tests/cases.yaml, so it cannot be technically verified.".into(), "Add test cases.")),
                    Err(e) => findings.push(Finding::build(Severity::Error, "verification_missing", &bundle.bundle_ref(), Subject::file("tests/cases.yaml"), e.to_string(), "Provide readable test cases.")),
                },
            }
        }
        Kind::Algopipe => {
            let c = check_publication(conn, root, &bundle, Release::Executable);
            findings.extend(c.findings.iter().cloned());
            check = Some(c);
        }
    }
    findings.dedup_by(|a, b| a.code == b.code && a.subject == b.subject);
    if findings.iter().any(|f| f.is_error()) {
        return Err(SubmitError::Rejected(findings));
    }

    // 4. Identity: same kind for an id; version never overwritten.
    let index = published_index(root);
    if index.iter().any(|p| p.id == id && p.kind != bundle.kind) {
        return Err(reject("identity_conflict", Subject::bundle(id.clone()), format!("The id {id} is used by a bundle of a different kind."), "Choose a different id."));
    }

    // 5. Freeze: header marked published, graph pinned to exact content.
    let main = if bundle.kind == Kind::Algopipe { "ALGOPIPE.md" } else { "ALGONODE.md" };
    let mut frozen_graph = None;
    if let (Some(graph), Some(c)) = (&bundle.graph, &check) {
        let mut g = graph.clone();
        for node in &mut g.nodes {
            if let Some(resolved) = c.nodes.iter().find(|r| r.instance_id == node.instance_id) {
                node.node_ref.content_id = Some(resolved.content_id.clone());
            }
        }
        g.implementation_pins = c.pins.clone();
        frozen_graph = Some(g);
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (rel, bytes) in &sub.files {
        if rel == "graph.yaml" {
            if let Some(g) = &frozen_graph {
                files.push((rel.clone(), to_yaml(g).map_err(SubmitError::Internal)?.into_bytes()));
                continue;
            }
        }
        if rel == main {
            let text = String::from_utf8(bytes.clone()).map_err(|_| reject("not_utf8", Subject::file(main), "The header file is not UTF-8.", "Re-save as UTF-8.")).map_err(|e| e)?;
            let text = set_field(&text, "status", &json!("published"))
                .and_then(|t| set_field(&t, "research_use_only", &json!(true)))
                .and_then(|t| if bundle.kind == Kind::Algopipe { set_field(&t, "release_kind", &json!("executable")) } else { Ok(t) })
                .map_err(|e| reject(e.code, Subject::file(main), e.message, "Correct the frontmatter header."))?;
            files.push((rel.clone(), text.into_bytes()));
        } else {
            files.push((rel.clone(), bytes.clone()));
        }
    }
    let hashed: Vec<(String, String)> = files.iter().map(|(p, b)| (p.clone(), format!("b3:{}", blake3::hash(b).to_hex()))).collect();
    let content_id = content_id_of(hashed.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let kind_str = bundle.kind.as_str().to_string();
    match detect_existing(&index, &id, &version, &content_id) {
        Existing::Identical => {
            return Ok(Accepted { kind: kind_str, id, version, content_id, already_present: true });
        }
        Existing::Conflict => return Err(SubmitError::VersionExists),
        Existing::None => {}
    }
    let lock = BundleLock {
        schema: "quantify-kb/1".to_string(),
        id: id.clone(),
        version: version.clone(),
        content_id: content_id.clone(),
        computational_identity: frozen_graph.as_ref().map(computational_identity),
        release_kind: Some(if bundle.kind == Kind::Algopipe { "executable" } else { "knowledge" }.to_string()),
        disclosures: check.as_ref().map(|c| c.disclosures.clone()).unwrap_or_default(),
        dependency_summary: check.as_ref().map(|c| json!({ "total": c.nodes.len(), "unresolved": 0 })),
        verification_summary: check.as_ref().map(|c| json!({
            "verified": c.nodes.iter().filter(|n| n.verified).count(),
            "total": c.nodes.len(),
            "unverified": Vec::<String>::new(),
        })),
        files: hashed.iter().map(|(p, h)| LockFile { path: p.clone(), blake3: h.trim_start_matches("b3:").to_string() }).collect(),
    };
    files.push((LOCK_FILE.to_string(), to_yaml(&lock).map_err(SubmitError::Internal)?.into_bytes()));

    // 6. Write, record local trust + technical verification, then re-assess.
    let sub_dir = if bundle.kind == Kind::Algopipe { "pipes" } else { "nodes" };
    let dest = root.join(sub_dir).join(&id).join(&version);
    if dest.exists() {
        return Err(SubmitError::VersionExists);
    }
    write_bundle_atomic(&dest, &files).map_err(|e| SubmitError::Internal(e.to_string()))?;
    mark_read_only(&dest);

    let rollback = |conn: &Connection| {
        let _ = trust::forget(root, &id, &version, &content_id);
        // Reuse the cleanup path so read-only files and empty folders go away.
        let _ = refresh(conn, root, &mut |_| {});
        let dir = dest.clone();
        crate::cleanup::force_remove_version_dir(root, &dir);
        let _ = refresh(conn, root, &mut |_| {});
    };

    if let Some(imp) = &bundle.implementation {
        if let Err(e) = trust::record(root, &id, &version, &content_id, imp.trust, "submit") {
            rollback(conn);
            return Err(e.into());
        }
    }
    if bundle.kind == Kind::Algonode {
        let (published, _) = read_bundle(&dest);
        let Some(subject) = published.as_ref().and_then(verification_subject) else {
            rollback(conn);
            return Err(SubmitError::Internal("could not derive the verification subject".into()));
        };
        let suite = report.as_ref().map(suite_of);
        if let Err(e) = record_event(root, VerificationType::Technical, VEvent::Passed, &subject, suite, None, None) {
            rollback(conn);
            return Err(SubmitError::Internal(format!("could not record verification: {e:?}")));
        }
    }
    refresh(conn, root, &mut |_| {})?;
    let assessed = assess_dir(conn, root, &dest);
    if !assessed.executability.executable {
        let codes: Vec<&str> = assessed.executability.reasons.iter().map(|r| r.code).collect();
        rollback(conn);
        return Err(reject("not_executable", Subject::bundle(id.clone()), format!("The stored version would not be executable ({}).", codes.join(", ")), "Resolve the listed reasons and resubmit."));
    }
    crate::event_repo::append(conn, "submitted", &format!("{id}@{version}"), &json!({ "kind": kind_str, "content_id": content_id }))?;
    // Cleanup keeps invariants after every submission (it deletes nothing here
    // unless an unrelated version has meanwhile become unexecutable).
    recompute_and_purge(conn, root, "submit").map_err(|e| SubmitError::Internal(e.to_string()))?;
    remove_stale_staging(root);
    Ok(Accepted { kind: kind_str, id, version, content_id, already_present: false })
}

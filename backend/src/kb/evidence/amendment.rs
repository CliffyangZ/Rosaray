//! Evidence amendments (FR-051, SC-016): a correction or withdrawal of a piece of
//! evidence is *appended* to the version's hash-chained history outside the
//! bundle. It changes no bundle file, no Run row, and never a version's identity;
//! it is computed against the dependency index and stored, so every dependent
//! version can show it.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use super::chain::{append, ChainError, NewRecord};
use super::deprecation::{self, chain_dir};
use crate::data_repository::sqlite::kb_event_repo;
use crate::kb::bundle::read::read_bundle;
use crate::kb::bundle::service::{sync_catalog, KbServiceError};
use crate::kb::catalog::query::path_of;
use crate::kb::catalog::repo::forget_versions;

pub const KINDS: &[&str] = &["correction", "withdrawal"];

#[derive(Debug, Clone)]
pub struct AmendmentRequest {
    pub kind: String,
    pub evidence_id: String,
    pub reason: String,
    pub resulting_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AmendmentOutcome {
    pub amendment_id: String,
    pub seq: u64,
    pub chain_head: String,
    /// The amended version and every published version that depends on it.
    pub affected_versions: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AmendError {
    #[error("unknown amendment kind")]
    UnknownKind,
    #[error("a reason and resulting status are required")]
    Incomplete,
    #[error("the version has no such evidence record")]
    UnknownEvidence,
    #[error(transparent)]
    Service(#[from] KbServiceError),
    #[error(transparent)]
    Chain(#[from] ChainError),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// Every published version that references `id@version` (via the dependency index).
pub fn dependents(conn: &Connection, id: &str, version: &str) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT p.id, p.version FROM kb_dependency d
         JOIN kb_catalog_entry p ON p.path = d.from_path
         WHERE d.to_id = ?1 AND d.to_version = ?2 AND p.status = 'published' AND p.id IS NOT NULL
         ORDER BY p.id, p.version",
    )?;
    let rows = stmt.query_map([id, version], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

pub fn amend(
    conn: &Connection,
    root: &Path,
    kind_dir: &str,
    id: &str,
    version: &str,
    req: AmendmentRequest,
) -> Result<AmendmentOutcome, AmendError> {
    if !KINDS.contains(&req.kind.as_str()) {
        return Err(AmendError::UnknownKind);
    }
    if req.reason.trim().is_empty() || req.resulting_status.trim().is_empty() {
        return Err(AmendError::Incomplete);
    }
    let path = path_of(conn, kind_dir, id, version, "published")?.ok_or(KbServiceError::NotFound)?;
    let (bundle, _) = read_bundle(&root.join(&path));
    let bundle = bundle.ok_or(KbServiceError::NotFound)?;
    if !bundle.evidence.iter().any(|e| e.evidence_id == req.evidence_id) {
        return Err(AmendError::UnknownEvidence);
    }

    let mut affected: Vec<(String, String)> = dependents(conn, id, version)?;
    affected.push((id.to_string(), version.to_string()));
    affected.sort();
    affected.dedup();
    let affected_labels: Vec<String> = affected.iter().map(|(i, v)| format!("{i}@{v}")).collect();

    let mut extra = serde_json::Map::new();
    extra.insert("original_evidence".into(), json!({ "bundle": format!("{id}@{version}"), "evidence_id": req.evidence_id }));
    extra.insert("affected_versions".into(), json!(affected_labels));
    extra.insert("resulting_status".into(), json!(req.resulting_status));
    let head = append(
        &chain_dir(root, id, version),
        NewRecord {
            kind: req.kind.clone(),
            subject: json!({ "id": id, "version": version }),
            reason: Some(req.reason.trim().to_string()),
            author: kb_event_repo::LOCAL_ACTOR.to_string(),
            extra,
        },
    )?;
    kb_event_repo::append(
        conn,
        "amendment_recorded",
        &format!("{id}@{version}"),
        &json!({ "kind": req.kind, "evidence_id": req.evidence_id, "seq": head.seq, "affected": affected_labels.len() }),
    )?;
    // Dependents' folders did not change, so their rows are re-read explicitly.
    forget_versions(conn, &affected)?;
    sync_catalog(conn, root)?;
    Ok(AmendmentOutcome { amendment_id: head.record_id, seq: head.seq, chain_head: head.hash, affected_versions: affected_labels })
}

/// The amendments recorded against `id@version` itself.
pub fn list(root: &Path, id: &str, version: &str) -> std::io::Result<Vec<serde_json::Value>> {
    let chain = deprecation::read(root, id, version)?;
    Ok(chain
        .records
        .iter()
        .filter(|r| r.kind != deprecation::KIND)
        .map(|r| json!({ "seq": r.seq, "kind": r.kind, "reason": r.reason, "at": r.created_at, "author": r.author, "detail": r.extra }))
        .collect())
}

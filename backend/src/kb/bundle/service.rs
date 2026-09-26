//! Draft lifecycle for Knowledge Bundles: create (blank or duplicate), read,
//! save, discard — plus identity-conflict detection (FR-034, FR-037, FR-038).
//! Saving an invalid draft is allowed; validation is advisory here and only
//! blocks at publication.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::json;

use super::draft::{open_draft, revision_of, save_draft, DraftConflict, DraftError};
use super::frontmatter::{replace_body, set_field};
use super::model::{to_yaml, GraphFile, Kind, TargetDataProfile};
use super::read::{read_bundle, Bundle, LOCK_FILE};
use super::write::{write_bundle_atomic, WriteError};
use crate::data_repository::sqlite::kb_event_repo;
use crate::designer::validate::bundle::{validate_bundle, BundleCheck};
use crate::designer::validate::finding::Finding;
use crate::kb::catalog::query::CatalogResolver;
use crate::kb::catalog::repo::{draft_dir, forget_draft_session, record_draft_session};
use crate::kb::catalog::scan::discover_bundle_dirs;
use crate::kb::identity::validate_identity;

#[derive(Debug, thiserror::Error)]
pub enum KbServiceError {
    #[error("identity_conflict: {0}")]
    IdentityConflict(String),
    #[error("not_found")]
    NotFound,
    #[error("bundle_invalid")]
    Invalid(Vec<Finding>),
    #[error("draft_conflict")]
    DraftConflict(DraftConflict),
    #[error("published_immutable")]
    PublishedImmutable,
    #[error("not_publishable")]
    NotPublishable(Vec<Finding>),
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

impl From<DraftError> for KbServiceError {
    fn from(e: DraftError) -> Self {
        match e {
            DraftError::Conflict(c) => KbServiceError::DraftConflict(c),
            DraftError::Write(WriteError::PublishedImmutable(_)) => KbServiceError::PublishedImmutable,
            DraftError::Write(WriteError::UnsafePath(p)) => KbServiceError::UnsafePath(p),
            DraftError::Write(WriteError::Io(e)) | DraftError::Io(e) => KbServiceError::Io(e),
        }
    }
}

impl From<WriteError> for KbServiceError {
    fn from(e: WriteError) -> Self {
        DraftError::Write(e).into()
    }
}

/// Accepts `algonode|algopipe` and the `nodes|pipes` aliases used in paths.
pub fn parse_kind(s: &str) -> Option<Kind> {
    match s {
        "algonode" | "nodes" | "node" => Some(Kind::Algonode),
        "algopipe" | "pipes" | "pipe" => Some(Kind::Algopipe),
        _ => None,
    }
}

// ---- published index & conflict detection (FR-037) ----------------------------

#[derive(Debug, Clone)]
pub struct PublishedRef {
    pub kind: Kind,
    pub id: String,
    pub version: String,
    pub content_id: String,
    pub dir: PathBuf,
}

/// Every published (locked) bundle on disk, by lightly reading identity and
/// `bundle.lock`; the KB folders — not the catalog — are authoritative.
pub fn published_index(root: &Path) -> Vec<PublishedRef> {
    let mut out = Vec::new();
    for dir in discover_bundle_dirs(root) {
        let Ok(text) = std::fs::read_to_string(dir.join(LOCK_FILE)) else { continue };
        let Ok(lock) = super::model::parse_yaml::<super::model::BundleLock>(&text) else { continue };
        let kind = if dir.join("ALGOPIPE.md").is_file() { Kind::Algopipe } else { Kind::Algonode };
        out.push(PublishedRef { kind, id: lock.id, version: lock.version, content_id: lock.content_id, dir });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Existing {
    None,
    /// The same `id@version` with the same content is already present.
    Identical,
    /// The same `id@version` exists with different content (`identity_conflict`).
    Conflict,
}

pub fn detect_existing(index: &[PublishedRef], id: &str, version: &str, content_id: &str) -> Existing {
    match index.iter().find(|p| p.id == id && p.version == version) {
        None => Existing::None,
        Some(p) if p.content_id == content_id => Existing::Identical,
        Some(_) => Existing::Conflict,
    }
}

// ---- create ----------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CreateDraft {
    pub id: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    /// Duplicate this published bundle instead of starting blank.
    pub from: Option<(String, String)>,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DraftHandle {
    pub kind: Kind,
    pub id: String,
    pub revision: String,
    pub findings: Vec<Finding>,
}

fn frontmatter_yaml(kind: Kind, id: &str, version: &str, name: Option<&str>, summary: Option<&str>) -> String {
    let mut m = serde_yaml_ng::Mapping::new();
    let mut put = |k: &str, v: serde_yaml_ng::Value| {
        m.insert(serde_yaml_ng::Value::String(k.into()), v);
    };
    let s = |v: &str| serde_yaml_ng::Value::String(v.to_string());
    put("schema", s("quantify-kb/1"));
    put("kind", s(kind.as_str()));
    put("id", s(id));
    put("version", s(version));
    put("name", s(name.unwrap_or(id)));
    if let Some(sum) = summary {
        put("summary", s(sum));
    }
    put("status", s("draft"));
    put("research_use_only", serde_yaml_ng::Value::Bool(true));
    put("intended_use", s(""));
    put("limitations", s(""));
    serde_yaml_ng::to_string(&m).expect("mapping serializes")
}

fn blank_files(kind: Kind, req: &CreateDraft, version: &str) -> Vec<(String, Vec<u8>)> {
    let head = frontmatter_yaml(kind, &req.id, version, req.name.as_deref(), req.summary.as_deref());
    match kind {
        Kind::Algonode => vec![
            ("ALGONODE.md".into(), format!("---\n{head}---\n").into_bytes()),
            (
                "contract.yaml".into(),
                b"schema: quantify-kb/1\ninputs: []\noutputs: []\nparameters: []\nprerequisites: []\n".to_vec(),
            ),
        ],
        Kind::Algopipe => vec![
            ("ALGOPIPE.md".into(), format!("---\n{head}---\n").into_bytes()),
            ("graph.yaml".into(), b"schema: quantify-kb/1\nnodes: []\nedges: []\n".to_vec()),
        ],
    }
}

/// Reads a bundle directory as `(relative path, bytes)` pairs.
pub fn read_files(dir: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for (rel, _) in crate::kb::identity::file_hashes(dir)? {
        out.push((rel.clone(), std::fs::read(dir.join(&rel))?));
    }
    Ok(out)
}

pub fn create_draft(conn: &Connection, root: &Path, kind: Kind, req: CreateDraft) -> Result<DraftHandle, KbServiceError> {
    let id_findings: Vec<Finding> = validate_identity(&req.id, req.version.as_deref().unwrap_or("0.1.0"));
    if !id_findings.is_empty() {
        return Err(KbServiceError::Invalid(id_findings));
    }
    let dir = draft_dir(root, kind.as_str(), &req.id);
    if dir.exists() {
        return Err(KbServiceError::IdentityConflict(format!("a draft with id {} already exists", req.id)));
    }
    let published = published_index(root);
    if published.iter().any(|p| p.id == req.id && p.kind != kind) {
        return Err(KbServiceError::IdentityConflict(format!(
            "the id {} is already used by a published bundle of a different kind",
            req.id
        )));
    }

    let files = match &req.from {
        None => blank_files(kind, &req, req.version.as_deref().unwrap_or("0.1.0")),
        Some((from_id, from_version)) => {
            let source = published
                .iter()
                .find(|p| p.kind == kind && &p.id == from_id && &p.version == from_version)
                .ok_or(KbServiceError::NotFound)?;
            let version = req.version.clone().unwrap_or_else(|| {
                if from_id == &req.id {
                    bump_patch(from_version)
                } else {
                    "0.1.0".to_string()
                }
            });
            duplicate_files(source, kind, &req, &version)?
        }
    };
    write_bundle_atomic(&dir, &files)?;

    let (bundle, mut findings, revision) = open_draft(&dir)?;
    findings.extend(advisory_findings(conn, bundle.as_ref()));
    record_draft_session(conn, kind.as_str(), &req.id, &revision)?;
    kb_event_repo::append(conn, "draft_saved", &format!("{}:{}", kind.as_str(), req.id), &json!({ "revision": revision, "created": true }))?;
    sync_catalog(conn, root)?;
    Ok(DraftHandle { kind, id: req.id, revision, findings })
}

fn duplicate_files(
    source: &PublishedRef,
    kind: Kind,
    req: &CreateDraft,
    version: &str,
) -> Result<Vec<(String, Vec<u8>)>, KbServiceError> {
    let main = if kind == Kind::Algopipe { "ALGOPIPE.md" } else { "ALGONODE.md" };
    let mut files = Vec::new();
    for (rel, bytes) in read_files(&source.dir)? {
        if rel == LOCK_FILE {
            continue;
        }
        if rel == main {
            let mut text = String::from_utf8(bytes).map_err(|_| KbServiceError::UnsafePath(rel.clone()))?;
            let bad = |_| KbServiceError::Invalid(Vec::new());
            text = set_field(&text, "id", &json!(req.id)).map_err(bad)?;
            text = set_field(&text, "version", &json!(version)).map_err(bad)?;
            text = set_field(&text, "status", &json!("draft")).map_err(bad)?;
            if let Some(name) = &req.name {
                text = set_field(&text, "name", &json!(name)).map_err(bad)?;
            }
            files.push((rel, text.into_bytes()));
        } else {
            files.push((rel, bytes));
        }
    }
    Ok(files)
}

pub fn bump_patch(version: &str) -> String {
    match semver::Version::parse(version) {
        Ok(mut v) => {
            v.patch += 1;
            v.pre = semver::Prerelease::EMPTY;
            v.build = semver::BuildMetadata::EMPTY;
            v.to_string()
        }
        Err(_) => "0.1.0".to_string(),
    }
}

// ---- read ------------------------------------------------------------------------

pub struct DraftView {
    pub bundle: Option<Bundle>,
    pub findings: Vec<Finding>,
    pub revision: String,
    /// UTF-8 files for editing; binary files are omitted.
    pub files: BTreeMap<String, String>,
}

pub fn read_draft(conn: &Connection, root: &Path, kind: Kind, id: &str) -> Result<DraftView, KbServiceError> {
    let dir = draft_dir(root, kind.as_str(), id);
    if !dir.is_dir() {
        return Err(KbServiceError::NotFound);
    }
    let (bundle, mut findings, revision) = open_draft(&dir)?;
    findings.extend(advisory_findings(conn, bundle.as_ref()));
    record_draft_session(conn, kind.as_str(), id, &revision)?;
    let files = read_files(&dir)?
        .into_iter()
        .filter_map(|(p, b)| String::from_utf8(b).ok().map(|t| (p, t)))
        .collect();
    Ok(DraftView { bundle, findings, revision, files })
}

/// Advisory (non-blocking) findings for a draft, resolving pipe references
/// against the catalog.
pub fn advisory_findings(conn: &Connection, bundle: Option<&Bundle>) -> Vec<Finding> {
    match bundle {
        Some(b) => validate_bundle(b, &BundleCheck::with_resolver(&CatalogResolver { conn })),
        None => Vec::new(),
    }
}

// ---- save ------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SaveOutcome {
    pub revision: String,
    pub findings: Vec<Finding>,
}

pub fn save_draft_files(
    conn: &Connection,
    root: &Path,
    kind: Kind,
    id: &str,
    base_revision: &str,
    files: &BTreeMap<String, String>,
) -> Result<SaveOutcome, KbServiceError> {
    let dir = draft_dir(root, kind.as_str(), id);
    if !dir.is_dir() {
        // A published bundle at this id is immutable; anything else is unknown.
        return Err(if published_index(root).iter().any(|p| p.id == id && p.kind == kind) {
            KbServiceError::PublishedImmutable
        } else {
            KbServiceError::NotFound
        });
    }
    let pairs: Vec<(String, Vec<u8>)> = files.iter().map(|(p, t)| (p.clone(), t.clone().into_bytes())).collect();
    let revision = save_draft(&dir, base_revision, &pairs, &[])?;
    finish_save(conn, root, &dir, kind, id, revision)
}

fn finish_save(conn: &Connection, root: &Path, dir: &Path, kind: Kind, id: &str, revision: String) -> Result<SaveOutcome, KbServiceError> {
    let (bundle, mut findings) = read_bundle(dir);
    findings.extend(advisory_findings(conn, bundle.as_ref()));
    record_draft_session(conn, kind.as_str(), id, &revision)?;
    kb_event_repo::append(conn, "draft_saved", &format!("{}:{id}", kind.as_str()), &json!({ "revision": revision }))?;
    sync_catalog(conn, root)?;
    Ok(SaveOutcome { revision, findings })
}

/// Structured AlgoPipe save (T049): the service serializes the graph and
/// description, keeping presentation-only `layout` separate from
/// computational fields. `ref.version: draft` is allowed here (drafts only).
pub fn save_pipe_structured(
    conn: &Connection,
    root: &Path,
    id: &str,
    base_revision: &str,
    graph: GraphFile,
    profile: Option<TargetDataProfile>,
    description: Option<String>,
) -> Result<SaveOutcome, KbServiceError> {
    let dir = draft_dir(root, "algopipe", id);
    if !dir.is_dir() {
        return Err(if published_index(root).iter().any(|p| p.id == id && p.kind == Kind::Algopipe) {
            KbServiceError::PublishedImmutable
        } else {
            KbServiceError::NotFound
        });
    }
    let mut graph = graph;
    if graph.schema.is_empty() {
        graph.schema = "quantify-kb/1".to_string();
    }
    if profile.is_some() {
        graph.target_data_profile = profile;
    }
    let graph_yaml = to_yaml(&graph).map_err(|e| KbServiceError::UnsafePath(e))?;
    let mut files = vec![("graph.yaml".to_string(), graph_yaml.into_bytes())];
    if let Some(desc) = description {
        let current = std::fs::read_to_string(dir.join("ALGOPIPE.md"))?;
        let updated = replace_body(&current, &desc).map_err(|e| KbServiceError::UnsafePath(e.message))?;
        files.push(("ALGOPIPE.md".to_string(), updated.into_bytes()));
    }
    let revision = save_draft(&dir, base_revision, &files, &[])?;
    finish_save(conn, root, &dir, Kind::Algopipe, id, revision)
}

// ---- discard ---------------------------------------------------------------------

/// Deletes a draft. Published bundles are never deletable here.
pub fn discard_draft(conn: &Connection, root: &Path, kind: Kind, id: &str) -> Result<(), KbServiceError> {
    let dir = draft_dir(root, kind.as_str(), id);
    if !dir.is_dir() {
        return Err(if published_index(root).iter().any(|p| p.id == id && p.kind == kind) {
            KbServiceError::PublishedImmutable
        } else {
            KbServiceError::NotFound
        });
    }
    if dir.join(LOCK_FILE).exists() {
        return Err(KbServiceError::PublishedImmutable);
    }
    std::fs::remove_dir_all(&dir)?;
    forget_draft_session(conn, kind.as_str(), id)?;
    kb_event_repo::append(conn, "draft_saved", &format!("{}:{id}", kind.as_str()), &json!({ "discarded": true }))?;
    sync_catalog(conn, root)?;
    Ok(())
}

/// Brings the derived catalog in line with the folders after a mutation.
/// Progress and external-change reports are not needed here.
pub fn sync_catalog(conn: &Connection, root: &Path) -> rusqlite::Result<()> {
    crate::kb::catalog::repo::refresh(conn, root, &mut |_| {}).map(|_| ())
}

/// Current revision of a draft directory (for callers that only need it).
pub fn draft_revision(root: &Path, kind: Kind, id: &str) -> std::io::Result<String> {
    revision_of(&draft_dir(root, kind.as_str(), id))
}

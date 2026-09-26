//! `.algobundle` importer (FR-045, FR-047, research §15–16). Import is staged:
//! the archive is safety-checked, extracted into a private staging folder,
//! verified against its manifest and every `bundle.lock`, guarded for patient
//! data, and turned into a plan. Only `confirm` touches the knowledge base, and
//! only when nothing conflicts. Nothing in an archive is ever executed, and
//! imported implementations are recorded `untrusted` (locally, see `trust`).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use super::export::{ExecutionDep, Manifest, MANIFEST_FILE};
use super::patient_guard::{check_file, finding_for, known_content, parse_reviews, Verdict, REVIEW_FILE};
use crate::data_repository::sqlite::kb_event_repo;
use crate::designer::validate::finding::{BundleRef, Finding, Severity, Subject};
use crate::kb::bundle::model::{parse_yaml, Kind, Trust};
use crate::kb::bundle::read::{read_bundle, LOCK_FILE};
use crate::kb::bundle::service::{detect_existing, published_index, sync_catalog, Existing};
use crate::kb::bundle::write::{mark_read_only, write_bundle_atomic};
use crate::kb::evidence::chain::read_chain;
use crate::kb::trust;

pub const MAX_ENTRIES: usize = 20_000;
pub const MAX_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RATIO: u64 = 100;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The archive itself is unsafe or malformed (zip-slip, symlink, bomb, …).
    #[error("archive_rejected: {0}")]
    ArchiveRejected(String),
    #[error("patient_data_blocked")]
    PatientDataBlocked(Vec<Finding>),
    #[error("identity_conflict")]
    IdentityConflict(Vec<PlanItem>),
    #[error("not_found")]
    NotFound,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanItem {
    pub kind: Kind,
    pub id: String,
    pub version: String,
    pub content_id: String,
    /// Only for `unsupported`: why the bundle cannot be imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportPlan {
    pub new: Vec<PlanItem>,
    pub identical: Vec<PlanItem>,
    pub conflicts: Vec<PlanItem>,
    pub omitted_execution_deps: Vec<ExecutionDep>,
    pub unsupported: Vec<PlanItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Staged {
    pub staged_id: Uuid,
    pub plan: ImportPlan,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyOutcome {
    pub applied: Vec<PlanItem>,
    pub identical: Vec<PlanItem>,
}

/// Staging lives beside (not inside) the KB root so staged bundles are never
/// scanned into the catalog.
pub fn staging_root(kb_root: &Path) -> PathBuf {
    kb_root.with_file_name("kb-staging")
}

fn reject(msg: impl Into<String>) -> ImportError {
    ImportError::ArchiveRejected(msg.into())
}

/// Validates an archive entry name: relative, no `..`, no backslashes, no
/// drive letters.
fn safe_name(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') || name.starts_with('/') || name.contains(':') {
        return None;
    }
    let path = Path::new(name);
    for c in path.components() {
        match c {
            std::path::Component::Normal(_) => {}
            _ => return None,
        }
    }
    Some(path.to_path_buf())
}

/// Extracts to `dest` under the safety limits; returns the relative names.
fn extract(zip_bytes: &[u8], dest: &Path) -> Result<Vec<String>, ImportError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| reject("not a valid archive"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(reject("too many entries"));
    }
    let mut total: u64 = 0;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|_| reject("unreadable entry"))?;
        if entry.is_dir() {
            continue;
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(reject("symbolic links are not allowed"));
        }
        let name = entry.name().to_string();
        let rel = safe_name(&name).ok_or_else(|| reject("an entry has an unsafe path"))?;
        if entry.compressed_size() > 0 && entry.size() / entry.compressed_size() > MAX_RATIO {
            return Err(reject("compression ratio too high"));
        }
        // Enforce the size cap on what is actually read, not on the declared size.
        let remaining = MAX_EXPANDED_BYTES.saturating_sub(total);
        let mut data = Vec::new();
        entry.by_ref().take(remaining + 1).read_to_end(&mut data).map_err(|_| reject("unreadable entry"))?;
        total += data.len() as u64;
        if total > MAX_EXPANDED_BYTES {
            return Err(reject("archive expands beyond the size limit"));
        }
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, &data)?;
        names.push(name);
    }
    if total > 0 {
        let compressed = zip_bytes.len().max(1) as u64;
        if total / compressed > MAX_RATIO {
            return Err(reject("compression ratio too high"));
        }
    }
    Ok(names)
}

/// Splits `bundles/<kind>/<id>/<version>/<rel>`.
fn parse_bundle_path(name: &str) -> Option<(Kind, String, String, String)> {
    let mut parts = name.splitn(5, '/');
    if parts.next()? != "bundles" {
        return None;
    }
    let kind = crate::kb::bundle::service::parse_kind(parts.next()?)?;
    let id = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    let rel = parts.next()?.to_string();
    Some((kind, id, version, rel))
}

pub fn stage(conn: &Connection, root: &Path, zip_bytes: &[u8]) -> Result<Staged, ImportError> {
    let staged_id = Uuid::new_v4();
    let dir = staging_root(root).join(staged_id.to_string());
    let files_dir = dir.join("files");
    std::fs::create_dir_all(&files_dir)?;
    let result = stage_into(conn, root, zip_bytes, &dir, &files_dir, staged_id);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

fn stage_into(
    conn: &Connection,
    root: &Path,
    zip_bytes: &[u8],
    dir: &Path,
    files_dir: &Path,
    staged_id: Uuid,
) -> Result<Staged, ImportError> {
    let names = extract(zip_bytes, files_dir)?;
    let manifest_text = std::fs::read_to_string(files_dir.join(MANIFEST_FILE)).map_err(|_| reject("manifest.yaml is missing"))?;
    let manifest: Manifest = parse_yaml(&manifest_text).map_err(|_| reject("manifest.yaml is not valid"))?;
    if crate::kb::bundle::frontmatter::check_schema(&manifest.schema).is_err() {
        return Err(reject("unsupported manifest schema"));
    }

    // Every file must be listed, and match its recorded hash.
    let listed: BTreeMap<&str, &str> = manifest.entries.iter().map(|e| (e.path.as_str(), e.blake3.as_str())).collect();
    for name in &names {
        if name != MANIFEST_FILE && !listed.contains_key(name.as_str()) {
            return Err(reject("the archive holds a file its manifest does not list"));
        }
    }
    for (path, hash) in &listed {
        let bytes = std::fs::read(files_dir.join(safe_name(path).ok_or_else(|| reject("unsafe manifest path"))?))
            .map_err(|_| reject("a listed file is missing"))?;
        if format!("b3:{}", blake3::hash(&bytes).to_hex()) != *hash {
            return Err(reject("a file does not match its manifest hash"));
        }
    }

    // Group files by bundle.
    let mut bundles: BTreeMap<(Kind, String, String), Vec<String>> = BTreeMap::new();
    let mut amendment_chains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in listed.keys() {
        if let Some((kind, id, version, _rel)) = parse_bundle_path(name) {
            bundles.entry((kind, id, version)).or_default().push(name.to_string());
        } else if let Some(rest) = name.strip_prefix("amendments/") {
            let target = rest.split('/').next().unwrap_or("").to_string();
            amendment_chains.entry(target).or_default().push(name.to_string());
        } else {
            return Err(reject("the archive has an unsupported layout"));
        }
    }

    let known = known_content(conn)?;
    let index = published_index(root);
    let mut plan = ImportPlan { omitted_execution_deps: manifest.omitted_execution_deps.clone(), ..Default::default() };
    let mut findings: Vec<Finding> = Vec::new();
    let mut blocked: Vec<Finding> = Vec::new();

    for ((kind, id, version), paths) in &bundles {
        let bundle_dir = files_dir.join(super::export::bundle_archive_dir(*kind, id, version));
        let mut item = PlanItem { kind: *kind, id: id.clone(), version: version.clone(), content_id: String::new(), code: None };

        // Patient-data guard over every file — before anything else.
        let bref = BundleRef::new(id.clone(), Some(version.clone()));
        let reviews = std::fs::read_to_string(bundle_dir.join(REVIEW_FILE)).map(|t| parse_reviews(&t)).unwrap_or_default();
        let mut unresolved = false;
        for p in paths {
            let rel = p.strip_prefix(&format!("{}/", super::export::bundle_archive_dir(*kind, id, version))).unwrap_or(p);
            let bytes = std::fs::read(files_dir.join(p))?;
            let verdict = check_file(rel, &bytes, &known, &reviews);
            if let Some(f) = finding_for(&bref, rel, &verdict) {
                match verdict {
                    Verdict::Blocked(_) => blocked.push(f),
                    _ => {
                        unresolved = true;
                        findings.push(f);
                    }
                }
            }
        }
        if unresolved {
            item.code = Some(super::patient_guard::CODE_UNRESOLVED.into());
            plan.unsupported.push(item);
            continue;
        }

        let (bundle, rf) = read_bundle(&bundle_dir);
        let Some(bundle) = bundle else {
            item.code = Some("bundle_invalid".into());
            findings.extend(rf);
            plan.unsupported.push(item);
            continue;
        };
        let Some(lock) = &bundle.lock else {
            item.code = Some("import_unlocked_bundle".into());
            findings.push(Finding::build(
                Severity::Error,
                "import_unlocked_bundle",
                &bref,
                Subject::bundle(id.clone()),
                format!("{id}@{version} in the archive is not a published, hash-locked version."),
                "Only published versions can be exchanged; publish it first.",
            ));
            plan.unsupported.push(item);
            continue;
        };
        item.content_id = lock.content_id.clone();
        if let Some(bad) = rf.iter().find(|f| f.is_error()) {
            item.code = Some(bad.code.clone());
            findings.extend(rf);
            plan.unsupported.push(item);
            continue;
        }
        if bundle.header.id != *id || bundle.header.version != *version || bundle.kind != *kind {
            item.code = Some("frontmatter_contract_mismatch".into());
            plan.unsupported.push(item);
            continue;
        }
        match detect_existing(&index, id, version, &item.content_id) {
            Existing::None => plan.new.push(item),
            Existing::Identical => plan.identical.push(item),
            Existing::Conflict => plan.conflicts.push(item),
        }
    }

    if !blocked.is_empty() {
        return Err(ImportError::PatientDataBlocked(blocked));
    }

    // Amendment chains must verify; a broken one is reported and not applied.
    for (target, paths) in &amendment_chains {
        let chain_dir = files_dir.join("amendments").join(target);
        if let Ok(read) = read_chain(&chain_dir) {
            if let Some(brk) = read.broken {
                findings.push(brk.to_finding(&BundleRef::new(target.clone(), None)));
                for p in paths {
                    let _ = std::fs::remove_file(files_dir.join(p));
                }
            }
        }
    }

    std::fs::write(dir.join("plan.json"), serde_json::to_vec(&plan).expect("plan serializes"))?;
    Ok(Staged { staged_id, plan, findings })
}

fn load_staged(root: &Path, staged_id: Uuid) -> Result<(PathBuf, ImportPlan), ImportError> {
    let dir = staging_root(root).join(staged_id.to_string());
    let text = std::fs::read(dir.join("plan.json")).map_err(|_| ImportError::NotFound)?;
    let plan = serde_json::from_slice(&text).map_err(|_| ImportError::NotFound)?;
    Ok((dir, plan))
}

/// Applies a staged import. All-or-nothing: any conflict (same `id@version`,
/// different content) refuses the whole import and writes nothing (US1-7).
pub fn confirm(conn: &Connection, root: &Path, staged_id: Uuid) -> Result<ApplyOutcome, ImportError> {
    let (dir, plan) = load_staged(root, staged_id)?;
    let files_dir = dir.join("files");

    // Re-check against the current KB: it may have changed since staging.
    let index = published_index(root);
    let mut conflicts = plan.conflicts.clone();
    let mut to_apply = Vec::new();
    let mut identical = plan.identical.clone();
    for item in &plan.new {
        match detect_existing(&index, &item.id, &item.version, &item.content_id) {
            Existing::None => to_apply.push(item.clone()),
            Existing::Identical => identical.push(item.clone()),
            Existing::Conflict => conflicts.push(item.clone()),
        }
    }
    if !conflicts.is_empty() {
        return Err(ImportError::IdentityConflict(conflicts));
    }

    for item in &to_apply {
        let src = files_dir.join(super::export::bundle_archive_dir(item.kind, &item.id, &item.version));
        let sub = if item.kind == Kind::Algopipe { "pipes" } else { "nodes" };
        let dest = root.join(sub).join(&item.id).join(&item.version);
        let files = crate::kb::bundle::service::read_files(&src)?;
        write_bundle_atomic(&dest, &files).map_err(|e| ImportError::Io(std::io::Error::other(e.to_string())))?;
        mark_read_only(&dest);
        // Local, untrusted by default: never marks the bundle `implemented`.
        trust::record(root, &item.id, &item.version, &item.content_id, Trust::Untrusted, "import")?;
    }

    // Amendment chains that this KB does not have yet.
    let chains_root = files_dir.join("amendments");
    if chains_root.is_dir() {
        for entry in std::fs::read_dir(&chains_root)?.filter_map(|e| e.ok()) {
            let target = entry.file_name().to_string_lossy().to_string();
            let dest = root.join("amendments").join(&target);
            if !dest.exists() {
                std::fs::create_dir_all(&dest)?;
                for (rel, bytes) in crate::kb::bundle::service::read_files(&entry.path())? {
                    std::fs::write(dest.join(rel), bytes)?;
                }
            }
        }
    }

    kb_event_repo::append(
        conn,
        "imported",
        "algobundle",
        &json!({ "applied": to_apply.len(), "identical": identical.len(), "omitted_execution_deps": plan.omitted_execution_deps.len() }),
    )?;
    let _ = std::fs::remove_dir_all(&dir);
    sync_catalog(conn, root)?;
    Ok(ApplyOutcome { applied: to_apply, identical })
}

pub fn cancel(root: &Path, staged_id: Uuid) -> Result<(), ImportError> {
    let dir = staging_root(root).join(staged_id.to_string());
    if !dir.is_dir() {
        return Err(ImportError::NotFound);
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

/// Referenced so the lock file name has one definition site for callers.
pub const BUNDLE_LOCK: &str = LOCK_FILE;

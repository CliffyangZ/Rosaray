//! Catalog scanner (FR-040): walks the KB root, reads every bundle-shaped
//! folder, and turns each into a catalog row plus findings. A bundle that
//! fails to read is *indexed as `invalid`*, never skipped. Identity comes from
//! frontmatter, not the folder name; a folder that disagrees only earns a
//! `path_identity_mismatch` warning (US1-5).

use std::path::{Path, PathBuf};

use super::status::derive;
use crate::contract::bundle::{validate_bundle, BundleCheck};
use crate::contract::finding::{BundleRef, Finding, Severity, Subject};
use crate::bundle::model::Kind;
use crate::bundle::read::{read_bundle, Bundle};

/// Top-level folders that hold history chains, not bundles.
const NON_BUNDLE_ROOTS: &[&str] = &["amendments", "verification"];
const BUNDLE_MARKERS: &[&str] = &["ALGONODE.md", "ALGOPIPE.md", "contract.yaml", "graph.yaml", "bundle.lock"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStatus {
    Draft,
    Published,
    Invalid,
}

impl EntryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntryStatus::Draft => "draft",
            EntryStatus::Published => "published",
            EntryStatus::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSignature {
    /// Newest modification time of any file or folder, in milliseconds.
    pub mtime_ms: i64,
    /// Total bytes of all files.
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct DependencyRef {
    pub to_id: String,
    pub to_version: String,
    pub to_content_id: Option<String>,
}

/// One catalog row, before persistence.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    /// Relative to the KB root, `/`-separated.
    pub path: String,
    pub signature: DirSignature,
    pub id: Option<String>,
    pub version: Option<String>,
    pub kind: Option<Kind>,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub purpose: Option<String>,
    pub intended_use: Option<String>,
    pub domain: Option<String>,
    pub status: EntryStatus,
    pub maturity: Option<String>,
    pub release_kind: Option<String>,
    pub availability: String,
    pub verification: Option<String>,
    pub dataset_validation: String,
    pub origin: Option<String>,
    pub amendment_count: i64,
    pub data_kinds: Option<String>,
    pub content_id: Option<String>,
    pub dependencies: Vec<DependencyRef>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Copy)]
pub enum ScanEvent {
    Progress { scanned: u64, total: Option<u64>, invalid: u64 },
    Complete { indexed: u64, invalid: u64 },
}

fn is_transient(name: &str) -> bool {
    name.contains(".tmp-") || name.contains(".old-")
}

/// Every bundle-shaped folder under `root` (a folder holding any marker
/// file), skipping history chains and staging leftovers. Bundle folders are
/// not descended into.
pub fn discover_bundle_dirs(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, top: bool, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut children: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        children.sort();
        if !top && BUNDLE_MARKERS.iter().any(|m| dir.join(m).is_file()) {
            out.push(dir.to_path_buf());
            return;
        }
        for child in children {
            if !child.is_dir() {
                continue;
            }
            let name = child.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if is_transient(&name) || (top && NON_BUNDLE_ROOTS.contains(&name.as_str())) {
                continue;
            }
            walk(&child, false, out);
        }
    }
    let mut out = Vec::new();
    walk(root, true, &mut out);
    out
}

pub fn rel_path(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn dir_signature(dir: &Path) -> DirSignature {
    fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    fn walk(dir: &Path, sig: &mut DirSignature) {
        if let Ok(meta) = std::fs::metadata(dir) {
            sig.mtime_ms = sig.mtime_ms.max(mtime_ms(&meta));
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, sig);
            } else if let Ok(meta) = entry.metadata() {
                sig.mtime_ms = sig.mtime_ms.max(mtime_ms(&meta));
                sig.size += meta.len();
            }
        }
    }
    let mut sig = DirSignature { mtime_ms: 0, size: 0 };
    walk(dir, &mut sig);
    sig
}

/// The bundle folder's signature folded with the history chains that describe
/// it (`amendments/<id>@<version>`, `verification/<id>`), so appending a
/// deprecation or verification record is noticed by incremental refresh.
pub fn signature_with_chains(root: &Path, dir: &Path, id: Option<&str>, version: Option<&str>) -> DirSignature {
    let mut sig = dir_signature(dir);
    let mut fold = |extra: DirSignature| {
        sig.mtime_ms = sig.mtime_ms.max(extra.mtime_ms);
        sig.size += extra.size;
    };
    if let (Some(id), Some(version)) = (id, version) {
        fold(dir_signature(&root.join("amendments").join(format!("{id}@{version}"))));
    }
    if let Some(id) = id {
        fold(dir_signature(&root.join("verification").join(id)));
    }
    // A local trust decision changes maturity without touching the bundle.
    if let Ok(meta) = std::fs::metadata(root.join(crate::trust::TRUST_FILE)) {
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
        fold(DirSignature { mtime_ms: mtime, size: meta.len() });
    }
    sig
}

/// Reads one bundle folder into a catalog row.
pub fn scan_bundle(root: &Path, dir: &Path) -> ScanEntry {
    let path = rel_path(root, dir);
    let (bundle, mut findings) = read_bundle(dir);
    let signature = signature_with_chains(
        root,
        dir,
        bundle.as_ref().map(|b| b.header.id.as_str()).filter(|s| !s.is_empty()),
        bundle.as_ref().map(|b| b.header.version.as_str()).filter(|s| !s.is_empty()),
    );

    let folder_name = dir.file_name().map(|n| n.to_string_lossy().to_string());
    let guessed_kind = if dir.join("ALGOPIPE.md").is_file() || dir.join("graph.yaml").is_file() {
        Some(Kind::Algopipe)
    } else if dir.join("ALGONODE.md").is_file() || dir.join("contract.yaml").is_file() {
        Some(Kind::Algonode)
    } else {
        None
    };

    // Reader errors make a bundle `invalid` — except a hash mismatch on a
    // published version, which leaves it readable but `unavailable`.
    let unreadable = bundle.is_none()
        || findings
            .iter()
            .any(|f| f.severity == Severity::Error && f.code != "published_bundle_modified");

    let Some(bundle) = bundle else {
        return ScanEntry {
            path,
            signature,
            id: None,
            version: None,
            kind: guessed_kind,
            name: folder_name,
            summary: None,
            purpose: None,
            intended_use: None,
            domain: None,
            status: EntryStatus::Invalid,
            maturity: None,
            release_kind: None,
            availability: "unavailable".to_string(),
            verification: None,
            dataset_validation: "none".to_string(),
            origin: None,
            amendment_count: 0,
            data_kinds: None,
            content_id: None,
            dependencies: Vec::new(),
            findings,
        };
    };

    findings.extend(validate_bundle(&bundle, &BundleCheck::files_only()));
    if let Some(f) = identity_path_finding(&bundle, root) {
        findings.push(f);
    }

    let published = bundle.is_published();
    let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
    let derived = derive(root, &bundle, modified);
    findings.extend(derived.findings.clone());
    let status = if unreadable {
        EntryStatus::Invalid
    } else if published {
        EntryStatus::Published
    } else {
        EntryStatus::Draft
    };

    let dependencies = bundle
        .graph
        .as_ref()
        .map(|g| {
            let mut seen = std::collections::BTreeSet::new();
            g.nodes
                .iter()
                .filter(|n| seen.insert((n.node_ref.id.clone(), n.node_ref.version.clone())))
                .map(|n| DependencyRef {
                    to_id: n.node_ref.id.clone(),
                    to_version: n.node_ref.version.clone(),
                    to_content_id: n.node_ref.content_id.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    let data_kinds = bundle.contract.as_ref().map(|c| {
        let mut kinds: Vec<&str> = c
            .inputs
            .iter()
            .chain(c.outputs.iter())
            .filter_map(|p| p.artifact_kind.as_deref())
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds.join(" ")
    });

    let release_kind = if published {
        Some(bundle.lock.as_ref().and_then(|l| l.release_kind.clone()).unwrap_or_else(|| "knowledge".to_string()))
    } else {
        Some("draft".to_string())
    };

    ScanEntry {
        path,
        signature,
        id: non_empty(&bundle.header.id),
        version: non_empty(&bundle.header.version),
        kind: Some(bundle.kind),
        name: bundle.header.name.clone(),
        summary: bundle.header.summary.clone(),
        purpose: bundle.contract.as_ref().and_then(|c| c.purpose.clone()),
        intended_use: bundle.header.intended_use.clone(),
        domain: bundle.header.domain.clone(),
        status,
        maturity: derived.maturity.map(str::to_string),
        release_kind,
        availability: if unreadable { "unavailable" } else { derived.availability }.to_string(),
        verification: (bundle.kind == Kind::Algonode).then(|| derived.verification.as_str().to_string()),
        dataset_validation: derived.dataset_validation.to_string(),
        origin: bundle.header.origin.clone(),
        amendment_count: amendment_count(root, &bundle),
        data_kinds,
        content_id: bundle.lock.as_ref().map(|l| l.content_id.clone()),
        dependencies,
        findings,
    }
}

/// Amendments recorded against this version, plus — for a pipe — those recorded
/// against the versions it depends on, so a reader of the pipe sees them (FR-051).
fn amendment_count(root: &Path, bundle: &Bundle) -> i64 {
    let count = |id: &str, version: &str| {
        crate::evidence::deprecation::read(root, id, version).map(|c| c.records.iter().filter(|r| r.kind != crate::evidence::deprecation::KIND).count() as i64).unwrap_or(0)
    };
    let mut n = if bundle.is_published() { count(&bundle.header.id, &bundle.header.version) } else { 0 };
    if let Some(g) = &bundle.graph {
        let mut seen = std::collections::BTreeSet::new();
        for node in &g.nodes {
            if seen.insert((node.node_ref.id.clone(), node.node_ref.version.clone())) {
                n += count(&node.node_ref.id, &node.node_ref.version);
            }
        }
    }
    n
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Warns when the folder layout disagrees with the declared identity.
fn identity_path_finding(bundle: &Bundle, root: &Path) -> Option<Finding> {
    let rel = rel_path(root, &bundle.dir);
    let parts: Vec<&str> = rel.split('/').collect();
    let id = &bundle.header.id;
    let version = &bundle.header.version;
    let matches = if bundle.is_published() {
        parts.len() >= 2 && parts[parts.len() - 2] == id && parts[parts.len() - 1] == version
    } else {
        parts.last().is_some_and(|last| last == id)
    };
    if matches || id.is_empty() {
        return None;
    }
    Some(Finding::build(
        Severity::Warning,
        "path_identity_mismatch",
        &BundleRef::new(id.clone(), Some(version.clone()).filter(|v| !v.is_empty())),
        Subject::bundle(id.clone()),
        format!("The folder location does not match the declared identity {id}@{version}; identity comes from the frontmatter, so it still resolves."),
        "No action needed, or move the folder to its conventional location.",
    ))
}

/// Scans every bundle under `root`, reporting progress. Bundle files are
/// only read, never modified (SC-013).
pub fn scan_all(root: &Path, progress: &mut dyn FnMut(ScanEvent)) -> Vec<ScanEntry> {
    let dirs = discover_bundle_dirs(root);
    let total = dirs.len() as u64;
    let mut entries = Vec::with_capacity(dirs.len());
    let mut invalid = 0u64;
    for (i, dir) in dirs.iter().enumerate() {
        let entry = scan_bundle(root, dir);
        if entry.status == EntryStatus::Invalid {
            invalid += 1;
        }
        entries.push(entry);
        progress(ScanEvent::Progress {
            scanned: i as u64 + 1,
            total: Some(total),
            invalid,
        });
    }
    progress(ScanEvent::Complete {
        indexed: entries.len() as u64,
        invalid,
    });
    entries
}

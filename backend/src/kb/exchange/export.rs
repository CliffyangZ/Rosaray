//! `.algobundle` exporter (FR-039, FR-047, research §15–16). Export is always
//! two steps: build a reviewable manifest (`plan_export`), then write exactly
//! that manifest (`write_archive`); the caller passes back the manifest's hash
//! so nothing can change between review and write. Patient-data rules cannot
//! be switched off from here.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::patient_guard::{check_file, finding_for, known_content, parse_reviews, AssetReviewFile, Verdict, REVIEW_FILE};
use crate::designer::validate::finding::{Finding, Severity, Subject};
use crate::kb::bundle::model::{parse_yaml, to_yaml, EvidenceRecord, Kind};
use crate::kb::bundle::read::{read_bundle, Bundle, LOCK_FILE};
use crate::kb::bundle::service::{published_index, read_files, KbServiceError, PublishedRef};
use crate::kb::evidence::chain::read_chain;
use crate::kb::identity::canonical_json;

pub const MANIFEST_FILE: &str = "manifest.yaml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Path inside the archive.
    pub path: String,
    pub blake3: String,
    /// `primary | dependency | evidence | amendment | reviewed_test_asset`
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncludedBundle {
    pub kind: Kind,
    pub id: String,
    pub version: String,
    /// `primary | dependency`
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExcludedFile {
    pub bundle: String,
    pub path: String,
    /// `patient_status_unresolved | not_distributable | execution_asset`
    pub code: String,
}

/// What the destination must supply to actually run a node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionDep {
    pub bundle: String,
    pub implementation_id: String,
    pub implementation_version: String,
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    #[serde(default)]
    pub schema: String,
    pub entries: Vec<ManifestEntry>,
    pub included: Vec<IncludedBundle>,
    pub excluded: Vec<ExcludedFile>,
    pub omitted_execution_deps: Vec<ExecutionDep>,
}

impl Manifest {
    /// BLAKE3 over the canonical manifest, so review and write agree.
    pub fn hash(&self) -> String {
        let value = serde_json::to_value(self).expect("manifest serializes");
        format!("b3:{}", blake3::hash(&canonical_json(&value)).to_hex())
    }
}

pub struct ExportPlan {
    pub manifest: Manifest,
    pub manifest_hash: String,
    pub findings: Vec<Finding>,
    /// A patient-data rule fired that aborts the export.
    pub blocked: bool,
    /// `(archive path, bytes)` for every manifest entry.
    files: Vec<(String, Vec<u8>)>,
}

pub fn bundle_archive_dir(kind: Kind, id: &str, version: &str) -> String {
    format!("bundles/{}/{id}/{version}", kind.as_str())
}

/// `amendments/<id>@<version>/` — one append-only chain per published version.
pub fn amendment_chain_name(id: &str, version: &str) -> String {
    format!("{id}@{version}")
}

pub fn plan_export(
    conn: &Connection,
    root: &Path,
    kind: Kind,
    id: &str,
    version: &str,
) -> Result<ExportPlan, KbServiceError> {
    let index = published_index(root);
    let primary = index
        .iter()
        .find(|p| p.kind == kind && p.id == id && p.version == version)
        .ok_or(KbServiceError::NotFound)?
        .clone();

    let mut findings = Vec::new();
    let mut to_include: Vec<(PublishedRef, &'static str)> = vec![(primary.clone(), "primary")];
    let (bundle, read_findings) = read_bundle(&primary.dir);
    let bundle = bundle.ok_or_else(|| KbServiceError::Invalid(read_findings.clone()))?;
    if read_findings.iter().any(|f| f.is_error()) {
        return Err(KbServiceError::Invalid(read_findings));
    }
    if let Some(graph) = &bundle.graph {
        let mut seen = std::collections::BTreeSet::new();
        for n in &graph.nodes {
            let r = &n.node_ref;
            if !seen.insert((r.id.clone(), r.version.clone())) {
                continue;
            }
            match index.iter().find(|p| p.id == r.id && p.version == r.version) {
                Some(dep) if r.content_id.as_ref().is_none_or(|c| *c == dep.content_id) => {
                    to_include.push((dep.clone(), "dependency"))
                }
                Some(_) => findings.push(Finding::build(
                    Severity::Error,
                    "dependency_content_mismatch",
                    &bundle.bundle_ref(),
                    Subject::new(crate::designer::validate::finding::SubjectType::NodeInstance, n.instance_id.clone()),
                    format!("{}@{} exists but its content differs from the one this pipe was built against.", r.id, r.version),
                    "Restore the original version before exporting.",
                )),
                None => findings.push(Finding::build(
                    Severity::Error,
                    "dependency_unresolved",
                    &bundle.bundle_ref(),
                    Subject::new(crate::designer::validate::finding::SubjectType::NodeInstance, n.instance_id.clone()),
                    format!("{}@{} is not in the knowledge base, so the pipe cannot be exported completely.", r.id, r.version),
                    "Import or restore that version, then export again.",
                )),
            }
        }
    }
    if findings.iter().any(|f| f.is_error()) {
        return Err(KbServiceError::Invalid(findings));
    }

    let known = known_content(conn)?;
    let mut manifest = Manifest { schema: "quantify-kb/1".into(), ..Default::default() };
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut blocked = false;

    for (pref, reason) in &to_include {
        let (b, rf) = read_bundle(&pref.dir);
        let b = b.ok_or_else(|| KbServiceError::Invalid(rf.clone()))?;
        if rf.iter().any(|f| f.is_error()) {
            return Err(KbServiceError::Invalid(rf));
        }
        manifest.included.push(IncludedBundle {
            kind: pref.kind,
            id: pref.id.clone(),
            version: pref.version.clone(),
            reason: reason.to_string(),
        });
        let label = format!("{}@{}", pref.id, pref.version);
        let bref = b.bundle_ref();
        let dir = bundle_archive_dir(pref.kind, &pref.id, &pref.version);
        let reviews = read_reviews(&pref.dir);
        let exec_assets: Vec<String> = b.implementation.as_ref().map(|i| i.assets.iter().map(|a| a.path.clone()).collect()).unwrap_or_default();
        let non_distributable = non_distributable_files(&b);

        for (rel, bytes) in read_files(&pref.dir)? {
            if exec_assets.contains(&rel) {
                manifest.excluded.push(ExcludedFile { bundle: label.clone(), path: rel, code: "execution_asset".into() });
                continue;
            }
            if non_distributable.contains(&rel) {
                manifest.excluded.push(ExcludedFile { bundle: label.clone(), path: rel, code: "not_distributable".into() });
                continue;
            }
            let verdict = check_file(&rel, &bytes, &known, &reviews);
            if let Some(f) = finding_for(&bref, &rel, &verdict) {
                findings.push(f);
            }
            match verdict {
                Verdict::Blocked(_) => blocked = true,
                Verdict::Excluded(_) => manifest.excluded.push(ExcludedFile {
                    bundle: label.clone(),
                    path: rel,
                    code: super::patient_guard::CODE_UNRESOLVED.into(),
                }),
                Verdict::Allowed => {
                    let entry_reason = entry_reason(&rel, reason);
                    let archive_path = format!("{dir}/{rel}");
                    manifest.entries.push(ManifestEntry {
                        path: archive_path.clone(),
                        blake3: format!("b3:{}", blake3::hash(&bytes).to_hex()),
                        reason: entry_reason,
                    });
                    files.insert(archive_path, bytes);
                }
            }
        }

        if let Some(imp) = &b.implementation {
            manifest.omitted_execution_deps.push(ExecutionDep {
                bundle: label.clone(),
                implementation_id: imp.implementation_id.clone(),
                implementation_version: imp.implementation_version.clone(),
                assets: exec_assets,
            });
        }

        // Amendment chain for this exact version.
        let chain_name = amendment_chain_name(&pref.id, &pref.version);
        let chain_dir = root.join("amendments").join(&chain_name);
        if chain_dir.is_dir() {
            let read = read_chain(&chain_dir)?;
            if let Some(brk) = read.broken {
                return Err(KbServiceError::Invalid(vec![brk.to_finding(&bref)]));
            }
            for (rel, bytes) in read_files(&chain_dir)? {
                let archive_path = format!("amendments/{chain_name}/{rel}");
                manifest.entries.push(ManifestEntry {
                    path: archive_path.clone(),
                    blake3: format!("b3:{}", blake3::hash(&bytes).to_hex()),
                    reason: "amendment".into(),
                });
                files.insert(archive_path, bytes);
            }
        }
    }

    manifest.entries.sort_by(|a, b| a.path.cmp(&b.path));
    manifest.excluded.sort_by(|a, b| (&a.bundle, &a.path).cmp(&(&b.bundle, &b.path)));
    manifest.included.sort_by(|a, b| (&a.id, &a.version).cmp(&(&b.id, &b.version)));
    manifest.omitted_execution_deps.sort_by(|a, b| a.bundle.cmp(&b.bundle));
    let manifest_hash = manifest.hash();
    Ok(ExportPlan { manifest, manifest_hash, findings, blocked, files: files.into_iter().collect() })
}

fn read_reviews(dir: &Path) -> AssetReviewFile {
    std::fs::read_to_string(dir.join(REVIEW_FILE)).map(|t| parse_reviews(&t)).unwrap_or_default()
}

/// Evidence files whose record says `distributable: false`.
fn non_distributable_files(b: &Bundle) -> Vec<String> {
    let mut out = Vec::new();
    for rel in b.files.iter().filter(|f| f.starts_with("references/")) {
        if let Ok(text) = std::fs::read_to_string(b.dir.join(rel)) {
            if let Ok(rec) = parse_yaml::<EvidenceRecord>(&text) {
                if !rec.distributable {
                    out.push(rel.clone());
                }
            }
        }
    }
    out
}

fn entry_reason(rel: &str, bundle_reason: &str) -> String {
    if rel.starts_with("references/") {
        "evidence".into()
    } else if rel.starts_with("tests/") || rel.starts_with("assets/") {
        if rel.ends_with(".yaml") || rel.ends_with(".yml") || rel.ends_with(".json") || rel.ends_with(".md") || rel.ends_with(".txt") {
            bundle_reason.into()
        } else {
            "reviewed_test_asset".into()
        }
    } else {
        bundle_reason.into()
    }
}

impl ExportPlan {
    /// Writes the reviewed manifest and files as a ZIP. Refuses if a rule
    /// blocked the export or if `expected_hash` is not this plan's hash.
    pub fn write_archive(&self, expected_hash: &str) -> Result<Vec<u8>, ExportRefused> {
        if self.blocked {
            return Err(ExportRefused::PatientDataBlocked);
        }
        if self.manifest_hash != expected_hash {
            return Err(ExportRefused::ManifestChanged);
        }
        Ok(pack_archive(&self.manifest, &self.files))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportRefused {
    PatientDataBlocked,
    ManifestChanged,
}

/// Packs `manifest.yaml` plus `files` into a deterministic ZIP. Public so
/// tests can assemble hostile archives; export itself only calls it after the
/// guard has passed.
pub fn pack_archive(manifest: &Manifest, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        let manifest_yaml = to_yaml(manifest).expect("manifest serializes");
        zip.start_file(MANIFEST_FILE, opts).expect("zip entry");
        zip.write_all(manifest_yaml.as_bytes()).expect("zip write");
        for (path, bytes) in files {
            zip.start_file(path.as_str(), opts).expect("zip entry");
            zip.write_all(bytes).expect("zip write");
        }
        zip.finish().expect("zip finish");
    }
    buf.into_inner()
}

/// The lock file is part of every exported bundle; expose its name for tests.
pub const BUNDLE_LOCK: &str = LOCK_FILE;

//! Bundle reader: loads a directory into a `Bundle` plus findings, never
//! panicking on malformed input (FR-040 — an invalid bundle is reported, not
//! skipped). Published bundles are verified against `bundle.lock`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::frontmatter::{parse_header, Header};
use super::model::{
    parse_yaml, BundleLock, Contract, EvidenceRecord, GraphFile, Implementation, Kind,
};
use crate::contract::finding::{BundleRef, Finding, Severity, Subject};
use crate::identity::{content_id_of, file_hashes};

pub const LOCK_FILE: &str = "bundle.lock";

#[derive(Debug, Clone)]
pub struct Bundle {
    pub dir: PathBuf,
    pub kind: Kind,
    pub header: Header,
    pub body: String,
    pub contract: Option<Contract>,
    pub implementation: Option<Implementation>,
    pub graph: Option<GraphFile>,
    pub lock: Option<BundleLock>,
    pub evidence: Vec<EvidenceRecord>,
    /// Every file under the bundle, relative `/`-paths, sorted.
    pub files: Vec<String>,
}

impl Bundle {
    /// A bundle with a `bundle.lock` is published (immutable).
    pub fn is_published(&self) -> bool {
        self.lock.is_some()
    }

    pub fn bundle_ref(&self) -> BundleRef {
        BundleRef::new(self.header.id.clone(), non_empty(&self.header.version))
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

struct Reporter {
    bundle: BundleRef,
    findings: Vec<Finding>,
}

impl Reporter {
    fn err(&mut self, code: &str, file: &str, explanation: String, action: &str) {
        self.push(Severity::Error, code, file, explanation, action);
    }
    fn push(&mut self, severity: Severity, code: &str, file: &str, explanation: String, action: &str) {
        self.findings.push(Finding::build(
            severity,
            code,
            &self.bundle,
            Subject::file(file),
            explanation,
            action,
        ));
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Reads an optional typed YAML file. `Ok(None)` when the file is absent.
fn read_yaml<T: for<'de> Deserialize<'de>>(
    dir: &Path,
    name: &str,
    r: &mut Reporter,
) -> Option<T> {
    let path = dir.join(name);
    if !path.is_file() {
        return None;
    }
    match read_text(&path).and_then(|t| parse_yaml::<T>(&t)) {
        Ok(v) => Some(v),
        Err(e) => {
            r.err(
                "bundle_file_invalid",
                name,
                format!("{name} could not be read as valid YAML for this schema: {e}"),
                "Fix the syntax or the offending field in this file and refresh the catalog.",
            );
            None
        }
    }
}

pub fn read_bundle(dir: &Path) -> (Option<Bundle>, Vec<Finding>) {
    let fallback_id = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut r = Reporter {
        bundle: BundleRef::new(fallback_id, None),
        findings: Vec::new(),
    };

    let (kind, main_file) = if dir.join("ALGONODE.md").is_file() {
        (Kind::Algonode, "ALGONODE.md")
    } else if dir.join("ALGOPIPE.md").is_file() {
        (Kind::Algopipe, "ALGOPIPE.md")
    } else {
        r.err(
            "bundle_file_missing",
            "ALGONODE.md",
            "The folder has neither ALGONODE.md nor ALGOPIPE.md, so it cannot be identified as a knowledge bundle.".into(),
            "Add the bundle's description file with a valid frontmatter header, or remove the folder from the knowledge base.",
        );
        return (None, r.findings);
    };

    let text = match read_text(&dir.join(main_file)) {
        Ok(t) => t,
        Err(e) => {
            r.err(
                "bundle_unreadable",
                main_file,
                format!("{main_file} could not be read: {e}"),
                "Check the file's encoding (UTF-8) and permissions.",
            );
            return (None, r.findings);
        }
    };
    let (header, body) = match parse_header(&text) {
        Ok(v) => v,
        Err(e) => {
            r.err(
                e.code,
                main_file,
                format!("{main_file}: {}", e.message),
                "Correct the frontmatter header at the top of the file.",
            );
            return (None, r.findings);
        }
    };
    r.bundle = BundleRef::new(
        if header.id.is_empty() { r.bundle.id.clone() } else { header.id.clone() },
        non_empty(&header.version),
    );

    if header.kind.is_some_and(|k| k != kind) {
        r.err(
            "frontmatter_contract_mismatch",
            main_file,
            format!("{main_file} declares kind \"{}\" but the file is a {} description.", header.kind.map(|k| k.as_str()).unwrap_or(""), kind.as_str()),
            "Make the frontmatter `kind` match the bundle type.",
        );
    }

    let mut contract = None;
    let mut graph = None;
    match kind {
        Kind::Algonode => {
            contract = read_yaml::<Contract>(dir, "contract.yaml", &mut r);
            if contract.is_none() && !dir.join("contract.yaml").is_file() {
                r.err(
                    "bundle_file_missing",
                    "contract.yaml",
                    "The AlgoNode has no contract.yaml, so its ports and parameters are unknown.".into(),
                    "Add contract.yaml describing inputs, outputs, parameters and reproducibility.",
                );
            }
            if let Some(c) = &contract {
                cross_check_contract(&header, c, &mut r);
            }
        }
        Kind::Algopipe => {
            graph = read_yaml::<GraphFile>(dir, "graph.yaml", &mut r);
            if graph.is_none() && !dir.join("graph.yaml").is_file() {
                r.err(
                    "bundle_file_missing",
                    "graph.yaml",
                    "The AlgoPipe has no graph.yaml, so its nodes and connections are unknown.".into(),
                    "Add graph.yaml with nodes and edges.",
                );
            }
            if let Some(g) = &graph {
                if !g.schema.is_empty() && g.schema != header.schema {
                    r.err(
                        "frontmatter_contract_mismatch",
                        "graph.yaml",
                        format!("graph.yaml schema \"{}\" differs from the frontmatter schema \"{}\".", g.schema, header.schema),
                        "Make both files declare the same schema.",
                    );
                }
            }
        }
    }
    let implementation = read_yaml::<Implementation>(dir, "implementation.yaml", &mut r);

    let mut evidence = Vec::new();
    let refs = dir.join("references");
    if refs.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(&refs)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        names.sort();
        for p in names {
            if p.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                let rel = format!("references/{}", p.file_name().unwrap().to_string_lossy());
                match read_text(&p).and_then(|t| parse_yaml::<EvidenceRecord>(&t)) {
                    Ok(rec) => evidence.push(rec),
                    Err(e) => r.err(
                        "bundle_file_invalid",
                        &rel,
                        format!("{rel} is not a valid evidence record: {e}"),
                        "Fix the record's fields or remove the file.",
                    ),
                }
            }
        }
    }

    let hashes = match file_hashes(dir) {
        Ok(h) => h,
        Err(e) => {
            r.err(
                "bundle_unreadable",
                ".",
                format!("The bundle's files could not be listed: {e}"),
                "Check folder permissions.",
            );
            Vec::new()
        }
    };
    let files: Vec<String> = hashes.iter().map(|(p, _)| p.clone()).collect();

    let lock = if dir.join(LOCK_FILE).is_file() {
        let lock = read_yaml::<BundleLock>(dir, LOCK_FILE, &mut r);
        match &lock {
            Some(l) => verify_lock(l, &header, &hashes, &mut r),
            None => r.err(
                "published_bundle_modified",
                LOCK_FILE,
                "bundle.lock exists but is unreadable, so this published version cannot be verified.".into(),
                "Restore the original bundle.lock from a copy or re-import the bundle.",
            ),
        }
        lock
    } else {
        None
    };

    let bundle = Bundle {
        dir: dir.to_path_buf(),
        kind,
        header,
        body,
        contract,
        implementation,
        graph,
        lock,
        evidence,
        files,
    };
    (Some(bundle), r.findings)
}

fn cross_check_contract(header: &Header, c: &Contract, r: &mut Reporter) {
    let mut mismatches = Vec::new();
    if !c.schema.is_empty() && c.schema != header.schema {
        mismatches.push(format!("schema \"{}\" vs \"{}\"", c.schema, header.schema));
    }
    if c.node_type.as_ref().is_some_and(|t| *t != header.id) {
        mismatches.push("node_type vs id".to_string());
    }
    if c.definition_version.as_ref().is_some_and(|v| *v != header.version) {
        mismatches.push("definition_version vs version".to_string());
    }
    if !mismatches.is_empty() {
        r.err(
            "frontmatter_contract_mismatch",
            "contract.yaml",
            format!("contract.yaml disagrees with the frontmatter: {}.", mismatches.join(", ")),
            "Make the shared identity fields identical in both files.",
        );
    }
}

fn verify_lock(lock: &BundleLock, header: &Header, hashes: &[(String, String)], r: &mut Reporter) {
    let mut problems: Vec<String> = Vec::new();
    if lock.id != header.id || lock.version != header.version {
        problems.push("identity differs from the frontmatter".to_string());
    }
    let on_disk: std::collections::BTreeMap<&str, &str> = hashes
        .iter()
        .filter(|(p, _)| p != LOCK_FILE)
        .map(|(p, h)| (p.as_str(), h.as_str()))
        .collect();
    for f in &lock.files {
        let want = f.blake3.strip_prefix("b3:").unwrap_or(&f.blake3);
        match on_disk.get(f.path.as_str()) {
            None => problems.push(format!("{} is missing", f.path)),
            Some(h) if h.strip_prefix("b3:").unwrap_or(h) != want => {
                problems.push(format!("{} was modified", f.path))
            }
            Some(_) => {}
        }
    }
    let locked: std::collections::BTreeSet<&str> = lock.files.iter().map(|f| f.path.as_str()).collect();
    for path in on_disk.keys() {
        if !locked.contains(path) {
            problems.push(format!("{path} was added"));
        }
    }
    if problems.is_empty() {
        let actual = content_id_of(hashes.iter().map(|(p, h)| (p.as_str(), h.as_str())));
        if actual != lock.content_id {
            problems.push("content identity differs from bundle.lock".to_string());
        }
    }
    if !problems.is_empty() {
        r.err(
            "published_bundle_modified",
            LOCK_FILE,
            format!("This published version no longer matches its bundle.lock: {}.", problems.join("; ")),
            "Restore the original files, or re-import the version; a published version is never edited in place.",
        );
    }
}

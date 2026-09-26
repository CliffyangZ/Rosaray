#![allow(dead_code)]
//! Shared helpers: a temp QKB (KB root + database) and bundle builders.

use std::path::{Path, PathBuf};

use rosaray_qkb::submit::{submit, Accepted, SubmitError, Submission};
use rusqlite::Connection;
use serde_json::{json, Value};

pub struct Kb {
    pub dir: tempfile::TempDir,
    pub root: PathBuf,
    pub conn: Connection,
}

impl Kb {
    pub fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("knowledge-base");
        rosaray_qkb::ensure_layout(&root).unwrap();
        let conn = rosaray_qkb::store::open(&dir.path().join("qkb.sqlite")).unwrap();
        Self { dir, root, conn }
    }

    /// The six built-in nodes, submitted and verified.
    pub fn seeded() -> Self {
        let kb = Self::empty();
        assert_eq!(rosaray_qkb::seed::install(&kb.conn, &kb.root).unwrap(), 6);
        kb
    }

    pub fn submit(&self, files: Vec<(String, Vec<u8>)>) -> Result<Accepted, SubmitError> {
        submit(&self.conn, &self.root, Submission { files })
    }

    pub fn submit_pipe(&self, id: &str, version: &str, graph: Value) -> Result<Accepted, SubmitError> {
        self.submit(pipe_files(id, version, graph))
    }

    pub fn purge(&self, trigger: &str) -> Vec<rosaray_qkb::cleanup::DeletionRecord> {
        rosaray_qkb::cleanup::recompute_and_purge(&self.conn, &self.root, trigger).unwrap()
    }

    pub fn entries(&self) -> Vec<rosaray_qkb::catalog::query::CatalogEntry> {
        rosaray_qkb::catalog::repo::refresh(&self.conn, &self.root, &mut |_| {}).unwrap();
        let f = rosaray_qkb::catalog::query::EntryFilter { status: Some("published".into()), limit: Some(500), ..Default::default() };
        rosaray_qkb::catalog::query::query_entries(&self.conn, &f).unwrap().entries
    }

    pub fn has_version(&self, kind: &str, id: &str, version: &str) -> bool {
        let sub = if kind == "algopipe" { "pipes" } else { "nodes" };
        self.root.join(sub).join(id).join(version).is_dir()
    }
}

pub fn node_ref(inst: &str, id: &str, version: &str, params: Value) -> Value {
    json!({ "instance_id": inst, "ref": { "id": id, "version": version }, "parameters": params })
}

pub fn demo_graph(threshold: Value) -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node_ref("src", "rosaray.image-source", "1.0.0", json!({})),
            node_ref("norm", "rosaray.normalize", "1.0.0", json!({})),
            node_ref("th", "rosaray.threshold", "1.0.0", threshold),
        ],
        "edges": [
            { "from": "src.image", "to": "norm.image" },
            { "from": "norm.image", "to": "th.image" },
        ],
        "target_data_profile": { "profile_version": 1, "require": [{ "predicate": "modality", "equals": "intraoral-photo" }] },
    })
}

/// Full area pipe: needs pixel spacing (node prerequisite), so applicability
/// depends on the query's data conditions.
pub fn area_graph() -> Value {
    json!({
        "schema": "quantify-kb/1",
        "nodes": [
            node_ref("src", "rosaray.image-source", "1.0.0", json!({})),
            node_ref("norm", "rosaray.normalize", "1.0.0", json!({})),
            node_ref("th", "rosaray.threshold", "1.0.0", json!({ "mode": "otsu" })),
            node_ref("a", "rosaray.area", "1.0.0", json!({})),
        ],
        "edges": [
            { "from": "src.image", "to": "norm.image" },
            { "from": "norm.image", "to": "th.image" },
            { "from": "th.mask", "to": "a.mask" },
        ],
        "target_data_profile": { "profile_version": 1, "require": [
            { "predicate": "modality", "equals": "intraoral-photo" },
            { "predicate": "pixel_spacing_present", "equals": true },
        ] },
    })
}

pub fn pipe_files(id: &str, version: &str, graph: Value) -> Vec<(String, Vec<u8>)> {
    pipe_files_described(id, version, "Threshold demo", graph)
}

pub fn pipe_files_described(id: &str, version: &str, summary: &str, graph: Value) -> Vec<(String, Vec<u8>)> {
    let md = format!(
        "---\nschema: quantify-kb/1\nkind: algopipe\nid: {id}\nversion: {version}\nname: Demo {id}\nsummary: {summary}\nstatus: draft\nresearch_use_only: true\nintended_use: {summary}\nlimitations: Demo only\n---\nBody\n"
    );
    vec![
        ("ALGOPIPE.md".into(), md.into_bytes()),
        ("graph.yaml".into(), serde_yaml_ng::to_string(&graph).unwrap().into_bytes()),
    ]
}

pub fn hash_tree(dir: &Path) -> Vec<(String, String)> {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, String)>) {
        let Ok(rd) = std::fs::read_dir(cur) else { return };
        let mut children: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        children.sort();
        for p in children {
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                out.push((rel, blake3::hash(&std::fs::read(&p).unwrap()).to_hex().to_string()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// Writes a *published-shaped* bundle the way older QKB versions could have
/// left one (e.g. a knowledge-only pipe), bypassing submission.
pub fn write_legacy_published(root: &Path, kind: &str, id: &str, version: &str, files: Vec<(String, Vec<u8>)>, release_kind: &str) {
    use rosaray_qkb::bundle::model::{to_yaml, BundleLock, LockFile};
    use rosaray_qkb::bundle::write::write_bundle_atomic;
    let hashed: Vec<(String, String)> = files.iter().map(|(p, b)| (p.clone(), format!("b3:{}", blake3::hash(b).to_hex()))).collect();
    let content_id = rosaray_qkb::identity::content_id_of(hashed.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let lock = BundleLock {
        schema: "quantify-kb/1".into(),
        id: id.into(),
        version: version.into(),
        content_id,
        computational_identity: None,
        release_kind: Some(release_kind.into()),
        disclosures: vec![],
        dependency_summary: None,
        verification_summary: None,
        files: hashed.iter().map(|(p, h)| LockFile { path: p.clone(), blake3: h.trim_start_matches("b3:").to_string() }).collect(),
    };
    let mut all = files;
    all.push(("bundle.lock".into(), to_yaml(&lock).unwrap().into_bytes()));
    let sub = if kind == "algopipe" { "pipes" } else { "nodes" };
    write_bundle_atomic(&root.join(sub).join(id).join(version), &all).unwrap();
}

pub fn write_draft(root: &Path, kind: &str, id: &str, files: Vec<(String, Vec<u8>)>) {
    let sub = if kind == "algopipe" { "pipes" } else { "nodes" };
    let dir = root.join("drafts").join(sub).join(id);
    std::fs::create_dir_all(&dir).unwrap();
    for (rel, bytes) in files {
        std::fs::write(dir.join(rel), bytes).unwrap();
    }
}


pub const SO: &str = "so-local-1";

pub fn query_body(request_id: &str, purpose: &str, conditions: Value) -> Value {
    json!({ "protocol_version": "system-one/1", "request_id": request_id, "task_purpose": purpose, "data_conditions": conditions })
}

pub fn select_body(request_id: &str, id: &str, version: &str, content_id: &str, reason: &str) -> Value {
    json!({ "protocol_version": "system-one/1", "request_id": request_id, "decision": "selected",
            "selection": { "id": id, "version": version, "content_id": content_id }, "reason": reason })
}

pub fn abstain_body(request_id: &str, reason: &str) -> Value {
    json!({ "protocol_version": "system-one/1", "request_id": request_id, "decision": "abstained", "reason": reason })
}

pub fn uuid(n: u32) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

/// Seeded KB with a threshold pipe and an area pipe (the latter needs pixel spacing).
pub struct Two {
    pub kb: Kb,
    pub demo: rosaray_qkb::submit::Accepted,
    pub area: rosaray_qkb::submit::Accepted,
}

pub fn two_pipes() -> Two {
    let kb = Kb::seeded();
    let demo = kb.submit(pipe_files_described("acme.demo", "1.0.0", "Threshold segmentation of intraoral photographs", demo_graph(json!({ "mode": "otsu" })))).unwrap();
    let area = kb.submit(pipe_files_described("acme.area", "1.0.0", "Threshold segmentation with area measurement", area_graph())).unwrap();
    Two { kb, demo, area }
}

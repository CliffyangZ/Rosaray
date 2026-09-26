//! Preview orchestration for AlgoPipe drafts (US4, research §11): incremental,
//! single-image, in-memory. It runs a target node's ancestor cone through the
//! Runtime Adapter, reuses cached node results whose inputs did not change, and
//! labels every outcome `verified` or `unverified` — unverified being sticky
//! along the cone and downstream. It never creates a Run record and never
//! persists anything but a viewable blob: this module has no reference to the
//! run repository (Constitution III, SC-009).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rusqlite::Connection;
use serde::Serialize;

use crate::designer::runtime_adapter::pipeline::execute_node;
use crate::designer::runtime_adapter::{effective_parameters, Artifact, ExecContext, ImageF32, NodeError, Outputs, Registry};
use crate::domain::pipeline_snapshot::{compute_equivalence_key, GraphEdge, GraphNode, PipelineGraph, PipelineSnapshotError};
use crate::kb::bundle::model::{Contract, GraphFile, NodeRef};
use crate::kb::bundle::read::read_bundle;
use crate::kb::catalog::query::path_of;
use crate::kb::catalog::status::{derive, SPECIFICATION_ONLY};
use crate::kb::identity::split_endpoint;

/// One node instance, resolved to what Preview needs to know about it.
#[derive(Debug, Clone)]
pub struct PreviewNode {
    pub instance_id: String,
    pub contract: Contract,
    pub implementation: Option<(String, String)>,
    /// Identifies the exact definition content (`content_id`, or a hash of a
    /// draft's files) so the cache key changes whenever the definition does.
    pub definition_identity: String,
    /// Has a resolvable, trusted implementation.
    pub previewable: bool,
    /// Currently `technically_verified` for this exact definition + implementation.
    pub verified: bool,
}

/// Resolves a node reference against the catalog (published version, or the
/// draft when the reference says `draft`).
pub fn resolve_node(conn: &Connection, root: &std::path::Path, instance_id: &str, r: &NodeRef) -> Option<PreviewNode> {
    let is_version = semver::Version::parse(&r.version).is_ok();
    let (status, version) = if is_version { ("published", r.version.as_str()) } else { ("draft", "") };
    let path = if is_version {
        path_of(conn, "algonode", &r.id, version, status).ok().flatten()
    } else {
        conn.query_row(
            "SELECT path FROM kb_catalog_entry WHERE kind = 'algonode' AND id = ?1 AND status = 'draft' LIMIT 1",
            [&r.id],
            |row| row.get::<_, String>(0),
        )
        .ok()
    }?;
    let dir = root.join(path);
    let (bundle, findings) = read_bundle(&dir);
    let bundle = bundle?;
    let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
    let d = derive(root, &bundle, modified);
    let contract = bundle.contract.clone()?;
    let definition_identity = match &bundle.lock {
        Some(l) => l.content_id.clone(),
        None => format!("draft:{}", crate::kb::identity::content_id(&dir).unwrap_or_default()),
    };
    Some(PreviewNode {
        instance_id: instance_id.to_string(),
        contract,
        implementation: bundle.implementation.as_ref().map(|i| (i.implementation_id.clone(), i.implementation_version.clone())),
        definition_identity,
        previewable: d.maturity.is_some_and(|m| m != SPECIFICATION_ONLY) && d.availability != "unavailable",
        verified: d.verification.is_valid(),
    })
}

/// Lossless node results, in memory only (Preview artifacts are never
/// persisted, FR-011). Keyed by the extended equivalence key.
#[derive(Default)]
pub struct NodeCache {
    entries: Mutex<HashMap<String, Arc<Outputs>>>,
}

impl NodeCache {
    pub fn get(&self, key: &str) -> Option<Arc<Outputs>> {
        self.entries.lock().unwrap().get(key).cloned()
    }
    pub fn insert(&self, key: String, outputs: Arc<Outputs>) {
        self.entries.lock().unwrap().insert(key, outputs);
    }
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Per-pipe, per-node Preview status (FR-016): `idle | stale | queued | running
/// | ready | failed | cancelled`.
#[derive(Default)]
pub struct PreviewBoard {
    pipes: Mutex<HashMap<String, BoardEntry>>,
}

#[derive(Default, Clone)]
struct BoardEntry {
    revision: String,
    nodes: HashMap<String, &'static str>,
}

impl PreviewBoard {
    pub fn set(&self, pipe_id: &str, revision: &str, node: &str, state: &'static str) {
        let mut pipes = self.pipes.lock().unwrap();
        let e = pipes.entry(pipe_id.to_string()).or_default();
        e.revision = revision.to_string();
        e.nodes.insert(node.to_string(), state);
    }

    /// A computational edit: these nodes' earlier results are out of date.
    pub fn mark_stale(&self, pipe_id: &str, nodes: &[String]) {
        let mut pipes = self.pipes.lock().unwrap();
        let e = pipes.entry(pipe_id.to_string()).or_default();
        for n in nodes {
            e.nodes.insert(n.clone(), "stale");
        }
    }

    /// Current status of every node of the pipe: `idle` until something ran,
    /// `stale` once a computational edit reached it. Only the edited node and
    /// its downstream cone go stale; upstream results stay `ready` (FR-018).
    pub fn statuses(&self, pipe_id: &str, all_nodes: &[String]) -> BTreeMap<String, &'static str> {
        let pipes = self.pipes.lock().unwrap();
        let entry = pipes.get(pipe_id).cloned().unwrap_or_default();
        all_nodes.iter().map(|n| (n.clone(), entry.nodes.get(n).copied().unwrap_or("idle"))).collect()
    }
}

#[derive(Debug, Clone)]
pub enum PreviewFailure {
    /// A node in the cone is specification-only or otherwise cannot run.
    NotPreviewable { nodes: Vec<String> },
    UnknownTarget(String),
    Graph(String),
    Node { failing_node_id: String, error: NodeError },
}

#[derive(Debug, Clone, Serialize)]
pub struct PipePreviewOutcome {
    #[serde(skip)]
    pub artifact: Artifact,
    pub node_reuse: BTreeMap<String, bool>,
    pub reused: bool,
    pub verification_state: &'static str,
    pub unverified_nodes: Vec<String>,
    pub computed_nodes: Vec<String>,
    pub pipeline_snapshot_id: uuid::Uuid,
    pub duration_ms: u64,
    pub port: String,
}

pub struct PipePreviewRequest<'a> {
    pub graph: &'a GraphFile,
    pub nodes: &'a BTreeMap<String, PreviewNode>,
    pub target: &'a str,
    pub port: Option<&'a str>,
    pub source: Arc<ImageF32>,
    pub source_identity: &'a str,
    pub pixel_spacing_mm: Option<(f64, f64)>,
    pub pipe_id: &'a str,
    pub revision: &'a str,
}

fn to_pipeline_graph(graph: &GraphFile, nodes: &BTreeMap<String, PreviewNode>) -> PipelineGraph {
    let mut pg = PipelineGraph::default();
    for n in &graph.nodes {
        let Some(pn) = nodes.get(&n.instance_id) else { continue };
        let (imp_id, imp_ver) = pn.implementation.clone().unwrap_or_default();
        let params = effective_parameters(&pn.contract, n.parameters.as_ref());
        let deterministic = pn.contract.reproducibility.as_ref().map(|r| r.deterministic).unwrap_or(false);
        pg.nodes.push(GraphNode {
            node_id: n.instance_id.clone(),
            node_type: n.node_ref.id.clone(),
            // The cache key gains the implementation and definition identity.
            implementation_version: format!("{imp_id}/{imp_ver}/{}", pn.definition_identity),
            canonical_parameters: serde_json::Value::Object(params),
            reproducible: deterministic,
            seed: None,
        });
    }
    for e in &graph.edges {
        let (from, from_port) = split_endpoint(&e.from);
        let (to, to_port) = split_endpoint(&e.to);
        pg.edges.push(GraphEdge { from, to, from_port, to_port });
    }
    pg
}

/// Runs the target's ancestor cone. `board` receives per-node status changes.
pub fn run_pipe_preview(
    cache: &NodeCache,
    board: &PreviewBoard,
    registry: &Registry,
    req: PipePreviewRequest,
) -> Result<PipePreviewOutcome, PreviewFailure> {
    let pg = to_pipeline_graph(req.graph, req.nodes);
    let cone: Vec<String> = pg
        .ancestors_of(req.target)
        .map_err(|e| match e {
            PipelineSnapshotError::UnknownNode(id) => PreviewFailure::UnknownTarget(id),
            PipelineSnapshotError::Cycle => PreviewFailure::Graph("the pipe contains a cycle".into()),
            PipelineSnapshotError::MissingSeed(ids) => PreviewFailure::Graph(format!("nodes need a seed: {ids:?}")),
        })?
        .iter()
        .map(|n| n.node_id.clone())
        .collect();

    let blocked: Vec<String> = cone.iter().filter(|i| req.nodes.get(*i).is_none_or(|n| !n.previewable)).cloned().collect();
    if !blocked.is_empty() {
        return Err(PreviewFailure::NotPreviewable { nodes: blocked });
    }
    let unverified: Vec<String> = cone.iter().filter(|i| !req.nodes[*i].verified).cloned().collect();

    // Spacing changes results (e.g. area), and it is not part of the image
    // content identity, so it is folded into the key explicitly.
    let source_key = match req.pixel_spacing_mm {
        Some((x, y)) => format!("{}|spacing={x},{y}", req.source_identity),
        None => req.source_identity.to_string(),
    };

    for i in &cone {
        board.set(req.pipe_id, req.revision, i, "queued");
    }
    let start = Instant::now();
    let mut results: HashMap<String, Arc<Outputs>> = HashMap::new();
    let mut reuse = BTreeMap::new();
    let mut computed = Vec::new();
    for inst in &cone {
        let key = compute_equivalence_key(&pg, inst, &source_key).map_err(|e| PreviewFailure::Graph(e.to_string()))?;
        if let Some(hit) = cache.get(&key) {
            board.set(req.pipe_id, req.revision, inst, "ready");
            results.insert(inst.clone(), hit);
            reuse.insert(inst.clone(), true);
            continue;
        }
        board.set(req.pipe_id, req.revision, inst, "running");
        let node = &req.nodes[inst];
        let fail = |error: NodeError| {
            board.set(req.pipe_id, req.revision, inst, "failed");
            PreviewFailure::Node { failing_node_id: inst.clone(), error }
        };
        let (imp_id, _) = node.implementation.clone().ok_or_else(|| fail(NodeError::new("implementation_unavailable", "the node has no implementation")))?;
        let given = req.graph.nodes.iter().find(|n| &n.instance_id == inst).and_then(|n| n.parameters.as_ref());
        let ctx = ExecContext { source: Some(req.source.clone()), pixel_spacing_mm: req.pixel_spacing_mm, seed: None };
        // The same execution a formal Run uses; only the caching around it differs.
        let outputs = execute_node(req.graph, inst, &node.contract, &imp_id, given, &results, &ctx, registry).map_err(fail)?;
        let outputs = Arc::new(outputs);
        cache.insert(key, outputs.clone());
        results.insert(inst.clone(), outputs);
        board.set(req.pipe_id, req.revision, inst, "ready");
        reuse.insert(inst.clone(), false);
        computed.push(inst.clone());
    }

    let target_outputs = &results[req.target];
    let port = match req.port {
        Some(p) => p.to_string(),
        None => req.nodes[req.target].contract.outputs.first().map(|p| p.port_id.clone()).unwrap_or_default(),
    };
    let artifact = target_outputs
        .get(&port)
        .cloned()
        .ok_or_else(|| PreviewFailure::Node { failing_node_id: req.target.to_string(), error: NodeError::new("output_missing", format!("the node has no output \"{port}\"")) })?;
    Ok(PipePreviewOutcome {
        artifact,
        reused: reuse.get(req.target).copied().unwrap_or(false),
        node_reuse: reuse,
        verification_state: if unverified.is_empty() { "verified" } else { "unverified" },
        unverified_nodes: unverified,
        computed_nodes: computed,
        pipeline_snapshot_id: crate::domain::pipeline_snapshot::pipeline_snapshot_id(&crate::domain::pipeline_snapshot::compute_graph_identity(&pg)),
        duration_ms: start.elapsed().as_millis() as u64,
        port,
    })
}

//! Executing a *published* AlgoPipe for a formal Run. Unlike Preview it uses
//! only published, hash-pinned definitions and the implementations pinned at
//! publication, keeps no caches, and shares no state with Preview — this module
//! never touches `preview` (Constitution III). Node execution itself is the very
//! same code Preview uses (`runtime_adapter::pipeline`), so the two agree.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::Connection;

use crate::designer::runtime_adapter::pipeline::execute_node;
use crate::designer::runtime_adapter::{effective_parameters, Artifact, ExecContext, NodeError, Outputs, Registry};
use crate::domain::pipeline_snapshot::{GraphEdge, GraphNode, PipelineGraph};
use crate::kb::bundle::model::{Contract, GraphFile};
use crate::kb::bundle::read::{read_bundle, Bundle};
use crate::kb::catalog::query::path_of;
use crate::kb::identity::split_endpoint;

#[derive(Debug, Clone)]
pub struct RunNode {
    pub contract: Contract,
    pub implementation_id: String,
    pub implementation_version: String,
    pub definition_content_id: String,
}

/// Resolves every node of the published `pipe` to its published definition and
/// pinned implementation. Admission has already checked these exist.
pub fn resolve_run_nodes(conn: &Connection, root: &Path, pipe: &Bundle) -> Result<BTreeMap<String, RunNode>, String> {
    let graph = pipe.graph.as_ref().ok_or("the pipe has no graph")?;
    let mut out = BTreeMap::new();
    for n in &graph.nodes {
        let path = path_of(conn, "algonode", &n.node_ref.id, &n.node_ref.version, "published")
            .ok()
            .flatten()
            .ok_or_else(|| format!("{} is not available", n.node_ref.id))?;
        let dep = read_bundle(&root.join(path)).0.ok_or("a node definition cannot be read")?;
        let pin = graph.implementation_pins.iter().find(|p| p.instance_id == n.instance_id).ok_or("a node has no pinned implementation")?;
        out.insert(
            n.instance_id.clone(),
            RunNode {
                contract: dep.contract.clone().ok_or("a node has no contract")?,
                implementation_id: pin.implementation_id.clone(),
                implementation_version: pin.version.clone(),
                definition_content_id: n.node_ref.content_id.clone().unwrap_or_default(),
            },
        );
    }
    Ok(out)
}

/// The Run's pipeline graph: node type, pinned implementation + definition
/// identity as the "version", effective parameters, port-level edges — so the
/// Pipeline Snapshot's identity changes whenever anything computational does.
pub fn snapshot_graph(graph: &GraphFile, nodes: &BTreeMap<String, RunNode>) -> PipelineGraph {
    let mut pg = PipelineGraph::default();
    for n in &graph.nodes {
        let Some(rn) = nodes.get(&n.instance_id) else { continue };
        pg.nodes.push(GraphNode {
            node_id: n.instance_id.clone(),
            node_type: n.node_ref.id.clone(),
            implementation_version: format!("{}/{}/{}", rn.implementation_id, rn.implementation_version, rn.definition_content_id),
            canonical_parameters: serde_json::Value::Object(effective_parameters(&rn.contract, n.parameters.as_ref())),
            reproducible: rn.contract.reproducibility.as_ref().map(|r| r.deterministic).unwrap_or(false),
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

/// The unique node nothing consumes, or `None` when there is not exactly one.
pub fn default_target(graph: &GraphFile) -> Option<String> {
    let sinks: Vec<&str> = graph
        .nodes
        .iter()
        .map(|n| n.instance_id.as_str())
        .filter(|i| !graph.edges.iter().any(|e| split_endpoint(&e.from).0 == *i))
        .collect();
    (sinks.len() == 1).then(|| sinks[0].to_string())
}

pub struct OfficialOutput {
    pub artifact: Artifact,
    pub duration_ms: u64,
}

/// Runs `target`'s ancestor cone, uncached. `Err` names the failing node.
pub fn execute_pipe(
    graph: &GraphFile,
    nodes: &BTreeMap<String, RunNode>,
    pg: &PipelineGraph,
    target: &str,
    port: Option<&str>,
    ctx: &ExecContext,
) -> Result<OfficialOutput, (String, NodeError)> {
    let start = Instant::now();
    let registry = Registry::builtin();
    let order = pg
        .ancestors_of(target)
        .map_err(|e| (target.to_string(), NodeError::new("invalid_pipeline_graph", e.to_string())))?;
    let mut results: HashMap<String, Arc<Outputs>> = HashMap::new();
    for gn in order {
        let inst = gn.node_id.as_str();
        let rn = nodes.get(inst).ok_or_else(|| (inst.to_string(), NodeError::new("dependency_unavailable", "a node definition is missing")))?;
        let given = graph.nodes.iter().find(|n| n.instance_id == inst).and_then(|n| n.parameters.as_ref());
        let outputs = execute_node(graph, inst, &rn.contract, &rn.implementation_id, given, &results, ctx, &registry).map_err(|e| (inst.to_string(), e))?;
        results.insert(inst.to_string(), Arc::new(outputs));
    }
    let target_outputs = &results[target];
    let port_id = match port {
        Some(p) => p.to_string(),
        None => nodes[target].contract.outputs.first().map(|p| p.port_id.clone()).unwrap_or_default(),
    };
    let artifact = target_outputs
        .get(&port_id)
        .cloned()
        .ok_or_else(|| (target.to_string(), NodeError::new("output_missing", format!("the node has no output \"{port_id}\""))))?;
    Ok(OfficialOutput { artifact, duration_ms: start.elapsed().as_millis() as u64 })
}

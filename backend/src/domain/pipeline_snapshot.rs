//! `PipelineSnapshot` and Preview content-equivalence (data-model.md
//! PipelineSnapshot, FR-014/FR-015). A Preview artifact for a given target
//! node is reusable only if that node's own type, implementation version,
//! canonical parameters, and seed — together with every upstream node's
//! same fields and the ultimate source content identity — are unchanged.
//! Node position, selection state, and any other UI-only field never enter
//! this key (FR-014); a node that is not a pure function of those fields
//! MUST supply a `seed` before it can be cached or reused at all (FR-015).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn default_true() -> bool {
    true
}

fn default_implementation_version() -> String {
    "1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub node_id: String,
    pub node_type: String,
    #[serde(default = "default_implementation_version")]
    pub implementation_version: String,
    #[serde(default)]
    pub canonical_parameters: serde_json::Value,
    /// Whether this node's output is a pure function of its declared inputs
    /// (type + version + parameters + upstream content identity). A
    /// non-reproducible node (e.g. one that reads external/random state)
    /// MUST supply `seed` before its output can be cached or reused
    /// (FR-015).
    #[serde(default = "default_true")]
    pub reproducible: bool,
    #[serde(default)]
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

/// The graph shape sent as `pipeline_snapshot` in `POST /preview` and
/// `POST /runs` (contracts/local-service-api.md) — content-affecting nodes
/// and typed edges only, matching `PipelineSnapshot.graph_identity`'s
/// canonical serialization (FR-014).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl PipelineGraph {
    pub fn node(&self, node_id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.node_id == node_id)
    }

    fn input_node_id(&self, node_id: &str) -> Option<&str> {
        self.edges
            .iter()
            .find(|e| e.to == node_id)
            .map(|e| e.from.as_str())
    }

    /// The source-to-target chain of nodes feeding `target_node_id`,
    /// inclusive of the target itself, in execution order. Each node in
    /// this graph shape has at most one input edge, so the chain is a
    /// simple walk back to a root.
    fn chain_to(&self, target_node_id: &str) -> Result<Vec<&GraphNode>, PipelineSnapshotError> {
        let mut chain = Vec::new();
        let mut current = target_node_id.to_string();
        loop {
            let node = self
                .node(&current)
                .ok_or_else(|| PipelineSnapshotError::UnknownNode(current.clone()))?;
            chain.push(node);
            if chain.len() > self.nodes.len() {
                return Err(PipelineSnapshotError::Cycle);
            }
            match self.input_node_id(&current) {
                Some(prev) => current = prev.to_string(),
                None => break,
            }
        }
        chain.reverse();
        Ok(chain)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PipelineSnapshotError {
    #[error("unknown node in pipeline graph: {0}")]
    UnknownNode(String),
    #[error("pipeline graph contains a cycle")]
    Cycle,
    #[error("node(s) require a seed before their output can be cached or reused: {0:?}")]
    MissingSeed(Vec<String>),
}

/// Persisted, immutable identity of a whole pipeline graph (data-model.md
/// PipelineSnapshot). Used by official Runs (US4); Preview only needs the
/// per-node `compute_equivalence_key` below, but both derive from the same
/// canonical graph serialization so a Preview cache hit and a Run's
/// `pipeline_snapshot_id` agree on identical graphs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSnapshot {
    pub id: Uuid,
    pub graph_identity: String,
    pub node_versions: std::collections::BTreeMap<String, String>,
    pub canonical_parameters: std::collections::BTreeMap<String, serde_json::Value>,
    pub immutable: bool,
}

impl PipelineSnapshot {
    pub fn from_graph(graph: &PipelineGraph) -> Self {
        let graph_identity = compute_graph_identity(graph);
        Self {
            id: pipeline_snapshot_id(&graph_identity),
            node_versions: graph
                .nodes
                .iter()
                .map(|n| (n.node_id.clone(), n.implementation_version.clone()))
                .collect(),
            canonical_parameters: graph
                .nodes
                .iter()
                .map(|n| (n.node_id.clone(), n.canonical_parameters.clone()))
                .collect(),
            graph_identity,
            immutable: true,
        }
    }
}

/// Deterministically orders a JSON value's object keys so unrelated key
/// order never changes the hash — only content does.
fn canonical_json(value: &serde_json::Value) -> String {
    fn sorted(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                serde_json::Value::Object(
                    entries
                        .into_iter()
                        .map(|(k, v)| (k.clone(), sorted(v)))
                        .collect(),
                )
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(sorted).collect())
            }
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

/// BLAKE3 hash of the canonical graph — content-affecting fields only
/// (node type/version/parameters/seed, typed edges), no UI-only state such
/// as node position (FR-014).
pub fn compute_graph_identity(graph: &PipelineGraph) -> String {
    let mut nodes: Vec<&GraphNode> = graph.nodes.iter().collect();
    nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    let mut edges: Vec<&GraphEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| (a.from.as_str(), a.to.as_str()).cmp(&(b.from.as_str(), b.to.as_str())));

    let mut hasher = blake3::Hasher::new();
    for node in &nodes {
        hasher.update(node.node_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(node.node_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(node.implementation_version.as_bytes());
        hasher.update(b"\0");
        hasher.update(canonical_json(&node.canonical_parameters).as_bytes());
        hasher.update(b"\0");
        if let Some(seed) = node.seed {
            hasher.update(&seed.to_le_bytes());
        }
        hasher.update(b"\n");
    }
    for edge in &edges {
        hasher.update(edge.from.as_bytes());
        hasher.update(b"->");
        hasher.update(edge.to.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// Deterministic UUID derived from the graph identity, so identical graphs
/// always share one `PipelineSnapshot.id` (data-model.md: "id = pipeline
/// graph identity for this snapshot") without a lookup.
pub fn pipeline_snapshot_id(graph_identity: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, graph_identity.as_bytes())
}

/// FR-014's content-equivalence key for `target_node_id`: folds every
/// upstream node's type/version/parameters/seed and the ultimate source
/// content identity into one hash, so identical pipelines fed identical
/// input content always resolve to the same key — regardless of node
/// position, current UI selection, or pipeline revision label. Returns
/// `MissingSeed` if any node in the chain is non-reproducible and has no
/// seed (FR-015): such a node's output must never be cached or reused.
pub fn compute_equivalence_key(
    graph: &PipelineGraph,
    target_node_id: &str,
    source_content_identity: &str,
) -> Result<String, PipelineSnapshotError> {
    let chain = graph.chain_to(target_node_id)?;

    let missing_seed: Vec<String> = chain
        .iter()
        .filter(|n| !n.reproducible && n.seed.is_none())
        .map(|n| n.node_id.clone())
        .collect();
    if !missing_seed.is_empty() {
        return Err(PipelineSnapshotError::MissingSeed(missing_seed));
    }

    let mut content = source_content_identity.to_string();
    for node in chain {
        let mut hasher = blake3::Hasher::new();
        hasher.update(node.node_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(node.implementation_version.as_bytes());
        hasher.update(b"\0");
        hasher.update(canonical_json(&node.canonical_parameters).as_bytes());
        hasher.update(b"\0");
        hasher.update(content.as_bytes());
        hasher.update(b"\0");
        if let Some(seed) = node.seed {
            hasher.update(&seed.to_le_bytes());
        }
        content = hasher.finalize().to_hex().to_string();
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, ty: &str) -> GraphNode {
        GraphNode {
            node_id: id.into(),
            node_type: ty.into(),
            implementation_version: "1".into(),
            canonical_parameters: serde_json::json!({}),
            reproducible: true,
            seed: None,
        }
    }

    fn linear_graph() -> PipelineGraph {
        PipelineGraph {
            nodes: vec![node("source", "source"), node("blur", "gaussian")],
            edges: vec![GraphEdge {
                from: "source".into(),
                to: "blur".into(),
            }],
        }
    }

    #[test]
    fn identical_inputs_produce_identical_key() {
        let graph = linear_graph();
        let a = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap();
        let b = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_source_content_changes_key() {
        let graph = linear_graph();
        let a = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap();
        let b = compute_equivalence_key(&graph, "blur", "img-content-2").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_parameters_change_key() {
        let mut graph = linear_graph();
        let a = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap();
        graph.nodes[1].canonical_parameters = serde_json::json!({"sigma": 2});
        let b = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn parameter_key_order_does_not_affect_key() {
        let mut a = linear_graph();
        a.nodes[1].canonical_parameters = serde_json::json!({"sigma": 2, "mode": "fast"});
        let mut b = linear_graph();
        b.nodes[1].canonical_parameters = serde_json::json!({"mode": "fast", "sigma": 2});
        assert_eq!(
            compute_equivalence_key(&a, "blur", "img-content-1").unwrap(),
            compute_equivalence_key(&b, "blur", "img-content-1").unwrap()
        );
    }

    #[test]
    fn non_reproducible_node_without_seed_is_rejected() {
        let mut graph = linear_graph();
        graph.nodes[1].reproducible = false;
        let err = compute_equivalence_key(&graph, "blur", "img-content-1").unwrap_err();
        assert_eq!(err, PipelineSnapshotError::MissingSeed(vec!["blur".into()]));
    }

    #[test]
    fn non_reproducible_node_with_seed_is_accepted() {
        let mut graph = linear_graph();
        graph.nodes[1].reproducible = false;
        graph.nodes[1].seed = Some(42);
        assert!(compute_equivalence_key(&graph, "blur", "img-content-1").is_ok());
    }

    #[test]
    fn unknown_target_node_is_rejected() {
        let graph = linear_graph();
        assert_eq!(
            compute_equivalence_key(&graph, "missing", "img-content-1").unwrap_err(),
            PipelineSnapshotError::UnknownNode("missing".into())
        );
    }

    #[test]
    fn graph_identity_ignores_node_order() {
        let a = linear_graph();
        let mut b = linear_graph();
        b.nodes.reverse();
        assert_eq!(compute_graph_identity(&a), compute_graph_identity(&b));
    }

    #[test]
    fn identical_graphs_share_one_snapshot_id() {
        let a = compute_graph_identity(&linear_graph());
        let b = compute_graph_identity(&linear_graph());
        assert_eq!(pipeline_snapshot_id(&a), pipeline_snapshot_id(&b));
    }
}

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
    /// Named output port on `from` (feature 002 DAG graphs). Absent on
    /// legacy chain graphs; when absent it never enters any identity hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_port: Option<String>,
    /// Named input port on `to`; see `from_port`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_port: Option<String>,
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

    /// Incoming edges of `node_id` in a deterministic order (by target
    /// port, then source node, then source port).
    fn inputs_of(&self, node_id: &str) -> Vec<&GraphEdge> {
        let mut inputs: Vec<&GraphEdge> = self.edges.iter().filter(|e| e.to == node_id).collect();
        inputs.sort_by(|a, b| {
            (&a.to_port, &a.from, &a.from_port).cmp(&(&b.to_port, &b.from, &b.from_port))
        });
        inputs
    }

    /// `target_node_id` and every node that (transitively) feeds it, in a
    /// topological execution order (each node after all its inputs). For a
    /// legacy single-input chain this is exactly the source-to-target chain.
    pub fn ancestors_of(
        &self,
        target_node_id: &str,
    ) -> Result<Vec<&GraphNode>, PipelineSnapshotError> {
        enum Mark {
            Visiting,
            Done,
        }
        fn visit<'a>(
            graph: &'a PipelineGraph,
            id: &str,
            marks: &mut std::collections::HashMap<String, Mark>,
            order: &mut Vec<&'a GraphNode>,
        ) -> Result<(), PipelineSnapshotError> {
            match marks.get(id) {
                Some(Mark::Done) => return Ok(()),
                Some(Mark::Visiting) => return Err(PipelineSnapshotError::Cycle),
                None => {}
            }
            let node = graph
                .node(id)
                .ok_or_else(|| PipelineSnapshotError::UnknownNode(id.to_string()))?;
            marks.insert(id.to_string(), Mark::Visiting);
            for edge in graph.inputs_of(id) {
                visit(graph, &edge.from, marks, order)?;
            }
            marks.insert(id.to_string(), Mark::Done);
            order.push(node);
            Ok(())
        }
        let mut marks = std::collections::HashMap::new();
        let mut order = Vec::new();
        visit(self, target_node_id, &mut marks, &mut order)?;
        Ok(order)
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
    /// Feature 002: the published AlgoPipe `content_id` this snapshot was built
    /// from. Not part of the snapshot's identity.
    #[serde(default)]
    pub source_algopipe_content_id: Option<String>,
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
            source_algopipe_content_id: None,
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
    edges.sort_by(|a, b| {
        (&a.from, &a.to, &a.from_port, &a.to_port).cmp(&(&b.from, &b.to, &b.from_port, &b.to_port))
    });

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
        // Ports enter the hash only when present, so legacy port-less
        // graphs hash byte-identically to before (golden test).
        if let Some(port) = &edge.from_port {
            hasher.update(b":");
            hasher.update(port.as_bytes());
        }
        hasher.update(b"->");
        hasher.update(edge.to.as_bytes());
        if let Some(port) = &edge.to_port {
            hasher.update(b":");
            hasher.update(port.as_bytes());
        }
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
    let chain = graph.ancestors_of(target_node_id)?;

    let missing_seed: Vec<String> = chain
        .iter()
        .filter(|n| !n.reproducible && n.seed.is_none())
        .map(|n| n.node_id.clone())
        .collect();
    if !missing_seed.is_empty() {
        return Err(PipelineSnapshotError::MissingSeed(missing_seed));
    }

    // Each node's content = H(type, version, params, input content, seed).
    // A source node's input content is the source image identity; a
    // single-input node's is its upstream's content (so legacy chains keep
    // their exact keys); a multi-input node folds each `port=content` pair.
    let mut contents: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for node in &chain {
        let inputs = graph.inputs_of(&node.node_id);
        let input_content = match inputs.as_slice() {
            [] => source_content_identity.to_string(),
            [only] if only.to_port.is_none() && only.from_port.is_none() => {
                contents[only.from.as_str()].clone()
            }
            many => many
                .iter()
                .map(|e| {
                    format!(
                        "{}={}@{}",
                        e.to_port.as_deref().unwrap_or(""),
                        contents[e.from.as_str()],
                        e.from_port.as_deref().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(";"),
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(node.node_type.as_bytes());
        hasher.update(b"\0");
        hasher.update(node.implementation_version.as_bytes());
        hasher.update(b"\0");
        hasher.update(canonical_json(&node.canonical_parameters).as_bytes());
        hasher.update(b"\0");
        hasher.update(input_content.as_bytes());
        hasher.update(b"\0");
        if let Some(seed) = node.seed {
            hasher.update(&seed.to_le_bytes());
        }
        contents.insert(node.node_id.as_str(), hasher.finalize().to_hex().to_string());
    }
    Ok(contents[target_node_id].clone())
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
                from_port: None,
                to_port: None,
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

    #[test]
    fn multi_input_node_walks_all_ancestors_in_topological_order() {
        let e = |from: &str, to: &str, tp: &str| GraphEdge {
            from: from.into(),
            to: to.into(),
            from_port: Some("out".into()),
            to_port: Some(tp.into()),
        };
        let graph = PipelineGraph {
            nodes: vec![node("a", "source"), node("b", "source"), node("j", "join"), node("x", "other")],
            edges: vec![e("a", "j", "left"), e("b", "j", "right")],
        };
        let ids: Vec<_> = graph.ancestors_of("j").unwrap().iter().map(|n| n.node_id.clone()).collect();
        assert_eq!(ids, vec!["a", "b", "j"]);
        assert!(compute_equivalence_key(&graph, "j", "img").is_ok());
    }

    #[test]
    fn cycle_is_detected() {
        let e = |from: &str, to: &str| GraphEdge {
            from: from.into(),
            to: to.into(),
            from_port: None,
            to_port: None,
        };
        let graph = PipelineGraph {
            nodes: vec![node("a", "x"), node("b", "x")],
            edges: vec![e("a", "b"), e("b", "a")],
        };
        assert_eq!(graph.ancestors_of("b").unwrap_err(), PipelineSnapshotError::Cycle);
    }
}

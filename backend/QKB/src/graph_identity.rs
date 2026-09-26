//! Graph identity hashing, extracted verbatim from the legacy
//! `domain::pipeline_snapshot` so pipe `computational_identity` values stay
//! byte-identical (golden test `graph_identity_golden_test`).

use serde::{Deserialize, Serialize};

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


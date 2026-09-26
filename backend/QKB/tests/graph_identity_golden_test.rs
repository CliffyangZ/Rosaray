//! Golden regression for graph identity (ported from the legacy suite): pipe
//! `computational_identity` values must stay byte-identical so accepted
//! versions keep their identity across refactors.

use rosaray_qkb::graph_identity::{compute_graph_identity, GraphEdge, GraphNode, PipelineGraph};

fn node(id: &str, ty: &str, params: serde_json::Value, seed: Option<u64>) -> GraphNode {
    GraphNode {
        node_id: id.into(),
        node_type: ty.into(),
        implementation_version: "1".into(),
        canonical_parameters: params,
        reproducible: true,
        seed,
    }
}

fn edge(from: &str, to: &str) -> GraphEdge {
    GraphEdge { from: from.into(), to: to.into(), from_port: None, to_port: None }
}

fn graphs() -> Vec<PipelineGraph> {
    vec![
        PipelineGraph {
            nodes: vec![node("source", "source", serde_json::json!({}), None), node("blur", "gaussian", serde_json::json!({"sigma": 2}), None)],
            edges: vec![edge("source", "blur")],
        },
        PipelineGraph {
            nodes: vec![
                node("s", "source", serde_json::json!({}), None),
                node("n", "normalize", serde_json::json!({"mode": "minmax"}), None),
                node("t", "threshold", serde_json::json!({"level": 0.5, "otsu": false}), Some(7)),
                node("m", "morphology", serde_json::json!({"op": "open", "k": 3}), None),
            ],
            edges: vec![edge("s", "n"), edge("n", "t"), edge("t", "m")],
        },
        PipelineGraph {
            nodes: vec![node("a", "source", serde_json::json!({}), None), node("b", "area", serde_json::json!({"unit": "mm2"}), None)],
            edges: vec![edge("a", "b")],
        },
    ]
}

const GOLDEN_IDENTITY: [&str; 3] = [
    "edbaabad1b49d01306c2313b989e7bc9eae7d040f50c854e7333eb4d4d8225e3",
    "82bb41c8d7440bb22502b2620eae626f0939833797ddc0e235cd73af134c9970",
    "00417d1f28f85429b0fc6593f22be93f7af95dfada57765b148fcb4f3c6479a0",
];

#[test]
fn graph_identities_are_unchanged() {
    for (i, g) in graphs().iter().enumerate() {
        assert_eq!(compute_graph_identity(g), GOLDEN_IDENTITY[i], "graph identity {i} drifted");
    }
}

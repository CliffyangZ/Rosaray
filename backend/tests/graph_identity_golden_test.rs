//! Golden regression for feature 001's graph identity (T007). The hashes
//! below were captured from `pipeline_snapshot.rs` BEFORE the DAG
//! generalization (T008) and must stay byte-identical after it, so legacy
//! Runs and Preview cache keys keep resolving.

use rosaray_service::domain::pipeline_snapshot::{
    compute_equivalence_key, compute_graph_identity, GraphEdge, GraphNode, PipelineGraph,
};

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
    GraphEdge {
        from: from.into(),
        to: to.into(),
        from_port: None,
        to_port: None,
    }
}

fn graphs() -> Vec<(PipelineGraph, &'static str)> {
    vec![
        (
            PipelineGraph {
                nodes: vec![
                    node("source", "source", serde_json::json!({}), None),
                    node("blur", "gaussian", serde_json::json!({"sigma": 2}), None),
                ],
                edges: vec![edge("source", "blur")],
            },
            "blur",
        ),
        (
            PipelineGraph {
                nodes: vec![
                    node("s", "source", serde_json::json!({}), None),
                    node("n", "normalize", serde_json::json!({"mode": "minmax"}), None),
                    node("t", "threshold", serde_json::json!({"level": 0.5, "otsu": false}), Some(7)),
                    node("m", "morphology", serde_json::json!({"op": "open", "k": 3}), None),
                ],
                edges: vec![edge("s", "n"), edge("n", "t"), edge("t", "m")],
            },
            "m",
        ),
        (
            PipelineGraph {
                nodes: vec![
                    node("a", "source", serde_json::json!({}), None),
                    node("b", "area", serde_json::json!({"unit": "mm2"}), None),
                ],
                edges: vec![edge("a", "b")],
            },
            "a",
        ),
    ]
}

const GOLDEN_IDENTITY: [&str; 3] = [
    "edbaabad1b49d01306c2313b989e7bc9eae7d040f50c854e7333eb4d4d8225e3",
    "82bb41c8d7440bb22502b2620eae626f0939833797ddc0e235cd73af134c9970",
    "00417d1f28f85429b0fc6593f22be93f7af95dfada57765b148fcb4f3c6479a0",
];
const GOLDEN_KEY: [&str; 3] = [
    "e45d6dab39c9078815479a7f4f789f842076923f434000c9a20cc884567686a7",
    "d7a6188da91ea67e568d10d86ef6e76f5aa40793dcac14c02d03dceafd7cf5dd",
    "f0da5dce8b35fdd23c182b40c28afb83d1de489d6d9926df11518e4adbbb41a4",
];

#[test]
fn legacy_chain_identities_are_unchanged() {
    let computed: Vec<(String, String)> = graphs()
        .iter()
        .map(|(g, target)| {
            (
                compute_graph_identity(g),
                compute_equivalence_key(g, target, "img-content-1").unwrap(),
            )
        })
        .collect();
    for (i, (id, key)) in computed.iter().enumerate() {
        println!("GOLDEN {i} identity={id} key={key}");
    }
    for (i, (id, key)) in computed.iter().enumerate() {
        assert_eq!(id, GOLDEN_IDENTITY[i], "graph identity {i} drifted");
        assert_eq!(key, GOLDEN_KEY[i], "equivalence key {i} drifted");
    }
}

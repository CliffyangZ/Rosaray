//! Official Run orchestration (User Story 4): every Run is durably,
//! atomically, and fully traceably recorded (FR-018/FR-020/FR-022) —
//! never marked successful with missing data (FR-020), never silently
//! losing failure detail (FR-021), and never silently accepting a
//! divergent result from a supposedly-identical repeat (FR-015, SC-006).
//! Runs never touch the Preview cache (Constitution Principle III) — a
//! Run's input/output always round-trips through the blob store and
//! SQLite on its own, independent of anything Preview has cached for the
//! same content.

use std::time::Instant;

use crate::domain::pipeline_snapshot::{
    compute_equivalence_key, PipelineGraph, PipelineSnapshotError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    UnknownNode(String),
    Cycle,
    MissingSeed(Vec<String>),
    ComputeFailed { failing_node_id: String },
    /// FR-015/SC-006: this Run's own output diverged from a prior Run with
    /// an identical dataset version, image, pipeline snapshot, seed, and
    /// target node — the node is not a pure function of its declared
    /// inputs and must not be silently trusted.
    NonReproducible { failing_node_id: String },
    PersistenceFailed,
}

impl From<PipelineSnapshotError> for RunError {
    fn from(err: PipelineSnapshotError) -> Self {
        match err {
            PipelineSnapshotError::UnknownNode(id) => RunError::UnknownNode(id),
            PipelineSnapshotError::Cycle => RunError::Cycle,
            PipelineSnapshotError::MissingSeed(ids) => RunError::MissingSeed(ids),
        }
    }
}

impl RunError {
    pub fn failing_node_id(&self, target_node_id: &str) -> String {
        match self {
            RunError::UnknownNode(id) => id.clone(),
            RunError::Cycle => target_node_id.to_string(),
            RunError::MissingSeed(ids) => ids
                .first()
                .cloned()
                .unwrap_or_else(|| target_node_id.to_string()),
            RunError::ComputeFailed { failing_node_id } => failing_node_id.clone(),
            RunError::NonReproducible { failing_node_id } => failing_node_id.clone(),
            RunError::PersistenceFailed => target_node_id.to_string(),
        }
    }

    /// FR-021: a safe error summary derived only from node/pipeline
    /// identities — never raw image content, never non-essential subject
    /// information (patient id, split, file paths).
    pub fn redacted_summary(&self) -> String {
        match self {
            RunError::UnknownNode(id) => format!("Pipeline references an unknown node '{id}'."),
            RunError::Cycle => "Pipeline graph contains a cycle.".to_string(),
            RunError::MissingSeed(ids) => format!(
                "Node(s) require a seed before they can be run: {ids:?}"
            ),
            RunError::ComputeFailed { failing_node_id } => {
                format!("Node '{failing_node_id}' failed to produce output.")
            }
            RunError::NonReproducible { failing_node_id } => format!(
                "Node '{failing_node_id}' produced a different result than a prior Run with identical inputs; it is not reproducible."
            ),
            RunError::PersistenceFailed => {
                "Run output could not be durably persisted.".to_string()
            }
        }
    }
}

#[derive(Debug)]
pub struct RunComputeOutcome {
    pub output_bytes: Vec<u8>,
    pub duration_ms: u64,
}

/// Deterministically computes the target node's output for one official
/// Run, given the same FR-014 content-equivalence key Preview uses — an
/// official Run and a Preview of content-equivalent inputs are expected to
/// agree (both stand in for the same future Algorithm Layer node runtime,
/// `data_engine::preview::compute_output_bytes`).
pub fn compute_run_output(
    graph: &PipelineGraph,
    target_node_id: &str,
    source_content_identity: &str,
) -> Result<RunComputeOutcome, RunError> {
    let start = Instant::now();
    let equivalence_key =
        compute_equivalence_key(graph, target_node_id, source_content_identity)?;
    let output_bytes = crate::data_engine::preview::compute_output_bytes(&equivalence_key);
    Ok(RunComputeOutcome {
        output_bytes,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pipeline_snapshot::{GraphEdge, GraphNode};

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
    fn identical_inputs_produce_identical_output() {
        let graph = linear_graph();
        let a = compute_run_output(&graph, "blur", "img-1").unwrap();
        let b = compute_run_output(&graph, "blur", "img-1").unwrap();
        assert_eq!(a.output_bytes, b.output_bytes);
    }

    #[test]
    fn different_source_content_changes_output() {
        let graph = linear_graph();
        let a = compute_run_output(&graph, "blur", "img-1").unwrap();
        let b = compute_run_output(&graph, "blur", "img-2").unwrap();
        assert_ne!(a.output_bytes, b.output_bytes);
    }

    #[test]
    fn non_reproducible_node_without_seed_is_rejected() {
        let mut graph = linear_graph();
        graph.nodes[1].reproducible = false;
        let err = compute_run_output(&graph, "blur", "img-1").unwrap_err();
        assert_eq!(err, RunError::MissingSeed(vec!["blur".into()]));
        assert_eq!(err.failing_node_id("blur"), "blur");
    }

    #[test]
    fn redacted_summary_never_echoes_raw_content_identity() {
        let err = RunError::ComputeFailed {
            failing_node_id: "blur".into(),
        };
        let summary = err.redacted_summary();
        assert!(summary.contains("blur"));
        assert!(!summary.contains("img-1"));
    }
}

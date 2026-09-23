//! Preview compute orchestration (User Story 3): cache-hit reuse, cache-miss
//! recompute, rejection of non-reproducible nodes without a qualifying seed
//! (FR-015), lineage/diagnostics (FR-016), and request-context isolation so
//! a late-arriving result for a superseded selection never overwrites that
//! selection's last successful artifact (FR-017). Never creates a
//! `RunRecord` (FR-013) — that is `data_engine::run` (User Story 4).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use uuid::Uuid;

use crate::data_repository::memory_cache::{PreviewCache, PreviewCacheEntry};
use crate::domain::pipeline_snapshot::{
    compute_equivalence_key, compute_graph_identity, pipeline_snapshot_id, PipelineGraph,
    PipelineSnapshotError,
};
use crate::domain::ArtifactKind;

#[derive(Debug, Clone)]
pub struct LastSuccessful {
    pub content_identity: String,
    pub kind: ArtifactKind,
}

/// Tracks, per `(image_asset_id, target_node_id)` selection, which request
/// is currently the latest. A completion whose generation is no longer the
/// latest for its selection is a late arrival for a superseded image,
/// pipeline revision, or target node — its result is still returned to its
/// own caller, but it must never become the selection's recorded
/// `last_successful` result (FR-017).
#[derive(Default)]
pub struct RequestIsolation {
    generation: Mutex<HashMap<(Uuid, String), u64>>,
    last_successful: Mutex<HashMap<(Uuid, String), LastSuccessful>>,
}

impl RequestIsolation {
    fn key(image_asset_id: Uuid, target_node_id: &str) -> (Uuid, String) {
        (image_asset_id, target_node_id.to_string())
    }

    pub fn begin(&self, image_asset_id: Uuid, target_node_id: &str) -> u64 {
        let mut map = self.generation.lock().unwrap();
        let key = Self::key(image_asset_id, target_node_id);
        let next = map.get(&key).copied().unwrap_or(0) + 1;
        map.insert(key, next);
        next
    }

    pub fn is_current(&self, image_asset_id: Uuid, target_node_id: &str, generation: u64) -> bool {
        let map = self.generation.lock().unwrap();
        map.get(&Self::key(image_asset_id, target_node_id)) == Some(&generation)
    }

    /// Records `result` as the selection's latest success only if no newer
    /// request has started since `generation` was issued; a superseded
    /// completion is silently dropped instead of clobbering a fresher
    /// result (FR-017).
    pub fn record_success(
        &self,
        image_asset_id: Uuid,
        target_node_id: &str,
        generation: u64,
        result: LastSuccessful,
    ) {
        if !self.is_current(image_asset_id, target_node_id, generation) {
            return;
        }
        self.last_successful
            .lock()
            .unwrap()
            .insert(Self::key(image_asset_id, target_node_id), result);
    }

    pub fn last_successful(
        &self,
        image_asset_id: Uuid,
        target_node_id: &str,
    ) -> Option<LastSuccessful> {
        self.last_successful
            .lock()
            .unwrap()
            .get(&Self::key(image_asset_id, target_node_id))
            .cloned()
    }
}

#[derive(Debug, Clone)]
pub struct PreviewDiagnostics {
    pub input_content_identity: String,
    pub output_content_identity: String,
    pub producing_node_id: String,
    pub duration_ms: u64,
    pub cache_hit: bool,
}

#[derive(Debug)]
pub struct PreviewOutcome {
    pub artifact_content_identity: String,
    pub artifact_kind: ArtifactKind,
    pub reused: bool,
    pub request_context_id: Uuid,
    pub pipeline_snapshot_id: Uuid,
    pub diagnostics: PreviewDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    UnknownNode(String),
    Cycle,
    MissingSeed(Vec<String>),
    ComputeFailed { failing_node_id: String },
}

impl From<PipelineSnapshotError> for PreviewError {
    fn from(err: PipelineSnapshotError) -> Self {
        match err {
            PipelineSnapshotError::UnknownNode(id) => PreviewError::UnknownNode(id),
            PipelineSnapshotError::Cycle => PreviewError::Cycle,
            PipelineSnapshotError::MissingSeed(ids) => PreviewError::MissingSeed(ids),
        }
    }
}

impl PreviewError {
    /// The single node responsible for this failure, for the contract's
    /// "failing node identified" requirement (FR-017, Acceptance Scenario
    /// US3-4). `target_node_id` is the fallback when the graph itself never
    /// resolved past lookup (e.g. an unknown target).
    pub fn failing_node_id(&self, target_node_id: &str) -> String {
        match self {
            PreviewError::UnknownNode(id) => id.clone(),
            PreviewError::Cycle => target_node_id.to_string(),
            PreviewError::MissingSeed(ids) => ids
                .first()
                .cloned()
                .unwrap_or_else(|| target_node_id.to_string()),
            PreviewError::ComputeFailed { failing_node_id } => failing_node_id.clone(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            PreviewError::UnknownNode(_) => "unknown_node",
            PreviewError::Cycle => "invalid_pipeline_graph",
            PreviewError::MissingSeed(_) => "missing_seed",
            PreviewError::ComputeFailed { .. } => "compute_failed",
        }
    }

    pub fn message(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PreviewError::UnknownNode(id) => write!(f, "unknown target node '{id}'"),
            PreviewError::Cycle => write!(f, "pipeline graph contains a cycle"),
            PreviewError::MissingSeed(ids) => write!(
                f,
                "node(s) require a seed before their output can be reused: {ids:?}"
            ),
            PreviewError::ComputeFailed { failing_node_id } => {
                write!(f, "node '{failing_node_id}' failed to produce output")
            }
        }
    }
}

/// Placeholder for the future Algorithm Layer/Execution Layer node runtime
/// (plan.md Project Structure scopes actual pixel computation outside the
/// Data Layer). Given the already-verified content-equivalence key, this
/// deterministically derives output bytes so identical inputs always
/// reproduce identical output content — the exact guarantee FR-015 requires
/// of a reproducible node, without depending on a real node executor
/// existing yet.
pub fn compute_output_bytes(equivalence_key: &str) -> Vec<u8> {
    equivalence_key.as_bytes().to_vec()
}

pub struct PreviewRequest<'a> {
    pub image_asset_id: Uuid,
    pub source_content_identity: &'a str,
    pub graph: &'a PipelineGraph,
    pub target_node_id: &'a str,
}

/// Runs one `POST /preview` request end to end: derives the FR-014
/// equivalence key (rejecting a non-reproducible node with no seed per
/// FR-015), reuses a cache hit or recomputes on a miss via `write_output`,
/// and — only if this request is still the latest for its selection —
/// records the result as that selection's `last_successful` (FR-017).
pub fn run_preview(
    cache: &PreviewCache,
    isolation: &RequestIsolation,
    write_output: impl FnOnce(&[u8]) -> Result<String, ()>,
    req: PreviewRequest,
) -> Result<PreviewOutcome, PreviewError> {
    let generation = isolation.begin(req.image_asset_id, req.target_node_id);
    let request_context_id = Uuid::new_v4();

    let graph_identity = compute_graph_identity(req.graph);
    let snapshot_id = pipeline_snapshot_id(&graph_identity);

    let equivalence_key =
        compute_equivalence_key(req.graph, req.target_node_id, req.source_content_identity)?;

    let start = Instant::now();
    let (output_identity, cache_hit) = match cache.get(&equivalence_key) {
        Some(hit) => (hit.output_content_identity, true),
        None => {
            let bytes = compute_output_bytes(&equivalence_key);
            let identity = write_output(&bytes).map_err(|_| PreviewError::ComputeFailed {
                failing_node_id: req.target_node_id.to_string(),
            })?;
            cache.insert(
                equivalence_key,
                PreviewCacheEntry {
                    output_content_identity: identity.clone(),
                    producing_node_id: req.target_node_id.to_string(),
                    pipeline_snapshot_id: snapshot_id,
                    image_asset_id: req.image_asset_id,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            );
            (identity, false)
        }
    };
    let duration_ms = start.elapsed().as_millis() as u64;

    isolation.record_success(
        req.image_asset_id,
        req.target_node_id,
        generation,
        LastSuccessful {
            content_identity: output_identity.clone(),
            kind: ArtifactKind::Image,
        },
    );

    Ok(PreviewOutcome {
        artifact_content_identity: output_identity.clone(),
        artifact_kind: ArtifactKind::Image,
        reused: cache_hit,
        request_context_id,
        pipeline_snapshot_id: snapshot_id,
        diagnostics: PreviewDiagnostics {
            input_content_identity: req.source_content_identity.to_string(),
            output_content_identity: output_identity,
            producing_node_id: req.target_node_id.to_string(),
            duration_ms,
            cache_hit,
        },
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

    fn write_to_map<'a>(
        store: &'a Mutex<HashMap<String, Vec<u8>>>,
    ) -> impl FnOnce(&[u8]) -> Result<String, ()> + 'a {
        move |bytes: &[u8]| {
            let identity = crate::domain::content_identity::content_identity(bytes);
            store
                .lock()
                .unwrap()
                .insert(identity.clone(), bytes.to_vec());
            Ok(identity)
        }
    }

    #[test]
    fn repeated_identical_request_is_reused_from_cache() {
        let cache = PreviewCache::default();
        let isolation = RequestIsolation::default();
        let graph = linear_graph();
        let store = Mutex::new(HashMap::new());

        let first = run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                image_asset_id: Uuid::new_v4(),
                source_content_identity: "img-1",
                graph: &graph,
                target_node_id: "blur",
            },
        )
        .unwrap();
        assert!(!first.reused);

        let second = run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                // A different `image_asset_id` on purpose: cache reuse is
                // keyed by content-equivalence, not by which selection asked.
                image_asset_id: Uuid::new_v4(),
                source_content_identity: "img-1",
                graph: &graph,
                target_node_id: "blur",
            },
        );
        assert!(second.unwrap().reused);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_clear_produces_a_recomputable_miss() {
        let cache = PreviewCache::default();
        let isolation = RequestIsolation::default();
        let graph = linear_graph();
        let store = Mutex::new(HashMap::new());
        let image_asset_id = Uuid::new_v4();

        run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                image_asset_id,
                source_content_identity: "img-1",
                graph: &graph,
                target_node_id: "blur",
            },
        )
        .unwrap();

        cache.clear();

        let after_clear = run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                image_asset_id,
                source_content_identity: "img-1",
                graph: &graph,
                target_node_id: "blur",
            },
        )
        .unwrap();
        assert!(!after_clear.reused);
    }

    #[test]
    fn non_reproducible_node_without_seed_is_rejected_without_touching_cache() {
        let cache = PreviewCache::default();
        let isolation = RequestIsolation::default();
        let mut graph = linear_graph();
        graph.nodes[1].reproducible = false;
        let store = Mutex::new(HashMap::new());

        let err = run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                image_asset_id: Uuid::new_v4(),
                source_content_identity: "img-1",
                graph: &graph,
                target_node_id: "blur",
            },
        )
        .unwrap_err();

        assert_eq!(err, PreviewError::MissingSeed(vec!["blur".into()]));
        assert_eq!(err.failing_node_id("blur"), "blur");
        assert!(cache.is_empty());
    }

    /// FR-017: an older request that finishes after a newer one was issued
    /// for the *same selection* must never overwrite the newer request's
    /// recorded `last_successful` result.
    #[test]
    fn superseded_completion_never_overwrites_last_successful() {
        let cache = PreviewCache::default();
        let isolation = RequestIsolation::default();
        let image_asset_id = Uuid::new_v4();
        let store = Mutex::new(HashMap::new());

        let mut fresh_graph = linear_graph();
        fresh_graph.nodes[1].canonical_parameters = serde_json::json!({"sigma": 3});

        // Simulate the older request having *started* first (generation 1)
        // by pre-registering it, then let a newer request (generation 2)
        // both start and finish before the older one's late completion is
        // recorded.
        let older_generation = isolation.begin(image_asset_id, "blur");

        let fresh = run_preview(
            &cache,
            &isolation,
            write_to_map(&store),
            PreviewRequest {
                image_asset_id,
                source_content_identity: "img-1",
                graph: &fresh_graph,
                target_node_id: "blur",
            },
        )
        .unwrap();

        // The stale request now completes and tries to record its result —
        // it must be dropped because a newer generation has since started.
        isolation.record_success(
            image_asset_id,
            "blur",
            older_generation,
            LastSuccessful {
                content_identity: "stale-output".into(),
                kind: ArtifactKind::Image,
            },
        );

        let recorded = isolation.last_successful(image_asset_id, "blur").unwrap();
        assert_eq!(recorded.content_identity, fresh.artifact_content_identity);
    }
}

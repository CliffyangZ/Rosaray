//! Query/persistence for PipelineSnapshot/RunInputArtifact/RunRecord/
//! MetricSet (FR-051: business rules live in `data_engine`, this module is
//! storage only). The atomicity `data-model.md` requires of `RunRecord`'s
//! `succeeded` transition (FR-020) is a property of *how callers sequence*
//! these functions inside one `rusqlite::Transaction`, not of any single
//! function here — see `finalize_run_success`.

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::domain::pipeline_snapshot::PipelineSnapshot;
use crate::domain::run::{MetricSet, RunInputArtifact, RunPolicy, RunRecord, RunStatus};

/// Idempotent: a `PipelineSnapshot.id` is deterministic from its graph
/// identity (`pipeline_snapshot_id`), so re-running an identical pipeline
/// never produces a duplicate row.
pub fn insert_pipeline_snapshot(
    conn: &Connection,
    snapshot: &PipelineSnapshot,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO pipeline_snapshots (id, graph_identity, node_versions_json, canonical_parameters_json)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            snapshot.id.to_string(),
            snapshot.graph_identity,
            serde_json::to_string(&snapshot.node_versions).unwrap(),
            serde_json::to_string(&snapshot.canonical_parameters).unwrap(),
        ],
    )?;
    Ok(())
}

pub fn insert_run_input_artifact(
    conn: &Connection,
    artifact: &RunInputArtifact,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO run_input_artifacts (id, content_identity, source_image_asset_id, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            artifact.id.to_string(),
            artifact.content_identity,
            artifact.source_image_asset_id.to_string(),
            artifact.created_at,
        ],
    )?;
    Ok(())
}

/// Inserts the initial `running` row (FR-018): every identity the Run needs
/// is already fixed at this point (dataset version + fingerprint, image
/// identity, input artifact, pipeline snapshot, seed) — only the outcome
/// (`output_content_identities`, `metric_set_id`, `ended_at`) is still
/// pending.
pub fn insert_run_record(conn: &Connection, run: &RunRecord) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO run_records (
            id, status, dataset_version_id, dataset_fingerprint, image_asset_id, image_asset_identity,
            run_input_artifact_id, pipeline_snapshot_id, target_node_id, seed, node_versions_json,
            started_at, ended_at, output_content_identities_json, retain_intermediates,
            metric_set_id, error_summary, failed_stage
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            run.id.to_string(),
            run.status.as_str(),
            run.dataset_version_id.to_string(),
            run.dataset_fingerprint,
            run.image_asset_id.to_string(),
            run.image_asset_identity,
            run.run_input_artifact_id.to_string(),
            run.pipeline_snapshot_id.to_string(),
            run.target_node_id,
            run.seed as i64,
            serde_json::to_string(&run.node_versions).unwrap(),
            run.started_at,
            run.ended_at,
            serde_json::to_string(&run.output_content_identities).unwrap(),
            run.run_policy.retain_intermediates as i64,
            run.metric_set_id.map(|id| id.to_string()),
            run.error_summary,
            run.failed_stage,
        ],
    )?;
    Ok(())
}

pub fn insert_metric_set(conn: &Connection, metrics: &MetricSet) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO metric_sets (id, run_record_id, dice, area_mm2, foreground_pixels, connected_components, step_timings_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            metrics.id.to_string(),
            metrics.run_record_id.to_string(),
            metrics.dice,
            metrics.area_mm2,
            metrics.foreground_pixels,
            metrics.connected_components,
            serde_json::to_string(&metrics.step_timings).unwrap(),
        ],
    )?;
    Ok(())
}

/// Persists the successful outcome (FR-020, data-model.md RunRecord
/// validation rule). Callers MUST run this in the same `rusqlite`
/// transaction as `insert_metric_set` — that pairing, committed or not
/// together, is what makes a `succeeded` row never partially populated: any
/// interruption before `tx.commit()` leaves the row `running` (its prior
/// state), never `succeeded` with a missing `metric_set_id`.
pub fn finalize_run_success(
    conn: &Connection,
    run_id: Uuid,
    ended_at: &str,
    output_content_identities: &[String],
    metric_set_id: Uuid,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE run_records
         SET status = 'succeeded', ended_at = ?1, output_content_identities_json = ?2, metric_set_id = ?3
         WHERE id = ?4 AND status = 'running'",
        params![
            ended_at,
            serde_json::to_string(output_content_identities).unwrap(),
            metric_set_id.to_string(),
            run_id.to_string(),
        ],
    )?;
    Ok(())
}

/// Persists a failure (FR-021): retains the Run Record, never fabricates a
/// successful-looking result, and records only the redacted `error_summary`
/// the caller computed (never raw image content or non-essential subject
/// info — that redaction is `data_engine::run::RunError`'s job, not this
/// storage layer's).
pub fn finalize_run_failure(
    conn: &Connection,
    run_id: Uuid,
    ended_at: &str,
    error_summary: &str,
    failed_stage: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE run_records
         SET status = 'failed', ended_at = ?1, error_summary = ?2, failed_stage = ?3
         WHERE id = ?4 AND status = 'running'",
        params![ended_at, error_summary, failed_stage, run_id.to_string()],
    )?;
    Ok(())
}

/// Startup recovery (FR-020, quickstart.md §4.2): a fresh process start
/// means no `running` row can actually still be executing, so any left over
/// from an interrupted process are reconciled to `failed` — never silently
/// left looking "in progress" forever, and never promoted to `succeeded`
/// (data would be missing).
pub fn recover_interrupted_runs(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE run_records
         SET status = 'failed', ended_at = COALESCE(ended_at, started_at),
             error_summary = 'Run was interrupted before it finished (process restarted).',
             failed_stage = 'persisting'
         WHERE status = 'running'",
        [],
    )
}

fn row_to_run_record(row: &rusqlite::Row) -> rusqlite::Result<RunRecord> {
    let output_json: String = row.get(13)?;
    let node_versions_json: String = row.get(10)?;
    Ok(RunRecord {
        id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
        status: RunStatus::parse(&row.get::<_, String>(1)?),
        dataset_version_id: Uuid::parse_str(&row.get::<_, String>(2)?).unwrap(),
        dataset_fingerprint: row.get(3)?,
        image_asset_id: Uuid::parse_str(&row.get::<_, String>(4)?).unwrap(),
        image_asset_identity: row.get(5)?,
        run_input_artifact_id: Uuid::parse_str(&row.get::<_, String>(6)?).unwrap(),
        pipeline_snapshot_id: Uuid::parse_str(&row.get::<_, String>(7)?).unwrap(),
        target_node_id: row.get(8)?,
        seed: row.get::<_, i64>(9)? as u64,
        node_versions: serde_json::from_str(&node_versions_json).unwrap_or_default(),
        started_at: row.get(11)?,
        ended_at: row.get(12)?,
        output_content_identities: serde_json::from_str(&output_json).unwrap_or_default(),
        run_policy: RunPolicy {
            retain_intermediates: row.get::<_, i64>(14)? != 0,
        },
        metric_set_id: row
            .get::<_, Option<String>>(15)?
            .map(|s| Uuid::parse_str(&s).unwrap()),
        error_summary: row.get(16)?,
        failed_stage: row.get(17)?,
    })
}

const RUN_RECORD_COLUMNS: &str = "id, status, dataset_version_id, dataset_fingerprint, image_asset_id, image_asset_identity,
     run_input_artifact_id, pipeline_snapshot_id, target_node_id, seed, node_versions_json,
     started_at, ended_at, output_content_identities_json, retain_intermediates, metric_set_id, error_summary, failed_stage";

pub fn get_run_record(conn: &Connection, run_id: Uuid) -> rusqlite::Result<Option<RunRecord>> {
    conn.query_row(
        &format!("SELECT {RUN_RECORD_COLUMNS} FROM run_records WHERE id = ?1"),
        params![run_id.to_string()],
        row_to_run_record,
    )
    .optional()
}

/// Filtered run history (`GET /runs?dataset_version_id=&image_asset_id=`),
/// newest first.
pub fn list_run_records(
    conn: &Connection,
    dataset_version_id: Option<Uuid>,
    image_asset_id: Option<Uuid>,
) -> rusqlite::Result<Vec<RunRecord>> {
    let mut sql = format!("SELECT {RUN_RECORD_COLUMNS} FROM run_records WHERE 1 = 1");
    if dataset_version_id.is_some() {
        sql.push_str(" AND dataset_version_id = ?1");
    }
    if image_asset_id.is_some() {
        sql.push_str(if dataset_version_id.is_some() {
            " AND image_asset_id = ?2"
        } else {
            " AND image_asset_id = ?1"
        });
    }
    sql.push_str(" ORDER BY started_at DESC");

    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<String> = [dataset_version_id, image_asset_id]
        .into_iter()
        .flatten()
        .map(|id| id.to_string())
        .collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(ids), row_to_run_record)?;
    rows.collect()
}

pub fn get_metric_set(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<MetricSet>> {
    conn.query_row(
        "SELECT id, run_record_id, dice, area_mm2, foreground_pixels, connected_components, step_timings_json
         FROM metric_sets WHERE id = ?1",
        params![id.to_string()],
        |row| {
            let step_timings_json: String = row.get(6)?;
            Ok(MetricSet {
                id,
                run_record_id: Uuid::parse_str(&row.get::<_, String>(1)?).unwrap(),
                dice: row.get(2)?,
                area_mm2: row.get(3)?,
                foreground_pixels: row.get(4)?,
                connected_components: row.get(5)?,
                step_timings: serde_json::from_str(&step_timings_json).unwrap_or_default(),
            })
        },
    )
    .optional()
}

/// The first successful Run's output for this exact composite input
/// (dataset version, image, pipeline snapshot, seed, target node) — used by
/// the reproducibility check (FR-015/SC-006): a later identical-input Run
/// whose own output diverges from this one is not a pure function of its
/// declared inputs and must be classified non-reproducible rather than
/// silently accepted.
pub fn first_successful_output(
    conn: &Connection,
    dataset_version_id: Uuid,
    image_asset_id: Uuid,
    pipeline_snapshot_id: Uuid,
    seed: u64,
    target_node_id: &str,
) -> rusqlite::Result<Option<String>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT output_content_identities_json FROM run_records
             WHERE dataset_version_id = ?1 AND image_asset_id = ?2 AND pipeline_snapshot_id = ?3
               AND seed = ?4 AND target_node_id = ?5 AND status = 'succeeded'
             ORDER BY started_at ASC LIMIT 1",
            params![
                dataset_version_id.to_string(),
                image_asset_id.to_string(),
                pipeline_snapshot_id.to_string(),
                seed as i64,
                target_node_id,
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json
        .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
        .and_then(|v| v.into_iter().next()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_repository::sqlite;
    use crate::data_repository::sqlite::dataset_repo;
    use crate::domain::dataset::{Dimensions, ImageAsset, ImageAssetStatus, MetadataStatus, Split};
    use crate::domain::dataset::{DatasetVersion, ValidationStatus, ValidationSummary};
    use chrono::Utc;

    fn setup() -> (Connection, Uuid, Uuid, Uuid) {
        let conn = Connection::open_in_memory().unwrap();
        sqlite::run_migrations(&conn).unwrap();
        let project_id = sqlite::ensure_default_project(&conn).unwrap();
        let dataset = dataset_repo::get_or_create_dataset(&conn, project_id, "Test").unwrap();
        let image_id = Uuid::new_v4();
        let asset = ImageAsset {
            id: image_id,
            external_source_uri: "file:///x.png".into(),
            source_content_identity: "src-1".into(),
            imported_content_identity: "imp-1".into(),
            dimensions: Dimensions {
                width: 10,
                height: 10,
            },
            source_created_at: None,
            imported_at: Utc::now().to_rfc3339(),
            status: ImageAssetStatus::Available,
            patient_id: Some("P1".into()),
            split: Some(Split::Train),
            reference_mask_id: None,
            metadata_status: MetadataStatus::Complete,
        };
        dataset_repo::insert_image_asset(&conn, &asset).unwrap();
        let version = DatasetVersion {
            id: Uuid::new_v4(),
            dataset_id: dataset.id,
            fingerprint: "fp-1".into(),
            derived_from_version_id: None,
            created_at: Utc::now().to_rfc3339(),
            image_asset_ids: vec![image_id],
            validation_summary: ValidationSummary {
                status: ValidationStatus::Ok,
                finding_ids: vec![],
            },
        };
        dataset_repo::insert_dataset_version(&conn, &version).unwrap();
        (conn, dataset.id, version.id, image_id)
    }

    fn sample_snapshot() -> PipelineSnapshot {
        use crate::domain::pipeline_snapshot::{GraphNode, PipelineGraph};
        let graph = PipelineGraph {
            nodes: vec![GraphNode {
                node_id: "n1".into(),
                node_type: "source".into(),
                implementation_version: "1".into(),
                canonical_parameters: serde_json::json!({}),
                reproducible: true,
                seed: None,
            }],
            edges: vec![],
        };
        PipelineSnapshot::from_graph(&graph)
    }

    #[test]
    fn run_record_round_trips_running_to_succeeded() {
        let (conn, _dataset_id, version_id, image_id) = setup();
        let snapshot = sample_snapshot();
        insert_pipeline_snapshot(&conn, &snapshot).unwrap();

        let run_id = Uuid::new_v4();
        let input_artifact = RunInputArtifact {
            id: Uuid::new_v4(),
            content_identity: "imp-1".into(),
            source_image_asset_id: image_id,
            created_at: Utc::now().to_rfc3339(),
        };
        insert_run_input_artifact(&conn, &input_artifact).unwrap();

        let run = RunRecord {
            id: run_id,
            status: RunStatus::Running,
            dataset_version_id: version_id,
            dataset_fingerprint: "fp-1".into(),
            image_asset_id: image_id,
            image_asset_identity: "imp-1".into(),
            run_input_artifact_id: input_artifact.id,
            pipeline_snapshot_id: snapshot.id,
            target_node_id: "n1".into(),
            seed: 42,
            node_versions: snapshot.node_versions.clone(),
            started_at: Utc::now().to_rfc3339(),
            ended_at: None,
            output_content_identities: vec![],
            run_policy: RunPolicy {
                retain_intermediates: false,
            },
            metric_set_id: None,
            error_summary: None,
            failed_stage: None,
        };
        insert_run_record(&conn, &run).unwrap();

        let fetched = get_run_record(&conn, run_id).unwrap().unwrap();
        assert_eq!(fetched.status, RunStatus::Running);

        let metric_set_id = Uuid::new_v4();
        insert_metric_set(
            &conn,
            &MetricSet {
                id: metric_set_id,
                run_record_id: run_id,
                dice: None,
                area_mm2: None,
                foreground_pixels: None,
                connected_components: None,
                step_timings: Default::default(),
            },
        )
        .unwrap();
        finalize_run_success(
            &conn,
            run_id,
            &Utc::now().to_rfc3339(),
            &["out-1".to_string()],
            metric_set_id,
        )
        .unwrap();

        let fetched = get_run_record(&conn, run_id).unwrap().unwrap();
        assert_eq!(fetched.status, RunStatus::Succeeded);
        assert_eq!(fetched.output_content_identities, vec!["out-1".to_string()]);
        assert_eq!(fetched.metric_set_id, Some(metric_set_id));
    }

    #[test]
    fn recover_interrupted_runs_marks_running_rows_failed_never_succeeded() {
        let (conn, _dataset_id, version_id, image_id) = setup();
        let snapshot = sample_snapshot();
        insert_pipeline_snapshot(&conn, &snapshot).unwrap();
        let input_artifact = RunInputArtifact {
            id: Uuid::new_v4(),
            content_identity: "imp-1".into(),
            source_image_asset_id: image_id,
            created_at: Utc::now().to_rfc3339(),
        };
        insert_run_input_artifact(&conn, &input_artifact).unwrap();
        let run_id = Uuid::new_v4();
        insert_run_record(
            &conn,
            &RunRecord {
                id: run_id,
                status: RunStatus::Running,
                dataset_version_id: version_id,
                dataset_fingerprint: "fp-1".into(),
                image_asset_id: image_id,
                image_asset_identity: "imp-1".into(),
                run_input_artifact_id: input_artifact.id,
                pipeline_snapshot_id: snapshot.id,
                target_node_id: "n1".into(),
                seed: 1,
                node_versions: Default::default(),
                started_at: Utc::now().to_rfc3339(),
                ended_at: None,
                output_content_identities: vec![],
                run_policy: RunPolicy {
                    retain_intermediates: false,
                },
                metric_set_id: None,
                error_summary: None,
                failed_stage: None,
            },
        )
        .unwrap();

        let recovered = recover_interrupted_runs(&conn).unwrap();
        assert_eq!(recovered, 1);

        let fetched = get_run_record(&conn, run_id).unwrap().unwrap();
        assert_eq!(fetched.status, RunStatus::Failed);
        assert!(fetched.metric_set_id.is_none());
        assert!(fetched.error_summary.is_some());
    }
}

//! Persistence for the housekeeping commands (`POST /cache/clear`,
//! `DELETE /dataset-versions/{id}`, `DELETE /runs/{id}` — FR-029/FR-030).
//! Deletes never issue UPDATEs against immutable rows: they either remove a
//! whole entity with everything that would otherwise dangle, or refuse.

use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

/// One thing a removal would invalidate, as reported by `dry_run`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Affected {
    pub kind: &'static str,
    pub id: Uuid,
}

fn affected(kind: &'static str, ids: Vec<String>) -> Vec<Affected> {
    ids.into_iter()
        .map(|id| Affected {
            kind,
            id: Uuid::parse_str(&id).expect("stored UUID is always valid"),
        })
        .collect()
}

fn column(conn: &Connection, sql: &str, args: &[&str]) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
    rows.collect()
}

/// Whether any *official* row (asset, mask, Run input/output) still points
/// at `identity`. Thumbnails are cache, so they deliberately don't count.
pub fn content_is_official(conn: &Connection, identity: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM image_assets WHERE imported_content_identity = ?1)
             OR EXISTS(SELECT 1 FROM reference_masks WHERE content_identity = ?1)
             OR EXISTS(SELECT 1 FROM run_input_artifacts WHERE content_identity = ?1)
             OR EXISTS(SELECT 1 FROM run_records WHERE output_content_identities_json LIKE ?2)",
        params![identity, format!("%\"{identity}\"%")],
        |r| r.get(0),
    )
}

pub fn content_is_thumbnail(conn: &Connection, identity: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM thumbnail_artifacts WHERE content_identity = ?1)",
        [identity],
        |r| r.get(0),
    )
}

/// Drops every thumbnail row. Returns the affected image assets and the
/// thumbnail blob identities that were referenced.
pub fn clear_thumbnails(conn: &Connection) -> rusqlite::Result<(Vec<Uuid>, Vec<String>)> {
    let images = column(
        conn,
        "SELECT id FROM image_assets WHERE imported_content_identity IN
             (SELECT source_content_identity FROM thumbnail_artifacts)",
        &[],
    )?;
    let blobs = column(
        conn,
        "SELECT content_identity FROM thumbnail_artifacts WHERE content_identity IS NOT NULL",
        &[],
    )?;
    conn.execute("DELETE FROM thumbnail_artifacts", [])?;
    Ok((
        images
            .into_iter()
            .map(|s| Uuid::parse_str(&s).expect("stored UUID is always valid"))
            .collect(),
        blobs,
    ))
}

// ------------------------------------------------------------------ runs

pub fn run_status(conn: &Connection, run_id: Uuid) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT status FROM run_records WHERE id = ?1",
        [run_id.to_string()],
        |r| r.get(0),
    )
    .optional()
}

pub fn run_impact(conn: &Connection, run_id: Uuid) -> rusqlite::Result<Vec<Affected>> {
    Ok(affected(
        "metric_set",
        column(
            conn,
            "SELECT id FROM metric_sets WHERE run_record_id = ?1",
            &[&run_id.to_string()],
        )?,
    ))
}

/// Removes one Run and everything only it owned. Returns blob identities
/// that may now be unreferenced (caller decides, post-commit).
pub fn delete_run(tx: &Transaction, run_id: Uuid) -> rusqlite::Result<Vec<String>> {
    let id = run_id.to_string();
    let (input_id, snapshot_id, outputs_json): (String, String, String) = tx.query_row(
        "SELECT run_input_artifact_id, pipeline_snapshot_id, output_content_identities_json
         FROM run_records WHERE id = ?1",
        [&id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut candidates: Vec<String> = serde_json::from_str(&outputs_json).unwrap_or_default();
    if let Some(identity) = tx
        .query_row(
            "SELECT content_identity FROM run_input_artifacts WHERE id = ?1",
            [&input_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        candidates.push(identity);
    }

    tx.execute("DELETE FROM metric_sets WHERE run_record_id = ?1", [&id])?;
    tx.execute("DELETE FROM run_records WHERE id = ?1", [&id])?;
    tx.execute(
        "DELETE FROM run_input_artifacts WHERE id = ?1
           AND NOT EXISTS (SELECT 1 FROM run_records WHERE run_input_artifact_id = ?1)",
        [&input_id],
    )?;
    tx.execute(
        "DELETE FROM pipeline_snapshots WHERE id = ?1
           AND NOT EXISTS (SELECT 1 FROM run_records WHERE pipeline_snapshot_id = ?1)",
        [&snapshot_id],
    )?;
    Ok(candidates)
}

// -------------------------------------------------------- dataset versions

pub struct VersionImpact {
    pub runs: Vec<String>,
    pub derived_versions: Vec<String>,
    pub exclusive_images: Vec<String>,
}

impl VersionImpact {
    pub fn would_invalidate(&self) -> Vec<Affected> {
        let mut out = affected("run", self.runs.clone());
        out.extend(affected("image_asset", self.exclusive_images.clone()));
        out
    }

    pub fn blocked_by(&self) -> Vec<Affected> {
        affected("derived_dataset_version", self.derived_versions.clone())
    }
}

pub fn version_exists(conn: &Connection, id: Uuid) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM dataset_versions WHERE id = ?1)",
        [id.to_string()],
        |r| r.get(0),
    )
}

pub fn version_impact(conn: &Connection, version_id: Uuid) -> rusqlite::Result<VersionImpact> {
    let id = version_id.to_string();
    let runs = column(
        conn,
        "SELECT id FROM run_records WHERE dataset_version_id = ?1",
        &[&id],
    )?;
    let derived_versions = column(
        conn,
        "SELECT id FROM dataset_versions WHERE derived_from_version_id = ?1",
        &[&id],
    )?;
    // Assets that live only in this version and that no Run outside it uses.
    let exclusive_images = column(
        conn,
        "SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1
           AND image_asset_id NOT IN (SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id != ?1)
           AND image_asset_id NOT IN (SELECT image_asset_id FROM run_records WHERE dataset_version_id != ?1)",
        &[&id],
    )?;
    Ok(VersionImpact {
        runs,
        derived_versions,
        exclusive_images,
    })
}

/// Removes a Dataset Version, its Runs, and exclusively-owned images.
/// Returns (candidate blob identities, thumbnail-cache source identities
/// to drop). The caller must have refused already if `derived_versions`
/// is non-empty.
pub fn delete_version(
    tx: &Transaction,
    version_id: Uuid,
    impact: &VersionImpact,
) -> rusqlite::Result<Vec<String>> {
    let id = version_id.to_string();
    let dataset_id: String = tx.query_row(
        "SELECT dataset_id FROM dataset_versions WHERE id = ?1",
        [&id],
        |r| r.get(0),
    )?;

    let mut candidates = BTreeSet::new();
    for run in &impact.runs {
        candidates.extend(delete_run(tx, Uuid::parse_str(run).expect("stored UUID"))?);
    }

    tx.execute("DELETE FROM validation_findings WHERE dataset_version_id = ?1", [&id])?;
    tx.execute("DELETE FROM dataset_version_images WHERE dataset_version_id = ?1", [&id])?;
    tx.execute("DELETE FROM dataset_versions WHERE id = ?1", [&id])?;

    for image in &impact.exclusive_images {
        // A Run for another version may have pointed here after all;
        // never leave it dangling.
        let still_used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_records WHERE image_asset_id = ?1)
                 OR EXISTS(SELECT 1 FROM dataset_version_images WHERE image_asset_id = ?1)",
            [image],
            |r| r.get(0),
        )?;
        if still_used {
            continue;
        }
        let (imported, mask_id): (String, Option<String>) = tx.query_row(
            "SELECT imported_content_identity, reference_mask_id FROM image_assets WHERE id = ?1",
            [image],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        candidates.insert(imported.clone());
        tx.execute("DELETE FROM image_assets WHERE id = ?1", [image])?;
        if let Some(mask_id) = mask_id {
            if let Some(mask_identity) = tx
                .query_row(
                    "SELECT content_identity FROM reference_masks WHERE id = ?1",
                    [&mask_id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            {
                candidates.insert(mask_identity);
            }
            tx.execute("DELETE FROM reference_masks WHERE id = ?1", [&mask_id])?;
        }
        // The thumbnail cache is keyed by imported content identity.
        tx.execute(
            "DELETE FROM thumbnail_artifacts WHERE source_content_identity = ?1
               AND NOT EXISTS (SELECT 1 FROM image_assets WHERE imported_content_identity = ?1)",
            [&imported],
        )?;
    }

    tx.execute(
        "DELETE FROM research_subjects WHERE dataset_id = ?1 AND deidentified_patient_id NOT IN (
             SELECT patient_id FROM image_assets WHERE patient_id IS NOT NULL AND id IN (
                 SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id IN
                     (SELECT id FROM dataset_versions WHERE dataset_id = ?1)))",
        [&dataset_id],
    )?;
    tx.execute(
        "UPDATE datasets SET latest_version_id = (
             SELECT id FROM dataset_versions WHERE dataset_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT 1)
         WHERE id = ?1 AND latest_version_id = ?2",
        params![dataset_id, id],
    )?;
    Ok(candidates.into_iter().filter(|c| !c.is_empty()).collect())
}

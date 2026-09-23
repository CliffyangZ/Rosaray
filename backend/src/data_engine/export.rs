//! Export Bundle build and import validation (User Story 5, FR-026/FR-027/
//! FR-034/FR-035). A bundle is one encrypted container (see
//! `crypto::export_key`) whose plaintext is:
//!
//! ```text
//! u64 LE json_len | json(BundleBody) | blob bytes, concatenated in `content_list` order
//! ```
//!
//! Relational rows travel as generic table dumps so the bundle round-trips
//! exactly what the schema holds — fingerprints, identities and metrics are
//! copied, never recomputed. Preview/thumbnail data is excluded by
//! construction: `thumbnail_artifacts` is not in `TABLES` and blobs are
//! collected only from official (dataset/run) references.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::types::{Value, ValueRef};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Map;
use uuid::Uuid;

use crate::crypto::export_key::{self, BundleCryptoError};
use crate::data_repository::blob_store::{BlobStore, BlobStoreError};
use crate::data_repository::sqlite::CONTRACT_VERSION;
use crate::domain::content_identity::content_identity;
use crate::domain::ServiceError;

pub type Row = Map<String, serde_json::Value>;

/// Tables carried by a bundle, in FK-safe insertion order. `projects` is
/// deliberately absent: rows are re-parented onto the target project.
const TABLES: &[&str] = &[
    "datasets",
    "research_subjects",
    "dataset_versions",
    "reference_masks",
    "image_assets",
    "dataset_version_images",
    "validation_findings",
    "pipeline_snapshots",
    "run_input_artifacts",
    "run_records",
    "metric_sets",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentEntry {
    pub content_identity: String,
    pub size_bytes: u64,
}

/// `ExportBundle.manifest` (data-model.md, FR-026).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub contract_version: String,
    pub dataset_version_ids: Vec<Uuid>,
    pub run_record_ids: Vec<Uuid>,
    pub content_list: Vec<ContentEntry>,
    pub integrity_proof: String,
    pub excludes_preview: bool,
}

#[derive(Serialize, Deserialize)]
struct BundleBody {
    manifest: Manifest,
    tables: BTreeMap<String, Vec<Row>>,
}

pub struct BuiltBundle {
    pub manifest: Manifest,
    pub key_id: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ImportSummary {
    pub dataset_version_ids: Vec<Uuid>,
    pub run_record_ids: Vec<Uuid>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Service(#[from] ServiceError),
    #[error("storage failure: {0}")]
    Storage(String),
}

fn storage(e: impl std::fmt::Display) -> ExportError {
    ExportError::Storage(e.to_string())
}

impl From<ExportError> for ServiceError {
    fn from(e: ExportError) -> Self {
        match e {
            ExportError::Service(s) => s,
            ExportError::Storage(_) => ServiceError::ServiceUnavailable,
        }
    }
}

// ---------------------------------------------------------------- export

pub fn build_bundle(
    conn: &Connection,
    blobs: &BlobStore,
    master: &crate::crypto::MasterKey,
    dataset_version_ids: &[Uuid],
    run_record_ids: &[Uuid],
    credential: &str,
) -> Result<BuiltBundle, ExportError> {
    if credential.is_empty() {
        return Err(ServiceError::CredentialRequired.into());
    }
    if dataset_version_ids.is_empty() && run_record_ids.is_empty() {
        return Err(ServiceError::NotFound.into());
    }

    // Resolve the closure: selected runs pull in their dataset versions, and
    // every version pulls in its derivation ancestors (FK integrity).
    let mut version_ids: BTreeSet<String> =
        dataset_version_ids.iter().map(|u| u.to_string()).collect();
    let mut run_ids: BTreeSet<String> = BTreeSet::new();
    for run_id in run_record_ids {
        let version: Option<String> = conn
            .query_row(
                "SELECT dataset_version_id FROM run_records WHERE id = ?1",
                [run_id.to_string()],
                |r| r.get(0),
            )
            .ok();
        let version = version.ok_or(ServiceError::NotFound)?;
        run_ids.insert(run_id.to_string());
        version_ids.insert(version);
    }
    let mut frontier: Vec<String> = version_ids.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        let parent: Option<Option<String>> = conn
            .query_row(
                "SELECT derived_from_version_id FROM dataset_versions WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .ok();
        match parent {
            None => return Err(ServiceError::NotFound.into()),
            Some(Some(p)) => {
                if version_ids.insert(p.clone()) {
                    frontier.push(p);
                }
            }
            Some(None) => {}
        }
    }

    let mut tables: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    let mut add = |table: &str, sql: &str, id: &str| -> Result<(), ExportError> {
        let rows = dump_rows(conn, sql, params![id]).map_err(storage)?;
        tables.entry(table.to_string()).or_default().extend(rows);
        Ok(())
    };
    for v in &version_ids {
        add("dataset_versions", "SELECT * FROM dataset_versions WHERE id = ?1", v)?;
        add(
            "datasets",
            "SELECT * FROM datasets WHERE id = (SELECT dataset_id FROM dataset_versions WHERE id = ?1)",
            v,
        )?;
        add(
            "dataset_version_images",
            "SELECT * FROM dataset_version_images WHERE dataset_version_id = ?1",
            v,
        )?;
        add(
            "image_assets",
            "SELECT * FROM image_assets WHERE id IN (SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1)",
            v,
        )?;
        add(
            "validation_findings",
            "SELECT * FROM validation_findings WHERE dataset_version_id = ?1",
            v,
        )?;
        add(
            "research_subjects",
            "SELECT * FROM research_subjects WHERE dataset_id = (SELECT dataset_id FROM dataset_versions WHERE id = ?1)
               AND deidentified_patient_id IN (SELECT patient_id FROM image_assets WHERE id IN
                   (SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1))",
            v,
        )?;
        add(
            "reference_masks",
            "SELECT * FROM reference_masks WHERE id IN (SELECT reference_mask_id FROM image_assets WHERE id IN
                (SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1))",
            v,
        )?;
    }
    // Every version's official runs are opt-in: only explicitly selected runs travel.
    for r in &run_ids {
        add("run_records", "SELECT * FROM run_records WHERE id = ?1", r)?;
        add(
            "pipeline_snapshots",
            "SELECT * FROM pipeline_snapshots WHERE id = (SELECT pipeline_snapshot_id FROM run_records WHERE id = ?1)",
            r,
        )?;
        add(
            "run_input_artifacts",
            "SELECT * FROM run_input_artifacts WHERE id = (SELECT run_input_artifact_id FROM run_records WHERE id = ?1)",
            r,
        )?;
        add(
            "metric_sets",
            "SELECT * FROM metric_sets WHERE run_record_id = ?1",
            r,
        )?;
    }
    for rows in tables.values_mut() {
        dedupe_by_key(rows);
    }
    // Reproducible ordering so identical selections yield identical proofs.
    for rows in tables.values_mut() {
        rows.sort_by_key(|r| format!("{:?}", r));
    }

    let identities = collect_content_identities(&tables);
    let mut content_list = Vec::new();
    let mut blob_bytes: Vec<Vec<u8>> = Vec::new();
    for identity in &identities {
        let bytes = blobs.read(master, identity).map_err(|e| match e {
            BlobStoreError::NotFound => ExportError::Service(ServiceError::StaleReference),
            BlobStoreError::Integrity => ExportError::Service(ServiceError::StaleReference),
            other => storage(other),
        })?;
        content_list.push(ContentEntry {
            content_identity: identity.clone(),
            size_bytes: bytes.len() as u64,
        });
        blob_bytes.push(bytes);
    }

    let tables_json = serde_json::to_vec(&tables).map_err(storage)?;
    let manifest = Manifest {
        contract_version: CONTRACT_VERSION.to_string(),
        dataset_version_ids: version_ids
            .iter()
            .map(|s| Uuid::parse_str(s).expect("stored UUID"))
            .collect(),
        run_record_ids: run_ids
            .iter()
            .map(|s| Uuid::parse_str(s).expect("stored UUID"))
            .collect(),
        integrity_proof: integrity_proof(&tables_json, &identities),
        content_list,
        excludes_preview: true,
    };
    let body = BundleBody {
        manifest: manifest.clone(),
        tables,
    };
    let body_json = serde_json::to_vec(&body).map_err(storage)?;

    let mut plaintext =
        Vec::with_capacity(8 + body_json.len() + blob_bytes.iter().map(Vec::len).sum::<usize>());
    plaintext.extend_from_slice(&(body_json.len() as u64).to_le_bytes());
    plaintext.extend_from_slice(&body_json);
    for b in &blob_bytes {
        plaintext.extend_from_slice(b);
    }

    let (bytes, key_id) = export_key::seal(credential, &plaintext).map_err(storage)?;
    Ok(BuiltBundle {
        manifest,
        key_id,
        bytes,
    })
}

fn integrity_proof(tables_json: &[u8], identities: &BTreeSet<String>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(tables_json.len() as u64).to_le_bytes());
    hasher.update(tables_json);
    for id in identities {
        hasher.update(id.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// Every non-empty blob identity official rows point at. Thumbnails are not
/// in `TABLES`, so they can never appear here (FR-026).
fn collect_content_identities(tables: &BTreeMap<String, Vec<Row>>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut take = |table: &str, col: &str| {
        for row in tables.get(table).into_iter().flatten() {
            if let Some(serde_json::Value::String(s)) = row.get(col) {
                if !s.is_empty() {
                    out.insert(s.clone());
                }
            }
        }
    };
    take("image_assets", "imported_content_identity");
    take("reference_masks", "content_identity");
    take("run_input_artifacts", "content_identity");
    for row in tables.get("run_records").into_iter().flatten() {
        if let Some(serde_json::Value::String(s)) = row.get("output_content_identities_json") {
            if let Ok(ids) = serde_json::from_str::<Vec<String>>(s) {
                out.extend(ids.into_iter().filter(|i| !i.is_empty()));
            }
        }
    }
    out
}

fn dedupe_by_key(rows: &mut Vec<Row>) {
    let mut seen = BTreeSet::new();
    rows.retain(|r| seen.insert(format!("{:?}", r)));
}

fn dump_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare(sql)?;
    let cols: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let rows = stmt.query_map(params, |r| {
        let mut map = Row::new();
        for (i, name) in cols.iter().enumerate() {
            let v = match r.get_ref(i)? {
                ValueRef::Null => serde_json::Value::Null,
                ValueRef::Integer(n) => n.into(),
                ValueRef::Real(f) => serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                ValueRef::Text(t) => {
                    serde_json::Value::String(String::from_utf8_lossy(t).into_owned())
                }
                ValueRef::Blob(_) => serde_json::Value::Null,
            };
            map.insert(name.clone(), v);
        }
        Ok(map)
    })?;
    rows.collect()
}

// ---------------------------------------------------------------- import

/// Full import pipeline. Every check that can reject runs before the first
/// write to the target project: container integrity → credential →
/// content associations → data-contract compatibility → apply (FR-027).
pub fn import_bundle(
    conn: &mut Connection,
    blobs: &BlobStore,
    master: &crate::crypto::MasterKey,
    project_id: Uuid,
    credential: &str,
    container: &[u8],
) -> Result<ImportSummary, ExportError> {
    if credential.is_empty() {
        return Err(ServiceError::CredentialRequired.into());
    }
    let plaintext = export_key::open(credential, container).map_err(|e| match e {
        BundleCryptoError::Credential => ServiceError::CredentialInvalid,
        BundleCryptoError::Malformed | BundleCryptoError::Integrity => ServiceError::BundleTampered,
        BundleCryptoError::Encrypt => ServiceError::ServiceUnavailable,
    })?;

    let tampered = || ExportError::Service(ServiceError::BundleTampered);
    if plaintext.len() < 8 {
        return Err(tampered());
    }
    let json_len = u64::from_le_bytes(plaintext[..8].try_into().unwrap()) as usize;
    let json_end = 8usize.checked_add(json_len).ok_or_else(tampered)?;
    if json_end > plaintext.len() {
        return Err(tampered());
    }
    let body: BundleBody = serde_json::from_slice(&plaintext[8..json_end]).map_err(|_| tampered())?;
    let manifest = &body.manifest;

    // Content associations: blobs slice cleanly, each hashes to its listed
    // identity, the list matches what rows reference, and the proof holds.
    let mut blob_map: BTreeMap<&str, &[u8]> = BTreeMap::new();
    let mut at = json_end;
    for entry in &manifest.content_list {
        let end = at
            .checked_add(entry.size_bytes as usize)
            .filter(|e| *e <= plaintext.len())
            .ok_or_else(tampered)?;
        let bytes = &plaintext[at..end];
        if content_identity(bytes) != entry.content_identity {
            return Err(tampered());
        }
        blob_map.insert(entry.content_identity.as_str(), bytes);
        at = end;
    }
    if at != plaintext.len() {
        return Err(tampered());
    }
    let referenced = collect_content_identities(&body.tables);
    let listed: BTreeSet<String> = blob_map.keys().map(|s| s.to_string()).collect();
    if referenced != listed || !manifest.excludes_preview {
        return Err(tampered());
    }
    let tables_json = serde_json::to_vec(&body.tables).map_err(storage)?;
    if integrity_proof(&tables_json, &referenced) != manifest.integrity_proof {
        return Err(tampered());
    }
    for (table, rows) in &body.tables {
        if !TABLES.contains(&table.as_str()) {
            return Err(tampered());
        }
        let _ = rows;
    }
    // Relational associations: every manifest id present, every run's
    // dataset version and every version image present.
    let ids_in = |table: &str| -> BTreeSet<String> {
        body.tables
            .get(table)
            .into_iter()
            .flatten()
            .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    };
    let versions = ids_in("dataset_versions");
    let runs = ids_in("run_records");
    let assets = ids_in("image_assets");
    if !manifest
        .dataset_version_ids
        .iter()
        .all(|id| versions.contains(&id.to_string()))
        || !manifest
            .run_record_ids
            .iter()
            .all(|id| runs.contains(&id.to_string()))
    {
        return Err(tampered());
    }
    for r in body.tables.get("run_records").into_iter().flatten() {
        let ok = |k: &str, set: &BTreeSet<String>| {
            r.get(k).and_then(|v| v.as_str()).is_some_and(|s| set.contains(s))
        };
        if !ok("dataset_version_id", &versions) || !ok("image_asset_id", &assets) {
            return Err(tampered());
        }
    }
    for r in body.tables.get("dataset_version_images").into_iter().flatten() {
        let ok = |k: &str, set: &BTreeSet<String>| {
            r.get(k).and_then(|v| v.as_str()).is_some_and(|s| set.contains(s))
        };
        if !ok("dataset_version_id", &versions) || !ok("image_asset_id", &assets) {
            return Err(tampered());
        }
    }

    // Data-contract compatibility: same major version.
    let major = |v: &str| v.split('.').next().map(str::to_string);
    if major(&manifest.contract_version) != major(CONTRACT_VERSION)
        || manifest.contract_version.split('.').count() != 3
    {
        return Err(ServiceError::BundleIncompatible.into());
    }

    // ---- apply: blobs first (content-addressed, so orphans are harmless),
    // then one all-or-nothing transaction for the relational rows.
    for (identity, bytes) in &blob_map {
        let written = blobs.write(master, bytes).map_err(storage)?;
        debug_assert_eq!(&written, identity);
    }
    let tx = conn.transaction().map_err(storage)?;
    for table in TABLES {
        let Some(rows) = body.tables.get(*table) else { continue };
        let allowed = table_columns(&tx, table).map_err(storage)?;
        for row in rows {
            let mut row = row.clone();
            if *table == "datasets" {
                row.insert("project_id".into(), project_id.to_string().into());
                row.insert("latest_version_id".into(), serde_json::Value::Null);
            }
            insert_row(&tx, table, &allowed, &row)?;
        }
    }
    // Point each newly-created dataset at its newest version.
    tx.execute(
        "UPDATE datasets SET latest_version_id = (
             SELECT id FROM dataset_versions WHERE dataset_id = datasets.id
             ORDER BY created_at DESC, rowid DESC LIMIT 1)
         WHERE latest_version_id IS NULL",
        [],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;

    Ok(ImportSummary {
        dataset_version_ids: manifest.dataset_version_ids.clone(),
        run_record_ids: manifest.run_record_ids.clone(),
    })
}

fn table_columns(conn: &Connection, table: &str) -> rusqlite::Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt.query_map([], |r| r.get::<_, String>(1))?;
    cols.collect()
}

/// `INSERT OR IGNORE`: re-importing the same immutable rows is a no-op
/// rather than an overwrite (FR-032). Column names come from an
/// attacker-controllable payload, so each must exist in the real schema.
fn insert_row(
    conn: &Connection,
    table: &str,
    allowed: &BTreeSet<String>,
    row: &Row,
) -> Result<(), ExportError> {
    if row.is_empty() || row.keys().any(|k| !allowed.contains(k)) {
        return Err(ServiceError::BundleTampered.into());
    }
    let cols: Vec<&String> = row.keys().collect();
    let sql = format!(
        "INSERT OR IGNORE INTO {table} ({}) VALUES ({})",
        cols.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", "),
        (1..=cols.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", "),
    );
    let values: Vec<Value> = cols
        .iter()
        .map(|c| match &row[*c] {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Integer(*b as i64),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Real(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::String(s) => Value::Text(s.clone()),
            other => Value::Text(other.to_string()),
        })
        .collect();
    conn.execute(&sql, rusqlite::params_from_iter(values))
        .map_err(storage)?;
    Ok(())
}

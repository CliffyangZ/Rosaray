//! Query/persistence for Dataset/DatasetVersion/ImageAsset/ResearchSubject/
//! ReferenceMask (FR-051: business rules live in `data_engine`, this module
//! is storage only).

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::domain::dataset::{
    Dataset, DatasetVersion, Dimensions, ImageAsset, ImageAssetStatus, MaskValidity,
    MetadataStatus, ReferenceMask, Split, ValidationFinding, ValidationStatus, ValidationSummary,
};

pub fn get_or_create_dataset(
    conn: &Connection,
    project_id: Uuid,
    display_name: &str,
) -> rusqlite::Result<Dataset> {
    let existing = conn
        .query_row(
            "SELECT id, latest_version_id FROM datasets WHERE project_id = ?1 AND display_name = ?2",
            params![project_id.to_string(), display_name],
            |row| {
                let id: String = row.get(0)?;
                let latest: Option<String> = row.get(1)?;
                Ok((id, latest))
            },
        )
        .optional()?;

    if let Some((id, latest)) = existing {
        return Ok(Dataset {
            id: Uuid::parse_str(&id).expect("stored UUID is always valid"),
            project_id,
            display_name: display_name.to_string(),
            latest_version_id: latest.map(|v| Uuid::parse_str(&v).unwrap()),
        });
    }

    let id = Uuid::new_v4();
    conn.execute(
        "INSERT INTO datasets (id, project_id, display_name, latest_version_id) VALUES (?1, ?2, ?3, NULL)",
        params![id.to_string(), project_id.to_string(), display_name],
    )?;
    Ok(Dataset {
        id,
        project_id,
        display_name: display_name.to_string(),
        latest_version_id: None,
    })
}

pub fn get_latest_dataset_version(
    conn: &Connection,
    dataset_id: Uuid,
) -> rusqlite::Result<Option<DatasetVersion>> {
    let latest_id: Option<String> = conn
        .query_row(
            "SELECT latest_version_id FROM datasets WHERE id = ?1",
            params![dataset_id.to_string()],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    match latest_id {
        Some(id) => get_dataset_version(conn, Uuid::parse_str(&id).unwrap()),
        None => Ok(None),
    }
}

pub fn get_dataset_version(
    conn: &Connection,
    version_id: Uuid,
) -> rusqlite::Result<Option<DatasetVersion>> {
    let row = conn
        .query_row(
            "SELECT dataset_id, fingerprint, derived_from_version_id, created_at, validation_status
             FROM dataset_versions WHERE id = ?1",
            params![version_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;

    let Some((dataset_id, fingerprint, derived_from, created_at, validation_status)) = row else {
        return Ok(None);
    };

    let mut stmt = conn.prepare(
        "SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1",
    )?;
    let image_asset_ids: Vec<Uuid> = stmt
        .query_map(params![version_id.to_string()], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .map(|s| Uuid::parse_str(&s).unwrap())
        .collect();

    let mut stmt =
        conn.prepare("SELECT id FROM validation_findings WHERE dataset_version_id = ?1")?;
    let finding_ids: Vec<Uuid> = stmt
        .query_map(params![version_id.to_string()], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .map(|s| Uuid::parse_str(&s).unwrap())
        .collect();

    Ok(Some(DatasetVersion {
        id: version_id,
        dataset_id: Uuid::parse_str(&dataset_id).unwrap(),
        fingerprint,
        derived_from_version_id: derived_from.map(|s| Uuid::parse_str(&s).unwrap()),
        created_at,
        image_asset_ids,
        validation_summary: ValidationSummary {
            status: parse_validation_status(&validation_status),
            finding_ids,
        },
    }))
}

fn parse_validation_status(s: &str) -> ValidationStatus {
    match s {
        "blocked" => ValidationStatus::Blocked,
        "warned" => ValidationStatus::Warned,
        _ => ValidationStatus::Ok,
    }
}

fn validation_status_str(status: ValidationStatus) -> &'static str {
    match status {
        ValidationStatus::Ok => "ok",
        ValidationStatus::Blocked => "blocked",
        ValidationStatus::Warned => "warned",
    }
}

/// Persists a new immutable `DatasetVersion` row, its image membership
/// snapshot, and updates the parent `Dataset.latest_version_id` — callers
/// run this inside the same transaction as the `ImageAsset` inserts it
/// depends on (FR-044: atomic confirm).
pub fn insert_dataset_version(conn: &Connection, version: &DatasetVersion) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO dataset_versions (id, dataset_id, fingerprint, derived_from_version_id, created_at, validation_status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            version.id.to_string(),
            version.dataset_id.to_string(),
            version.fingerprint,
            version.derived_from_version_id.map(|u| u.to_string()),
            version.created_at,
            validation_status_str(version.validation_summary.status),
        ],
    )?;
    for image_id in &version.image_asset_ids {
        conn.execute(
            "INSERT INTO dataset_version_images (dataset_version_id, image_asset_id) VALUES (?1, ?2)",
            params![version.id.to_string(), image_id.to_string()],
        )?;
    }
    conn.execute(
        "UPDATE datasets SET latest_version_id = ?1 WHERE id = ?2",
        params![version.id.to_string(), version.dataset_id.to_string()],
    )?;
    Ok(())
}

pub fn insert_image_asset(conn: &Connection, asset: &ImageAsset) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO image_assets (
            id, external_source_uri, source_content_identity, imported_content_identity,
            width, height, source_created_at, imported_at, status, patient_id, split,
            reference_mask_id, metadata_status
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            asset.id.to_string(),
            asset.external_source_uri,
            asset.source_content_identity,
            asset.imported_content_identity,
            asset.dimensions.width,
            asset.dimensions.height,
            asset.source_created_at,
            asset.imported_at,
            status_str(asset.status),
            asset.patient_id,
            asset.split.map(|s| s.as_str().to_string()),
            asset.reference_mask_id.map(|u| u.to_string()),
            metadata_status_str(asset.metadata_status),
        ],
    )?;
    Ok(())
}

fn status_str(status: ImageAssetStatus) -> &'static str {
    match status {
        ImageAssetStatus::Available => "available",
        ImageAssetStatus::SourceMissing => "source_missing",
        ImageAssetStatus::SourceChanged => "source_changed",
    }
}

fn metadata_status_str(status: MetadataStatus) -> &'static str {
    match status {
        MetadataStatus::Complete => "complete",
        MetadataStatus::Incomplete => "incomplete",
    }
}

pub fn image_asset_by_id(conn: &Connection, id: Uuid) -> rusqlite::Result<Option<ImageAsset>> {
    conn.query_row(
        "SELECT external_source_uri, source_content_identity, imported_content_identity,
                width, height, source_created_at, imported_at, status, patient_id, split,
                reference_mask_id, metadata_status
         FROM image_assets WHERE id = ?1",
        params![id.to_string()],
        |row| {
            Ok(ImageAsset {
                id,
                external_source_uri: row.get(0)?,
                source_content_identity: row.get(1)?,
                imported_content_identity: row.get(2)?,
                dimensions: Dimensions {
                    width: row.get(3)?,
                    height: row.get(4)?,
                },
                source_created_at: row.get(5)?,
                imported_at: row.get(6)?,
                status: parse_status(&row.get::<_, String>(7)?),
                patient_id: row.get(8)?,
                split: row
                    .get::<_, Option<String>>(9)?
                    .and_then(|s| Split::parse(&s)),
                reference_mask_id: row
                    .get::<_, Option<String>>(10)?
                    .map(|s| Uuid::parse_str(&s).unwrap()),
                metadata_status: parse_metadata_status(&row.get::<_, String>(11)?),
            })
        },
    )
    .optional()
}

pub fn image_assets_for_version(
    conn: &Connection,
    version_id: Uuid,
) -> rusqlite::Result<Vec<ImageAsset>> {
    let mut stmt = conn.prepare(
        "SELECT image_asset_id FROM dataset_version_images WHERE dataset_version_id = ?1",
    )?;
    let ids: Vec<Uuid> = stmt
        .query_map(params![version_id.to_string()], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .map(|s| Uuid::parse_str(&s).unwrap())
        .collect();
    let mut assets = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(asset) = image_asset_by_id(conn, id)? {
            assets.push(asset);
        }
    }
    Ok(assets)
}

fn parse_status(s: &str) -> ImageAssetStatus {
    match s {
        "source_missing" => ImageAssetStatus::SourceMissing,
        "source_changed" => ImageAssetStatus::SourceChanged,
        _ => ImageAssetStatus::Available,
    }
}

fn parse_metadata_status(s: &str) -> MetadataStatus {
    match s {
        "complete" => MetadataStatus::Complete,
        _ => MetadataStatus::Incomplete,
    }
}

pub fn insert_reference_mask(conn: &Connection, mask: &ReferenceMask) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO reference_masks (id, content_identity, width, height, compatible_with_image_id, validity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            mask.id.to_string(),
            mask.content_identity,
            mask.dimensions.width,
            mask.dimensions.height,
            mask.compatible_with_image_id.to_string(),
            mask_validity_str(mask.validity),
        ],
    )?;
    Ok(())
}

fn mask_validity_str(validity: MaskValidity) -> &'static str {
    match validity {
        MaskValidity::Valid => "valid",
        MaskValidity::IncompatibleDimensions => "incompatible_dimensions",
        MaskValidity::Unreadable => "unreadable",
    }
}

pub fn masks_for_version(
    conn: &Connection,
    version_id: Uuid,
) -> rusqlite::Result<Vec<ReferenceMask>> {
    let mut stmt = conn.prepare(
        "SELECT rm.id, rm.content_identity, rm.width, rm.height, rm.compatible_with_image_id, rm.validity
         FROM reference_masks rm
         JOIN image_assets ia ON ia.reference_mask_id = rm.id
         JOIN dataset_version_images dvi ON dvi.image_asset_id = ia.id
         WHERE dvi.dataset_version_id = ?1",
    )?;
    let masks = stmt
        .query_map(params![version_id.to_string()], |row| {
            Ok(ReferenceMask {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                content_identity: row.get(1)?,
                dimensions: Dimensions {
                    width: row.get(2)?,
                    height: row.get(3)?,
                },
                compatible_with_image_id: Uuid::parse_str(&row.get::<_, String>(4)?).unwrap(),
                validity: match row.get::<_, String>(5)?.as_str() {
                    "incompatible_dimensions" => MaskValidity::IncompatibleDimensions,
                    "unreadable" => MaskValidity::Unreadable,
                    _ => MaskValidity::Valid,
                },
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(masks)
}

pub fn ensure_research_subject(
    conn: &Connection,
    dataset_id: Uuid,
    deidentified_patient_id: &str,
) -> rusqlite::Result<Uuid> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM research_subjects WHERE dataset_id = ?1 AND deidentified_patient_id = ?2",
            params![dataset_id.to_string(), deidentified_patient_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(Uuid::parse_str(&id).unwrap());
    }
    let id = Uuid::new_v4();
    conn.execute(
        "INSERT INTO research_subjects (id, dataset_id, deidentified_patient_id) VALUES (?1, ?2, ?3)",
        params![id.to_string(), dataset_id.to_string(), deidentified_patient_id],
    )?;
    Ok(id)
}

pub fn insert_validation_finding(
    conn: &Connection,
    finding: &ValidationFinding,
) -> rusqlite::Result<()> {
    let affected = serde_json::to_string(&finding.affected_image_asset_ids).unwrap();
    conn.execute(
        "INSERT INTO validation_findings (id, dataset_version_id, category, severity, affected_image_asset_ids_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            finding.id.to_string(),
            finding.dataset_version_id.to_string(),
            category_str(finding.category),
            severity_str(finding.severity),
            affected,
        ],
    )?;
    Ok(())
}

fn category_str(c: crate::domain::dataset::FindingCategory) -> &'static str {
    use crate::domain::dataset::FindingCategory::*;
    match c {
        PatientSplitLeakage => "patient_split_leakage",
        MissingPatientId => "missing_patient_id",
        MissingOrIncompatibleMask => "missing_or_incompatible_mask",
        DuplicateContent => "duplicate_content",
        IncompleteMetadata => "incomplete_metadata",
    }
}

fn severity_str(s: crate::domain::dataset::Severity) -> &'static str {
    match s {
        crate::domain::dataset::Severity::Blocking => "blocking",
        crate::domain::dataset::Severity::Warning => "warning",
    }
}

/// All Datasets belonging to a project — used by Explorer startup discovery
/// (US2) so the frontend does not depend on having just completed an import
/// to learn a Dataset's identity.
pub fn list_datasets_for_project(
    conn: &Connection,
    project_id: Uuid,
) -> rusqlite::Result<Vec<Dataset>> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, latest_version_id FROM datasets WHERE project_id = ?1 ORDER BY display_name ASC",
    )?;
    let datasets = stmt
        .query_map(params![project_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .map(|(id, display_name, latest)| Dataset {
            id: Uuid::parse_str(&id).expect("stored UUID is always valid"),
            project_id,
            display_name,
            latest_version_id: latest.map(|v| Uuid::parse_str(&v).unwrap()),
        })
        .collect();
    Ok(datasets)
}

pub fn list_versions_for_dataset(
    conn: &Connection,
    dataset_id: Uuid,
) -> rusqlite::Result<Vec<DatasetVersion>> {
    let mut stmt = conn
        .prepare("SELECT id FROM dataset_versions WHERE dataset_id = ?1 ORDER BY created_at ASC")?;
    let ids: Vec<Uuid> = stmt
        .query_map(params![dataset_id.to_string()], |row| {
            row.get::<_, String>(0)
        })?
        .filter_map(|r| r.ok())
        .map(|s| Uuid::parse_str(&s).unwrap())
        .collect();
    let mut versions = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(v) = get_dataset_version(conn, id)? {
            versions.push(v);
        }
    }
    Ok(versions)
}

/// The most recently created `DatasetVersion` whose membership snapshot
/// includes this image — used to give `GET /image-assets/{id}/display` a
/// version context (data-model.md ImageDisplayDescriptor).
pub fn latest_version_containing_image(
    conn: &Connection,
    image_id: Uuid,
) -> rusqlite::Result<Option<Uuid>> {
    conn.query_row(
        "SELECT dvi.dataset_version_id FROM dataset_version_images dvi
         JOIN dataset_versions dv ON dv.id = dvi.dataset_version_id
         WHERE dvi.image_asset_id = ?1
         ORDER BY dv.created_at DESC LIMIT 1",
        params![image_id.to_string()],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|opt| opt.map(|s| Uuid::parse_str(&s).unwrap()))
}

pub fn reference_mask_by_id(
    conn: &Connection,
    id: Uuid,
) -> rusqlite::Result<Option<ReferenceMask>> {
    conn.query_row(
        "SELECT content_identity, width, height, compatible_with_image_id, validity FROM reference_masks WHERE id = ?1",
        params![id.to_string()],
        |row| {
            Ok(ReferenceMask {
                id,
                content_identity: row.get(0)?,
                dimensions: Dimensions {
                    width: row.get(1)?,
                    height: row.get(2)?,
                },
                compatible_with_image_id: Uuid::parse_str(&row.get::<_, String>(3)?).unwrap(),
                validity: match row.get::<_, String>(4)?.as_str() {
                    "incompatible_dimensions" => MaskValidity::IncompatibleDimensions,
                    "unreadable" => MaskValidity::Unreadable,
                    _ => MaskValidity::Valid,
                },
            })
        },
    )
    .optional()
}

/// All source and imported content identities already known to this
/// project — used for import-time duplicate detection (FR-006).
pub fn all_known_content_identities(
    conn: &Connection,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    let mut stmt = conn
        .prepare("SELECT source_content_identity, imported_content_identity FROM image_assets")?;
    let mut set = std::collections::HashSet::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        set.insert(row.get::<_, String>(0)?);
        set.insert(row.get::<_, String>(1)?);
    }
    Ok(set)
}

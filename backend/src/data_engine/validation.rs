//! Dataset Version validation findings (FR-008): patient/split leakage,
//! missing patient id, missing/incompatible reference mask, duplicate
//! content, and incomplete metadata.

use std::collections::HashMap;

use uuid::Uuid;

use crate::domain::content_identity::content_identity;
use crate::domain::dataset::{
    FindingCategory, ImageAsset, ImageAssetStatus, MaskValidity, ReferenceMask, Severity,
    ValidationFinding,
};

/// FR-031: re-checks the external source file before serving/using an
/// `ImageAsset` — both `source_missing` and `source_changed` MUST block new
/// Preview/official Run creation, but a prior `RunRecord` stays readable via
/// its own `RunInputArtifact` (that record is untouched by this check).
pub fn check_source_availability(asset: &ImageAsset) -> ImageAssetStatus {
    let Some(path) = asset.external_source_uri.strip_prefix("file://") else {
        return asset.status;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return ImageAssetStatus::SourceMissing;
    };
    if content_identity(&bytes) != asset.source_content_identity {
        return ImageAssetStatus::SourceChanged;
    }
    ImageAssetStatus::Available
}

pub fn validate_dataset_version(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
    masks: &[ReferenceMask],
) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();

    findings.extend(find_patient_split_leakage(dataset_version_id, images));
    findings.extend(find_missing_patient_id(dataset_version_id, images));
    findings.extend(find_missing_or_incompatible_mask(
        dataset_version_id,
        images,
        masks,
    ));
    findings.extend(find_duplicate_content(dataset_version_id, images));
    findings.extend(find_incomplete_metadata(dataset_version_id, images));

    findings
}

/// A `ResearchSubject`'s images must all share one `split` value; a
/// violation blocks the dataset from official-evaluation use (data-model.md
/// ResearchSubject validation rule, spec Acceptance Scenario US1-2).
fn find_patient_split_leakage(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
) -> Vec<ValidationFinding> {
    let mut by_patient: HashMap<&str, Vec<&ImageAsset>> = HashMap::new();
    for image in images {
        if let Some(patient_id) = image.patient_id.as_deref() {
            by_patient.entry(patient_id).or_default().push(image);
        }
    }

    let mut findings = Vec::new();
    for (_, group) in by_patient {
        let distinct_splits: std::collections::HashSet<_> =
            group.iter().filter_map(|i| i.split).collect();
        if distinct_splits.len() > 1 {
            findings.push(ValidationFinding {
                id: Uuid::new_v4(),
                dataset_version_id,
                category: FindingCategory::PatientSplitLeakage,
                severity: Severity::Blocking,
                affected_image_asset_ids: group.iter().map(|i| i.id).collect(),
            });
        }
    }
    findings
}

fn find_missing_patient_id(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
) -> Vec<ValidationFinding> {
    let affected: Vec<Uuid> = images
        .iter()
        .filter(|i| i.patient_id.is_none())
        .map(|i| i.id)
        .collect();
    if affected.is_empty() {
        return Vec::new();
    }
    vec![ValidationFinding {
        id: Uuid::new_v4(),
        dataset_version_id,
        category: FindingCategory::MissingPatientId,
        severity: Severity::Warning,
        affected_image_asset_ids: affected,
    }]
}

/// FR-038: metrics that need a reference mask must not be computed unless
/// `validity == valid` and the mask's `compatible_with_image_id` matches.
fn find_missing_or_incompatible_mask(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
    masks: &[ReferenceMask],
) -> Vec<ValidationFinding> {
    let mut affected = Vec::new();
    for image in images {
        let Some(mask_id) = image.reference_mask_id else {
            continue;
        };
        let mask = masks.iter().find(|m| m.id == mask_id);
        let is_valid = mask
            .map(|m| m.validity == MaskValidity::Valid && m.compatible_with_image_id == image.id)
            .unwrap_or(false);
        if !is_valid {
            affected.push(image.id);
        }
    }
    if affected.is_empty() {
        return Vec::new();
    }
    vec![ValidationFinding {
        id: Uuid::new_v4(),
        dataset_version_id,
        category: FindingCategory::MissingOrIncompatibleMask,
        severity: Severity::Warning,
        affected_image_asset_ids: affected,
    }]
}

/// Defensive re-check at the version level (import-time dedup already
/// excludes duplicates from confirmation, FR-006) — catches any content
/// collision that still ends up in one version's membership.
fn find_duplicate_content(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
) -> Vec<ValidationFinding> {
    let mut by_identity: HashMap<&str, Vec<Uuid>> = HashMap::new();
    for image in images {
        by_identity
            .entry(image.imported_content_identity.as_str())
            .or_default()
            .push(image.id);
    }
    let affected: Vec<Uuid> = by_identity
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flatten()
        .collect();
    if affected.is_empty() {
        return Vec::new();
    }
    vec![ValidationFinding {
        id: Uuid::new_v4(),
        dataset_version_id,
        category: FindingCategory::DuplicateContent,
        severity: Severity::Blocking,
        affected_image_asset_ids: affected,
    }]
}

fn find_incomplete_metadata(
    dataset_version_id: Uuid,
    images: &[ImageAsset],
) -> Vec<ValidationFinding> {
    use crate::domain::dataset::MetadataStatus;
    let affected: Vec<Uuid> = images
        .iter()
        .filter(|i| i.metadata_status == MetadataStatus::Incomplete)
        .map(|i| i.id)
        .collect();
    if affected.is_empty() {
        return Vec::new();
    }
    vec![ValidationFinding {
        id: Uuid::new_v4(),
        dataset_version_id,
        category: FindingCategory::IncompleteMetadata,
        severity: Severity::Warning,
        affected_image_asset_ids: affected,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dataset::{Dimensions, ImageAssetStatus, MetadataStatus, Split};

    fn image(id: Uuid, patient: Option<&str>, split: Option<Split>) -> ImageAsset {
        ImageAsset {
            id,
            external_source_uri: "file:///x.png".into(),
            source_content_identity: format!("src-{id}"),
            imported_content_identity: format!("imp-{id}"),
            dimensions: Dimensions {
                width: 10,
                height: 10,
            },
            source_created_at: None,
            imported_at: "2026-01-01T00:00:00Z".into(),
            status: ImageAssetStatus::Available,
            patient_id: patient.map(String::from),
            split,
            reference_mask_id: None,
            metadata_status: if patient.is_some() && split.is_some() {
                MetadataStatus::Complete
            } else {
                MetadataStatus::Incomplete
            },
            pixel_spacing_mm: None,
            spacing_source: None,
        }
    }

    #[test]
    fn detects_patient_split_leakage() {
        let a = image(Uuid::new_v4(), Some("P1"), Some(Split::Train));
        let b = image(Uuid::new_v4(), Some("P1"), Some(Split::Test));
        let findings = validate_dataset_version(Uuid::new_v4(), &[a, b], &[]);
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::PatientSplitLeakage));
    }

    #[test]
    fn no_leakage_when_same_patient_same_split() {
        let a = image(Uuid::new_v4(), Some("P1"), Some(Split::Train));
        let b = image(Uuid::new_v4(), Some("P1"), Some(Split::Train));
        let findings = validate_dataset_version(Uuid::new_v4(), &[a, b], &[]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::PatientSplitLeakage));
    }

    #[test]
    fn detects_source_missing_and_source_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("src.png");
        std::fs::write(&path, b"original bytes").unwrap();

        let mut asset = image(Uuid::new_v4(), Some("P1"), Some(Split::Train));
        asset.external_source_uri = format!("file://{}", path.display());
        asset.source_content_identity = content_identity(b"original bytes");
        assert_eq!(
            check_source_availability(&asset),
            ImageAssetStatus::Available
        );

        std::fs::write(&path, b"tampered bytes").unwrap();
        assert_eq!(
            check_source_availability(&asset),
            ImageAssetStatus::SourceChanged
        );

        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            check_source_availability(&asset),
            ImageAssetStatus::SourceMissing
        );
    }

    #[test]
    fn flags_missing_patient_id() {
        let a = image(Uuid::new_v4(), None, Some(Split::Train));
        let findings = validate_dataset_version(Uuid::new_v4(), &[a], &[]);
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::MissingPatientId));
    }
}

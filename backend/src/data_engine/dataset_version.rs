//! Dataset Version fingerprinting and derivation (FR-007, FR-032,
//! research.md §4). A fingerprint covers exactly the fields the spec says
//! affect research interpretation — image content, patient mapping, split,
//! and mask relationship — and nothing else, so a display-name-only change
//! never produces a new version.

use uuid::Uuid;

use crate::domain::dataset::ValidationSummary;
use crate::domain::dataset::{DatasetVersion, ImageAsset, ValidationStatus};

pub struct FingerprintInput {
    pub image_content_identity: String,
    pub patient_id: Option<String>,
    pub split: Option<String>,
    pub reference_mask_content_identity: Option<String>,
    /// Calibration is research-relevant (feature 002): a change yields a new
    /// Dataset Version. `None` adds nothing to the hash, so fingerprints of
    /// uncalibrated data are byte-identical to before (Constitution II).
    pub pixel_spacing_mm: Option<(f64, f64)>,
    pub spacing_source: Option<String>,
}

impl FingerprintInput {
    pub fn from_image_asset(asset: &ImageAsset, mask_content_identity: Option<String>) -> Self {
        Self {
            image_content_identity: asset.imported_content_identity.clone(),
            patient_id: asset.patient_id.clone(),
            split: asset.split.map(|s| s.as_str().to_string()),
            reference_mask_content_identity: mask_content_identity,
            pixel_spacing_mm: asset.pixel_spacing_mm.map(|p| (p.x, p.y)),
            spacing_source: asset.spacing_source.map(|s| s.as_str().to_string()),
        }
    }
}

/// BLAKE3 hash of the sorted tuple list — sorted so that member order
/// (which has no research meaning) never changes the fingerprint.
pub fn compute_fingerprint(mut inputs: Vec<FingerprintInput>) -> String {
    inputs.sort_by(|a, b| a.image_content_identity.cmp(&b.image_content_identity));

    let mut hasher = blake3::Hasher::new();
    for input in &inputs {
        hasher.update(input.image_content_identity.as_bytes());
        hasher.update(b"\0");
        hasher.update(input.patient_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"\0");
        hasher.update(input.split.as_deref().unwrap_or("").as_bytes());
        hasher.update(b"\0");
        hasher.update(
            input
                .reference_mask_content_identity
                .as_deref()
                .unwrap_or("")
                .as_bytes(),
        );
        if let Some((x, y)) = input.pixel_spacing_mm {
            hasher.update(b"\0spacing\0");
            hasher.update(&x.to_le_bytes());
            hasher.update(&y.to_le_bytes());
            hasher.update(input.spacing_source.as_deref().unwrap_or("").as_bytes());
        }
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// Builds the next `DatasetVersion` for a dataset. Returns `None` when the
/// computed fingerprint is unchanged from `previous`, so the caller can
/// skip writing a redundant version (FR-007's converse: a real content/
/// membership change always produces one).
pub fn derive_next_version(
    dataset_id: Uuid,
    previous: Option<&DatasetVersion>,
    image_asset_ids: Vec<Uuid>,
    fingerprint_inputs: Vec<FingerprintInput>,
    created_at: String,
) -> Option<DatasetVersion> {
    let fingerprint = compute_fingerprint(fingerprint_inputs);
    if let Some(prev) = previous {
        if prev.fingerprint == fingerprint {
            return None;
        }
    }
    Some(DatasetVersion {
        id: Uuid::new_v4(),
        dataset_id,
        fingerprint,
        derived_from_version_id: previous.map(|p| p.id),
        created_at,
        image_asset_ids,
        validation_summary: ValidationSummary {
            status: ValidationStatus::Ok,
            finding_ids: Vec::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(identity: &str, patient: &str, split: &str) -> FingerprintInput {
        FingerprintInput {
            image_content_identity: identity.into(),
            patient_id: Some(patient.into()),
            split: Some(split.into()),
            reference_mask_content_identity: None,
            pixel_spacing_mm: None,
            spacing_source: None,
        }
    }

    #[test]
    fn absent_spacing_leaves_existing_fingerprints_unchanged() {
        // The pre-feature-002 hash layout, reproduced by hand (T021).
        let fp = compute_fingerprint(vec![input("img1", "p1", "train")]);
        let mut legacy = blake3::Hasher::new();
        legacy.update(b"img1\0p1\0train\0\n");
        assert_eq!(fp, legacy.finalize().to_hex().to_string());
    }

    #[test]
    fn changing_spacing_changes_the_fingerprint() {
        let base = compute_fingerprint(vec![input("img1", "p1", "train")]);
        let with = |x: f64, y: f64, src: &str| {
            let mut i = input("img1", "p1", "train");
            i.pixel_spacing_mm = Some((x, y));
            i.spacing_source = Some(src.into());
            compute_fingerprint(vec![i])
        };
        let a = with(0.05, 0.05, "metadata");
        assert_ne!(a, base);
        assert_ne!(a, with(0.06, 0.05, "metadata"));
        assert_ne!(a, with(0.05, 0.05, "user_entered"));
        assert_eq!(a, with(0.05, 0.05, "metadata"));
    }

    #[test]
    fn order_does_not_affect_fingerprint() {
        let a = compute_fingerprint(vec![
            input("img1", "p1", "train"),
            input("img2", "p2", "test"),
        ]);
        let b = compute_fingerprint(vec![
            input("img2", "p2", "test"),
            input("img1", "p1", "train"),
        ]);
        assert_eq!(a, b);
    }

    #[test]
    fn content_change_changes_fingerprint() {
        let a = compute_fingerprint(vec![input("img1", "p1", "train")]);
        let b = compute_fingerprint(vec![input("img1-changed", "p1", "train")]);
        assert_ne!(a, b);
    }

    #[test]
    fn patient_or_split_change_changes_fingerprint() {
        let a = compute_fingerprint(vec![input("img1", "p1", "train")]);
        let b = compute_fingerprint(vec![input("img1", "p1", "test")]);
        assert_ne!(a, b);
    }

    #[test]
    fn unchanged_membership_produces_no_new_version() {
        let dataset_id = Uuid::new_v4();
        let inputs = vec![input("img1", "p1", "train")];
        let first = derive_next_version(
            dataset_id,
            None,
            vec![Uuid::new_v4()],
            inputs,
            "2026-01-01T00:00:00Z".into(),
        )
        .unwrap();

        let same_inputs = vec![input("img1", "p1", "train")];
        let second = derive_next_version(
            dataset_id,
            Some(&first),
            first.image_asset_ids.clone(),
            same_inputs,
            "2026-01-02T00:00:00Z".into(),
        );
        assert!(second.is_none());
    }
}

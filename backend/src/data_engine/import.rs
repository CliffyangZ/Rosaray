//! Import pipeline (User Story 1): bounded scan, format/readability
//! validation, duplicate detection, grayscale representation generation,
//! and Metadata Manifest parsing — all pure/testable against the
//! filesystem, with no HTTP or SQL dependency (data_engine vs.
//! data_repository split, FR-051).

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, ImageReader};
use thiserror::Error;

use crate::domain::content_identity::content_identity;
use crate::domain::dataset::{
    CandidateClassification, ImportCandidate, ManifestMatch, MetadataManifest,
    MetadataManifestEntry, Split,
};

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("io error scanning {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("unreadable image: {0}")]
    Unreadable(String),
    #[error("unsupported format: only PNG/JPEG are accepted")]
    Unsupported,
    #[error("manifest is not valid JSON: {0}")]
    InvalidManifest(String),
    #[error("manifest references source_ref '{0}' more than once")]
    DuplicateManifestMapping(String),
}

pub const DEFAULT_MAX_SCAN_FILES: usize = 10_000;
pub const DEFAULT_MAX_SCAN_DEPTH: usize = 32;

/// Bounded folder scan (FR-043, spec Edge Cases): skips hidden files/dirs,
/// never follows a symlink into a directory already visited (cycle guard
/// via canonicalized-path dedup), and stops at `max_files`/`max_depth`
/// rather than traversing unbounded. Per-entry permission errors are
/// skipped, not fatal to the whole scan.
pub fn scan_folder(
    root: &Path,
    max_files: usize,
    max_depth: usize,
) -> Result<Vec<PathBuf>, ImportError> {
    let mut found = Vec::new();
    let mut visited_dirs: HashSet<PathBuf> = HashSet::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if found.len() >= max_files || depth > max_depth {
            continue;
        }
        let canonical = match dir.canonicalize() {
            Ok(c) => c,
            Err(_) => continue, // permission error or dangling symlink: skip silently
        };
        if !visited_dirs.insert(canonical) {
            continue; // already visited this real directory — cycle guard
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if found.len() >= max_files {
                break;
            }
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue; // hidden file/dir
            }
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if file_type.is_dir() || file_type.is_symlink() {
                stack.push((path, depth + 1));
            } else if file_type.is_file() {
                found.push(path);
            }
        }
    }

    Ok(found)
}

/// FR-005: validates readability and format (PNG/JPEG only) before any
/// write. Returns the decoded image and its raw source bytes; a failure
/// here MUST NOT create any dataset/image row.
pub fn validate_readable(path: &Path) -> Result<(DynamicImage, Vec<u8>), ImportError> {
    let bytes = std::fs::read(path).map_err(|source| ImportError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let format = image::guess_format(&bytes).map_err(|_| ImportError::Unsupported)?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(ImportError::Unsupported);
    }
    let reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let image = reader
        .decode()
        .map_err(|e| ImportError::Unreadable(e.to_string()))?;
    Ok((image, bytes))
}

/// FR-002/research.md §4: the grayscale research representation, encoded
/// as PNG so it is itself losslessly reconstructible from the source at any
/// later point.
pub fn render_grayscale_png(image: &DynamicImage) -> Vec<u8> {
    let gray = image.to_luma8();
    let mut out = Vec::new();
    gray.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("encoding an in-memory grayscale buffer as PNG cannot fail");
    out
}

/// research.md §6: matched to candidates purely by `source_ref` — never
/// inferred from filename/folder (FR-046).
pub fn parse_manifest(json: &str) -> Result<MetadataManifest, ImportError> {
    let manifest: MetadataManifest =
        serde_json::from_str(json).map_err(|e| ImportError::InvalidManifest(e.to_string()))?;

    let mut seen = HashSet::new();
    for entry in &manifest.entries {
        if !seen.insert(entry.source_ref.clone()) {
            return Err(ImportError::DuplicateManifestMapping(
                entry.source_ref.clone(),
            ));
        }
    }
    Ok(manifest)
}

fn manifest_entry_for<'a>(
    manifest: Option<&'a MetadataManifest>,
    source_ref: &str,
) -> Option<&'a MetadataManifestEntry> {
    manifest.and_then(|m| m.entries.iter().find(|e| e.source_ref == source_ref))
}

/// One scanned candidate's raw material before classification.
pub struct ScannedFile {
    pub source_ref: String,
    pub path: PathBuf,
}

/// Classifies every scanned candidate against duplicate content already
/// present (FR-006) and the optional manifest (FR-045/FR-046), without
/// writing anything — the Import Batch Preview contract (FR-044: zero
/// writes until confirm).
pub fn classify_candidates(
    files: &[ScannedFile],
    existing_content_identities: &HashSet<String>,
    manifest: Option<&MetadataManifest>,
) -> Vec<ImportCandidate> {
    let mut seen_in_batch: HashSet<String> = HashSet::new();
    let mut candidates = Vec::with_capacity(files.len());

    for file in files {
        let entry = manifest_entry_for(manifest, &file.source_ref);
        let manifest_match = match (manifest, entry) {
            (Some(_), Some(_)) => ManifestMatch::Matched,
            (Some(_), None) => ManifestMatch::Unmatched,
            (None, _) => ManifestMatch::Unmatched,
        };

        let candidate = match validate_readable(&file.path) {
            Ok((_, bytes)) => {
                let identity = content_identity(&bytes);
                if existing_content_identities.contains(&identity)
                    || !seen_in_batch.insert(identity)
                {
                    ImportCandidate {
                        source_ref: file.source_ref.clone(),
                        classification: CandidateClassification::Duplicate,
                        resolved_patient_id: None,
                        resolved_split: None,
                        resolved_mask_ref: None,
                        manifest_match,
                        reason: Some(
                            "content-identical to an already-imported or in-batch image".into(),
                        ),
                    }
                } else {
                    let resolved_split = entry
                        .and_then(|e| e.split.as_deref())
                        .and_then(Split::parse);
                    let split_was_invalid = entry
                        .and_then(|e| e.split.as_deref())
                        .map(|s| Split::parse(s).is_none())
                        .unwrap_or(false);
                    ImportCandidate {
                        source_ref: file.source_ref.clone(),
                        classification: CandidateClassification::Importable,
                        resolved_patient_id: entry.and_then(|e| e.patient_id.clone()),
                        resolved_split,
                        resolved_mask_ref: entry.and_then(|e| e.reference_mask_ref.clone()),
                        manifest_match,
                        reason: split_was_invalid.then(|| {
                            "manifest split value is not one of train/validation/test".to_string()
                        }),
                    }
                }
            }
            Err(ImportError::Unsupported) => ImportCandidate {
                source_ref: file.source_ref.clone(),
                classification: CandidateClassification::Unsupported,
                resolved_patient_id: None,
                resolved_split: None,
                resolved_mask_ref: None,
                manifest_match,
                reason: Some("only PNG/JPEG are supported".into()),
            },
            Err(e) => ImportCandidate {
                source_ref: file.source_ref.clone(),
                classification: CandidateClassification::Unreadable,
                resolved_patient_id: None,
                resolved_split: None,
                resolved_mask_ref: None,
                manifest_match,
                reason: Some(e.to_string()),
            },
        };
        candidates.push(candidate);
    }

    candidates
}

/// Per-classification counts for the Import Batch Preview response.
pub fn candidate_counts(candidates: &[ImportCandidate]) -> HashMap<&'static str, usize> {
    let mut counts = HashMap::new();
    for c in candidates {
        let key = match c.classification {
            CandidateClassification::Importable => "importable",
            CandidateClassification::Skipped => "skipped",
            CandidateClassification::Duplicate => "duplicate",
            CandidateClassification::Unreadable => "unreadable",
            CandidateClassification::Unsupported => "unsupported",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_png(path: &Path, w: u32, h: u32) {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([10, 20, 30]));
        image::DynamicImage::ImageRgb8(img).save(path).unwrap();
    }

    #[test]
    fn scan_folder_finds_files_and_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        write_png(&dir.path().join("a.png"), 4, 4);
        std::fs::write(dir.path().join(".hidden.png"), b"junk").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        write_png(&sub.join("b.png"), 4, 4);

        let found =
            scan_folder(dir.path(), DEFAULT_MAX_SCAN_FILES, DEFAULT_MAX_SCAN_DEPTH).unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn scan_folder_respects_max_files_bound() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            write_png(&dir.path().join(format!("{i}.png")), 2, 2);
        }
        let found = scan_folder(dir.path(), 3, DEFAULT_MAX_SCAN_DEPTH).unwrap();
        assert!(found.len() <= 3);
    }

    #[test]
    fn validate_readable_rejects_non_image_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-an-image.png");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"not a real png").unwrap();
        assert!(matches!(
            validate_readable(&path),
            Err(ImportError::Unsupported)
        ));
    }

    #[test]
    fn duplicate_content_is_classified_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        write_png(&a, 4, 4);
        std::fs::copy(&a, &b).unwrap();

        let files = vec![
            ScannedFile {
                source_ref: "a.png".into(),
                path: a,
            },
            ScannedFile {
                source_ref: "b.png".into(),
                path: b,
            },
        ];
        let candidates = classify_candidates(&files, &HashSet::new(), None);
        assert_eq!(
            candidates[0].classification,
            CandidateClassification::Importable
        );
        assert_eq!(
            candidates[1].classification,
            CandidateClassification::Duplicate
        );
    }

    #[test]
    fn manifest_maps_patient_and_split_by_source_ref_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("weird_name_12.png");
        write_png(&a, 4, 4);

        let manifest = MetadataManifest {
            id: uuid::Uuid::new_v4(),
            manifest_version: "1.0".into(),
            entries: vec![MetadataManifestEntry {
                source_ref: "weird_name_12.png".into(),
                patient_id: Some("P001".into()),
                split: Some("train".into()),
                reference_mask_ref: None,
            }],
        };

        let files = vec![ScannedFile {
            source_ref: "weird_name_12.png".into(),
            path: a,
        }];
        let candidates = classify_candidates(&files, &HashSet::new(), Some(&manifest));
        assert_eq!(candidates[0].resolved_patient_id, Some("P001".into()));
        assert_eq!(candidates[0].resolved_split, Some(Split::Train));
        assert_eq!(candidates[0].manifest_match, ManifestMatch::Matched);
    }

    #[test]
    fn duplicate_manifest_mapping_is_rejected() {
        let json = serde_json::json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "manifest_version": "1.0",
            "entries": [
                {"source_ref": "a.png", "patient_id": "P1", "split": "train", "reference_mask_ref": null},
                {"source_ref": "a.png", "patient_id": "P2", "split": "test", "reference_mask_ref": null}
            ]
        })
        .to_string();
        assert!(matches!(
            parse_manifest(&json),
            Err(ImportError::DuplicateManifestMapping(_))
        ));
    }
}

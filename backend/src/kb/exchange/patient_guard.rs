//! Raw patient-data exclusion (FR-047 / FR-053, research §16). Deny-by-default,
//! applied on export *and* import. There is no parameter anywhere that can
//! disable a rule. Findings name the file and the rule — never file contents.
//!
//! Rules, in order:
//! 1. content hash equals a known Image Asset / Reference Mask identity → block
//! 2. DICOM/NIfTI, or identifier-type metadata (EXIF/PNG text) → block
//! 3. raster or table under `assets/` or `tests/` with no matching
//!    `review: non-patient` record → exclude (`patient_status_unresolved`)
//! Free text is not scanned for names (not reliably possible); the UI warns
//! at every free-text field instead.

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::designer::validate::finding::{BundleRef, Finding, Severity, Subject};
use crate::domain::content_identity::content_identity;
use crate::kb::bundle::model::parse_yaml;

pub const REVIEW_FILE: &str = "asset-review.yaml";

pub const CODE_BLOCKED: &str = "patient_data_blocked";
pub const CODE_UNRESOLVED: &str = "patient_status_unresolved";

/// Content identities the project already knows as research data.
#[derive(Debug, Clone, Default)]
pub struct KnownContent(pub HashSet<String>);

impl KnownContent {
    pub fn contains(&self, identity: &str) -> bool {
        self.0.contains(identity)
    }
}

/// Loads every Image Asset (source + imported) and Reference Mask identity.
pub fn known_content(conn: &Connection) -> rusqlite::Result<KnownContent> {
    let mut set = crate::data_repository::sqlite::dataset_repo::all_known_content_identities(conn)?;
    let mut stmt = conn.prepare("SELECT content_identity FROM reference_masks")?;
    for id in stmt.query_map([], |r| r.get::<_, String>(0))? {
        set.insert(id?);
    }
    Ok(KnownContent(set))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    KnownContentHash,
    DicomOrNifti,
    IdentifierMetadata,
    UnreviewedAsset,
}

impl Rule {
    pub fn describe(&self) -> &'static str {
        match self {
            Rule::KnownContentHash => "it is a copy of research data held in this project",
            Rule::DicomOrNifti => "it is a DICOM or NIfTI medical image file",
            Rule::IdentifierMetadata => "it carries identifier-type metadata",
            Rule::UnreviewedAsset => "no non-patient review is recorded for this image or table",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    /// Aborts the whole export/import.
    Blocked(Rule),
    /// The file is left out and reported.
    Excluded(Rule),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetReview {
    pub path: String,
    pub blake3: String,
    pub review: String,
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AssetReviewFile {
    #[serde(default)]
    pub reviews: Vec<AssetReview>,
}

pub fn parse_reviews(text: &str) -> AssetReviewFile {
    parse_yaml(text).unwrap_or_default()
}

const RASTER_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff", "webp"];
const TABLE_EXT: &[&str] = &["csv", "tsv", "xls", "xlsx", "parquet", "json_table"];

fn ext(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn is_raster(path: &str) -> bool {
    RASTER_EXT.contains(&ext(path).as_str())
}

fn is_table(path: &str) -> bool {
    TABLE_EXT.contains(&ext(path).as_str())
}

fn under_asset_dir(path: &str) -> bool {
    path.split('/').any(|seg| seg == "assets" || seg == "tests")
}

/// Judges one file. `reviews` are the bundle's `asset-review.yaml` records.
pub fn check_file(path: &str, bytes: &[u8], known: &KnownContent, reviews: &AssetReviewFile) -> Verdict {
    // Rule 1: byte-identical copy, or a raster whose grayscale research
    // representation is a known imported identity.
    if known.contains(&content_identity(bytes)) {
        return Verdict::Blocked(Rule::KnownContentHash);
    }
    if is_raster(path) {
        if let Ok(img) = image::load_from_memory(bytes) {
            let representation = crate::data_engine::import::render_grayscale_png(&img);
            if known.contains(&content_identity(&representation)) {
                return Verdict::Blocked(Rule::KnownContentHash);
            }
        }
    }
    // Rule 2
    if is_dicom_or_nifti(path, bytes) {
        return Verdict::Blocked(Rule::DicomOrNifti);
    }
    if has_identifier_metadata(path, bytes) {
        return Verdict::Blocked(Rule::IdentifierMetadata);
    }
    // Rule 3
    if under_asset_dir(path) && (is_raster(path) || is_table(path)) {
        let hash = format!("b3:{}", blake3::hash(bytes).to_hex());
        let bare = hash.trim_start_matches("b3:");
        let reviewed = reviews.reviews.iter().any(|r| {
            r.path == path && r.review == "non-patient" && r.blake3.trim_start_matches("b3:") == bare
        });
        if !reviewed {
            return Verdict::Excluded(Rule::UnreviewedAsset);
        }
    }
    Verdict::Allowed
}

fn is_dicom_or_nifti(path: &str, bytes: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".dcm") || lower.ends_with(".nii") || lower.ends_with(".nii.gz") {
        return true;
    }
    if bytes.len() > 132 && &bytes[128..132] == b"DICM" {
        return true;
    }
    // NIfTI-1: magic at offset 344; NIfTI-2: at offset 4.
    if bytes.len() > 348 && matches!(&bytes[344..347], b"n+1" | b"ni1") && bytes[347] == 0 {
        return true;
    }
    if bytes.len() > 8 && matches!(&bytes[4..7], b"n+2" | b"ni2") && bytes[7] == 0 {
        return true;
    }
    false
}

/// EXIF tags that identify a person, device or place.
const IDENTIFIER_EXIF_TAGS: &[u16] = &[
    0x010E, // ImageDescription
    0x013B, // Artist
    0x013C, // HostComputer
    0x8298, // Copyright
    0x8825, // GPS IFD pointer
    0x9286, // UserComment
    0xA420, // ImageUniqueID
    0xA430, // CameraOwnerName
    0xA431, // BodySerialNumber
    0xA435, // LensSerialNumber
];

const IDENTIFIER_PNG_KEYWORDS: &[&str] = &["author", "artist", "copyright", "description", "comment", "person", "patient", "name"];

fn has_identifier_metadata(path: &str, bytes: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return jpeg_exif(bytes).is_some_and(|tiff| tiff_has_identifier_tag(tiff));
    }
    if lower.ends_with(".tif") || lower.ends_with(".tiff") {
        return tiff_has_identifier_tag(bytes);
    }
    if lower.ends_with(".png") {
        return png_identifier_chunks(bytes);
    }
    false
}

/// The TIFF block inside a JPEG's APP1 `Exif` segment.
fn jpeg_exif(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF {
        let marker = bytes[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            break;
        }
        let seg = &bytes[i + 4..i + 2 + len];
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            return Some(&seg[6..]);
        }
        i += 2 + len;
    }
    None
}

fn tiff_has_identifier_tag(t: &[u8]) -> bool {
    if t.len() < 8 {
        return false;
    }
    let le = match &t[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return false,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
    };
    if u16_at(2) != Some(42) {
        return false;
    }
    let mut pending = vec![u32_at(4).unwrap_or(0) as usize];
    let mut visited = 0;
    while let Some(ifd) = pending.pop() {
        visited += 1;
        if visited > 8 {
            break;
        }
        let Some(n) = u16_at(ifd) else { continue };
        for k in 0..n as usize {
            let entry = ifd + 2 + k * 12;
            let Some(tag) = u16_at(entry) else { break };
            if IDENTIFIER_EXIF_TAGS.contains(&tag) {
                return true;
            }
            if tag == 0x8769 {
                // Exif sub-IFD
                if let Some(off) = u32_at(entry + 8) {
                    pending.push(off as usize);
                }
            }
        }
    }
    false
}

fn png_identifier_chunks(bytes: &[u8]) -> bool {
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return false;
    }
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
        let kind = &bytes[i + 4..i + 8];
        let data_start = i + 8;
        let Some(data) = bytes.get(data_start..data_start + len) else { break };
        match kind {
            b"eXIf" if tiff_has_identifier_tag(data) => return true,
            b"tEXt" | b"iTXt" | b"zTXt" => {
                let key_end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
                let key = String::from_utf8_lossy(&data[..key_end]).to_ascii_lowercase();
                if IDENTIFIER_PNG_KEYWORDS.contains(&key.as_str()) {
                    return true;
                }
            }
            _ => {}
        }
        i = data_start + len + 4;
    }
    false
}

/// A finding for a guarded file: names the file and the rule only.
pub fn finding_for(bundle: &BundleRef, path: &str, verdict: &Verdict) -> Option<Finding> {
    let (code, severity, rule) = match verdict {
        Verdict::Allowed => return None,
        Verdict::Blocked(rule) => (CODE_BLOCKED, Severity::Error, rule),
        Verdict::Excluded(rule) => (CODE_UNRESOLVED, Severity::Warning, rule),
    };
    let action = match verdict {
        Verdict::Blocked(_) => "Remove this file from the bundle; raw patient data can never be exchanged.",
        _ => "Record a non-patient review for this file in asset-review.yaml (reviewer, time and hash) to include it, or remove it.",
    };
    Some(Finding::build(
        severity,
        code,
        bundle,
        Subject::file(path),
        format!("The file {path} is not included because {}.", rule.describe()),
        action,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(px: u8) -> Vec<u8> {
        let img = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(4, 4, image::Luma([px])));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn no_reviews() -> AssetReviewFile {
        AssetReviewFile::default()
    }

    #[test]
    fn rule_1_blocks_a_byte_copy_and_a_grayscale_copy() {
        let bytes = png(10);
        let mut known = KnownContent::default();
        known.0.insert(content_identity(&bytes));
        assert_eq!(check_file("tests/x.png", &bytes, &known, &no_reviews()), Verdict::Blocked(Rule::KnownContentHash));

        // A different encoding of the same pixels matches the imported representation.
        let img = image::load_from_memory(&bytes).unwrap();
        let rep = crate::data_engine::import::render_grayscale_png(&img);
        let mut known = KnownContent::default();
        known.0.insert(content_identity(&rep));
        assert_eq!(check_file("tests/y.png", &bytes, &known, &no_reviews()), Verdict::Blocked(Rule::KnownContentHash));
    }

    #[test]
    fn rule_2_blocks_dicom_nifti_and_identifier_metadata() {
        let known = KnownContent::default();
        let mut dicom = vec![0u8; 200];
        dicom[128..132].copy_from_slice(b"DICM");
        assert_eq!(check_file("a.bin", &dicom, &known, &no_reviews()), Verdict::Blocked(Rule::DicomOrNifti));
        assert_eq!(check_file("scan.nii.gz", b"x", &known, &no_reviews()), Verdict::Blocked(Rule::DicomOrNifti));
        let mut nifti = vec![0u8; 400];
        nifti[344..348].copy_from_slice(b"n+1\0");
        assert_eq!(check_file("v.dat", &nifti, &known, &no_reviews()), Verdict::Blocked(Rule::DicomOrNifti));

        // JPEG with an EXIF Artist tag.
        let mut tiff = b"II*\0\x08\0\0\0".to_vec();
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x013Bu16.to_le_bytes());
        tiff.extend_from_slice(&[2, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let len = (app1.len() + 2) as u16;
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend_from_slice(&len.to_be_bytes());
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(check_file("p.jpg", &jpeg, &known, &no_reviews()), Verdict::Blocked(Rule::IdentifierMetadata));

        // PNG tEXt "Author".
        let mut p = png(3);
        let idx = p.len() - 12; // before IEND
        let mut chunk = Vec::new();
        let data = b"Author\0Someone";
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&0u32.to_be_bytes());
        p.splice(idx..idx, chunk);
        assert_eq!(check_file("q.png", &p, &known, &no_reviews()), Verdict::Blocked(Rule::IdentifierMetadata));
    }

    #[test]
    fn rule_3_excludes_unreviewed_rasters_and_tables_only_under_asset_dirs() {
        let known = KnownContent::default();
        let bytes = png(5);
        assert_eq!(check_file("tests/case1.png", &bytes, &known, &no_reviews()), Verdict::Excluded(Rule::UnreviewedAsset));
        assert_eq!(check_file("assets/lut.csv", b"a,b\n", &known, &no_reviews()), Verdict::Excluded(Rule::UnreviewedAsset));
        assert_eq!(check_file("references/diagram.png", &bytes, &known, &no_reviews()), Verdict::Allowed);
        assert_eq!(check_file("tests/expected.yaml", b"k: v", &known, &no_reviews()), Verdict::Allowed);
    }

    #[test]
    fn a_matching_non_patient_review_allows_the_asset_but_never_overrides_rules_1_2() {
        let bytes = png(6);
        let reviews = AssetReviewFile {
            reviews: vec![AssetReview {
                path: "tests/case1.png".into(),
                blake3: format!("b3:{}", blake3::hash(&bytes).to_hex()),
                review: "non-patient".into(),
                reviewer: Some("r".into()),
                reviewed_at: None,
            }],
        };
        let mut known = KnownContent::default();
        assert_eq!(check_file("tests/case1.png", &bytes, &known, &reviews), Verdict::Allowed);
        // A review for different bytes does not count.
        assert_eq!(check_file("tests/case1.png", &png(7), &known, &reviews), Verdict::Excluded(Rule::UnreviewedAsset));
        // A review never overrides a known-content match.
        known.0.insert(content_identity(&bytes));
        assert_eq!(check_file("tests/case1.png", &bytes, &known, &reviews), Verdict::Blocked(Rule::KnownContentHash));
    }

    #[test]
    fn findings_name_file_and_rule_but_never_contents() {
        let b = BundleRef::new("a.b", Some("1.0.0".into()));
        let f = finding_for(&b, "tests/case1.png", &Verdict::Blocked(Rule::KnownContentHash)).unwrap();
        assert_eq!(f.code, CODE_BLOCKED);
        assert!(f.explanation.contains("tests/case1.png"));
        assert!(!f.explanation.contains("b3:"));
        assert!(finding_for(&b, "x", &Verdict::Allowed).is_none());
    }
}

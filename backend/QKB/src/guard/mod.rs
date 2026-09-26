//! Privacy guard (constitution IV, FR-012). Deny-by-default; there is no
//! parameter that disables a rule. Findings name the rule and the location,
//! never the offending content.
//!
//! Two entry points:
//! * [`check_payload`] — System One requests/reports (JSON values).
//! * [`check_bundle_file`] — files of a submitted method bundle.

pub mod metadata;

use base64::Engine as _;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Why content was refused. `describe` never echoes the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    RawImage,
    MedicalImageContainer,
    IdentifierMetadata,
    PatientIdentifier(&'static str),
    BinaryContent,
}

impl Rejection {
    pub fn describe(&self) -> String {
        match self {
            Rejection::RawImage => "raw image content is not accepted".into(),
            Rejection::MedicalImageContainer => "DICOM/NIfTI content is not accepted".into(),
            Rejection::IdentifierMetadata => "identifier-type image metadata is not accepted".into(),
            Rejection::PatientIdentifier(kind) => format!("a direct patient identifier ({kind}) is not accepted"),
            Rejection::BinaryContent => "binary or unreadable content is not accepted".into(),
        }
    }
}

const MAX_STRING: usize = 4000;

fn image_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xFF, 0xD8, 0xFF])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"BM")
        || bytes.starts_with(b"II*\0")
        || bytes.starts_with(b"MM\0*")
        || (bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

fn identifier_patterns() -> &'static [(&'static str, Regex)] {
    static P: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    P.get_or_init(|| {
        let mk = |n, r: &str| (n, Regex::new(r).expect("static regex"));
        vec![
            mk("email address", r"(?i)[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}"),
            mk("national id number", r"\b[A-Z][12]\d{8}\b"),
            mk("phone number", r"(?:\+?\d[\s\-]?){9,}\d"),
            mk("date of birth", r"(?i)\b(dob|date of birth|born on|birthday)\b|出生(日期|年月日)?"),
            mk("patient name or record number", r"(?i)\b(patient name|patient id|mrn|medical record (no|number))\b|病歷號|病患姓名|患者姓名|姓名[:：]"),
        ]
    })
}

/// Protocol machine identifiers (request UUIDs, `b3:` content ids) are long
/// digit runs by construction and are not free text.
fn is_machine_identifier(s: &str) -> bool {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| {
        Regex::new(r"^(?:[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}|b3:[0-9a-f]{64})$")
            .expect("static regex")
    })
    .is_match(s)
}

fn check_string(s: &str) -> Result<(), Rejection> {
    if is_machine_identifier(s) {
        return Ok(());
    }
    if s.len() > MAX_STRING {
        return Err(Rejection::BinaryContent);
    }
    if s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(Rejection::BinaryContent);
    }
    // Long opaque tokens (base64 and the like): decode and look for image /
    // medical-image magic; anything else that long and unspaced is not a description.
    for token in s.split_whitespace() {
        if token.len() < 64 || !token.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_')) {
            continue;
        }
        let engine = if token.contains('-') || token.contains('_') {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
        } else {
            base64::engine::general_purpose::STANDARD_NO_PAD
        };
        if let Ok(bytes) = engine.decode(token.trim_end_matches('=')) {
            if image_magic(&bytes) || metadata::is_dicom_or_nifti("", &bytes) {
                return Err(Rejection::RawImage);
            }
        }
        return Err(Rejection::BinaryContent);
    }
    for (name, re) in identifier_patterns() {
        if re.is_match(s) {
            return Err(Rejection::PatientIdentifier(name));
        }
    }
    Ok(())
}

/// Scans every string (and key) of a JSON value.
pub fn check_payload(value: &Value) -> Result<(), Rejection> {
    match value {
        Value::String(s) => check_string(s),
        Value::Array(items) => items.iter().try_for_each(check_payload),
        Value::Object(map) => map.iter().try_for_each(|(k, v)| {
            check_string(k)?;
            check_payload(v)
        }),
        _ => Ok(()),
    }
}

fn is_raster_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp"].iter().any(|e| lower.ends_with(e))
}

/// Judges one file of a submitted bundle: no raster images, no DICOM/NIfTI,
/// no identifier metadata anywhere in a method bundle.
pub fn check_bundle_file(path: &str, bytes: &[u8]) -> Result<(), Rejection> {
    if metadata::is_dicom_or_nifti(path, bytes) {
        return Err(Rejection::MedicalImageContainer);
    }
    if is_raster_path(path) || image_magic(bytes) {
        return Err(Rejection::RawImage);
    }
    if metadata::has_identifier_metadata(path, bytes) {
        return Err(Rejection::IdentifierMetadata);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_descriptions_pass() {
        assert!(check_payload(&json!({"task_purpose": "segment_area_measurement", "dtype": "uint16"})).is_ok());
    }

    #[test]
    fn identifiers_are_rejected() {
        assert!(check_payload(&json!({"x": "contact jane@example.com"})).is_err());
        assert!(check_payload(&json!({"x": "patient name: Wang"})).is_err());
        assert!(check_payload(&json!({"x": "A123456789"})).is_err());
    }

    #[test]
    fn protocol_identifiers_are_not_mistaken_for_phone_numbers() {
        assert!(check_payload(&json!({"request_id": "00000000-0000-4000-8000-000000000001"})).is_ok());
        let cid = format!("b3:{}", "0123456789abcdef".repeat(4));
        assert!(check_payload(&json!({"content_id": cid})).is_ok());
        // A UUID embedded in prose is still free text and stays scanned.
        assert!(check_payload(&json!({"x": "call 0912-345-678 now"})).is_err());
    }

    #[test]
    fn long_prose_is_not_mistaken_for_base64() {
        let prose = "segmentation of intraoral photographs followed by area measurement of the thresholded gingival region";
        assert!(check_payload(&json!({"task_purpose": prose})).is_ok());
    }

    #[test]
    fn base64_image_is_rejected() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n0000000000000000000000000000000000000000000000000000");
        assert_eq!(check_payload(&json!({"x": png})), Err(Rejection::RawImage));
    }

    #[test]
    fn bundle_images_are_rejected() {
        assert_eq!(check_bundle_file("assets/a.png", b"x"), Err(Rejection::RawImage));
        assert!(check_bundle_file("contract.yaml", b"schema: x").is_ok());
    }
}

//! Byte-level detectors for medical-image containers and identifier metadata,
//! ported from the legacy patient guard (no image decoding, no dataset lookup).

pub fn is_dicom_or_nifti(path: &str, bytes: &[u8]) -> bool {
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

pub fn has_identifier_metadata(path: &str, bytes: &[u8]) -> bool {
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


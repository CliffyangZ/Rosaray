//! PDF text layer extraction (research §12): pure Rust, offline, no OCR. A
//! page with no extractable text is *reported* (`pages_without_text`), never
//! guessed at — scanned pages stay explicit unresolved content.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PageText {
    /// 1-based page number.
    pub page: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PdfText {
    pub page_count: u32,
    pub pages: Vec<PageText>,
    pub pages_without_text: Vec<u32>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PdfError {
    #[error("the file is not a readable PDF")]
    Unreadable,
}

fn has_text(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// Text of one page, keeping the line structure the content stream implies
/// (`T*`, `'`, `"`, a vertical `Td`/`TD`, `Tm`, `ET`). `lopdf::extract_text`
/// drops those breaks, which would glue headings onto the next sentence.
fn page_text(doc: &lopdf::Document, page_number: u32) -> Option<String> {
    use lopdf::Object;
    use std::collections::BTreeMap;

    // `lopdf` keeps its font-encoding type private, so each encoding is captured
    // in a decoding closure instead of being named.
    type Decoder<'a> = Box<dyn Fn(&[u8]) -> Option<String> + 'a>;

    fn collect(text: &mut String, dec: &Decoder<'_>, operands: &[Object]) {
        for operand in operands {
            match operand {
                Object::String(bytes, _) => {
                    if let Some(t) = dec(bytes) {
                        text.push_str(&t);
                    }
                }
                Object::Array(items) => {
                    collect(text, dec, items);
                }
                Object::Integer(i) if *i < -100 => text.push(' '),
                _ => {}
            }
        }
    }
    fn newline(text: &mut String) {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
    }

    let page_id = *doc.get_pages().get(&page_number)?;
    let fonts = doc.get_page_fonts(page_id).ok()?;
    let decoders: BTreeMap<Vec<u8>, Decoder<'_>> = fonts
        .into_iter()
        .filter_map(|(name, font)| {
            font.get_font_encoding(doc).ok().map(|enc| {
                let d: Decoder<'_> = Box::new(move |b: &[u8]| lopdf::Document::decode_text(&enc, b).ok());
                (name, d)
            })
        })
        .collect();
    let content = doc.get_and_decode_page_content(page_id).ok()?;
    let mut text = String::new();
    let mut current: Option<&Decoder<'_>> = None;
    for op in &content.operations {
        match op.operator.as_str() {
            "Tf" => current = op.operands.first().and_then(|o| o.as_name().ok()).and_then(|n| decoders.get(n)),
            "Tj" | "TJ" => {
                if let Some(dec) = current {
                    collect(&mut text, dec, &op.operands);
                }
            }
            "'" | "\"" => {
                newline(&mut text);
                if let Some(dec) = current {
                    collect(&mut text, dec, &op.operands);
                }
            }
            "T*" | "Tm" | "ET" => newline(&mut text),
            "Td" | "TD" => {
                let ty = op.operands.get(1).and_then(|o| o.as_float().ok()).unwrap_or(0.0);
                if ty != 0.0 {
                    newline(&mut text);
                }
            }
            _ => {}
        }
    }
    Some(text)
}

/// Extracts per-page text. Uses `lopdf`, falling back to `pdf-extract` for
/// pages whose fonts `lopdf` cannot decode. Neither is trusted not to panic on
/// hostile input, so both run under `catch_unwind`.
pub fn extract(bytes: &[u8]) -> Result<PdfText, PdfError> {
    let doc = std::panic::catch_unwind(|| lopdf::Document::load_mem(bytes))
        .map_err(|_| PdfError::Unreadable)?
        .map_err(|_| PdfError::Unreadable)?;
    let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    if numbers.is_empty() {
        return Err(PdfError::Unreadable);
    }

    let mut texts: Vec<Option<String>> = numbers
        .iter()
        .map(|n| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page_text(&doc, *n))).ok().flatten())
        .collect();

    if texts.iter().any(|t| t.as_deref().is_none_or(|s| !has_text(s))) {
        let fallback = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes)).ok().and_then(Result::ok);
        if let Some(pages) = fallback {
            for (i, slot) in texts.iter_mut().enumerate() {
                if slot.as_deref().is_none_or(|s| !has_text(s)) {
                    if let Some(t) = pages.get(i) {
                        *slot = Some(t.clone());
                    }
                }
            }
        }
    }

    let mut pages = Vec::new();
    let mut pages_without_text = Vec::new();
    for (n, t) in numbers.iter().zip(texts) {
        match t.filter(|s| has_text(s)) {
            Some(text) => pages.push(PageText { page: *n, text }),
            None => pages_without_text.push(*n),
        }
    }
    Ok(PdfText { page_count: numbers.len() as u32, pages, pages_without_text })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &[u8] = include_bytes!("../../../tests/fixtures/kb/text-layer.pdf");
    const SCANNED: &[u8] = include_bytes!("../../../tests/fixtures/kb/scanned.pdf");

    #[test]
    fn reads_the_text_layer_page_by_page() {
        let t = extract(TEXT).unwrap();
        assert_eq!(t.page_count, 2);
        assert!(t.pages_without_text.is_empty());
        assert!(t.pages[0].text.contains("normalized by percentile stretching"), "{:?}", t.pages[0].text);
        // Line structure survives: a heading is not glued to the next sentence.
        assert!(t.pages[0].text.lines().any(|l| l == "1.2 Preprocessing"), "{:?}", t.pages[0].text);
        assert!(t.pages[1].text.contains("Dice coefficient"));
        assert_eq!(t.pages[1].page, 2);
    }

    #[test]
    fn scanned_pages_are_reported_not_guessed() {
        let t = extract(SCANNED).unwrap();
        assert_eq!(t.page_count, 2);
        assert!(t.pages.is_empty());
        assert_eq!(t.pages_without_text, vec![1, 2]);
    }

    #[test]
    fn garbage_is_unreadable_not_a_panic() {
        assert_eq!(extract(b"definitely not a pdf").unwrap_err(), PdfError::Unreadable);
        assert_eq!(extract(b"%PDF-1.4\n1 0 obj\n<<").unwrap_err(), PdfError::Unreadable);
    }
}

//! FR-012: patient identifiers and raw images are refused before anything is stored.

mod common;

use base64::Engine as _;
use common::*;
use rosaray_qkb::system_one::{query, selection, SoError};
use serde_json::json;

fn counts(t: &Two) -> (i64, i64) {
    let q: i64 = t.kb.conn.query_row("SELECT count(*) FROM qkb_query", [], |r| r.get(0)).unwrap();
    let s: i64 = t.kb.conn.query_row("SELECT count(*) FROM qkb_selection", [], |r| r.get(0)).unwrap();
    (q, s)
}

#[test]
fn identifiers_and_images_in_queries_are_rejected_and_not_persisted() {
    let t = two_pipes();
    let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n0000000000000000000000000000000000000000000000000000000000");
    for (n, purpose) in [
        (1, "segment photo of patient name: Chen".to_string()),
        (2, "contact jane.doe@example.com about this".to_string()),
        (3, "record A123456789".to_string()),
        (4, png),
    ] {
        let q = query_body(&uuid(n), &purpose, json!({}));
        assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::RejectedContent(_))), "case {n}");
    }
    let q = query_body(&uuid(5), "threshold", json!({ "modality": "jane.doe@example.com" }));
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::RejectedContent(_))));
    assert_eq!(counts(&t), (0, 0));
}

#[test]
fn identifiers_in_a_selection_reason_are_rejected() {
    let t = two_pipes();
    query::answer(&t.kb.conn, &t.kb.root, SO, &query_body(&uuid(10), "threshold segmentation", json!({ "modality": "intraoral-photo" }))).unwrap();
    let body = select_body(&uuid(10), "acme.demo", "1.0.0", &t.demo.content_id, "worked well for patient name: Lin");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &body), Err(SoError::RejectedContent(_))));
    assert_eq!(counts(&t), (1, 0));
}

#[test]
fn oversized_text_is_refused() {
    let t = two_pipes();
    let q = query_body(&uuid(11), &"a ".repeat(200), json!({}));
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::Malformed(_))));
    let q = query_body(&uuid(12), &"x".repeat(5000), json!({}));
    assert!(query::answer(&t.kb.conn, &t.kb.root, SO, &q).is_err());
}

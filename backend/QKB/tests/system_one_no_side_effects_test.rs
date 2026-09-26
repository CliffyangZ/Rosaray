//! FR-011: queries and selections never create, edit, publish or execute a method.

mod common;

use common::*;
use rosaray_qkb::system_one::{query, selection};
use serde_json::json;

#[test]
fn the_knowledge_base_is_byte_identical_after_an_exchange() {
    let t = two_pipes();
    let before = hash_tree(&t.kb.root);
    query::answer(&t.kb.conn, &t.kb.root, SO, &query_body(&uuid(1), "threshold segmentation", json!({ "modality": "intraoral-photo" }))).unwrap();
    selection::report(&t.kb.conn, &t.kb.root, SO, &select_body(&uuid(1), "acme.demo", "1.0.0", &t.demo.content_id, "fits")).unwrap();
    query::answer(&t.kb.conn, &t.kb.root, SO, &query_body(&uuid(2), "nothing like this exists", json!({}))).unwrap();
    selection::report(&t.kb.conn, &t.kb.root, SO, &abstain_body(&uuid(2), "nothing offered")).unwrap();
    assert_eq!(hash_tree(&t.kb.root), before);
    let deletions: i64 = t.kb.conn.query_row("SELECT count(*) FROM qkb_deletion_record", [], |r| r.get(0)).unwrap();
    assert_eq!(deletions, 0);
}

//! FR-005 / SC-003: the catalog is rebuildable from files without changing any
//! accepted identity; only executable versions are listed.

mod common;

use common::*;
use rosaray_qkb::catalog::repo::rebuild;
use serde_json::json;

fn identities(kb: &Kb) -> Vec<(String, String, String, String)> {
    let mut v: Vec<_> = kb
        .entries()
        .into_iter()
        .map(|e| (e.kind.unwrap(), e.id.unwrap(), e.version.unwrap(), e.content_id.unwrap()))
        .collect();
    v.sort();
    v
}

#[test]
fn dropping_and_rebuilding_the_index_keeps_every_identity() {
    let kb = Kb::seeded();
    kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    let before = identities(&kb);
    assert_eq!(before.len(), 7);

    rebuild(&kb.conn, &kb.root, &mut |_| {}).unwrap();
    assert_eq!(identities(&kb), before);
    // Editing the DB never matters: files are authoritative.
    kb.conn.execute("DELETE FROM kb_catalog_entry", []).unwrap();
    kb.conn.execute("DELETE FROM kb_fts", []).unwrap();
    assert_eq!(identities(&kb), before);
}

#[test]
fn every_listed_entry_is_verified_and_available() {
    let kb = Kb::seeded();
    kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    for e in kb.entries() {
        assert_eq!(e.availability, "available", "{:?}", e.id);
        if e.kind.as_deref() == Some("algonode") {
            assert_eq!(e.verification.as_deref(), Some("passed"));
        } else {
            assert_eq!(e.release_kind.as_deref(), Some("executable"));
        }
    }
}

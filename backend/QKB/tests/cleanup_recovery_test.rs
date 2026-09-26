//! Write-ahead deletion: a record written as `pending` before the crash is
//! replayed on the next start.

mod common;

use common::*;
use rosaray_qkb::cleanup::{list_deletions, recover_pending};
use serde_json::json;

#[test]
fn a_pending_deletion_is_replayed_after_a_crash() {
    let kb = Kb::seeded();
    let a = kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();

    // Simulate: the transaction committed `pending`, the process died before removing files.
    kb.conn
        .execute(
            "INSERT INTO qkb_deletion_record (deletion_id, kind, id, version, content_id, reason_code, root_cause,
                affected_dependents_json, trigger, state, path, recorded_at)
             VALUES ('d1','algopipe','acme.demo','1.0.0',?1,'verification_withdrawn',NULL,'[]','startup','pending','pipes/acme.demo/1.0.0','2026-01-01T00:00:00Z')",
            [&a.content_id],
        )
        .unwrap();
    assert!(kb.has_version("algopipe", "acme.demo", "1.0.0"));

    assert_eq!(recover_pending(&kb.conn, &kb.root).unwrap(), 1);
    assert!(!kb.has_version("algopipe", "acme.demo", "1.0.0"));
    let recs = list_deletions(&kb.conn, None).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].state, "done");
    assert_eq!(recover_pending(&kb.conn, &kb.root).unwrap(), 0, "replay is idempotent");
}

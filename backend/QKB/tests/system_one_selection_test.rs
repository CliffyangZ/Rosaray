//! SC-002: every class of report gets a consistent, explainable outcome.

mod common;

use common::*;
use rosaray_qkb::bundle::model::Trust;
use rosaray_qkb::system_one::{query, selection, SoError};
use rosaray_qkb::author;
use serde_json::{json, Value};

fn asked(t: &Two, n: u32) -> Value {
    let q = query_body(&uuid(n), "threshold segmentation", json!({ "modality": "intraoral-photo" }));
    query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap()
}

fn rows(t: &Two, table: &str) -> i64 {
    t.kb.conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

#[test]
fn a_valid_selection_is_accepted_and_recorded_with_a_contract_snapshot() {
    let t = two_pipes();
    asked(&t, 1);
    let body = select_body(&uuid(1), "acme.demo", "1.0.0", &t.demo.content_id, "closest fit to the task");
    let r = selection::report(&t.kb.conn, &t.kb.root, SO, &body).unwrap();
    assert_eq!(r["validation"], "accepted");
    assert_eq!(r["record"]["decision"], "selected");
    assert_eq!(r["record"]["selection"]["content_id"], t.demo.content_id.as_str());
    assert_eq!(r["record"]["source_id"], SO);
    assert!(r["record"]["contract_snapshot"]["contract"]["nodes"].is_array());
    assert_eq!(r["record"]["selected_version_deleted"], false);

    let read = selection::read_record(&t.kb.conn, &uuid(1)).unwrap();
    assert_eq!(read["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(read["selection"]["validation"], "accepted");
}

#[test]
fn abstaining_is_recorded_and_never_forced() {
    let t = two_pipes();
    asked(&t, 2);
    let r = selection::report(&t.kb.conn, &t.kb.root, SO, &abstain_body(&uuid(2), "none of these fit the modality")).unwrap();
    assert_eq!(r["validation"], "accepted");
    assert_eq!(r["record"]["decision"], "abstained");
    assert!(r["record"]["selection"].is_null());
}

#[test]
fn a_repeat_returns_the_same_record_and_a_different_report_is_refused() {
    let t = two_pipes();
    asked(&t, 3);
    let body = select_body(&uuid(3), "acme.demo", "1.0.0", &t.demo.content_id, "fits");
    let a = selection::report(&t.kb.conn, &t.kb.root, SO, &body).unwrap();
    let b = selection::report(&t.kb.conn, &t.kb.root, SO, &body).unwrap();
    assert_eq!(a, b);
    assert_eq!(rows(&t, "qkb_selection"), 1);
    let other = abstain_body(&uuid(3), "changed my mind");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &other), Err(SoError::ConflictingReport)));
    assert_eq!(rows(&t, "qkb_selection"), 1, "no contradictory record");
}

#[test]
fn a_version_that_was_not_offered_is_rejected_and_the_rejection_is_recorded() {
    let t = two_pipes();
    asked(&t, 4); // only acme.demo was offered
    let body = select_body(&uuid(4), "acme.area", "1.0.0", &t.area.content_id, "pick the other one");
    let r = selection::report(&t.kb.conn, &t.kb.root, SO, &body).unwrap();
    assert_eq!(r["validation"], "rejected");
    assert_eq!(r["rejection_code"], "not_in_candidates");
}

#[test]
fn a_wrong_content_identity_is_rejected() {
    let t = two_pipes();
    asked(&t, 5);
    let wrong = format!("b3:{}", "0".repeat(64));
    let r = selection::report(&t.kb.conn, &t.kb.root, SO, &select_body(&uuid(5), "acme.demo", "1.0.0", &wrong, "x")).unwrap();
    assert_eq!(r["rejection_code"], "identity_mismatch");
}

#[test]
fn a_version_deleted_after_the_query_is_rejected_on_the_recheck() {
    let t = two_pipes();
    asked(&t, 6);
    author::set_trust(&t.kb.conn, &t.kb.root, "rosaray.threshold", "1.0.0", Trust::Untrusted).unwrap();
    let r = selection::report(&t.kb.conn, &t.kb.root, SO, &select_body(&uuid(6), "acme.demo", "1.0.0", &t.demo.content_id, "x")).unwrap();
    assert_eq!(r["validation"], "rejected");
    assert_eq!(r["rejection_code"], "version_deleted");
}

#[test]
fn a_report_for_an_unknown_request_or_by_another_source_is_refused_without_a_record() {
    let t = two_pipes();
    asked(&t, 7);
    let body = select_body(&uuid(99), "acme.demo", "1.0.0", &t.demo.content_id, "x");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &body), Err(SoError::UnknownRequest)));
    let body = select_body(&uuid(7), "acme.demo", "1.0.0", &t.demo.content_id, "x");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, "intruder", &body), Err(SoError::AccessDenied)));
    assert_eq!(rows(&t, "qkb_selection"), 0);
}

#[test]
fn incompatible_protocol_and_bad_shapes_leave_no_record() {
    let t = two_pipes();
    asked(&t, 8);
    let mut body = select_body(&uuid(8), "acme.demo", "1.0.0", &t.demo.content_id, "x");
    body["protocol_version"] = json!("system-one/9");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &body), Err(SoError::ProtocolIncompatible)));
    let mut body = abstain_body(&uuid(8), "x");
    body["selection"] = json!({ "id": "acme.demo", "version": "1.0.0", "content_id": t.demo.content_id });
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &body), Err(SoError::Malformed(_))));
    let mut body = select_body(&uuid(8), "acme.demo", "1.0.0", &t.demo.content_id, "");
    body["reason"] = json!("   ");
    assert!(matches!(selection::report(&t.kb.conn, &t.kb.root, SO, &body), Err(SoError::Malformed(_))));
    assert_eq!(rows(&t, "qkb_selection"), 0);
}

#[test]
fn a_selection_stays_readable_after_the_method_is_deleted() {
    let t = two_pipes();
    asked(&t, 9);
    selection::report(&t.kb.conn, &t.kb.root, SO, &select_body(&uuid(9), "acme.demo", "1.0.0", &t.demo.content_id, "fits")).unwrap();
    author::set_trust(&t.kb.conn, &t.kb.root, "rosaray.threshold", "1.0.0", Trust::Untrusted).unwrap();
    assert!(!t.kb.has_version("algopipe", "acme.demo", "1.0.0"));

    let read = selection::read_record(&t.kb.conn, &uuid(9)).unwrap();
    let rec = &read["selection"]["record"];
    assert_eq!(rec["selected_version_deleted"], true);
    assert_eq!(rec["selection"]["id"], "acme.demo");
    assert!(rec["contract_snapshot"]["contract"]["nodes"].as_array().unwrap().len() == 3, "the contract as it was is preserved");
    assert_eq!(read["candidates"].as_array().unwrap().len(), 1);
}

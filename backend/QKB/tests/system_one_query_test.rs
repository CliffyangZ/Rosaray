//! US2: candidates are only published, currently executable AlgoPipes that fit
//! the declared data conditions; the answer is stored and replayable.

mod common;

use common::*;
use rosaray_qkb::system_one::{query, SoError};
use serde_json::json;

fn ids(resp: &serde_json::Value) -> Vec<String> {
    resp["candidates"].as_array().unwrap().iter().map(|c| c["pipe"]["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn only_applicable_executable_pipes_are_returned_with_fixed_identity_and_contract() {
    let t = two_pipes();
    let q = query_body(&uuid(1), "threshold segmentation", json!({ "modality": "intraoral-photo" }));
    let r = query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap();
    assert_eq!(r["status"], "answered");
    assert_eq!(ids(&r), vec!["acme.demo"], "the area pipe needs pixel spacing, which was not declared");
    let c = &r["candidates"][0];
    assert_eq!(c["pipe"]["content_id"], t.demo.content_id.as_str());
    assert_eq!(c["state"], "executable");
    assert_eq!(c["applicability"]["result"], "applicable");
    assert!(c["contract"]["nodes"].as_array().unwrap().len() == 3);
    assert!(c["limitations"].is_string() && c["summary"].is_string());
    assert!(c["dependencies"].as_array().unwrap().iter().all(|d| d["content_id"].is_string()));
    // Nodes are never candidates.
    assert!(!ids(&r).iter().any(|i| i.starts_with("rosaray.")));
}

#[test]
fn declaring_more_facts_widens_the_candidates() {
    let t = two_pipes();
    let q = query_body(&uuid(2), "threshold segmentation area", json!({ "modality": "intraoral-photo", "pixel_spacing_mm": { "x": 0.05, "y": 0.05 } }));
    let r = query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap();
    let mut got = ids(&r);
    got.sort();
    assert_eq!(got, vec!["acme.area", "acme.demo"]);
}

#[test]
fn inapplicable_pipes_can_be_requested_and_are_marked() {
    let t = two_pipes();
    let mut q = query_body(&uuid(3), "threshold segmentation", json!({ "modality": "intraoral-photo" }));
    q["include_inapplicable"] = json!(true);
    let r = query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap();
    let area = r["candidates"].as_array().unwrap().iter().find(|c| c["pipe"]["id"] == "acme.area").unwrap();
    assert_eq!(area["applicability"]["result"], "not_applicable");
}

#[test]
fn no_matching_method_is_reported_not_invented() {
    let t = two_pipes();
    let r = query::answer(&t.kb.conn, &t.kb.root, SO, &query_body(&uuid(4), "cardiac ultrasound strain", json!({}))).unwrap();
    assert_eq!(r["status"], "no_candidates");
    assert!(r["candidates"].as_array().unwrap().is_empty());
}

#[test]
fn repeating_a_query_replays_the_stored_answer_and_a_changed_body_conflicts() {
    let t = two_pipes();
    let q = query_body(&uuid(5), "threshold segmentation", json!({ "modality": "intraoral-photo" }));
    let a = query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap();
    let b = query::answer(&t.kb.conn, &t.kb.root, SO, &q).unwrap();
    assert_eq!(a, b);
    let changed = query_body(&uuid(5), "area measurement", json!({}));
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &changed), Err(SoError::ConflictingRequest)));
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, "someone-else", &q), Err(SoError::AccessDenied)));
}

#[test]
fn incompatible_protocol_versions_and_malformed_bodies_fail_explicitly() {
    let t = two_pipes();
    let mut q = query_body(&uuid(6), "threshold", json!({}));
    q["protocol_version"] = json!("system-one/2");
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::ProtocolIncompatible)));
    let mut q = query_body(&uuid(7), "threshold", json!({}));
    q["surprise"] = json!(1);
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::Malformed(_))));
    let q = query_body(&uuid(8), "threshold", json!({ "shoe_size": 42 }));
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::Malformed(_))));
    let q = json!({ "protocol_version": "system-one/1", "request_id": "not-a-uuid", "task_purpose": "x" });
    assert!(matches!(query::answer(&t.kb.conn, &t.kb.root, SO, &q), Err(SoError::Malformed(_))));
}

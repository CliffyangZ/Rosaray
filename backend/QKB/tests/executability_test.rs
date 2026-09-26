//! Executability reasons (data-model §2). Data applicability is a different
//! question and never affects executability.

mod common;

use common::*;
use rosaray_qkb::bundle::model::Trust;
use rosaray_qkb::executability::assess_dir;
use rosaray_qkb::{author, trust};
use serde_json::json;

fn codes(kb: &Kb, rel: &str) -> Vec<&'static str> {
    assess_dir(&kb.conn, &kb.root, &kb.root.join(rel)).executability.reasons.iter().map(|r| r.code).collect()
}

#[test]
fn verified_trusted_nodes_and_their_pipes_are_executable() {
    let kb = Kb::seeded();
    kb.submit_pipe("acme.area", "1.0.0", area_graph()).unwrap();
    assert!(codes(&kb, "nodes/rosaray.area/1.0.0").is_empty());
    assert!(codes(&kb, "pipes/acme.area/1.0.0").is_empty(), "pixel-spacing needs are applicability, not executability");
}

#[test]
fn untrusted_withdrawn_and_unverified_nodes_are_not_executable() {
    let kb = Kb::seeded();
    let cid = |id: &str| -> String {
        let (b, _) = rosaray_qkb::bundle::read::read_bundle(&kb.root.join("nodes").join(id).join("1.0.0"));
        b.unwrap().lock.unwrap().content_id
    };
    // Change decisions without purging, to look at the raw reasons.
    trust::record(&kb.root, "rosaray.normalize", "1.0.0", &cid("rosaray.normalize"), Trust::Untrusted, "test").unwrap();
    assert_eq!(codes(&kb, "nodes/rosaray.normalize/1.0.0"), vec!["implementation_untrusted"]);

    // A node with no verification chain at all.
    std::fs::remove_dir_all(kb.root.join("verification/rosaray.threshold")).unwrap();
    assert_eq!(codes(&kb, "nodes/rosaray.threshold/1.0.0"), vec!["verification_missing"]);
}

#[test]
fn a_tampered_published_version_is_not_executable() {
    let kb = Kb::seeded();
    let f = kb.root.join("nodes/rosaray.area/1.0.0/contract.yaml");
    let mut perms = std::fs::metadata(&f).unwrap().permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(&f, perms).unwrap();
    std::fs::write(&f, "schema: quantify-kb/1\ninputs: []\noutputs: []\nparameters: []\nprerequisites: []\n").unwrap();
    assert!(codes(&kb, "nodes/rosaray.area/1.0.0").contains(&"content_modified"));
    let deleted = kb.purge("rescan");
    assert!(deleted.iter().any(|d| d.id == "rosaray.area"));
}

#[test]
fn reverifying_runs_the_nodes_own_tests() {
    let kb = Kb::seeded();
    let (passed, deleted) = author::reverify(&kb.conn, &kb.root, "rosaray.area", "1.0.0").unwrap();
    assert!(passed && deleted.is_empty());
    let _ = json!({});
}

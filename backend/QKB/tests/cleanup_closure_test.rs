//! FR-006 / constitution III: permanent deletion with dependency closure,
//! deletion records, no automatic restoration, and preservation of everything
//! still executable.

mod common;

use common::*;
use rosaray_qkb::author;
use rosaray_qkb::bundle::model::Trust;
use serde_json::json;

fn two_pipes(kb: &Kb) {
    kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    kb.submit_pipe("acme.area", "1.0.0", area_graph()).unwrap();
}

#[test]
fn revoking_a_required_node_deletes_it_and_every_dependent_pipe() {
    let kb = Kb::seeded();
    two_pipes(&kb);
    let gaussian_before = hash_tree(&kb.root.join("nodes/rosaray.gaussian-blur"));
    let source_before = hash_tree(&kb.root.join("nodes/rosaray.image-source"));

    let deleted = author::set_trust(&kb.conn, &kb.root, "rosaray.normalize", "1.0.0", Trust::Untrusted).unwrap();

    let ids: Vec<String> = deleted.iter().map(|d| format!("{}@{}", d.id, d.version)).collect();
    assert!(ids.contains(&"rosaray.normalize@1.0.0".to_string()), "{ids:?}");
    assert!(ids.contains(&"acme.demo@1.0.0".to_string()) && ids.contains(&"acme.area@1.0.0".to_string()), "{ids:?}");
    let node = deleted.iter().find(|d| d.id == "rosaray.normalize").unwrap();
    assert_eq!(node.reason_code, "implementation_untrusted");
    assert_eq!(node.trigger, "trust_change");
    assert_eq!(node.state, "done");
    assert_eq!(node.affected_dependents, vec!["algopipe:acme.area@1.0.0", "algopipe:acme.demo@1.0.0"]);
    for pipe in deleted.iter().filter(|d| d.kind == "algopipe") {
        assert_eq!(pipe.reason_code, "cascade");
        assert_eq!(pipe.root_cause.as_deref(), Some("algonode:rosaray.normalize@1.0.0"));
    }

    // Files are gone — including the verification chain of the last version.
    assert!(!kb.has_version("algonode", "rosaray.normalize", "1.0.0"));
    assert!(!kb.has_version("algopipe", "acme.demo", "1.0.0") && !kb.has_version("algopipe", "acme.area", "1.0.0"));
    assert!(!kb.root.join("verification/rosaray.normalize").exists());
    assert!(!kb.entries().iter().any(|e| e.id.as_deref() == Some("rosaray.normalize")));
    // Unaffected executable versions are untouched.
    assert_eq!(hash_tree(&kb.root.join("nodes/rosaray.gaussian-blur")), gaussian_before);
    assert_eq!(hash_tree(&kb.root.join("nodes/rosaray.image-source")), source_before);
    assert!(kb.has_version("algonode", "rosaray.threshold", "1.0.0"));

    // No dangling references remain and the deletion is explainable later.
    assert!(kb.purge("rescan").is_empty(), "the state is a fixpoint");
    let recorded = rosaray_qkb::cleanup::list_deletions(&kb.conn, Some(("rosaray.normalize", ""))).unwrap();
    assert_eq!(recorded.len(), 1);
}

#[test]
fn restoring_the_cause_does_not_bring_anything_back() {
    let kb = Kb::seeded();
    two_pipes(&kb);
    author::set_trust(&kb.conn, &kb.root, "rosaray.normalize", "1.0.0", Trust::Untrusted).unwrap();
    // Resubmitting the node makes the node exist again (a new submission)…
    let seed = rosaray_qkb::seed::seed_bundles().into_iter().find(|s| s.id == "rosaray.normalize").unwrap();
    let files = seed.files.iter().map(|(r, t)| (r.to_string(), t.as_bytes().to_vec())).collect();
    kb.submit(files).unwrap();
    assert!(kb.has_version("algonode", "rosaray.normalize", "1.0.0"));
    // …but the deleted pipes are not restored.
    assert!(!kb.has_version("algopipe", "acme.demo", "1.0.0"));
    assert!(!kb.has_version("algopipe", "acme.area", "1.0.0"));
    assert_eq!(rosaray_qkb::cleanup::list_deletions(&kb.conn, None).unwrap().len(), 3, "history keeps the first deletion");
}

#[test]
fn a_withdrawn_verification_pauses_and_therefore_deletes() {
    let kb = Kb::seeded();
    two_pipes(&kb);
    let deleted = author::withdraw_verification(&kb.conn, &kb.root, "rosaray.threshold", "1.0.0", "suspect").unwrap();
    let node = deleted.iter().find(|d| d.id == "rosaray.threshold").unwrap();
    assert_eq!(node.reason_code, "verification_withdrawn");
    assert!(!kb.has_version("algopipe", "acme.demo", "1.0.0"));
}

#[test]
fn a_query_specific_data_mismatch_never_deletes() {
    let kb = Kb::seeded();
    two_pipes(&kb); // acme.area needs pixel spacing; nothing about that is checked here
    assert!(kb.purge("rescan").is_empty());
    assert!(kb.has_version("algopipe", "acme.area", "1.0.0"));
}

#[test]
fn migration_removes_drafts_knowledge_only_and_unreadable_versions_but_keeps_executable_ones() {
    let kb = Kb::seeded();
    two_pipes(&kb);
    let keep_node = hash_tree(&kb.root.join("nodes/rosaray.area"));
    let keep_pipe = hash_tree(&kb.root.join("pipes/acme.demo"));

    // Legacy leftovers.
    write_draft(&kb.root, "algopipe", "old.draft", pipe_files("old.draft", "0.1.0", demo_graph(json!({}))));
    write_legacy_published(&kb.root, "algopipe", "old.knowledge", "1.0.0", pipe_files("old.knowledge", "1.0.0", demo_graph(json!({}))), "knowledge");
    let corrupt = kb.root.join("nodes/old.corrupt/1.0.0");
    std::fs::create_dir_all(&corrupt).unwrap();
    std::fs::write(corrupt.join("ALGONODE.md"), "not a bundle").unwrap();

    let deleted = kb.purge("migration");
    let by_id = |id: &str| deleted.iter().find(|d| d.id == id).unwrap_or_else(|| panic!("no record for {id}: {deleted:?}"));
    assert_eq!(by_id("old.draft").reason_code, "draft_removed");
    assert_eq!(by_id("old.knowledge").reason_code, "not_executable_release");
    assert_eq!(by_id("old.corrupt").reason_code, "unrecognized_format");
    assert!(deleted.iter().all(|d| d.trigger == "migration"));

    assert!(!kb.root.join("drafts").exists());
    assert!(!kb.root.join("pipes/old.knowledge").exists() && !kb.root.join("nodes/old.corrupt").exists());
    assert_eq!(hash_tree(&kb.root.join("nodes/rosaray.area")), keep_node, "SC-003: executable content is unchanged");
    assert_eq!(hash_tree(&kb.root.join("pipes/acme.demo")), keep_pipe);
    assert!(kb.has_version("algopipe", "acme.area", "1.0.0"));
}

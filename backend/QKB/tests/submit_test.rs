//! US1 / FR-002: submission = validate, then store a fixed version. A failed
//! submission leaves nothing behind — no draft, no partial folder.

mod common;

use common::*;
use rosaray_qkb::submit::SubmitError;
use serde_json::json;

#[test]
fn a_valid_pipe_is_accepted_as_a_fixed_executable_version() {
    let kb = Kb::seeded();
    let a = kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    assert!(!a.already_present && a.content_id.starts_with("b3:"));
    assert!(kb.has_version("algopipe", "acme.demo", "1.0.0"));

    let dir = kb.root.join("pipes/acme.demo/1.0.0");
    let graph = std::fs::read_to_string(dir.join("graph.yaml")).unwrap();
    assert!(graph.contains("content_id: b3:"), "references are frozen to content ids");
    assert!(graph.contains("implementation_pins"));
    let lock = std::fs::read_to_string(dir.join("bundle.lock")).unwrap();
    assert!(lock.contains("release_kind: executable"));
    assert!(!kb.root.join("drafts").exists(), "no drafts folder is ever created");
}

#[test]
fn resubmitting_identical_content_is_idempotent_and_different_content_conflicts() {
    let kb = Kb::seeded();
    let first = kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    let before = hash_tree(&kb.root.join("pipes"));
    let again = kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "otsu" }))).unwrap();
    assert!(again.already_present);
    assert_eq!(again.content_id, first.content_id);

    let err = kb.submit_pipe("acme.demo", "1.0.0", demo_graph(json!({ "mode": "manual", "value": 100 }))).unwrap_err();
    assert!(matches!(err, SubmitError::VersionExists), "{err:?}");
    assert_eq!(hash_tree(&kb.root.join("pipes")), before, "the accepted version is untouched");

    // New content is a new fixed version.
    let v2 = kb.submit_pipe("acme.demo", "1.1.0", demo_graph(json!({ "mode": "manual", "value": 100 }))).unwrap();
    assert_ne!(v2.content_id, first.content_id);
    assert!(kb.has_version("algopipe", "acme.demo", "1.0.0"), "the old version stays alongside the new one");
}

#[test]
fn a_pipe_with_a_missing_dependency_is_rejected_and_nothing_is_written() {
    let kb = Kb::seeded();
    let mut g = demo_graph(json!({ "mode": "otsu" }));
    g["nodes"][1]["ref"]["version"] = json!("9.9.9");
    let before = hash_tree(&kb.root);
    let err = kb.submit_pipe("acme.bad", "1.0.0", g).unwrap_err();
    let SubmitError::Rejected(findings) = err else { panic!("expected rejection, got {err:?}") };
    assert!(findings.iter().any(|f| f.is_error()));
    assert!(findings.iter().all(|f| !f.explanation.is_empty() && !f.action.is_empty()));
    assert_eq!(hash_tree(&kb.root), before, "the KB is byte-for-byte unchanged");
    assert!(!kb.root.join("pipes/acme.bad").exists());
}

#[test]
fn a_knowledge_only_node_without_an_implementation_is_rejected() {
    let kb = Kb::empty();
    let md = "---\nschema: quantify-kb/1\nkind: algonode\nid: acme.spec\nversion: 1.0.0\nname: Spec only\nsummary: s\nstatus: draft\nresearch_use_only: true\nintended_use: research\nlimitations: none\n---\nBody\n";
    let contract = "schema: quantify-kb/1\ninputs: []\noutputs: []\nparameters: []\nprerequisites: []\n";
    let err = kb
        .submit(vec![("ALGONODE.md".into(), md.as_bytes().to_vec()), ("contract.yaml".into(), contract.as_bytes().to_vec())])
        .unwrap_err();
    let SubmitError::Rejected(findings) = err else { panic!("{err:?}") };
    assert!(findings.iter().any(|f| f.code == "implementation_unavailable"), "{findings:?}");
    assert!(kb.entries().is_empty());
    assert!(!kb.root.join("nodes/acme.spec").exists());
}

#[test]
fn a_node_whose_own_tests_fail_is_rejected() {
    let kb = Kb::empty();
    let seed = rosaray_qkb::seed::seed_bundles().into_iter().find(|s| s.id == "rosaray.threshold").unwrap();
    let files: Vec<(String, Vec<u8>)> = seed
        .files
        .iter()
        .map(|(rel, text)| {
            let text = if *rel == "tests/cases.yaml" { text.replace("\"expect\":", "\"expect_broken\":") } else { text.to_string() };
            (rel.to_string(), text.into_bytes())
        })
        .collect();
    let err = kb.submit(files).unwrap_err();
    assert!(matches!(err, SubmitError::Rejected(_)), "{err:?}");
    assert!(kb.entries().is_empty());
}

#[test]
fn raw_images_and_lock_files_are_refused() {
    let kb = Kb::seeded();
    let mut files = pipe_files("acme.img", "1.0.0", demo_graph(json!({ "mode": "otsu" })));
    files.push(("assets/sample.png".into(), b"\x89PNG\r\n\x1a\nxxxx".to_vec()));
    assert!(matches!(kb.submit(files).unwrap_err(), SubmitError::RejectedContent(_)));

    let mut files = pipe_files("acme.lock", "1.0.0", demo_graph(json!({ "mode": "otsu" })));
    files.push(("bundle.lock".into(), b"x".to_vec()));
    assert!(matches!(kb.submit(files).unwrap_err(), SubmitError::Rejected(_)));
    assert!(!kb.root.join("pipes/acme.img").exists() && !kb.root.join("pipes/acme.lock").exists());
}

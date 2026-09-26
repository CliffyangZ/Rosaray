//! Constitution spot-checks for feature 002 (quickstart "Constitution
//! spot-checks"): offline (I), immutable history (II), Preview/Run separation
//! (III), research-use labelling and safe wording (FR-033), and encryption at
//! rest for research records (VI).

mod common;

use reqwest::Method;
use serde_json::json;

fn read(rel: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn rs_files(dir: &str) -> Vec<(String, String)> {
    fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(p).unwrap().flatten() {
            if e.path().is_dir() {
                walk(&e.path(), out);
            } else if e.path().extension().is_some_and(|x| x == "rs") {
                out.push(e.path());
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.into_iter().map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap())).collect()
}

/// I: nothing in the feature can reach the network — there is no HTTP client in the
/// production dependency set and no outbound socket code in the new modules.
#[test]
fn the_feature_has_no_network_capability() {
    let manifest = read("Cargo.toml");
    let deps = manifest.split("[dev-dependencies]").next().unwrap();
    for client in ["reqwest", "hyper-util", "ureq", "isahc", "surf", "curl"] {
        assert!(!deps.contains(client), "production dependency {client} could reach the network");
    }
    for dir in ["src/kb", "src/designer"] {
        for (path, text) in rs_files(dir) {
            let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
            for needle in ["TcpStream", "UdpSocket", "std::net::", "TcpListener"] {
                assert!(!code.contains(needle), "{path} uses {needle}");
            }
        }
    }
}

/// III: Run and Export code paths never reference Preview types.
#[test]
fn run_and_export_code_never_touch_preview() {
    let files = [
        "src/data_engine/run.rs",
        "src/data_engine/export.rs",
        "src/designer/official.rs",
        "src/api/algopipe_run.rs",
        "src/api/runs.rs",
        "src/api/export.rs",
        "src/kb/exchange/export.rs",
    ];
    for f in files {
        let text = read(f);
        let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for needle in ["preview_cache", "PreviewCache", "NodeCache", "PreviewBoard", "designer::preview", "run_preview", "PreviewOutcome", "preview_nodes", "RequestIsolation"] {
            assert!(!code.contains(needle), "{f} references Preview state ({needle})");
        }
    }
    // ...and Preview never reaches the Run repository.
    for f in ["src/api/pipe_preview.rs", "src/designer/preview.rs", "src/api/preview.rs"] {
        let text = read(f);
        assert!(!text.contains("run_repo"), "{f} references the run repository");
    }
}

/// II: published bundles are byte-identical after every non-publishing workflow.
#[tokio::test]
async fn published_history_survives_every_workflow_untouched() {
    let svc = common::spawn().await;
    svc.install_seeds();
    let published = || (common::hash_tree(&svc.kb_root.join("nodes")), common::hash_tree(&svc.kb_root.join("pipes")));
    let before = published();

    for path in ["/kb/refresh", "/kb/rebuild"] {
        assert_eq!(svc.json(Method::POST, path, None).await.0, 200);
    }
    svc.json(
        Method::POST,
        "/kb/nodes/rosaray.threshold/1.0.0/verification",
        Some(json!({ "type": "technical", "event": "passed", "implementation_id": "builtin.threshold", "implementation_version": "1" })),
    )
    .await;
    svc.json(Method::POST, "/kb/nodes/rosaray.threshold/1.0.0/deprecate", Some(json!({ "reason": "test" }))).await;
    svc.json(
        Method::POST,
        "/kb/nodes/rosaray.threshold/1.0.0/verification",
        Some(json!({ "type": "technical", "event": "withdrawn", "implementation_id": "builtin.threshold", "implementation_version": "1", "reason": "x" })),
    )
    .await;
    svc.create_draft_from_fixture("algonode", "acme.spec-only", "valid-spec-only-node").await;
    let (_, entries) = svc.json(Method::GET, "/kb/entries", None).await;
    assert!(entries["entries"].as_array().unwrap().len() >= 7);
    let (_, preview) = svc.json(Method::POST, "/kb/export/preview", Some(json!({ "kind": "algonode", "id": "rosaray.threshold", "version": "1.0.0" }))).await;
    assert_eq!(preview["blocked"], false);

    assert_eq!(published(), before, "published bundles never change");
}

/// VI: research records (catalog, papers, candidates, decisions, audit) are
/// unreadable without the project key.
#[tokio::test]
async fn research_records_are_unreadable_without_the_project_key() {
    let svc = common::spawn().await;
    svc.install_seeds();
    svc.state.db.lock().unwrap().execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    let path = svc._project_dir_path().join("rosaray.sqlite3");
    let raw = std::fs::read(&path).unwrap();
    assert!(!raw.starts_with(b"SQLite format 3"), "the database file is not plaintext SQLite");
    assert!(!raw.windows(14).any(|w| w == b"kb_catalog_ent"), "table names are not readable on disk");

    let plain = rusqlite::Connection::open(&path).unwrap();
    assert!(plain.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err(), "no key, no data");
    let wrong = rusqlite::Connection::open(&path).unwrap();
    wrong.execute_batch("PRAGMA key = \"x'00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff'\";").unwrap();
    assert!(wrong.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0)).is_err(), "a wrong key is not silently empty");
}

/// FR-033 / IV: every method view is labelled research-use-only, and no UI string
/// implies a diagnosis or treatment recommendation.
#[test]
fn the_ui_is_labelled_research_use_only_and_never_implies_clinical_use() {
    for f in ["node_inspector.js", "kb_catalog.js", "draft_editor.js", "paper_review.js"] {
        let text = read(&format!("../frontend/src/{f}"));
        assert!(text.to_ascii_lowercase().contains("research use only") || text.contains("RESEARCH_ONLY"), "{f} lacks the research-use-only label");
    }
    let index = read("../frontend/index.html");
    assert!(index.contains("Research use only"), "the status bar keeps its label");
    // Every mention of clinical words must be a negation or a refusal.
    let allowed = ["not for diagnosis", "not a clinical", "diagnosis or treatment", "cannot become", "can only be rejected", "diagnosis or treatment statement", "clinical statement", "unsafe"];
    for f in ["node_inspector.js", "kb_catalog.js", "draft_editor.js", "paper_review.js", "graph_editor.js", "service_client.js"] {
        for (i, line) in read(&format!("../frontend/src/{f}")).lines().enumerate() {
            let l = line.to_ascii_lowercase();
            if l.contains("diagnos") || l.contains("treatment") || l.contains("therapy") || l.contains("prognos") {
                assert!(allowed.iter().any(|a| l.contains(a)) || l.trim_start().starts_with("//"), "{f}:{}: {line}", i + 1);
            }
        }
    }
}

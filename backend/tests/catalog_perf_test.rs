//! SC-001: with 2,000 method versions, search and filter respond within one
//! second for at least 95% of operations. (Bundles are written directly as
//! published-shaped folders; this measures catalog query cost, not submission.)
//!
//! Run with output: `cargo test --test catalog_perf_test -- --nocapture`

mod common;

use std::time::{Duration, Instant};

use common::*;
use rosaray_qkb::bundle::model::{to_yaml, BundleLock, LockFile};
use rosaray_qkb::bundle::write::write_bundle_atomic;

const VERSIONS: usize = 2_000;
const TOPICS: [&str; 8] = ["threshold", "segmentation", "morphology", "gaussian", "normalization", "measurement", "calibration", "registration"];

fn write_pipe(root: &std::path::Path, i: usize) {
    let id = format!("perf.pipe{:04}", i / 2);
    let version = format!("1.{}.0", i % 2);
    let topic = TOPICS[i % TOPICS.len()];
    let md = format!("---\nschema: quantify-kb/1\nkind: algopipe\nid: {id}\nversion: {version}\nname: {topic} pipeline {i}\nsummary: {topic} of study {i} images with variant {}\nstatus: published\nresearch_use_only: true\nintended_use: {topic} for cohort {}\nlimitations: synthetic\ndomain: {}\n---\nBody\n", i * 7, i % 13, if i % 2 == 0 { "dental-image" } else { "generic" });
    let graph = "schema: quantify-kb/1\nnodes: []\nedges: []\n";
    let files = vec![("ALGOPIPE.md".to_string(), md.into_bytes()), ("graph.yaml".to_string(), graph.as_bytes().to_vec())];
    let hashed: Vec<(String, String)> = files.iter().map(|(p, b)| (p.clone(), format!("b3:{}", blake3::hash(b).to_hex()))).collect();
    let content_id = rosaray_qkb::identity::content_id_of(hashed.iter().map(|(p, h)| (p.as_str(), h.as_str())));
    let lock = BundleLock {
        schema: "quantify-kb/1".into(),
        id: id.clone(),
        version: version.clone(),
        content_id,
        computational_identity: None,
        release_kind: Some("executable".into()),
        disclosures: vec![],
        dependency_summary: None,
        verification_summary: None,
        files: hashed.iter().map(|(p, h)| LockFile { path: p.clone(), blake3: h.trim_start_matches("b3:").to_string() }).collect(),
    };
    let mut all = files;
    all.push(("bundle.lock".into(), to_yaml(&lock).unwrap().into_bytes()));
    write_bundle_atomic(&root.join("pipes").join(&id).join(&version), &all).unwrap();
}

#[tokio::test]
async fn search_and_filter_stay_under_a_second_with_two_thousand_versions() {
    let svc = spawn().await;
    let t0 = Instant::now();
    for i in 0..VERSIONS {
        write_pipe(&svc.state.kb_root, i);
    }
    println!("wrote {VERSIONS} bundles in {:?}", t0.elapsed());
    {
        let db = svc.state.db.lock().unwrap();
        let t1 = Instant::now();
        rosaray_qkb::catalog::repo::refresh(&db, &svc.state.kb_root, &mut |_| {}).unwrap();
        println!("first index of {VERSIONS} bundles: {:?}", t1.elapsed());
        let t2 = Instant::now();
        rosaray_qkb::catalog::repo::refresh(&db, &svc.state.kb_root, &mut |_| {}).unwrap();
        println!("incremental refresh: {:?}", t2.elapsed());
    }
    let total: i64 = svc.state.db.lock().unwrap().query_row("SELECT count(*) FROM kb_catalog_entry WHERE status = 'published'", [], |r| r.get(0)).unwrap();
    assert!(total >= VERSIONS as i64, "{total}");

    let mut samples: Vec<Duration> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for t in TOPICS {
        paths.push(format!("/qkb/v1/catalog/entries?q={t}"));
        paths.push(format!("/qkb/v1/catalog/entries?q={t}&domain=dental-image&limit=50"));
    }
    paths.push("/qkb/v1/catalog/entries?kind=algopipe&domain=generic&limit=100".into());
    paths.push("/qkb/v1/catalog/entries?kind=algonode".into());
    paths.push("/qkb/v1/catalog/entries?purpose=cohort".into());
    for round in 0..10 {
        for p in &paths {
            let t0 = Instant::now();
            let (s, b) = svc.get(As::Frontend, p).await;
            samples.push(t0.elapsed());
            assert_eq!(s, reqwest::StatusCode::OK, "{p} round {round}");
            assert!(b["entries"].is_array());
        }
    }
    samples.sort();
    let p95 = samples[((samples.len() as f64 * 0.95).ceil() as usize).min(samples.len()) - 1];
    println!("catalog ops: n={} p95={:?} max={:?}", samples.len(), p95, samples.last().unwrap());
    assert!(p95 < Duration::from_secs(1), "p95 {p95:?} exceeds 1s");
}

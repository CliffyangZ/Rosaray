//! Performance pass against the representative-project targets
//! (spec SC-001 / SC-015 / SC-017): 1,000 images and 500 official Runs.
//!
//! Fixture images are small (32×32) so the pass stays fast enough for CI;
//! SC-001's 50-megapixel ceiling is about decode/encode cost, which this
//! does not exercise — see the note printed at the end. What it does
//! measure is everything that scales with record count: list queries,
//! per-image display descriptors, Run lookup and artifact reads.
//!
//! Run with output: `cargo test --test perf -- --nocapture`

mod common;

use reqwest::Method;
use std::time::{Duration, Instant};

const IMAGES: usize = 1_000;
const RUNS: usize = 500;

fn write_unique_png(path: &std::path::Path, i: usize) {
    let img = image::GrayImage::from_fn(32, 32, |x, y| {
        image::Luma([((x as usize * 7 + y as usize * 13 + i * 31 + (i >> 4) * 101) % 256) as u8])
    });
    // Guarantee byte-uniqueness across all 1,000 files: a per-file marker pixel.
    let mut img = img;
    img.put_pixel(0, 0, image::Luma([(i % 256) as u8]));
    img.put_pixel(1, 0, image::Luma([(i / 256) as u8]));
    img.save(path).unwrap();
}

fn percentile(samples: &mut [Duration], p: f64) -> Duration {
    samples.sort();
    let idx = ((samples.len() as f64 * p).ceil() as usize).clamp(1, samples.len()) - 1;
    samples[idx]
}

fn report(name: &str, samples: &mut [Duration]) -> (Duration, Duration) {
    let (p95, p99) = (percentile(samples, 0.95), percentile(samples, 0.99));
    println!(
        "{name:<34} n={:<4} p95={:>7.1}ms  p99={:>7.1}ms  max={:>7.1}ms",
        samples.len(),
        p95.as_secs_f64() * 1e3,
        p99.as_secs_f64() * 1e3,
        samples.last().unwrap().as_secs_f64() * 1e3
    );
    (p95, p99)
}

async fn timed(
    service: &common::TestService,
    path: &str,
) -> (Duration, reqwest::StatusCode, Vec<u8>) {
    let start = Instant::now();
    let resp = service.request(Method::GET, path).send().await.unwrap();
    let status = resp.status();
    let body = resp.bytes().await.unwrap().to_vec();
    (start.elapsed(), status, body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn representative_project_meets_latency_targets() {
    let service = common::spawn().await;

    // SC-017: ready (or an actionable error) within 2s.
    let (t, status, body) = timed(&service, "/session").await;
    assert!(status.is_success() && String::from_utf8_lossy(&body).contains("ready"));
    assert!(t < Duration::from_secs(2), "session ready took {t:?}");

    let dir = tempfile::tempdir().unwrap();
    for i in 0..IMAGES {
        write_unique_png(&dir.path().join(format!("img_{i:04}.png")), i);
    }
    let start = Instant::now();
    let (_, version_id) = service.import_dir(dir.path(), "Perf").await;
    println!("import of {IMAGES} images: {:?}", start.elapsed());
    let image_ids = service.image_ids(&version_id).await;
    assert_eq!(image_ids.len(), IMAGES);

    // 500 official Runs, spread across images, each with its own seed.
    let start = Instant::now();
    let mut run_ids = Vec::with_capacity(RUNS);
    for i in 0..RUNS {
        let (status, created) = service
            .json(
                Method::POST,
                "/runs",
                Some(serde_json::json!({
                    "dataset_version_id": version_id,
                    "image_asset_id": image_ids[i * 2 % IMAGES],
                    "pipeline_snapshot": common::TestService::blur_pipeline(),
                    "target_node_id": "blur",
                    "seed": i,
                })),
            )
            .await;
        assert_eq!(status, 200, "{created}");
        run_ids.push(created["run_id"].as_str().unwrap().to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let (_, runs) = service
            .json(Method::GET, &format!("/runs?dataset_version_id={version_id}"), None)
            .await;
        let runs = runs["runs"].as_array().unwrap();
        let done = runs.iter().filter(|r| r["status"] != "running").count();
        if done == RUNS {
            assert!(runs.iter().all(|r| r["status"] == "succeeded"));
            break;
        }
        assert!(Instant::now() < deadline, "only {done}/{RUNS} runs finished");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("{RUNS} official runs completed: {:?}", start.elapsed());

    // SC-015: Explorer list within 1s at 1,000 images.
    let mut list = Vec::new();
    for _ in 0..20 {
        let (t, status, _) = timed(&service, &format!("/dataset-versions/{version_id}/images")).await;
        assert!(status.is_success());
        list.push(t);
    }
    let (p95, _) = report("SC-015 image list (1000)", &mut list);
    assert!(p95 < Duration::from_secs(1));

    // SC-015: selected-image display descriptor.
    let mut display = Vec::new();
    for id in image_ids.iter().step_by(IMAGES / 200) {
        let (t, status, _) = timed(&service, &format!("/image-assets/{id}/display")).await;
        assert!(status.is_success());
        display.push(t);
    }
    let (p95, p99) = report("SC-015 image display", &mut display);
    assert!(p95 < Duration::from_secs(1) && p99 < Duration::from_secs(1));

    // SC-001: reading a stored result — Run record, then its artifact bytes.
    let mut result_reads = Vec::new();
    for run_id in run_ids.iter().step_by(RUNS / 200) {
        let start = Instant::now();
        let (_, run) = service.json(Method::GET, &format!("/runs/{run_id}"), None).await;
        let artifact = run["output_artifact_refs"][0]["id"].as_str().unwrap().to_string();
        let (_, status, bytes) = timed(&service, &format!("/artifacts/{artifact}/content")).await;
        assert!(status.is_success() && !bytes.is_empty());
        result_reads.push(start.elapsed());
    }
    let (p95, p99) = report("SC-001 result read (run+artifact)", &mut result_reads);
    assert!(p95 < Duration::from_millis(250), "p95 {p95:?} over 250ms");
    assert!(p99 < Duration::from_secs(1), "p99 {p99:?} over 1s");

    // SC-001 list-scale lookups: run history for the version.
    let mut history = Vec::new();
    for _ in 0..20 {
        let (t, status, _) = timed(&service, &format!("/runs?dataset_version_id={version_id}")).await;
        assert!(status.is_success());
        history.push(t);
    }
    let (p95, _) = report("run history (500)", &mut history);
    assert!(p95 < Duration::from_secs(1));

    println!("note: 32x32 fixtures; the 50-megapixel decode path is not covered by this pass");
}

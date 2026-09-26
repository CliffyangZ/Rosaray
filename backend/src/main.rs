use std::path::PathBuf;

use rosaray_qkb::auth::random_token;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("rosaray-service {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let project_dir = args
        .iter()
        .position(|a| a == "--project-dir")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./rosaray-project"));

    let rt = tokio::runtime::Runtime::new().expect("failed to start async runtime");
    rt.block_on(run(project_dir));
}

async fn run(project_dir: PathBuf) {
    let catalog_token = random_token();
    let (state, report) = rosaray_service::bootstrap::open(&project_dir, catalog_token.clone()).expect("failed to open the QKB");
    if report.legacy_files_present {
        eprintln!(
            "legacy Dataset/Run files were found in {} and are left untouched; retrieve them with the legacy tools (docs/legacy-retrieval.md)",
            project_dir.display()
        );
    }
    if report.deleted_versions > 0 {
        eprintln!("removed {} method version(s) that are no longer executable (see GET /qkb/v1/deletions)", report.deleted_versions);
    }

    let router = rosaray_service::api::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("failed to bind loopback listener");
    let port = listener.local_addr().expect("listener has a local address").port();

    // One machine-readable line for the launching process (frontend dev server):
    // the ephemeral port and the read-only catalog credential. The author and
    // System One credentials are owner-only files in `qkb-credentials/`.
    println!("{}", serde_json::json!({ "port": port, "session_token": catalog_token }));

    axum::serve(listener, router).await.expect("service exited unexpectedly");
}

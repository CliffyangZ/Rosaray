use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use rand::RngCore;
use rosaray_service::api::session::SessionState;
use rosaray_service::api::{AppState, AppStateInner};
use rosaray_service::crypto::{self, MasterKey};
use rosaray_service::data_repository::blob_store::BlobStore;
use rosaray_service::data_repository::sqlite;

const VERIFIER_PLAINTEXT: &[u8] = b"rosaray-master-key-verifier";

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
    std::fs::create_dir_all(&project_dir).expect("failed to create project directory");

    let master_key = load_or_init_master_key(&project_dir);

    let db_path = project_dir.join("rosaray.sqlite3");
    let db = sqlite::open(&db_path).expect("failed to open/migrate project database");
    let project_id = sqlite::ensure_default_project(&db).expect("failed to bootstrap project row");
    // FR-020: a fresh process start means no `running` Run Record can
    // actually still be executing — reconcile any left over from a killed
    // prior process to `failed` rather than leaving them looking
    // perpetually in-progress or, worse, ever promoting them to `succeeded`.
    rosaray_service::data_repository::sqlite::run_repo::recover_interrupted_runs(&db)
        .expect("failed to reconcile interrupted runs");

    let blob_store =
        BlobStore::new(project_dir.join("blobs")).expect("failed to initialize blob store");

    let session_token = generate_session_token();
    let (event_tx, _rx) = rosaray_service::events::new_channel();

    let state = AppState(Arc::new(AppStateInner {
        session_token: session_token.clone(),
        session_state: RwLock::new(SessionState::Ready),
        db: Mutex::new(db),
        blob_store,
        master_key,
        event_tx,
        project_id,
        pending_batches: Default::default(),
        pending_batch_paths: Default::default(),
        artifact_registry: Default::default(),
        pending_requests: Default::default(),
        preview_cache: Default::default(),
        preview_isolation: Default::default(),
    }));

    let router = rosaray_service::api::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind loopback listener");
    let port = listener
        .local_addr()
        .expect("listener has a local address")
        .port();

    // Single machine-readable line for the launching process (frontend dev
    // server, packaged app) to pick up the ephemeral port and session token
    // — never written to a file, never logged again after this.
    println!(
        "{}",
        serde_json::json!({ "port": port, "session_token": session_token })
    );

    axum::serve(listener, router)
        .await
        .expect("service exited unexpectedly");
}

fn load_or_init_master_key(project_dir: &std::path::Path) -> MasterKey {
    let salt_path = project_dir.join("project.salt");
    let verifier_path = project_dir.join("project.verifier");

    let passphrase = std::env::var("ROSARAY_PASSPHRASE").unwrap_or_else(|_| {
        eprintln!(
            "ROSARAY_PASSPHRASE is not set; refusing to start without an explicit project passphrase"
        );
        std::process::exit(1);
    });

    if salt_path.exists() {
        let salt = std::fs::read(&salt_path).expect("failed to read project salt");
        let master_key = MasterKey::derive(&passphrase, &salt).expect("key derivation failed");
        let wrapped = std::fs::read(&verifier_path).expect("failed to read project verifier");
        if crypto::decrypt_with_key(master_key.as_bytes(), &decode_wrapped(&wrapped))
            .map(|plain| plain == VERIFIER_PLAINTEXT)
            .unwrap_or(false)
        {
            master_key
        } else {
            eprintln!("credential_invalid: passphrase does not match this project");
            std::process::exit(1);
        }
    } else {
        let salt = crypto::generate_salt();
        std::fs::write(&salt_path, salt).expect("failed to persist project salt");
        let master_key = MasterKey::derive(&passphrase, &salt).expect("key derivation failed");
        let wrapped = crypto::encrypt_with_key(master_key.as_bytes(), VERIFIER_PLAINTEXT)
            .expect("verifier encryption failed");
        std::fs::write(&verifier_path, encode_wrapped(&wrapped))
            .expect("failed to persist project verifier");
        master_key
    }
}

fn encode_wrapped(wrapped: &crypto::WrappedKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(crypto::NONCE_LEN + wrapped.ciphertext.len());
    out.extend_from_slice(&wrapped.nonce);
    out.extend_from_slice(&wrapped.ciphertext);
    out
}

fn decode_wrapped(raw: &[u8]) -> crypto::WrappedKey {
    let (nonce, ciphertext) = raw.split_at(crypto::NONCE_LEN);
    crypto::WrappedKey {
        nonce: nonce
            .try_into()
            .expect("verifier file has a valid nonce prefix"),
        ciphertext: ciphertext.to_vec(),
    }
}

fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

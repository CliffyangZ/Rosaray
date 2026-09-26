#![allow(dead_code)]
//! In-process service over a temp project directory.

use std::path::{Path, PathBuf};

use rosaray_service::api::{build_router, AppState};
use serde_json::{json, Value};

pub struct TestService {
    pub base_url: String,
    pub state: AppState,
    pub project_dir: PathBuf,
    pub catalog_token: String,
    pub report: rosaray_service::bootstrap::StartupReport,
    _dir: Option<tempfile::TempDir>,
    _server: tokio::task::JoinHandle<()>,
}

pub async fn spawn() -> TestService {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    spawn_in(&path, Some(dir)).await
}

pub async fn spawn_in(project_dir: &Path, keep: Option<tempfile::TempDir>) -> TestService {
    let catalog_token = "test-catalog-token-0123456789abcdef0123456789".to_string();
    let (state, report) = rosaray_service::bootstrap::open(project_dir, catalog_token.clone()).unwrap();
    let router = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestService { base_url: format!("http://127.0.0.1:{port}"), state, project_dir: project_dir.to_path_buf(), catalog_token, report, _dir: keep, _server: server }
}

pub enum As {
    Frontend,
    Author,
    SystemOne,
    Nobody,
    /// The frontend credential in another scope's header.
    FrontendAsAuthor,
    FrontendAsSystemOne,
}

impl TestService {
    pub async fn call(&self, who: As, method: reqwest::Method, path: &str, body: Option<Value>) -> (reqwest::StatusCode, Value) {
        let mut req = reqwest::Client::new().request(method, format!("{}{}", self.base_url, path));
        let c = &self.state.creds;
        req = match who {
            As::Frontend => req.header("X-Rosaray-Session", &c.catalog),
            As::Author => req.header("X-Rosaray-Author", &c.author),
            As::SystemOne => req.header("X-Rosaray-SystemOne", &c.system_one),
            As::Nobody => req,
            As::FrontendAsAuthor => req.header("X-Rosaray-Author", &c.catalog),
            As::FrontendAsSystemOne => req.header("X-Rosaray-SystemOne", &c.catalog),
        };
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn get(&self, who: As, path: &str) -> (reqwest::StatusCode, Value) {
        self.call(who, reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, who: As, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
        self.call(who, reqwest::Method::POST, path, Some(body)).await
    }
}

pub fn pipe_submission(id: &str, version: &str) -> Value {
    let md = format!("---\nschema: quantify-kb/1\nkind: algopipe\nid: {id}\nversion: {version}\nname: Demo {id}\nsummary: Threshold segmentation of intraoral photographs\nstatus: draft\nresearch_use_only: true\nintended_use: Threshold segmentation of intraoral photographs\nlimitations: Demo only\n---\nBody\n");
    let graph = json!({
        "schema": "quantify-kb/1",
        "nodes": [
            { "instance_id": "src", "ref": { "id": "rosaray.image-source", "version": "1.0.0" }, "parameters": {} },
            { "instance_id": "norm", "ref": { "id": "rosaray.normalize", "version": "1.0.0" }, "parameters": {} },
            { "instance_id": "th", "ref": { "id": "rosaray.threshold", "version": "1.0.0" }, "parameters": { "mode": "otsu" } },
        ],
        "edges": [ { "from": "src.image", "to": "norm.image" }, { "from": "norm.image", "to": "th.image" } ],
        "target_data_profile": { "profile_version": 1, "require": [{ "predicate": "modality", "equals": "intraoral-photo" }] },
    });
    json!({ "files": { "ALGOPIPE.md": md, "graph.yaml": serde_json::to_string(&graph).unwrap() } })
}

pub fn hash_tree(dir: &Path) -> Vec<(String, String)> {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, String)>) {
        let Ok(rd) = std::fs::read_dir(cur) else { return };
        let mut children: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        children.sort();
        for p in children {
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push((p.strip_prefix(base).unwrap().to_string_lossy().to_string(), blake3::hash(&std::fs::read(&p).unwrap()).to_hex().to_string()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

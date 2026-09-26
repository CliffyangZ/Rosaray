//! Deprecation notices (FR-013, FR-043): appended to the version's amendment
//! chain, never written into the bundle. A deprecated version stays exactly as
//! published and existing references keep resolving to it.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::json;

use super::chain::{append, read_chain, ChainError, ChainHead, ChainRead};

pub const KIND: &str = "deprecation";

/// `amendments/<id>@<version>/` — one chain per published version.
pub fn chain_dir(root: &Path, id: &str, version: &str) -> PathBuf {
    root.join("amendments").join(format!("{id}@{version}"))
}

pub fn deprecate(
    root: &Path,
    id: &str,
    version: &str,
    reason: &str,
    replacement: Option<(&str, &str)>,
) -> Result<ChainHead, ChainError> {
    let mut extra = serde_json::Map::new();
    if let Some((rid, rver)) = replacement {
        extra.insert("replacement".into(), json!({ "id": rid, "version": rver }));
    }
    append(
        &chain_dir(root, id, version),
        super::chain::NewRecord {
            kind: KIND.to_string(),
            subject: json!({ "id": id, "version": version }),
            reason: Some(reason.to_string()),
            author: crate::event_repo::LOCAL_ACTOR.to_string(),
            extra,
        },
    )
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeprecationState {
    pub deprecated: bool,
    pub reason: Option<String>,
    pub replacement: Option<serde_json::Value>,
    pub at: Option<String>,
}

pub fn state_of(chain: &ChainRead) -> DeprecationState {
    match chain.records.iter().rev().find(|r| r.kind == KIND) {
        Some(r) => DeprecationState {
            deprecated: true,
            reason: r.reason.clone(),
            replacement: r.extra.get("replacement").cloned(),
            at: Some(r.created_at.clone()),
        },
        None => DeprecationState::default(),
    }
}

pub fn read(root: &Path, id: &str, version: &str) -> std::io::Result<ChainRead> {
    read_chain(&chain_dir(root, id, version))
}

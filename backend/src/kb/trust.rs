//! Local implementation-trust decisions.
//!
//! Trust is a *local* decision about a bundle's declared implementation, so it
//! lives beside the bundles (`local-trust.yaml`), never inside them: rewriting
//! a published bundle's `implementation.yaml` on import would break its
//! `bundle.lock` and change its `content_id` (Constitution II, FR-043). A
//! bundle with no local record is `untrusted`; nothing imported ever executes
//! (FR-045) and an untrusted binding never counts as `implemented`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kb::bundle::model::{parse_yaml, to_yaml, Trust};
use crate::kb::bundle::read::Bundle;

pub const TRUST_FILE: &str = "local-trust.yaml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustEntry {
    pub id: String,
    pub version: String,
    pub content_id: String,
    pub trust: Trust,
    /// `seed | publish | import`
    pub source: String,
    pub decided_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub entries: Vec<TrustEntry>,
}

fn path(root: &Path) -> PathBuf {
    root.join(TRUST_FILE)
}

pub fn load(root: &Path) -> TrustFile {
    std::fs::read_to_string(path(root))
        .ok()
        .and_then(|t| parse_yaml::<TrustFile>(&t).ok())
        .unwrap_or_default()
}

/// Records (or replaces) the decision for one exact `id@version` + content.
pub fn record(root: &Path, id: &str, version: &str, content_id: &str, trust: Trust, source: &str) -> std::io::Result<()> {
    let mut file = load(root);
    file.schema = "quantify-kb/1".to_string();
    file.entries.retain(|e| !(e.id == id && e.version == version && e.content_id == content_id));
    file.entries.push(TrustEntry {
        id: id.to_string(),
        version: version.to_string(),
        content_id: content_id.to_string(),
        trust,
        source: source.to_string(),
        decided_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    let yaml = to_yaml(&file).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(root)?;
    // Same atomic-replace discipline as bundles, but for a single file.
    let tmp = root.join(format!("{TRUST_FILE}.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, yaml)?;
    std::fs::rename(tmp, path(root))
}

/// The trust in force for `bundle`: the local record for exactly this
/// `id@version` and content, else `untrusted`. An unlocked (draft) bundle's
/// declared trust is honoured only for drafts the researcher is authoring.
pub fn effective_trust(root: &Path, bundle: &Bundle) -> Trust {
    let Some(imp) = &bundle.implementation else { return Trust::Untrusted };
    match &bundle.lock {
        Some(lock) => load(root)
            .entries
            .iter()
            .find(|e| e.id == lock.id && e.version == lock.version && e.content_id == lock.content_id)
            .map(|e| e.trust)
            .unwrap_or(Trust::Untrusted),
        None => imp.trust,
    }
}

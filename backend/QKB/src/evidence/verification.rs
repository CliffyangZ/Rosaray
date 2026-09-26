//! Verification-record chains (research §13). One append-only, hash-chained
//! history per node id under `verification/<node-id>/`. Technical status is
//! *derived* from the chain for an exact subject tuple — never stored on the
//! bundle, never inferred from a different implementation or definition.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::chain::{append, read_chain, ChainError, ChainHead, ChainRead, NewRecord};

/// The exact identity a verification applies to (FR-052): restoring
/// eligibility needs a `passed` record for this identical tuple.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subject {
    pub node_id: String,
    pub definition_content_id: String,
    pub implementation_id: String,
    pub implementation_version: String,
    #[serde(default)]
    pub asset_content_ids: Vec<String>,
}

impl Subject {
    pub fn normalized(mut self) -> Self {
        self.asset_content_ids.sort();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationType {
    Technical,
    Dataset,
}

impl VerificationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerificationType::Technical => "technical",
            VerificationType::Dataset => "dataset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Passed,
    Failed,
    Withdrawn,
}

impl Event {
    pub fn as_str(&self) -> &'static str {
        match self {
            Event::Passed => "passed",
            Event::Failed => "failed",
            Event::Withdrawn => "withdrawn",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "passed" => Some(Event::Passed),
            "failed" => Some(Event::Failed),
            "withdrawn" => Some(Event::Withdrawn),
            _ => None,
        }
    }
}

pub fn chain_dir(root: &Path, node_id: &str) -> PathBuf {
    root.join("verification").join(node_id)
}

/// Appends a verification event for `subject` to the node's chain.
pub fn record_event(
    root: &Path,
    vtype: VerificationType,
    event: Event,
    subject: &Subject,
    suite: Option<Value>,
    scope: Option<Value>,
    reason: Option<String>,
) -> Result<ChainHead, ChainError> {
    let mut extra = serde_json::Map::new();
    extra.insert("type".into(), json!(vtype.as_str()));
    if let Some(s) = suite {
        extra.insert("suite".into(), s);
    }
    if let Some(s) = scope {
        extra.insert("scope".into(), s);
    }
    append(
        &chain_dir(root, &subject.node_id),
        NewRecord {
            kind: event.as_str().to_string(),
            subject: serde_json::to_value(subject.clone().normalized()).expect("subject serializes"),
            reason,
            author: crate::event_repo::LOCAL_ACTOR.to_string(),
            extra,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// No record exists for this exact tuple.
    None,
    Passed,
    Failed,
    Withdrawn,
}

impl Status {
    /// Valid = the last event for the exact tuple is `passed`.
    pub fn is_valid(&self) -> bool {
        *self == Status::Passed
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::None => "none",
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Withdrawn => "withdrawn",
        }
    }
}

/// Current status of `subject` for `vtype`: the last event in the chain whose
/// subject tuple is identical. A record for another implementation version,
/// asset set or definition content never counts.
pub fn current_status(chain: &ChainRead, vtype: VerificationType, subject: &Subject) -> Status {
    let want = serde_json::to_value(subject.clone().normalized()).expect("subject serializes");
    for rec in chain.records.iter().rev() {
        if rec.extra.get("type").and_then(Value::as_str) != Some(vtype.as_str()) {
            continue;
        }
        if rec.subject != want {
            continue;
        }
        return match Event::parse(&rec.kind) {
            Some(Event::Passed) => Status::Passed,
            Some(Event::Failed) => Status::Failed,
            Some(Event::Withdrawn) => Status::Withdrawn,
            None => Status::None,
        };
    }
    Status::None
}

/// Reads the node's chain and answers for one subject. A broken chain yields
/// `Err(break)` so callers can mark the version unavailable.
pub fn status_for(root: &Path, vtype: VerificationType, subject: &Subject) -> std::io::Result<(Status, ChainRead)> {
    let chain = read_chain(&chain_dir(root, &subject.node_id))?;
    let status = if chain.broken.is_some() { Status::None } else { current_status(&chain, vtype, subject) };
    Ok((status, chain))
}

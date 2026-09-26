//! Executability (constitution II/III, data-model §2): may this version remain
//! in the QKB? Derived on demand from files and append-only chains, never
//! stored. Task-specific data applicability is a *different* question
//! ([`crate::eligibility::applicability`]) and never affects retention.
//!
//! * AlgoNode: implementation binding resolves, its local trust is not
//!   `untrusted`, the published content is intact, and its technical
//!   verification for the exact subject tuple is currently `passed`.
//! * AlgoPipe: published as an `executable` release, and every pinned
//!   dependency is present, intact, resolvable, trusted and verified.

use std::path::Path;

use rusqlite::Connection;

use crate::bundle::model::{Kind, Trust};
use crate::bundle::read::{read_bundle, Bundle};
use crate::catalog::status::derive;
use crate::eligibility::{dataset_independent, Reason};
use crate::evidence::verification::Status;

#[derive(Debug, Clone, PartialEq)]
pub struct Executability {
    pub executable: bool,
    pub reasons: Vec<Reason>,
}

impl Executability {
    fn from(reasons: Vec<Reason>) -> Self {
        Self { executable: reasons.is_empty(), reasons }
    }
    /// The first (most fundamental) reason code, for deletion records.
    pub fn primary_code(&self) -> Option<&'static str> {
        self.reasons.first().map(|r| r.code)
    }
}

/// Assessment of one bundle folder on disk.
#[derive(Debug, Clone)]
pub struct Assessment {
    pub kind: Option<Kind>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub content_id: Option<String>,
    pub executability: Executability,
}

/// Assesses the bundle folder at `dir`. An unreadable or unlocked (draft-shaped)
/// folder is `unrecognized_format` / `not_published`, never executable.
pub fn assess_dir(conn: &Connection, root: &Path, dir: &Path) -> Assessment {
    let (bundle, findings) = read_bundle(dir);
    let Some(bundle) = bundle else {
        return Assessment {
            kind: None,
            id: None,
            version: None,
            content_id: None,
            executability: Executability::from(vec![Reason::new("unrecognized_format")]),
        };
    };
    let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
    let executability = assess(conn, root, &bundle, modified);
    Assessment {
        kind: Some(bundle.kind),
        id: Some(bundle.header.id.clone()),
        version: Some(bundle.header.version.clone()),
        content_id: bundle.lock.as_ref().map(|l| l.content_id.clone()),
        executability,
    }
}

pub fn assess(conn: &Connection, root: &Path, bundle: &Bundle, modified: bool) -> Executability {
    if !bundle.is_published() {
        return Executability::from(vec![Reason::new("not_published")]);
    }
    match bundle.kind {
        Kind::Algonode => assess_node(root, bundle, modified),
        Kind::Algopipe => {
            let mut reasons = Vec::new();
            if modified {
                reasons.push(Reason::new("content_modified"));
            }
            reasons.extend(dataset_independent(conn, root, bundle).reasons);
            Executability::from(reasons)
        }
    }
}

fn assess_node(root: &Path, bundle: &Bundle, modified: bool) -> Executability {
    let mut reasons = Vec::new();
    if modified {
        reasons.push(Reason::new("content_modified"));
    }
    let derived = derive(root, bundle, modified);
    if bundle.implementation.is_none() || !derived.implementation_resolvable {
        reasons.push(Reason::new("implementation_unavailable"));
    } else if derived.trust == Trust::Untrusted {
        reasons.push(Reason::new("implementation_untrusted"));
    }
    match derived.verification {
        Status::Passed => {}
        Status::None => reasons.push(Reason::new("verification_missing")),
        Status::Failed => reasons.push(Reason::new("verification_failed")),
        Status::Withdrawn => reasons.push(Reason::new("verification_withdrawn")),
    }
    if derived.availability == "unavailable" && !modified {
        // A broken history chain: the evidence cannot be trusted.
        reasons.push(Reason::new("history_chain_broken"));
    }
    Executability::from(reasons)
}

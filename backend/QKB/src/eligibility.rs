//! Derived execution eligibility (data-model.md, FR-050/FR-052): whether a
//! published AlgoPipe version may start a *formal* Run — computed on demand from
//! files and append-only chains, never stored. Suspension and restoration are just
//! this value changing as verification records are appended; restoring needs a
//! `passed` record for the *identical* subject tuple.

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::bundle::model::Trust;
use crate::bundle::read::{read_bundle, Bundle};
use crate::catalog::query::path_of;
use crate::catalog::status::{derive, is_resolvable_implementation};
use crate::evidence::verification::{status_for, Status, Subject, VerificationType};
use crate::profile::{evaluate_profile, unmet_prerequisites, ImageFacts};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Reason {
    /// `not_executable_release | dependency_unavailable | implementation_unavailable |
    /// verification_withdrawn | verification_missing | profile_unsatisfied | prerequisite_unmet`
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl Reason {
    pub fn new(code: &'static str) -> Self {
        Self { code, node: None, predicate: None, detail: None }
    }
    pub fn node(mut self, n: &str) -> Self {
        self.node = Some(n.to_string());
        self
    }
    pub fn predicate(mut self, p: &str) -> Self {
        self.predicate = Some(p.to_string());
        self
    }
    pub fn detail(mut self, d: Value) -> Self {
        self.detail = Some(d);
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Eligibility {
    pub eligible: bool,
    pub reasons: Vec<Reason>,
}

impl Eligibility {
    pub fn from(reasons: Vec<Reason>) -> Self {
        Self { eligible: reasons.is_empty(), reasons }
    }
    pub fn codes(&self) -> Vec<&'static str> {
        self.reasons.iter().map(|r| r.code).collect()
    }
}

/// Loads a published pipe by `id@version`.
pub fn load_pipe(conn: &Connection, root: &Path, id: &str, version: &str) -> Option<Bundle> {
    let path = path_of(conn, "algopipe", id, version, "published").ok().flatten()?;
    read_bundle(&root.join(path)).0
}

/// Everything that does not depend on which dataset is chosen: the release kind,
/// the dependencies, the pinned implementations and each node's *current*
/// technical verification for its exact subject tuple.
pub fn dataset_independent(conn: &Connection, root: &Path, pipe: &Bundle) -> Eligibility {
    let mut reasons = Vec::new();
    let executable = pipe.lock.as_ref().and_then(|l| l.release_kind.as_deref()) == Some("executable");
    if !executable {
        reasons.push(Reason::new("not_executable_release"));
        return Eligibility::from(reasons);
    }
    let Some(graph) = &pipe.graph else {
        reasons.push(Reason::new("dependency_unavailable"));
        return Eligibility::from(reasons);
    };
    for n in &graph.nodes {
        let r = &n.node_ref;
        let name = n.instance_id.as_str();
        let dep = path_of(conn, "algonode", &r.id, &r.version, "published").ok().flatten().and_then(|p| {
            let (b, findings) = read_bundle(&root.join(p));
            b.map(|b| (b, findings))
        });
        let Some((dep, findings)) = dep else {
            reasons.push(Reason::new("dependency_unavailable").node(name).detail(json!({ "ref": format!("{}@{}", r.id, r.version) })));
            continue;
        };
        let modified = findings.iter().any(|f| f.code == "published_bundle_modified");
        let derived = derive(root, &dep, modified);
        let intact = dep.lock.as_ref().map(|l| l.content_id.as_str()) == r.content_id.as_deref();
        if derived.availability == "unavailable" || !intact {
            reasons.push(Reason::new("dependency_unavailable").node(name).detail(json!({ "ref": format!("{}@{}", r.id, r.version) })));
            continue;
        }
        let Some(pin) = graph.implementation_pins.iter().find(|p| p.instance_id == n.instance_id) else {
            reasons.push(Reason::new("implementation_unavailable").node(name).detail(json!({ "why": "not pinned" })));
            continue;
        };
        if derived.trust == Trust::Untrusted || !is_resolvable_implementation(&pin.implementation_id) {
            reasons.push(Reason::new("implementation_unavailable").node(name).detail(json!({ "implementation_id": pin.implementation_id })));
            continue;
        }
        let subject = Subject {
            node_id: r.id.clone(),
            definition_content_id: r.content_id.clone().unwrap_or_default(),
            implementation_id: pin.implementation_id.clone(),
            implementation_version: pin.version.clone(),
            asset_content_ids: pin.asset_content_ids.clone(),
        };
        match status_for(root, VerificationType::Technical, &subject) {
            Ok((Status::Passed, _)) => {}
            Ok((Status::None, chain)) if chain.broken.is_some() => {
                reasons.push(Reason::new("dependency_unavailable").node(name).detail(json!({ "why": "history chain broken" })))
            }
            Ok((Status::None, _)) => reasons.push(Reason::new("verification_missing").node(name)),
            Ok((_, _)) => reasons.push(Reason::new("verification_withdrawn").node(name)),
            Err(_) => reasons.push(Reason::new("dependency_unavailable").node(name)),
        }
    }
    Eligibility::from(reasons)
}

/// Applicability of an executable pipe to *declared* data conditions: the pipe's
/// Target Data Profile and every node's prerequisites evaluated against `facts`.
/// This never affects whether the pipe is retained (constitution III) — it only
/// says whether the pipe fits one query.
pub fn applicability(conn: &Connection, root: &Path, pipe: &Bundle, facts: &ImageFacts) -> Vec<Reason> {
    let images = std::slice::from_ref(facts);
    let mut reasons: Vec<Reason> = Vec::new();
    let Some(graph) = &pipe.graph else { return reasons };
    if let Some(profile) = &graph.target_data_profile {
        let eval = evaluate_profile(profile.require.iter().map(|p| (p.predicate.as_str(), &p.args)), images);
        for u in eval.unsatisfied {
            reasons.push(Reason::new("profile_unsatisfied").predicate(&u.predicate).detail(json!({
                "expected": u.expected, "observed": u.observed,
            })));
        }
    }
    for n in &graph.nodes {
        let Some(path) = path_of(conn, "algonode", &n.node_ref.id, &n.node_ref.version, "published").ok().flatten() else { continue };
        let Some(contract) = read_bundle(&root.join(path)).0.and_then(|b| b.contract) else { continue };
        let unmet = unmet_prerequisites(contract.prerequisites.iter().map(|p| (p.predicate.as_str(), &p.args)), facts);
        for u in unmet {
            let r = Reason::new("prerequisite_unmet").node(&n.instance_id).predicate(&u.predicate).detail(json!({ "expected": u.expected, "observed": u.observed }));
            if !reasons.contains(&r) {
                reasons.push(r);
            }
        }
    }
    reasons
}
